use kiln_compiler::annotations::{default_registry, run_user_processors};
use kiln_compiler::lexer::Lexer;
use kiln_compiler::parser::ast::Item;
use kiln_compiler::parser::Parser;

fn parse_and_run(src: &str) -> kiln_compiler::parser::ast::SourceFile {
    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    let mut ast = Parser::new(tokens).parse_file().expect("parse failed");
    let registry = default_registry();
    run_user_processors(&mut ast, &registry);
    ast
}

fn fn_names(ast: &kiln_compiler::parser::ast::SourceFile) -> Vec<String> {
    ast.items
        .iter()
        .filter_map(|i| {
            if let Item::Function(f) = i {
                Some(f.name.clone())
            } else {
                None
            }
        })
        .collect()
}

fn fn_body_stmt_count(ast: &kiln_compiler::parser::ast::SourceFile, name: &str) -> usize {
    ast.items
        .iter()
        .find_map(|i| {
            if let Item::Function(f) = i {
                if f.name == name {
                    return Some(f.body.stmts.len());
                }
            }
            None
        })
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Noop processor: returns (None, []) -- keeps original, emits nothing
// ---------------------------------------------------------------------------

#[test]
fn noop_processor_leaves_source_unchanged() {
    let src = r#"
annotation Noop { }
processor Noop(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    return (None, Vec.new())
}
@Noop
def greet() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    let names = fn_names(&ast);
    assert!(
        names.contains(&"greet".to_string()),
        "greet must still exist: {names:?}"
    );
    assert!(
        names.contains(&"main".to_string()),
        "main must still exist: {names:?}"
    );
    assert_eq!(names.len(), 2, "no extra items emitted: {names:?}");
}

// ---------------------------------------------------------------------------
// Body-wrapping processor: replaces body with gen { <<target.body>> }
// (effectively a no-op transform, but exercises gen splice)
// ---------------------------------------------------------------------------

#[test]
fn wrap_processor_preserves_body_statement_count() {
    let src = r#"
annotation Wrap { }
processor Wrap(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@Wrap
def work() -> void {
    x: int = 1
    y: int = 2
}
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    // The "work" function had 2 statements; after wrapping with gen { <<target.body>> }
    // it should still have 2 statements (body is preserved via splice).
    let count = fn_body_stmt_count(&ast, "work");
    assert_eq!(
        count, 2,
        "wrapped body must still have 2 stmts, got {count}"
    );
}

// ---------------------------------------------------------------------------
// Prepend processor: adds a statement before the original body
// ---------------------------------------------------------------------------

#[test]
fn prepend_processor_adds_statement_to_body() {
    let src = r#"
annotation LogEnter { }
processor LogEnter(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        sentinel: int = 0
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@LogEnter
def action() -> void {
    x: int = 42
}
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    // "action" had 1 statement; after prepending sentinel it should have 2.
    let count = fn_body_stmt_count(&ast, "action");
    assert_eq!(count, 2, "prepend processor must add 1 stmt, got {count}");
}
