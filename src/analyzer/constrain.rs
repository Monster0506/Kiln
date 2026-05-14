use crate::analyzer::env::GenericBound;
use crate::analyzer::ty::Ty;
use crate::analyzer::typed_ast::{
    TypedBlock, TypedClosureBody, TypedExpr, TypedExprKind, TypedFile, TypedItem, TypedStmt,
    TypedStringSegment,
};
use crate::diagnostics::Span;
use crate::parser::ast::BinOp;

// ---------------------------------------------------------------------------
// Constraint type
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum ConstraintReason {
    Operator(BinOp),
    UnaryNeg,
    Interpolation,
    GenericBoundCheck { param: String, bound: String, fn_name: String },
}

impl ConstraintReason {
    pub fn context_string(&self) -> String {
        match self {
            ConstraintReason::Operator(op) => format!(" (required by operator `{op:?}`)"),
            ConstraintReason::UnaryNeg => " (required by unary `-`)".into(),
            ConstraintReason::Interpolation => " (required by string interpolation)".into(),
            ConstraintReason::GenericBoundCheck { param, bound, fn_name } => {
                format!(" (required by bound `{param}: {bound}` on `{fn_name}`)")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Constraint {
    pub ty: Ty,
    pub iface: String,
    pub span: Span,
    pub reason: ConstraintReason,
}

// ---------------------------------------------------------------------------
// Collection entry point
// ---------------------------------------------------------------------------

pub fn collect_constraints(file: &TypedFile) -> Vec<Constraint> {
    let mut out = Vec::new();
    for item in &file.items {
        collect_item(item, &mut out);
    }
    out
}

fn collect_item(item: &TypedItem, out: &mut Vec<Constraint>) {
    match item {
        TypedItem::Function(f) => collect_block(&f.body, out),
        TypedItem::ImplBlock(ib) => {
            for m in &ib.methods {
                collect_block(&m.body, out);
            }
            for h in &ib.hooks {
                collect_block(&h.body, out);
            }
        }
        _ => {}
    }
}

fn collect_block(block: &TypedBlock, out: &mut Vec<Constraint>) {
    for stmt in &block.stmts {
        collect_stmt(stmt, out);
    }
}

fn collect_stmt(stmt: &TypedStmt, out: &mut Vec<Constraint>) {
    match stmt {
        TypedStmt::Expr(e) => collect_expr(e, out),
        TypedStmt::Return { value, .. } => {
            if let Some(v) = value {
                collect_expr(v, out);
            }
        }
        TypedStmt::VarDecl { value, .. } => collect_expr(value, out),
        TypedStmt::Assign { target, value, .. } => {
            collect_expr(target, out);
            collect_expr(value, out);
        }
        TypedStmt::If { branches, else_branch, .. } => {
            for (cond, body) in branches {
                collect_expr(cond, out);
                collect_block(body, out);
            }
            if let Some(eb) = else_branch {
                collect_block(eb, out);
            }
        }
        TypedStmt::While { cond, body, .. } => {
            collect_expr(cond, out);
            collect_block(body, out);
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            collect_block(body, out);
            collect_expr(cond, out);
        }
        TypedStmt::For { iterable, body, .. } => {
            collect_expr(iterable, out);
            collect_block(body, out);
        }
        TypedStmt::Raise { value, .. } => {
            if let Some(v) = value {
                collect_expr(v, out);
            }
        }
        TypedStmt::TryCatch { body, handlers, finally, .. } => {
            collect_block(body, out);
            for h in handlers {
                collect_block(&h.body, out);
            }
            if let Some(f) = finally {
                collect_block(f, out);
            }
        }
        TypedStmt::FnDef(f) => collect_block(&f.body, out),
        TypedStmt::Break(_) | TypedStmt::Continue(_) => {}
    }
}

fn collect_expr(expr: &TypedExpr, out: &mut Vec<Constraint>) {
    match &expr.kind {
        TypedExprKind::BinOp { op, left, right } => {
            collect_expr(left, out);
            collect_expr(right, out);
            if let Some(iface) = binop_required_iface(op) {
                out.push(Constraint {
                    ty: left.ty.clone(),
                    iface: iface.to_string(),
                    span: expr.span,
                    reason: ConstraintReason::Operator(op.clone()),
                });
            }
        }

        TypedExprKind::UnOp { op, operand } => {
            collect_expr(operand, out);
            if matches!(op, crate::parser::ast::UnOp::Neg) {
                out.push(Constraint {
                    ty: operand.ty.clone(),
                    iface: "Negatable".into(),
                    span: expr.span,
                    reason: ConstraintReason::UnaryNeg,
                });
            }
        }

        TypedExprKind::Str(segs) => {
            for seg in segs {
                if let TypedStringSegment::Interp(e) = seg {
                    collect_expr(e, out);
                    out.push(Constraint {
                        ty: e.ty.clone(),
                        iface: "Display".into(),
                        span: e.span,
                        reason: ConstraintReason::Interpolation,
                    });
                }
            }
        }

        TypedExprKind::Call { callee, args, generic_bounds, generic_params } => {
            if let TypedExprKind::Ident(name) = &callee.kind {
                // Generic bounds declared on the called function.
                if !generic_bounds.is_empty() {
                    let arg_tys: Vec<(Ty, Span)> =
                        args.iter().map(|a| (a.ty.clone(), a.span)).collect();
                    emit_call_bound_constraints(
                        name,
                        generic_bounds,
                        generic_params,
                        &arg_tys,
                        out,
                    );
                }
            }
            collect_expr(callee, out);
            for a in args {
                collect_expr(a, out);
            }
        }

        TypedExprKind::MethodCall { object, args, .. } => {
            collect_expr(object, out);
            for a in args {
                collect_expr(a, out);
            }
        }

        TypedExprKind::StaticCall { args, .. } => {
            for a in args {
                collect_expr(a, out);
            }
        }

        TypedExprKind::IndirectCall { fat_ptr, args } => {
            collect_expr(fat_ptr, out);
            for a in args {
                collect_expr(a, out);
            }
        }

        TypedExprKind::Field { object, .. } => collect_expr(object, out),

        TypedExprKind::Index { object, index } => {
            collect_expr(object, out);
            collect_expr(index, out);
        }

        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_expr(e, out);
            }
        }

        TypedExprKind::Tuple(elems) => {
            for e in elems {
                collect_expr(e, out);
            }
        }

        TypedExprKind::Match { scrutinee, arms } => {
            collect_expr(scrutinee, out);
            for arm in arms {
                collect_expr(&arm.body, out);
                if let Some(g) = &arm.guard {
                    collect_expr(g, out);
                }
            }
        }

        TypedExprKind::Closure { body, .. } => match body {
            TypedClosureBody::Expr(e) => collect_expr(e, out),
            TypedClosureBody::Block(b) => collect_block(b, out),
        },

        TypedExprKind::Unwrap(inner) => collect_expr(inner, out),
        TypedExprKind::As { expr, .. } => collect_expr(expr, out),
        TypedExprKind::Spawn(inner) => collect_expr(inner, out),
        TypedExprKind::Ref { expr, .. } => collect_expr(expr, out),
        TypedExprKind::Gen { body } => collect_block(body, out),
        TypedExprKind::GenSplice(inner) => collect_expr(inner, out),

        // Leaves
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Ident(_) => {}
    }
}

/// Returns the interface required to use this binary operator, if any.
///
/// Uses `PartialEq`/`PartialOrd` for `==`/`<` etc. so that float (which has
/// partial order but not total order) can be compared. Only `<=>` (spaceship)
/// requires a total order (`Ord`).
fn binop_required_iface(op: &BinOp) -> Option<&'static str> {
    match op {
        BinOp::Add => Some("Addable"),
        BinOp::Sub => Some("Subtractable"),
        BinOp::Mul => Some("Multiplicable"),
        BinOp::Div => Some("Divisible"),
        BinOp::Mod => Some("Remainder"),
        BinOp::Eq | BinOp::Ne => Some("PartialEq"),
        BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq => Some("PartialOrd"),
        BinOp::Spaceship => Some("Ord"),
        // &&/|| are bool-only, checked by the type checker.
        BinOp::And | BinOp::Or | BinOp::Pipe => None,
    }
}

/// Emit constraints for a direct call to a named function with generic bounds.
/// `bounds` come from `Symbol::Fn.generic_bounds`.
/// `arg_tys` are `(concrete_type, span)` for each positional argument.
pub fn emit_call_bound_constraints(
    fn_name: &str,
    bounds: &[GenericBound],
    generic_params: &[String],
    arg_tys: &[(Ty, Span)],
    out: &mut Vec<Constraint>,
) {
    for bound in bounds {
        let param_idx = generic_params.iter().position(|p| p == &bound.param);
        if let Some(idx) = param_idx {
            if let Some((ty, span)) = arg_tys.get(idx) {
                out.push(Constraint {
                    ty: ty.clone(),
                    iface: bound.iface.clone(),
                    span: *span,
                    reason: ConstraintReason::GenericBoundCheck {
                        param: bound.param.clone(),
                        bound: bound.iface.clone(),
                        fn_name: fn_name.to_string(),
                    },
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::typed_ast::*;
    use crate::diagnostics::Span;
    use crate::parser::ast::BinOp;

    fn s() -> Span {
        Span::new(0, 0)
    }

    fn int_expr() -> TypedExpr {
        TypedExpr { kind: TypedExprKind::Int(1), ty: Ty::Int, span: s() }
    }

    #[test]
    fn int_add_emits_addable() {
        let expr = TypedExpr {
            kind: TypedExprKind::BinOp {
                op: BinOp::Add,
                left: Box::new(int_expr()),
                right: Box::new(int_expr()),
            },
            ty: Ty::Int,
            span: s(),
        };
        let mut out = vec![];
        collect_expr(&expr, &mut out);
        assert!(out.iter().any(|c| c.iface == "Addable" && c.ty == Ty::Int));
    }

    #[test]
    fn string_interp_emits_display() {
        let interp = TypedExpr { kind: TypedExprKind::Int(42), ty: Ty::Int, span: s() };
        let expr = TypedExpr {
            kind: TypedExprKind::Str(vec![TypedStringSegment::Interp(interp)]),
            ty: Ty::Str,
            span: s(),
        };
        let mut out = vec![];
        collect_expr(&expr, &mut out);
        assert!(out.iter().any(|c| c.iface == "Display" && c.ty == Ty::Int));
    }

    #[test]
    fn no_constraints_for_bool_and() {
        let expr = TypedExpr {
            kind: TypedExprKind::BinOp {
                op: BinOp::And,
                left: Box::new(TypedExpr { kind: TypedExprKind::Bool(true), ty: Ty::Bool, span: s() }),
                right: Box::new(TypedExpr {
                    kind: TypedExprKind::Bool(false),
                    ty: Ty::Bool,
                    span: s(),
                }),
            },
            ty: Ty::Bool,
            span: s(),
        };
        let mut out = vec![];
        collect_expr(&expr, &mut out);
        assert!(out.is_empty());
    }
}
