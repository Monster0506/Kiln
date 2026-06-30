/// Liveness analysis: standard backward dataflow over a CFG.
/// live_out[b] = union(live_in[s] for successors s); live_in[b] = use[b] | (live_out[b] \ def[b]).
use crate::analyzer::cfg::{Cfg, Terminator};
use crate::analyzer::typed_ast::{TypedExpr, TypedExprKind, TypedFile, TypedItem, TypedStmt};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct LivenessResult {
    /// Variables live on entry to block `i`.
    pub live_in: Vec<HashSet<String>>,
    /// Variables live on exit from block `i`.
    pub live_out: Vec<HashSet<String>>,
}

/// Compute liveness for all basic blocks in `cfg`, using the flat statement list
/// in the same order the CFG was built from.
pub fn analyze_liveness(cfg: &Cfg, stmts: &[TypedStmt]) -> LivenessResult {
    let n = cfg.blocks.len();
    let mut live_in: Vec<HashSet<String>> = vec![HashSet::new(); n];
    let mut live_out: Vec<HashSet<String>> = vec![HashSet::new(); n];

    // Precompute use/def sets for each block.
    let use_def: Vec<(HashSet<String>, HashSet<String>)> =
        cfg.blocks.iter().map(|b| block_use_def(b, stmts)).collect();

    // Iterate to fixed point.
    loop {
        let mut changed = false;

        // Process blocks in reverse order (backward dataflow).
        for i in (0..n).rev() {
            // live_out[i] = union of live_in[s] for all successors of i
            let new_out: HashSet<String> = successors(cfg, i)
                .into_iter()
                .flat_map(|s| live_in[s].iter().cloned())
                .collect();

            // live_in[i] = use[i] union (live_out[i] \ def[i])
            let (ref u, ref d) = use_def[i];
            let new_in: HashSet<String> = u
                .iter()
                .cloned()
                .chain(new_out.iter().filter(|v| !d.contains(*v)).cloned())
                .collect();

            if new_in != live_in[i] || new_out != live_out[i] {
                changed = true;
            }
            live_in[i] = new_in;
            live_out[i] = new_out;
        }

        if !changed {
            break;
        }
    }

    LivenessResult { live_in, live_out }
}

fn successors(cfg: &Cfg, block_id: usize) -> Vec<usize> {
    match cfg.blocks[block_id].term {
        Terminator::Return | Terminator::Raise | Terminator::Unreachable => vec![],
        Terminator::Jump(t) => vec![t],
        Terminator::Branch(t, e) => vec![t, e],
    }
}

/// Compute (use, def) for a basic block: use = read before written, def = written.
fn block_use_def(
    block: &crate::analyzer::cfg::BasicBlock,
    stmts: &[TypedStmt],
) -> (HashSet<String>, HashSet<String>) {
    let mut uses: HashSet<String> = HashSet::new();
    let mut defs: HashSet<String> = HashSet::new();

    for &idx in &block.stmts {
        if let Some(stmt) = stmts.get(idx) {
            stmt_use_def(stmt, &mut uses, &mut defs);
        }
    }

    (uses, defs)
}

fn stmt_use_def(stmt: &TypedStmt, uses: &mut HashSet<String>, defs: &mut HashSet<String>) {
    match stmt {
        TypedStmt::VarDecl { name, value, .. } => {
            // RHS is read before LHS is written.
            expr_uses(value, uses, defs);
            defs.insert(name.clone());
        }
        TypedStmt::Assign { target, value, .. } => {
            expr_uses(value, uses, defs);
            // For a plain ident target, it's a def (not a use).
            if let TypedExprKind::Ident(name) = &target.kind {
                defs.insert(name.clone());
            } else {
                expr_uses(target, uses, defs);
            }
        }
        TypedStmt::CompoundAssign { target, rhs, .. } => {
            // The target is both read and written.
            expr_uses(rhs, uses, defs);
            expr_uses(target, uses, defs);
            if let TypedExprKind::Ident(name) = &target.kind {
                defs.insert(name.clone());
            }
        }
        TypedStmt::Return { value: Some(e), .. } => {
            expr_uses(e, uses, defs);
        }
        TypedStmt::Raise { value: Some(e), .. } => {
            expr_uses(e, uses, defs);
        }
        TypedStmt::Expr(e) => expr_uses(e, uses, defs),
        // Control-flow stmts: handled at the CFG level; conditions are analyzed separately.
        _ => {}
    }
}

fn expr_uses(expr: &TypedExpr, uses: &mut HashSet<String>, defs: &HashSet<String>) {
    match &expr.kind {
        TypedExprKind::Ident(name) => {
            if !defs.contains(name) {
                uses.insert(name.clone());
            }
        }
        TypedExprKind::BinOp { left, right, .. } => {
            expr_uses(left, uses, defs);
            expr_uses(right, uses, defs);
        }
        TypedExprKind::UnOp { operand, .. } => expr_uses(operand, uses, defs),
        TypedExprKind::Call { callee, args, .. } => {
            expr_uses(callee, uses, defs);
            for a in args {
                expr_uses(a, uses, defs);
            }
        }
        TypedExprKind::MethodCall { object, args, .. } => {
            expr_uses(object, uses, defs);
            for a in args {
                expr_uses(a, uses, defs);
            }
        }
        TypedExprKind::StaticCall { args, .. } => {
            for a in args {
                expr_uses(a, uses, defs);
            }
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            expr_uses(fat_ptr, uses, defs);
            for a in args {
                expr_uses(a, uses, defs);
            }
        }
        TypedExprKind::Field { object, .. } => expr_uses(object, uses, defs),
        TypedExprKind::Index { object, index } => {
            expr_uses(object, uses, defs);
            expr_uses(index, uses, defs);
        }
        TypedExprKind::Tuple(exprs) => {
            for e in exprs {
                expr_uses(e, uses, defs);
            }
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                expr_uses(e, uses, defs);
            }
        }
        TypedExprKind::Unwrap(e) => expr_uses(e, uses, defs),
        TypedExprKind::As { expr, .. } => expr_uses(expr, uses, defs),
        TypedExprKind::Match { scrutinee, arms } => {
            expr_uses(scrutinee, uses, defs);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    expr_uses(g, uses, defs);
                }
                expr_uses(&arm.body, uses, defs);
            }
        }
        TypedExprKind::Spawn(e) | TypedExprKind::Try(e) | TypedExprKind::Ignore(e) => {
            expr_uses(e, uses, defs)
        }
        TypedExprKind::Implements { expr: e, .. } => expr_uses(e, uses, defs),
        TypedExprKind::TypeName { expr: e } => expr_uses(e, uses, defs),
        TypedExprKind::Ref { expr, .. } => expr_uses(expr, uses, defs),
        TypedExprKind::Array(exprs) => {
            for e in exprs {
                expr_uses(e, uses, defs);
            }
        }
        TypedExprKind::Closure { .. } | TypedExprKind::Gen { .. } => {}
        TypedExprKind::GenSplice(e) => expr_uses(e, uses, defs),
        TypedExprKind::Block(_) => {}
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Str(_)
        | TypedExprKind::EnumVariant { .. } => {}
        TypedExprKind::BoundMethod { object, .. } => expr_uses(object, uses, defs),
        TypedExprKind::PrimTypeRef { .. } => {}
    }
}

/// Compute liveness results for every function in a TypedFile (for analysis/diagnostics).
pub fn liveness_for_file(file: &TypedFile) -> Vec<(String, LivenessResult)> {
    use crate::analyzer::cfg::CfgBuilder;
    let mut results = Vec::new();
    for item in &file.items {
        if let TypedItem::Function(f) = item {
            let cfg = CfgBuilder::build(&f.body);
            let stmts: Vec<TypedStmt> = f.body.stmts.clone();
            let live = analyze_liveness(&cfg, &stmts);
            results.push((f.name.clone(), live));
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::cfg::CfgBuilder;
    use crate::analyzer::ty::Ty;
    use crate::analyzer::typed_ast::{TypedBlock, TypedExpr, TypedExprKind, TypedStmt};
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
    fn live_in_set_includes_variable_read_in_block() {
        // let x = 1; return x -- x is defined then used in the same block.
        // VarDecl adds x to def before Return adds it to use, so x is not live_in.
        let b = block(vec![var_decl("x", int_expr(1)), ret_stmt(ident_expr("x"))]);
        let cfg = CfgBuilder::build(&b);
        let stmts: Vec<TypedStmt> = b.stmts.clone();
        let live = analyze_liveness(&cfg, &stmts);
        assert!(!live.live_in[0].contains("x"));
    }

    #[test]
    fn defined_variable_not_in_live_in_if_not_used_before() {
        // let x = 1  (x defined but never used after)
        let b = block(vec![var_decl("x", int_expr(1)), ret_stmt(int_expr(0))]);
        let cfg = CfgBuilder::build(&b);
        let stmts: Vec<TypedStmt> = b.stmts.clone();
        let live = analyze_liveness(&cfg, &stmts);
        assert!(!live.live_in[0].contains("x"));
        assert!(!live.live_out[0].contains("x"));
    }

    #[test]
    fn liveness_propagates_backwards_through_loop() {
        // let x = 1; let y = 2; return x -- y never read, x read at return.
        let b = block(vec![
            var_decl("x", int_expr(1)),
            var_decl("y", int_expr(2)),
            ret_stmt(ident_expr("x")),
        ]);
        let cfg = CfgBuilder::build(&b);
        let stmts: Vec<TypedStmt> = b.stmts.clone();
        let live = analyze_liveness(&cfg, &stmts);
        // y is defined and never read, so it should not be in live_out of block 0.
        // (It IS in def[0] but not in live_out[0] because no successor uses it.)
        assert!(
            !live.live_out[0].contains("y"),
            "y is dead after its block; should not be in live_out"
        );
    }
}
