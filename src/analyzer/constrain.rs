use crate::analyzer::env::GenericBound;
use crate::analyzer::op_hierarchy::compound_assign_iface;
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
    CompoundAssign(BinOp),
    UnaryNeg,
    UnaryPos,
    Interpolation,
    GenericBoundCheck {
        param: String,
        bound: String,
        fn_name: String,
        /// `true` when written explicitly in the function signature.
        is_explicit: bool,
        /// Where in the function body this bound is used (may be set even for explicit bounds).
        source_span: Option<Span>,
        /// Human-readable description of the usage site, e.g. "call to `T.zero()`".
        source_desc: String,
    },
}

impl ConstraintReason {
    pub fn context_string(&self) -> String {
        match self {
            ConstraintReason::Operator(op) => format!(" (required by operator `{op:?}`)"),
            ConstraintReason::CompoundAssign(op) => {
                format!(" (required by compound-assign operator `{op:?}=`)")
            }
            ConstraintReason::UnaryNeg => " (required by unary `-`)".into(),
            ConstraintReason::UnaryPos => " (required by unary `+`)".into(),
            ConstraintReason::Interpolation => " (required by string interpolation)".into(),
            ConstraintReason::GenericBoundCheck {
                param,
                bound,
                fn_name,
                is_explicit,
                ..
            } => {
                if *is_explicit {
                    format!(" (explicit bound `{param}: {bound}` on `{fn_name}`)")
                } else {
                    format!(" (inferred bound `{param}: {bound}` on `{fn_name}`)")
                }
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
        TypedStmt::CompoundAssign {
            target,
            op,
            rhs,
            span,
        } => {
            collect_expr(target, out);
            collect_expr(rhs, out);
            if let Some(iface) = compound_assign_iface(op) {
                out.push(Constraint {
                    ty: target.ty.clone(),
                    iface: iface.to_string(),
                    span: *span,
                    reason: ConstraintReason::CompoundAssign(op.clone()),
                });
            }
        }
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
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
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
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
            if matches!(op, crate::parser::ast::UnOp::Pos) {
                out.push(Constraint {
                    ty: operand.ty.clone(),
                    iface: "Normalizeable".into(),
                    span: expr.span,
                    reason: ConstraintReason::UnaryPos,
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

        TypedExprKind::Call {
            callee,
            args,
            generic_bounds,
            generic_params,
            param_tys,
        } => {
            if let TypedExprKind::Ident(name) = &callee.kind {
                // Generic bounds declared on the called function (includes inferred bounds).
                if !generic_bounds.is_empty() {
                    let arg_tys: Vec<(Ty, Span)> =
                        args.iter().map(|a| (a.ty.clone(), a.span)).collect();
                    emit_call_bound_constraints(
                        name,
                        generic_bounds,
                        generic_params,
                        param_tys,
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
        TypedExprKind::Array(elems) => {
            for e in elems {
                collect_expr(e, out);
            }
        }
        TypedExprKind::Gen { body } => collect_block(body, out),
        TypedExprKind::GenSplice(inner) => collect_expr(inner, out),

        // Leaves
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Ident(_)
        | TypedExprKind::EnumVariant { .. } => {}
    }
}

/// Returns the interface required to use this binary operator, if any.
///
/// Uses `PartialEq`/`PartialOrd` for `==`/`<` etc. so that float (which has
/// partial order but not total order) can be compared. Only `<=>` (spaceship)
/// requires a total order (`Ord`).
pub fn binop_required_iface(op: &BinOp) -> Option<&'static str> {
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
/// `bounds` come from `Symbol::Fn.generic_bounds` plus inferred_bounds.
/// `param_tys` are the declared parameter types (may contain GenericParam).
/// `arg_tys` are `(concrete_type, span)` for each positional argument.
///
/// We unify each `param_tys[i]` with `arg_tys[i].ty` to build a substitution
/// map from generic param names to concrete types, then check each bound against
/// the substituted type.
pub fn emit_call_bound_constraints(
    fn_name: &str,
    bounds: &[GenericBound],
    generic_params: &[String],
    param_tys: &[Ty],
    arg_tys: &[(Ty, Span)],
    out: &mut Vec<Constraint>,
) {
    // Build substitution: generic param name -> concrete type.
    let mut subst: std::collections::HashMap<String, (Ty, Span)> = std::collections::HashMap::new();
    for (i, param_ty) in param_tys.iter().enumerate() {
        if let Some((arg_ty, arg_span)) = arg_tys.get(i) {
            unify_param(param_ty, arg_ty, *arg_span, generic_params, &mut subst);
        }
    }

    for bound in bounds {
        if let Some((concrete_ty, span)) = subst.get(&bound.param) {
            out.push(Constraint {
                ty: concrete_ty.clone(),
                iface: bound.iface.clone(),
                span: *span,
                reason: ConstraintReason::GenericBoundCheck {
                    param: bound.param.clone(),
                    bound: bound.iface.clone(),
                    fn_name: fn_name.to_string(),
                    is_explicit: bound.is_explicit,
                    source_span: bound.source_span,
                    source_desc: bound.source_desc.clone(),
                },
            });
        }
    }
}

/// Unify `param_ty` (declared, possibly contains GenericParam) against `arg_ty`
/// (concrete) and record any GenericParam -> concrete_ty mappings in `subst`.
fn unify_param(
    param_ty: &Ty,
    arg_ty: &Ty,
    span: Span,
    generic_params: &[String],
    subst: &mut std::collections::HashMap<String, (Ty, Span)>,
) {
    match param_ty {
        Ty::GenericParam(p) if generic_params.iter().any(|g| g == p) => {
            subst
                .entry(p.clone())
                .or_insert_with(|| (arg_ty.clone(), span));
        }
        Ty::Named(_, pname, pargs) => {
            if let Ty::Named(_, aname, aargs) = arg_ty {
                if pname == aname {
                    for (pa, aa) in pargs.iter().zip(aargs.iter()) {
                        unify_param(pa, aa, span, generic_params, subst);
                    }
                }
            }
        }
        _ => {}
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
        TypedExpr {
            kind: TypedExprKind::Int(1),
            ty: Ty::Int,
            span: s(),
        }
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
        let interp = TypedExpr {
            kind: TypedExprKind::Int(42),
            ty: Ty::Int,
            span: s(),
        };
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
                left: Box::new(TypedExpr {
                    kind: TypedExprKind::Bool(true),
                    ty: Ty::Bool,
                    span: s(),
                }),
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
