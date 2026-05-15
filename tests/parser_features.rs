use kiln_compiler::lexer::Lexer;
use kiln_compiler::parser::Parser;

fn parse_ok(src: &str) {
    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    Parser::new(tokens)
        .parse_file()
        .unwrap_or_else(|e| panic!("parse error: {e:?}"));
}

fn parse_fails(src: &str) -> bool {
    let tokens = match Lexer::new(src).tokenize() {
        Ok(t) => t,
        Err(_) => return true,
    };
    Parser::new(tokens).parse_file().is_err()
}

// ---------------------------------------------------------------------------
// 7: Associated type projection in type position  (I.Item)
// ---------------------------------------------------------------------------

#[test]
fn assoc_type_projection_parses_in_return_position() {
    parse_ok(
        r#"
interface Container {
    type Item
    def get() -> Self.Item {}
}
"#,
    );
}

#[test]
fn assoc_type_projection_parses_as_generic_arg() {
    parse_ok(
        r#"
interface Iter {
    type Item
    def collect() -> Vec[Self.Item] {}
}
"#,
    );
}

// ---------------------------------------------------------------------------
// 8: Associated type bindings (Name=Type) in generic arg lists
// ---------------------------------------------------------------------------

#[test]
fn assoc_type_binding_parses_in_extends() {
    parse_ok(
        r#"
interface Addable: AddableWith[Self, Output=Self] {}
"#,
    );
}

#[test]
fn assoc_type_binding_parses_mixed_with_positional() {
    parse_ok(
        r#"
interface Foo: SomeTrait[int, Output=str] {}
"#,
    );
}

// ---------------------------------------------------------------------------
// 9: Variance annotations (+T, -T) in generic param lists
// ---------------------------------------------------------------------------

#[test]
fn covariant_param_parses() {
    parse_ok(
        r#"
struct ReadOnly[+T] { val: T }
"#,
    );
}

#[test]
fn contravariant_param_parses() {
    parse_ok(
        r#"
struct Writer[-T] { val: T }
"#,
    );
}

#[test]
fn mixed_variance_params_parse() {
    parse_ok(
        r#"
struct Func[-In, +Out] { val: int }
"#,
    );
}

#[test]
fn variance_with_bound_parses() {
    parse_ok(
        r#"
struct Ordered[+T: Comparable] { val: T }
"#,
    );
}

// ---------------------------------------------------------------------------
// 10: Unary + operator
// ---------------------------------------------------------------------------

#[test]
fn unary_plus_parses_in_return() {
    parse_ok("def f(x: int) -> int { return +x }");
}

#[test]
fn unary_plus_parses_in_assignment() {
    parse_ok("def f(x: float) -> float { y: float = +x return y }");
}

// ---------------------------------------------------------------------------
// 11: @static hooks
// ---------------------------------------------------------------------------

#[test]
fn static_hook_parses_in_impl_block() {
    parse_ok(
        r#"
impl Zero for int {
    @static
    hook zero() -> int {}
}
"#,
    );
}

#[test]
fn static_hook_parses_in_interface() {
    parse_ok(
        r#"
interface Zero {
    @static
    hook zero() -> Self
}
"#,
    );
}

#[test]
fn static_annotation_sets_is_static_on_hook() {
    use kiln_compiler::lexer::Lexer;
    use kiln_compiler::parser::ast::{HookName, Item, ImplBlock};
    use kiln_compiler::parser::Parser;

    let src = r#"impl Zero for int { @static hook zero() -> int {} }"#;
    let tokens = Lexer::new(src).tokenize().unwrap();
    let file = Parser::new(tokens).parse_file().unwrap();
    match &file.items[0] {
        Item::ImplBlock(ImplBlock { hooks, .. }) => {
            assert_eq!(hooks.len(), 1);
            assert!(hooks[0].annotations.iter().any(|a| a.name == "static"),
                "expected @static annotation on hook");
        }
        other => panic!("expected impl block, got {other:?}"),
    }
}
