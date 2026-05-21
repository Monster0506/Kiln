/// Dead assignment elimination: remove VarDecl bindings that are never read,
/// provided the initializer expression has no side effects.
///
/// This is a simple backward linear scan within each block. It does not
/// handle cross-block liveness (loops, branches); those cases are conservative.
use crate::analyzer::typed_ast::{
    TypedBlock, TypedCatchHandler, TypedExpr, TypedExprKind, TypedFile, TypedFnDef, TypedHookDef,
    TypedImplBlock, TypedItem, TypedStmt,
};
use std::collections::HashSet;

// --- Public API ---------------------------------------------------------------

pub fn dce_file(file: TypedFile) -> TypedFile {
    let items = file
        .items
        .into_iter()
        .map(|item| match item {
            TypedItem::Function(f) => TypedItem::Function(dce_fn(f)),
            TypedItem::ImplBlock(ib) => TypedItem::ImplBlock(dce_impl(ib)),
            other => other,
        })
        .collect();
    TypedFile {
        items,
        span: file.span,
    }
}

fn dce_fn(mut f: TypedFnDef) -> TypedFnDef {
    f.body = dce_block(f.body);
    f
}

fn dce_impl(mut ib: TypedImplBlock) -> TypedImplBlock {
    ib.methods = ib.methods.into_iter().map(dce_fn).collect();
    ib.hooks = ib
        .hooks
        .into_iter()
        .map(|mut h: TypedHookDef| {
            h.body = dce_block(h.body);
            h
        })
        .collect();
    ib
}

pub fn dce_block(block: TypedBlock) -> TypedBlock {
    // Collect which names are read anywhere in the block (conservative: includes all reads).
    let mut live: HashSet<String> = HashSet::new();
    for stmt in &block.stmts {
        collect_reads_stmt(stmt, &mut live);
    }

    // Second pass: rebuild, dropping dead immutable VarDecl with pure RHS.
    let mut new_stmts = Vec::with_capacity(block.stmts.len());
    for stmt in block.stmts {
        match &stmt {
            TypedStmt::VarDecl {
                name,
                value,
                mutable,
                ..
            } => {
                // Mutable bindings might have their value read via later assignment;
                // we only eliminate immutable let bindings.
                if !mutable && !live.contains(name) && !expr_has_side_effects(value) {
                    // Drop this dead binding entirely.
                    continue;
                }
            }
            // Redundant self-assignment: x = x -> drop (no side effects in either)
            TypedStmt::Assign { target, value, .. } => {
                if let (TypedExprKind::Ident(tname), TypedExprKind::Ident(vname)) =
                    (&target.kind, &value.kind)
                {
                    if tname == vname {
                        continue;
                    }
                }
            }
            _ => {}
        }
        // Recurse into sub-blocks.
        let stmt = dce_stmt(stmt);
        new_stmts.push(stmt);
    }

    TypedBlock {
        stmts: new_stmts,
        span: block.span,
    }
}

fn dce_stmt(stmt: TypedStmt) -> TypedStmt {
    match stmt {
        TypedStmt::If {
            branches,
            else_branch,
            span,
        } => {
            let branches = branches
                .into_iter()
                .map(|(cond, body)| (cond, dce_block(body)))
                .collect();
            let else_branch = else_branch.map(dce_block);
            TypedStmt::If {
                branches,
                else_branch,
                span,
            }
        }
        TypedStmt::While { cond, body, span } => TypedStmt::While {
            cond,
            body: dce_block(body),
            span,
        },
        TypedStmt::DoWhile { body, cond, span } => TypedStmt::DoWhile {
            body: dce_block(body),
            cond,
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
            iterable,
            body: dce_block(body),
            iter_ty,
            span,
        },
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => TypedStmt::TryCatch {
            body: dce_block(body),
            handlers: handlers
                .into_iter()
                .map(|mut h: TypedCatchHandler| {
                    h.body = dce_block(h.body);
                    h
                })
                .collect(),
            finally: finally.map(dce_block),
            span,
        },
        TypedStmt::FnDef(f) => TypedStmt::FnDef(dce_fn(f)),
        other => other,
    }
}

// --- Helpers ------------------------------------------------------------------

/// Collect all variable names that are *read* (not just written) in a statement.
fn collect_reads_stmt(stmt: &TypedStmt, out: &mut HashSet<String>) {
    match stmt {
        TypedStmt::VarDecl { value, .. } => collect_reads_expr(value, out),
        TypedStmt::Assign { target, value, .. } => {
            // Reading from value counts; writing to target only counts for sub-reads inside it.
            collect_reads_expr(value, out);
            // For field/index targets, reading the base object also counts.
            collect_reads_in_target(target, out);
        }
        TypedStmt::CompoundAssign { target, rhs, .. } => {
            collect_reads_expr(rhs, out);
            // Compound assign reads and writes the target.
            collect_reads_expr(target, out);
        }
        TypedStmt::Return { value, .. } => {
            if let Some(e) = value {
                collect_reads_expr(e, out);
            }
        }
        TypedStmt::Raise { value, .. } => {
            if let Some(e) = value {
                collect_reads_expr(e, out);
            }
        }
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (cond, body) in branches {
                collect_reads_expr(cond, out);
                for s in &body.stmts {
                    collect_reads_stmt(s, out);
                }
            }
            if let Some(b) = else_branch {
                for s in &b.stmts {
                    collect_reads_stmt(s, out);
                }
            }
        }
        TypedStmt::While { cond, body, .. } => {
            collect_reads_expr(cond, out);
            for s in &body.stmts {
                collect_reads_stmt(s, out);
            }
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            for s in &body.stmts {
                collect_reads_stmt(s, out);
            }
            collect_reads_expr(cond, out);
        }
        TypedStmt::For { iterable, body, .. } => {
            collect_reads_expr(iterable, out);
            for s in &body.stmts {
                collect_reads_stmt(s, out);
            }
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            for s in &body.stmts {
                collect_reads_stmt(s, out);
            }
            for h in handlers {
                for s in &h.body.stmts {
                    collect_reads_stmt(s, out);
                }
            }
            if let Some(b) = finally {
                for s in &b.stmts {
                    collect_reads_stmt(s, out);
                }
            }
        }
        TypedStmt::FnDef(f) => {
            for s in &f.body.stmts {
                collect_reads_stmt(s, out);
            }
        }
        TypedStmt::Expr(e) => collect_reads_expr(e, out),
        TypedStmt::Break(_) | TypedStmt::Continue(_) => {}
    }
}

fn collect_reads_in_target(expr: &TypedExpr, out: &mut HashSet<String>) {
    match &expr.kind {
        TypedExprKind::Ident(_) => {
            // Writing to a plain ident: don't count as a read.
        }
        TypedExprKind::Field { object, .. } => collect_reads_expr(object, out),
        TypedExprKind::Index { object, index } => {
            collect_reads_expr(object, out);
            collect_reads_expr(index, out);
        }
        _ => collect_reads_expr(expr, out),
    }
}

fn collect_reads_expr(expr: &TypedExpr, out: &mut HashSet<String>) {
    match &expr.kind {
        TypedExprKind::Ident(name) => {
            out.insert(name.clone());
        }
        TypedExprKind::Call { callee, args, .. } => {
            collect_reads_expr(callee, out);
            for a in args {
                collect_reads_expr(a, out);
            }
        }
        TypedExprKind::MethodCall { object, args, .. } => {
            collect_reads_expr(object, out);
            for a in args {
                collect_reads_expr(a, out);
            }
        }
        TypedExprKind::StaticCall { args, .. } => {
            for a in args {
                collect_reads_expr(a, out);
            }
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            collect_reads_expr(fat_ptr, out);
            for a in args {
                collect_reads_expr(a, out);
            }
        }
        TypedExprKind::BinOp { left, right, .. } => {
            collect_reads_expr(left, out);
            collect_reads_expr(right, out);
        }
        TypedExprKind::UnOp { operand, .. } => collect_reads_expr(operand, out),
        TypedExprKind::Field { object, .. } => collect_reads_expr(object, out),
        TypedExprKind::Index { object, index } => {
            collect_reads_expr(object, out);
            collect_reads_expr(index, out);
        }
        TypedExprKind::Tuple(exprs) => {
            for e in exprs {
                collect_reads_expr(e, out);
            }
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_reads_expr(e, out);
            }
        }
        TypedExprKind::Unwrap(e) => collect_reads_expr(e, out),
        TypedExprKind::As { expr, .. } => collect_reads_expr(expr, out),
        TypedExprKind::Match { scrutinee, arms } => {
            collect_reads_expr(scrutinee, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    collect_reads_expr(g, out);
                }
                collect_reads_expr(&arm.body, out);
            }
        }
        TypedExprKind::Closure { body, .. } => {
            use crate::analyzer::typed_ast::TypedClosureBody;
            match body {
                TypedClosureBody::Expr(e) => collect_reads_expr(e, out),
                TypedClosureBody::Block(b) => {
                    for s in &b.stmts {
                        collect_reads_stmt(s, out);
                    }
                }
            }
        }
        TypedExprKind::Spawn(e) => collect_reads_expr(e, out),
        TypedExprKind::Ref { expr, .. } => collect_reads_expr(expr, out),
        TypedExprKind::Array(exprs) => {
            for e in exprs {
                collect_reads_expr(e, out);
            }
        }
        TypedExprKind::Gen { body } => {
            for s in &body.stmts {
                collect_reads_stmt(s, out);
            }
        }
        TypedExprKind::GenSplice(e) => collect_reads_expr(e, out),
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Str(_)
        | TypedExprKind::EnumVariant { .. } => {}
    }
}

/// Returns true if the expression has observable side effects (calls, raises, spawns).
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
        TypedExprKind::Tuple(exprs) => exprs.iter().any(expr_has_side_effects),
        TypedExprKind::StructLiteral { fields, .. } => {
            fields.iter().any(|(_, e)| expr_has_side_effects(e))
        }
        TypedExprKind::Unwrap(e) => expr_has_side_effects(e),
        TypedExprKind::As { expr, .. } => expr_has_side_effects(expr),
        TypedExprKind::Match { scrutinee, arms } => {
            expr_has_side_effects(scrutinee)
                || arms.iter().any(|a| {
                    a.guard.as_ref().is_some_and(expr_has_side_effects)
                        || expr_has_side_effects(&a.body)
                })
        }
        TypedExprKind::Closure { .. } => false,
        TypedExprKind::Ref { expr, .. } => expr_has_side_effects(expr),
        TypedExprKind::Array(exprs) => exprs.iter().any(expr_has_side_effects),
        TypedExprKind::Gen { .. } | TypedExprKind::GenSplice(_) => false,
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Str(_)
        | TypedExprKind::Ident(_)
        | TypedExprKind::EnumVariant { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::Ty;
    use crate::analyzer::typed_ast::{TypedExpr, TypedExprKind};
    use crate::diagnostics::Span;

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

    fn var_decl(name: &str, value: TypedExpr) -> TypedStmt {
        TypedStmt::VarDecl {
            name: name.into(),
            ty: Ty::Int,
            value,
            mutable: false,
            span: s(),
        }
    }

    fn ret_stmt(e: TypedExpr) -> TypedStmt {
        TypedStmt::Return {
            value: Some(e),
            span: s(),
        }
    }

    fn block(stmts: Vec<TypedStmt>) -> TypedBlock {
        TypedBlock { stmts, span: s() }
    }

    #[test]
    fn dead_let_binding_is_removed() {
        // let x = 42   (x never read) -> should be eliminated
        // return 0
        let b = block(vec![var_decl("x", int_expr(42)), ret_stmt(int_expr(0))]);
        let result = dce_block(b);
        assert_eq!(
            result.stmts.len(),
            1,
            "dead let should be removed, only return should remain"
        );
        assert!(matches!(result.stmts[0], TypedStmt::Return { .. }));
    }

    #[test]
    fn live_variable_is_not_removed() {
        // let x = 42
        // return x   (x is read) -> keep both
        let b = block(vec![var_decl("x", int_expr(42)), ret_stmt(ident_expr("x"))]);
        let result = dce_block(b);
        assert_eq!(result.stmts.len(), 2, "live variable binding must be kept");
    }

    #[test]
    fn redundant_self_assignment_is_removed() {
        // x = x should be eliminated
        let b = block(vec![
            TypedStmt::Assign {
                target: ident_expr("x"),
                value: ident_expr("x"),
                span: s(),
            },
            ret_stmt(ident_expr("x")),
        ]);
        let result = dce_block(b);
        assert_eq!(result.stmts.len(), 1, "x = x should be eliminated");
        assert!(matches!(result.stmts[0], TypedStmt::Return { .. }));
    }

    #[test]
    fn assignment_with_side_effecting_rhs_is_kept() {
        // let x = some_call()  (even if x is unused, call has side effects)
        let call_expr = TypedExpr {
            kind: TypedExprKind::Call {
                callee: Box::new(ident_expr("some_call")),
                args: vec![],
                fn_name: "some_call".into(),
                generic_bounds: vec![],
                generic_params: vec![],
                param_tys: vec![],
            },
            ty: Ty::Int,
            span: s(),
        };
        let b = block(vec![var_decl("x", call_expr), ret_stmt(int_expr(0))]);
        let result = dce_block(b);
        assert_eq!(
            result.stmts.len(),
            2,
            "side-effecting RHS must not be eliminated even if LHS is dead"
        );
    }
}
