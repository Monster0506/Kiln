use clap::{Parser, Subcommand};
use kiln_compiler::lexer::Lexer;
use kiln_compiler::parser::{ParseError, Parser as KilnParser};
use std::fs;

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
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Lex { file } => {
            let src = fs::read_to_string(&file).unwrap_or_else(|e| {
                eprintln!("error reading {file}: {e}");
                std::process::exit(1);
            });

            match Lexer::new(&src).tokenize() {
                Ok(tokens) => {
                    for tok in &tokens {
                        println!("{:?}", tok);
                    }
                }
                Err(errors) => {
                    for e in &errors {
                        eprintln!("lex error: {e}");
                    }
                    std::process::exit(1);
                }
            }
        }
        Command::Parse { file } => {
            let src = fs::read_to_string(&file).unwrap_or_else(|e| {
                eprintln!("error reading {file}: {e}");
                std::process::exit(1);
            });

            let tokens = Lexer::new(&src).tokenize().unwrap_or_else(|errors| {
                for e in &errors {
                    eprintln!("lex error: {e}");
                }
                std::process::exit(1);
            });

            match KilnParser::new(tokens).parse_file() {
                Ok(ast) => println!("{:#?}", ast),
                Err(ParseError::Unexpected { found, expected, span }) => {
                    eprintln!("parse error at {:?}: expected {expected}, found {found:?}", span);
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("parse error: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}
