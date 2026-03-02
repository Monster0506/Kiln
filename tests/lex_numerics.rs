use kiln_compiler::lexer::{Lexer, TokenKind};

#[test]
fn lex_integer() {
    let tokens = Lexer::new("42").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Int(42));
}

#[test]
fn lex_integer_with_underscores() {
    let tokens = Lexer::new("1_000_000").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Int(1_000_000));
}

#[test]
fn lex_zero() {
    let tokens = Lexer::new("0").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Int(0));
}

#[test]
fn lex_float() {
    let tokens = Lexer::new("3.14").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Float(3.14));
}

#[test]
fn lex_float_with_underscores() {
    let tokens = Lexer::new("6.626_070").tokenize().unwrap();
    match &tokens[0].kind {
        TokenKind::Float(f) => assert!((f - 6.626070).abs() < 1e-6),
        other => panic!("expected Float, got {:?}", other),
    }
}

#[test]
fn integer_dot_ident_not_float() {
    // `1.foo` must lex as Int(1), Dot, Ident("foo"), not a float.
    let tokens = Lexer::new("1.foo").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Int(1));
    assert_eq!(tokens[1].kind, TokenKind::Dot);
    assert_eq!(tokens[2].kind, TokenKind::Ident("foo".into()));
}
