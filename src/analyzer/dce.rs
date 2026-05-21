/// Dead assignment elimination: remove VarDecl bindings that are never read,
/// provided the initializer expression has no side effects.
///
/// This is a simple backward linear scan within each block. It does not
/// handle cross-block liveness (loops, branches); those cases are conservative.
use crate::analyzer::typed_ast::{
    TypedBlock, TypedCatchHandler, TypedExpr, TypedExprKind, TypedFile, TypedFnDef, TypedHookDef,
    TypedImplBlock, TypedItem, TypedStmt,
};
use std::collections::{HashMap, HashSet};

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

// --- Emit-only passes ---------------------------------------------------------

/// Write-after-write elimination (emit path only).
/// `mut x = e1; x = e2` -> `mut x = e2` when e2 does not read x.
pub fn waw_file(file: TypedFile) -> TypedFile {
    let items = file
        .items
        .into_iter()
        .map(|item| match item {
            TypedItem::Function(mut f) => {
                f.body = waw_block(f.body);
                TypedItem::Function(f)
            }
            TypedItem::ImplBlock(mut ib) => {
                ib.methods = ib
                    .methods
                    .into_iter()
                    .map(|mut f| {
                        f.body = waw_block(f.body);
                        f
                    })
                    .collect();
                ib.hooks = ib
                    .hooks
                    .into_iter()
                    .map(|mut h| {
                        h.body = waw_block(h.body);
                        h
                    })
                    .collect();
                TypedItem::ImplBlock(ib)
            }
            other => other,
        })
        .collect();
    TypedFile {
        items,
        span: file.span,
    }
}

fn waw_block(block: TypedBlock) -> TypedBlock {
    let mut stmts = block.stmts;
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i + 1 < stmts.len() {
            // Pattern: VarDecl { name, mutable:true, value: e1 } followed by Assign { target: Ident(name), value: e2 }
            // where e2 does not read `name`. Replace with VarDecl { name, value: e2 }.
            let is_waw = if let (
                TypedStmt::VarDecl {
                    name: n1,
                    mutable: true,
                    ..
                },
                TypedStmt::Assign {
                    target, value: e2, ..
                },
            ) = (&stmts[i], &stmts[i + 1])
            {
                if let TypedExprKind::Ident(n2) = &target.kind {
                    n1 == n2 && !expr_reads_name(e2, n2)
                } else {
                    false
                }
            } else {
                false
            };
            if is_waw {
                let assign = stmts.remove(i + 1);
                if let TypedStmt::Assign { value: e2, .. } = assign {
                    if let TypedStmt::VarDecl { value, .. } = &mut stmts[i] {
                        *value = e2;
                        changed = true;
                    }
                }
            } else {
                i += 1;
            }
        }
    }
    // Recurse into sub-blocks.
    let stmts = stmts.into_iter().map(waw_stmt).collect();
    TypedBlock {
        stmts,
        span: block.span,
    }
}

fn waw_stmt(stmt: TypedStmt) -> TypedStmt {
    match stmt {
        TypedStmt::If {
            branches,
            else_branch,
            span,
        } => TypedStmt::If {
            branches: branches
                .into_iter()
                .map(|(c, b)| (c, waw_block(b)))
                .collect(),
            else_branch: else_branch.map(waw_block),
            span,
        },
        TypedStmt::While { cond, body, span } => TypedStmt::While {
            cond,
            body: waw_block(body),
            span,
        },
        TypedStmt::DoWhile { body, cond, span } => TypedStmt::DoWhile {
            body: waw_block(body),
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
            body: waw_block(body),
            iter_ty,
            span,
        },
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => TypedStmt::TryCatch {
            body: waw_block(body),
            handlers: handlers
                .into_iter()
                .map(|mut h| {
                    h.body = waw_block(h.body);
                    h
                })
                .collect(),
            finally: finally.map(waw_block),
            span,
        },
        other => other,
    }
}

fn expr_reads_name(expr: &TypedExpr, name: &str) -> bool {
    match &expr.kind {
        TypedExprKind::Ident(n) => n == name,
        TypedExprKind::BinOp { left, right, .. } => {
            expr_reads_name(left, name) || expr_reads_name(right, name)
        }
        TypedExprKind::UnOp { operand, .. } => expr_reads_name(operand, name),
        TypedExprKind::Call { callee, args, .. } => {
            expr_reads_name(callee, name) || args.iter().any(|a| expr_reads_name(a, name))
        }
        TypedExprKind::MethodCall { object, args, .. } => {
            expr_reads_name(object, name) || args.iter().any(|a| expr_reads_name(a, name))
        }
        TypedExprKind::StaticCall { args, .. } => args.iter().any(|a| expr_reads_name(a, name)),
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            expr_reads_name(fat_ptr, name) || args.iter().any(|a| expr_reads_name(a, name))
        }
        TypedExprKind::Field { object, .. } => expr_reads_name(object, name),
        TypedExprKind::Index { object, index } => {
            expr_reads_name(object, name) || expr_reads_name(index, name)
        }
        TypedExprKind::Tuple(exprs) | TypedExprKind::Array(exprs) => {
            exprs.iter().any(|e| expr_reads_name(e, name))
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            fields.iter().any(|(_, e)| expr_reads_name(e, name))
        }
        TypedExprKind::Unwrap(e) | TypedExprKind::Spawn(e) | TypedExprKind::GenSplice(e) => {
            expr_reads_name(e, name)
        }
        TypedExprKind::As { expr, .. } | TypedExprKind::Ref { expr, .. } => {
            expr_reads_name(expr, name)
        }
        TypedExprKind::Match { scrutinee, arms } => {
            expr_reads_name(scrutinee, name)
                || arms.iter().any(|a| {
                    a.guard.as_ref().is_some_and(|g| expr_reads_name(g, name))
                        || expr_reads_name(&a.body, name)
                })
        }
        _ => false,
    }
}

/// Demote `mut` flags on VarDecls that are never assigned within their block tree (emit path only).
/// This is safe only in the emit path because codegen uses `mutable` for alloca decisions.
pub fn demote_mut_flags_file(file: TypedFile) -> TypedFile {
    let items = file
        .items
        .into_iter()
        .map(|item| match item {
            TypedItem::Function(mut f) => {
                f.body = demote_mut_block(f.body);
                TypedItem::Function(f)
            }
            TypedItem::ImplBlock(mut ib) => {
                ib.methods = ib
                    .methods
                    .into_iter()
                    .map(|mut f| {
                        f.body = demote_mut_block(f.body);
                        f
                    })
                    .collect();
                ib.hooks = ib
                    .hooks
                    .into_iter()
                    .map(|mut h| {
                        h.body = demote_mut_block(h.body);
                        h
                    })
                    .collect();
                TypedItem::ImplBlock(ib)
            }
            other => other,
        })
        .collect();
    TypedFile {
        items,
        span: file.span,
    }
}

fn demote_mut_block(block: TypedBlock) -> TypedBlock {
    let assigned = collect_assigned_in_block_tree(&block);
    let stmts = block
        .stmts
        .into_iter()
        .map(|stmt| demote_mut_stmt(stmt, &assigned))
        .collect();
    TypedBlock {
        stmts,
        span: block.span,
    }
}

fn demote_mut_stmt(stmt: TypedStmt, assigned: &HashSet<String>) -> TypedStmt {
    match stmt {
        TypedStmt::VarDecl {
            name,
            ty,
            value,
            mutable,
            span,
        } => TypedStmt::VarDecl {
            mutable: mutable && assigned.contains(&name),
            name,
            ty,
            value,
            span,
        },
        TypedStmt::If {
            branches,
            else_branch,
            span,
        } => TypedStmt::If {
            branches: branches
                .into_iter()
                .map(|(c, b)| (c, demote_mut_block(b)))
                .collect(),
            else_branch: else_branch.map(demote_mut_block),
            span,
        },
        TypedStmt::While { cond, body, span } => TypedStmt::While {
            cond,
            body: demote_mut_block(body),
            span,
        },
        TypedStmt::DoWhile { body, cond, span } => TypedStmt::DoWhile {
            body: demote_mut_block(body),
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
            body: demote_mut_block(body),
            iter_ty,
            span,
        },
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => TypedStmt::TryCatch {
            body: demote_mut_block(body),
            handlers: handlers
                .into_iter()
                .map(|mut h| {
                    h.body = demote_mut_block(h.body);
                    h
                })
                .collect(),
            finally: finally.map(demote_mut_block),
            span,
        },
        other => other,
    }
}

fn collect_assigned_in_block_tree(block: &TypedBlock) -> HashSet<String> {
    let mut out = HashSet::new();
    collect_assigned_stmts(&block.stmts, &mut out);
    out
}

fn collect_assigned_stmts(stmts: &[TypedStmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            TypedStmt::Assign { target, .. } => {
                if let TypedExprKind::Ident(name) = &target.kind {
                    out.insert(name.clone());
                }
            }
            TypedStmt::CompoundAssign { target, .. } => {
                if let TypedExprKind::Ident(name) = &target.kind {
                    out.insert(name.clone());
                }
            }
            TypedStmt::If {
                branches,
                else_branch,
                ..
            } => {
                for (_, b) in branches {
                    collect_assigned_stmts(&b.stmts, out);
                }
                if let Some(b) = else_branch {
                    collect_assigned_stmts(&b.stmts, out);
                }
            }
            TypedStmt::While { body, .. } | TypedStmt::DoWhile { body, .. } => {
                collect_assigned_stmts(&body.stmts, out);
            }
            TypedStmt::For { body, .. } => collect_assigned_stmts(&body.stmts, out),
            TypedStmt::TryCatch {
                body,
                handlers,
                finally,
                ..
            } => {
                collect_assigned_stmts(&body.stmts, out);
                for h in handlers {
                    collect_assigned_stmts(&h.body.stmts, out);
                }
                if let Some(b) = finally {
                    collect_assigned_stmts(&b.stmts, out);
                }
            }
            _ => {}
        }
    }
}

/// Inline immutable single-use bindings at their use site (emit path only).
/// For each `x: T = expr` where x appears exactly once in subsequent stmts and is never assigned,
/// replace that read with `expr` and remove the VarDecl.
pub fn single_use_inline_file(file: TypedFile) -> TypedFile {
    let items = file
        .items
        .into_iter()
        .map(|item| match item {
            TypedItem::Function(mut f) => {
                f.body = single_use_inline_block(f.body);
                TypedItem::Function(f)
            }
            TypedItem::ImplBlock(mut ib) => {
                ib.methods = ib
                    .methods
                    .into_iter()
                    .map(|mut f| {
                        f.body = single_use_inline_block(f.body);
                        f
                    })
                    .collect();
                ib.hooks = ib
                    .hooks
                    .into_iter()
                    .map(|mut h| {
                        h.body = single_use_inline_block(h.body);
                        h
                    })
                    .collect();
                TypedItem::ImplBlock(ib)
            }
            other => other,
        })
        .collect();
    TypedFile {
        items,
        span: file.span,
    }
}

fn single_use_inline_block(block: TypedBlock) -> TypedBlock {
    let mut stmts = block.stmts;
    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;
        while i < stmts.len() {
            // Only inline immutable (non-mut) bindings with no side effects.
            let candidate = if let TypedStmt::VarDecl {
                name,
                mutable: false,
                value,
                ..
            } = &stmts[i]
            {
                if !expr_has_side_effects(value) {
                    Some((name.clone(), value.clone()))
                } else {
                    None
                }
            } else {
                None
            };
            if let Some((name, value)) = candidate {
                let subsequent = &stmts[i + 1..];
                let count = count_reads_in_stmts(subsequent, &name);
                if count == 1 {
                    // Inline: substitute in subsequent stmts, remove VarDecl.
                    let rest: Vec<TypedStmt> = stmts.drain(i + 1..).collect();
                    let rest: Vec<TypedStmt> = rest
                        .into_iter()
                        .map(|s| substitute_name_in_stmt(s, &name, &value))
                        .collect();
                    stmts.remove(i); // remove the VarDecl
                    stmts.extend(rest);
                    changed = true;
                    // Don't advance i; try again at same position.
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
    }
    // Recurse into sub-blocks.
    let stmts = stmts.into_iter().map(single_use_inline_stmt).collect();
    TypedBlock {
        stmts,
        span: block.span,
    }
}

fn single_use_inline_stmt(stmt: TypedStmt) -> TypedStmt {
    match stmt {
        TypedStmt::If {
            branches,
            else_branch,
            span,
        } => TypedStmt::If {
            branches: branches
                .into_iter()
                .map(|(c, b)| (c, single_use_inline_block(b)))
                .collect(),
            else_branch: else_branch.map(single_use_inline_block),
            span,
        },
        TypedStmt::While { cond, body, span } => TypedStmt::While {
            cond,
            body: single_use_inline_block(body),
            span,
        },
        TypedStmt::DoWhile { body, cond, span } => TypedStmt::DoWhile {
            body: single_use_inline_block(body),
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
            body: single_use_inline_block(body),
            iter_ty,
            span,
        },
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => TypedStmt::TryCatch {
            body: single_use_inline_block(body),
            handlers: handlers
                .into_iter()
                .map(|mut h| {
                    h.body = single_use_inline_block(h.body);
                    h
                })
                .collect(),
            finally: finally.map(single_use_inline_block),
            span,
        },
        other => other,
    }
}

fn count_reads_in_stmts(stmts: &[TypedStmt], name: &str) -> usize {
    stmts.iter().map(|s| count_reads_in_stmt(s, name)).sum()
}

fn count_reads_in_stmt(stmt: &TypedStmt, name: &str) -> usize {
    match stmt {
        TypedStmt::VarDecl { value, .. } => count_reads_in_expr(value, name),
        TypedStmt::Assign { target, value, .. } => {
            count_reads_in_expr(value, name) + count_reads_in_target_expr(target, name)
        }
        TypedStmt::CompoundAssign { target, rhs, .. } => {
            count_reads_in_expr(target, name) + count_reads_in_expr(rhs, name)
        }
        TypedStmt::Return { value, .. } => {
            value.as_ref().map_or(0, |e| count_reads_in_expr(e, name))
        }
        TypedStmt::Raise { value, .. } => {
            value.as_ref().map_or(0, |e| count_reads_in_expr(e, name))
        }
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            branches
                .iter()
                .map(|(c, b)| count_reads_in_expr(c, name) + count_reads_in_stmts(&b.stmts, name))
                .sum::<usize>()
                + else_branch
                    .as_ref()
                    .map_or(0, |b| count_reads_in_stmts(&b.stmts, name))
        }
        TypedStmt::While { cond, body, .. } => {
            count_reads_in_expr(cond, name) + count_reads_in_stmts(&body.stmts, name)
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            count_reads_in_stmts(&body.stmts, name) + count_reads_in_expr(cond, name)
        }
        TypedStmt::For { iterable, body, .. } => {
            count_reads_in_expr(iterable, name) + count_reads_in_stmts(&body.stmts, name)
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            count_reads_in_stmts(&body.stmts, name)
                + handlers
                    .iter()
                    .map(|h| count_reads_in_stmts(&h.body.stmts, name))
                    .sum::<usize>()
                + finally
                    .as_ref()
                    .map_or(0, |b| count_reads_in_stmts(&b.stmts, name))
        }
        TypedStmt::Expr(e) => count_reads_in_expr(e, name),
        TypedStmt::FnDef(f) => count_reads_in_stmts(&f.body.stmts, name),
        TypedStmt::Break(_) | TypedStmt::Continue(_) => 0,
    }
}

fn count_reads_in_target_expr(expr: &TypedExpr, name: &str) -> usize {
    match &expr.kind {
        TypedExprKind::Ident(_) => 0, // writing to ident, not reading it
        TypedExprKind::Field { object, .. } => count_reads_in_expr(object, name),
        TypedExprKind::Index { object, index } => {
            count_reads_in_expr(object, name) + count_reads_in_expr(index, name)
        }
        _ => count_reads_in_expr(expr, name),
    }
}

fn count_reads_in_expr(expr: &TypedExpr, name: &str) -> usize {
    match &expr.kind {
        TypedExprKind::Ident(n) => {
            if n == name {
                1
            } else {
                0
            }
        }
        TypedExprKind::BinOp { left, right, .. } => {
            count_reads_in_expr(left, name) + count_reads_in_expr(right, name)
        }
        TypedExprKind::UnOp { operand, .. } => count_reads_in_expr(operand, name),
        TypedExprKind::Call { callee, args, .. } => {
            count_reads_in_expr(callee, name)
                + args
                    .iter()
                    .map(|a| count_reads_in_expr(a, name))
                    .sum::<usize>()
        }
        TypedExprKind::MethodCall { object, args, .. } => {
            count_reads_in_expr(object, name)
                + args
                    .iter()
                    .map(|a| count_reads_in_expr(a, name))
                    .sum::<usize>()
        }
        TypedExprKind::StaticCall { args, .. } => {
            args.iter().map(|a| count_reads_in_expr(a, name)).sum()
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            count_reads_in_expr(fat_ptr, name)
                + args
                    .iter()
                    .map(|a| count_reads_in_expr(a, name))
                    .sum::<usize>()
        }
        TypedExprKind::Field { object, .. } => count_reads_in_expr(object, name),
        TypedExprKind::Index { object, index } => {
            count_reads_in_expr(object, name) + count_reads_in_expr(index, name)
        }
        TypedExprKind::Tuple(exprs) | TypedExprKind::Array(exprs) => {
            exprs.iter().map(|e| count_reads_in_expr(e, name)).sum()
        }
        TypedExprKind::StructLiteral { fields, .. } => fields
            .iter()
            .map(|(_, e)| count_reads_in_expr(e, name))
            .sum(),
        TypedExprKind::Unwrap(e) | TypedExprKind::Spawn(e) | TypedExprKind::GenSplice(e) => {
            count_reads_in_expr(e, name)
        }
        TypedExprKind::As { expr, .. } | TypedExprKind::Ref { expr, .. } => {
            count_reads_in_expr(expr, name)
        }
        TypedExprKind::Match { scrutinee, arms } => {
            count_reads_in_expr(scrutinee, name)
                + arms
                    .iter()
                    .map(|a| {
                        a.guard.as_ref().map_or(0, |g| count_reads_in_expr(g, name))
                            + count_reads_in_expr(&a.body, name)
                    })
                    .sum::<usize>()
        }
        TypedExprKind::Str(segs) => {
            use crate::analyzer::typed_ast::TypedStringSegment;
            segs.iter()
                .map(|s| match s {
                    TypedStringSegment::Interp(e) => count_reads_in_expr(e, name),
                    TypedStringSegment::Text(_) => 0,
                })
                .sum()
        }
        TypedExprKind::Closure { body, .. } => {
            use crate::analyzer::typed_ast::TypedClosureBody;
            match body {
                TypedClosureBody::Expr(e) => count_reads_in_expr(e, name),
                TypedClosureBody::Block(b) => count_reads_in_stmts(&b.stmts, name),
            }
        }
        TypedExprKind::Gen { body } => count_reads_in_stmts(&body.stmts, name),
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::EnumVariant { .. } => 0,
    }
}

fn substitute_name_in_stmt(stmt: TypedStmt, name: &str, replacement: &TypedExpr) -> TypedStmt {
    match stmt {
        TypedStmt::VarDecl {
            name: n,
            ty,
            value,
            mutable,
            span,
        } => TypedStmt::VarDecl {
            value: substitute_name_in_expr(value, name, replacement),
            name: n,
            ty,
            mutable,
            span,
        },
        TypedStmt::Assign {
            target,
            value,
            span,
        } => TypedStmt::Assign {
            value: substitute_name_in_expr(value, name, replacement),
            target: substitute_name_in_expr(target, name, replacement),
            span,
        },
        TypedStmt::CompoundAssign {
            target,
            op,
            rhs,
            span,
        } => TypedStmt::CompoundAssign {
            rhs: substitute_name_in_expr(rhs, name, replacement),
            target: substitute_name_in_expr(target, name, replacement),
            op,
            span,
        },
        TypedStmt::Return { value, span } => TypedStmt::Return {
            value: value.map(|e| substitute_name_in_expr(e, name, replacement)),
            span,
        },
        TypedStmt::Raise { value, span } => TypedStmt::Raise {
            value: value.map(|e| substitute_name_in_expr(e, name, replacement)),
            span,
        },
        TypedStmt::Expr(e) => TypedStmt::Expr(substitute_name_in_expr(e, name, replacement)),
        TypedStmt::If {
            branches,
            else_branch,
            span,
        } => TypedStmt::If {
            branches: branches
                .into_iter()
                .map(|(c, b)| {
                    (
                        substitute_name_in_expr(c, name, replacement),
                        substitute_name_in_block(b, name, replacement),
                    )
                })
                .collect(),
            else_branch: else_branch.map(|b| substitute_name_in_block(b, name, replacement)),
            span,
        },
        TypedStmt::While { cond, body, span } => TypedStmt::While {
            cond: substitute_name_in_expr(cond, name, replacement),
            body: substitute_name_in_block(body, name, replacement),
            span,
        },
        TypedStmt::DoWhile { body, cond, span } => TypedStmt::DoWhile {
            body: substitute_name_in_block(body, name, replacement),
            cond: substitute_name_in_expr(cond, name, replacement),
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
            iterable: substitute_name_in_expr(iterable, name, replacement),
            body: substitute_name_in_block(body, name, replacement),
            binding,
            binding_ty,
            iter_ty,
            span,
        },
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => TypedStmt::TryCatch {
            body: substitute_name_in_block(body, name, replacement),
            handlers: handlers
                .into_iter()
                .map(|mut h| {
                    h.body = substitute_name_in_block(h.body, name, replacement);
                    h
                })
                .collect(),
            finally: finally.map(|b| substitute_name_in_block(b, name, replacement)),
            span,
        },
        other => other,
    }
}

fn substitute_name_in_block(block: TypedBlock, name: &str, replacement: &TypedExpr) -> TypedBlock {
    let stmts = block
        .stmts
        .into_iter()
        .map(|s| substitute_name_in_stmt(s, name, replacement))
        .collect();
    TypedBlock {
        stmts,
        span: block.span,
    }
}

fn substitute_name_in_expr(expr: TypedExpr, name: &str, replacement: &TypedExpr) -> TypedExpr {
    let ty = expr.ty.clone();
    let span = expr.span;
    let kind = match expr.kind {
        TypedExprKind::Ident(ref n) if n == name => return replacement.clone(),
        TypedExprKind::BinOp { op, left, right } => TypedExprKind::BinOp {
            op,
            left: Box::new(substitute_name_in_expr(*left, name, replacement)),
            right: Box::new(substitute_name_in_expr(*right, name, replacement)),
        },
        TypedExprKind::UnOp { op, operand } => TypedExprKind::UnOp {
            op,
            operand: Box::new(substitute_name_in_expr(*operand, name, replacement)),
        },
        TypedExprKind::Call {
            callee,
            args,
            fn_name,
            generic_bounds,
            generic_params,
            param_tys,
        } => TypedExprKind::Call {
            callee: Box::new(substitute_name_in_expr(*callee, name, replacement)),
            args: args
                .into_iter()
                .map(|a| substitute_name_in_expr(a, name, replacement))
                .collect(),
            fn_name,
            generic_bounds,
            generic_params,
            param_tys,
        },
        TypedExprKind::MethodCall {
            object,
            method_fn,
            args,
        } => TypedExprKind::MethodCall {
            object: Box::new(substitute_name_in_expr(*object, name, replacement)),
            args: args
                .into_iter()
                .map(|a| substitute_name_in_expr(a, name, replacement))
                .collect(),
            method_fn,
        },
        TypedExprKind::StaticCall { method_fn, args } => TypedExprKind::StaticCall {
            args: args
                .into_iter()
                .map(|a| substitute_name_in_expr(a, name, replacement))
                .collect(),
            method_fn,
        },
        TypedExprKind::IndirectCall { fat_ptr, args } => TypedExprKind::IndirectCall {
            fat_ptr: Box::new(substitute_name_in_expr(*fat_ptr, name, replacement)),
            args: args
                .into_iter()
                .map(|a| substitute_name_in_expr(a, name, replacement))
                .collect(),
        },
        TypedExprKind::Field { object, field } => TypedExprKind::Field {
            object: Box::new(substitute_name_in_expr(*object, name, replacement)),
            field,
        },
        TypedExprKind::Index { object, index } => TypedExprKind::Index {
            object: Box::new(substitute_name_in_expr(*object, name, replacement)),
            index: Box::new(substitute_name_in_expr(*index, name, replacement)),
        },
        TypedExprKind::Tuple(exprs) => TypedExprKind::Tuple(
            exprs
                .into_iter()
                .map(|e| substitute_name_in_expr(e, name, replacement))
                .collect(),
        ),
        TypedExprKind::Array(exprs) => TypedExprKind::Array(
            exprs
                .into_iter()
                .map(|e| substitute_name_in_expr(e, name, replacement))
                .collect(),
        ),
        TypedExprKind::StructLiteral { ty_name, fields } => TypedExprKind::StructLiteral {
            ty_name,
            fields: fields
                .into_iter()
                .map(|(k, v)| (k, substitute_name_in_expr(v, name, replacement)))
                .collect(),
        },
        TypedExprKind::Unwrap(e) => {
            TypedExprKind::Unwrap(Box::new(substitute_name_in_expr(*e, name, replacement)))
        }
        TypedExprKind::Spawn(e) => {
            TypedExprKind::Spawn(Box::new(substitute_name_in_expr(*e, name, replacement)))
        }
        TypedExprKind::GenSplice(e) => {
            TypedExprKind::GenSplice(Box::new(substitute_name_in_expr(*e, name, replacement)))
        }
        TypedExprKind::As { expr: e, ty: aty } => TypedExprKind::As {
            expr: Box::new(substitute_name_in_expr(*e, name, replacement)),
            ty: aty,
        },
        TypedExprKind::Ref { mutable, expr: e } => TypedExprKind::Ref {
            mutable,
            expr: Box::new(substitute_name_in_expr(*e, name, replacement)),
        },
        TypedExprKind::Str(segs) => {
            use crate::analyzer::typed_ast::TypedStringSegment;
            TypedExprKind::Str(
                segs.into_iter()
                    .map(|s| match s {
                        TypedStringSegment::Interp(e) => TypedStringSegment::Interp(
                            substitute_name_in_expr(e, name, replacement),
                        ),
                        other => other,
                    })
                    .collect(),
            )
        }
        TypedExprKind::Match { scrutinee, arms } => TypedExprKind::Match {
            scrutinee: Box::new(substitute_name_in_expr(*scrutinee, name, replacement)),
            arms: arms
                .into_iter()
                .map(|mut arm| {
                    arm.body = substitute_name_in_expr(arm.body, name, replacement);
                    if let Some(g) = arm.guard {
                        arm.guard = Some(substitute_name_in_expr(g, name, replacement));
                    }
                    arm
                })
                .collect(),
        },
        TypedExprKind::Closure { params, body } => {
            use crate::analyzer::typed_ast::TypedClosureBody;
            TypedExprKind::Closure {
                params,
                body: match body {
                    TypedClosureBody::Expr(e) => TypedClosureBody::Expr(Box::new(
                        substitute_name_in_expr(*e, name, replacement),
                    )),
                    TypedClosureBody::Block(b) => {
                        TypedClosureBody::Block(substitute_name_in_block(b, name, replacement))
                    }
                },
            }
        }
        TypedExprKind::Gen { body } => TypedExprKind::Gen {
            body: substitute_name_in_block(body, name, replacement),
        },
        other => other,
    };
    TypedExpr { kind, ty, span }
}

/// Remove user-defined functions not reachable from `main` (emit path only).
pub fn eliminate_dead_fns(file: TypedFile, user_names: &HashSet<String>) -> TypedFile {
    // Collect all function bodies by name for reachability BFS.
    let mut fn_bodies: HashMap<String, Vec<TypedStmt>> = HashMap::new();
    for item in &file.items {
        if let TypedItem::Function(f) = item {
            fn_bodies.insert(f.name.clone(), f.body.stmts.clone());
        }
    }

    // BFS from main.
    let mut reachable: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = vec!["main".into()];
    while let Some(name) = queue.pop() {
        if reachable.contains(&name) {
            continue;
        }
        reachable.insert(name.clone());
        if let Some(stmts) = fn_bodies.get(&name) {
            let mut called = HashSet::new();
            for s in stmts {
                collect_called_fns_stmt(s, &mut called);
            }
            for callee in called {
                if !reachable.contains(&callee) {
                    queue.push(callee);
                }
            }
        }
    }

    let items = file
        .items
        .into_iter()
        .filter(|item| {
            match item {
                TypedItem::Function(f) => {
                    // Keep if not user-defined, or if reachable.
                    !user_names.contains(&f.name) || reachable.contains(&f.name)
                }
                _ => true,
            }
        })
        .collect();
    TypedFile {
        items,
        span: file.span,
    }
}

fn collect_called_fns_stmt(stmt: &TypedStmt, out: &mut HashSet<String>) {
    match stmt {
        TypedStmt::VarDecl { value, .. } => collect_called_fns_expr(value, out),
        TypedStmt::Assign { value, target, .. } => {
            collect_called_fns_expr(value, out);
            collect_called_fns_expr(target, out);
        }
        TypedStmt::CompoundAssign { rhs, target, .. } => {
            collect_called_fns_expr(rhs, out);
            collect_called_fns_expr(target, out);
        }
        TypedStmt::Return { value, .. } => {
            if let Some(e) = value {
                collect_called_fns_expr(e, out);
            }
        }
        TypedStmt::Raise { value, .. } => {
            if let Some(e) = value {
                collect_called_fns_expr(e, out);
            }
        }
        TypedStmt::Expr(e) => collect_called_fns_expr(e, out),
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (c, b) in branches {
                collect_called_fns_expr(c, out);
                for s in &b.stmts {
                    collect_called_fns_stmt(s, out);
                }
            }
            if let Some(b) = else_branch {
                for s in &b.stmts {
                    collect_called_fns_stmt(s, out);
                }
            }
        }
        TypedStmt::While { cond, body, .. } => {
            collect_called_fns_expr(cond, out);
            for s in &body.stmts {
                collect_called_fns_stmt(s, out);
            }
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            for s in &body.stmts {
                collect_called_fns_stmt(s, out);
            }
            collect_called_fns_expr(cond, out);
        }
        TypedStmt::For { iterable, body, .. } => {
            collect_called_fns_expr(iterable, out);
            for s in &body.stmts {
                collect_called_fns_stmt(s, out);
            }
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            for s in &body.stmts {
                collect_called_fns_stmt(s, out);
            }
            for h in handlers {
                for s in &h.body.stmts {
                    collect_called_fns_stmt(s, out);
                }
            }
            if let Some(b) = finally {
                for s in &b.stmts {
                    collect_called_fns_stmt(s, out);
                }
            }
        }
        TypedStmt::FnDef(f) => {
            for s in &f.body.stmts {
                collect_called_fns_stmt(s, out);
            }
        }
        TypedStmt::Break(_) | TypedStmt::Continue(_) => {}
    }
}

fn collect_called_fns_expr(expr: &TypedExpr, out: &mut HashSet<String>) {
    match &expr.kind {
        TypedExprKind::Call {
            fn_name,
            callee,
            args,
            ..
        } => {
            out.insert(fn_name.clone());
            collect_called_fns_expr(callee, out);
            for a in args {
                collect_called_fns_expr(a, out);
            }
        }
        TypedExprKind::MethodCall {
            object,
            method_fn,
            args,
        } => {
            out.insert(method_fn.clone());
            collect_called_fns_expr(object, out);
            for a in args {
                collect_called_fns_expr(a, out);
            }
        }
        TypedExprKind::StaticCall {
            method_fn, args, ..
        } => {
            out.insert(method_fn.clone());
            for a in args {
                collect_called_fns_expr(a, out);
            }
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            collect_called_fns_expr(fat_ptr, out);
            for a in args {
                collect_called_fns_expr(a, out);
            }
        }
        TypedExprKind::BinOp { left, right, .. } => {
            collect_called_fns_expr(left, out);
            collect_called_fns_expr(right, out);
        }
        TypedExprKind::UnOp { operand, .. } => collect_called_fns_expr(operand, out),
        TypedExprKind::Field { object, .. } => collect_called_fns_expr(object, out),
        TypedExprKind::Index { object, index } => {
            collect_called_fns_expr(object, out);
            collect_called_fns_expr(index, out);
        }
        TypedExprKind::Tuple(exprs) | TypedExprKind::Array(exprs) => {
            for e in exprs {
                collect_called_fns_expr(e, out);
            }
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_called_fns_expr(e, out);
            }
        }
        TypedExprKind::Unwrap(e) | TypedExprKind::Spawn(e) | TypedExprKind::GenSplice(e) => {
            collect_called_fns_expr(e, out)
        }
        TypedExprKind::As { expr, .. } | TypedExprKind::Ref { expr, .. } => {
            collect_called_fns_expr(expr, out)
        }
        TypedExprKind::Match { scrutinee, arms } => {
            collect_called_fns_expr(scrutinee, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    collect_called_fns_expr(g, out);
                }
                collect_called_fns_expr(&arm.body, out);
            }
        }
        TypedExprKind::Str(segs) => {
            use crate::analyzer::typed_ast::TypedStringSegment;
            for s in segs {
                if let TypedStringSegment::Interp(e) = s {
                    collect_called_fns_expr(e, out);
                }
            }
        }
        _ => {}
    }
}

/// Remove user-defined globals not referenced anywhere in function bodies (emit path only).
pub fn eliminate_dead_globals(file: TypedFile, user_names: &HashSet<String>) -> TypedFile {
    // Collect all names referenced inside any function body.
    let mut referenced: HashSet<String> = HashSet::new();
    for item in &file.items {
        match item {
            TypedItem::Function(f) => {
                for s in &f.body.stmts {
                    collect_reads_stmt(s, &mut referenced);
                }
            }
            TypedItem::ImplBlock(ib) => {
                for f in &ib.methods {
                    for s in &f.body.stmts {
                        collect_reads_stmt(s, &mut referenced);
                    }
                }
                for h in &ib.hooks {
                    for s in &h.body.stmts {
                        collect_reads_stmt(s, &mut referenced);
                    }
                }
            }
            _ => {}
        }
    }
    let items = file
        .items
        .into_iter()
        .filter(|item| match item {
            TypedItem::Global(g) => {
                let name = &g.name;
                !user_names.contains(name) || referenced.contains(name)
            }
            _ => true,
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
