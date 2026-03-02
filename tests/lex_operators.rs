use kiln_compiler::lexer::{Lexer, TokenKind};

#[test]
fn lex_arrow() {
    let tokens = Lexer::new("->").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Arrow);
}

#[test]
fn lex_fat_arrow() {
    let tokens = Lexer::new("=>").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::FatArrow);
}

#[test]
fn lex_larrow() {
    let tokens = Lexer::new("<-").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::LArrow);
}

#[test]
fn lex_spaceship() {
    let tokens = Lexer::new("<=>").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Spaceship);
}

#[test]
fn lex_lteq_vs_spaceship() {
    let tokens = Lexer::new("<= <=>").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::LtEq);
    assert_eq!(tokens[1].kind, TokenKind::Spaceship);
}

#[test]
fn lex_amp_amp() {
    let tokens = Lexer::new("&&").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::AmpAmp);
}

#[test]
fn lex_amp_single() {
    let tokens = Lexer::new("& ").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::Amp);
}
