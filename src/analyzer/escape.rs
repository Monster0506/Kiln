/// Escape analysis: determine which local struct bindings do NOT escape their function scope.
/// A variable escapes if returned, passed as an argument, or stored into a field/collection.
use crate::analyzer::typed_ast::{
    TypedBlock, TypedExpr, TypedExprKind, TypedFile, TypedItem, TypedStmt,
};
use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub struct EscapeInfo {
    /// Variable names that do NOT escape their enclosing function.
    pub non_escaping: HashSet<String>,
}

/// Analyze a function body and return escape info for its local bindings.
pub fn analyze_escapes(block: &TypedBlock) -> EscapeInfo {
    // Collect all locally declared variable names.
    let mut locals: HashSet<String> = HashSet::new();
    collect_locals_block(block, &mut locals);

    // Collect all names that escape.
    let mut escaping: HashSet<String> = HashSet::new();
    check_block_escapes(block, &mut escaping);

    let non_escaping = locals.difference(&escaping).cloned().collect();
    EscapeInfo { non_escaping }
}

/// Compute escape info for every function in a TypedFile.
pub fn escape_for_file(file: &TypedFile) -> Vec<(String, EscapeInfo)> {
    let mut results = Vec::new();
    for item in &file.items {
        if let TypedItem::Function(f) = item {
            let info = analyze_escapes(&f.body);
            results.push((f.name.clone(), info));
        }
    }
    results
}

fn collect_locals_block(block: &TypedBlock, out: &mut HashSet<String>) {
    for stmt in &block.stmts {
        collect_locals_stmt(stmt, out);
    }
}

fn collect_locals_stmt(stmt: &TypedStmt, out: &mut HashSet<String>) {
    match stmt {
        TypedStmt::VarDecl { name, .. } => {
            out.insert(name.clone());
        }
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (_, body) in branches {
                collect_locals_block(body, out);
            }
            if let Some(b) = else_branch {
                collect_locals_block(b, out);
            }
        }
        TypedStmt::While { body, .. } => collect_locals_block(body, out),
        TypedStmt::DoWhile { body, .. } => collect_locals_block(body, out),
        TypedStmt::For { body, .. } => collect_locals_block(body, out),
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            collect_locals_block(body, out);
            for h in handlers {
                collect_locals_block(&h.body, out);
            }
            if let Some(b) = finally {
                collect_locals_block(b, out);
            }
        }
        TypedStmt::FnDef(f) => collect_locals_block(&f.body, out),
        _ => {}
    }
}

fn check_block_escapes(block: &TypedBlock, escaping: &mut HashSet<String>) {
    for stmt in &block.stmts {
        check_stmt_escapes(stmt, escaping);
    }
}

fn check_stmt_escapes(stmt: &TypedStmt, escaping: &mut HashSet<String>) {
    match stmt {
        TypedStmt::Return { value: Some(e), .. } => {
            // Any ident returned escapes.
            collect_escaping_expr(e, escaping, true);
        }
        TypedStmt::Assign { target, value, .. } => {
            // x.field = y  -> y escapes (stored into struct field)
            if let TypedExprKind::Field { object, .. } = &target.kind {
                collect_idents(object, escaping);
            }
            // Also, if value is assigned to a field, it escapes.
            if matches!(target.kind, TypedExprKind::Field { .. }) {
                collect_idents(value, escaping);
            }
            collect_escaping_expr(value, escaping, false);
        }
        TypedStmt::VarDecl { value, .. } => {
            collect_escaping_expr(value, escaping, false);
        }
        TypedStmt::Expr(e) => collect_escaping_expr(e, escaping, false),
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (cond, body) in branches {
                collect_escaping_expr(cond, escaping, false);
                check_block_escapes(body, escaping);
            }
            if let Some(b) = else_branch {
                check_block_escapes(b, escaping);
            }
        }
        TypedStmt::While { cond, body, .. } => {
            collect_escaping_expr(cond, escaping, false);
            check_block_escapes(body, escaping);
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            check_block_escapes(body, escaping);
            collect_escaping_expr(cond, escaping, false);
        }
        TypedStmt::For { iterable, body, .. } => {
            collect_escaping_expr(iterable, escaping, false);
            check_block_escapes(body, escaping);
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            check_block_escapes(body, escaping);
            for h in handlers {
                check_block_escapes(&h.body, escaping);
            }
            if let Some(b) = finally {
                check_block_escapes(b, escaping);
            }
        }
        TypedStmt::Raise { value: Some(e), .. } => {
            collect_escaping_expr(e, escaping, true);
        }
        _ => {}
    }
}

fn collect_escaping_expr(expr: &TypedExpr, escaping: &mut HashSet<String>, returning: bool) {
    match &expr.kind {
        TypedExprKind::Ident(name) => {
            if returning {
                escaping.insert(name.clone());
            }
        }
        TypedExprKind::Call { callee, args, .. }
        | TypedExprKind::MethodCall {
            object: callee,
            args,
            ..
        } => {
            // Arguments passed to any call escape.
            collect_idents(callee, escaping);
            for a in args {
                collect_idents(a, escaping);
            }
        }
        TypedExprKind::StaticCall { args, .. } => {
            for a in args {
                collect_idents(a, escaping);
            }
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            collect_idents(fat_ptr, escaping);
            for a in args {
                collect_idents(a, escaping);
            }
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_escaping_expr(e, escaping, returning);
            }
        }
        TypedExprKind::BinOp { left, right, .. } => {
            collect_escaping_expr(left, escaping, returning);
            collect_escaping_expr(right, escaping, returning);
        }
        TypedExprKind::UnOp { operand, .. } => {
            collect_escaping_expr(operand, escaping, returning);
        }
        _ => {}
    }
}

fn collect_idents(expr: &TypedExpr, escaping: &mut HashSet<String>) {
    if let TypedExprKind::Ident(name) = &expr.kind {
        escaping.insert(name.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::Ty;
    use crate::analyzer::typed_ast::{TypedExpr, TypedExprKind, TypedStmt};
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

    fn block(stmts: Vec<TypedStmt>) -> TypedBlock {
        TypedBlock { stmts, span: s() }
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

    #[test]
    fn non_escaping_local_is_identified() {
        // let x = 42   (x is defined but never returned or passed to a call)
        let b = block(vec![
            var_decl("x", int_expr(42)),
            TypedStmt::Return {
                value: Some(int_expr(0)),
                span: s(),
            },
        ]);
        let info = analyze_escapes(&b);
        assert!(
            info.non_escaping.contains("x"),
            "x should be non-escaping since it is never returned or passed to calls"
        );
    }

    #[test]
    fn returned_variable_escapes() {
        // let x = 42; return x
        let b = block(vec![
            var_decl("x", int_expr(42)),
            TypedStmt::Return {
                value: Some(ident_expr("x")),
                span: s(),
            },
        ]);
        let info = analyze_escapes(&b);
        assert!(
            !info.non_escaping.contains("x"),
            "x should escape because it is returned"
        );
    }

    #[test]
    fn variable_passed_to_call_escapes() {
        // let x = 42; some_fn(x)
        let call_expr = TypedExpr {
            kind: TypedExprKind::Call {
                callee: Box::new(ident_expr("some_fn")),
                args: vec![ident_expr("x")],
                fn_name: "some_fn".into(),
                generic_bounds: vec![],
                generic_params: vec![],
                param_tys: vec![],
            },
            ty: Ty::Void,
            span: s(),
        };
        let b = block(vec![
            var_decl("x", int_expr(42)),
            TypedStmt::Expr(call_expr),
        ]);
        let info = analyze_escapes(&b);
        assert!(
            !info.non_escaping.contains("x"),
            "x should escape because it is passed to a call"
        );
    }
}
