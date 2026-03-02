use kiln_compiler::lexer::{Lexer, TokenKind};

#[test]
fn lex_simple_string() {
    // "hello" -> StringStart, StringText("hello"), StringEnd, Eof
    let tokens = Lexer::new("\"hello\"").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::StringStart);
    assert_eq!(tokens[1].kind, TokenKind::StringText("hello".into()));
    assert_eq!(tokens[2].kind, TokenKind::StringEnd);
}

#[test]
fn lex_empty_string() {
    let tokens = Lexer::new("\"\"").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::StringStart);
    assert_eq!(tokens[1].kind, TokenKind::StringEnd);
}

#[test]
fn lex_unterminated_string() {
    let result = Lexer::new("\"hello").tokenize();
    assert!(result.is_err());
}
