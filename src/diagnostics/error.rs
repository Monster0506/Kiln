use thiserror::Error;
use super::Span;

#[derive(Debug, Error, PartialEq)]
pub enum LexError {
    #[error("unexpected character '{ch}' at offset {span:?}")]
    UnexpectedChar { ch: char, span: Span },

    #[error("unterminated string literal starting at offset {span:?}")]
    UnterminatedString { span: Span },

    #[error("invalid numeric literal at offset {span:?}")]
    InvalidNumeric { span: Span },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lex_error_display() {
        let e = LexError::UnexpectedChar {
            ch: '$',
            span: Span::new(5, 6),
        };
        let msg = e.to_string();
        assert!(msg.contains('$'));
    }
}
