use crate::analyzer::ty::{Ty, TypeId};
use crate::analyzer::typed_ast::{
    TypedBlock, TypedExpr, TypedExprKind, TypedFnDef, TypedParam, TypedStmt, TypedStringSegment,
};
use crate::diagnostics::Span;
use crate::parser::ast::BinOp;

pub fn s() -> Span {
    Span::new(0, 0)
}

// TypeId(0) is safe here; layout lookup uses the name, not the id.
pub fn dummy_id() -> TypeId {
    TypeId(0)
}

pub fn named_ty(name: &str) -> Ty {
    Ty::Named(dummy_id(), name.into(), vec![])
}

pub fn vec_ty(inner: Ty) -> Ty {
    Ty::Named(dummy_id(), "Vec".into(), vec![inner])
}

pub fn vec_str_ty() -> Ty {
    vec_ty(Ty::Str)
}

pub fn tex(kind: TypedExprKind, ty: Ty) -> TypedExpr {
    TypedExpr {
        kind,
        ty,
        span: s(),
    }
}

pub fn tint(n: i64) -> TypedExpr {
    tex(TypedExprKind::Int(n), Ty::Int)
}

pub fn tbool(b: bool) -> TypedExpr {
    tex(TypedExprKind::Bool(b), Ty::Bool)
}

pub fn tstr_lit(text: &str) -> TypedExpr {
    let segs = if text.is_empty() {
        vec![]
    } else {
        vec![TypedStringSegment::Text(text.into())]
    };
    tex(TypedExprKind::Str(segs), Ty::Str)
}

pub fn tident(name: &str, ty: Ty) -> TypedExpr {
    tex(TypedExprKind::Ident(name.into()), ty)
}

pub fn tfield(obj: TypedExpr, field: &str, ty: Ty) -> TypedExpr {
    tex(
        TypedExprKind::Field {
            object: Box::new(obj),
            field: field.into(),
        },
        ty,
    )
}

pub fn tcall(fn_name: &str, args: Vec<TypedExpr>, return_ty: Ty) -> TypedExpr {
    tex(
        TypedExprKind::Call {
            callee: Box::new(tex(TypedExprKind::Ident(fn_name.into()), Ty::Unknown)),
            args,
            fn_name: fn_name.into(),
            generic_bounds: vec![],
            generic_params: vec![],
            param_tys: vec![],
        },
        return_ty,
    )
}

pub fn tmethod(obj: TypedExpr, method_fn: &str, args: Vec<TypedExpr>, return_ty: Ty) -> TypedExpr {
    tex(
        TypedExprKind::MethodCall {
            object: Box::new(obj),
            method_fn: method_fn.into(),
            args,
        },
        return_ty,
    )
}

pub fn tbinop(op: BinOp, left: TypedExpr, right: TypedExpr, ty: Ty) -> TypedExpr {
    tex(
        TypedExprKind::BinOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        ty,
    )
}

pub fn tstruct_literal(ty_name: &str, fields: Vec<(String, TypedExpr)>) -> TypedExpr {
    tex(
        TypedExprKind::StructLiteral {
            ty_name: ty_name.into(),
            fields,
        },
        named_ty(ty_name),
    )
}

pub fn tvar_decl(name: &str, ty: Ty, value: TypedExpr, mutable: bool) -> TypedStmt {
    TypedStmt::VarDecl {
        name: name.into(),
        ty,
        value,
        mutable,
        span: s(),
    }
}

pub fn tassign(target: TypedExpr, value: TypedExpr) -> TypedStmt {
    TypedStmt::Assign {
        target,
        value,
        span: s(),
    }
}

pub fn treturn(value: Option<TypedExpr>) -> TypedStmt {
    TypedStmt::Return { value, span: s() }
}

pub fn tif(branches: Vec<(TypedExpr, TypedBlock)>, else_branch: Option<TypedBlock>) -> TypedStmt {
    TypedStmt::If {
        branches,
        else_branch,
        span: s(),
    }
}

pub fn twhile(cond: TypedExpr, body: TypedBlock) -> TypedStmt {
    TypedStmt::While {
        cond,
        body,
        span: s(),
    }
}

pub fn tblock(stmts: Vec<TypedStmt>) -> TypedBlock {
    TypedBlock { stmts, span: s() }
}

pub fn impure_fn(
    name: &str,
    params: Vec<TypedParam>,
    return_type: Ty,
    body: TypedBlock,
) -> TypedFnDef {
    TypedFnDef {
        name: name.into(),
        params,
        variadic: None,
        return_type,
        body,
        is_builtin: false,
        is_inline: false,
        is_declaration: false,
        is_entry: false,
        is_impure: true,
        throws: false,
        span: s(),
    }
}

pub fn tparam(name: &str, ty: Ty) -> TypedParam {
    TypedParam {
        name: name.into(),
        ty,
        mutable: false,
        span: s(),
    }
}
