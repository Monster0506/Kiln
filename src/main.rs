#[cfg(feature = "profiling")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

use clap::{Parser, Subcommand};
use kiln_compiler::analyzer::{
    analyze_with_base, analyze_with_base_and_registry, analyze_with_base_and_symbols, SymbolList,
};
use kiln_compiler::annotations::{
    default_registry, run_processors, run_source_processors, run_user_processors,
};
use kiln_compiler::codegen::{compile::compile, context::CodegenContext, emit};
use kiln_compiler::diagnostics::timing::{BuildStats, ItemCounts, PhaseTimer};
use kiln_compiler::diagnostics::{RenderOpts, SourceMap};
use kiln_compiler::lexer::Lexer;
use kiln_compiler::parser::ast::Item;
use kiln_compiler::parser::Parser as KilnParser;
use kiln_compiler::test_harness::inject_harness;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "kiln", about = "The Kiln compiler")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Lex a source file and print the token stream
    Lex {
        /// Path to the .kn file
        file: String,
    },
    /// Parse a source file and print the AST
    Parse {
        /// Path to the .kn file
        file: String,
    },
    /// Type-check a source file
    Check {
        /// Path to the .kn file
        file: PathBuf,
        /// Re-run check on every file change
        #[arg(long)]
        watch: bool,
        /// Print phase timing table to stderr
        #[arg(long)]
        timing: bool,
        /// Print profile stats (type registry, method frequency) to stderr
        #[arg(long)]
        profile: bool,
    },
    /// Compile a source file to a native executable (or object with --no-link)
    Build {
        /// Path to the .kn source file
        file: PathBuf,
        /// Output path (default: <file>.exe on Windows, <file> on Unix)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Emit an object file only; do not link
        #[arg(long)]
        no_link: bool,
        /// Re-run build on every file change
        #[arg(long)]
        watch: bool,
        /// Print phase timing table to stderr
        #[arg(long)]
        timing: bool,
        /// Print full timing detail to stderr (implies --timing)
        #[arg(long)]
        verbose: bool,
        /// Number of optimization loop iterations (0 = none, default 3)
        #[arg(short = 'O', long = "opt-level", default_value_t = 3u8)]
        opt_level: u8,
        /// Write the optimizer-transformed source to <file>.opt.kn
        #[arg(long)]
        emit: bool,
        /// Print profile stats (type registry, method frequency) to stderr
        #[arg(long)]
        profile: bool,
    },
    /// Compile and run a source file
    Run {
        /// Path to the .kn source file
        file: PathBuf,
        /// Print phase timing table to stderr
        #[arg(long)]
        timing: bool,
        /// Print full timing detail to stderr (implies --timing)
        #[arg(long)]
        verbose: bool,
        /// Number of optimization loop iterations (0 = none, default 3)
        #[arg(short = 'O', long = "opt-level", default_value_t = 3u8)]
        opt_level: u8,
        /// Write the optimizer-transformed source to <file>.opt.kn
        #[arg(long)]
        emit: bool,
        /// Print profile stats (type registry, method frequency) to stderr
        #[arg(long)]
        profile: bool,
    },
    /// Run @test-annotated functions in a source file
    Test {
        /// Path to the .kn source file
        file: PathBuf,
        /// Print phase timing table to stderr
        #[arg(long)]
        timing: bool,
        /// Print full timing detail to stderr (implies --timing)
        #[arg(long)]
        verbose: bool,
        /// Number of optimization loop iterations (0 = none, default 3)
        #[arg(short = 'O', long = "opt-level", default_value_t = 3u8)]
        opt_level: u8,
    },
}

struct BuildOptions {
    timing: bool,
    verbose: bool,
    opt_level: u8,
    emit: bool,
    profile: bool,
}

fn build_exe(file: &PathBuf, output: Option<PathBuf>, opts: &BuildOptions) -> PathBuf {
    match run_build(file, output, false, opts) {
        BuildOutcome::Ok(path) => path,
        BuildOutcome::Errors(errs) => {
            for e in &errs {
                eprintln!("{e}");
            }
            std::process::exit(1);
        }
    }
}

fn count_ast_nodes(ast: &kiln_compiler::parser::ast::SourceFile) -> usize {
    ast.items.len()
}

fn count_item_kinds(ast: &kiln_compiler::parser::ast::SourceFile) -> ItemCounts {
    let mut c = ItemCounts::default();
    for item in &ast.items {
        match item {
            Item::Function(_) => c.functions += 1,
            Item::Struct(_) => c.structs += 1,
            Item::Enum(_) => c.enums += 1,
            Item::ProcessorDef(_) => c.processors += 1,
            _ => {}
        }
    }
    c
}

fn build_symbol_rows(syms: &SymbolList) -> Vec<(String, String, usize)> {
    use kiln_compiler::analyzer::env::Symbol;
    let mut rows: Vec<(String, String, usize)> = syms
        .iter()
        .filter_map(|(name, sym)| {
            let (kind, count) = match sym {
                Symbol::Fn { .. } => ("Fn", 1usize),
                Symbol::FnOverloadSet { overloads } => ("Fn", overloads.len()),
                Symbol::Var { .. } => ("Var", 1),
                Symbol::Type { .. } => ("Type", 1),
                Symbol::TypeAlias(_) => ("Alias", 1),
                Symbol::Iface { .. } => ("Iface", 1),
                Symbol::Const { .. } => ("Const", 1),
                Symbol::StructField { .. } => return None,
            };
            Some((name.clone(), kind.to_string(), count))
        })
        .collect();
    rows.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    rows
}

fn diag_prefix(code: &str) -> &'static str {
    if code.starts_with('W') {
        "warning"
    } else {
        "error"
    }
}

fn emit_diagnostic(code: &str, msg: &str, snippet: &str) {
    eprintln!("{}[{code}]: {msg}", diag_prefix(code));
    eprintln!("{snippet}");
}

fn emit_diagnostic_no_span(code: &str, msg: &str) {
    eprintln!("{}[{code}]: {msg}", diag_prefix(code));
}

fn diag_summary(errors: usize, warnings: usize) -> String {
    let es = if errors == 1 { "" } else { "s" };
    let ws = if warnings == 1 { "" } else { "s" };
    match (errors, warnings) {
        (0, 0) => String::new(),
        (0, w) => format!("{w} warning{ws}"),
        (e, 0) => format!("{e} error{es}"),
        (e, w) => format!("{e} error{es}, {w} warning{ws}"),
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
struct DiagKey {
    code: String,
    file: String,
    message: String,
}

struct CheckResult {
    outcome: CheckOutcome,
    keys: std::collections::HashSet<DiagKey>,
}

fn compute_gutter_width(
    map: &SourceMap,
    errs: &[kiln_compiler::analyzer::AnalysisError],
    context_after: usize,
) -> usize {
    errs.iter()
        .flat_map(|e| {
            let (line, _) = map.location_of(e.span().start);
            let note_lines: Vec<usize> = e
                .note_info()
                .into_iter()
                .filter_map(|(_, ns)| ns)
                .map(|ns| {
                    let (l, _) = map.location_of(ns.start);
                    l + context_after
                })
                .collect();
            let mut lines = vec![line + context_after];
            lines.extend(note_lines);
            lines.into_iter()
        })
        .map(|l| l.to_string().len())
        .max()
        .unwrap_or(1)
}

fn format_analysis_errors_rich(
    errs: &[kiln_compiler::analyzer::AnalysisError],
    map: &SourceMap,
    src: &str,
    path: &str,
) -> Vec<String> {
    use kiln_compiler::diagnostics::colors::get_colors;
    let c = get_colors();
    let gw = compute_gutter_width(map, errs, 1);
    errs.iter()
        .map(|e| {
            let code = e.code();
            let label_owned = e.caret_label();
            let label = label_owned.as_deref();
            let opts = RenderOpts {
                code,
                caret_label: label,
                gutter_width: gw,
                hyperlinks: c.hyperlinks,
                context_before: 2,
                context_after: 1,
            };
            let block = map.render_rich(src, e.span(), path, &opts);
            let prefix_color = c.code_color(code);
            let header = format!(
                "{prefix_color}{}[{code}]:{} {}",
                diag_prefix(code),
                c.reset,
                e.message()
            );
            let mut msg = format!("{header}\n{block}");
            for (note, note_span) in e.note_info() {
                if let Some(ns) = note_span {
                    let note_opts = RenderOpts {
                        code: "note",
                        caret_label: None,
                        gutter_width: gw,
                        hyperlinks: c.hyperlinks,
                        context_before: 0,
                        context_after: 0,
                    };
                    let note_block = map.render_note_rich(src, ns, path, &note, &note_opts);
                    msg.push('\n');
                    msg.push_str(&note_block);
                } else {
                    let nc = c.note;
                    let r = c.reset;
                    msg.push_str(&format!("\n{nc}note:{r} {note}"));
                }
            }
            msg
        })
        .collect()
}

fn diag_keys_from_errs(
    errs: &[kiln_compiler::analyzer::AnalysisError],
    path: &str,
) -> std::collections::HashSet<DiagKey> {
    errs.iter()
        .map(|e| DiagKey {
            code: e.code().to_string(),
            file: path.to_string(),
            message: e.message(),
        })
        .collect()
}

pub enum BuildOutcome {
    Ok(PathBuf),
    Errors(Vec<String>),
}

fn user_item_names(
    ast: &kiln_compiler::parser::ast::SourceFile,
) -> std::collections::HashSet<String> {
    ast.items
        .iter()
        .filter_map(|i| match i {
            Item::Function(f) => Some(f.name.clone()),
            Item::Struct(s) => Some(s.name.clone()),
            Item::Enum(e) => Some(e.name.clone()),
            Item::Global(g) => Some(g.name.clone()),
            Item::Const(c) => Some(c.name.clone()),
            Item::Interface(i) => Some(i.name.clone()),
            Item::ImplBlock(b) => match &b.for_type {
                kiln_compiler::parser::ast::TypeExpr::Named { name, .. } => Some(name.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn run_build(
    file: &PathBuf,
    output: Option<PathBuf>,
    no_link: bool,
    opts: &BuildOptions,
) -> BuildOutcome {
    let verbose = opts.verbose;
    let timing = opts.timing || opts.verbose;

    let path = file.to_string_lossy().to_string();
    let src = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => return BuildOutcome::Errors(vec![format!("error reading {path}: {e}")]),
    };
    let map = SourceMap::new(&src);

    let mut timer = PhaseTimer::new();
    let mut stats = BuildStats {
        source_file: path.clone(),
        source_lines: src.lines().count(),
        ..BuildStats::default()
    };

    timer.start("lex");
    let tokens = match Lexer::new(&src).tokenize() {
        Ok(t) => t,
        Err(errors) => {
            let msgs = errors
                .iter()
                .map(|e| {
                    let snippet = map.render_diagnostic(&src, e.span(), &path);
                    format!(
                        "{}[{}]: {}\n{}",
                        diag_prefix(e.code()),
                        e.code(),
                        e.message(),
                        snippet
                    )
                })
                .collect();
            return BuildOutcome::Errors(msgs);
        }
    };
    timer.stop();
    stats.token_count = tokens.len();

    timer.start("parse");
    let mut ast = match KilnParser::new(tokens).parse_file() {
        Ok(a) => a,
        Err(e) => {
            let msg = e.message();
            let formatted = if let Some(span) = e.span() {
                let snippet = map.render_diagnostic(&src, span, &path);
                format!(
                    "{}[{}]: {}\n{}",
                    diag_prefix(e.code()),
                    e.code(),
                    msg,
                    snippet
                )
            } else {
                format!("{}[{}]: {}", diag_prefix(e.code()), e.code(), msg)
            };
            return BuildOutcome::Errors(vec![formatted]);
        }
    };
    timer.stop();
    stats.ast_node_count = count_ast_nodes(&ast);
    stats.item_counts = count_item_kinds(&ast);

    ast.items.retain(|item| {
        if let kiln_compiler::parser::ast::Item::Function(f) = item {
            return !f.annotations.iter().any(|a| a.name == "test");
        }
        true
    });

    let registry = default_registry();
    timer.start("processors");
    run_source_processors(&mut ast, &registry);
    let mut proc_errors: Vec<kiln_compiler::analyzer::AnalysisError> = vec![];
    let proc_runs = run_user_processors(&mut ast, &mut proc_errors);
    timer.stop();
    stats.processor_runs = proc_runs;

    let base_dir = file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    if !proc_errors.is_empty() {
        emit_analysis_errors(&proc_errors, &map, &src, &path);
    }

    timer.start("analyze");
    let analyze_result = if opts.profile {
        analyze_with_base_and_symbols(&ast, &base_dir)
            .map(|(tf, reg, syms, warns)| (tf, reg, Some(syms), warns))
    } else {
        analyze_with_base_and_registry(&ast, &base_dir)
            .map(|(tf, reg, warns)| (tf, reg, None, warns))
    };
    let (mut typed_file, type_registry, env_symbols, build_warnings) = match analyze_result {
        Ok(t) => t,
        Err(errs) => {
            let ecount = errs.iter().filter(|e| !e.code().starts_with('W')).count();
            let wcount = errs.iter().filter(|e| e.code().starts_with('W')).count();
            let mut msgs = format_analysis_errors_rich(&errs, &map, &src, &path);
            let summary = diag_summary(ecount, wcount);
            if !summary.is_empty() {
                msgs.push(summary);
            }
            return BuildOutcome::Errors(msgs);
        }
    };
    for w in &build_warnings {
        let snippet = map.render_diagnostic(&src, w.span(), &path);
        emit_diagnostic(w.code(), &w.message(), &snippet);
        for (note, note_span) in w.note_info() {
            if let Some(ns) = note_span {
                let note_block = map.render_note(&src, ns, &path, &note);
                eprintln!("{note_block}");
            } else {
                eprintln!("note: {note}");
            }
        }
    }
    timer.stop();
    run_processors(&ast, &mut typed_file, &registry);

    if opts.emit {
        let user_names = user_item_names(&ast);
        let mut opt = typed_file.clone();
        for _ in 0..opts.opt_level {
            kiln_compiler::analyzer::opt_notes::reset_changes();
            opt = kiln_compiler::analyzer::fold::fold_file(opt);
            let _ = kiln_compiler::analyzer::opt_notes::drain_notes();
            opt = kiln_compiler::analyzer::prop::propagate_file(opt);
            if kiln_compiler::analyzer::opt_notes::change_count() == 0 {
                break;
            }
        }
        if opts.opt_level > 0 {
            use kiln_compiler::analyzer::{dce, fold, opt_notes, prop, purity, unroll};
            // Final fold pass.
            opt_notes::reset_changes();
            opt = fold::fold_file(opt);
            let _ = opt_notes::drain_notes();
            // WAW: mut x = e1; x = e2 -> mut x = e2.
            opt = dce::waw_file(opt);
            // Remove dead immutable bindings.
            opt = dce::dce_file(opt);
            // Second prop pass after WAW.
            opt = prop::propagate_file(opt);
            opt = fold::fold_file(opt);
            opt = dce::dce_file(opt);
            // Demote mut flags that are no longer assigned (emit-only).
            opt = dce::demote_mut_flags_file(opt);
            // Build impure set and tag transitively impure functions in the AST.
            let impure = purity::build_impure_set(&opt);
            opt = purity::tag_impure_functions(opt, &impure);
            // Inline pure function calls with all-literal args, then fold the result.
            opt = dce::inline_pure_const_calls_file(opt, &impure);
            opt = fold::fold_file(opt);
            // Unroll small constant-bound while loops, then fold and prop the result.
            opt = unroll::unroll_file(opt);
            opt = prop::propagate_file(opt);
            opt = fold::fold_file(opt);
            opt = dce::waw_file(opt);
            opt = dce::dce_file(opt);
            // Inline single-use immutable bindings (purity-aware: can inline pure calls).
            opt = dce::single_use_inline_file_with_purity(opt, &impure);
            // DCE using purity info (can drop dead bindings whose RHS calls pure functions).
            opt = dce::dce_file_with_purity(opt, &impure);
            // Remove unreachable user functions and unused globals.
            opt = dce::eliminate_dead_fns(opt, &user_names);
            opt = dce::eliminate_dead_globals(opt, &user_names);
        }
        let opt_src = kiln_compiler::analyzer::pretty::emit_optimized(&opt, &user_names);
        let opt_path = file.with_extension("opt.kn");
        if let Err(e) = fs::write(&opt_path, &opt_src) {
            eprintln!("emit warning: could not write {}: {e}", opt_path.display());
        } else {
            eprintln!("emitted {}", opt_path.display());
        }
    }

    let module_name = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let mut cgx = CodegenContext::new(module_name);

    timer.start("codegen");
    let fn_times = match compile(
        &typed_file,
        &mut cgx,
        verbose,
        opts.opt_level,
        &type_registry,
    ) {
        Ok(t) => t,
        Err(e) => return BuildOutcome::Errors(vec![format!("codegen error: {e}")]),
    };
    timer.stop();
    if verbose {
        stats.fn_codegen_times = fn_times;
    }

    timer.start("emit");
    let obj_bytes = match emit::emit_object(cgx) {
        Ok(b) => b,
        Err(e) => return BuildOutcome::Errors(vec![format!("emit error: {e}")]),
    };
    timer.stop();
    stats.object_bytes = obj_bytes.len();

    let out_path = if no_link {
        let obj_path = output.unwrap_or_else(|| file.with_extension("o"));
        if let Err(e) = fs::write(&obj_path, &obj_bytes) {
            return BuildOutcome::Errors(vec![format!("write error: {e}")]);
        }
        obj_path
    } else {
        let exe_ext = if cfg!(windows) { "exe" } else { "" };
        let exe_path = output.unwrap_or_else(|| file.with_extension(exe_ext));
        let tmp_obj = std::env::temp_dir().join(format!(
            "kiln_{}.o",
            file.file_stem().and_then(|s| s.to_str()).unwrap_or("out")
        ));

        timer.start("link");
        if let Err(e) = emit::link_executable(&obj_bytes, &tmp_obj, &exe_path, false) {
            return BuildOutcome::Errors(vec![format!("link error: {e}")]);
        }
        timer.stop();
        let _ = fs::remove_file(&tmp_obj);

        if let Ok(meta) = fs::metadata(&exe_path) {
            stats.binary_bytes = meta.len() as usize;
        }
        stats.object_path = tmp_obj.to_string_lossy().to_string();
        stats.binary_path = exe_path.to_string_lossy().to_string();

        exe_path
    };

    if timing {
        timer.report(&stats, verbose, &mut std::io::stderr());
    }

    if opts.profile {
        let mut stats = type_registry.profile_stats();
        if let Some(syms) = env_symbols {
            stats.symbols = build_symbol_rows(&syms);
        }
        stats.report(&mut std::io::stderr());
    }

    BuildOutcome::Ok(out_path)
}

fn format_build_result(outcome: &BuildOutcome, time: &str) -> String {
    match outcome {
        BuildOutcome::Ok(path) => format!("[ok] {time}  built {}", path.display()),
        BuildOutcome::Errors(errs) => {
            let first = errs
                .first()
                .map(|s| s.lines().next().unwrap_or(""))
                .unwrap_or("error");
            format!("[error] {time}  {first}")
        }
    }
}

fn run_build_watch(file: &PathBuf, output: Option<PathBuf>, no_link: bool, opts: &BuildOptions) {
    use notify::{Config, Event, RecommendedWatcher, Watcher};
    use std::sync::mpsc;
    use std::time::Duration;

    let path = file.to_string_lossy().to_string();
    println!("watching {path}");

    let outcome = run_build(file, output.clone(), no_link, opts);
    println!("{}", format_build_result(&outcome, &current_time_str()));
    if let BuildOutcome::Errors(ref errs) = outcome {
        for e in errs {
            println!("{e}");
        }
    }

    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(tx, Config::default()).unwrap_or_else(|e| {
        eprintln!("watcher setup error: {e}");
        std::process::exit(1);
    });
    let imports = imported_paths(file);
    register_watch_paths(&mut watcher, file, &imports);

    while rx.recv().is_ok() {
        let deadline = std::time::Instant::now() + Duration::from_millis(50);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(_) => {}
                Err(_) => break,
            }
        }

        let new_imports = imported_paths(file);
        for p in &new_imports {
            if !imports.contains(p) {
                watcher
                    .watch(p.as_ref(), notify::RecursiveMode::NonRecursive)
                    .unwrap_or_else(|e| eprintln!("watch error on {}: {e}", p.display()));
            }
        }

        let outcome = run_build(file, output.clone(), no_link, opts);
        println!("{}", format_build_result(&outcome, &current_time_str()));
        if let BuildOutcome::Errors(ref errs) = outcome {
            for e in errs {
                println!("{e}");
            }
        }
    }
}

pub enum CheckOutcome {
    Ok,
    Errors(Vec<String>),
}

fn run_check(file: &PathBuf, profile: bool, timing: bool) -> CheckResult {
    let path = file.to_string_lossy().to_string();
    let src = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            return CheckResult {
                outcome: CheckOutcome::Errors(vec![format!("error reading {path}: {e}")]),
                keys: Default::default(),
            }
        }
    };
    let map = SourceMap::new(&src);
    let mut timer = kiln_compiler::diagnostics::timing::PhaseTimer::new();

    timer.start("lex");
    let tokens = match Lexer::new(&src).tokenize() {
        Ok(t) => t,
        Err(errors) => {
            let msgs = errors
                .iter()
                .map(|e| {
                    let snippet = map.render_diagnostic(&src, e.span(), &path);
                    format!(
                        "{}[{}]: {}\n{}",
                        diag_prefix(e.code()),
                        e.code(),
                        e.message(),
                        snippet
                    )
                })
                .collect();
            return CheckResult {
                outcome: CheckOutcome::Errors(msgs),
                keys: Default::default(),
            };
        }
    };
    timer.stop();

    timer.start("parse");
    let mut ast = match KilnParser::new(tokens).parse_file() {
        Ok(a) => a,
        Err(e) => {
            let msg = e.message();
            let formatted = if let Some(span) = e.span() {
                let snippet = map.render_diagnostic(&src, span, &path);
                format!(
                    "{}[{}]: {}\n{}",
                    diag_prefix(e.code()),
                    e.code(),
                    msg,
                    snippet
                )
            } else {
                format!("{}[{}]: {}", diag_prefix(e.code()), e.code(), msg)
            };
            return CheckResult {
                outcome: CheckOutcome::Errors(vec![formatted]),
                keys: Default::default(),
            };
        }
    };
    timer.stop();

    let registry = default_registry();
    run_source_processors(&mut ast, &registry);
    let mut proc_errors: Vec<kiln_compiler::analyzer::AnalysisError> = vec![];
    run_user_processors(&mut ast, &mut proc_errors);

    let base_dir = file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    fn emit_warnings_rich(
        warnings: &[kiln_compiler::analyzer::AnalysisError],
        map: &SourceMap,
        src: &str,
        path: &str,
    ) {
        use kiln_compiler::diagnostics::colors::get_colors;
        let c = get_colors();
        let gw = compute_gutter_width(map, warnings, 1);
        for w in warnings {
            let code = w.code();
            let label_owned = w.caret_label();
            let label = label_owned.as_deref();
            let opts = RenderOpts {
                code,
                caret_label: label,
                gutter_width: gw,
                hyperlinks: c.hyperlinks,
                context_before: 2,
                context_after: 1,
            };
            let block = map.render_rich(src, w.span(), path, &opts);
            let prefix_color = c.code_color(code);
            eprintln!(
                "{prefix_color}{}[{code}]:{} {}",
                diag_prefix(code),
                c.reset,
                w.message()
            );
            eprintln!("{block}");
            for (note, note_span) in w.note_info() {
                if let Some(ns) = note_span {
                    let note_opts = RenderOpts {
                        code: "note",
                        caret_label: None,
                        gutter_width: gw,
                        hyperlinks: c.hyperlinks,
                        context_before: 0,
                        context_after: 0,
                    };
                    let note_block = map.render_note_rich(src, ns, path, &note, &note_opts);
                    eprintln!("{note_block}");
                } else {
                    let nc = c.note;
                    let r = c.reset;
                    eprintln!("{nc}note:{r} {note}");
                }
            }
            eprintln!();
        }
    }

    timer.start("analyze");
    let result = if profile {
        match analyze_with_base_and_symbols(&ast, &base_dir) {
            Ok((_, type_registry, syms, warnings)) => {
                timer.stop();
                let keys = diag_keys_from_errs(&warnings, &path);
                emit_warnings_rich(&warnings, &map, &src, &path);
                let summary = diag_summary(0, warnings.len());
                if !summary.is_empty() {
                    eprintln!("{summary}");
                }
                let mut stats = type_registry.profile_stats();
                stats.symbols = build_symbol_rows(&syms);
                stats.report(&mut std::io::stderr());
                CheckResult {
                    outcome: CheckOutcome::Ok,
                    keys,
                }
            }
            Err(errs) => {
                timer.stop();
                let keys = diag_keys_from_errs(&errs, &path);
                let ecount = errs.iter().filter(|e| !e.code().starts_with('W')).count();
                let wcount = errs.iter().filter(|e| e.code().starts_with('W')).count();
                let mut msgs = format_analysis_errors_rich(&errs, &map, &src, &path);
                let summary = diag_summary(ecount, wcount);
                if !summary.is_empty() {
                    msgs.push(summary);
                }
                CheckResult {
                    outcome: CheckOutcome::Errors(msgs),
                    keys,
                }
            }
        }
    } else {
        match analyze_with_base(&ast, &base_dir) {
            Ok((_, warnings)) => {
                timer.stop();
                let keys = diag_keys_from_errs(&warnings, &path);
                emit_warnings_rich(&warnings, &map, &src, &path);
                let summary = diag_summary(0, warnings.len());
                if !summary.is_empty() {
                    eprintln!("{summary}");
                }
                CheckResult {
                    outcome: CheckOutcome::Ok,
                    keys,
                }
            }
            Err(errs) => {
                timer.stop();
                let keys = diag_keys_from_errs(&errs, &path);
                let ecount = errs.iter().filter(|e| !e.code().starts_with('W')).count();
                let wcount = errs.iter().filter(|e| e.code().starts_with('W')).count();
                let mut msgs = format_analysis_errors_rich(&errs, &map, &src, &path);
                let summary = diag_summary(ecount, wcount);
                if !summary.is_empty() {
                    msgs.push(summary);
                }
                CheckResult {
                    outcome: CheckOutcome::Errors(msgs),
                    keys,
                }
            }
        }
    };

    if timing {
        timer.report_simple("kiln check timing", &mut std::io::stderr());
    }

    result
}

fn format_watch_result(outcome: &CheckOutcome, time: &str) -> String {
    match outcome {
        CheckOutcome::Ok => format!("[ok] {time}"),
        CheckOutcome::Errors(errs) => {
            let first = errs
                .first()
                .map(|s| s.lines().next().unwrap_or(""))
                .unwrap_or("error");
            format!("[error] {time}  {first}")
        }
    }
}

fn current_time_str() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

fn imported_paths(file: &PathBuf) -> Vec<PathBuf> {
    use kiln_compiler::analyzer::collect_imported_disk_paths;
    let src = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    let base_dir = file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let tokens = match kiln_compiler::lexer::Lexer::new(&src).tokenize() {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    match kiln_compiler::parser::Parser::new(tokens).parse_file() {
        Ok(ast) => collect_imported_disk_paths(&ast, &base_dir),
        Err(_) => vec![],
    }
}

fn register_watch_paths(watcher: &mut dyn notify::Watcher, file: &PathBuf, extra: &[PathBuf]) {
    use notify::RecursiveMode;
    watcher
        .watch(file.as_ref(), RecursiveMode::NonRecursive)
        .unwrap_or_else(|e| eprintln!("watch error: {e}"));
    for p in extra {
        watcher
            .watch(p.as_ref(), RecursiveMode::NonRecursive)
            .unwrap_or_else(|e| eprintln!("watch error on {}: {e}", p.display()));
    }
}

fn print_watch_result(
    result: &CheckResult,
    time: &str,
    prev_keys: Option<&std::collections::HashSet<DiagKey>>,
) {
    let unchanged = prev_keys == Some(&result.keys);
    if unchanged {
        match &result.outcome {
            CheckOutcome::Ok => println!("[ok] {time} (no change)"),
            CheckOutcome::Errors(_) => println!("[error] {time} (no change)"),
        }
        return;
    }

    if let Some(prev) = prev_keys {
        let mut fixed: Vec<_> = prev.difference(&result.keys).collect();
        let mut new: Vec<_> = result.keys.difference(prev).collect();
        fixed.sort_by_key(|k| (&k.file, &k.code, &k.message));
        new.sort_by_key(|k| (&k.file, &k.code, &k.message));
        for k in &fixed {
            println!(
                "  fixed: {}[{}]: {}",
                diag_prefix(&k.code),
                k.code,
                k.message
            );
        }
        for k in &new {
            println!(
                "  new:   {}[{}]: {}",
                diag_prefix(&k.code),
                k.code,
                k.message
            );
        }
    }

    println!("{}", format_watch_result(&result.outcome, time));
    if let CheckOutcome::Errors(ref errs) = result.outcome {
        for (i, e) in errs.iter().enumerate() {
            if i > 0 {
                println!();
            }
            println!("{e}");
        }
    }
}

fn run_watch(file: &PathBuf) {
    use notify::{Config, Event, RecommendedWatcher, Watcher};
    use std::collections::HashSet;
    use std::sync::mpsc;
    use std::time::Duration;

    let path = file.to_string_lossy().to_string();
    println!("watching {path}");

    let result = run_check(file, false, false);
    print_watch_result(&result, &current_time_str(), None);
    let mut prev_keys: HashSet<DiagKey> = result.keys;

    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(tx, Config::default()).unwrap_or_else(|e| {
        eprintln!("watcher setup error: {e}");
        std::process::exit(1);
    });
    let imports = imported_paths(file);
    register_watch_paths(&mut watcher, file, &imports);

    while rx.recv().is_ok() {
        let deadline = std::time::Instant::now() + Duration::from_millis(50);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match rx.recv_timeout(remaining) {
                Ok(_) => {}
                Err(_) => break,
            }
        }

        let new_imports = imported_paths(file);
        for p in &new_imports {
            if !imports.contains(p) {
                watcher
                    .watch(p.as_ref(), notify::RecursiveMode::NonRecursive)
                    .unwrap_or_else(|e| eprintln!("watch error on {}: {e}", p.display()));
            }
        }

        let result = run_check(file, false, false);
        print_watch_result(&result, &current_time_str(), Some(&prev_keys));
        prev_keys = result.keys;
    }
}

fn emit_analysis_errors(
    errs: &[kiln_compiler::analyzer::AnalysisError],
    map: &SourceMap,
    src: &str,
    path: &str,
) {
    let msgs = format_analysis_errors_rich(errs, map, src, path);
    for (i, msg) in msgs.iter().enumerate() {
        if i > 0 {
            eprintln!();
        }
        eprintln!("{msg}");
    }
}

fn main() {
    #[cfg(feature = "profiling")]
    let _profiler = dhat::Profiler::new_heap();

    let cli = Cli::parse();
    match cli.command {
        Command::Lex { file } => {
            let src = fs::read_to_string(&file).unwrap_or_else(|e| {
                eprintln!("error reading {file}: {e}");
                std::process::exit(1);
            });
            let map = SourceMap::new(&src);
            match Lexer::new(&src).tokenize() {
                Ok(tokens) => {
                    for tok in &tokens {
                        println!("{:?}", tok);
                    }
                }
                Err(errors) => {
                    for e in &errors {
                        let snippet = map.render_diagnostic(&src, e.span(), &file);
                        emit_diagnostic(e.code(), &e.message(), &snippet);
                    }
                    std::process::exit(1);
                }
            }
        }

        Command::Check {
            file,
            watch,
            timing,
            profile,
        } => {
            if watch {
                run_watch(&file);
            } else {
                match run_check(&file, profile, timing).outcome {
                    CheckOutcome::Ok => println!("ok"),
                    CheckOutcome::Errors(errs) => {
                        for (i, e) in errs.iter().enumerate() {
                            if i > 0 {
                                eprintln!();
                            }
                            eprintln!("{e}");
                        }
                        std::process::exit(1);
                    }
                }
            }
        }

        Command::Build {
            file,
            output,
            no_link,
            watch,
            timing,
            verbose,
            opt_level,
            emit,
            profile,
        } => {
            let opts = BuildOptions {
                timing,
                verbose,
                opt_level,
                emit,
                profile,
            };
            if watch {
                run_build_watch(&file, output, no_link, &opts);
            } else {
                match run_build(&file, output, no_link, &opts) {
                    BuildOutcome::Ok(path) => println!("built {}", path.display()),
                    BuildOutcome::Errors(errs) => {
                        for e in &errs {
                            eprintln!("{e}");
                        }
                        std::process::exit(1);
                    }
                }
            }
        }

        Command::Run {
            file,
            timing,
            verbose,
            opt_level,
            emit,
            profile,
        } => {
            let opts = BuildOptions {
                timing,
                verbose,
                opt_level,
                emit,
                profile,
            };
            let exe_path = build_exe(&file, None, &opts);
            let status = std::process::Command::new(&exe_path)
                .status()
                .unwrap_or_else(|e| {
                    eprintln!("run error: {e}");
                    std::process::exit(1);
                });
            std::process::exit(status.code().unwrap_or(0));
        }

        Command::Test {
            file,
            timing,
            verbose,
            opt_level,
        } => {
            let opts = BuildOptions {
                timing,
                verbose,
                opt_level,
                emit: false,
                profile: false,
            };
            let path = file.to_string_lossy().to_string();
            let src = fs::read_to_string(&file).unwrap_or_else(|e| {
                eprintln!("error reading {path}: {e}");
                std::process::exit(1);
            });
            let map = SourceMap::new(&src);
            let tokens = Lexer::new(&src).tokenize().unwrap_or_else(|errors| {
                for e in &errors {
                    let snippet = map.render_diagnostic(&src, e.span(), &path);
                    emit_diagnostic(e.code(), &e.message(), &snippet);
                }
                std::process::exit(1);
            });
            let mut ast = KilnParser::new(tokens).parse_file().unwrap_or_else(|e| {
                let msg = e.message();
                if let Some(span) = e.span() {
                    let snippet = map.render_diagnostic(&src, span, &path);
                    emit_diagnostic(e.code(), &msg, &snippet);
                } else {
                    emit_diagnostic_no_span(e.code(), &msg);
                }
                std::process::exit(1);
            });
            let registry = default_registry();
            run_source_processors(&mut ast, &registry);
            let mut proc_errors: Vec<kiln_compiler::analyzer::AnalysisError> = vec![];
            run_user_processors(&mut ast, &mut proc_errors);
            inject_harness(&mut ast);
            let base_dir = file
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let (mut typed_file, type_registry, run_warnings) =
                analyze_with_base_and_registry(&ast, &base_dir).unwrap_or_else(|errs| {
                    emit_analysis_errors(&errs, &map, &src, &path);
                    std::process::exit(1);
                });
            emit_analysis_errors(&run_warnings, &map, &src, &path);
            run_processors(&ast, &mut typed_file, &registry);
            let module_name = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("module");
            let mut cgx = CodegenContext::new(module_name);
            compile(
                &typed_file,
                &mut cgx,
                opts.verbose,
                opts.opt_level,
                &type_registry,
            )
            .unwrap_or_else(|e| {
                eprintln!("codegen error: {e}");
                std::process::exit(1);
            });
            let obj_bytes = emit::emit_object(cgx).unwrap_or_else(|e| {
                eprintln!("emit error: {e}");
                std::process::exit(1);
            });
            let exe_path = std::env::temp_dir().join(format!(
                "kiln_test_{}.exe",
                file.file_stem().and_then(|s| s.to_str()).unwrap_or("out")
            ));
            let tmp_obj = std::env::temp_dir().join(format!(
                "kiln_test_{}.o",
                file.file_stem().and_then(|s| s.to_str()).unwrap_or("out")
            ));
            emit::link_executable(&obj_bytes, &tmp_obj, &exe_path, false).unwrap_or_else(|e| {
                eprintln!("link error: {e}");
                std::process::exit(1);
            });
            let _ = fs::remove_file(&tmp_obj);
            let status = std::process::Command::new(&exe_path)
                .status()
                .unwrap_or_else(|e| {
                    eprintln!("run error: {e}");
                    std::process::exit(1);
                });
            let _ = fs::remove_file(&exe_path);
            if status.success() {
                println!("all tests passed");
            } else {
                eprintln!("tests failed");
                std::process::exit(status.code().unwrap_or(1));
            }
        }

        Command::Parse { file } => {
            let src = fs::read_to_string(&file).unwrap_or_else(|e| {
                eprintln!("error reading {file}: {e}");
                std::process::exit(1);
            });
            let map = SourceMap::new(&src);

            let tokens = Lexer::new(&src).tokenize().unwrap_or_else(|errors| {
                for e in &errors {
                    let snippet = map.render_diagnostic(&src, e.span(), &file);
                    emit_diagnostic(e.code(), &e.message(), &snippet);
                }
                std::process::exit(1);
            });

            match KilnParser::new(tokens).parse_file() {
                Ok(ast) => println!("{:#?}", ast),
                Err(e) => {
                    let msg = e.message();
                    if let Some(span) = e.span() {
                        let snippet = map.render_diagnostic(&src, span, &file);
                        emit_diagnostic(e.code(), &msg, &snippet);
                    } else {
                        emit_diagnostic_no_span(e.code(), &msg);
                    }
                    std::process::exit(1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_build_valid_file_returns_ok() {
        let file = PathBuf::from("examples/hello.kn");
        let opts = BuildOptions {
            timing: false,
            verbose: false,
            opt_level: 3,
            emit: false,
            profile: false,
        };
        let result = run_build(&file, None, true, &opts);
        assert!(
            matches!(result, BuildOutcome::Ok(_)),
            "expected ok for valid file"
        );
        // Clean up the object file produced by no_link=true.
        let _ = std::fs::remove_file(file.with_extension("o"));
    }

    #[test]
    fn run_build_invalid_file_returns_errors() {
        let file = PathBuf::from("examples/check_undefined.kn");
        let opts = BuildOptions {
            timing: false,
            verbose: false,
            opt_level: 3,
            emit: false,
            profile: false,
        };
        let result = run_build(&file, None, true, &opts);
        assert!(
            matches!(result, BuildOutcome::Errors(_)),
            "expected errors for file with undefined names"
        );
    }

    #[test]
    fn format_build_result_ok_includes_path() {
        let s = format_build_result(
            &BuildOutcome::Ok(PathBuf::from("build/hello.exe")),
            "14:05:01",
        );
        assert!(s.starts_with("[ok] 14:05:01"), "prefix wrong: {s}");
        assert!(s.contains("hello.exe"), "missing path: {s}");
    }

    #[test]
    fn format_build_result_error_includes_timestamp_and_first_error() {
        let errors = vec!["error[E001]: UndefinedName: ghost_value".to_string()];
        let s = format_build_result(&BuildOutcome::Errors(errors), "14:05:09");
        assert!(s.starts_with("[error] 14:05:09"), "prefix wrong: {s}");
        assert!(s.contains("E001"), "missing error code: {s}");
    }

    #[test]
    fn run_check_valid_file_returns_ok() {
        let result = run_check(&PathBuf::from("examples/check_valid.kn"), false, false);
        assert!(
            matches!(result.outcome, CheckOutcome::Ok),
            "expected ok for valid file"
        );
    }

    #[test]
    fn run_check_undefined_name_returns_errors() {
        let result = run_check(&PathBuf::from("examples/check_undefined.kn"), false, false);
        assert!(
            matches!(result.outcome, CheckOutcome::Errors(_)),
            "expected errors for file with undefined names"
        );
    }

    fn key(code: &str, message: &str) -> DiagKey {
        DiagKey {
            code: code.to_string(),
            file: "test.kn".to_string(),
            message: message.to_string(),
        }
    }

    fn keyset(pairs: &[(&str, &str)]) -> std::collections::HashSet<DiagKey> {
        pairs.iter().map(|(c, m)| key(c, m)).collect()
    }

    #[test]
    fn diff_unchanged_when_keys_identical() {
        let prev = keyset(&[("E001", "undefined name `x`"), ("E002", "type mismatch")]);
        let current = prev.clone();
        let fixed: Vec<_> = prev.difference(&current).collect();
        let new: Vec<_> = current.difference(&prev).collect();
        assert!(fixed.is_empty(), "nothing fixed when keys identical");
        assert!(new.is_empty(), "nothing new when keys identical");
        assert_eq!(prev, current, "sets equal means unchanged");
    }

    #[test]
    fn diff_fixed_when_error_disappears() {
        let prev = keyset(&[
            ("E001", "undefined name `x`"),
            ("E003", "cannot assign to `n`"),
        ]);
        let current = keyset(&[("E001", "undefined name `x`")]);
        let fixed: Vec<_> = prev.difference(&current).collect();
        let new: Vec<_> = current.difference(&prev).collect();
        assert_eq!(fixed.len(), 1, "one error fixed");
        assert_eq!(fixed[0].code, "E003");
        assert!(new.is_empty(), "nothing new");
    }

    #[test]
    fn diff_new_when_error_appears() {
        let prev = keyset(&[("E001", "undefined name `x`")]);
        let current = keyset(&[("E001", "undefined name `x`"), ("E002", "type mismatch")]);
        let fixed: Vec<_> = prev.difference(&current).collect();
        let new: Vec<_> = current.difference(&prev).collect();
        assert!(fixed.is_empty(), "nothing fixed");
        assert_eq!(new.len(), 1, "one new error");
        assert_eq!(new[0].code, "E002");
    }

    #[test]
    fn diff_mixed_fixed_and_new() {
        let prev = keyset(&[
            ("E001", "undefined name `x`"),
            ("E003", "cannot assign to `n`"),
        ]);
        let current = keyset(&[("E001", "undefined name `x`"), ("E002", "type mismatch")]);
        let mut fixed: Vec<_> = prev.difference(&current).collect();
        let mut new: Vec<_> = current.difference(&prev).collect();
        fixed.sort_by_key(|k| &k.code);
        new.sort_by_key(|k| &k.code);
        assert_eq!(fixed.len(), 1);
        assert_eq!(fixed[0].code, "E003", "E003 was fixed");
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].code, "E002", "E002 is new");
    }

    #[test]
    fn diff_stable_across_message_not_position() {
        // Same error message = same key even if position changes (line shift scenario).
        let prev = keyset(&[("E001", "undefined name `ghost_value`")]);
        let current = keyset(&[("E001", "undefined name `ghost_value`")]);
        assert_eq!(
            prev, current,
            "same message means same key despite any position shift"
        );
    }

    #[test]
    fn diff_distinct_messages_are_separate_keys() {
        let prev = keyset(&[("E002", "type mismatch: expected `bool`, found `int`")]);
        let current = keyset(&[("E002", "type mismatch: expected `float`, found `bool`")]);
        let fixed: Vec<_> = prev.difference(&current).collect();
        let new: Vec<_> = current.difference(&prev).collect();
        assert_eq!(fixed.len(), 1, "different message = different key");
        assert_eq!(new.len(), 1, "different message = different key");
    }

    #[test]
    fn run_check_produces_diag_keys_for_errors() {
        let result = run_check(&PathBuf::from("examples/check_undefined.kn"), false, false);
        assert!(!result.keys.is_empty(), "expected at least one DiagKey");
        let key = result.keys.iter().next().unwrap();
        assert_eq!(key.code, "E001", "undefined name should produce E001");
        assert!(
            !key.message.is_empty(),
            "key should carry the error message"
        );
    }

    #[test]
    fn format_watch_result_ok_produces_ok_line() {
        let s = format_watch_result(&CheckOutcome::Ok, "14:02:01");
        assert_eq!(s, "[ok] 14:02:01");
    }

    #[test]
    fn format_watch_result_error_includes_timestamp_and_first_error() {
        let errors = vec!["error[E001]: UndefinedName: ghost_value".to_string()];
        let s = format_watch_result(&CheckOutcome::Errors(errors), "14:02:09");
        assert!(s.starts_with("[error] 14:02:09"), "prefix wrong: {s}");
        assert!(s.contains("E001"), "missing error code: {s}");
    }

    #[test]
    fn format_watch_result_error_with_no_errors_vec_shows_unknown() {
        let s = format_watch_result(&CheckOutcome::Errors(vec![]), "00:00:00");
        assert!(s.starts_with("[error]"), "should still be error: {s}");
    }
}
