use crate::analyzer::env::{Env, Symbol};
use crate::analyzer::error::AnalysisError;
use crate::analyzer::infer::{check_assignable, infer_expr};
use crate::analyzer::resolve::resolve_type_expr;
use crate::analyzer::ty::{Ty, TypeRegistry};
use crate::parser::ast::{Block, Expr, Stmt};

/// Check all statements in `block`. Opens and closes a new scope automatically.
pub fn check_block(
    block: &Block,
    env: &mut Env,
    registry: &TypeRegistry,
    return_ty: &Ty,
    errors: &mut Vec<AnalysisError>,
) {
    env.push_scope();
    for stmt in &block.stmts {
        check_stmt(stmt, env, registry, return_ty, errors);
    }
    env.pop_scope();
}

fn check_stmt(
    stmt: &Stmt,
    env: &mut Env,
    registry: &TypeRegistry,
    return_ty: &Ty,
    errors: &mut Vec<AnalysisError>,
) {
    match stmt {
        Stmt::VarDecl {
            name,
            ty,
            value,
            mutable,
            span,
        } => {
            // Shadowing is allowed in Kiln: redeclaring a name creates a new binding.
            let declared = resolve_type_expr(ty, env, registry, errors);
            let found = infer_expr(value, env, registry, errors);
            check_assignable(&declared, &found, &value.span(), errors);
            env.define(
                name,
                Symbol::Var {
                    ty: declared,
                    mutable: *mutable,
                    span: *span,
                },
            );
        }

        Stmt::Assign {
            target,
            value,
            span: _,
        } => {
            // Check that the target binding is mutable (if it's a plain ident).
            if let Expr::Ident(name, ident_span) = target {
                match env.lookup(name) {
                    Some(Symbol::Var { mutable: false, .. }) => {
                        errors.push(AnalysisError::AssignToImmutable {
                            name: name.clone(),
                            span: *ident_span,
                        });
                        return;
                    }
                    None => {
                        errors.push(AnalysisError::UndefinedName {
                            name: name.clone(),
                            span: *ident_span,
                        });
                        return;
                    }
                    _ => {}
                }
            }
            let target_ty = infer_expr(target, env, registry, errors);
            let value_ty = infer_expr(value, env, registry, errors);
            check_assignable(&target_ty, &value_ty, &value.span(), errors);
        }

        Stmt::Return { value, span } => {
            let (found, value_span) = match value {
                Some(v) => (infer_expr(v, env, registry, errors), v.span()),
                None => (Ty::Void, *span),
            };
            check_assignable(return_ty, &found, &value_span, errors);
        }

        Stmt::Raise { value, .. } => {
            if let Some(v) = value {
                infer_expr(v, env, registry, errors);
            }
        }

        Stmt::Expr(expr) => {
            infer_expr(expr, env, registry, errors);
        }

        Stmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (cond, block) in branches {
                let cond_ty = infer_expr(cond, env, registry, errors);
                if !matches!(cond_ty, Ty::Bool | Ty::Unknown) {
                    errors.push(AnalysisError::TypeMismatch {
                        expected: "bool".into(),
                        found: cond_ty.to_string(),
                        span: cond.span(),
                    });
                }
                check_block(block, env, registry, return_ty, errors);
            }
            if let Some(eb) = else_branch {
                check_block(eb, env, registry, return_ty, errors);
            }
        }

        Stmt::While { cond, body, span } => {
            let cond_ty = infer_expr(cond, env, registry, errors);
            if !matches!(cond_ty, Ty::Bool | Ty::Unknown) {
                errors.push(AnalysisError::TypeMismatch {
                    expected: "bool".into(),
                    found: cond_ty.to_string(),
                    span: *span,
                });
            }
            check_block(body, env, registry, return_ty, errors);
        }

        Stmt::DoWhile { body, cond, span } => {
            check_block(body, env, registry, return_ty, errors);
            let cond_ty = infer_expr(cond, env, registry, errors);
            if !matches!(cond_ty, Ty::Bool | Ty::Unknown) {
                errors.push(AnalysisError::TypeMismatch {
                    expected: "bool".into(),
                    found: cond_ty.to_string(),
                    span: *span,
                });
            }
        }

        Stmt::For {
            binding,
            binding_ty,
            iterable,
            body,
            span,
        } => {
            let iter_ty = infer_expr(iterable, env, registry, errors);
            let elem_ty = match &iter_ty {
                Ty::Vec(t) | Ty::Set(t) => *t.clone(),
                Ty::Str => Ty::Str,
                _ => Ty::Unknown,
            };
            if let Some(ann) = binding_ty {
                let ann_ty = resolve_type_expr(ann, env, registry, errors);
                check_assignable(&ann_ty, &elem_ty, span, errors);
            }
            env.push_scope();
            env.define(
                binding,
                Symbol::Var {
                    ty: elem_ty,
                    mutable: false,
                    span: *span,
                },
            );
            for s in &body.stmts {
                check_stmt(s, env, registry, return_ty, errors);
            }
            env.pop_scope();
        }

        Stmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            check_block(body, env, registry, return_ty, errors);
            for handler in handlers {
                let exc_ty = resolve_type_expr(&handler.ty, env, registry, errors);
                env.push_scope();
                env.define(
                    &handler.binding,
                    Symbol::Var {
                        ty: exc_ty,
                        mutable: false,
                        span: handler.span,
                    },
                );
                check_block(&handler.body, env, registry, return_ty, errors);
                env.pop_scope();
            }
            if let Some(fb) = finally {
                check_block(fb, env, registry, return_ty, errors);
            }
        }

        Stmt::FnDef(f) => {
            let ret = resolve_type_expr(&f.return_type, env, registry, errors);
            let params: Vec<(String, Ty)> = f
                .params
                .iter()
                .map(|p| {
                    (
                        p.name.clone(),
                        resolve_type_expr(&p.ty, env, registry, errors),
                    )
                })
                .collect();
            // Register in current scope so the function can recurse
            env.define(
                &f.name,
                Symbol::Fn {
                    generic_params: f.generic_params.iter().map(|g| g.name.clone()).collect(),
                    params: params.clone(),
                    ret: ret.clone(),
                    span: f.span,
                },
            );
            env.push_scope();
            for (pname, pty) in &params {
                env.define(
                    pname,
                    Symbol::Var {
                        ty: pty.clone(),
                        mutable: false,
                        span: f.span,
                    },
                );
            }
            check_block(&f.body, env, registry, &ret, errors);
            env.pop_scope();
        }

        Stmt::Break(_) | Stmt::Continue(_) => {}
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

    fn int_type() -> TypeExpr {
        TypeExpr::Named {
            name: "int".into(),
            generics: vec![],
            span: s(),
        }
    }
    fn bool_type() -> TypeExpr {
        TypeExpr::Named {
            name: "bool".into(),
            generics: vec![],
            span: s(),
        }
    }

    #[test]
    fn var_decl_ok() {
        let block = Block {
            stmts: vec![Stmt::VarDecl {
                name: "x".into(),
                ty: int_type(),
                value: Expr::Int(42, s()),
                mutable: false,
                span: s(),
            }],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn var_decl_type_mismatch() {
        let block = Block {
            stmts: vec![Stmt::VarDecl {
                name: "x".into(),
                ty: bool_type(),
                value: Expr::Int(1, s()),
                mutable: false,
                span: s(),
            }],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert_eq!(errs.len(), 1);
    }

    // Shadowing is ALLOWED in Kiln (new design).
    #[test]
    fn shadowing_is_allowed() {
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_type(),
                    value: Expr::Int(1, s()),
                    mutable: false,
                    span: s(),
                },
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_type(),
                    value: Expr::Int(2, s()),
                    mutable: false,
                    span: s(),
                },
            ],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(errs.is_empty(), "shadowing should be allowed: {errs:?}");
    }

    // Assigning to an immutable binding is an error.
    #[test]
    fn assign_to_immutable_is_error() {
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_type(),
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
        check_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert_eq!(errs.len(), 1);
        assert!(matches!(&errs[0], AnalysisError::AssignToImmutable { .. }));
    }

    // Assigning to a mut binding is fine.
    #[test]
    fn assign_to_mut_is_ok() {
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_type(),
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
        check_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
    }
}
