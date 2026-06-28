use crate::analyzer::typed_ast::{
    TypedExprKind, TypedFile, TypedItem, TypedStmt, TypedStringSegment,
};
use std::collections::{HashMap, HashSet};

/// Write the computed impure set back into the TypedFile, marking transitively impure
/// functions/hooks so subsequent passes and .opt.kn output can see the flag.
pub fn tag_impure_functions(mut file: TypedFile, impure: &HashSet<String>) -> TypedFile {
    for item in &mut file.items {
        match item {
            TypedItem::Function(f) => {
                if impure.contains(&f.name) {
                    f.is_impure = true;
                }
            }
            TypedItem::ImplBlock(ib) => {
                for m in &mut ib.methods {
                    if impure.contains(&m.name) {
                        m.is_impure = true;
                    }
                }
                for h in &mut ib.hooks {
                    let name = hook_key(&h.name);
                    if impure.contains(&name) {
                        h.is_impure = true;
                    }
                }
            }
            _ => {}
        }
    }
    file
}

/// Build the transitive set of impure function names. Seeds from is_impure=true defs,
/// then expands to callers until fixed point.
pub fn build_impure_set(file: &TypedFile) -> HashSet<String> {
    let mut impure: HashSet<String> = HashSet::new();
    let mut fn_bodies: HashMap<String, Vec<TypedStmt>> = HashMap::new();

    for item in &file.items {
        match item {
            TypedItem::Function(f) => {
                if f.is_impure {
                    impure.insert(f.name.clone());
                }
                fn_bodies.insert(f.name.clone(), f.body.stmts.clone());
            }
            TypedItem::ImplBlock(ib) => {
                for m in &ib.methods {
                    if m.is_impure {
                        impure.insert(m.name.clone());
                    }
                    fn_bodies.insert(m.name.clone(), m.body.stmts.clone());
                }
                for h in &ib.hooks {
                    let name = hook_key(&h.name);
                    if h.is_impure {
                        impure.insert(name.clone());
                    }
                    fn_bodies.insert(name, h.body.stmts.clone());
                }
            }
            _ => {}
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        let mut newly: Vec<String> = Vec::new();
        for (name, stmts) in &fn_bodies {
            if impure.contains(name) {
                continue;
            }
            if stmts_touch_impure(stmts, &impure) {
                newly.push(name.clone());
            }
        }
        for name in newly {
            impure.insert(name);
            changed = true;
        }
    }

    impure
}

fn hook_key(name: &crate::parser::ast::HookName) -> String {
    match name {
        crate::parser::ast::HookName::Named(s) => s.clone(),
        crate::parser::ast::HookName::Op(s) => format!("__hook_op_{s}"),
    }
}

fn stmts_touch_impure(stmts: &[TypedStmt], impure: &HashSet<String>) -> bool {
    stmts.iter().any(|s| stmt_touches_impure(s, impure))
}

fn stmt_touches_impure(stmt: &TypedStmt, impure: &HashSet<String>) -> bool {
    match stmt {
        TypedStmt::VarDecl { value, .. } => expr_touches_impure(value, impure),
        TypedStmt::Assign { target, value, .. } => {
            expr_touches_impure(value, impure) || expr_touches_impure(target, impure)
        }
        TypedStmt::CompoundAssign { target, rhs, .. } => {
            expr_touches_impure(rhs, impure) || expr_touches_impure(target, impure)
        }
        TypedStmt::Return { value, .. } => value
            .as_ref()
            .is_some_and(|e| expr_touches_impure(e, impure)),
        TypedStmt::Raise { value, .. } => value
            .as_ref()
            .is_some_and(|e| expr_touches_impure(e, impure)),
        TypedStmt::Expr(e) => expr_touches_impure(e, impure),
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            branches.iter().any(|(c, b)| {
                expr_touches_impure(c, impure) || stmts_touch_impure(&b.stmts, impure)
            }) || else_branch
                .as_ref()
                .is_some_and(|b| stmts_touch_impure(&b.stmts, impure))
        }
        TypedStmt::While { cond, body, .. } => {
            expr_touches_impure(cond, impure) || stmts_touch_impure(&body.stmts, impure)
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            stmts_touch_impure(&body.stmts, impure) || expr_touches_impure(cond, impure)
        }
        TypedStmt::For { iterable, body, .. } => {
            expr_touches_impure(iterable, impure) || stmts_touch_impure(&body.stmts, impure)
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            stmts_touch_impure(&body.stmts, impure)
                || handlers
                    .iter()
                    .any(|h| stmts_touch_impure(&h.body.stmts, impure))
                || finally
                    .as_ref()
                    .is_some_and(|b| stmts_touch_impure(&b.stmts, impure))
        }
        TypedStmt::FnDef(f) => stmts_touch_impure(&f.body.stmts, impure),
        TypedStmt::Break(_) | TypedStmt::Continue(_) => false,
    }
}

fn expr_touches_impure(
    expr: &crate::analyzer::typed_ast::TypedExpr,
    impure: &HashSet<String>,
) -> bool {
    match &expr.kind {
        TypedExprKind::Call {
            fn_name,
            callee,
            args,
            ..
        } => {
            impure.contains(fn_name)
                || expr_touches_impure(callee, impure)
                || args.iter().any(|a| expr_touches_impure(a, impure))
        }
        TypedExprKind::MethodCall {
            object,
            method_fn,
            args,
        } => {
            impure.contains(method_fn)
                || expr_touches_impure(object, impure)
                || args.iter().any(|a| expr_touches_impure(a, impure))
        }
        TypedExprKind::StaticCall { method_fn, args } => {
            impure.contains(method_fn) || args.iter().any(|a| expr_touches_impure(a, impure))
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            let _ = (fat_ptr, args);
            true
        }
        TypedExprKind::Spawn(_) => true,
        TypedExprKind::BinOp { left, right, .. } => {
            expr_touches_impure(left, impure) || expr_touches_impure(right, impure)
        }
        TypedExprKind::UnOp { operand, .. } => expr_touches_impure(operand, impure),
        TypedExprKind::Field { object, .. } => expr_touches_impure(object, impure),
        TypedExprKind::Index { object, index } => {
            expr_touches_impure(object, impure) || expr_touches_impure(index, impure)
        }
        TypedExprKind::Tuple(exprs) | TypedExprKind::Array(exprs) => {
            exprs.iter().any(|e| expr_touches_impure(e, impure))
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            fields.iter().any(|(_, e)| expr_touches_impure(e, impure))
        }
        TypedExprKind::Unwrap(e) | TypedExprKind::GenSplice(e) => expr_touches_impure(e, impure),
        TypedExprKind::As { expr: e, .. } | TypedExprKind::Ref { expr: e, .. } => {
            expr_touches_impure(e, impure)
        }
        TypedExprKind::Match { scrutinee, arms } => {
            expr_touches_impure(scrutinee, impure)
                || arms.iter().any(|a| {
                    a.guard
                        .as_ref()
                        .is_some_and(|g| expr_touches_impure(g, impure))
                        || expr_touches_impure(&a.body, impure)
                })
        }
        TypedExprKind::Str(segs) => segs.iter().any(|s| match s {
            TypedStringSegment::Interp(e) => expr_touches_impure(e, impure),
            TypedStringSegment::Text(_) => false,
        }),
        TypedExprKind::Closure { body, .. } => {
            use crate::analyzer::typed_ast::TypedClosureBody;
            match body {
                TypedClosureBody::Expr(e) => expr_touches_impure(e, impure),
                TypedClosureBody::Block(b) => stmts_touch_impure(&b.stmts, impure),
            }
        }
        TypedExprKind::Gen { body } => stmts_touch_impure(&body.stmts, impure),
        TypedExprKind::Block(stmts) => stmts_touch_impure(stmts, impure),
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Ident(_)
        | TypedExprKind::EnumVariant { .. } => false,
        TypedExprKind::BoundMethod { object, .. } => expr_touches_impure(object, impure),
        TypedExprKind::PrimTypeRef { .. } => false,
    }
}
