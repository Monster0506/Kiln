use crate::analyzer::dce::substitute_name_in_stmt;
use crate::analyzer::ty::Ty;
use crate::analyzer::typed_ast::{
    TypedBlock, TypedExpr, TypedExprKind, TypedFile, TypedFnDef, TypedHookDef, TypedImplBlock,
    TypedItem, TypedStmt,
};
use crate::parser::ast::BinOp;
use std::collections::{HashMap, HashSet};

const UNROLL_LIMIT: i64 = 8;

pub fn unroll_file(file: TypedFile) -> TypedFile {
    TypedFile {
        items: file.items.into_iter().map(unroll_item).collect(),
        ..file
    }
}

fn unroll_item(item: TypedItem) -> TypedItem {
    match item {
        TypedItem::Function(f) => TypedItem::Function(unroll_fn(f)),
        TypedItem::ImplBlock(ib) => TypedItem::ImplBlock(TypedImplBlock {
            methods: ib.methods.into_iter().map(unroll_fn).collect(),
            hooks: ib.hooks.into_iter().map(unroll_hook).collect(),
            ..ib
        }),
        other => other,
    }
}

fn unroll_fn(f: TypedFnDef) -> TypedFnDef {
    TypedFnDef {
        body: unroll_block(f.body, &mut HashMap::new()),
        ..f
    }
}

fn unroll_hook(h: TypedHookDef) -> TypedHookDef {
    TypedHookDef {
        body: unroll_block(h.body, &mut HashMap::new()),
        ..h
    }
}

fn unroll_block(block: TypedBlock, known: &mut HashMap<String, i64>) -> TypedBlock {
    let mut new_stmts: Vec<TypedStmt> = Vec::with_capacity(block.stmts.len());
    for stmt in block.stmts {
        process_stmt(stmt, known, &mut new_stmts);
    }
    TypedBlock {
        stmts: new_stmts,
        span: block.span,
    }
}

fn process_stmt(stmt: TypedStmt, known: &mut HashMap<String, i64>, out: &mut Vec<TypedStmt>) {
    match stmt {
        TypedStmt::VarDecl {
            ref name,
            ref value,
            ..
        } => {
            match &value.kind {
                TypedExprKind::Int(v) => {
                    known.insert(name.clone(), *v);
                }
                _ => {
                    known.remove(name);
                }
            }
            out.push(stmt);
        }
        TypedStmt::Assign {
            ref target,
            ref value,
            ..
        } => {
            if let TypedExprKind::Ident(name) = &target.kind {
                match &value.kind {
                    TypedExprKind::Int(v) => {
                        known.insert(name.clone(), *v);
                    }
                    _ => {
                        known.remove(name);
                    }
                }
            }
            out.push(stmt);
        }
        TypedStmt::CompoundAssign { ref target, .. } => {
            if let TypedExprKind::Ident(name) = &target.kind {
                known.remove(name);
            }
            out.push(stmt);
        }
        TypedStmt::While { cond, body, span } => {
            match try_unroll_while(&cond, &body, known) {
                Some(unrolled) => {
                    // Track the counter's final value (last stmt is the counter assignment).
                    if let Some(counter) = extract_counter_name(&cond) {
                        if let Some(TypedStmt::Assign { ref value, .. }) = unrolled.last() {
                            match &value.kind {
                                TypedExprKind::Int(v) => {
                                    known.insert(counter, *v);
                                }
                                _ => {
                                    known.remove(&counter);
                                }
                            }
                        } else {
                            known.remove(&counter);
                        }
                    }
                    out.extend(unrolled);
                }
                None => {
                    // Can't unroll: evict all loop-assigned vars from known.
                    for name in collect_assigned(&body) {
                        known.remove(&name);
                    }
                    // Recursively process nested loops in the body.
                    let new_body = unroll_block(body, &mut HashMap::new());
                    out.push(TypedStmt::While {
                        cond,
                        body: new_body,
                        span,
                    });
                }
            }
        }
        TypedStmt::DoWhile { body, cond, span } => {
            for name in collect_assigned(&body) {
                known.remove(&name);
            }
            let new_body = unroll_block(body, &mut HashMap::new());
            out.push(TypedStmt::DoWhile {
                body: new_body,
                cond,
                span,
            });
        }
        TypedStmt::For {
            binding,
            binding_ty,
            iterable,
            body,
            iter_ty,
            span,
        } => {
            for name in collect_assigned(&body) {
                known.remove(&name);
            }
            let new_body = unroll_block(body, &mut HashMap::new());
            out.push(TypedStmt::For {
                binding,
                binding_ty,
                iterable,
                body: new_body,
                iter_ty,
                span,
            });
        }
        TypedStmt::If {
            branches,
            else_branch,
            span,
        } => {
            // We can't know which branch runs, so clear all constants after an if.
            known.clear();
            let new_branches = branches
                .into_iter()
                .map(|(cond, body)| (cond, unroll_block(body, &mut HashMap::new())))
                .collect();
            let new_else = else_branch.map(|b| unroll_block(b, &mut HashMap::new()));
            out.push(TypedStmt::If {
                branches: new_branches,
                else_branch: new_else,
                span,
            });
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => {
            known.clear();
            let new_handlers = handlers
                .into_iter()
                .map(|h| crate::analyzer::typed_ast::TypedCatchHandler {
                    body: unroll_block(h.body, &mut HashMap::new()),
                    ..h
                })
                .collect();
            out.push(TypedStmt::TryCatch {
                body: unroll_block(body, &mut HashMap::new()),
                handlers: new_handlers,
                finally: finally.map(|b| unroll_block(b, &mut HashMap::new())),
                span,
            });
        }
        other => {
            out.push(other);
        }
    }
}

// --- Core unroll logic -------------------------------------------------------

fn try_unroll_while(
    cond: &TypedExpr,
    body: &TypedBlock,
    known: &HashMap<String, i64>,
) -> Option<Vec<TypedStmt>> {
    // Parse: counter < limit  or  counter <= limit
    let (counter, limit, inclusive) = extract_lt_condition(cond)?;

    // Initial value must be known from surrounding straight-line code.
    let init = *known.get(&counter)?;

    // Find the unconditional increment at the top level of the body.
    let (incr_idx, step) = find_increment(&body.stmts, &counter)?;

    // Refuse to unroll if any break or continue applies to this loop.
    if has_break_or_continue(body) {
        return None;
    }

    let end = if inclusive { limit + 1 } else { limit };
    if step <= 0 || init >= end {
        return None;
    }

    // Compute trip count and apply the limit.
    let trip_count = (end - init + step - 1) / step;
    if trip_count > UNROLL_LIMIT {
        return None;
    }

    let span = cond.span;
    let mut result: Vec<TypedStmt> = Vec::with_capacity(trip_count as usize * body.stmts.len());

    for k in 0..trip_count {
        let counter_val = init + k * step;
        let counter_expr = TypedExpr {
            kind: TypedExprKind::Int(counter_val),
            ty: Ty::Int,
            span,
        };
        for (idx, stmt) in body.stmts.iter().enumerate() {
            if idx == incr_idx {
                continue;
            }
            let substituted = substitute_name_in_stmt(stmt.clone(), &counter, &counter_expr);
            result.push(substituted);
        }
    }

    // Emit the counter's final value so downstream passes can track it.
    let final_val = init + trip_count * step;
    result.push(TypedStmt::Assign {
        target: TypedExpr {
            kind: TypedExprKind::Ident(counter.clone()),
            ty: Ty::Int,
            span,
        },
        value: TypedExpr {
            kind: TypedExprKind::Int(final_val),
            ty: Ty::Int,
            span,
        },
        span,
    });

    Some(result)
}

// --- Helpers -----------------------------------------------------------------

fn extract_lt_condition(cond: &TypedExpr) -> Option<(String, i64, bool)> {
    if let TypedExprKind::BinOp { op, left, right } = &cond.kind {
        let inclusive = match op {
            BinOp::Lt => false,
            BinOp::LtEq => true,
            _ => return None,
        };
        if let (TypedExprKind::Ident(counter), TypedExprKind::Int(limit)) =
            (&left.kind, &right.kind)
        {
            return Some((counter.clone(), *limit, inclusive));
        }
    }
    None
}

fn extract_counter_name(cond: &TypedExpr) -> Option<String> {
    if let TypedExprKind::BinOp { op, left, .. } = &cond.kind {
        if matches!(op, BinOp::Lt | BinOp::LtEq) {
            if let TypedExprKind::Ident(name) = &left.kind {
                return Some(name.clone());
            }
        }
    }
    None
}

// Find `counter = counter + step` or `counter += step` at the TOP LEVEL of stmts.
// Only positive integer steps are accepted. Returns (statement index, step).
fn find_increment(stmts: &[TypedStmt], counter: &str) -> Option<(usize, i64)> {
    for (idx, stmt) in stmts.iter().enumerate() {
        match stmt {
            TypedStmt::Assign { target, value, .. } => {
                if let TypedExprKind::Ident(name) = &target.kind {
                    if name == counter {
                        if let TypedExprKind::BinOp {
                            op: BinOp::Add,
                            left,
                            right,
                        } = &value.kind
                        {
                            let step = match (&left.kind, &right.kind) {
                                (TypedExprKind::Ident(n), TypedExprKind::Int(s))
                                    if n == counter =>
                                {
                                    *s
                                }
                                (TypedExprKind::Int(s), TypedExprKind::Ident(n))
                                    if n == counter =>
                                {
                                    *s
                                }
                                _ => continue,
                            };
                            if step > 0 {
                                return Some((idx, step));
                            }
                        }
                    }
                }
            }
            TypedStmt::CompoundAssign {
                target, op, rhs, ..
            } => {
                if *op == BinOp::Add {
                    if let TypedExprKind::Ident(name) = &target.kind {
                        if name == counter {
                            if let TypedExprKind::Int(step) = &rhs.kind {
                                if *step > 0 {
                                    return Some((idx, *step));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

// Returns true if any break or continue applies to this loop level
// (does NOT recurse into nested loops, since their break/continue targets them).
fn has_break_or_continue(block: &TypedBlock) -> bool {
    block.stmts.iter().any(stmt_has_break_or_continue)
}

fn stmt_has_break_or_continue(stmt: &TypedStmt) -> bool {
    match stmt {
        TypedStmt::Break(_) | TypedStmt::Continue(_) => true,
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            branches.iter().any(|(_, b)| has_break_or_continue(b))
                || else_branch.as_ref().is_some_and(has_break_or_continue)
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            has_break_or_continue(body)
                || handlers.iter().any(|h| has_break_or_continue(&h.body))
                || finally.as_ref().is_some_and(has_break_or_continue)
        }
        // Nested loops own their own break/continue -- don't recurse into them.
        TypedStmt::While { .. } | TypedStmt::DoWhile { .. } | TypedStmt::For { .. } => false,
        _ => false,
    }
}

fn collect_assigned(block: &TypedBlock) -> HashSet<String> {
    let mut names = HashSet::new();
    collect_assigned_in_block(block, &mut names);
    names
}

fn collect_assigned_in_block(block: &TypedBlock, names: &mut HashSet<String>) {
    for stmt in &block.stmts {
        collect_assigned_in_stmt(stmt, names);
    }
}

fn collect_assigned_in_stmt(stmt: &TypedStmt, names: &mut HashSet<String>) {
    match stmt {
        TypedStmt::Assign { target, .. } | TypedStmt::CompoundAssign { target, .. } => {
            if let TypedExprKind::Ident(name) = &target.kind {
                names.insert(name.clone());
            }
        }
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (_, b) in branches {
                collect_assigned_in_block(b, names);
            }
            if let Some(b) = else_branch {
                collect_assigned_in_block(b, names);
            }
        }
        TypedStmt::While { body, .. }
        | TypedStmt::DoWhile { body, .. }
        | TypedStmt::For { body, .. } => {
            collect_assigned_in_block(body, names);
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            collect_assigned_in_block(body, names);
            for h in handlers {
                collect_assigned_in_block(&h.body, names);
            }
            if let Some(b) = finally {
                collect_assigned_in_block(b, names);
            }
        }
        _ => {}
    }
}
