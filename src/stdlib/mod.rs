use crate::lexer::Lexer;
use crate::parser::{ast::SourceFile, Parser};
use std::sync::OnceLock;

const PRELUDE_SRC: &str = include_str!("prelude.kn");
const AST_SRC: &str = include_str!("ast.kn");
const BUILTINS_SRC: &str = include_str!("builtins.kn");
const INTERFACES_SRC: &str = include_str!("interfaces.kn");
const IMPLS_SRC: &str = include_str!("impls.kn");
const FUNCTIONS_SRC: &str = include_str!("functions.kn");

fn parse_src(src: &str, label: &str) -> SourceFile {
    let tokens = Lexer::new(src).tokenize().unwrap_or_else(|e| {
        eprintln!("internal error: {label} lex failed: {e:?}");
        std::process::exit(1);
    });
    Parser::new(tokens).parse_file().unwrap_or_else(|e| {
        eprintln!("internal error: {label} parse failed: {e:?}");
        std::process::exit(1);
    })
}

static PRELUDE_CACHE: OnceLock<SourceFile> = OnceLock::new();
static AST_STDLIB_CACHE: OnceLock<SourceFile> = OnceLock::new();

/// Virtual filesystem for embedded stdlib modules, used by the import resolver
/// so that prelude.kn can import builtins/interfaces/impls/functions naturally.
pub fn stdlib_virtual_fs() -> std::collections::HashMap<String, String> {
    [
        ("builtins", BUILTINS_SRC),
        ("interfaces", INTERFACES_SRC),
        ("impls", IMPLS_SRC),
        ("functions", FUNCTIONS_SRC),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// Parse prelude.kn (which imports its sections via normal import statements).
/// The result is cached globally -- prelude source never changes at runtime.
/// The caller is responsible for resolving those imports via stdlib_virtual_fs().
pub fn parse_prelude() -> SourceFile {
    PRELUDE_CACHE
        .get_or_init(|| parse_src(PRELUDE_SRC, "prelude.kn"))
        .clone()
}

/// Parse the AST type declarations (FnDecl, Block, Decl, etc.) and return their items.
/// The result is cached globally.
pub fn parse_ast_stdlib() -> SourceFile {
    AST_STDLIB_CACHE
        .get_or_init(|| parse_src(AST_SRC, "ast.kn"))
        .clone()
}
