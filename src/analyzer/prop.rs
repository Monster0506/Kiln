use crate::analyzer::fold::{fold_block, fold_expr};
use crate::analyzer::typed_ast::{
    TypedBlock, TypedCatchHandler, TypedExpr, TypedExprKind, TypedFile, TypedFnDef, TypedHookDef,
    TypedItem, TypedStmt,
};
use std::collections::HashMap;

/// Propagate constants within a block. Maintains a map of definitely-constant bindings.
/// Only propagates Int, Float, Bool literals (not Str to avoid complexity).
pub fn propagate_block(block: TypedBlock) -> TypedBlock {
    let mut constants: HashMap<String, TypedExprKind> = HashMap::new();
    let stmts = block
        .stmts
        .into_iter()
        .map(|stmt| propagate_stmt(stmt, &mut constants))
        .collect();
    // Run fold again to collapse newly-constant expressions
    fold_block(TypedBlock {
        stmts,
        span: block.span,
    })
}

fn is_propagatable_literal(kind: &TypedExprKind) -> bool {
    matches!(
        kind,
        TypedExprKind::Int(_) | TypedExprKind::Float(_) | TypedExprKind::Bool(_)
    )
}

fn propagate_stmt(stmt: TypedStmt, constants: &mut HashMap<String, TypedExprKind>) -> TypedStmt {
    match stmt {
        TypedStmt::VarDecl {
            name,
            ty,
            value,
            mutable,
            span,
        } => {
            let value = propagate_expr(value, constants);
            let value = fold_expr(value);
            // Register as constant if it's a propagatable literal and immutable
            if !mutable && is_propagatable_literal(&value.kind) {
                constants.insert(name.clone(), value.kind.clone());
            }
            TypedStmt::VarDecl {
                name,
                ty,
                value,
                mutable,
                span,
            }
        }
        TypedStmt::Assign {
            target,
            value,
            span,
        } => {
            // Invalidate the target name if it's a simple ident
            if let TypedExprKind::Ident(ref name) = target.kind {
                constants.remove(name);
            }
            let value = propagate_expr(value, constants);
            let value = fold_expr(value);
            TypedStmt::Assign {
                target: propagate_expr(target, constants),
                value,
                span,
            }
        }
        TypedStmt::CompoundAssign {
            target,
            op,
            rhs,
            span,
        } => {
            // Invalidate
            if let TypedExprKind::Ident(ref name) = target.kind {
                constants.remove(name);
            }
            TypedStmt::CompoundAssign {
                target: propagate_expr(target, constants),
                op,
                rhs: fold_expr(propagate_expr(rhs, constants)),
                span,
            }
        }
        TypedStmt::Return { value, span } => TypedStmt::Return {
            value: value.map(|e| fold_expr(propagate_expr(e, constants))),
            span,
        },
        TypedStmt::Raise { value, span } => TypedStmt::Raise {
            value: value.map(|e| fold_expr(propagate_expr(e, constants))),
            span,
        },
        TypedStmt::If {
            branches,
            else_branch,
            span,
        } => {
            // Branches may have different bindings - conservatively clear all after
            let branches = branches
                .into_iter()
                .map(|(cond, body)| {
                    let cond = fold_expr(propagate_expr(cond, constants));
                    let body = propagate_block(body);
                    (cond, body)
                })
                .collect();
            // Conservative: don't propagate across branches (could invalidate)
            constants.clear();
            TypedStmt::If {
                branches,
                else_branch: else_branch.map(propagate_block),
                span,
            }
        }
        TypedStmt::While { cond, body, span } => {
            // In loops, invalidate everything (loop body may modify bindings)
            constants.clear();
            TypedStmt::While {
                cond: fold_expr(propagate_expr(cond, constants)),
                body: propagate_block(body),
                span,
            }
        }
        TypedStmt::DoWhile { body, cond, span } => {
            constants.clear();
            TypedStmt::DoWhile {
                body: propagate_block(body),
                cond: fold_expr(propagate_expr(cond, constants)),
                span,
            }
        }
        TypedStmt::For {
            binding,
            binding_ty,
            iterable,
            body,
            iter_ty,
            span,
        } => {
            constants.clear();
            TypedStmt::For {
                binding,
                binding_ty,
                iterable: fold_expr(propagate_expr(iterable, constants)),
                body: propagate_block(body),
                iter_ty,
                span,
            }
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => {
            constants.clear();
            TypedStmt::TryCatch {
                body: propagate_block(body),
                handlers: handlers
                    .into_iter()
                    .map(|h| TypedCatchHandler {
                        ty: h.ty,
                        binding: h.binding,
                        body: propagate_block(h.body),
                        span: h.span,
                    })
                    .collect(),
                finally: finally.map(propagate_block),
                span,
            }
        }
        TypedStmt::FnDef(f) => TypedStmt::FnDef(propagate_fn(f)),
        TypedStmt::Expr(e) => TypedStmt::Expr(fold_expr(propagate_expr(e, constants))),
        other => other,
    }
}

fn propagate_expr(expr: TypedExpr, constants: &HashMap<String, TypedExprKind>) -> TypedExpr {
    let span = expr.span;
    let ty = expr.ty.clone();

    match expr.kind {
        TypedExprKind::Ident(ref name) => {
            if let Some(lit) = constants.get(name) {
                return TypedExpr {
                    kind: lit.clone(),
                    ty,
                    span,
                };
            }
            expr
        }
        TypedExprKind::BinOp { op, left, right } => TypedExpr {
            kind: TypedExprKind::BinOp {
                op,
                left: Box::new(propagate_expr(*left, constants)),
                right: Box::new(propagate_expr(*right, constants)),
            },
            ty,
            span,
        },
        TypedExprKind::UnOp { op, operand } => TypedExpr {
            kind: TypedExprKind::UnOp {
                op,
                operand: Box::new(propagate_expr(*operand, constants)),
            },
            ty,
            span,
        },
        TypedExprKind::Call {
            callee,
            args,
            fn_name,
            generic_bounds,
            generic_params,
            param_tys,
        } => TypedExpr {
            kind: TypedExprKind::Call {
                callee: Box::new(propagate_expr(*callee, constants)),
                args: args
                    .into_iter()
                    .map(|a| propagate_expr(a, constants))
                    .collect(),
                fn_name,
                generic_bounds,
                generic_params,
                param_tys,
            },
            ty,
            span,
        },
        TypedExprKind::MethodCall {
            object,
            method_fn,
            args,
        } => TypedExpr {
            kind: TypedExprKind::MethodCall {
                object: Box::new(propagate_expr(*object, constants)),
                method_fn,
                args: args
                    .into_iter()
                    .map(|a| propagate_expr(a, constants))
                    .collect(),
            },
            ty,
            span,
        },
        TypedExprKind::StaticCall { method_fn, args } => TypedExpr {
            kind: TypedExprKind::StaticCall {
                method_fn,
                args: args
                    .into_iter()
                    .map(|a| propagate_expr(a, constants))
                    .collect(),
            },
            ty,
            span,
        },
        TypedExprKind::IndirectCall { fat_ptr, args } => TypedExpr {
            kind: TypedExprKind::IndirectCall {
                fat_ptr: Box::new(propagate_expr(*fat_ptr, constants)),
                args: args
                    .into_iter()
                    .map(|a| propagate_expr(a, constants))
                    .collect(),
            },
            ty,
            span,
        },
        TypedExprKind::Field { object, field } => TypedExpr {
            kind: TypedExprKind::Field {
                object: Box::new(propagate_expr(*object, constants)),
                field,
            },
            ty,
            span,
        },
        TypedExprKind::Index { object, index } => TypedExpr {
            kind: TypedExprKind::Index {
                object: Box::new(propagate_expr(*object, constants)),
                index: Box::new(propagate_expr(*index, constants)),
            },
            ty,
            span,
        },
        TypedExprKind::Tuple(exprs) => TypedExpr {
            kind: TypedExprKind::Tuple(
                exprs
                    .into_iter()
                    .map(|e| propagate_expr(e, constants))
                    .collect(),
            ),
            ty,
            span,
        },
        TypedExprKind::StructLiteral { ty_name, fields } => TypedExpr {
            kind: TypedExprKind::StructLiteral {
                ty_name,
                fields: fields
                    .into_iter()
                    .map(|(k, v)| (k, propagate_expr(v, constants)))
                    .collect(),
            },
            ty,
            span,
        },
        TypedExprKind::Unwrap(e) => TypedExpr {
            kind: TypedExprKind::Unwrap(Box::new(propagate_expr(*e, constants))),
            ty,
            span,
        },
        TypedExprKind::As {
            expr: e,
            ty: cast_ty,
        } => TypedExpr {
            kind: TypedExprKind::As {
                expr: Box::new(propagate_expr(*e, constants)),
                ty: cast_ty,
            },
            ty,
            span,
        },
        // Leaf nodes
        other => TypedExpr {
            kind: other,
            ty,
            span,
        },
    }
}

fn propagate_fn(f: TypedFnDef) -> TypedFnDef {
    TypedFnDef {
        body: propagate_block(f.body),
        ..f
    }
}

fn propagate_hook(h: TypedHookDef) -> TypedHookDef {
    TypedHookDef {
        body: propagate_block(h.body),
        ..h
    }
}

pub fn propagate_file(file: TypedFile) -> TypedFile {
    let items = file
        .items
        .into_iter()
        .map(|item| match item {
            TypedItem::Function(f) => TypedItem::Function(propagate_fn(f)),
            TypedItem::ImplBlock(mut ib) => {
                ib.methods = ib.methods.into_iter().map(propagate_fn).collect();
                ib.hooks = ib.hooks.into_iter().map(propagate_hook).collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::Ty;
    use crate::analyzer::typed_ast::{
        TypedBlock, TypedExpr, TypedExprKind, TypedFile, TypedFnDef, TypedItem,
    };
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

    fn ident_expr(name: &str) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Ident(name.into()),
            ty: Ty::Int,
            span: s(),
        }
    }

    fn binop_expr(op: BinOp, left: TypedExpr, right: TypedExpr) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::BinOp {
                op,
                left: Box::new(left),
                right: Box::new(right),
            },
            ty: Ty::Int,
            span: s(),
        }
    }

    fn make_file(stmts: Vec<TypedStmt>) -> TypedFile {
        TypedFile {
            items: vec![TypedItem::Function(TypedFnDef {
                name: "f".into(),
                params: vec![],
                variadic: None,
                return_type: Ty::Int,
                body: TypedBlock { stmts, span: s() },
                is_builtin: false,
                is_inline: false,
                is_declaration: false,
                is_entry: false,
                span: s(),
            })],
            span: s(),
        }
    }

    fn assert_return_is_int(file: &TypedFile, expected: i64) {
        if let TypedItem::Function(f) = &file.items[0] {
            for stmt in &f.body.stmts {
                if let TypedStmt::Return { value: Some(e), .. } = stmt {
                    match &e.kind {
                        TypedExprKind::Int(n) => {
                            assert_eq!(*n, expected, "expected return Int({})", expected);
                            return;
                        }
                        other => panic!("expected Int({}), got {:?}", expected, other),
                    }
                }
            }
            panic!("no return statement found");
        }
    }

    #[test]
    fn propagate_integer_const_into_expression() {
        // let x = 5; return x + 1  =>  return 6
        let stmts = vec![
            TypedStmt::VarDecl {
                name: "x".into(),
                ty: Ty::Int,
                value: int_expr(5),
                mutable: false,
                span: s(),
            },
            TypedStmt::Return {
                value: Some(binop_expr(BinOp::Add, ident_expr("x"), int_expr(1))),
                span: s(),
            },
        ];
        let file = propagate_file(make_file(stmts));
        assert_return_is_int(&file, 6);
    }

    #[test]
    fn mutation_invalidates_propagated_constant() {
        // let mut x = 5; x = 10; return x  => return x (unknown at compile time)
        let stmts = vec![
            TypedStmt::VarDecl {
                name: "x".into(),
                ty: Ty::Int,
                value: int_expr(5),
                mutable: true,
                span: s(),
            },
            TypedStmt::Assign {
                target: ident_expr("x"),
                value: int_expr(10),
                span: s(),
            },
            TypedStmt::Return {
                value: Some(ident_expr("x")),
                span: s(),
            },
        ];
        let file = propagate_file(make_file(stmts));
        // x was invalidated by mutation (mutable), so the return should not be a constant 5
        if let TypedItem::Function(f) = &file.items[0] {
            for stmt in &f.body.stmts {
                if let TypedStmt::Return { value: Some(e), .. } = stmt {
                    // Should be Int(10) since we assigned 10, or Ident("x") -- not Int(5)
                    assert!(
                        !matches!(&e.kind, TypedExprKind::Int(5)),
                        "mutable var x=5 then x=10 should not propagate 5"
                    );
                    return;
                }
            }
        }
    }

    #[test]
    fn propagation_enables_subsequent_fold() {
        // let x = 3; let y = 4; return x + y  =>  return 7
        let stmts = vec![
            TypedStmt::VarDecl {
                name: "x".into(),
                ty: Ty::Int,
                value: int_expr(3),
                mutable: false,
                span: s(),
            },
            TypedStmt::VarDecl {
                name: "y".into(),
                ty: Ty::Int,
                value: int_expr(4),
                mutable: false,
                span: s(),
            },
            TypedStmt::Return {
                value: Some(binop_expr(BinOp::Add, ident_expr("x"), ident_expr("y"))),
                span: s(),
            },
        ];
        let file = propagate_file(make_file(stmts));
        assert_return_is_int(&file, 7);
    }

    #[test]
    fn non_constant_binding_is_not_propagated() {
        // A call result should not be propagated
        let call_expr = TypedExpr {
            kind: TypedExprKind::Call {
                callee: Box::new(TypedExpr {
                    kind: TypedExprKind::Ident("get_val".into()),
                    ty: Ty::Int,
                    span: s(),
                }),
                args: vec![],
                fn_name: "get_val".into(),
                generic_bounds: vec![],
                generic_params: vec![],
                param_tys: vec![],
            },
            ty: Ty::Int,
            span: s(),
        };
        let stmts = vec![
            TypedStmt::VarDecl {
                name: "x".into(),
                ty: Ty::Int,
                value: call_expr,
                mutable: false,
                span: s(),
            },
            TypedStmt::Return {
                value: Some(ident_expr("x")),
                span: s(),
            },
        ];
        let file = propagate_file(make_file(stmts));
        // x is bound to a call result, not a literal -- should remain as Ident("x")
        if let TypedItem::Function(f) = &file.items[0] {
            for stmt in &f.body.stmts {
                if let TypedStmt::Return { value: Some(e), .. } = stmt {
                    assert!(
                        matches!(&e.kind, TypedExprKind::Ident(n) if n == "x"),
                        "expected Ident(x) in return, got {:?}",
                        e.kind
                    );
                    return;
                }
            }
        }
    }
}
