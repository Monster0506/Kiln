use clap::{Parser, Subcommand};
use kiln_compiler::analyzer::analyze_with_base;
use kiln_compiler::annotations::{default_registry, run_processors, run_user_processors};
use kiln_compiler::codegen::{compile::compile, context::CodegenContext, emit};
use kiln_compiler::diagnostics::timing::{BuildStats, ItemCounts, PhaseTimer};
use kiln_compiler::diagnostics::SourceMap;
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
        /// Print phase timing table to stderr
        #[arg(long)]
        timing: bool,
        /// Print full timing detail to stderr (implies --timing)
        #[arg(long)]
        verbose: bool,
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
    },
}

struct BuildOptions {
    timing: bool,
    verbose: bool,
}

fn build_exe(file: &PathBuf, output: Option<PathBuf>, opts: &BuildOptions) -> PathBuf {
    let timing = opts.timing || opts.verbose;
    let verbose = opts.verbose;

    let path = file.to_string_lossy().to_string();
    let src = fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("error reading {path}: {e}");
        std::process::exit(1);
    });
    let map = SourceMap::new(&src);

    let mut timer = PhaseTimer::new();
    let mut stats = BuildStats {
        source_file: path.clone(),
        source_lines: src.lines().count(),
        ..BuildStats::default()
    };

    timer.start("lex");
    let tokens = Lexer::new(&src).tokenize().unwrap_or_else(|errors| {
        for e in &errors {
            let snippet = map.render_diagnostic(&src, e.span(), &path);
            emit_error(e.kind(), e.code(), &e.message(), &snippet);
        }
        std::process::exit(1);
    });
    timer.stop();
    stats.token_count = tokens.len();

    timer.start("parse");
    let mut ast = KilnParser::new(tokens).parse_file().unwrap_or_else(|e| {
        let msg = e.message();
        if let Some(span) = e.span() {
            let snippet = map.render_diagnostic(&src, span, &path);
            emit_error(e.kind(), e.code(), &msg, &snippet);
        } else {
            emit_error_no_span(e.kind(), e.code(), &msg);
        }
        std::process::exit(1);
    });
    timer.stop();
    stats.ast_node_count = count_ast_nodes(&ast);
    stats.item_counts = count_item_kinds(&ast);

    // Strip @test-annotated functions from production builds.
    ast.items.retain(|item| {
        if let kiln_compiler::parser::ast::Item::Function(f) = item {
            return !f.annotations.iter().any(|a| a.name == "test");
        }
        true
    });

    let registry = default_registry();
    timer.start("processors");
    run_processors(&mut ast, &registry);
    let proc_runs = run_user_processors(&mut ast, &registry);
    timer.stop();
    stats.processor_runs = proc_runs;

    let base_dir = file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    timer.start("analyze");
    let typed_file = analyze_with_base(&ast, &base_dir).unwrap_or_else(|errs| {
        emit_analysis_errors(&errs, &map, &src, &path);
        std::process::exit(1);
    });
    timer.stop();

    let module_name = file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let mut cgx = CodegenContext::new(module_name);

    timer.start("codegen");
    let fn_times = compile(&typed_file, &mut cgx, verbose).unwrap_or_else(|e| {
        eprintln!("codegen error: {e}");
        std::process::exit(1);
    });
    timer.stop();
    if verbose {
        stats.fn_codegen_times = fn_times;
    }

    timer.start("emit");
    let obj_bytes = emit::emit_object(cgx).unwrap_or_else(|e| {
        eprintln!("emit error: {e}");
        std::process::exit(1);
    });
    timer.stop();
    stats.object_bytes = obj_bytes.len();

    let exe_ext = if cfg!(windows) { "exe" } else { "" };
    let exe_path = output.unwrap_or_else(|| file.with_extension(exe_ext));
    let tmp_obj = std::env::temp_dir().join(format!(
        "kiln_{}.o",
        file.file_stem().and_then(|s| s.to_str()).unwrap_or("out")
    ));

    timer.start("link");
    emit::link_executable(&obj_bytes, &tmp_obj, &exe_path, false).unwrap_or_else(|e| {
        eprintln!("link error: {e}");
        std::process::exit(1);
    });
    timer.stop();
    let _ = fs::remove_file(&tmp_obj);

    if let Ok(meta) = fs::metadata(&exe_path) {
        stats.binary_bytes = meta.len() as usize;
    }
    stats.object_path = tmp_obj.to_string_lossy().to_string();
    stats.binary_path = exe_path.to_string_lossy().to_string();

    if timing {
        timer.report(&stats, verbose, &mut std::io::stderr());
    }

    exe_path
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

fn emit_error(kind: &str, code: &str, msg: &str, snippet: &str) {
    eprintln!("error[{code}]: {kind}: {msg}");
    eprintln!("{snippet}");
}

fn emit_error_no_span(kind: &str, code: &str, msg: &str) {
    eprintln!("error[{code}]: {kind}: {msg}");
}

pub enum CheckOutcome {
    Ok,
    Errors(Vec<String>),
}

fn run_check(file: &PathBuf) -> CheckOutcome {
    let path = file.to_string_lossy().to_string();
    let src = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => return CheckOutcome::Errors(vec![format!("error reading {path}: {e}")]),
    };
    let map = SourceMap::new(&src);

    let tokens = match Lexer::new(&src).tokenize() {
        Ok(t) => t,
        Err(errors) => {
            let msgs = errors
                .iter()
                .map(|e| {
                    let snippet = map.render_diagnostic(&src, e.span(), &path);
                    format!(
                        "error[{}]: {}: {}\n{}",
                        e.code(),
                        e.kind(),
                        e.message(),
                        snippet
                    )
                })
                .collect();
            return CheckOutcome::Errors(msgs);
        }
    };

    let mut ast = match KilnParser::new(tokens).parse_file() {
        Ok(a) => a,
        Err(e) => {
            let msg = e.message();
            let formatted = if let Some(span) = e.span() {
                let snippet = map.render_diagnostic(&src, span, &path);
                format!("error[{}]: {}: {}\n{}", e.code(), e.kind(), msg, snippet)
            } else {
                format!("error[{}]: {}: {}", e.code(), e.kind(), msg)
            };
            return CheckOutcome::Errors(vec![formatted]);
        }
    };

    let registry = default_registry();
    run_processors(&mut ast, &registry);
    run_user_processors(&mut ast, &registry);

    let base_dir = file
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    match analyze_with_base(&ast, &base_dir) {
        Ok(_) => CheckOutcome::Ok,
        Err(errs) => {
            let msgs = errs
                .iter()
                .map(|e| {
                    let snippet = map.render_diagnostic(&src, e.span(), &path);
                    format!(
                        "error[{}]: {}: {}\n{}",
                        e.code(),
                        e.kind(),
                        e.message(),
                        snippet
                    )
                })
                .collect();
            CheckOutcome::Errors(msgs)
        }
    }
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

fn run_watch(file: &PathBuf) {
    use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
    use std::sync::mpsc;
    use std::time::Duration;

    let path = file.to_string_lossy().to_string();
    println!("watching {path}");

    let outcome = run_check(file);
    println!("{}", format_watch_result(&outcome, &current_time_str()));
    if let CheckOutcome::Errors(ref errs) = outcome {
        for e in errs {
            println!("{e}");
        }
    }

    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher = RecommendedWatcher::new(tx, Config::default()).unwrap_or_else(|e| {
        eprintln!("watcher setup error: {e}");
        std::process::exit(1);
    });
    watcher
        .watch(file.as_ref(), RecursiveMode::NonRecursive)
        .unwrap_or_else(|e| {
            eprintln!("watch error: {e}");
            std::process::exit(1);
        });

    while rx.recv().is_ok() {
        // Debounce: drain remaining events within 50ms.
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

        let outcome = run_check(file);
        println!("{}", format_watch_result(&outcome, &current_time_str()));
        if let CheckOutcome::Errors(ref errs) = outcome {
            for e in errs {
                println!("{e}");
            }
        }
    }
}

fn emit_analysis_errors(
    errs: &[kiln_compiler::analyzer::AnalysisError],
    map: &SourceMap,
    src: &str,
    path: &str,
) {
    for e in errs {
        let snippet = map.render_diagnostic(src, e.span(), path);
        emit_error(e.kind(), e.code(), &e.message(), &snippet);
        for (note, note_span) in e.note_info() {
            if let Some(ns) = note_span {
                let note_block = map.render_note(src, ns, path, &note);
                eprintln!("{note_block}");
            } else {
                eprintln!("note: {note}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_check_valid_file_returns_ok() {
        let result = run_check(&PathBuf::from("examples/check_valid.kn"));
        assert!(
            matches!(result, CheckOutcome::Ok),
            "expected ok for valid file"
        );
    }

    #[test]
    fn run_check_undefined_name_returns_errors() {
        let result = run_check(&PathBuf::from("examples/check_undefined.kn"));
        assert!(
            matches!(result, CheckOutcome::Errors(_)),
            "expected errors for file with undefined names"
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

fn main() {
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
                        emit_error(e.kind(), e.code(), &e.message(), &snippet);
                    }
                    std::process::exit(1);
                }
            }
        }

        Command::Check { file, watch } => {
            if watch {
                run_watch(&file);
            } else {
                match run_check(&file) {
                    CheckOutcome::Ok => println!("ok"),
                    CheckOutcome::Errors(errs) => {
                        for e in &errs {
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
            timing,
            verbose,
        } => {
            let opts = BuildOptions { timing, verbose };
            if no_link {
                let path = file.to_string_lossy().to_string();
                let src = fs::read_to_string(&file).unwrap_or_else(|e| {
                    eprintln!("error reading {path}: {e}");
                    std::process::exit(1);
                });
                let map = SourceMap::new(&src);
                let tokens = Lexer::new(&src).tokenize().unwrap_or_else(|errors| {
                    for e in &errors {
                        let snippet = map.render_diagnostic(&src, e.span(), &path);
                        emit_error(e.kind(), e.code(), &e.message(), &snippet);
                    }
                    std::process::exit(1);
                });
                let mut ast = KilnParser::new(tokens).parse_file().unwrap_or_else(|e| {
                    let msg = e.message();
                    if let Some(span) = e.span() {
                        let snippet = map.render_diagnostic(&src, span, &path);
                        emit_error(e.kind(), e.code(), &msg, &snippet);
                    } else {
                        emit_error_no_span(e.kind(), e.code(), &msg);
                    }
                    std::process::exit(1);
                });
                let registry = default_registry();
                run_processors(&mut ast, &registry);
                run_user_processors(&mut ast, &registry);
                let base_dir = file
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let typed_file = analyze_with_base(&ast, &base_dir).unwrap_or_else(|errs| {
                    emit_analysis_errors(&errs, &map, &src, &path);
                    std::process::exit(1);
                });
                let module_name = file
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("module");
                let mut cgx = CodegenContext::new(module_name);
                compile(&typed_file, &mut cgx, opts.verbose).unwrap_or_else(|e| {
                    eprintln!("codegen error: {e}");
                    std::process::exit(1);
                });
                let obj_bytes = emit::emit_object(cgx).unwrap_or_else(|e| {
                    eprintln!("emit error: {e}");
                    std::process::exit(1);
                });
                let obj_path = output.unwrap_or_else(|| file.with_extension("o"));
                fs::write(&obj_path, &obj_bytes).unwrap_or_else(|e| {
                    eprintln!("write error: {e}");
                    std::process::exit(1);
                });
                println!("built {}", obj_path.display());
            } else {
                let exe_path = build_exe(&file, output, &opts);
                println!("built {}", exe_path.display());
            }
        }

        Command::Run {
            file,
            timing,
            verbose,
        } => {
            let opts = BuildOptions { timing, verbose };
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
        } => {
            let opts = BuildOptions { timing, verbose };
            let path = file.to_string_lossy().to_string();
            let src = fs::read_to_string(&file).unwrap_or_else(|e| {
                eprintln!("error reading {path}: {e}");
                std::process::exit(1);
            });
            let map = SourceMap::new(&src);
            let tokens = Lexer::new(&src).tokenize().unwrap_or_else(|errors| {
                for e in &errors {
                    let snippet = map.render_diagnostic(&src, e.span(), &path);
                    emit_error(e.kind(), e.code(), &e.message(), &snippet);
                }
                std::process::exit(1);
            });
            let mut ast = KilnParser::new(tokens).parse_file().unwrap_or_else(|e| {
                let msg = e.message();
                if let Some(span) = e.span() {
                    let snippet = map.render_diagnostic(&src, span, &path);
                    emit_error(e.kind(), e.code(), &msg, &snippet);
                } else {
                    emit_error_no_span(e.kind(), e.code(), &msg);
                }
                std::process::exit(1);
            });
            let registry = default_registry();
            run_processors(&mut ast, &registry);
            run_user_processors(&mut ast, &registry);
            inject_harness(&mut ast);
            let base_dir = file
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let typed_file = analyze_with_base(&ast, &base_dir).unwrap_or_else(|errs| {
                emit_analysis_errors(&errs, &map, &src, &path);
                std::process::exit(1);
            });
            let module_name = file
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("module");
            let mut cgx = CodegenContext::new(module_name);
            compile(&typed_file, &mut cgx, opts.verbose).unwrap_or_else(|e| {
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
                    emit_error(e.kind(), e.code(), &e.message(), &snippet);
                }
                std::process::exit(1);
            });

            match KilnParser::new(tokens).parse_file() {
                Ok(ast) => println!("{:#?}", ast),
                Err(e) => {
                    let msg = e.message();
                    if let Some(span) = e.span() {
                        let snippet = map.render_diagnostic(&src, span, &file);
                        emit_error(e.kind(), e.code(), &msg, &snippet);
                    } else {
                        emit_error_no_span(e.kind(), e.code(), &msg);
                    }
                    std::process::exit(1);
                }
            }
        }
    }
}
