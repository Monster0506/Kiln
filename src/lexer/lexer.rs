use crate::diagnostics::LexError;
use super::token::{Token, TokenKind};
use super::mode::Mode;

pub struct Lexer<'src> {
    pub(super) src: &'src str,
    pub(super) chars: std::iter::Peekable<std::str::CharIndices<'src>>,
    pub(super) pos: usize,
    pub(super) mode_stack: Vec<Mode>,
}

impl<'src> Lexer<'src> {
    pub fn new(src: &'src str) -> Self {
        Self {
            src,
            chars: src.char_indices().peekable(),
            pos: 0,
            mode_stack: vec![Mode::Normal],
        }
    }

    pub(super) fn peek(&mut self) -> Option<char> {
        self.chars.peek().map(|&(_, c)| c)
    }

    pub(super) fn advance(&mut self) -> Option<char> {
        match self.chars.next() {
            Some((i, c)) => {
                self.pos = i + c.len_utf8();
                Some(c)
            }
            None => None,
        }
    }

    pub(super) fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(super) fn current_mode(&self) -> &Mode {
        self.mode_stack.last().expect("mode stack should never be empty")
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, Vec<LexError>> {
        let mut tokens = Vec::new();
        let mut errors = Vec::new();
        loop {
            match self.next_token() {
                Ok(tok) => {
                    let is_eof = tok.kind == TokenKind::Eof;
                    tokens.push(tok);
                    if is_eof { break; }
                }
                Err(e) => {
                    errors.push(e);
                    break;
                }
            }
        }
        if errors.is_empty() { Ok(tokens) } else { Err(errors) }
    }

    pub fn next_token(&mut self) -> Result<Token, LexError> {
        match self.current_mode().clone() {
            Mode::Normal => self.lex_normal(),
            Mode::String => self.lex_string(),
            Mode::Interp { depth } => self.lex_interp(depth),
        }
    }
}
