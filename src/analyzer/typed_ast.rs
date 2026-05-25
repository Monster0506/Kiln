use crate::analyzer::ty::Ty;
use crate::diagnostics::Span;
use crate::parser::ast::{BinOp, HookName, ImplKind, UnOp};

// -- Top-level ---------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TypedFile {
    pub items: Vec<TypedItem>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypedItem {
    Function(TypedFnDef),
    Struct(TypedStructDef),
    Enum(TypedEnumDef),
    ImplBlock(TypedImplBlock),
    Interface(TypedInterfaceDef),
    Global(TypedGlobalVar),
    Const(TypedConstDef),
}

#[derive(Debug, Clone)]
pub struct TypedConstDef {
    pub name: String,
    pub ty: Ty,
    pub value: TypedExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedGlobalVar {
    pub name: String,
    pub ty: Ty,
    pub init: TypedExpr,
    pub mutable: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedFnDef {
    pub name: String,
    pub params: Vec<TypedParam>,
    pub variadic: Option<String>,
    pub return_type: Ty,
    pub body: TypedBlock,
    pub is_builtin: bool,
    pub is_inline: bool,
    pub is_declaration: bool,
    pub is_entry: bool,
    pub is_impure: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedParam {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedStructDef {
    pub name: String,
    pub is_builtin: bool,
    pub fields: Vec<TypedField>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedField {
    pub name: String,
    pub ty: Ty,
    pub is_priv: bool,
    pub is_indirect: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedEnumDef {
    pub name: String,
    pub variants: Vec<TypedEnumVariant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedEnumVariant {
    pub name: String,
    pub fields: Vec<TypedField>,
    pub discriminant: Option<i64>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedImplBlock {
    pub interface: String,
    pub for_type: String,
    /// Resolved self type including any generic params, e.g. `Vec(GenericParam("T"))`.
    /// Used by the monomorphizer to detect generic impls and derive substitutions.
    pub for_type_ty: Ty,
    pub kind: ImplKind,
    pub methods: Vec<TypedFnDef>,
    pub hooks: Vec<TypedHookDef>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedInterfaceDef {
    pub name: String,
    pub methods: Vec<TypedInterfaceMethod>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedInterfaceMethod {
    pub name: String,
    pub params: Vec<TypedParam>,
    pub return_type: Ty,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedHookDef {
    pub is_static: bool,
    pub is_impure: bool,
    pub name: HookName,
    pub params: Vec<TypedParam>,
    pub return_type: Ty,
    pub body: TypedBlock,
    pub span: Span,
}

// -- Statements --------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TypedBlock {
    pub stmts: Vec<TypedStmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypedStmt {
    VarDecl {
        name: String,
        ty: Ty,
        value: TypedExpr,
        mutable: bool,
        span: Span,
    },
    Assign {
        target: TypedExpr,
        value: TypedExpr,
        span: Span,
    },
    CompoundAssign {
        target: TypedExpr,
        op: BinOp,
        rhs: TypedExpr,
        span: Span,
    },
    Return {
        value: Option<TypedExpr>,
        span: Span,
    },
    Raise {
        value: Option<TypedExpr>,
        span: Span,
    },
    Break(Span),
    Continue(Span),
    If {
        branches: Vec<(TypedExpr, TypedBlock)>,
        else_branch: Option<TypedBlock>,
        span: Span,
    },
    While {
        cond: TypedExpr,
        body: TypedBlock,
        span: Span,
    },
    DoWhile {
        body: TypedBlock,
        cond: TypedExpr,
        span: Span,
    },
    For {
        binding: String,
        binding_ty: Ty,
        iterable: TypedExpr,
        body: TypedBlock,
        /// Iterator type for custom Iterable dispatch; None for Vec/enum/builtin iteration.
        iter_ty: Option<Ty>,
        span: Span,
    },
    TryCatch {
        body: TypedBlock,
        handlers: Vec<TypedCatchHandler>,
        finally: Option<TypedBlock>,
        span: Span,
    },
    FnDef(TypedFnDef),
    Expr(TypedExpr),
}

#[derive(Debug, Clone)]
pub struct TypedCatchHandler {
    pub ty: Ty,
    pub binding: String,
    pub body: TypedBlock,
    pub span: Span,
}

// -- Expressions -------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypedExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(Vec<TypedStringSegment>),
    /// Variable or function reference
    Ident(String),
    Tuple(Vec<TypedExpr>),
    StructLiteral {
        ty_name: String,
        fields: Vec<(String, TypedExpr)>,
    },
    /// Direct call: callee is a typed expr (ident or closure)
    Call {
        callee: Box<TypedExpr>,
        args: Vec<TypedExpr>,
        /// Original user-visible function name (never mangled), used in diagnostics.
        fn_name: String,
        /// Generic bounds on the called function, carried for constraint checking.
        generic_bounds: Vec<crate::analyzer::env::GenericBound>,
        generic_params: Vec<String>,
        /// Declared parameter types of the callee (with generic params), used to
        /// unify call arguments against generic params for bound checking.
        param_tys: Vec<Ty>,
    },
    /// Instance method call; method_fn is the qualified function name
    MethodCall {
        object: Box<TypedExpr>,
        method_fn: String,
        args: Vec<TypedExpr>,
    },
    /// Static (type-namespace) call; method_fn is the qualified function name
    StaticCall {
        method_fn: String,
        args: Vec<TypedExpr>,
    },
    /// Fat-pointer indirect call for callable-typed values
    IndirectCall {
        fat_ptr: Box<TypedExpr>,
        args: Vec<TypedExpr>,
    },
    Field {
        object: Box<TypedExpr>,
        field: String,
    },
    Index {
        object: Box<TypedExpr>,
        index: Box<TypedExpr>,
    },
    BinOp {
        op: BinOp,
        left: Box<TypedExpr>,
        right: Box<TypedExpr>,
    },
    UnOp {
        op: UnOp,
        operand: Box<TypedExpr>,
    },
    EnumVariant {
        enum_name: String,
        variant: String,
        discriminant: i64,
    },
    Unwrap(Box<TypedExpr>),
    As {
        expr: Box<TypedExpr>,
        ty: Ty,
    },
    Match {
        scrutinee: Box<TypedExpr>,
        arms: Vec<TypedMatchArm>,
    },
    Closure {
        params: Vec<TypedParam>,
        body: TypedClosureBody,
    },
    Spawn(Box<TypedExpr>),
    Ref {
        mutable: bool,
        expr: Box<TypedExpr>,
    },
    Array(Vec<TypedExpr>),
    Gen {
        body: TypedBlock,
    },
    GenSplice(Box<TypedExpr>),
}

#[derive(Debug, Clone)]
pub enum TypedStringSegment {
    Text(String),
    Interp(TypedExpr),
}

#[derive(Debug, Clone)]
pub struct TypedMatchArm {
    pub pattern: TypedPattern,
    pub guard: Option<TypedExpr>,
    pub body: TypedExpr,
    /// When the scrutinee is an enum variant pattern, the known discriminant value.
    /// Set by the analyzer so codegen can omit redundant tag checks in the arm body.
    pub narrowed_discriminant: Option<i64>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypedPattern {
    Wildcard(Span),
    Literal(TypedExpr),
    TypeBinding {
        ty: String,
        name: String,
        span: Span,
    },
    InterfaceGuard {
        interface: String,
        name: String,
        span: Span,
    },
    Struct {
        variant: String,
        fields: Vec<(String, String)>,
        span: Span,
    },
    Tuple(Vec<TypedPattern>, Span),
}

#[derive(Debug, Clone)]
pub enum TypedClosureBody {
    Expr(Box<TypedExpr>),
    Block(TypedBlock),
}
