use kiln_compiler::diagnostics::Span;
use kiln_compiler::lexer::{Lexer, TokenKind};

#[test]
fn lex_identifier() {
    let tokens = Lexer::new("hello").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Ident("hello".into()));
}

#[test]
fn lex_keyword_def() {
    let tokens = Lexer::new("def").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Def);
}

#[test]
fn lex_keyword_true_false() {
    let tokens = Lexer::new("true false").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::True);
    assert_eq!(tokens[1].kind, TokenKind::False);
}

#[test]
fn ident_with_underscore() {
    let tokens = Lexer::new("my_var").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Ident("my_var".into()));
}

#[test]
fn ident_span_is_correct() {
    let tokens = Lexer::new("  foo  ").tokenize().unwrap();
    assert_eq!(tokens[0].span, Span::new(2, 5));
}
