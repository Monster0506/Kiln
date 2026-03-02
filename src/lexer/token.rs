/// Every distinct kind of token in Kiln.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Int(i64),
    Float(f64),
    /// Plain text segment inside a string, between interpolations.
    StringText(String),
    True,
    False,

    // String interpolation
    /// Opening `"` of a string literal.
    StringStart,
    /// Closing `"` of a string literal.
    StringEnd,
    /// Opening `{` of an interpolation expression.
    InterpStart,
    /// Closing `}` of an interpolation expression.
    InterpEnd,

    // Identifiers
    Ident(String),

    // Keywords
    Def, Struct, Enum, Interface, Impl, Annotation, Processor,
    Type, Hook, Const, Priv, Return, Raise, Spawn, As, Mut,
    If, Elif, Else, While, Do, For, Break, Continue, Match,
    Try, Except, Finally,
    Import, Export, Self_, Void,

    // Operators
    Plus, Minus, Star, Slash,
    Eq,        // `=`
    EqEq,      // `==`
    Bang,      // `!`
    Lt,        // `<`
    Gt,        // `>`
    LtEq,      // `<=`
    GtEq,      // `>=`
    Spaceship, // `<=>`
    AmpAmp,    // `&&`
    PipePipe,  // `||`
    Pipe,      // `|`
    Arrow,     // `->`
    FatArrow,  // `=>`
    LArrow,    // `<-`
    Question,  // `?`
    Amp,       // `&`
    At,        // `@`

    // Punctuation
    LParen, RParen,
    LBrace, RBrace,
    LBracket, RBracket,
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
