use kiln_compiler::lexer::Lexer;
use kiln_compiler::parser::Parser;

fn parse_ok(src: &str) {
    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    Parser::new(tokens)
        .parse_file()
        .unwrap_or_else(|e| panic!("parse error: {e:?}"));
}

// 7: Associated type projection in type position  (I.Item)

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

// 8: Associated type bindings (Name=Type) in generic arg lists

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

// 9: Variance annotations (+T, -T) in generic param lists

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

// 10: Unary + operator

#[test]
fn unary_plus_parses_in_return() {
    parse_ok("def f(x: int) -> int { return +x }");
}

#[test]
fn unary_plus_parses_in_assignment() {
    parse_ok("def f(x: float) -> float { y: float = +x return y }");
}

// 11: @static hooks

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

// Multi-bound generic parameter syntax

#[test]
fn multi_bound_plus_syntax() {
    parse_ok(
        r#"
def sum[T:Addable + Zero](items: Vec[T]) -> T {
    let total: T = T.zero()
    for item <- items {
        total += item
    }
    total
}
"#,
    );
}

#[test]
fn multi_bound_paren_syntax() {
    parse_ok(
        r#"
def sum[T:(Addable, Zero)](items: Vec[T]) -> T {
    let total: T = T.zero()
    for item <- items {
        total += item
    }
    total
}
"#,
    );
}

#[test]
fn multi_bound_paren_single_bound() {
    parse_ok(r#"def foo[T:(Addable)](x: T) -> T { x }"#);
}

#[test]
fn multi_bound_plus_and_paren_yield_same_bounds() {
    use kiln_compiler::parser::ast::{GenericParamKind, Item};

    let src_plus = r#"def f[T:Addable + Zero](x: T) -> T { x }"#;
    let src_paren = r#"def f[T:(Addable, Zero)](x: T) -> T { x }"#;

    let tok_plus = Lexer::new(src_plus).tokenize().unwrap();
    let file_plus = Parser::new(tok_plus).parse_file().unwrap();
    let tok_paren = Lexer::new(src_paren).tokenize().unwrap();
    let file_paren = Parser::new(tok_paren).parse_file().unwrap();

    let bound_names = |file: &kiln_compiler::parser::ast::SourceFile| -> Vec<String> {
        if let Item::Function(f) = &file.items[0] {
            let p = &f.generic_params[0];
            assert_eq!(p.kind, GenericParamKind::Type);
            p.bounds
                .iter()
                .filter_map(|b| {
                    if let kiln_compiler::parser::ast::TypeExpr::Named { name, .. } = b {
                        Some(name.clone())
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            panic!("expected fn def")
        }
    };

    assert_eq!(bound_names(&file_plus), bound_names(&file_paren));
    assert_eq!(bound_names(&file_plus), vec!["Addable", "Zero"]);
}

#[test]
fn static_annotation_sets_is_static_on_hook() {
    use kiln_compiler::lexer::Lexer;
    use kiln_compiler::parser::ast::{ImplBlock, Item};
    use kiln_compiler::parser::Parser;

    let src = r#"impl Zero for int { @static hook zero() -> int {} }"#;
    let tokens = Lexer::new(src).tokenize().unwrap();
    let file = Parser::new(tokens).parse_file().unwrap();
    match &file.items[0] {
        Item::ImplBlock(ImplBlock { hooks, .. }) => {
            assert_eq!(hooks.len(), 1);
            assert!(
                hooks[0].annotations.iter().any(|a| a.name == "static"),
                "expected @static annotation on hook"
            );
        }
        other => panic!("expected impl block, got {other:?}"),
    }
}
