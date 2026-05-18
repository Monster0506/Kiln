use crate::analyzer::env::{Env, Symbol};
use crate::analyzer::error::AnalysisError;
use crate::analyzer::infer::{check_assignable, infer_typed_expr, substitute_t};
use crate::analyzer::resolve::resolve_type_expr;
use crate::analyzer::ty::{Ty, TypeKind, TypeRegistry};
use crate::analyzer::typed_ast::{
    TypedBlock, TypedCatchHandler, TypedFnDef, TypedParam, TypedStmt,
};
use crate::parser::ast::{Block, Expr, FnDef, Stmt};

/// Check all statements in `block` and return a typed block.
pub fn check_typed_block(
    block: &Block,
    env: &mut Env,
    registry: &TypeRegistry,
    return_ty: &Ty,
    errors: &mut Vec<AnalysisError>,
) -> TypedBlock {
    env.push_scope();
    let mut stmts: Vec<TypedStmt> = Vec::new();
    for s in &block.stmts {
        stmts.push(check_typed_stmt(s, env, registry, return_ty, errors));
    }
    env.pop_scope();
    TypedBlock {
        stmts,
        span: block.span,
    }
}

fn check_typed_stmt(
    stmt: &Stmt,
    env: &mut Env,
    registry: &TypeRegistry,
    return_ty: &Ty,
    errors: &mut Vec<AnalysisError>,
) -> TypedStmt {
    match stmt {
        Stmt::VarDecl {
            name,
            ty,
            value,
            mutable,
            span,
        } => {
            let declared = resolve_type_expr(ty, env, errors);
            let typed_val = infer_typed_expr(value, env, registry, errors);
            if let Ty::Interface(_, iface_name) = &declared {
                // Verify the assigned type implements the interface. Unknown is
                // allowed to avoid double-reporting after earlier errors.
                if let Some(type_name) = crate::analyzer::infer::type_name_of(&typed_val.ty) {
                    if registry.get_conformances(&type_name, iface_name).is_empty() {
                        errors.push(AnalysisError::TypeMismatch {
                            expected: iface_name.clone(),
                            found: type_name,
                            span: typed_val.span,
                        });
                    }
                }
            } else {
                check_assignable(&declared, &typed_val.ty, &typed_val.span, errors);
            }
            env.define(
                name,
                Symbol::Var {
                    ty: declared.clone(),
                    mutable: *mutable,
                    span: *span,
                },
            );
            TypedStmt::VarDecl {
                name: name.clone(),
                ty: declared,
                value: typed_val,
                mutable: *mutable,
                span: *span,
            }
        }

        Stmt::Assign {
            target,
            value,
            span,
        } => {
            if let Expr::Ident(name, ident_span) = target {
                match env.lookup(name) {
                    Some(Symbol::Var { mutable: false, .. }) => {
                        errors.push(AnalysisError::AssignToImmutable {
                            name: name.clone(),
                            span: *ident_span,
                        });
                    }
                    None => {
                        errors.push(AnalysisError::UndefinedName {
                            name: name.clone(),
                            span: *ident_span,
                        });
                    }
                    _ => {}
                }
            }
            let typed_target = infer_typed_expr(target, env, registry, errors);
            let typed_val = infer_typed_expr(value, env, registry, errors);
            check_assignable(&typed_target.ty, &typed_val.ty, &typed_val.span, errors);
            TypedStmt::Assign {
                target: typed_target,
                value: typed_val,
                span: *span,
            }
        }

        Stmt::CompoundAssign {
            target,
            op,
            rhs,
            span,
        } => {
            if let Expr::Ident(name, ident_span) = target {
                match env.lookup(name) {
                    Some(Symbol::Var { mutable: false, .. }) => {
                        errors.push(AnalysisError::AssignToImmutable {
                            name: name.clone(),
                            span: *ident_span,
                        });
                    }
                    None => {
                        errors.push(AnalysisError::UndefinedName {
                            name: name.clone(),
                            span: *ident_span,
                        });
                    }
                    _ => {}
                }
            }
            TypedStmt::CompoundAssign {
                target: infer_typed_expr(target, env, registry, errors),
                op: op.clone(),
                rhs: infer_typed_expr(rhs, env, registry, errors),
                span: *span,
            }
        }

        Stmt::Return { value, span } => {
            let (typed, val_span) = match value {
                Some(v) => {
                    let te = infer_typed_expr(v, env, registry, errors);
                    let s = te.span;
                    (Some(te), s)
                }
                None => (None, *span),
            };
            let found = typed.as_ref().map(|t| t.ty.clone()).unwrap_or(Ty::Void);
            check_assignable(return_ty, &found, &val_span, errors);
            TypedStmt::Return {
                value: typed,
                span: *span,
            }
        }

        Stmt::Raise { value, span } => {
            let typed = value
                .as_ref()
                .map(|v| infer_typed_expr(v, env, registry, errors));
            TypedStmt::Raise {
                value: typed,
                span: *span,
            }
        }

        Stmt::Expr(expr) => TypedStmt::Expr(infer_typed_expr(expr, env, registry, errors)),

        Stmt::If {
            branches,
            else_branch,
            span,
        } => {
            let mut typed_branches = Vec::new();
            for (cond, block) in branches {
                let tc = infer_typed_expr(cond, env, registry, errors);
                if !matches!(tc.ty, Ty::Bool | Ty::Unknown) {
                    errors.push(AnalysisError::TypeMismatch {
                        expected: "bool".into(),
                        found: tc.ty.to_string(),
                        span: cond.span(),
                    });
                }
                let tb = check_typed_block(block, env, registry, return_ty, errors);
                typed_branches.push((tc, tb));
            }
            let else_typed = else_branch
                .as_ref()
                .map(|b| check_typed_block(b, env, registry, return_ty, errors));
            TypedStmt::If {
                branches: typed_branches,
                else_branch: else_typed,
                span: *span,
            }
        }

        Stmt::While { cond, body, span } => {
            let tc = infer_typed_expr(cond, env, registry, errors);
            if !matches!(tc.ty, Ty::Bool | Ty::Unknown) {
                errors.push(AnalysisError::TypeMismatch {
                    expected: "bool".into(),
                    found: tc.ty.to_string(),
                    span: *span,
                });
            }
            let tb = check_typed_block(body, env, registry, return_ty, errors);
            TypedStmt::While {
                cond: tc,
                body: tb,
                span: *span,
            }
        }

        Stmt::DoWhile { body, cond, span } => {
            let tb = check_typed_block(body, env, registry, return_ty, errors);
            let tc = infer_typed_expr(cond, env, registry, errors);
            if !matches!(tc.ty, Ty::Bool | Ty::Unknown) {
                errors.push(AnalysisError::TypeMismatch {
                    expected: "bool".into(),
                    found: tc.ty.to_string(),
                    span: *span,
                });
            }
            TypedStmt::DoWhile {
                body: tb,
                cond: tc,
                span: *span,
            }
        }

        Stmt::For {
            binding,
            binding_ty,
            iterable,
            body,
            span,
        } => {
            let ti = infer_typed_expr(iterable, env, registry, errors);
            // Determine element type and optional iterator type for custom Iterable dispatch.
            let mut iter_ty: Option<Ty> = None;
            let elem_ty = match &ti.ty {
                Ty::Named(_, name, args) if name == "Vec" => {
                    let elem = args.first().cloned().unwrap_or(Ty::Unknown);
                    if let Some(iter_method) = registry.find_method("Vec", "iter") {
                        iter_ty = Some(substitute_t(&iter_method.ret, &elem));
                    }
                    elem
                }
                Ty::Named(_, name, args) if name == "Set" => {
                    args.first().cloned().unwrap_or(Ty::Unknown)
                }
                Ty::Str => Ty::Str,
                Ty::Named(_, name, args) if args.is_empty() => {
                    match registry.lookup_by_name(name) {
                        Some(entry) if matches!(&entry.kind, TypeKind::Enum { .. }) => {
                            ti.ty.clone()
                        }
                        _ => {
                            // Check for custom Iterable: look for an iter() method.
                            if let Some(iter_method) = registry.find_method(name, "iter") {
                                let it = iter_method.ret.clone();
                                // Derive element type from Iterator::next() -> Option[Item].
                                let item_ty = if let Ty::Named(_, iter_name, _) = &it {
                                    registry
                                        .find_method(iter_name, "next")
                                        .and_then(|m| {
                                            if let Ty::Named(_, opt_name, opt_args) = &m.ret {
                                                if opt_name == "Option" {
                                                    opt_args.first().cloned()
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or(Ty::Unknown)
                                } else {
                                    Ty::Unknown
                                };
                                iter_ty = Some(it);
                                item_ty
                            } else {
                                Ty::Unknown
                            }
                        }
                    }
                }
                _ => Ty::Unknown,
            };
            let ann_ty = if let Some(ann) = binding_ty {
                let at = resolve_type_expr(ann, env, errors);
                check_assignable(&at, &elem_ty, span, errors);
                at
            } else {
                elem_ty.clone()
            };
            env.push_scope();
            env.define(
                binding,
                Symbol::Var {
                    ty: elem_ty,
                    mutable: false,
                    span: *span,
                },
            );
            let mut tb_stmts = Vec::new();
            for s in &body.stmts {
                tb_stmts.push(check_typed_stmt(s, env, registry, return_ty, errors));
            }
            env.pop_scope();
            let typed_body = TypedBlock {
                stmts: tb_stmts,
                span: body.span,
            };
            TypedStmt::For {
                binding: binding.clone(),
                binding_ty: ann_ty,
                iterable: ti,
                body: typed_body,
                iter_ty,
                span: *span,
            }
        }

        Stmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => {
            let typed_body = check_typed_block(body, env, registry, return_ty, errors);
            let mut typed_handlers: Vec<TypedCatchHandler> = Vec::new();
            for h in handlers {
                let exc_ty = resolve_type_expr(&h.ty, env, errors);
                env.push_scope();
                env.define(
                    &h.binding,
                    Symbol::Var {
                        ty: exc_ty.clone(),
                        mutable: false,
                        span: h.span,
                    },
                );
                let hb = check_typed_block(&h.body, env, registry, return_ty, errors);
                env.pop_scope();
                typed_handlers.push(TypedCatchHandler {
                    ty: exc_ty,
                    binding: h.binding.clone(),
                    body: hb,
                    span: h.span,
                });
            }
            let typed_finally = finally
                .as_ref()
                .map(|b| check_typed_block(b, env, registry, return_ty, errors));
            TypedStmt::TryCatch {
                body: typed_body,
                handlers: typed_handlers,
                finally: typed_finally,
                span: *span,
            }
        }

        Stmt::FnDef(f) => TypedStmt::FnDef(check_fn_def(f, env, registry, errors)),

        Stmt::Break(s) => TypedStmt::Break(*s),
        Stmt::Continue(s) => TypedStmt::Continue(*s),
    }
}

/// Check a function definition and produce a typed one.
pub fn check_fn_def(
    f: &FnDef,
    env: &mut Env,
    registry: &TypeRegistry,
    errors: &mut Vec<AnalysisError>,
) -> TypedFnDef {
    let ret = resolve_type_expr(&f.return_type, env, errors);
    let mut params: Vec<TypedParam> = Vec::new();
    for p in &f.params {
        params.push(TypedParam {
            name: p.name.clone(),
            ty: resolve_type_expr(&p.ty, env, errors),
            span: p.span,
        });
    }

    env.define(
        &f.name,
        Symbol::Fn {
            generic_params: f.generic_params.iter().map(|g| g.name.clone()).collect(),
            generic_bounds: f
                .generic_params
                .iter()
                .flat_map(|g| {
                    g.bounds
                        .iter()
                        .map(move |b| crate::analyzer::env::GenericBound {
                            param: g.name.clone(),
                            iface: b.clone(),
                            is_explicit: true,
                            decl_span: Some(g.span),
                            source_span: None,
                            source_desc: String::new(),
                        })
                })
                .collect(),
            inferred_bounds: vec![],
            params: params
                .iter()
                .map(|p| (p.name.clone(), p.ty.clone()))
                .collect(),
            ret: ret.clone(),
            span: f.span,
        },
    );

    env.push_scope();
    for p in &params {
        env.define(
            &p.name,
            Symbol::Var {
                ty: p.ty.clone(),
                mutable: false,
                span: p.span,
            },
        );
    }
    let body = check_typed_block(&f.body, env, registry, &ret, errors);
    env.pop_scope();

    TypedFnDef {
        name: f.name.clone(),
        params,
        variadic: f.variadic.as_ref().map(|v| v.name.clone()),
        return_type: ret,
        body,
        is_builtin: false,
        is_inline: f.annotations.iter().any(|a| a.name == "inline"),
        is_declaration: false,
        span: f.span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::env::Env;
    use crate::analyzer::ty::{Ty, TypeRegistry};
    use crate::diagnostics::Span;
    use crate::parser::ast::*;

    fn s() -> Span {
        Span { start: 0, end: 0 }
    }
    fn int_ty() -> TypeExpr {
        TypeExpr::Named {
            name: "int".into(),
            generics: vec![],
            bindings: vec![],
            span: s(),
        }
    }
    fn bool_ty() -> TypeExpr {
        TypeExpr::Named {
            name: "bool".into(),
            generics: vec![],
            bindings: vec![],
            span: s(),
        }
    }

    #[test]
    fn var_decl_ok() {
        let block = Block {
            stmts: vec![Stmt::VarDecl {
                name: "x".into(),
                ty: int_ty(),
                value: Expr::Int(42, s()),
                mutable: false,
                span: s(),
            }],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn var_decl_type_mismatch() {
        let block = Block {
            stmts: vec![Stmt::VarDecl {
                name: "x".into(),
                ty: bool_ty(),
                value: Expr::Int(1, s()),
                mutable: false,
                span: s(),
            }],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn assign_to_immutable_is_error() {
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_ty(),
                    value: Expr::Int(1, s()),
                    mutable: false,
                    span: s(),
                },
                Stmt::Assign {
                    target: Expr::Ident("x".into(), s()),
                    value: Expr::Int(2, s()),
                    span: s(),
                },
            ],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert_eq!(errs.len(), 1);
        assert!(matches!(&errs[0], AnalysisError::AssignToImmutable { .. }));
    }

    #[test]
    fn assign_to_mut_is_ok() {
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_ty(),
                    value: Expr::Int(1, s()),
                    mutable: true,
                    span: s(),
                },
                Stmt::Assign {
                    target: Expr::Ident("x".into(), s()),
                    value: Expr::Int(2, s()),
                    span: s(),
                },
            ],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
    }
}
