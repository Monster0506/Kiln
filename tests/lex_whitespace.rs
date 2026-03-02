use kiln_compiler::lexer::{Lexer, TokenKind};

#[test]
fn whitespace_is_skipped() {
    let tokens = Lexer::new("   \t\n  ").tokenize().unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, TokenKind::Eof);
}

#[test]
fn comment_is_skipped() {
    let tokens = Lexer::new("# this is a comment\n").tokenize().unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, TokenKind::Eof);
}

#[test]
fn comment_without_newline_is_skipped() {
    let tokens = Lexer::new("# just a comment").tokenize().unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, TokenKind::Eof);
}
