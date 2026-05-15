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
