use crate::analyzer::env::{Env, Symbol};
use crate::analyzer::error::AnalysisError;
use crate::analyzer::resolve::resolve_type_expr;
use crate::analyzer::ty::{Ty, TypeRegistry};
use crate::diagnostics::Span;
use crate::parser::ast::{BinOp, ClosureBody, Expr, UnOp};

pub fn infer_expr(
    expr: &Expr,
    env: &Env,
    registry: &TypeRegistry,
    errors: &mut Vec<AnalysisError>,
) -> Ty {
    match expr {
        Expr::Int(_, _) => Ty::Int,
        Expr::Float(_, _) => Ty::Float,
        Expr::Bool(_, _) => Ty::Bool,
        Expr::Str(_, _) => Ty::Str,

        Expr::Ident(name, span) => match env.lookup(name) {
            Some(Symbol::Var { ty, .. }) => ty.clone(),
            Some(Symbol::Const { ty, .. }) => ty.clone(),
            Some(Symbol::Fn { params, ret, .. }) => {
                let ptys = params.iter().map(|(_, t)| t.clone()).collect();
                Ty::Callable(ptys, Box::new(ret.clone()))
            }
            _ => {
                errors.push(AnalysisError::UndefinedName {
                    name: name.clone(),
                    span: *span,
                });
                Ty::Unknown
            }
        },

        Expr::Tuple(elems, _) => Ty::Tuple(
            elems
                .iter()
                .map(|e| infer_expr(e, env, registry, errors))
                .collect(),
        ),

        Expr::BinOp {
            op,
            left,
            right,
            span,
        } => {
            let lt = infer_expr(left, env, registry, errors);
            let rt = infer_expr(right, env, registry, errors);
            infer_binop(op.clone(), lt, rt, span, errors)
        }

        Expr::UnOp { op, operand, span } => {
            let t = infer_expr(operand, env, registry, errors);
            match op {
                UnOp::Neg => {
                    if !matches!(t, Ty::Int | Ty::Float | Ty::Unknown) {
                        errors.push(AnalysisError::TypeMismatch {
                            expected: "int or float".into(),
                            found: t.to_string(),
                            span: *span,
                        });
                        return Ty::Unknown;
                    }
                    t
                }
                UnOp::Not => {
                    if !matches!(t, Ty::Bool | Ty::Unknown) {
                        errors.push(AnalysisError::TypeMismatch {
                            expected: "bool".into(),
                            found: t.to_string(),
                            span: *span,
                        });
                    }
                    Ty::Bool
                }
            }
        }

        Expr::As { expr, ty, .. } => {
            let _ = infer_expr(expr, env, registry, errors);
            resolve_type_expr(ty, env, registry, errors)
        }

        Expr::Unwrap(inner, span) => match infer_expr(inner, env, registry, errors) {
            Ty::Option(inner) => *inner,
            Ty::Unknown => Ty::Unknown,
            other => {
                errors.push(AnalysisError::TypeMismatch {
                    expected: "Option[T]".into(),
                    found: other.to_string(),
                    span: *span,
                });
                Ty::Unknown
            }
        },

        Expr::Call { callee, args, span } => {
            let callee_ty = infer_expr(callee, env, registry, errors);
            match callee_ty {
                Ty::Callable(params, ret) => {
                    if params.len() != args.len() {
                        errors.push(AnalysisError::TypeMismatch {
                            expected: format!("{} argument(s)", params.len()),
                            found: format!("{} argument(s)", args.len()),
                            span: *span,
                        });
                        return Ty::Unknown;
                    }
                    for (param_ty, arg) in params.iter().zip(args.iter()) {
                        let arg_ty = infer_expr(arg, env, registry, errors);
                        check_assignable(param_ty, &arg_ty, span, errors);
                    }
                    *ret
                }
                Ty::Unknown => Ty::Unknown,
                _ => Ty::Unknown, // method calls on named types resolved later
            }
        }

        Expr::Field { object, .. } => {
            let _ = infer_expr(object, env, registry, errors);
            Ty::Unknown // full struct field resolution requires type layout
        }

        Expr::Index { object, index, .. } => {
            let obj_ty = infer_expr(object, env, registry, errors);
            let _ = infer_expr(index, env, registry, errors);
            match obj_ty {
                Ty::Vec(elem) => *elem,
                Ty::Map(_, v) => *v,
                _ => Ty::Unknown,
            }
        }

        Expr::StructLiteral { ty, span, .. } => match env.lookup(ty) {
            Some(Symbol::Type { id, .. }) => Ty::Named(id.clone(), ty.clone()),
            _ => {
                errors.push(AnalysisError::UndefinedName {
                    name: ty.clone(),
                    span: *span,
                });
                Ty::Unknown
            }
        },

        Expr::Match {
            scrutinee, arms, ..
        } => {
            let _ = infer_expr(scrutinee, env, registry, errors);
            arms.iter()
                .map(|a| infer_expr(&a.body, env, registry, errors))
                .find(|t| *t != Ty::Unknown)
                .unwrap_or(Ty::Unknown)
        }

        Expr::Closure { params, body, .. } => {
            let ptys = params
                .iter()
                .map(|p| resolve_type_expr(&p.ty, env, registry, errors))
                .collect();
            let ret = match body {
                ClosureBody::Expr(e) => infer_expr(e, env, registry, errors),
                ClosureBody::Block(_) => Ty::Unknown,
            };
            Ty::Callable(ptys, Box::new(ret))
        }

        Expr::Spawn(inner, _) => {
            let _ = infer_expr(inner, env, registry, errors);
            Ty::Unknown // Task[T] resolved later
        }

        Expr::Ref { expr, mutable, .. } => {
            let inner = infer_expr(expr, env, registry, errors);
            Ty::Ref(Box::new(inner), *mutable)
        }

        Expr::Gen { .. } | Expr::GenSplice(_, _) => Ty::Unknown,
    }
}

fn infer_binop(op: BinOp, lt: Ty, rt: Ty, span: &Span, errors: &mut Vec<AnalysisError>) -> Ty {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Div => match (&lt, &rt) {
            (Ty::Int, Ty::Int) => Ty::Int,
            (Ty::Float, Ty::Float) => Ty::Float,
            (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => Ty::Float,
            (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
            _ => {
                errors.push(AnalysisError::TypeMismatch {
                    expected: "numeric types".into(),
                    found: format!("{lt} and {rt}"),
                    span: *span,
                });
                Ty::Unknown
            }
        },
        Eq | Lt | Gt | LtEq | GtEq | Spaceship => Ty::Bool,
        And | Or => {
            if !matches!(lt, Ty::Bool | Ty::Unknown) || !matches!(rt, Ty::Bool | Ty::Unknown) {
                errors.push(AnalysisError::TypeMismatch {
                    expected: "bool".into(),
                    found: format!("{lt} and {rt}"),
                    span: *span,
                });
            }
            Ty::Bool
        }
        Pipe => Ty::Unknown,
    }
}

/// Check that `found` is assignable to `expected`.
/// Emits a TypeMismatch error if not. Silently accepts Unknown on either side.
pub fn check_assignable(expected: &Ty, found: &Ty, span: &Span, errors: &mut Vec<AnalysisError>) {
    if matches!(found, Ty::Unknown) || matches!(expected, Ty::Unknown) {
        return;
    }
    // int is assignable to float
    if *expected == Ty::Float && *found == Ty::Int {
        return;
    }
    if expected != found {
        errors.push(AnalysisError::TypeMismatch {
            expected: expected.to_string(),
            found: found.to_string(),
            span: *span,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::env::Env;
    use crate::analyzer::ty::{Ty, TypeRegistry};
    use crate::diagnostics::Span;
    use crate::parser::ast::{BinOp, Expr, UnOp};
    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    fn fresh() -> (Env, TypeRegistry) {
        (Env::new(), TypeRegistry::new())
    }

    #[test]
    fn literals() {
        let (env, reg) = fresh();
        let mut errs = vec![];
        assert_eq!(
            infer_expr(&Expr::Int(1, s()), &env, &reg, &mut errs),
            Ty::Int
        );
        assert_eq!(
            infer_expr(&Expr::Float(1.0, s()), &env, &reg, &mut errs),
            Ty::Float
        );
        assert_eq!(
            infer_expr(&Expr::Bool(true, s()), &env, &reg, &mut errs),
            Ty::Bool
        );
        assert!(errs.is_empty());
    }

    #[test]
    fn int_plus_int_is_int() {
        let (env, reg) = fresh();
        let mut errs = vec![];
        let expr = Expr::BinOp {
            op: BinOp::Add,
            left: Box::new(Expr::Int(1, s())),
            right: Box::new(Expr::Int(2, s())),
            span: s(),
        };
        assert_eq!(infer_expr(&expr, &env, &reg, &mut errs), Ty::Int);
        assert!(errs.is_empty());
    }

    #[test]
    fn int_plus_float_promotes_to_float() {
        let (env, reg) = fresh();
        let mut errs = vec![];
        let expr = Expr::BinOp {
            op: BinOp::Add,
            left: Box::new(Expr::Int(1, s())),
            right: Box::new(Expr::Float(2.0, s())),
            span: s(),
        };
        assert_eq!(infer_expr(&expr, &env, &reg, &mut errs), Ty::Float);
    }

    #[test]
    fn comparison_yields_bool() {
        let (env, reg) = fresh();
        let mut errs = vec![];
        let expr = Expr::BinOp {
            op: BinOp::Eq,
            left: Box::new(Expr::Int(1, s())),
            right: Box::new(Expr::Int(1, s())),
            span: s(),
        };
        assert_eq!(infer_expr(&expr, &env, &reg, &mut errs), Ty::Bool);
    }

    #[test]
    fn unary_neg_on_int_is_int() {
        let (env, reg) = fresh();
        let mut errs = vec![];
        let expr = Expr::UnOp {
            op: UnOp::Neg,
            operand: Box::new(Expr::Int(5, s())),
            span: s(),
        };
        assert_eq!(infer_expr(&expr, &env, &reg, &mut errs), Ty::Int);
    }

    #[test]
    fn string_literal_is_str() {
        let (env, reg) = fresh();
        let mut errs = vec![];
        let expr = Expr::Str(vec![], s());
        assert_eq!(infer_expr(&expr, &env, &reg, &mut errs), Ty::Str);
    }
}
