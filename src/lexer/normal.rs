use crate::diagnostics::{LexError, Span};
use super::token::{Token, TokenKind};
use super::mode::Mode;
use super::lexer::Lexer;

impl<'src> Lexer<'src> {
    pub(super) fn lex_normal(&mut self) -> Result<Token, LexError> {
        // Skip whitespace and comments.
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => { self.advance(); }
                Some('#') => {
                    while self.peek().map(|c| c != '\n').unwrap_or(false) {
                        self.advance();
                    }
                }
                _ => break,
            }
        }

        let start = self.pos;

        // Identifiers and keywords: start with a letter or '_'.
        if let Some(c) = self.peek() {
            if c.is_alphabetic() || c == '_' {
                while self.peek().map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false) {
                    self.advance();
                }
                let text = &self.src[start..self.pos];
                return Ok(Token::new(keyword_or_ident(text), start, self.pos));
            }
        }

        // Numeric literals (integer and float).
        if let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                let mut raw = String::new();
                while self.peek().map(|c| c.is_ascii_digit() || c == '_').unwrap_or(false) {
                    let ch = self.advance().unwrap();
                    if ch != '_' { raw.push(ch); }
                }
                // Check for a float: `1.5` but not `1.foo`.
                if self.peek() == Some('.') {
                    let after_dot = self.src[self.pos + 1..].chars().next();
                    if after_dot.map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        raw.push('.');
                        self.advance(); // consume '.'
                        while self.peek().map(|c| c.is_ascii_digit() || c == '_').unwrap_or(false) {
                            let ch = self.advance().unwrap();
                            if ch != '_' { raw.push(ch); }
                        }
                        let val: f64 = raw.parse().map_err(|_| LexError::InvalidNumeric {
                            span: Span::new(start, self.pos),
                        })?;
                        return Ok(Token::new(TokenKind::Float(val), start, self.pos));
                    }
                }
                let val: i64 = raw.parse().map_err(|_| LexError::InvalidNumeric {
                    span: Span::new(start, self.pos),
                })?;
                return Ok(Token::new(TokenKind::Int(val), start, self.pos));
            }
        }

        // String literal entry.
        if self.peek() == Some('"') {
            self.advance(); // consume opening `"`
            self.mode_stack.push(Mode::String);
            return Ok(Token::new(TokenKind::StringStart, start, self.pos));
        }

        // Single- and multi-character punctuation and operators.
        match self.advance() {
            None => Ok(Token::new(TokenKind::Eof, start, start)),
            Some('(') => Ok(Token::new(TokenKind::LParen,   start, self.pos)),
            Some(')') => Ok(Token::new(TokenKind::RParen,   start, self.pos)),
            Some('{') => Ok(Token::new(TokenKind::LBrace,   start, self.pos)),
            Some('}') => Ok(Token::new(TokenKind::RBrace,   start, self.pos)),
            Some('[') => Ok(Token::new(TokenKind::LBracket, start, self.pos)),
            Some(']') => Ok(Token::new(TokenKind::RBracket, start, self.pos)),
            Some(',') => Ok(Token::new(TokenKind::Comma,    start, self.pos)),
            Some(':') => Ok(Token::new(TokenKind::Colon,    start, self.pos)),
            Some('.') => Ok(Token::new(TokenKind::Dot,      start, self.pos)),
            Some('+') => Ok(Token::new(TokenKind::Plus,     start, self.pos)),
            Some('/') => Ok(Token::new(TokenKind::Slash,    start, self.pos)),
            Some('?') => Ok(Token::new(TokenKind::Question, start, self.pos)),
            Some('@') => Ok(Token::new(TokenKind::At,       start, self.pos)),
            Some('!') => Ok(Token::new(TokenKind::Bang,     start, self.pos)),
            Some('*') => Ok(Token::new(TokenKind::Star,     start, self.pos)),
            Some('|') => {
                if self.eat('|') {
                    Ok(Token::new(TokenKind::PipePipe, start, self.pos))
                } else {
                    Ok(Token::new(TokenKind::Pipe, start, self.pos))
                }
            }
            Some('-') => {
                if self.eat('>') {
                    Ok(Token::new(TokenKind::Arrow, start, self.pos))
                } else {
                    Ok(Token::new(TokenKind::Minus, start, self.pos))
                }
            }
            Some('=') => {
                if self.eat('>') {
                    Ok(Token::new(TokenKind::FatArrow, start, self.pos))
                } else if self.eat('=') {
                    Ok(Token::new(TokenKind::EqEq, start, self.pos))
                } else {
                    Ok(Token::new(TokenKind::Eq, start, self.pos))
                }
            }
            Some('<') => {
                if self.eat('=') {
                    if self.eat('>') {
                        Ok(Token::new(TokenKind::Spaceship, start, self.pos))
                    } else {
                        Ok(Token::new(TokenKind::LtEq, start, self.pos))
                    }
                } else if self.eat('-') {
                    Ok(Token::new(TokenKind::LArrow, start, self.pos))
                } else {
                    Ok(Token::new(TokenKind::Lt, start, self.pos))
                }
            }
            Some('>') => {
                if self.eat('=') {
                    Ok(Token::new(TokenKind::GtEq, start, self.pos))
                } else {
                    Ok(Token::new(TokenKind::Gt, start, self.pos))
                }
            }
            Some('&') => {
                if self.eat('&') {
                    Ok(Token::new(TokenKind::AmpAmp, start, self.pos))
                } else {
                    Ok(Token::new(TokenKind::Amp, start, self.pos))
                }
            }
            Some(c) => Err(LexError::UnexpectedChar { ch: c, span: Span::new(start, self.pos) }),
        }
    }
}

fn keyword_or_ident(s: &str) -> TokenKind {
    match s {
        "def"        => TokenKind::Def,
        "struct"     => TokenKind::Struct,
        "enum"       => TokenKind::Enum,
        "interface"  => TokenKind::Interface,
        "impl"       => TokenKind::Impl,
        "annotation" => TokenKind::Annotation,
        "processor"  => TokenKind::Processor,
        "type"       => TokenKind::Type,
        "hook"       => TokenKind::Hook,
        "const"      => TokenKind::Const,
        "priv"       => TokenKind::Priv,
        "return"     => TokenKind::Return,
        "raise"      => TokenKind::Raise,
        "spawn"      => TokenKind::Spawn,
        "as"         => TokenKind::As,
        "mut"        => TokenKind::Mut,
        "if"         => TokenKind::If,
        "elif"       => TokenKind::Elif,
        "else"       => TokenKind::Else,
        "while"      => TokenKind::While,
        "do"         => TokenKind::Do,
        "for"        => TokenKind::For,
        "break"      => TokenKind::Break,
        "continue"   => TokenKind::Continue,
        "match"      => TokenKind::Match,
        "try"        => TokenKind::Try,
        "except"     => TokenKind::Except,
        "finally"    => TokenKind::Finally,
        "import"     => TokenKind::Import,
        "export"     => TokenKind::Export,
        "Self"       => TokenKind::Self_,
        "void"       => TokenKind::Void,
        "true"       => TokenKind::True,
        "false"      => TokenKind::False,
        s            => TokenKind::Ident(s.to_string()),
    }
}
