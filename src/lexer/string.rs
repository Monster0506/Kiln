use super::lexer::Lexer;
use super::mode::Mode;
use super::token::{Token, TokenKind};
use crate::diagnostics::{LexError, Span};

impl<'src> Lexer<'src> {
    pub(super) fn lex_string(&mut self) -> Result<Token, LexError> {
        let start = self.pos;
        let mut text = String::new();

        loop {
            match self.peek() {
                None => {
                    return Err(LexError::UnterminatedString {
                        span: Span::new(start, self.pos),
                    });
                }
                Some('"') => {
                    if !text.is_empty() {
                        return Ok(Token::new(TokenKind::StringText(text), start, self.pos));
                    }
                    self.advance(); // consume closing `"`
                    self.mode_stack.pop();
                    return Ok(Token::new(TokenKind::StringEnd, start, self.pos));
                }
                Some('{') => {
                    if !text.is_empty() {
                        return Ok(Token::new(TokenKind::StringText(text), start, self.pos));
                    }
                    let brace_start = self.pos;
                    self.advance(); // consume `{`
                    self.mode_stack.pop();
                    self.mode_stack.push(Mode::Interp { depth: 1 });
                    return Ok(Token::new(TokenKind::InterpStart, brace_start, self.pos));
                }
                Some('\\') => {
                    self.advance(); // consume `\`
                    match self.peek() {
                        Some('{') => {
                            self.advance();
                            text.push('{');
                        }
                        Some('}') => {
                            self.advance();
                            text.push('}');
                        }
                        Some('n') => {
                            self.advance();
                            text.push('\n');
                        }
                        Some('t') => {
                            self.advance();
                            text.push('\t');
                        }
                        Some('"') => {
                            self.advance();
                            text.push('"');
                        }
                        Some('\\') => {
                            self.advance();
                            text.push('\\');
                        }
                        Some(c) => {
                            text.push('\\');
                            text.push(c);
                            self.advance();
                        }
                        None => {}
                    }
                }
                Some(c) => {
                    text.push(c);
                    self.advance();
                }
            }
        }
    }

    pub(super) fn lex_interp(&mut self, depth: usize) -> Result<Token, LexError> {
        // Skip whitespace inside the interpolation.
        while self.peek().map(|c| c.is_whitespace()).unwrap_or(false) {
            self.advance();
        }

        let start = self.pos;

        match self.peek() {
            None => Ok(Token::new(TokenKind::Eof, start, start)),
            Some('{') => {
                self.advance();
                *self.mode_stack.last_mut().unwrap() = Mode::Interp { depth: depth + 1 };
                Ok(Token::new(TokenKind::LBrace, start, self.pos))
            }
            Some('}') => {
                self.advance();
                if depth <= 1 {
                    self.mode_stack.pop();
                    self.mode_stack.push(Mode::String);
                    Ok(Token::new(TokenKind::InterpEnd, start, self.pos))
                } else {
                    *self.mode_stack.last_mut().unwrap() = Mode::Interp { depth: depth - 1 };
                    Ok(Token::new(TokenKind::RBrace, start, self.pos))
                }
            }
            // Delegate all other tokens to normal lexing.
            _ => {
                let saved_mode = self.mode_stack.last().cloned().unwrap();
                *self.mode_stack.last_mut().unwrap() = Mode::Normal;
                let tok = self.lex_normal()?;
                *self.mode_stack.last_mut().unwrap() = saved_mode;
                Ok(tok)
            }
        }
    }
}
