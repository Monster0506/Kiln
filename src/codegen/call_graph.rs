use crate::analyzer::typed_ast::{
    TypedBlock, TypedExpr, TypedExprKind, TypedFile, TypedItem, TypedStmt,
};
use std::collections::{HashMap, HashSet, VecDeque};

/// Maps function name -> set of directly called function names.
pub struct CallGraph {
    pub edges: HashMap<String, HashSet<String>>,
}

impl Default for CallGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl CallGraph {
    pub fn new() -> Self {
        CallGraph {
            edges: HashMap::new(),
        }
    }

    /// Return the set of callees for a given function (empty if unknown).
    pub fn callees(&self, fn_name: &str) -> &HashSet<String> {
        static EMPTY: std::sync::OnceLock<HashSet<String>> = std::sync::OnceLock::new();
        self.edges
            .get(fn_name)
            .unwrap_or_else(|| EMPTY.get_or_init(HashSet::new))
    }
}

/// Build a CallGraph from a TypedFile. Only top-level functions and impl methods are nodes;
/// the edges are the set of function names called from each body.
pub fn build_call_graph(file: &TypedFile) -> CallGraph {
    let mut graph = CallGraph::new();

    for item in &file.items {
        match item {
            TypedItem::Function(f) => {
                let mut callees = HashSet::new();
                collect_callees_block(&f.body, &mut callees);
                graph.edges.insert(f.name.clone(), callees);
            }
            TypedItem::ImplBlock(ib) => {
                for method in &ib.methods {
                    let qual = format!("{}_{}", ib.for_type, method.name);
                    let mut callees = HashSet::new();
                    collect_callees_block(&method.body, &mut callees);
                    graph.edges.insert(qual, callees);
                }
                for hook in &ib.hooks {
                    use crate::parser::ast::HookName;
                    let hook_suffix = match &hook.name {
                        HookName::Named(n) => n.clone(),
                        HookName::Op(op) => {
                            if hook.params.is_empty() {
                                crate::codegen::names::encode_unary_op(op).to_string()
                            } else {
                                crate::codegen::names::encode_op(op)
                            }
                        }
                    };
                    let qual = format!("{}__hook__{}", ib.for_type, hook_suffix);
                    let mut callees = HashSet::new();
                    collect_callees_block(&hook.body, &mut callees);
                    graph.edges.insert(qual, callees);
                }
            }
            _ => {}
        }
    }

    graph
}

fn collect_callees_block(block: &TypedBlock, out: &mut HashSet<String>) {
    for stmt in &block.stmts {
        collect_callees_stmt(stmt, out);
    }
}

fn collect_callees_stmt(stmt: &TypedStmt, out: &mut HashSet<String>) {
    match stmt {
        TypedStmt::VarDecl { value, .. } => collect_callees_expr(value, out),
        TypedStmt::Assign { target, value, .. } => {
            collect_callees_expr(target, out);
            collect_callees_expr(value, out);
        }
        TypedStmt::CompoundAssign { target, rhs, .. } => {
            collect_callees_expr(target, out);
            collect_callees_expr(rhs, out);
        }
        TypedStmt::Return { value, .. } => {
            if let Some(e) = value {
                collect_callees_expr(e, out);
            }
        }
        TypedStmt::Raise { value, .. } => {
            if let Some(e) = value {
                collect_callees_expr(e, out);
            }
        }
        TypedStmt::Break(_) | TypedStmt::Continue(_) => {}
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (cond, body) in branches {
                collect_callees_expr(cond, out);
                collect_callees_block(body, out);
            }
            if let Some(b) = else_branch {
                collect_callees_block(b, out);
            }
        }
        TypedStmt::While { cond, body, .. } => {
            collect_callees_expr(cond, out);
            collect_callees_block(body, out);
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            collect_callees_block(body, out);
            collect_callees_expr(cond, out);
        }
        TypedStmt::For {
            iterable,
            body,
            iter_ty,
            ..
        } => {
            collect_callees_expr(iterable, out);
            collect_callees_block(body, out);
            // For loops implicitly call iter() on the collection and next() on the
            // iterator -- these are not explicit Call nodes in the typed AST.
            use crate::codegen::mono::type_mono_name;
            out.insert(format!("{}_iter", type_mono_name(&iterable.ty)));
            if let Some(it) = iter_ty {
                out.insert(format!("{}_next", type_mono_name(it)));
            }
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            collect_callees_block(body, out);
            for h in handlers {
                collect_callees_block(&h.body, out);
            }
            if let Some(b) = finally {
                collect_callees_block(b, out);
            }
        }
        TypedStmt::FnDef(f) => {
            collect_callees_block(&f.body, out);
        }
        TypedStmt::Expr(e) => collect_callees_expr(e, out),
    }
}

fn collect_callees_expr(expr: &TypedExpr, out: &mut HashSet<String>) {
    match &expr.kind {
        TypedExprKind::Call {
            callee,
            args,
            fn_name,
            ..
        } => {
            out.insert(fn_name.clone());
            // After monomorphization the callee Ident holds the resolved name
            // (e.g. "println__T__str"), which may differ from fn_name ("println").
            // Record it so the BFS reaches the concrete monomorphized function.
            if let TypedExprKind::Ident(resolved) = &callee.kind {
                out.insert(resolved.clone());
            }
            collect_callees_expr(callee, out);
            for a in args {
                collect_callees_expr(a, out);
            }
        }
        TypedExprKind::MethodCall {
            object,
            method_fn,
            args,
        } => {
            out.insert(method_fn.clone());
            collect_callees_expr(object, out);
            for a in args {
                collect_callees_expr(a, out);
            }
        }
        TypedExprKind::StaticCall { method_fn, args } => {
            out.insert(method_fn.clone());
            for a in args {
                collect_callees_expr(a, out);
            }
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            collect_callees_expr(fat_ptr, out);
            for a in args {
                collect_callees_expr(a, out);
            }
        }
        TypedExprKind::BinOp { left, right, .. } => {
            collect_callees_expr(left, out);
            collect_callees_expr(right, out);
        }
        TypedExprKind::UnOp { operand, .. } => collect_callees_expr(operand, out),
        TypedExprKind::Field { object, .. } => collect_callees_expr(object, out),
        TypedExprKind::Index { object, index } => {
            collect_callees_expr(object, out);
            collect_callees_expr(index, out);
        }
        TypedExprKind::Tuple(exprs) => {
            for e in exprs {
                collect_callees_expr(e, out);
            }
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                collect_callees_expr(e, out);
            }
        }
        TypedExprKind::Unwrap(e) => collect_callees_expr(e, out),
        TypedExprKind::As { expr, .. } => collect_callees_expr(expr, out),
        TypedExprKind::Match { scrutinee, arms } => {
            collect_callees_expr(scrutinee, out);
            for arm in arms {
                if let Some(g) = &arm.guard {
                    collect_callees_expr(g, out);
                }
                collect_callees_expr(&arm.body, out);
            }
        }
        TypedExprKind::Closure { body, .. } => {
            use crate::analyzer::typed_ast::TypedClosureBody;
            match body {
                TypedClosureBody::Expr(e) => collect_callees_expr(e, out),
                TypedClosureBody::Block(b) => collect_callees_block(b, out),
            }
        }
        TypedExprKind::Spawn(e) => collect_callees_expr(e, out),
        TypedExprKind::Ref { expr, .. } => collect_callees_expr(expr, out),
        TypedExprKind::Array(exprs) => {
            for e in exprs {
                collect_callees_expr(e, out);
            }
        }
        TypedExprKind::Gen { body } => collect_callees_block(body, out),
        TypedExprKind::GenSplice(e) => collect_callees_expr(e, out),
        TypedExprKind::Str(segments) => {
            use crate::analyzer::typed_ast::TypedStringSegment;
            for seg in segments {
                if let TypedStringSegment::Interp(e) = seg {
                    collect_callees_expr(e, out);
                }
            }
        }
        // Leaf nodes - no sub-expressions
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Ident(_)
        | TypedExprKind::EnumVariant { .. } => {}
    }
}

/// BFS reachability from entry points. Returns the set of reachable function names.
pub fn reachable_functions(file: &TypedFile, graph: &CallGraph) -> HashSet<String> {
    let mut reachable: HashSet<String> = HashSet::new();
    let mut worklist: VecDeque<String> = VecDeque::new();

    let seed = |name: &str, worklist: &mut VecDeque<String>, reachable: &mut HashSet<String>| {
        if reachable.insert(name.to_string()) {
            worklist.push_back(name.to_string());
        }
    };

    // Seed with explicit entry points: is_entry==true or name=="main"
    for item in &file.items {
        if let TypedItem::Function(f) = item {
            if f.is_entry || f.name == "main" {
                seed(&f.name, &mut worklist, &mut reachable);
            }
        }
    }

    // Also seed from every always-reachable node in the graph (hooks, _to_str, etc.)
    // so that functions they call are also pulled in by the BFS.
    for name in graph.edges.keys() {
        if is_always_reachable(name) {
            seed(name, &mut worklist, &mut reachable);
        }
    }

    // BFS
    while let Some(name) = worklist.pop_front() {
        for callee in graph.callees(&name) {
            if reachable.insert(callee.clone()) {
                worklist.push_back(callee.clone());
            }
        }
    }

    reachable
}

/// Determine whether a FnJob name should be kept even if unreachable.
/// Hook functions, enum _to_str, and __kiln_init_globals are always kept.
pub fn is_always_reachable(name: &str) -> bool {
    name.contains("__hook__")
        || name.ends_with("_to_str")
        || name == "__kiln_init_globals"
        || name == "main"
}

/// Auto-inline candidates: functions with <= 5 stmts, no recursion, no Closure/Spawn.
pub fn find_auto_inline_candidates(typed_file: &TypedFile, graph: &CallGraph) -> HashSet<String> {
    let mut result = HashSet::new();
    for item in &typed_file.items {
        if let TypedItem::Function(f) = item {
            if f.is_inline || f.is_builtin || f.is_declaration || f.is_entry || f.name == "main" {
                continue;
            }
            let stmt_count = f.body.stmts.len();
            if stmt_count > 5 {
                continue;
            }
            // No recursive calls
            let callees = graph.callees(&f.name);
            if callees.contains(&f.name) {
                continue;
            }
            // No Closure or Spawn expressions
            if body_has_closure_or_spawn(&f.body) {
                continue;
            }
            result.insert(f.name.clone());
        }
    }
    result
}

fn body_has_closure_or_spawn(block: &TypedBlock) -> bool {
    block.stmts.iter().any(stmt_has_closure_or_spawn)
}

fn stmt_has_closure_or_spawn(stmt: &TypedStmt) -> bool {
    match stmt {
        TypedStmt::VarDecl { value, .. } => expr_has_closure_or_spawn(value),
        TypedStmt::Assign { target, value, .. } => {
            expr_has_closure_or_spawn(target) || expr_has_closure_or_spawn(value)
        }
        TypedStmt::CompoundAssign { target, rhs, .. } => {
            expr_has_closure_or_spawn(target) || expr_has_closure_or_spawn(rhs)
        }
        TypedStmt::Return { value, .. } => value.as_ref().is_some_and(expr_has_closure_or_spawn),
        TypedStmt::Raise { value, .. } => value.as_ref().is_some_and(expr_has_closure_or_spawn),
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            branches
                .iter()
                .any(|(c, b)| expr_has_closure_or_spawn(c) || body_has_closure_or_spawn(b))
                || else_branch.as_ref().is_some_and(body_has_closure_or_spawn)
        }
        TypedStmt::While { cond, body, .. } => {
            expr_has_closure_or_spawn(cond) || body_has_closure_or_spawn(body)
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            body_has_closure_or_spawn(body) || expr_has_closure_or_spawn(cond)
        }
        TypedStmt::For { iterable, body, .. } => {
            expr_has_closure_or_spawn(iterable) || body_has_closure_or_spawn(body)
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            body_has_closure_or_spawn(body)
                || handlers.iter().any(|h| body_has_closure_or_spawn(&h.body))
                || finally.as_ref().is_some_and(body_has_closure_or_spawn)
        }
        TypedStmt::FnDef(f) => body_has_closure_or_spawn(&f.body),
        TypedStmt::Expr(e) => expr_has_closure_or_spawn(e),
        TypedStmt::Break(_) | TypedStmt::Continue(_) => false,
    }
}

fn expr_has_closure_or_spawn(expr: &TypedExpr) -> bool {
    match &expr.kind {
        TypedExprKind::Closure { .. } | TypedExprKind::Spawn(_) => true,
        TypedExprKind::Call { callee, args, .. } => {
            expr_has_closure_or_spawn(callee) || args.iter().any(expr_has_closure_or_spawn)
        }
        TypedExprKind::MethodCall { object, args, .. } => {
            expr_has_closure_or_spawn(object) || args.iter().any(expr_has_closure_or_spawn)
        }
        TypedExprKind::StaticCall { args, .. } => args.iter().any(expr_has_closure_or_spawn),
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            expr_has_closure_or_spawn(fat_ptr) || args.iter().any(expr_has_closure_or_spawn)
        }
        TypedExprKind::BinOp { left, right, .. } => {
            expr_has_closure_or_spawn(left) || expr_has_closure_or_spawn(right)
        }
        TypedExprKind::UnOp { operand, .. } => expr_has_closure_or_spawn(operand),
        TypedExprKind::Field { object, .. } => expr_has_closure_or_spawn(object),
        TypedExprKind::Index { object, index } => {
            expr_has_closure_or_spawn(object) || expr_has_closure_or_spawn(index)
        }
        TypedExprKind::Tuple(es) => es.iter().any(expr_has_closure_or_spawn),
        TypedExprKind::StructLiteral { fields, .. } => {
            fields.iter().any(|(_, e)| expr_has_closure_or_spawn(e))
        }
        TypedExprKind::Unwrap(e) => expr_has_closure_or_spawn(e),
        TypedExprKind::As { expr, .. } => expr_has_closure_or_spawn(expr),
        TypedExprKind::Match { scrutinee, arms } => {
            expr_has_closure_or_spawn(scrutinee)
                || arms.iter().any(|a| {
                    a.guard.as_ref().is_some_and(expr_has_closure_or_spawn)
                        || expr_has_closure_or_spawn(&a.body)
                })
        }
        TypedExprKind::Ref { expr, .. } => expr_has_closure_or_spawn(expr),
        TypedExprKind::Array(es) => es.iter().any(expr_has_closure_or_spawn),
        TypedExprKind::Gen { body } => body_has_closure_or_spawn(body),
        TypedExprKind::GenSplice(e) => expr_has_closure_or_spawn(e),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::Ty;
    use crate::analyzer::typed_ast::{
        TypedBlock, TypedExpr, TypedExprKind, TypedFnDef, TypedItem, TypedStmt,
    };
    use crate::diagnostics::Span;

    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    fn empty_block() -> TypedBlock {
        TypedBlock {
            stmts: vec![],
            span: s(),
        }
    }

    fn make_fn(name: &str, body: TypedBlock, is_entry: bool) -> TypedFnDef {
        TypedFnDef {
            name: name.into(),
            params: vec![],
            variadic: None,
            return_type: Ty::Void,
            body,
            is_builtin: false,
            is_inline: false,
            is_declaration: false,
            is_entry,
            is_impure: false,
            span: s(),
        }
    }

    fn call_expr(fn_name: &str) -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Call {
                callee: Box::new(TypedExpr {
                    kind: TypedExprKind::Ident(fn_name.into()),
                    ty: Ty::Void,
                    span: s(),
                }),
                args: vec![],
                fn_name: fn_name.into(),
                generic_bounds: vec![],
                generic_params: vec![],
                param_tys: vec![],
            },
            ty: Ty::Void,
            span: s(),
        }
    }

    fn block_calling(fn_name: &str) -> TypedBlock {
        TypedBlock {
            stmts: vec![TypedStmt::Expr(call_expr(fn_name))],
            span: s(),
        }
    }

    #[test]
    fn call_graph_from_typed_file_includes_transitive_callees() {
        // main -> foo -> bar
        let bar_fn = make_fn("bar", empty_block(), false);
        let foo_fn = make_fn("foo", block_calling("bar"), false);
        let main_fn = make_fn("main", block_calling("foo"), true);

        let file = TypedFile {
            items: vec![
                TypedItem::Function(bar_fn),
                TypedItem::Function(foo_fn),
                TypedItem::Function(main_fn),
            ],
            span: s(),
        };

        let graph = build_call_graph(&file);
        let reachable = reachable_functions(&file, &graph);

        assert!(reachable.contains("main"), "main must be reachable");
        assert!(
            reachable.contains("foo"),
            "foo must be reachable (called from main)"
        );
        assert!(
            reachable.contains("bar"),
            "bar must be reachable (called from foo)"
        );
    }

    #[test]
    fn call_graph_excludes_unreachable_function() {
        // main does not call dead_fn
        let dead_fn = make_fn("dead_fn", empty_block(), false);
        let main_fn = make_fn("main", empty_block(), true);

        let file = TypedFile {
            items: vec![TypedItem::Function(dead_fn), TypedItem::Function(main_fn)],
            span: s(),
        };

        let graph = build_call_graph(&file);
        let reachable = reachable_functions(&file, &graph);

        assert!(reachable.contains("main"), "main must be reachable");
        assert!(
            !reachable.contains("dead_fn"),
            "dead_fn must not be reachable"
        );
    }

    #[test]
    fn entry_point_and_all_callees_are_emitted() {
        // entry -> helper
        let helper = make_fn("helper", empty_block(), false);
        let entry = make_fn("entry_fn", block_calling("helper"), true);

        let file = TypedFile {
            items: vec![TypedItem::Function(helper), TypedItem::Function(entry)],
            span: s(),
        };

        let graph = build_call_graph(&file);
        let reachable = reachable_functions(&file, &graph);

        assert!(reachable.contains("entry_fn"), "entry_fn must be reachable");
        assert!(reachable.contains("helper"), "helper must be reachable");
    }

    #[test]
    fn small_leaf_function_is_auto_inlined() {
        // foo has 1 stmt and no recursion
        let foo_body = TypedBlock {
            stmts: vec![TypedStmt::Return {
                value: Some(TypedExpr {
                    kind: TypedExprKind::Int(42),
                    ty: Ty::Int,
                    span: s(),
                }),
                span: s(),
            }],
            span: s(),
        };
        let foo_fn = make_fn("foo", foo_body, false);
        let main_fn = make_fn("main", block_calling("foo"), true);

        let file = TypedFile {
            items: vec![TypedItem::Function(foo_fn), TypedItem::Function(main_fn)],
            span: s(),
        };

        let graph = build_call_graph(&file);
        let candidates = find_auto_inline_candidates(&file, &graph);

        assert!(
            candidates.contains("foo"),
            "foo should be auto-inline candidate"
        );
        assert!(
            !candidates.contains("main"),
            "main should not be auto-inline candidate"
        );
    }

    #[test]
    fn recursive_function_is_not_auto_inlined() {
        // factorial calls itself
        let factorial_body = TypedBlock {
            stmts: vec![TypedStmt::Expr(call_expr("factorial"))],
            span: s(),
        };
        let factorial_fn = make_fn("factorial", factorial_body, false);
        let main_fn = make_fn("main", block_calling("factorial"), true);

        let file = TypedFile {
            items: vec![
                TypedItem::Function(factorial_fn),
                TypedItem::Function(main_fn),
            ],
            span: s(),
        };

        let graph = build_call_graph(&file);
        let candidates = find_auto_inline_candidates(&file, &graph);

        assert!(
            !candidates.contains("factorial"),
            "recursive function should not be auto-inlined"
        );
    }

    #[test]
    fn function_above_size_threshold_is_not_inlined() {
        // big_fn has 6 statements
        let stmts: Vec<TypedStmt> = (0..6)
            .map(|_| {
                TypedStmt::Expr(TypedExpr {
                    kind: TypedExprKind::Int(1),
                    ty: Ty::Int,
                    span: s(),
                })
            })
            .collect();
        let big_body = TypedBlock { stmts, span: s() };
        let big_fn = make_fn("big_fn", big_body, false);
        let main_fn = make_fn("main", block_calling("big_fn"), true);

        let file = TypedFile {
            items: vec![TypedItem::Function(big_fn), TypedItem::Function(main_fn)],
            span: s(),
        };

        let graph = build_call_graph(&file);
        let candidates = find_auto_inline_candidates(&file, &graph);

        assert!(
            !candidates.contains("big_fn"),
            "function above threshold should not be auto-inlined"
        );
    }
}
