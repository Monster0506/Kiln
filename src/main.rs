use clap::{Parser, Subcommand};
use kiln_compiler::analyzer::analyze;
use kiln_compiler::parse_prelude;
use kiln_compiler::codegen::{compile::compile, context::CodegenContext, emit};
use kiln_compiler::diagnostics::SourceMap;
use kiln_compiler::lexer::Lexer;
use kiln_compiler::parser::Parser as KilnParser;
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
        /// Show linker stderr output
        #[arg(long)]
        verbose: bool,
    },
    /// Compile and run a source file
    Run {
        /// Path to the .kn source file
        file: PathBuf,
        /// Show linker stderr output
        #[arg(long)]
        verbose: bool,
    },
}

fn build_exe(file: &PathBuf, output: Option<PathBuf>, verbose: bool) -> PathBuf {
    let path = file.to_string_lossy().to_string();
    let src = fs::read_to_string(file).unwrap_or_else(|e| {
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

    let ast = KilnParser::new(tokens).parse_file().unwrap_or_else(|e| {
        let msg = e.message();
        if let Some(span) = e.span() {
            let snippet = map.render_diagnostic(&src, span, &path);
            emit_error(e.kind(), e.code(), &msg, &snippet);
        } else {
            emit_error_no_span(e.kind(), e.code(), &msg);
        }
        std::process::exit(1);
    });

    let mut ast = ast;
    let prelude = parse_prelude();
    ast.items = prelude.items.into_iter().chain(ast.items).collect();

    let typed_file = analyze(&ast).unwrap_or_else(|errs| {
        for e in &errs {
            let snippet = map.render_diagnostic(&src, e.span(), &path);
            emit_error(e.kind(), e.code(), &e.message(), &snippet);
        }
        std::process::exit(1);
    });

    let module_name = file.file_stem().and_then(|s| s.to_str()).unwrap_or("module");
    let mut cgx = CodegenContext::new(module_name);
    compile(&typed_file, &mut cgx).unwrap_or_else(|e| {
        eprintln!("codegen error: {e}");
        std::process::exit(1);
    });

    let obj_bytes = emit::emit_object(cgx).unwrap_or_else(|e| {
        eprintln!("emit error: {e}");
        std::process::exit(1);
    });

    let exe_ext = if cfg!(windows) { "exe" } else { "" };
    let exe_path = output.unwrap_or_else(|| file.with_extension(exe_ext));
    let tmp_obj = std::env::temp_dir()
        .join(format!("kiln_{}.o", file.file_stem().and_then(|s| s.to_str()).unwrap_or("out")));
    emit::link_executable(&obj_bytes, &tmp_obj, &exe_path, verbose).unwrap_or_else(|e| {
        eprintln!("link error: {e}");
        std::process::exit(1);
    });
    let _ = fs::remove_file(&tmp_obj);
    exe_path
}

fn emit_error(kind: &str, code: &str, msg: &str, snippet: &str) {
    eprintln!("error[{code}]: {kind}: {msg}");
    eprintln!("{snippet}");
}

fn emit_error_no_span(kind: &str, code: &str, msg: &str) {
    eprintln!("error[{code}]: {kind}: {msg}");
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

        Command::Check { file } => {
            let path = file.to_string_lossy();
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

            let ast = KilnParser::new(tokens).parse_file().unwrap_or_else(|e| {
                let msg = e.message();
                if let Some(span) = e.span() {
                    let snippet = map.render_diagnostic(&src, span, &path);
                    emit_error(e.kind(), e.code(), &msg, &snippet);
                } else {
                    emit_error_no_span(e.kind(), e.code(), &msg);
                }
                std::process::exit(1);
            });

            let mut ast = ast;
            let prelude = parse_prelude();
            ast.items = prelude.items.into_iter().chain(ast.items).collect();

            match kiln_compiler::analyzer::analyze(&ast) {
                Ok(_) => println!("ok"),
                Err(errs) => {
                    for e in &errs {
                        let snippet = map.render_diagnostic(&src, e.span(), &path);
                        emit_error(e.kind(), e.code(), &e.message(), &snippet);
                    }
                    std::process::exit(1);
                }
            }
        }

        Command::Build { file, output, no_link, verbose } => {
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
                let ast = KilnParser::new(tokens).parse_file().unwrap_or_else(|e| {
                    let msg = e.message();
                    if let Some(span) = e.span() {
                        let snippet = map.render_diagnostic(&src, span, &path);
                        emit_error(e.kind(), e.code(), &msg, &snippet);
                    } else {
                        emit_error_no_span(e.kind(), e.code(), &msg);
                    }
                    std::process::exit(1);
                });
                let mut ast = ast;
                let prelude = parse_prelude();
                ast.items = prelude.items.into_iter().chain(ast.items).collect();
                let typed_file = analyze(&ast).unwrap_or_else(|errs| {
                    for e in &errs {
                        let snippet = map.render_diagnostic(&src, e.span(), &path);
                        emit_error(e.kind(), e.code(), &e.message(), &snippet);
                    }
                    std::process::exit(1);
                });
                let module_name = file.file_stem().and_then(|s| s.to_str()).unwrap_or("module");
                let mut cgx = CodegenContext::new(module_name);
                compile(&typed_file, &mut cgx).unwrap_or_else(|e| {
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
                let exe_path = build_exe(&file, output, verbose);
                println!("built {}", exe_path.display());
            }
        }

        Command::Run { file, verbose } => {
            let exe_path = build_exe(&file, None, verbose);
            let status = std::process::Command::new(&exe_path)
                .status()
                .unwrap_or_else(|e| {
                    eprintln!("run error: {e}");
                    std::process::exit(1);
                });
            std::process::exit(status.code().unwrap_or(0));
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
