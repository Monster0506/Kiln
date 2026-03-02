use kiln_compiler::lexer::{Lexer, TokenKind};

#[test]
fn empty_source_gives_eof() {
    let tokens = Lexer::new("").tokenize().unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, TokenKind::Eof);
}
