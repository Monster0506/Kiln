use kiln_compiler::lexer::{Lexer, TokenKind};

#[test]
fn lex_function_definition() {
    let src = r#"
def add(a: int, b: int) -> int {
  return a + b
}
"#;
    let tokens = Lexer::new(src).tokenize().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| t.kind.clone()).collect();

    assert!(kinds.contains(&TokenKind::Def));
    assert!(kinds.contains(&TokenKind::Ident("add".into())));
    assert!(kinds.contains(&TokenKind::Arrow));
    assert!(kinds.contains(&TokenKind::Ident("int".into())));
    assert!(kinds.contains(&TokenKind::Return));
    assert!(kinds.contains(&TokenKind::Plus));
}

#[test]
fn lex_struct_with_interpolation() {
    let src = r#"struct Point { x: float, y: float }
def to_str() -> str { return "({x}, {y})" }"#;

    let tokens = Lexer::new(src).tokenize().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| t.kind.clone()).collect();

    assert!(kinds.contains(&TokenKind::Struct));
    assert!(kinds.contains(&TokenKind::StringStart));
    assert!(kinds.contains(&TokenKind::InterpStart));
    assert!(kinds.contains(&TokenKind::InterpEnd));
    assert!(kinds.contains(&TokenKind::StringEnd));
}
