use crate::diagnostics::Span;

// -- Annotations --------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AnnotationUse {
    pub name: String,
    pub args: Vec<(String, Expr)>,
    pub span: Span,
}

// -- Type Expressions ---------------------------------------------------------

#[derive(Debug, Clone)]
pub enum TypeExpr {
    /// `int`, `str`, `MyStruct`, `Vec[T]`
    Named {
        name: String,
        generics: Vec<TypeExpr>,
        span: Span,
    },
    /// `A | B`
    Union(Vec<TypeExpr>, Span),
    /// `(A, B, C)`
    Tuple(Vec<TypeExpr>, Span),
    /// `Callable[(A, B), C]`
    Callable {
        params: Vec<TypeExpr>,
        ret: Box<TypeExpr>,
        span: Span,
    },
    /// `&T` or `&mut T`
    Ref {
        mutable: bool,
        inner: Box<TypeExpr>,
        span: Span,
    },
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Named { span, .. } => *span,
            TypeExpr::Union(_, span) => *span,
            TypeExpr::Tuple(_, span) => *span,
            TypeExpr::Callable { span, .. } => *span,
            TypeExpr::Ref { span, .. } => *span,
        }
    }
}

// -- Generic Parameters -------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GenericParam {
    pub name: String,
    /// Optional bound: `T: Comparable`
    pub bound: Option<String>,
    pub span: Span,
}

// -- Items (top-level declarations) ------------------------------------------

#[derive(Debug, Clone)]
pub enum Item {
    Function(FnDef),
    Struct(StructDef),
    Enum(EnumDef),
    Interface(InterfaceDef),
    ImplBlock(ImplBlock),
    AnnotationDef(AnnotationDef),
    ProcessorDef(ProcessorDef),
    TypeAlias(TypeAlias),
    Const(ConstDef),
    Import(Import),
    Export(Export),
}

#[derive(Debug, Clone)]
pub struct FnDef {
    pub annotations: Vec<AnnotationUse>,
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub variadic: Option<VariadicParam>,
    pub return_type: TypeExpr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub name: String,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct VariadicParam {
    pub name: String,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub annotations: Vec<AnnotationUse>,
    pub is_priv: bool,
    pub name: String,
    pub ty: TypeExpr,
    pub default: Option<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StructDef {
    pub annotations: Vec<AnnotationUse>,
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    pub interfaces: Vec<TypeExpr>,
    pub fields: Vec<Field>,
    pub methods: Vec<FnDef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Vec<Field>,
    pub discriminant: Option<i64>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumDef {
    pub annotations: Vec<AnnotationUse>,
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    pub interfaces: Vec<TypeExpr>,
    pub variants: Vec<EnumVariant>,
    pub methods: Vec<FnDef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct InterfaceItem {
    pub kind: InterfaceItemKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum InterfaceItemKind {
    /// Required field: `message: str`
    Field { name: String, ty: TypeExpr },
    /// Hook signature or with default body
    Hook {
        name: HookName,
        params: Vec<Param>,
        return_type: TypeExpr,
        default: Option<Block>,
    },
    /// Regular method with optional default body
    Method(FnDef),
}

/// The name of a hook, either a symbol or an identifier.
#[derive(Debug, Clone, PartialEq)]
pub enum HookName {
    Op(String),    // "+", "-", "==", "<=>", "[]", etc.
    Named(String), // "iter", "to_str", "drop", "next"
}

#[derive(Debug, Clone)]
pub struct InterfaceDef {
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    pub extends: Vec<TypeExpr>,
    pub items: Vec<InterfaceItem>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ImplBlock {
    pub interface: TypeExpr,
    pub for_type: TypeExpr,
    pub methods: Vec<FnDef>,
    pub hooks: Vec<HookDef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct HookDef {
    pub name: HookName,
    pub params: Vec<Param>,
    pub return_type: TypeExpr,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct AnnotationDef {
    pub name: String,
    pub fields: Vec<Field>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ProcessorDef {
    pub annotation_name: String,
    pub target_param: Param,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeAlias {
    pub name: String,
    pub generic_params: Vec<GenericParam>,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ConstDef {
    pub name: String,
    pub ty: TypeExpr,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub path: Vec<String>,
    pub symbols: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Export {
    pub symbols: Vec<String>,
    pub span: Span,
}

// -- Statements ---------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    /// `x: int = 5`
    VarDecl {
        name: String,
        ty: TypeExpr,
        value: Expr,
        span: Span,
    },
    /// `x = 5`
    Assign {
        target: Expr,
        value: Expr,
        span: Span,
    },
    /// `return expr`
    Return { value: Option<Expr>, span: Span },
    /// `raise expr`
    Raise { value: Expr, span: Span },
    /// `break`
    Break(Span),
    /// `continue`
    Continue(Span),
    /// `if/elif/else`
    If {
        branches: Vec<(Expr, Block)>,
        else_branch: Option<Block>,
        span: Span,
    },
    /// `while cond { body }`
    While { cond: Expr, body: Block, span: Span },
    /// `do { body } while cond`
    DoWhile { body: Block, cond: Expr, span: Span },
    /// `for x <- iterable { body }` or `for x: T <- iterable { body }`
    For {
        binding: String,
        binding_ty: Option<TypeExpr>,
        iterable: Expr,
        body: Block,
        span: Span,
    },
    /// `try { } except T as e { } finally { }`
    TryCatch {
        body: Block,
        handlers: Vec<CatchHandler>,
        finally: Option<Block>,
        span: Span,
    },
    /// Nested function definition
    FnDef(FnDef),
    /// Expression used as statement
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct CatchHandler {
    pub ty: TypeExpr,
    pub binding: String,
    pub body: Block,
    pub span: Span,
}

// -- Expressions --------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum Expr {
    Int(i64, Span),
    Float(f64, Span),
    Bool(bool, Span),
    /// Interpolated string: sequence of text and expression segments
    Str(Vec<StringSegment>, Span),
    Ident(String, Span),
    /// `(a, b, c)`
    Tuple(Vec<Expr>, Span),
    /// `Type { field: value, ... }`
    StructLiteral {
        ty: String,
        fields: Vec<(String, Expr)>,
        span: Span,
    },
    /// `f(a, b)` or `obj.method(a, b)`
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    /// `obj.field`
    Field {
        object: Box<Expr>,
        field: String,
        span: Span,
    },
    /// `obj[index]`
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    /// `a + b`, `a == b`, etc.
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    /// `-a`, `!a`
    UnOp {
        op: UnOp,
        operand: Box<Expr>,
        span: Span,
    },
    /// `expr?`
    Unwrap(Box<Expr>, Span),
    /// `expr as Type`
    As {
        expr: Box<Expr>,
        ty: TypeExpr,
        span: Span,
    },
    /// `match expr { arms }`
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
        span: Span,
    },
    /// `(params) -> body`
    Closure {
        params: Vec<Param>,
        body: ClosureBody,
        span: Span,
    },
    /// `spawn expr`
    Spawn(Box<Expr>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Int(_, s) => *s,
            Expr::Float(_, s) => *s,
            Expr::Bool(_, s) => *s,
            Expr::Str(_, s) => *s,
            Expr::Ident(_, s) => *s,
            Expr::Tuple(_, s) => *s,
            Expr::StructLiteral { span, .. } => *span,
            Expr::Call { span, .. } => *span,
            Expr::Field { span, .. } => *span,
            Expr::Index { span, .. } => *span,
            Expr::BinOp { span, .. } => *span,
            Expr::UnOp { span, .. } => *span,
            Expr::Unwrap(_, s) => *s,
            Expr::As { span, .. } => *span,
            Expr::Match { span, .. } => *span,
            Expr::Closure { span, .. } => *span,
            Expr::Spawn(_, s) => *s,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ClosureBody {
    Expr(Box<Expr>),
    Block(Block),
}

#[derive(Debug, Clone)]
pub enum StringSegment {
    Text(String),
    Interp(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    Spaceship,
    And,
    Or,
    Pipe,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
}

// -- Patterns -----------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    /// `_`
    Wildcard(Span),
    /// `42`, `"hello"`, `true`
    Literal(Expr),
    /// `int n`, type binding
    TypeBinding {
        ty: String,
        name: String,
        span: Span,
    },
    /// `Addable x`, interface guard
    InterfaceGuard {
        interface: String,
        name: String,
        span: Span,
    },
    /// `Circle { radius: r }`
    Struct {
        variant: String,
        fields: Vec<(String, String)>,
        span: Span,
    },
    /// `(a, b, c)`
    Tuple(Vec<Pattern>, Span),
}

impl Pattern {
    pub fn span(&self) -> Span {
        match self {
            Pattern::Wildcard(s) => *s,
            Pattern::Literal(e) => e.span(),
            Pattern::TypeBinding { span, .. } => *span,
            Pattern::InterfaceGuard { span, .. } => *span,
            Pattern::Struct { span, .. } => *span,
            Pattern::Tuple(_, s) => *s,
        }
    }
}

// -- Source file --------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub items: Vec<Item>,
    pub span: Span,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ast_types_compile() {
        let _: Option<Item> = None;
        let _: Option<Expr> = None;
        let _: Option<Stmt> = None;
        let _: Option<TypeExpr> = None;
    }
}
