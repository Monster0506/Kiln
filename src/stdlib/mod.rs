use crate::lexer::Lexer;
use crate::parser::{ast::SourceFile, Parser};

const PRELUDE_SRC: &str = include_str!("prelude.kn");

/// Parse the stdlib prelude and return its items.
pub fn parse_prelude() -> SourceFile {
    let tokens = Lexer::new(PRELUDE_SRC).tokenize().unwrap_or_else(|e| {
        eprintln!("internal error: prelude lex failed: {e:?}");
        std::process::exit(1);
    });
    Parser::new(tokens).parse_file().unwrap_or_else(|e| {
        eprintln!("internal error: prelude parse failed: {e:?}");
        std::process::exit(1);
    })
}
