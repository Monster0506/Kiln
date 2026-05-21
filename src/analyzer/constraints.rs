/// Simplified boolean constraint propagation.
///
/// Detects conditions that are syntactically always-true or always-false after
/// constant folding. Emits `TautologicalCondition` and `ContradictoryCondition`
/// analysis warnings.
///
/// This is the simple version: we only flag literal Bool conditions.
/// The full version (tracking inequalities across branches) is a future extension.
use crate::analyzer::error::AnalysisError;
use crate::analyzer::typed_ast::{
    TypedBlock, TypedExpr, TypedExprKind, TypedFile, TypedItem, TypedStmt,
};

/// Check all `if` conditions in a file for trivially true/false expressions.
pub fn check_tautological_conditions(file: &TypedFile) -> Vec<AnalysisError> {
    let mut errs = Vec::new();
    for item in &file.items {
        match item {
            TypedItem::Function(f) => check_block(&f.body, &mut errs),
            TypedItem::ImplBlock(ib) => {
                for m in &ib.methods {
                    check_block(&m.body, &mut errs);
                }
                for h in &ib.hooks {
                    check_block(&h.body, &mut errs);
                }
            }
            _ => {}
        }
    }
    errs
}

fn check_block(block: &TypedBlock, errs: &mut Vec<AnalysisError>) {
    for stmt in &block.stmts {
        check_stmt(stmt, errs);
    }
}

fn check_stmt(stmt: &TypedStmt, errs: &mut Vec<AnalysisError>) {
    match stmt {
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (cond, body) in branches {
                check_condition(cond, errs);
                check_block(body, errs);
            }
            if let Some(b) = else_branch {
                check_block(b, errs);
            }
        }
        TypedStmt::While { cond, body, .. } => {
            // `while true` is intentional (infinite loop); don't warn.
            if !matches!(cond.kind, TypedExprKind::Bool(true)) {
                check_condition(cond, errs);
            }
            check_block(body, errs);
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            check_block(body, errs);
            if !matches!(cond.kind, TypedExprKind::Bool(true)) {
                check_condition(cond, errs);
            }
        }
        TypedStmt::For { body, .. } => check_block(body, errs),
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            check_block(body, errs);
            for h in handlers {
                check_block(&h.body, errs);
            }
            if let Some(b) = finally {
                check_block(b, errs);
            }
        }
        TypedStmt::FnDef(f) => check_block(&f.body, errs),
        TypedStmt::Expr(_)
        | TypedStmt::VarDecl { .. }
        | TypedStmt::Assign { .. }
        | TypedStmt::CompoundAssign { .. }
        | TypedStmt::Return { .. }
        | TypedStmt::Raise { .. }
        | TypedStmt::Break(_)
        | TypedStmt::Continue(_) => {}
    }
}

fn check_condition(cond: &TypedExpr, errs: &mut Vec<AnalysisError>) {
    match &cond.kind {
        TypedExprKind::Bool(true) => {
            errs.push(AnalysisError::TautologicalCondition { span: cond.span });
        }
        TypedExprKind::Bool(false) => {
            errs.push(AnalysisError::ContradictoryCondition { span: cond.span });
        }
        _ => {}
    }
}

/// Check a single expression used as a condition (exported for tests).
pub fn check_expr_condition(expr: &TypedExpr) -> Option<AnalysisError> {
    let mut errs = Vec::new();
    check_condition(expr, &mut errs);
    errs.into_iter().next()
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

    fn bool_expr(b: bool) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Bool(b),
            ty: Ty::Bool,
            span: s(),
        }
    }

    fn ident_expr(name: &str) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Ident(name.into()),
            ty: Ty::Bool,
            span: s(),
        }
    }

    #[test]
    fn literal_true_condition_is_tautology() {
        let result = check_expr_condition(&bool_expr(true));
        assert!(
            matches!(result, Some(AnalysisError::TautologicalCondition { .. })),
            "Bool(true) as a condition should produce TautologicalCondition"
        );
    }

    #[test]
    fn literal_false_condition_is_contradiction() {
        let result = check_expr_condition(&bool_expr(false));
        assert!(
            matches!(result, Some(AnalysisError::ContradictoryCondition { .. })),
            "Bool(false) as a condition should produce ContradictoryCondition"
        );
    }

    #[test]
    fn independent_condition_produces_no_warning() {
        let result = check_expr_condition(&ident_expr("x"));
        assert!(
            result.is_none(),
            "variable condition should not produce a warning"
        );
    }
}
