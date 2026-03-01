/// Every distinct kind of token in Kiln.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    /// Integer literal, e.g. `42` or `1_000`
    Int(i64),
    /// Float literal, e.g. `3.14` or `1_000.5`
    Float(f64),
    /// Plain text segment inside a string (between interpolations)
    StringText(String),
    /// `true`
    True,
    /// `false`
    False,

    // String interpolation
    /// Opening `"` of a string literal
    StringStart,
    /// Closing `"` of a string literal
    StringEnd,
    /// Opening `{` of an interpolation inside a string
    InterpStart,
    /// Closing `}` of an interpolation inside a string
    InterpEnd,

    // Identifiers
    /// Any identifier that is not a keyword
    Ident(String),

    // Keywords
    Def, Struct, Enum, Interface, Impl, Annotation, Processor,
    Type, Hook, Const, Priv, Return, Raise, Spawn, As, Mut,
    If, Elif, Else, While, Do, For, Break, Continue, Match,
    Import, Export, Self_, Void,

    // Operators
    Plus, Minus, Star, Slash,
    Eq,           // `=`
    EqEq,         // `==`
    Bang,         // `!`
    Lt,           // `<`
    Gt,           // `>`
    LtEq,         // `<=`
    GtEq,         // `>=`
    Spaceship,    // `<=>`
    AmpAmp,       // `&&`
    PipePipe,     // `||`
    Pipe,         // `|`
    Arrow,        // `->`
    FatArrow,     // `=>`
    LArrow,       // `<-`
    Question,     // `?`
    Amp,          // `&`
    At,           // `@`

    // Punctuation
    LParen, RParen,     // `(` `)`
    LBrace, RBrace,     // `{` `}`
    LBracket, RBracket, // `[` `]`
    Comma, Colon, Dot,

    // Special
    Eof,
}

/// A token with its kind and source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: crate::diagnostics::Span,
}

impl Token {
    pub fn new(kind: TokenKind, start: usize, end: usize) -> Self {
        Self { kind, span: crate::diagnostics::Span::new(start, end) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_ident_not_keyword() {
        let t = Token::new(TokenKind::Ident("foo".into()), 0, 3);
        assert_eq!(t.kind, TokenKind::Ident("foo".into()));
        assert_eq!(t.span.len(), 3);
    }

    #[test]
    fn keyword_variants_exist() {
        // Smoke test: ensure key variants compile and are distinct
        assert_ne!(TokenKind::Def, TokenKind::Struct);
        assert_ne!(TokenKind::If, TokenKind::Elif);
    }
}
