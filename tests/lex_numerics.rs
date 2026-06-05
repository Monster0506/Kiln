use kiln_compiler::diagnostics::LexError;
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
#[allow(clippy::approx_constant)]
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
fn multiple_underscores_between_digits_are_allowed() {
    let tokens = Lexer::new("1__000").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Int(1000));
}

#[test]
fn trailing_underscore_in_integer_is_error() {
    let errs = Lexer::new("1000_").tokenize().unwrap_err();
    assert!(matches!(errs[0], LexError::InvalidNumeric { .. }));
}

#[test]
fn trailing_underscore_in_float_fraction_is_error() {
    let errs = Lexer::new("3.14_").tokenize().unwrap_err();
    assert!(matches!(errs[0], LexError::InvalidNumeric { .. }));
}

#[test]
fn underscore_before_decimal_point_is_error() {
    let errs = Lexer::new("1_.0").tokenize().unwrap_err();
    assert!(matches!(errs[0], LexError::InvalidNumeric { .. }));
}

#[test]
fn integer_dot_ident_not_float() {
    // `1.foo` must lex as Int(1), Dot, Ident("foo"), not a float.
    let tokens = Lexer::new("1.foo").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Int(1));
    assert_eq!(tokens[1].kind, TokenKind::Dot);
    assert_eq!(tokens[2].kind, TokenKind::Ident("foo".into()));
}
