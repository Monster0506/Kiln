use crate::diagnostics::Span;
use crate::parser::ast::{
    BinOp, Block, Expr, FnDef, HookDef, HookName, ImplBlock, ImplKind, MatchArm, Param, Pattern,
    Stmt, StringSegment, TypeExpr,
};

pub fn s() -> Span {
    Span::new(0, 0)
}

pub fn stype_named(name: &str) -> TypeExpr {
    TypeExpr::Named {
        name: name.into(),
        generics: vec![],
        bindings: vec![],
        span: s(),
    }
}

pub fn stype_int() -> TypeExpr {
    stype_named("int")
}

pub fn stype_str() -> TypeExpr {
    stype_named("str")
}

pub fn stype_bool() -> TypeExpr {
    stype_named("bool")
}

pub fn stype_vec(inner: TypeExpr) -> TypeExpr {
    TypeExpr::Named {
        name: "Vec".into(),
        generics: vec![inner],
        bindings: vec![],
        span: s(),
    }
}

pub fn stype_vec_str() -> TypeExpr {
    stype_vec(stype_str())
}

pub fn sint(n: i64) -> Expr {
    Expr::Int(n, s())
}

pub fn sbool(b: bool) -> Expr {
    Expr::Bool(b, s())
}

pub fn sstr(text: &str) -> Expr {
    Expr::Str(vec![StringSegment::Text(text.into())], s())
}

pub fn sempty_str() -> Expr {
    Expr::Str(vec![], s())
}

pub fn sident(name: &str) -> Expr {
    Expr::Ident(name.into(), s())
}

pub fn sfield(object: Expr, field: &str) -> Expr {
    Expr::Field {
        object: Box::new(object),
        field: field.into(),
        span: s(),
    }
}

pub fn scall_free(fn_name: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Box::new(sident(fn_name)),
        args,
        span: s(),
    }
}

pub fn smethod(obj: Expr, method: &str, args: Vec<Expr>) -> Expr {
    Expr::Call {
        callee: Box::new(sfield(obj, method)),
        args,
        span: s(),
    }
}

pub fn sbinop(op: BinOp, left: Expr, right: Expr) -> Expr {
    Expr::BinOp {
        op,
        left: Box::new(left),
        right: Box::new(right),
        span: s(),
    }
}

pub fn sstruct_literal(ty: &str, fields: Vec<(String, Expr)>) -> Expr {
    Expr::StructLiteral {
        ty: ty.into(),
        fields,
        span: s(),
    }
}

pub fn svar_decl(name: &str, ty: TypeExpr, value: Expr, mutable: bool) -> Stmt {
    Stmt::VarDecl {
        name: name.into(),
        ty,
        value,
        mutable,
        span: s(),
    }
}

pub fn sassign(target: Expr, value: Expr) -> Stmt {
    Stmt::Assign {
        target,
        value,
        span: s(),
    }
}

pub fn scompound_add(target: Expr, rhs: Expr) -> Stmt {
    Stmt::CompoundAssign {
        target,
        op: BinOp::Add,
        rhs,
        span: s(),
    }
}

pub fn sreturn(value: Option<Expr>) -> Stmt {
    Stmt::Return { value, span: s() }
}

pub fn sif(branches: Vec<(Expr, Block)>, else_branch: Option<Block>) -> Stmt {
    Stmt::If {
        branches,
        else_branch,
        span: s(),
    }
}

pub fn swhile(cond: Expr, body: Block) -> Stmt {
    Stmt::While {
        cond,
        body,
        span: s(),
    }
}

pub fn sblock(stmts: Vec<Stmt>) -> Block {
    Block { stmts, span: s() }
}

pub fn sparam(name: &str, ty: TypeExpr) -> Param {
    Param {
        name: name.into(),
        ty,
        mutable: false,
        span: s(),
    }
}

pub fn sfn_impure(name: &str, params: Vec<Param>, return_type: TypeExpr, body: Block) -> FnDef {
    FnDef {
        annotations: vec![],
        name: name.into(),
        generic_params: vec![],
        params,
        variadic: None,
        return_type,
        body,
        is_declaration: false,
        span: s(),
    }
}

pub fn texpr_base_name(t: &TypeExpr) -> Option<&str> {
    match t {
        TypeExpr::Named { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

pub fn simpl(interface: &str, for_type: &str, hooks: Vec<HookDef>) -> ImplBlock {
    ImplBlock {
        generic_params: vec![],
        interface: stype_named(interface),
        for_type: stype_named(for_type),
        self_alias: None,
        methods: vec![],
        hooks,
        assoc_bindings: vec![],
        kind: ImplKind::Plain,
        span: s(),
    }
}

pub fn shook(name: HookName, params: Vec<Param>, return_type: TypeExpr, body: Block) -> HookDef {
    HookDef {
        annotations: vec![],
        name,
        params,
        return_type: Some(return_type),
        body,
        span: s(),
    }
}

pub fn sfield_access(obj: &str, field: &str) -> Expr {
    sfield(sident(obj), field)
}

pub fn senum_access(enum_name: &str, variant: &str) -> Expr {
    Expr::EnumAccess {
        enum_name: enum_name.into(),
        variant: variant.into(),
        span: s(),
    }
}

pub fn spattern_struct(variant: &str, has_rest: bool) -> Pattern {
    Pattern::Struct {
        variant: variant.into(),
        fields: vec![],
        has_rest,
        span: s(),
    }
}

pub fn smatch_arm(pattern: Pattern, body: Expr) -> MatchArm {
    MatchArm {
        pattern,
        guard: None,
        body,
        span: s(),
    }
}

pub fn smatch(scrutinee: Expr, arms: Vec<MatchArm>) -> Expr {
    Expr::Match {
        scrutinee: Box::new(scrutinee),
        arms,
        span: s(),
    }
}
