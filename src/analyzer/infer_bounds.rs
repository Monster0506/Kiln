use crate::analyzer::constrain::binop_required_iface;
use crate::analyzer::env::GenericBound;
use crate::analyzer::op_hierarchy::compound_assign_iface;
use crate::analyzer::ty::{InterfaceId, Ty, TypeRegistry};
use crate::analyzer::typed_ast::{
    TypedBlock, TypedClosureBody, TypedExpr, TypedExprKind, TypedStmt, TypedStringSegment,
};
use crate::diagnostics::Span;
use crate::parser::ast::BinOp;

struct ProjectionCtx {
    span: Span,
    desc: String,
}

/// Emit a projected inferred bound for `Ty::Projection { base, assoc }` in a context
/// requiring `required_iface`, searching all interfaces that declare `assoc`.
fn emit_projection_bound(
    base: &str,
    assoc: &str,
    required_iface: &str,
    params: &[String],
    registry: &TypeRegistry,
    ctx: ProjectionCtx,
    out: &mut Vec<GenericBound>,
) {
    if !params.iter().any(|p| p == base) {
        return;
    }
    for iface_name in registry.interfaces_declaring_assoc(assoc) {
        let assoc_ty = Ty::Interface(InterfaceId(0), required_iface.to_string());
        out.push(GenericBound {
            param: base.to_string(),
            iface: iface_name.clone(),
            assoc_bindings: vec![(assoc.to_string(), assoc_ty)],
            is_explicit: false,
            decl_span: None,
            source_span: Some(ctx.span),
            source_desc: ctx.desc.clone(),
        });
    }
}

fn binop_symbol(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::LtEq => "<=",
        BinOp::GtEq => ">=",
        BinOp::Spaceship => "<=>",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::Pipe => "|>",
    }
}

/// Scan a function body and infer interface bounds required for each generic param.
/// Emits `GenericBound` for each operation (e.g. `a+b` -> `T: Addable`), stored on `inferred_bounds`.
pub fn infer_bounds_from_body(
    body: &TypedBlock,
    generic_params: &[String],
    registry: &TypeRegistry,
) -> Vec<GenericBound> {
    let mut bounds: Vec<GenericBound> = Vec::new();
    collect_block(body, generic_params, registry, &mut bounds);
    let deduped = dedup_bounds(bounds);
    minimize_bounds(deduped, registry)
}

fn dedup_bounds(mut v: Vec<GenericBound>) -> Vec<GenericBound> {
    v.sort_by(|a, b| a.param.cmp(&b.param).then(a.iface.cmp(&b.iface)));
    v.dedup_by(|a, b| a.param == b.param && a.iface == b.iface);
    v
}

/// Reduce a bound set to its antichain: drop `T: A` when some `T: B` also exists with
/// B more specific than A, avoiding redundant constraints at call sites.
fn minimize_bounds(bounds: Vec<GenericBound>, registry: &TypeRegistry) -> Vec<GenericBound> {
    let keep: Vec<bool> = bounds
        .iter()
        .map(|b| {
            !bounds.iter().any(|other| {
                other.param == b.param
                    && other.iface != b.iface
                    && registry.iface_implies(&other.iface, &b.iface)
            })
        })
        .collect();
    bounds
        .into_iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(b, _)| b)
        .collect()
}

fn collect_block(
    block: &TypedBlock,
    params: &[String],
    registry: &TypeRegistry,
    out: &mut Vec<GenericBound>,
) {
    for stmt in &block.stmts {
        collect_stmt(stmt, params, registry, out);
    }
}

fn collect_stmt(
    stmt: &TypedStmt,
    params: &[String],
    registry: &TypeRegistry,
    out: &mut Vec<GenericBound>,
) {
    match stmt {
        TypedStmt::VarDecl { value, .. } => collect_expr(value, params, registry, out),
        TypedStmt::Assign { target, value, .. } => {
            collect_expr(target, params, registry, out);
            collect_expr(value, params, registry, out);
        }
        TypedStmt::CompoundAssign {
            target,
            op,
            rhs,
            span,
        } => {
            collect_expr(target, params, registry, out);
            collect_expr(rhs, params, registry, out);
            if let Ty::GenericParam(p) = &target.ty {
                if params.iter().any(|x| x == p) {
                    if let Some(iface) = compound_assign_iface(op) {
                        let sym = binop_symbol(op);
                        out.push(GenericBound {
                            param: p.clone(),
                            iface: iface.to_string(),
                            assoc_bindings: vec![],
                            is_explicit: false,
                            decl_span: None,
                            source_span: Some(*span),
                            source_desc: format!("use of `{sym}=` on `{p}`"),
                        });
                    }
                }
            }
        }
        TypedStmt::Return { value, .. } => {
            if let Some(v) = value {
                collect_expr(v, params, registry, out);
            }
        }
        TypedStmt::Expr(e) => collect_expr(e, params, registry, out),
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (cond, body) in branches {
                collect_expr(cond, params, registry, out);
                collect_block(body, params, registry, out);
            }
            if let Some(eb) = else_branch {
                collect_block(eb, params, registry, out);
            }
        }
        TypedStmt::While { cond, body, .. } => {
            collect_expr(cond, params, registry, out);
            collect_block(body, params, registry, out);
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            collect_block(body, params, registry, out);
            collect_expr(cond, params, registry, out);
        }
        TypedStmt::For { iterable, body, .. } => {
            collect_expr(iterable, params, registry, out);
            collect_block(body, params, registry, out);
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            collect_block(body, params, registry, out);
            for h in handlers {
                collect_block(&h.body, params, registry, out);
            }
            if let Some(f) = finally {
                collect_block(f, params, registry, out);
            }
        }
        TypedStmt::FnDef(f) => collect_block(&f.body, params, registry, out),
        TypedStmt::Raise { value, .. } => {
            if let Some(v) = value {
                collect_expr(v, params, registry, out);
            }
        }
        TypedStmt::Break(_) | TypedStmt::Continue(_) => {}
    }
}

fn collect_expr(
    expr: &TypedExpr,
    params: &[String],
    registry: &TypeRegistry,
    out: &mut Vec<GenericBound>,
) {
    match &expr.kind {
        TypedExprKind::BinOp { op, left, right } => {
            collect_expr(left, params, registry, out);
            collect_expr(right, params, registry, out);
            if let Some(iface) = binop_required_iface(op) {
                let sym = binop_symbol(op);
                match &left.ty {
                    Ty::GenericParam(p) if params.iter().any(|x| x == p) => {
                        out.push(GenericBound {
                            param: p.clone(),
                            iface: iface.to_string(),
                            assoc_bindings: vec![],
                            is_explicit: false,
                            decl_span: None,
                            source_span: Some(expr.span),
                            source_desc: format!("use of `{sym}` on `{p}`"),
                        });
                    }
                    Ty::Projection { base, assoc } => {
                        emit_projection_bound(
                            base,
                            assoc,
                            iface,
                            params,
                            registry,
                            ProjectionCtx {
                                span: expr.span,
                                desc: format!("use of `{sym}` on `{base}.{assoc}`"),
                            },
                            out,
                        );
                    }
                    _ => {}
                }
            }
        }

        TypedExprKind::StaticCall { method_fn, args } => {
            for a in args {
                collect_expr(a, params, registry, out);
            }
            // method_fn is "ParamName_hookname"; emit ParamName: InterfaceForHook
            for param in params {
                let prefix = format!("{}_", param);
                if let Some(hook) = method_fn.strip_prefix(prefix.as_str()) {
                    for iface in registry.interfaces_for_hook(hook) {
                        out.push(GenericBound {
                            param: param.clone(),
                            iface,
                            assoc_bindings: vec![],
                            is_explicit: false,
                            decl_span: None,
                            source_span: Some(expr.span),
                            source_desc: format!("call to `{param}.{hook}()`"),
                        });
                    }
                }
            }
        }

        TypedExprKind::MethodCall { object, args, .. } => {
            collect_expr(object, params, registry, out);
            for a in args {
                collect_expr(a, params, registry, out);
            }
        }

        TypedExprKind::Call { callee, args, .. } => {
            collect_expr(callee, params, registry, out);
            for a in args {
                collect_expr(a, params, registry, out);
            }
        }

        TypedExprKind::IndirectCall { fat_ptr, args } => {
            collect_expr(fat_ptr, params, registry, out);
            for a in args {
                collect_expr(a, params, registry, out);
            }
        }

        TypedExprKind::Field { object, .. } => collect_expr(object, params, registry, out),

        TypedExprKind::Index { object, index } => {
            collect_expr(object, params, registry, out);
            collect_expr(index, params, registry, out);
        }

        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_expr(e, params, registry, out);
            }
        }

        TypedExprKind::Tuple(elems) => {
            for e in elems {
                collect_expr(e, params, registry, out);
            }
        }

        TypedExprKind::Match { scrutinee, arms } => {
            collect_expr(scrutinee, params, registry, out);
            for arm in arms {
                collect_expr(&arm.body, params, registry, out);
                if let Some(g) = &arm.guard {
                    collect_expr(g, params, registry, out);
                }
            }
        }

        TypedExprKind::UnOp { operand, .. } => collect_expr(operand, params, registry, out),
        TypedExprKind::Unwrap(inner) => collect_expr(inner, params, registry, out),
        TypedExprKind::As { expr, .. } => collect_expr(expr, params, registry, out),
        TypedExprKind::Spawn(inner) => collect_expr(inner, params, registry, out),
        TypedExprKind::Ref { expr, .. } => collect_expr(expr, params, registry, out),
        TypedExprKind::Array(elems) => {
            for e in elems {
                collect_expr(e, params, registry, out);
            }
        }
        TypedExprKind::Gen { body } => collect_block(body, params, registry, out),
        TypedExprKind::GenSplice(inner) => collect_expr(inner, params, registry, out),
        TypedExprKind::Str(segs) => {
            for seg in segs {
                if let TypedStringSegment::Interp(e) = seg {
                    collect_expr(e, params, registry, out);
                    match &e.ty {
                        Ty::GenericParam(p) if params.iter().any(|x| x == p) => {
                            out.push(GenericBound {
                                param: p.clone(),
                                iface: "Display".to_string(),
                                assoc_bindings: vec![],
                                is_explicit: false,
                                decl_span: None,
                                source_span: Some(e.span),
                                source_desc: format!("string interpolation of `{p}`"),
                            });
                        }
                        Ty::Projection { base, assoc } => {
                            emit_projection_bound(
                                base,
                                assoc,
                                "Display",
                                params,
                                registry,
                                ProjectionCtx {
                                    span: e.span,
                                    desc: format!("string interpolation of `{base}.{assoc}`"),
                                },
                                out,
                            );
                        }
                        _ => {}
                    }
                }
            }
        }
        TypedExprKind::Closure { body, .. } => match body {
            TypedClosureBody::Expr(e) => collect_expr(e, params, registry, out),
            TypedClosureBody::Block(b) => collect_block(b, params, registry, out),
        },

        TypedExprKind::Block(stmts) => {
            for s in stmts {
                collect_stmt(s, params, registry, out);
            }
        }
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Ident(_)
        | TypedExprKind::EnumVariant { .. } => {}
        TypedExprKind::BoundMethod { object, .. } => collect_expr(object, params, registry, out),
        TypedExprKind::PrimTypeRef { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::MethodEntry;
    use crate::analyzer::typed_ast::{TypedExpr, TypedExprKind};
    use crate::diagnostics::Span;
    use crate::parser::ast::BinOp;

    fn s() -> Span {
        Span::new(0, 0)
    }

    fn int_expr() -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Int(0),
            ty: Ty::Int,
            span: s(),
        }
    }

    fn param_expr(p: &str) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Ident(p.to_string()),
            ty: Ty::GenericParam(p.to_string()),
            span: s(),
        }
    }

    fn make_registry_with_zero() -> TypeRegistry {
        let mut r = TypeRegistry::new();
        r.register_interface_method(
            "Zero",
            MethodEntry {
                method_name: "zero".into(),
                qualified_fn: "zero".into(),
                params: vec![],
                ret: Ty::Unknown,
            },
        );
        r
    }

    #[test]
    fn binop_add_on_generic_infers_addable() {
        let reg = TypeRegistry::new();
        let body = TypedBlock {
            stmts: vec![TypedStmt::Expr(TypedExpr {
                kind: TypedExprKind::BinOp {
                    op: BinOp::Add,
                    left: Box::new(param_expr("T")),
                    right: Box::new(param_expr("T")),
                },
                ty: Ty::GenericParam("T".into()),
                span: s(),
            })],
            span: s(),
        };
        let bounds = infer_bounds_from_body(&body, &["T".to_string()], &reg);
        assert!(
            bounds
                .iter()
                .any(|b| b.param == "T" && b.iface == "Addable"),
            "expected T: Addable, got {bounds:?}"
        );
    }

    #[test]
    fn static_call_zero_on_generic_infers_zero() {
        let reg = make_registry_with_zero();
        let body = TypedBlock {
            stmts: vec![TypedStmt::Expr(TypedExpr {
                kind: TypedExprKind::StaticCall {
                    method_fn: "T_zero".into(),
                    args: vec![],
                },
                ty: Ty::Unknown,
                span: s(),
            })],
            span: s(),
        };
        let bounds = infer_bounds_from_body(&body, &["T".to_string()], &reg);
        assert!(
            bounds.iter().any(|b| b.param == "T" && b.iface == "Zero"),
            "expected T: Zero, got {bounds:?}"
        );
    }

    #[test]
    fn binop_on_non_generic_emits_no_bound() {
        let reg = TypeRegistry::new();
        let body = TypedBlock {
            stmts: vec![TypedStmt::Expr(TypedExpr {
                kind: TypedExprKind::BinOp {
                    op: BinOp::Add,
                    left: Box::new(int_expr()),
                    right: Box::new(int_expr()),
                },
                ty: Ty::Int,
                span: s(),
            })],
            span: s(),
        };
        let bounds = infer_bounds_from_body(&body, &["T".to_string()], &reg);
        assert!(bounds.is_empty(), "expected no bounds, got {bounds:?}");
    }

    #[test]
    fn compound_assign_on_generic_infers_addable() {
        let reg = TypeRegistry::new();
        let body = TypedBlock {
            stmts: vec![TypedStmt::CompoundAssign {
                target: param_expr("T"),
                op: BinOp::Add,
                rhs: param_expr("T"),
                span: s(),
            }],
            span: s(),
        };
        let bounds = infer_bounds_from_body(&body, &["T".to_string()], &reg);
        assert!(
            bounds
                .iter()
                .any(|b| b.param == "T" && b.iface == "Addable"),
            "expected T: Addable, got {bounds:?}"
        );
    }

    // Constraint minimization: T: Ord implies T: PartialOrd, so PartialOrd is dropped.
    #[test]
    fn minimize_drops_implied_bound() {
        use crate::analyzer::env::GenericBound;
        use crate::analyzer::ty::ConformanceEntry;
        let mut reg = TypeRegistry::new();
        reg.register_conformance(
            "int",
            "Ord",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        reg.register_interface_supers("Ord", vec!["PartialOrd".to_string()]);
        reg.precompute_transitive_closures();

        let bounds = vec![
            GenericBound {
                param: "T".into(),
                iface: "Ord".into(),
                assoc_bindings: vec![],
                is_explicit: false,
                decl_span: None,
                source_span: None,
                source_desc: String::new(),
            },
            GenericBound {
                param: "T".into(),
                iface: "PartialOrd".into(),
                assoc_bindings: vec![],
                is_explicit: false,
                decl_span: None,
                source_span: None,
                source_desc: String::new(),
            },
        ];
        let minimized = minimize_bounds(bounds, &reg);
        assert_eq!(
            minimized.len(),
            1,
            "PartialOrd should be dropped: {minimized:?}"
        );
        assert_eq!(minimized[0].iface, "Ord");
    }

    // Minimization keeps unrelated bounds.
    #[test]
    fn minimize_keeps_unrelated_bounds() {
        use crate::analyzer::env::GenericBound;
        let reg = TypeRegistry::new();

        let bounds = vec![
            GenericBound {
                param: "T".into(),
                iface: "Addable".into(),
                assoc_bindings: vec![],
                is_explicit: false,
                decl_span: None,
                source_span: None,
                source_desc: String::new(),
            },
            GenericBound {
                param: "T".into(),
                iface: "Display".into(),
                assoc_bindings: vec![],
                is_explicit: false,
                decl_span: None,
                source_span: None,
                source_desc: String::new(),
            },
        ];
        let minimized = minimize_bounds(bounds, &reg);
        assert_eq!(
            minimized.len(),
            2,
            "both unrelated bounds should be kept: {minimized:?}"
        );
    }
}
