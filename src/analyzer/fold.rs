use crate::analyzer::error::AnalysisError;
use crate::analyzer::typed_ast::{
    TypedBlock, TypedCatchHandler, TypedClosureBody, TypedExpr, TypedExprKind, TypedFile,
    TypedFnDef, TypedHookDef, TypedItem, TypedMatchArm, TypedStmt, TypedStringSegment,
};
use crate::diagnostics::Span;
use crate::parser::ast::{BinOp, UnOp};

#[allow(dead_code)]
fn zero_span() -> Span {
    Span { start: 0, end: 0 }
}

fn fmt_binop_op(op: &BinOp) -> &'static str {
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
        BinOp::Pipe => "|",
    }
}

fn fmt_unop_op(op: &UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
        UnOp::Pos => "+",
    }
}

fn fmt_expr_short(e: &TypedExpr) -> String {
    match &e.kind {
        TypedExprKind::Int(n) => n.to_string(),
        TypedExprKind::Float(f) => {
            if *f == f.floor() && f.abs() < 1e15_f64 {
                format!("{:.1}", f)
            } else {
                format!("{}", f)
            }
        }
        TypedExprKind::Bool(b) => b.to_string(),
        TypedExprKind::Str(segs) => {
            if segs.len() == 1 {
                if let TypedStringSegment::Text(t) = &segs[0] {
                    if t.len() <= 24 {
                        return format!("'{}'", t);
                    }
                }
            }
            "<str>".into()
        }
        TypedExprKind::Ident(n) => n.clone(),
        TypedExprKind::BinOp { op, left, right } => {
            format!(
                "({} {} {})",
                fmt_expr_short(left),
                fmt_binop_op(op),
                fmt_expr_short(right)
            )
        }
        TypedExprKind::UnOp { op, operand } => {
            format!("{}{}", fmt_unop_op(op), fmt_expr_short(operand))
        }
        _ => "<expr>".into(),
    }
}

macro_rules! fold_note {
    ($before:expr, $after:expr) => {
        crate::analyzer::opt_notes::note(format!("fold: {} -> {}", $before, $after));
    };
}

/// Recursively fold constant expressions in a TypedExpr.
pub fn fold_expr(expr: TypedExpr) -> TypedExpr {
    let span = expr.span;
    let ty = expr.ty.clone();

    match expr.kind {
        TypedExprKind::BinOp { op, left, right } => {
            let left = fold_expr(*left);
            let right = fold_expr(*right);
            try_fold_binop(op, left, right, ty, span)
        }
        TypedExprKind::UnOp { op, operand } => {
            let operand = fold_expr(*operand);
            try_fold_unop(op, operand, ty, span)
        }
        TypedExprKind::Call {
            callee,
            args,
            fn_name,
            generic_bounds,
            generic_params,
            param_tys,
        } => {
            let callee = fold_expr(*callee);
            let args = args.into_iter().map(fold_expr).collect();
            TypedExpr {
                kind: TypedExprKind::Call {
                    callee: Box::new(callee),
                    args,
                    fn_name,
                    generic_bounds,
                    generic_params,
                    param_tys,
                },
                ty,
                span,
            }
        }
        TypedExprKind::MethodCall {
            object,
            method_fn,
            args,
        } => {
            // Compile-time string length: "hello".len() -> Int(5)
            let object_folded = fold_expr(*object);
            let args_folded: Vec<TypedExpr> = args.into_iter().map(fold_expr).collect();
            if method_fn == "len" && args_folded.is_empty() {
                if let TypedExprKind::Str(ref segs) = object_folded.kind {
                    let all_text = segs
                        .iter()
                        .all(|s| matches!(s, TypedStringSegment::Text(_)));
                    if all_text {
                        let total: usize = segs
                            .iter()
                            .map(|s| {
                                if let TypedStringSegment::Text(t) = s {
                                    t.len()
                                } else {
                                    0
                                }
                            })
                            .sum();
                        fold_note!(format!("{}.len()", fmt_expr_short(&object_folded)), total);
                        return TypedExpr {
                            kind: TypedExprKind::Int(total as i64),
                            ty: crate::analyzer::ty::Ty::Int,
                            span,
                        };
                    }
                }
            }
            TypedExpr {
                kind: TypedExprKind::MethodCall {
                    object: Box::new(object_folded),
                    method_fn,
                    args: args_folded,
                },
                ty,
                span,
            }
        }
        TypedExprKind::StaticCall { method_fn, args } => {
            let args = args.into_iter().map(fold_expr).collect();
            TypedExpr {
                kind: TypedExprKind::StaticCall { method_fn, args },
                ty,
                span,
            }
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            let fat_ptr = fold_expr(*fat_ptr);
            let args = args.into_iter().map(fold_expr).collect();
            TypedExpr {
                kind: TypedExprKind::IndirectCall {
                    fat_ptr: Box::new(fat_ptr),
                    args,
                },
                ty,
                span,
            }
        }
        TypedExprKind::Field { object, field } => TypedExpr {
            kind: TypedExprKind::Field {
                object: Box::new(fold_expr(*object)),
                field,
            },
            ty,
            span,
        },
        TypedExprKind::Index { object, index } => TypedExpr {
            kind: TypedExprKind::Index {
                object: Box::new(fold_expr(*object)),
                index: Box::new(fold_expr(*index)),
            },
            ty,
            span,
        },
        TypedExprKind::Tuple(exprs) => TypedExpr {
            kind: TypedExprKind::Tuple(exprs.into_iter().map(fold_expr).collect()),
            ty,
            span,
        },
        TypedExprKind::StructLiteral { ty_name, fields } => TypedExpr {
            kind: TypedExprKind::StructLiteral {
                ty_name,
                fields: fields.into_iter().map(|(k, v)| (k, fold_expr(v))).collect(),
            },
            ty,
            span,
        },
        TypedExprKind::Unwrap(inner) => TypedExpr {
            kind: TypedExprKind::Unwrap(Box::new(fold_expr(*inner))),
            ty,
            span,
        },
        TypedExprKind::As {
            expr: inner,
            ty: cast_ty,
        } => {
            let inner = fold_expr(*inner);
            // Redundant cast elimination: if inner already has the same type, skip cast.
            if inner.ty == cast_ty {
                fold_note!(
                    format!("{} as {:?}", fmt_expr_short(&inner), cast_ty),
                    fmt_expr_short(&inner)
                );
                inner
            } else {
                TypedExpr {
                    kind: TypedExprKind::As {
                        expr: Box::new(inner),
                        ty: cast_ty,
                    },
                    ty,
                    span,
                }
            }
        }
        TypedExprKind::Match { scrutinee, arms } => {
            let scrutinee = fold_expr(*scrutinee);
            let arms = arms
                .into_iter()
                .map(|arm| TypedMatchArm {
                    pattern: arm.pattern,
                    guard: arm.guard.map(fold_expr),
                    body: fold_expr(arm.body),
                    narrowed_discriminant: arm.narrowed_discriminant,
                    span: arm.span,
                })
                .collect();
            TypedExpr {
                kind: TypedExprKind::Match {
                    scrutinee: Box::new(scrutinee),
                    arms,
                },
                ty,
                span,
            }
        }
        TypedExprKind::Closure { params, body } => {
            let body = match body {
                TypedClosureBody::Expr(e) => TypedClosureBody::Expr(Box::new(fold_expr(*e))),
                TypedClosureBody::Block(b) => TypedClosureBody::Block(fold_block(b)),
            };
            TypedExpr {
                kind: TypedExprKind::Closure { params, body },
                ty,
                span,
            }
        }
        TypedExprKind::Spawn(e) => TypedExpr {
            kind: TypedExprKind::Spawn(Box::new(fold_expr(*e))),
            ty,
            span,
        },
        TypedExprKind::Ref {
            mutable,
            expr: inner,
        } => TypedExpr {
            kind: TypedExprKind::Ref {
                mutable,
                expr: Box::new(fold_expr(*inner)),
            },
            ty,
            span,
        },
        TypedExprKind::Array(exprs) => TypedExpr {
            kind: TypedExprKind::Array(exprs.into_iter().map(fold_expr).collect()),
            ty,
            span,
        },
        TypedExprKind::Gen { body } => TypedExpr {
            kind: TypedExprKind::Gen {
                body: fold_block(body),
            },
            ty,
            span,
        },
        TypedExprKind::GenSplice(e) => TypedExpr {
            kind: TypedExprKind::GenSplice(Box::new(fold_expr(*e))),
            ty,
            span,
        },
        // Merge adjacent Text segments within a string literal and fold interp exprs
        TypedExprKind::Str(segs) => {
            let mut merged: Vec<TypedStringSegment> = Vec::new();
            for seg in segs {
                match seg {
                    TypedStringSegment::Text(t) => {
                        if let Some(TypedStringSegment::Text(prev)) = merged.last_mut() {
                            prev.push_str(&t);
                        } else {
                            merged.push(TypedStringSegment::Text(t));
                        }
                    }
                    TypedStringSegment::Interp(e) => {
                        merged.push(TypedStringSegment::Interp(fold_expr(e)));
                    }
                }
            }
            TypedExpr {
                kind: TypedExprKind::Str(merged),
                ty,
                span,
            }
        }
        // Leaf expressions -- unchanged
        other => TypedExpr {
            kind: other,
            ty,
            span,
        },
    }
}

fn try_fold_binop(
    op: BinOp,
    left: TypedExpr,
    right: TypedExpr,
    ty: crate::analyzer::ty::Ty,
    span: Span,
) -> TypedExpr {
    let left_kind = &left.kind;
    let right_kind = &right.kind;

    // Algebraic identity and simplification rules
    match (&op, left_kind, right_kind) {
        // x + 0 = x
        (BinOp::Add, _, TypedExprKind::Int(0)) => {
            fold_note!(fmt_expr_short(&left) + " + 0", fmt_expr_short(&left));
            return left;
        }
        // 0 + x = x
        (BinOp::Add, TypedExprKind::Int(0), _) => {
            fold_note!(
                "0 + ".to_owned() + &fmt_expr_short(&right),
                fmt_expr_short(&right)
            );
            return right;
        }
        // x - 0 = x
        (BinOp::Sub, _, TypedExprKind::Int(0)) => {
            fold_note!(fmt_expr_short(&left) + " - 0", fmt_expr_short(&left));
            return left;
        }
        // x - x = 0 (only safe for Ident)
        (BinOp::Sub, TypedExprKind::Ident(a), TypedExprKind::Ident(b)) if a == b => {
            fold_note!(format!("{} - {}", a, b), "0");
            return TypedExpr {
                kind: TypedExprKind::Int(0),
                ty,
                span,
            };
        }
        // x * 1 = x
        (BinOp::Mul, _, TypedExprKind::Int(1)) => {
            fold_note!(fmt_expr_short(&left) + " * 1", fmt_expr_short(&left));
            return left;
        }
        // 1 * x = x
        (BinOp::Mul, TypedExprKind::Int(1), _) => {
            fold_note!(
                "1 * ".to_owned() + &fmt_expr_short(&right),
                fmt_expr_short(&right)
            );
            return right;
        }
        // x * 0 = 0
        (BinOp::Mul, _, TypedExprKind::Int(0)) | (BinOp::Mul, TypedExprKind::Int(0), _) => {
            fold_note!(
                format!("{} * {}", fmt_expr_short(&left), fmt_expr_short(&right)),
                "0"
            );
            return TypedExpr {
                kind: TypedExprKind::Int(0),
                ty,
                span,
            };
        }
        // x / 1 = x
        (BinOp::Div, _, TypedExprKind::Int(1)) => {
            fold_note!(fmt_expr_short(&left) + " / 1", fmt_expr_short(&left));
            return left;
        }
        // x == true -> x, x == false -> !x
        (BinOp::Eq, _, TypedExprKind::Bool(true)) => {
            fold_note!(fmt_expr_short(&left) + " == true", fmt_expr_short(&left));
            return left;
        }
        (BinOp::Eq, TypedExprKind::Bool(true), _) => {
            fold_note!(
                "true == ".to_owned() + &fmt_expr_short(&right),
                fmt_expr_short(&right)
            );
            return right;
        }
        (BinOp::Eq, _, TypedExprKind::Bool(false)) => {
            fold_note!(
                fmt_expr_short(&left) + " == false",
                format!("!{}", fmt_expr_short(&left))
            );
            return TypedExpr {
                kind: TypedExprKind::UnOp {
                    op: UnOp::Not,
                    operand: Box::new(left),
                },
                ty,
                span,
            };
        }
        (BinOp::Eq, TypedExprKind::Bool(false), _) => {
            fold_note!(
                "false == ".to_owned() + &fmt_expr_short(&right),
                format!("!{}", fmt_expr_short(&right))
            );
            return TypedExpr {
                kind: TypedExprKind::UnOp {
                    op: UnOp::Not,
                    operand: Box::new(right),
                },
                ty,
                span,
            };
        }
        // Boolean short-circuit: false && expr -> false (if no side effects in expr)
        (BinOp::And, TypedExprKind::Bool(false), _) if !expr_has_side_effects(&right) => {
            fold_note!(format!("false && {}", fmt_expr_short(&right)), "false");
            return TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty,
                span,
            };
        }
        // true && x -> x (true is identity for &&)
        (BinOp::And, TypedExprKind::Bool(true), _) => {
            fold_note!(
                "true && ".to_owned() + &fmt_expr_short(&right),
                fmt_expr_short(&right)
            );
            return right;
        }
        // true || expr -> true (if no side effects)
        (BinOp::Or, TypedExprKind::Bool(true), _) if !expr_has_side_effects(&right) => {
            fold_note!(format!("true || {}", fmt_expr_short(&right)), "true");
            return TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty,
                span,
            };
        }
        // false || x -> x (false is identity for ||)
        (BinOp::Or, TypedExprKind::Bool(false), _) => {
            fold_note!(
                "false || ".to_owned() + &fmt_expr_short(&right),
                fmt_expr_short(&right)
            );
            return right;
        }
        // 0 - x -> -x
        (BinOp::Sub, TypedExprKind::Int(0), _) => {
            fold_note!(
                format!("0 - {}", fmt_expr_short(&right)),
                format!("-{}", fmt_expr_short(&right))
            );
            return TypedExpr {
                kind: TypedExprKind::UnOp {
                    op: UnOp::Neg,
                    operand: Box::new(right),
                },
                ty,
                span,
            };
        }
        // x * -1 -> -x
        (BinOp::Mul, _, TypedExprKind::Int(-1)) => {
            fold_note!(
                format!("{} * -1", fmt_expr_short(&left)),
                format!("-{}", fmt_expr_short(&left))
            );
            return TypedExpr {
                kind: TypedExprKind::UnOp {
                    op: UnOp::Neg,
                    operand: Box::new(left),
                },
                ty,
                span,
            };
        }
        (BinOp::Mul, TypedExprKind::Int(-1), _) => {
            fold_note!(
                format!("-1 * {}", fmt_expr_short(&right)),
                format!("-{}", fmt_expr_short(&right))
            );
            return TypedExpr {
                kind: TypedExprKind::UnOp {
                    op: UnOp::Neg,
                    operand: Box::new(right),
                },
                ty,
                span,
            };
        }
        // x % 1 = 0 for integers
        (BinOp::Mod, _, TypedExprKind::Int(1)) => {
            fold_note!(fmt_expr_short(&left) + " % 1", "0");
            return TypedExpr {
                kind: TypedExprKind::Int(0),
                ty,
                span,
            };
        }
        // Self-comparison identities for idents
        (BinOp::Eq, TypedExprKind::Ident(a), TypedExprKind::Ident(b)) if a == b => {
            fold_note!(format!("{} == {}", a, b), "true");
            return TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty,
                span,
            };
        }
        (BinOp::Ne, TypedExprKind::Ident(a), TypedExprKind::Ident(b)) if a == b => {
            fold_note!(format!("{} != {}", a, b), "false");
            return TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty,
                span,
            };
        }
        (BinOp::Lt, TypedExprKind::Ident(a), TypedExprKind::Ident(b)) if a == b => {
            fold_note!(format!("{} < {}", a, b), "false");
            return TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty,
                span,
            };
        }
        (BinOp::Gt, TypedExprKind::Ident(a), TypedExprKind::Ident(b)) if a == b => {
            fold_note!(format!("{} > {}", a, b), "false");
            return TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty,
                span,
            };
        }
        (BinOp::LtEq, TypedExprKind::Ident(a), TypedExprKind::Ident(b)) if a == b => {
            fold_note!(format!("{} <= {}", a, b), "true");
            return TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty,
                span,
            };
        }
        (BinOp::GtEq, TypedExprKind::Ident(a), TypedExprKind::Ident(b)) if a == b => {
            fold_note!(format!("{} >= {}", a, b), "true");
            return TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty,
                span,
            };
        }
        // Float algebraic identities
        (BinOp::Add, _, TypedExprKind::Float(f)) if *f == 0.0 => {
            fold_note!(fmt_expr_short(&left) + " + 0.0", fmt_expr_short(&left));
            return left;
        }
        (BinOp::Add, TypedExprKind::Float(f), _) if *f == 0.0 => {
            fold_note!(
                "0.0 + ".to_owned() + &fmt_expr_short(&right),
                fmt_expr_short(&right)
            );
            return right;
        }
        (BinOp::Sub, _, TypedExprKind::Float(f)) if *f == 0.0 => {
            fold_note!(fmt_expr_short(&left) + " - 0.0", fmt_expr_short(&left));
            return left;
        }
        (BinOp::Mul, _, TypedExprKind::Float(f)) if *f == 1.0 => {
            fold_note!(fmt_expr_short(&left) + " * 1.0", fmt_expr_short(&left));
            return left;
        }
        (BinOp::Mul, TypedExprKind::Float(f), _) if *f == 1.0 => {
            fold_note!(
                "1.0 * ".to_owned() + &fmt_expr_short(&right),
                fmt_expr_short(&right)
            );
            return right;
        }
        (BinOp::Div, _, TypedExprKind::Float(f)) if *f == 1.0 => {
            fold_note!(fmt_expr_short(&left) + " / 1.0", fmt_expr_short(&left));
            return left;
        }
        // x - (-y) = x + y
        (
            BinOp::Sub,
            _,
            TypedExprKind::UnOp {
                op: UnOp::Neg,
                operand: inner,
            },
        ) => {
            fold_note!(
                format!("{} - (-{})", fmt_expr_short(&left), fmt_expr_short(inner)),
                format!("{} + {}", fmt_expr_short(&left), fmt_expr_short(inner))
            );
            return TypedExpr {
                kind: TypedExprKind::BinOp {
                    op: BinOp::Add,
                    left: Box::new(left),
                    right: inner.clone(),
                },
                ty,
                span,
            };
        }
        // x != true -> !x
        (BinOp::Ne, _, TypedExprKind::Bool(true)) => {
            fold_note!(
                fmt_expr_short(&left) + " != true",
                format!("!{}", fmt_expr_short(&left))
            );
            return TypedExpr {
                kind: TypedExprKind::UnOp {
                    op: UnOp::Not,
                    operand: Box::new(left),
                },
                ty,
                span,
            };
        }
        (BinOp::Ne, TypedExprKind::Bool(true), _) => {
            fold_note!(
                "true != ".to_owned() + &fmt_expr_short(&right),
                format!("!{}", fmt_expr_short(&right))
            );
            return TypedExpr {
                kind: TypedExprKind::UnOp {
                    op: UnOp::Not,
                    operand: Box::new(right),
                },
                ty,
                span,
            };
        }
        // x != false -> x
        (BinOp::Ne, _, TypedExprKind::Bool(false)) => {
            fold_note!(fmt_expr_short(&left) + " != false", fmt_expr_short(&left));
            return left;
        }
        (BinOp::Ne, TypedExprKind::Bool(false), _) => {
            fold_note!(
                "false != ".to_owned() + &fmt_expr_short(&right),
                fmt_expr_short(&right)
            );
            return right;
        }
        // Modulo self: x % x = 0 for idents (safe -- even if x=0, result is 0 or div by zero anyway)
        (BinOp::Mod, TypedExprKind::Ident(a), TypedExprKind::Ident(b)) if a == b => {
            fold_note!(format!("{} % {}", a, b), "0");
            return TypedExpr {
                kind: TypedExprKind::Int(0),
                ty,
                span,
            };
        }
        // Idempotent AND: x && x -> x for idents
        (BinOp::And, TypedExprKind::Ident(a), TypedExprKind::Ident(b)) if a == b => {
            fold_note!(format!("{} && {}", a, b), a.clone());
            return left;
        }
        // Idempotent OR: x || x -> x for idents
        (BinOp::Or, TypedExprKind::Ident(a), TypedExprKind::Ident(b)) if a == b => {
            fold_note!(format!("{} || {}", a, b), a.clone());
            return left;
        }
        // x || true -> true (if no side effects in x)
        (BinOp::Or, _, TypedExprKind::Bool(true)) if !expr_has_side_effects(&left) => {
            fold_note!(format!("{} || true", fmt_expr_short(&left)), "true");
            return TypedExpr {
                kind: TypedExprKind::Bool(true),
                ty,
                span,
            };
        }
        // x && false -> false (if no side effects in x)
        (BinOp::And, _, TypedExprKind::Bool(false)) if !expr_has_side_effects(&left) => {
            fold_note!(format!("{} && false", fmt_expr_short(&left)), "false");
            return TypedExpr {
                kind: TypedExprKind::Bool(false),
                ty,
                span,
            };
        }
        _ => {}
    }

    // Float self-comparison and self-difference for ident operands
    match (&op, left_kind, right_kind) {
        (BinOp::Sub, TypedExprKind::Ident(a), TypedExprKind::Ident(b)) if a == b => {
            // Already handled above for the general case (produces Int(0)).
            // For floats we emit Float(0.0) since the type is float.
            // This arm is unreachable for Int because the earlier arm returns first.
            // We check the type to distinguish.
            if ty == crate::analyzer::ty::Ty::Float {
                fold_note!(format!("{} - {}", a, b), "0.0");
                return TypedExpr {
                    kind: TypedExprKind::Float(0.0),
                    ty,
                    span,
                };
            }
        }
        // Strength reduction: x * 2 -> x + x (avoids a multiply for small constant)
        (BinOp::Mul, TypedExprKind::Ident(n), TypedExprKind::Int(2)) => {
            fold_note!(format!("{} * 2", n), format!("{} + {}", n, n));
            return TypedExpr {
                kind: TypedExprKind::BinOp {
                    op: BinOp::Add,
                    left: Box::new(left.clone()),
                    right: Box::new(left),
                },
                ty,
                span,
            };
        }
        // 2 * x -> x + x
        (BinOp::Mul, TypedExprKind::Int(2), TypedExprKind::Ident(n)) => {
            fold_note!(format!("2 * {}", n), format!("{} + {}", n, n));
            return TypedExpr {
                kind: TypedExprKind::BinOp {
                    op: BinOp::Add,
                    left: Box::new(right.clone()),
                    right: Box::new(right),
                },
                ty,
                span,
            };
        }
        _ => {}
    }

    // Fully-constant folding
    match (&op, &left.kind, &right.kind) {
        // Int arithmetic
        (BinOp::Add, TypedExprKind::Int(a), TypedExprKind::Int(b)) => {
            let result = a.wrapping_add(*b);
            fold_note!(format!("{} + {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Int(result),
                ty,
                span,
            };
        }
        (BinOp::Sub, TypedExprKind::Int(a), TypedExprKind::Int(b)) => {
            let result = a.wrapping_sub(*b);
            fold_note!(format!("{} - {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Int(result),
                ty,
                span,
            };
        }
        (BinOp::Mul, TypedExprKind::Int(a), TypedExprKind::Int(b)) => {
            let result = a.wrapping_mul(*b);
            fold_note!(format!("{} * {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Int(result),
                ty,
                span,
            };
        }
        (BinOp::Div, TypedExprKind::Int(a), TypedExprKind::Int(b)) if *b != 0 => {
            let result = a.wrapping_div(*b);
            fold_note!(format!("{} / {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Int(result),
                ty,
                span,
            };
        }
        (BinOp::Mod, TypedExprKind::Int(a), TypedExprKind::Int(b)) if *b != 0 => {
            let result = a.wrapping_rem(*b);
            fold_note!(format!("{} % {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Int(result),
                ty,
                span,
            };
        }
        // Float arithmetic
        (BinOp::Add, TypedExprKind::Float(a), TypedExprKind::Float(b)) => {
            let result = a + b;
            fold_note!(format!("{} + {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Float(result),
                ty,
                span,
            };
        }
        (BinOp::Sub, TypedExprKind::Float(a), TypedExprKind::Float(b)) => {
            let result = a - b;
            fold_note!(format!("{} - {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Float(result),
                ty,
                span,
            };
        }
        (BinOp::Mul, TypedExprKind::Float(a), TypedExprKind::Float(b)) => {
            let result = a * b;
            fold_note!(format!("{} * {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Float(result),
                ty,
                span,
            };
        }
        (BinOp::Div, TypedExprKind::Float(a), TypedExprKind::Float(b)) => {
            let result = a / b;
            fold_note!(format!("{} / {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Float(result),
                ty,
                span,
            };
        }
        // Bool
        (BinOp::And, TypedExprKind::Bool(a), TypedExprKind::Bool(b)) => {
            let result = *a && *b;
            fold_note!(format!("{} && {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Bool(result),
                ty,
                span,
            };
        }
        (BinOp::Or, TypedExprKind::Bool(a), TypedExprKind::Bool(b)) => {
            let result = *a || *b;
            fold_note!(format!("{} || {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Bool(result),
                ty,
                span,
            };
        }
        // Int comparisons
        (BinOp::Eq, TypedExprKind::Int(a), TypedExprKind::Int(b)) => {
            let result = a == b;
            fold_note!(format!("{} == {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Bool(result),
                ty,
                span,
            };
        }
        (BinOp::Ne, TypedExprKind::Int(a), TypedExprKind::Int(b)) => {
            let result = a != b;
            fold_note!(format!("{} != {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Bool(result),
                ty,
                span,
            };
        }
        (BinOp::Lt, TypedExprKind::Int(a), TypedExprKind::Int(b)) => {
            let result = a < b;
            fold_note!(format!("{} < {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Bool(result),
                ty,
                span,
            };
        }
        (BinOp::LtEq, TypedExprKind::Int(a), TypedExprKind::Int(b)) => {
            let result = a <= b;
            fold_note!(format!("{} <= {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Bool(result),
                ty,
                span,
            };
        }
        (BinOp::Gt, TypedExprKind::Int(a), TypedExprKind::Int(b)) => {
            let result = a > b;
            fold_note!(format!("{} > {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Bool(result),
                ty,
                span,
            };
        }
        (BinOp::GtEq, TypedExprKind::Int(a), TypedExprKind::Int(b)) => {
            let result = a >= b;
            fold_note!(format!("{} >= {}", a, b), result);
            return TypedExpr {
                kind: TypedExprKind::Bool(result),
                ty,
                span,
            };
        }
        // Str concat (Pipe operator is used for string concatenation in Kiln)
        (BinOp::Pipe, TypedExprKind::Str(a), TypedExprKind::Str(b)) => {
            // Only fold if both are all-text segments
            if a.iter().all(|s| matches!(s, TypedStringSegment::Text(_)))
                && b.iter().all(|s| matches!(s, TypedStringSegment::Text(_)))
            {
                let a_text: String = a
                    .iter()
                    .filter_map(|s| {
                        if let TypedStringSegment::Text(t) = s {
                            Some(t.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                let b_text: String = b
                    .iter()
                    .filter_map(|s| {
                        if let TypedStringSegment::Text(t) = s {
                            Some(t.as_str())
                        } else {
                            None
                        }
                    })
                    .collect();
                let combined = format!("{}{}", a_text, b_text);
                fold_note!(
                    format!("'{}' | '{}'", a_text, b_text),
                    format!("'{}'", combined)
                );
                return TypedExpr {
                    kind: TypedExprKind::Str(vec![TypedStringSegment::Text(combined)]),
                    ty,
                    span,
                };
            }
        }
        _ => {}
    }

    TypedExpr {
        kind: TypedExprKind::BinOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        },
        ty,
        span,
    }
}

fn try_fold_unop(
    op: UnOp,
    operand: TypedExpr,
    ty: crate::analyzer::ty::Ty,
    span: Span,
) -> TypedExpr {
    match (&op, &operand.kind) {
        // Double negation elimination: --x -> x
        (
            UnOp::Neg,
            TypedExprKind::UnOp {
                op: UnOp::Neg,
                operand: inner,
            },
        ) => {
            fold_note!(
                format!("--{}", fmt_expr_short(inner)),
                fmt_expr_short(inner)
            );
            return *inner.clone();
        }
        // Double not elimination: !!x -> x
        (
            UnOp::Not,
            TypedExprKind::UnOp {
                op: UnOp::Not,
                operand: inner,
            },
        ) => {
            fold_note!(
                format!("!!{}", fmt_expr_short(inner)),
                fmt_expr_short(inner)
            );
            return *inner.clone();
        }
        // Neg normalization: -0 -> 0
        (UnOp::Neg, TypedExprKind::Int(0)) => {
            fold_note!("-0", "0");
            return TypedExpr {
                kind: TypedExprKind::Int(0),
                ty,
                span,
            };
        }
        (UnOp::Neg, TypedExprKind::Float(f)) if *f == 0.0 => {
            fold_note!("-0.0", "0.0");
            return TypedExpr {
                kind: TypedExprKind::Float(0.0),
                ty,
                span,
            };
        }
        // Literal folding
        (UnOp::Neg, TypedExprKind::Int(n)) => {
            let result = -n;
            fold_note!(format!("-{}", n), result);
            return TypedExpr {
                kind: TypedExprKind::Int(result),
                ty,
                span,
            };
        }
        (UnOp::Neg, TypedExprKind::Float(f)) => {
            let result = -f;
            fold_note!(format!("-{}", f), result);
            return TypedExpr {
                kind: TypedExprKind::Float(result),
                ty,
                span,
            };
        }
        (UnOp::Not, TypedExprKind::Bool(b)) => {
            let result = !b;
            fold_note!(format!("!{}", b), result);
            return TypedExpr {
                kind: TypedExprKind::Bool(result),
                ty,
                span,
            };
        }
        // DeMorgan's: !(a && b) -> (!a || !b)
        (
            UnOp::Not,
            TypedExprKind::BinOp {
                op: BinOp::And,
                left: a,
                right: b,
            },
        ) => {
            fold_note!(
                format!("!({} && {})", fmt_expr_short(a), fmt_expr_short(b)),
                format!("!{} || !{}", fmt_expr_short(a), fmt_expr_short(b))
            );
            let not_a = TypedExpr {
                kind: TypedExprKind::UnOp {
                    op: UnOp::Not,
                    operand: a.clone(),
                },
                ty: crate::analyzer::ty::Ty::Bool,
                span,
            };
            let not_b = TypedExpr {
                kind: TypedExprKind::UnOp {
                    op: UnOp::Not,
                    operand: b.clone(),
                },
                ty: crate::analyzer::ty::Ty::Bool,
                span,
            };
            return TypedExpr {
                kind: TypedExprKind::BinOp {
                    op: BinOp::Or,
                    left: Box::new(fold_expr(not_a)),
                    right: Box::new(fold_expr(not_b)),
                },
                ty,
                span,
            };
        }
        // DeMorgan's: !(a || b) -> (!a && !b)
        (
            UnOp::Not,
            TypedExprKind::BinOp {
                op: BinOp::Or,
                left: a,
                right: b,
            },
        ) => {
            fold_note!(
                format!("!({} || {})", fmt_expr_short(a), fmt_expr_short(b)),
                format!("!{} && !{}", fmt_expr_short(a), fmt_expr_short(b))
            );
            let not_a = TypedExpr {
                kind: TypedExprKind::UnOp {
                    op: UnOp::Not,
                    operand: a.clone(),
                },
                ty: crate::analyzer::ty::Ty::Bool,
                span,
            };
            let not_b = TypedExpr {
                kind: TypedExprKind::UnOp {
                    op: UnOp::Not,
                    operand: b.clone(),
                },
                ty: crate::analyzer::ty::Ty::Bool,
                span,
            };
            return TypedExpr {
                kind: TypedExprKind::BinOp {
                    op: BinOp::And,
                    left: Box::new(fold_expr(not_a)),
                    right: Box::new(fold_expr(not_b)),
                },
                ty,
                span,
            };
        }
        _ => {}
    }
    TypedExpr {
        kind: TypedExprKind::UnOp {
            op,
            operand: Box::new(operand),
        },
        ty,
        span,
    }
}

pub fn fold_block(block: TypedBlock) -> TypedBlock {
    let mut stmts: Vec<TypedStmt> = Vec::new();
    for raw in block.stmts {
        // Unreachable code pruning: once a terminator is seen, drop remaining statements.
        if is_block_terminated(&stmts) {
            crate::analyzer::opt_notes::note("fold: unreachable code after terminator eliminated");
            break;
        }
        let folded = fold_stmt(raw);
        // Constant if-branch pruning
        if let TypedStmt::If {
            ref branches,
            ref else_branch,
            ..
        } = folded
        {
            if let Some((cond, body)) = branches.first() {
                match &cond.kind {
                    TypedExprKind::Bool(true) => {
                        crate::analyzer::opt_notes::note(
                            "fold: if true { ... } -> then branch inlined",
                        );
                        stmts.extend(body.stmts.clone());
                        continue;
                    }
                    TypedExprKind::Bool(false) => {
                        if let Some(eb) = else_branch {
                            crate::analyzer::opt_notes::note(
                                "fold: if false { ... } -> else branch inlined",
                            );
                            stmts.extend(eb.stmts.clone());
                        } else {
                            crate::analyzer::opt_notes::note(
                                "fold: if false { ... } -> dead branch eliminated",
                            );
                        }
                        continue;
                    }
                    _ => {}
                }
            }
        }
        // Dead while-false pruning
        if let TypedStmt::While { ref cond, .. } = folded {
            if matches!(cond.kind, TypedExprKind::Bool(false)) {
                crate::analyzer::opt_notes::note(
                    "fold: while false { ... } -> dead loop eliminated",
                );
                continue;
            }
        }
        stmts.push(folded);
    }
    TypedBlock {
        stmts,
        span: block.span,
    }
}

/// Returns true if the last statement in `stmts` is an unconditional terminator
/// (return, raise, break, continue). Statements after such a terminator are dead code.
fn is_block_terminated(stmts: &[TypedStmt]) -> bool {
    matches!(
        stmts.last(),
        Some(
            TypedStmt::Return { .. }
                | TypedStmt::Raise { .. }
                | TypedStmt::Break(_)
                | TypedStmt::Continue(_)
        )
    )
}

fn fold_stmt(stmt: TypedStmt) -> TypedStmt {
    match stmt {
        TypedStmt::VarDecl {
            name,
            ty,
            value,
            mutable,
            span,
        } => TypedStmt::VarDecl {
            name,
            ty,
            value: fold_expr(value),
            mutable,
            span,
        },
        TypedStmt::Assign {
            target,
            value,
            span,
        } => TypedStmt::Assign {
            target: fold_expr(target),
            value: fold_expr(value),
            span,
        },
        TypedStmt::CompoundAssign {
            target,
            op,
            rhs,
            span,
        } => TypedStmt::CompoundAssign {
            target: fold_expr(target),
            op,
            rhs: fold_expr(rhs),
            span,
        },
        TypedStmt::Return { value, span } => TypedStmt::Return {
            value: value.map(fold_expr),
            span,
        },
        TypedStmt::Raise { value, span } => TypedStmt::Raise {
            value: value.map(fold_expr),
            span,
        },
        TypedStmt::If {
            branches,
            else_branch,
            span,
        } => {
            let branches = branches
                .into_iter()
                .map(|(cond, body)| (fold_expr(cond), fold_block(body)))
                .collect::<Vec<_>>();
            TypedStmt::If {
                branches,
                else_branch: else_branch.map(fold_block),
                span,
            }
        }
        TypedStmt::While { cond, body, span } => TypedStmt::While {
            cond: fold_expr(cond),
            body: fold_block(body),
            span,
        },
        TypedStmt::DoWhile { body, cond, span } => TypedStmt::DoWhile {
            body: fold_block(body),
            cond: fold_expr(cond),
            span,
        },
        TypedStmt::For {
            binding,
            binding_ty,
            iterable,
            body,
            iter_ty,
            span,
        } => TypedStmt::For {
            binding,
            binding_ty,
            iterable: fold_expr(iterable),
            body: fold_block(body),
            iter_ty,
            span,
        },
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => TypedStmt::TryCatch {
            body: fold_block(body),
            handlers: handlers
                .into_iter()
                .map(|h| TypedCatchHandler {
                    ty: h.ty,
                    binding: h.binding,
                    body: fold_block(h.body),
                    span: h.span,
                })
                .collect(),
            finally: finally.map(fold_block),
            span,
        },
        TypedStmt::FnDef(f) => TypedStmt::FnDef(fold_fn(f)),
        TypedStmt::Expr(e) => TypedStmt::Expr(fold_expr(e)),
        other => other,
    }
}

fn fold_fn(f: TypedFnDef) -> TypedFnDef {
    TypedFnDef {
        body: fold_block(f.body),
        ..f
    }
}

fn fold_hook(h: TypedHookDef) -> TypedHookDef {
    TypedHookDef {
        body: fold_block(h.body),
        ..h
    }
}

pub fn fold_file(file: TypedFile) -> TypedFile {
    let items = file
        .items
        .into_iter()
        .map(|item| match item {
            TypedItem::Function(f) => TypedItem::Function(fold_fn(f)),
            TypedItem::ImplBlock(mut ib) => {
                ib.methods = ib.methods.into_iter().map(fold_fn).collect();
                ib.hooks = ib.hooks.into_iter().map(fold_hook).collect();
                TypedItem::ImplBlock(ib)
            }
            TypedItem::Global(mut g) => {
                g.init = fold_expr(g.init);
                TypedItem::Global(g)
            }
            other => other,
        })
        .collect();
    TypedFile {
        items,
        span: file.span,
    }
}

/// Check for literal division by zero. Returns errors for each occurrence.
pub fn check_division_by_zero(file: &TypedFile) -> Vec<AnalysisError> {
    let mut errors = Vec::new();
    for item in &file.items {
        match item {
            TypedItem::Function(f) => check_dbz_block(&f.body, &mut errors),
            TypedItem::ImplBlock(ib) => {
                for m in &ib.methods {
                    check_dbz_block(&m.body, &mut errors);
                }
                for h in &ib.hooks {
                    check_dbz_block(&h.body, &mut errors);
                }
            }
            TypedItem::Global(g) => check_dbz_expr(&g.init, &mut errors),
            _ => {}
        }
    }
    errors
}

fn check_dbz_block(block: &TypedBlock, errors: &mut Vec<AnalysisError>) {
    for stmt in &block.stmts {
        check_dbz_stmt(stmt, errors);
    }
}

fn check_dbz_stmt(stmt: &TypedStmt, errors: &mut Vec<AnalysisError>) {
    match stmt {
        TypedStmt::VarDecl { value, .. } => check_dbz_expr(value, errors),
        TypedStmt::Assign { target, value, .. } => {
            check_dbz_expr(target, errors);
            check_dbz_expr(value, errors);
        }
        TypedStmt::CompoundAssign { target, rhs, .. } => {
            check_dbz_expr(target, errors);
            check_dbz_expr(rhs, errors);
        }
        TypedStmt::Return { value: Some(e), .. } => {
            check_dbz_expr(e, errors);
        }
        TypedStmt::Raise { value: Some(e), .. } => {
            check_dbz_expr(e, errors);
        }
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (cond, body) in branches {
                check_dbz_expr(cond, errors);
                check_dbz_block(body, errors);
            }
            if let Some(b) = else_branch {
                check_dbz_block(b, errors);
            }
        }
        TypedStmt::While { cond, body, .. } => {
            check_dbz_expr(cond, errors);
            check_dbz_block(body, errors);
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            check_dbz_block(body, errors);
            check_dbz_expr(cond, errors);
        }
        TypedStmt::For { iterable, body, .. } => {
            check_dbz_expr(iterable, errors);
            check_dbz_block(body, errors);
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            check_dbz_block(body, errors);
            for h in handlers {
                check_dbz_block(&h.body, errors);
            }
            if let Some(b) = finally {
                check_dbz_block(b, errors);
            }
        }
        TypedStmt::FnDef(f) => check_dbz_block(&f.body, errors),
        TypedStmt::Expr(e) => check_dbz_expr(e, errors),
        _ => {}
    }
}

fn check_dbz_expr(expr: &TypedExpr, errors: &mut Vec<AnalysisError>) {
    match &expr.kind {
        TypedExprKind::BinOp { op, left, right } => {
            check_dbz_expr(left, errors);
            check_dbz_expr(right, errors);
            if matches!(op, BinOp::Div | BinOp::Mod) {
                let is_zero = matches!(&right.kind, TypedExprKind::Int(0))
                    || matches!(&right.kind, TypedExprKind::Float(f) if *f == 0.0);
                if is_zero {
                    errors.push(AnalysisError::DivisionByZero { span: expr.span });
                }
            }
        }
        TypedExprKind::Call { callee, args, .. } => {
            check_dbz_expr(callee, errors);
            for a in args {
                check_dbz_expr(a, errors);
            }
        }
        TypedExprKind::MethodCall { object, args, .. } => {
            check_dbz_expr(object, errors);
            for a in args {
                check_dbz_expr(a, errors);
            }
        }
        TypedExprKind::StaticCall { args, .. } => {
            for a in args {
                check_dbz_expr(a, errors);
            }
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            check_dbz_expr(fat_ptr, errors);
            for a in args {
                check_dbz_expr(a, errors);
            }
        }
        TypedExprKind::UnOp { operand, .. } => check_dbz_expr(operand, errors),
        TypedExprKind::Field { object, .. } => check_dbz_expr(object, errors),
        TypedExprKind::Index { object, index } => {
            check_dbz_expr(object, errors);
            check_dbz_expr(index, errors);
        }
        TypedExprKind::Tuple(es) => {
            for e in es {
                check_dbz_expr(e, errors);
            }
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                check_dbz_expr(e, errors);
            }
        }
        TypedExprKind::Unwrap(e) => check_dbz_expr(e, errors),
        TypedExprKind::As { expr, .. } => check_dbz_expr(expr, errors),
        TypedExprKind::Match { scrutinee, arms } => {
            check_dbz_expr(scrutinee, errors);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    check_dbz_expr(g, errors);
                }
                check_dbz_expr(&arm.body, errors);
            }
        }
        TypedExprKind::Spawn(e) => check_dbz_expr(e, errors),
        TypedExprKind::Ref { expr, .. } => check_dbz_expr(expr, errors),
        TypedExprKind::Array(es) => {
            for e in es {
                check_dbz_expr(e, errors);
            }
        }
        TypedExprKind::Gen { body } => check_dbz_block(body, errors),
        TypedExprKind::GenSplice(e) => check_dbz_expr(e, errors),
        TypedExprKind::Closure { body, .. } => match body {
            TypedClosureBody::Expr(e) => check_dbz_expr(e, errors),
            TypedClosureBody::Block(b) => check_dbz_block(b, errors),
        },
        _ => {}
    }
}

/// Returns Some(n) if v == 2^n (n >= 0), else None. Used for strength reduction.
pub fn is_power_of_two(v: i64) -> Option<i64> {
    if v > 0 && v.count_ones() == 1 {
        Some(v.trailing_zeros() as i64)
    } else {
        None
    }
}

/// Returns true if expr might have observable side effects (calls, spawn, raise).
pub fn expr_has_side_effects(expr: &TypedExpr) -> bool {
    match &expr.kind {
        TypedExprKind::Call { .. }
        | TypedExprKind::MethodCall { .. }
        | TypedExprKind::StaticCall { .. }
        | TypedExprKind::IndirectCall { .. }
        | TypedExprKind::Spawn(_) => true,
        TypedExprKind::BinOp { left, right, .. } => {
            expr_has_side_effects(left) || expr_has_side_effects(right)
        }
        TypedExprKind::UnOp { operand, .. } => expr_has_side_effects(operand),
        TypedExprKind::Field { object, .. } => expr_has_side_effects(object),
        TypedExprKind::Index { object, index } => {
            expr_has_side_effects(object) || expr_has_side_effects(index)
        }
        TypedExprKind::Tuple(es) => es.iter().any(expr_has_side_effects),
        TypedExprKind::StructLiteral { fields, .. } => {
            fields.iter().any(|(_, e)| expr_has_side_effects(e))
        }
        TypedExprKind::Unwrap(e) => expr_has_side_effects(e),
        TypedExprKind::As { expr, .. } => expr_has_side_effects(expr),
        TypedExprKind::Match { scrutinee, arms } => {
            expr_has_side_effects(scrutinee) || arms.iter().any(|a| expr_has_side_effects(&a.body))
        }
        TypedExprKind::Array(es) => es.iter().any(expr_has_side_effects),
        TypedExprKind::Gen { body } => body.stmts.iter().any(stmt_has_side_effects),
        TypedExprKind::Closure { .. } => false,
        _ => false,
    }
}

fn stmt_has_side_effects(stmt: &TypedStmt) -> bool {
    match stmt {
        TypedStmt::Expr(e) => expr_has_side_effects(e),
        TypedStmt::VarDecl { value, .. } => expr_has_side_effects(value),
        TypedStmt::Assign { value, .. } => expr_has_side_effects(value),
        TypedStmt::Return { value, .. } => value.as_ref().is_some_and(expr_has_side_effects),
        TypedStmt::Raise { .. } => true,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::Ty;
    use crate::analyzer::typed_ast::{TypedExpr, TypedExprKind, TypedStringSegment};
    use crate::diagnostics::Span;
    use crate::parser::ast::BinOp;

    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    fn int_expr(n: i64) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Int(n),
            ty: Ty::Int,
            span: s(),
        }
    }

    #[allow(dead_code)]
    fn float_expr(f: f64) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Float(f),
            ty: Ty::Float,
            span: s(),
        }
    }

    fn bool_expr(b: bool) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Bool(b),
            ty: Ty::Bool,
            span: s(),
        }
    }

    fn str_expr(text: &str) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Str(vec![TypedStringSegment::Text(text.into())]),
            ty: Ty::Str,
            span: s(),
        }
    }

    fn ident_expr(name: &str) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Ident(name.into()),
            ty: Ty::Int,
            span: s(),
        }
    }

    fn binop(op: BinOp, left: TypedExpr, right: TypedExpr, ty: Ty) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            ty,
            span: s(),
        }
    }

    fn unop(op: UnOp, operand: TypedExpr, ty: Ty) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::UnOp {
                op,
                operand: Box::new(operand),
            },
            ty,
            span: s(),
        }
    }

    fn assert_int(expr: &TypedExpr, expected: i64) {
        match &expr.kind {
            TypedExprKind::Int(n) => assert_eq!(*n, expected, "expected Int({})", expected),
            other => panic!("expected Int({}), got {:?}", expected, other),
        }
    }

    fn assert_bool(expr: &TypedExpr, expected: bool) {
        match &expr.kind {
            TypedExprKind::Bool(b) => assert_eq!(*b, expected, "expected Bool({})", expected),
            other => panic!("expected Bool({}), got {:?}", expected, other),
        }
    }

    fn assert_ident(expr: &TypedExpr, expected: &str) {
        match &expr.kind {
            TypedExprKind::Ident(n) => assert_eq!(n, expected, "expected Ident({})", expected),
            other => panic!("expected Ident({}), got {:?}", expected, other),
        }
    }

    #[test]
    fn fold_integer_add_produces_literal() {
        let expr = binop(BinOp::Add, int_expr(3), int_expr(4), Ty::Int);
        let folded = fold_expr(expr);
        assert_int(&folded, 7);
    }

    #[test]
    fn fold_boolean_and_short_circuits_false() {
        let expr = binop(BinOp::And, bool_expr(false), bool_expr(true), Ty::Bool);
        let folded = fold_expr(expr);
        assert_bool(&folded, false);
    }

    #[test]
    fn fold_string_concat_two_literals() {
        // In Kiln, Pipe is the string concat operator
        let expr = binop(BinOp::Pipe, str_expr("hello"), str_expr(" world"), Ty::Str);
        let folded = fold_expr(expr);
        match folded.kind {
            TypedExprKind::Str(segs) => {
                assert_eq!(segs.len(), 1);
                if let TypedStringSegment::Text(t) = &segs[0] {
                    assert_eq!(t, "hello world");
                } else {
                    panic!("expected Text segment");
                }
            }
            other => panic!("expected Str, got {:?}", other),
        }
    }

    #[test]
    fn fold_does_not_change_non_constant_expr() {
        let expr = binop(BinOp::Add, ident_expr("x"), ident_expr("y"), Ty::Int);
        let folded = fold_expr(expr);
        assert!(matches!(folded.kind, TypedExprKind::BinOp { .. }));
    }

    #[test]
    fn fold_nested_expr_reduces_fully() {
        // (2 + 3) * 4 = 20
        let inner = binop(BinOp::Add, int_expr(2), int_expr(3), Ty::Int);
        let outer = binop(BinOp::Mul, inner, int_expr(4), Ty::Int);
        let folded = fold_expr(outer);
        assert_int(&folded, 20);
    }

    // Item 51 tests: Division-by-Zero detection
    #[test]
    fn literal_division_by_zero_is_error() {
        use crate::analyzer::typed_ast::{TypedBlock, TypedFile, TypedFnDef, TypedItem};
        let div_zero = binop(BinOp::Div, int_expr(10), int_expr(0), Ty::Int);
        let file = TypedFile {
            items: vec![TypedItem::Function(TypedFnDef {
                name: "f".into(),
                params: vec![],
                variadic: None,
                return_type: Ty::Void,
                body: TypedBlock {
                    stmts: vec![TypedStmt::Expr(div_zero)],
                    span: s(),
                },
                is_builtin: false,
                is_inline: false,
                is_declaration: false,
                is_entry: false,
                is_impure: false,
                span: s(),
            })],
            span: s(),
        };
        let errors = check_division_by_zero(&file);
        assert!(!errors.is_empty(), "expected division by zero error");
        assert!(matches!(errors[0], AnalysisError::DivisionByZero { .. }));
    }

    #[test]
    fn non_zero_divisor_produces_no_error() {
        use crate::analyzer::typed_ast::{TypedBlock, TypedFile, TypedFnDef, TypedItem};
        let div_ok = binop(BinOp::Div, int_expr(10), int_expr(2), Ty::Int);
        let file = TypedFile {
            items: vec![TypedItem::Function(TypedFnDef {
                name: "f".into(),
                params: vec![],
                variadic: None,
                return_type: Ty::Void,
                body: TypedBlock {
                    stmts: vec![TypedStmt::Expr(div_ok)],
                    span: s(),
                },
                is_builtin: false,
                is_inline: false,
                is_declaration: false,
                is_entry: false,
                is_impure: false,
                span: s(),
            })],
            span: s(),
        };
        let errors = check_division_by_zero(&file);
        assert!(errors.is_empty(), "no error for non-zero divisor");
    }

    #[test]
    fn division_by_variable_without_known_value_is_not_flagged() {
        use crate::analyzer::typed_ast::{TypedBlock, TypedFile, TypedFnDef, TypedItem};
        let div_var = binop(BinOp::Div, int_expr(10), ident_expr("n"), Ty::Int);
        let file = TypedFile {
            items: vec![TypedItem::Function(TypedFnDef {
                name: "f".into(),
                params: vec![],
                variadic: None,
                return_type: Ty::Void,
                body: TypedBlock {
                    stmts: vec![TypedStmt::Expr(div_var)],
                    span: s(),
                },
                is_builtin: false,
                is_inline: false,
                is_declaration: false,
                is_entry: false,
                is_impure: false,
                span: s(),
            })],
            span: s(),
        };
        let errors = check_division_by_zero(&file);
        assert!(errors.is_empty(), "variable divisor should not be flagged");
    }

    // Item 1 (additional): double negation elimination
    #[test]
    fn double_negation_eliminated() {
        let expr = unop(
            UnOp::Neg,
            unop(UnOp::Neg, ident_expr("x"), Ty::Int),
            Ty::Int,
        );
        let folded = fold_expr(expr);
        assert_ident(&folded, "x");
    }

    #[test]
    fn double_not_eliminated() {
        let expr = unop(
            UnOp::Not,
            unop(UnOp::Not, ident_expr("b"), Ty::Bool),
            Ty::Bool,
        );
        let folded = fold_expr(expr);
        assert_ident(&folded, "b");
    }

    // Item 2 (additional): algebraic identities
    #[test]
    fn add_zero_identity() {
        let expr = binop(BinOp::Add, ident_expr("x"), int_expr(0), Ty::Int);
        let folded = fold_expr(expr);
        assert_ident(&folded, "x");
    }

    #[test]
    fn mul_by_one_identity() {
        let expr = binop(BinOp::Mul, ident_expr("x"), int_expr(1), Ty::Int);
        let folded = fold_expr(expr);
        assert_ident(&folded, "x");
    }

    #[test]
    fn mul_by_zero_is_zero() {
        let expr = binop(BinOp::Mul, ident_expr("x"), int_expr(0), Ty::Int);
        let folded = fold_expr(expr);
        assert_int(&folded, 0);
    }

    #[test]
    fn sub_self_is_zero() {
        let expr = binop(BinOp::Sub, ident_expr("x"), ident_expr("x"), Ty::Int);
        let folded = fold_expr(expr);
        assert_int(&folded, 0);
    }

    // Item 3: comparison with bool literal
    #[test]
    fn eq_true_simplifies_to_self() {
        let expr = binop(BinOp::Eq, ident_expr("b"), bool_expr(true), Ty::Bool);
        let folded = fold_expr(expr);
        assert_ident(&folded, "b");
    }

    #[test]
    fn eq_false_becomes_not() {
        let expr = binop(BinOp::Eq, ident_expr("b"), bool_expr(false), Ty::Bool);
        let folded = fold_expr(expr);
        assert!(matches!(
            folded.kind,
            TypedExprKind::UnOp { op: UnOp::Not, .. }
        ));
    }

    // Item 12: boolean short-circuit folding
    #[test]
    fn false_and_expr_is_false_when_no_side_effects() {
        let expr = binop(BinOp::And, bool_expr(false), ident_expr("x"), Ty::Bool);
        let folded = fold_expr(expr);
        assert_bool(&folded, false);
    }

    // Item 27: neg normalization
    #[test]
    fn neg_zero_is_zero() {
        let expr = unop(UnOp::Neg, int_expr(0), Ty::Int);
        let folded = fold_expr(expr);
        assert_int(&folded, 0);
    }

    // Item 30: compile-time string length
    #[test]
    fn string_len_method_folds_to_int() {
        let str_e = str_expr("hello");
        let len_call = TypedExpr {
            kind: TypedExprKind::MethodCall {
                object: Box::new(str_e),
                method_fn: "len".into(),
                args: vec![],
            },
            ty: Ty::Int,
            span: s(),
        };
        let folded = fold_expr(len_call);
        assert_int(&folded, 5);
    }

    #[test]
    fn true_and_x_simplifies_to_x() {
        let expr = binop(BinOp::And, bool_expr(true), ident_expr("x"), Ty::Bool);
        let folded = fold_expr(expr);
        assert_ident(&folded, "x");
    }

    #[test]
    fn false_or_x_simplifies_to_x() {
        let expr = binop(BinOp::Or, bool_expr(false), ident_expr("x"), Ty::Bool);
        let folded = fold_expr(expr);
        assert_ident(&folded, "x");
    }

    #[test]
    fn zero_minus_x_becomes_negation() {
        let expr = binop(BinOp::Sub, int_expr(0), ident_expr("x"), Ty::Int);
        let folded = fold_expr(expr);
        assert!(
            matches!(folded.kind, TypedExprKind::UnOp { op: UnOp::Neg, .. }),
            "expected Neg, got {:?}",
            folded.kind
        );
    }

    #[test]
    fn mul_by_neg_one_becomes_negation() {
        let expr = binop(BinOp::Mul, ident_expr("x"), int_expr(-1), Ty::Int);
        let folded = fold_expr(expr);
        assert!(matches!(
            folded.kind,
            TypedExprKind::UnOp { op: UnOp::Neg, .. }
        ));
    }

    #[test]
    fn mod_by_one_is_zero() {
        let expr = binop(BinOp::Mod, ident_expr("x"), int_expr(1), Ty::Int);
        let folded = fold_expr(expr);
        assert_int(&folded, 0);
    }

    #[test]
    fn ident_eq_itself_is_true() {
        let expr = binop(BinOp::Eq, ident_expr("x"), ident_expr("x"), Ty::Bool);
        let folded = fold_expr(expr);
        assert_bool(&folded, true);
    }

    #[test]
    fn ident_ne_itself_is_false() {
        let expr = binop(BinOp::Ne, ident_expr("x"), ident_expr("x"), Ty::Bool);
        let folded = fold_expr(expr);
        assert_bool(&folded, false);
    }

    #[test]
    fn ident_lt_itself_is_false() {
        let expr = binop(BinOp::Lt, ident_expr("x"), ident_expr("x"), Ty::Bool);
        let folded = fold_expr(expr);
        assert_bool(&folded, false);
    }

    #[test]
    fn ident_lteq_itself_is_true() {
        let expr = binop(BinOp::LtEq, ident_expr("x"), ident_expr("x"), Ty::Bool);
        let folded = fold_expr(expr);
        assert_bool(&folded, true);
    }

    #[test]
    fn float_add_zero_identity() {
        let expr = binop(BinOp::Add, float_expr(3.14), float_expr(0.0), Ty::Float);
        let folded = fold_expr(expr);
        match folded.kind {
            TypedExprKind::Float(f) => assert!((f - 3.14).abs() < 1e-9),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn float_mul_one_identity() {
        let expr = binop(BinOp::Mul, float_expr(2.5), float_expr(1.0), Ty::Float);
        let folded = fold_expr(expr);
        match folded.kind {
            TypedExprKind::Float(f) => assert!((f - 2.5).abs() < 1e-9),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn adjacent_text_segments_are_merged() {
        let segs = vec![
            TypedStringSegment::Text("hello".into()),
            TypedStringSegment::Text(" ".into()),
            TypedStringSegment::Text("world".into()),
        ];
        let expr = TypedExpr {
            kind: TypedExprKind::Str(segs),
            ty: Ty::Str,
            span: s(),
        };
        let folded = fold_expr(expr);
        match folded.kind {
            TypedExprKind::Str(segs) => {
                assert_eq!(segs.len(), 1);
                if let TypedStringSegment::Text(t) = &segs[0] {
                    assert_eq!(t, "hello world");
                } else {
                    panic!("expected Text segment");
                }
            }
            other => panic!("expected Str, got {:?}", other),
        }
    }

    #[test]
    fn constant_true_if_branch_is_pruned_to_then() {
        use crate::analyzer::typed_ast::{TypedBlock, TypedStmt};
        let then_body = TypedBlock {
            stmts: vec![TypedStmt::Return {
                value: Some(int_expr(42)),
                span: s(),
            }],
            span: s(),
        };
        let block = TypedBlock {
            stmts: vec![TypedStmt::If {
                branches: vec![(bool_expr(true), then_body)],
                else_branch: None,
                span: s(),
            }],
            span: s(),
        };
        let folded = fold_block(block);
        assert_eq!(folded.stmts.len(), 1);
        assert!(matches!(folded.stmts[0], TypedStmt::Return { .. }));
    }

    #[test]
    fn constant_false_if_branch_is_pruned_to_else() {
        use crate::analyzer::typed_ast::{TypedBlock, TypedStmt};
        let then_body = TypedBlock {
            stmts: vec![TypedStmt::Return {
                value: Some(int_expr(0)),
                span: s(),
            }],
            span: s(),
        };
        let else_body = TypedBlock {
            stmts: vec![TypedStmt::Return {
                value: Some(int_expr(1)),
                span: s(),
            }],
            span: s(),
        };
        let block = TypedBlock {
            stmts: vec![TypedStmt::If {
                branches: vec![(bool_expr(false), then_body)],
                else_branch: Some(else_body),
                span: s(),
            }],
            span: s(),
        };
        let folded = fold_block(block);
        assert_eq!(folded.stmts.len(), 1);
        assert!(matches!(folded.stmts[0], TypedStmt::Return { .. }));
    }

    #[test]
    fn sub_neg_y_becomes_add() {
        // x - (-y) -> x + y
        let neg_y = unop(UnOp::Neg, ident_expr("y"), Ty::Int);
        let expr = binop(BinOp::Sub, ident_expr("x"), neg_y, Ty::Int);
        let folded = fold_expr(expr);
        assert!(
            matches!(folded.kind, TypedExprKind::BinOp { op: BinOp::Add, .. }),
            "expected Add, got {:?}",
            folded.kind
        );
    }

    #[test]
    fn ne_true_becomes_not() {
        let expr = binop(BinOp::Ne, ident_expr("b"), bool_expr(true), Ty::Bool);
        let folded = fold_expr(expr);
        assert!(matches!(
            folded.kind,
            TypedExprKind::UnOp { op: UnOp::Not, .. }
        ));
    }

    #[test]
    fn ne_false_simplifies_to_self() {
        let expr = binop(BinOp::Ne, ident_expr("b"), bool_expr(false), Ty::Bool);
        let folded = fold_expr(expr);
        assert_ident(&folded, "b");
    }

    #[test]
    fn mod_self_is_zero() {
        let expr = binop(BinOp::Mod, ident_expr("x"), ident_expr("x"), Ty::Int);
        let folded = fold_expr(expr);
        assert_int(&folded, 0);
    }

    #[test]
    fn unreachable_code_after_return_is_pruned() {
        use crate::analyzer::typed_ast::{TypedBlock, TypedStmt};
        let block = TypedBlock {
            stmts: vec![
                TypedStmt::Return {
                    value: Some(int_expr(1)),
                    span: s(),
                },
                // This statement is dead code
                TypedStmt::Expr(int_expr(99)),
            ],
            span: s(),
        };
        let folded = fold_block(block);
        assert_eq!(
            folded.stmts.len(),
            1,
            "dead code after return should be pruned"
        );
        assert!(matches!(folded.stmts[0], TypedStmt::Return { .. }));
    }

    #[test]
    fn dead_while_false_is_pruned() {
        use crate::analyzer::typed_ast::{TypedBlock, TypedStmt};
        let body = TypedBlock {
            stmts: vec![TypedStmt::Break(s())],
            span: s(),
        };
        let block = TypedBlock {
            stmts: vec![TypedStmt::While {
                cond: bool_expr(false),
                body,
                span: s(),
            }],
            span: s(),
        };
        let folded = fold_block(block);
        assert!(folded.stmts.is_empty(), "while false should be pruned");
    }

    // Idempotent AND: x && x -> x
    #[test]
    fn ident_and_itself_simplifies_to_itself() {
        let expr = binop(BinOp::And, ident_expr("x"), ident_expr("x"), Ty::Bool);
        let folded = fold_expr(expr);
        assert_ident(&folded, "x");
    }

    // Idempotent OR: x || x -> x
    #[test]
    fn ident_or_itself_simplifies_to_itself() {
        let expr = binop(BinOp::Or, ident_expr("x"), ident_expr("x"), Ty::Bool);
        let folded = fold_expr(expr);
        assert_ident(&folded, "x");
    }

    // Strength reduction: x * 2 -> x + x
    #[test]
    fn multiply_by_two_becomes_addition() {
        let expr = binop(BinOp::Mul, ident_expr("x"), int_expr(2), Ty::Int);
        let folded = fold_expr(expr);
        if let TypedExprKind::BinOp {
            op: BinOp::Add,
            left,
            right,
        } = &folded.kind
        {
            assert!(
                matches!(left.kind, TypedExprKind::Ident(ref n) if n == "x"),
                "left should be x"
            );
            assert!(
                matches!(right.kind, TypedExprKind::Ident(ref n) if n == "x"),
                "right should be x"
            );
        } else {
            panic!("x * 2 should become x + x, got {:?}", folded.kind);
        }
    }

    // Strength reduction: 2 * x -> x + x
    #[test]
    fn two_times_x_becomes_addition() {
        let expr = binop(BinOp::Mul, int_expr(2), ident_expr("x"), Ty::Int);
        let folded = fold_expr(expr);
        if let TypedExprKind::BinOp {
            op: BinOp::Add,
            left,
            right,
        } = &folded.kind
        {
            assert!(
                matches!(left.kind, TypedExprKind::Ident(ref n) if n == "x"),
                "left should be x"
            );
            assert!(
                matches!(right.kind, TypedExprKind::Ident(ref n) if n == "x"),
                "right should be x"
            );
        } else {
            panic!("2 * x should become x + x, got {:?}", folded.kind);
        }
    }

    // DeMorgan's: !(a && b) -> (!a || !b)
    #[test]
    fn not_and_becomes_or_of_nots() {
        let inner = binop(BinOp::And, ident_expr("a"), ident_expr("b"), Ty::Bool);
        let expr = unop(UnOp::Not, inner, Ty::Bool);
        let folded = fold_expr(expr);
        assert!(
            matches!(folded.kind, TypedExprKind::BinOp { op: BinOp::Or, .. }),
            "!(a && b) should become (!a || !b)"
        );
    }

    // DeMorgan's: !(a || b) -> (!a && !b)
    #[test]
    fn not_or_becomes_and_of_nots() {
        let inner = binop(BinOp::Or, ident_expr("a"), ident_expr("b"), Ty::Bool);
        let expr = unop(UnOp::Not, inner, Ty::Bool);
        let folded = fold_expr(expr);
        assert!(
            matches!(folded.kind, TypedExprKind::BinOp { op: BinOp::And, .. }),
            "!(a || b) should become (!a && !b)"
        );
    }

    // DeMorgan's with literal: !(true && x) -> (false || !x) -> !x
    #[test]
    fn demorgan_with_true_simplifies() {
        let inner = binop(BinOp::And, bool_expr(true), ident_expr("x"), Ty::Bool);
        let expr = unop(UnOp::Not, inner, Ty::Bool);
        let folded = fold_expr(expr);
        // !(true && x) -> (!true || !x) -> (false || !x) -> !x
        assert!(
            matches!(folded.kind, TypedExprKind::UnOp { op: UnOp::Not, .. }),
            "!(true && x) should simplify to !x"
        );
    }

    // x || true -> true (no side effects in x)
    #[test]
    fn ident_or_true_is_true() {
        let expr = binop(BinOp::Or, ident_expr("x"), bool_expr(true), Ty::Bool);
        let folded = fold_expr(expr);
        assert_bool(&folded, true);
    }

    // x && false -> false (no side effects in x)
    #[test]
    fn ident_and_false_is_false() {
        let expr = binop(BinOp::And, ident_expr("x"), bool_expr(false), Ty::Bool);
        let folded = fold_expr(expr);
        assert_bool(&folded, false);
    }
}
