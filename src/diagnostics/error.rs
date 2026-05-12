use super::Span;
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum LexError {
    #[error("unexpected character '{ch}' at offset {span:?}")]
    UnexpectedChar { ch: char, span: Span },

    #[error("unterminated string literal starting at offset {span:?}")]
    UnterminatedString { span: Span },

    #[error("invalid numeric literal at offset {span:?}")]
    InvalidNumeric { span: Span },
}

impl LexError {
    pub fn code(&self) -> &'static str {
        match self {
            LexError::UnexpectedChar { .. } => "L001",
            LexError::UnterminatedString { .. } => "L002",
            LexError::InvalidNumeric { .. } => "L003",
        }
    }

    pub fn kind(&self) -> &'static str {
        "lexical error"
    }

    pub fn message(&self) -> String {
        match self {
            LexError::UnexpectedChar { ch, .. } => format!("unexpected character '{ch}'"),
            LexError::UnterminatedString { .. } => "unterminated string literal".into(),
            LexError::InvalidNumeric { .. } => "invalid numeric literal".into(),
        }
    }

    pub fn span(&self) -> Span {
        match self {
            LexError::UnexpectedChar { span, .. } => *span,
            LexError::UnterminatedString { span } => *span,
            LexError::InvalidNumeric { span } => *span,
        }
    }
}
