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
