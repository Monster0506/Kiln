use kiln_compiler::lexer::{Lexer, TokenKind};

#[test]
fn lex_punctuation() {
    let tokens = Lexer::new("( ) { } [ ] , : .").tokenize().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();
    assert!(kinds.contains(&&TokenKind::LParen));
    assert!(kinds.contains(&&TokenKind::RParen));
    assert!(kinds.contains(&&TokenKind::LBrace));
    assert!(kinds.contains(&&TokenKind::RBrace));
    assert!(kinds.contains(&&TokenKind::LBracket));
    assert!(kinds.contains(&&TokenKind::RBracket));
    assert!(kinds.contains(&&TokenKind::Comma));
    assert!(kinds.contains(&&TokenKind::Colon));
    assert!(kinds.contains(&&TokenKind::Dot));
}
