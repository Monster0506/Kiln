use kiln_compiler::lexer::{Lexer, TokenKind};

#[test]
fn lex_interpolated_string() {
    // "hi {name}!" -> StringStart, StringText("hi "), InterpStart,
    //                 Ident("name"), InterpEnd, StringText("!"), StringEnd
    let tokens = Lexer::new("\"hi {name}!\"").tokenize().unwrap();
    assert_eq!(tokens[0].kind, TokenKind::StringStart);
    assert_eq!(tokens[1].kind, TokenKind::StringText("hi ".into()));
    assert_eq!(tokens[2].kind, TokenKind::InterpStart);
    assert_eq!(tokens[3].kind, TokenKind::Ident("name".into()));
    assert_eq!(tokens[4].kind, TokenKind::InterpEnd);
    assert_eq!(tokens[5].kind, TokenKind::StringText("!".into()));
    assert_eq!(tokens[6].kind, TokenKind::StringEnd);
}

#[test]
fn lex_escaped_brace() {
    // "a\{b" -> StringStart, StringText("a{b"), StringEnd
    let tokens = Lexer::new("\"a\\{b\"").tokenize().unwrap();
    assert_eq!(tokens[1].kind, TokenKind::StringText("a{b".into()));
}

#[test]
fn lex_nested_braces_in_interp() {
    // Inner braces are depth-tracked; verify no error.
    let result = Lexer::new("\"{Point { x: 1 }}\"").tokenize();
    assert!(result.is_ok());
}
