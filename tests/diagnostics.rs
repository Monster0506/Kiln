use kiln_compiler::diagnostics::{Span, LexError};

#[test]
fn span_len() {
    let s = Span::new(4, 9);
    assert_eq!(s.len(), 5);
}

#[test]
fn span_empty() {
    assert!(Span::new(3, 3).is_empty());
    assert!(!Span::new(3, 4).is_empty());
}

#[test]
fn lex_error_display_contains_char() {
    let e = LexError::UnexpectedChar { ch: '$', span: Span::new(5, 6) };
    assert!(e.to_string().contains('$'));
}
