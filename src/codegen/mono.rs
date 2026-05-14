use std::collections::{HashMap, HashSet, VecDeque};

use crate::analyzer::infer::type_name_of;
use crate::analyzer::ty::{Ty, TypeId};
use crate::analyzer::typed_ast::{
    TypedBlock, TypedCatchHandler, TypedClosureBody, TypedExpr, TypedExprKind, TypedFile,
    TypedFnDef, TypedHookDef, TypedImplBlock, TypedItem, TypedMatchArm, TypedParam, TypedStmt,
    TypedStringSegment,
};

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn monomorphize(file: TypedFile) -> TypedFile {
    let mut generic_fns: HashMap<String, TypedFnDef> = HashMap::new();
    for item in &file.items {
        if let TypedItem::Function(f) = item {
            if is_generic_fn(f) {
                generic_fns.insert(f.name.clone(), f.clone());
            }
        }
    }

    if generic_fns.is_empty() {
        return file;
    }

    let mut queue: VecDeque<(String, HashMap<String, Ty>)> = VecDeque::new();
    let mut done: HashSet<(String, Vec<(String, String)>)> = HashSet::new();

    for item in &file.items {
        match item {
            TypedItem::Function(f) if !is_generic_fn(f) => {
                seed_block(&f.body, &generic_fns, &mut queue);
            }
            TypedItem::ImplBlock(ib) => {
                for m in &ib.methods {
                    seed_block(&m.body, &generic_fns, &mut queue);
                }
                for h in &ib.hooks {
                    seed_block(&h.body, &generic_fns, &mut queue);
                }
            }
            _ => {}
        }
    }

    let mut new_fns: Vec<TypedFnDef> = Vec::new();
    while let Some((fn_name, subst)) = queue.pop_front() {
        let key = make_done_key(&fn_name, &subst);
        if !done.insert(key) {
            continue;
        }
        let Some(generic_fn) = generic_fns.get(&fn_name).cloned() else {
            continue;
        };
        let specialized = specialize_fn(&generic_fn, &subst, &generic_fns);
        seed_block(&specialized.body, &generic_fns, &mut queue);
        new_fns.push(specialized);
    }

    let empty: HashMap<String, Ty> = HashMap::new();
    let mut new_items: Vec<TypedItem> = Vec::new();
    for item in file.items {
        match item {
            TypedItem::Function(f) if is_generic_fn(&f) => {}
            TypedItem::Function(f) => {
                let body = subst_block(&f.body, &empty, &generic_fns);
                new_items.push(TypedItem::Function(TypedFnDef { body, ..f }));
            }
            TypedItem::ImplBlock(ib) => {
                let methods: Vec<TypedFnDef> = ib
                    .methods
                    .into_iter()
                    .map(|m| {
                        let body = subst_block(&m.body, &empty, &generic_fns);
                        TypedFnDef { body, ..m }
                    })
                    .collect();
                let hooks: Vec<TypedHookDef> = ib
                    .hooks
                    .into_iter()
                    .map(|h| {
                        let body = subst_block(&h.body, &empty, &generic_fns);
                        TypedHookDef { body, ..h }
                    })
                    .collect();
                new_items.push(TypedItem::ImplBlock(TypedImplBlock {
                    methods,
                    hooks,
                    ..ib
                }));
            }
            other => new_items.push(other),
        }
    }
    for f in new_fns {
        new_items.push(TypedItem::Function(f));
    }

    TypedFile {
        items: new_items,
        span: file.span,
    }
}

// ---------------------------------------------------------------------------
// Generic detection
// ---------------------------------------------------------------------------

fn is_generic_fn(f: &TypedFnDef) -> bool {
//    if f.is_builtin {
//        return false;
//    }
    f.params.iter().any(|p| contains_type_param(&p.ty))
}

fn contains_type_param(ty: &Ty) -> bool {
    match ty {
        Ty::Named(id, _) => *id == TypeId(0),
        Ty::Vec(t) | Ty::Set(t) | Ty::Option(t) | Ty::Shared(t) => contains_type_param(t),
        Ty::Ref(t, _) => contains_type_param(t),
        Ty::Map(k, v) => contains_type_param(k) || contains_type_param(v),
        Ty::Callable(ps, r) => ps.iter().any(contains_type_param) || contains_type_param(r),
        Ty::Tuple(ts) => ts.iter().any(contains_type_param),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Naming helpers
// ---------------------------------------------------------------------------

pub fn type_mono_name(ty: &Ty) -> String {
    match ty {
        Ty::Int => "int".into(),
        Ty::Float => "float".into(),
        Ty::Bool => "bool".into(),
        Ty::Str => "str".into(),
        Ty::Void => "void".into(),
        Ty::Named(_, name) | Ty::Interface(_, name) => name.clone(),
        Ty::Vec(t) => format!("Vec_{}", type_mono_name(t)),
        Ty::Set(t) => format!("Set_{}", type_mono_name(t)),
        Ty::Option(t) => format!("Option_{}", type_mono_name(t)),
        Ty::Shared(t) => format!("Shared_{}", type_mono_name(t)),
        Ty::Map(k, v) => format!("Map_{}_{}", type_mono_name(k), type_mono_name(v)),
        Ty::Tuple(ts) => {
            let inner: Vec<String> = ts.iter().map(type_mono_name).collect();
            format!("Tuple_{}", inner.join("_"))
        }
        _ => "unknown".into(),
    }
}

pub fn specialized_name(base: &str, subst: &HashMap<String, Ty>) -> String {
    let mut pairs: Vec<(&String, &Ty)> = subst.iter().collect();
    pairs.sort_by_key(|(k, _)| k.as_str());
    let parts: Vec<String> = pairs
        .into_iter()
        .flat_map(|(k, v)| [k.clone(), type_mono_name(v)])
        .collect();
    if parts.is_empty() {
        base.to_string()
    } else {
        format!("{}__{}", base, parts.join("__"))
    }
}

fn make_done_key(fn_name: &str, subst: &HashMap<String, Ty>) -> (String, Vec<(String, String)>) {
    let mut pairs: Vec<(String, String)> = subst
        .iter()
        .map(|(k, v)| (k.clone(), type_mono_name(v)))
        .collect();
    pairs.sort();
    (fn_name.to_string(), pairs)
}

// ---------------------------------------------------------------------------
// Seeding: scan for calls to generic functions
// ---------------------------------------------------------------------------

fn seed_block(
    block: &TypedBlock,
    generic_fns: &HashMap<String, TypedFnDef>,
    queue: &mut VecDeque<(String, HashMap<String, Ty>)>,
) {
    for stmt in &block.stmts {
        seed_stmt(stmt, generic_fns, queue);
    }
}

fn seed_stmt(
    stmt: &TypedStmt,
    generic_fns: &HashMap<String, TypedFnDef>,
    queue: &mut VecDeque<(String, HashMap<String, Ty>)>,
) {
    match stmt {
        TypedStmt::Expr(e) => seed_expr(e, generic_fns, queue),
        TypedStmt::Return { value: Some(e), .. } | TypedStmt::Raise { value: Some(e), .. } => {
            seed_expr(e, generic_fns, queue);
        }
        TypedStmt::Return { value: None, .. }
        | TypedStmt::Raise { value: None, .. }
        | TypedStmt::Break(_)
        | TypedStmt::Continue(_) => {}
        TypedStmt::VarDecl { value, .. } => seed_expr(value, generic_fns, queue),
        TypedStmt::Assign { target, value, .. } => {
            seed_expr(target, generic_fns, queue);
            seed_expr(value, generic_fns, queue);
        }
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (cond, body) in branches {
                seed_expr(cond, generic_fns, queue);
                seed_block(body, generic_fns, queue);
            }
            if let Some(eb) = else_branch {
                seed_block(eb, generic_fns, queue);
            }
        }
        TypedStmt::While { cond, body, .. } => {
            seed_expr(cond, generic_fns, queue);
            seed_block(body, generic_fns, queue);
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            seed_block(body, generic_fns, queue);
            seed_expr(cond, generic_fns, queue);
        }
        TypedStmt::For { iterable, body, .. } => {
            seed_expr(iterable, generic_fns, queue);
            seed_block(body, generic_fns, queue);
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            seed_block(body, generic_fns, queue);
            for h in handlers {
                seed_block(&h.body, generic_fns, queue);
            }
            if let Some(f) = finally {
                seed_block(f, generic_fns, queue);
            }
        }
        TypedStmt::FnDef(f) => seed_block(&f.body, generic_fns, queue),
    }
}

fn seed_expr(
    expr: &TypedExpr,
    generic_fns: &HashMap<String, TypedFnDef>,
    queue: &mut VecDeque<(String, HashMap<String, Ty>)>,
) {
    match &expr.kind {
        TypedExprKind::Call {
            callee,
            args,
            generic_params,
            ..
        } => {
            if !generic_params.is_empty() {
                if let TypedExprKind::Ident(name) = &callee.kind {
                    if let Some(gf) = generic_fns.get(name.as_str()) {
                        let arg_tys: Vec<Ty> = args.iter().map(|a| a.ty.clone()).collect();
                        let subst = unify_params(gf, &arg_tys);
                        if !subst.is_empty() {
                            queue.push_back((name.clone(), subst));
                        }
                    }
                }
            }
            seed_expr(callee, generic_fns, queue);
            for a in args {
                seed_expr(a, generic_fns, queue);
            }
        }
        TypedExprKind::BinOp { left, right, .. } => {
            seed_expr(left, generic_fns, queue);
            seed_expr(right, generic_fns, queue);
        }
        TypedExprKind::UnOp { operand, .. } => seed_expr(operand, generic_fns, queue),
        TypedExprKind::MethodCall { object, args, .. } => {
            seed_expr(object, generic_fns, queue);
            for a in args {
                seed_expr(a, generic_fns, queue);
            }
        }
        TypedExprKind::StaticCall { args, .. } => {
            for a in args {
                seed_expr(a, generic_fns, queue);
            }
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            seed_expr(fat_ptr, generic_fns, queue);
            for a in args {
                seed_expr(a, generic_fns, queue);
            }
        }
        TypedExprKind::Field { object, .. } => seed_expr(object, generic_fns, queue),
        TypedExprKind::Index { object, index } => {
            seed_expr(object, generic_fns, queue);
            seed_expr(index, generic_fns, queue);
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                seed_expr(e, generic_fns, queue);
            }
        }
        TypedExprKind::Tuple(elems) => {
            for e in elems {
                seed_expr(e, generic_fns, queue);
            }
        }
        TypedExprKind::Match { scrutinee, arms } => {
            seed_expr(scrutinee, generic_fns, queue);
            for arm in arms {
                seed_expr(&arm.body, generic_fns, queue);
                if let Some(g) = &arm.guard {
                    seed_expr(g, generic_fns, queue);
                }
            }
        }
        TypedExprKind::Closure { body, .. } => match body {
            TypedClosureBody::Expr(e) => seed_expr(e, generic_fns, queue),
            TypedClosureBody::Block(b) => seed_block(b, generic_fns, queue),
        },
        TypedExprKind::Unwrap(inner)
        | TypedExprKind::Spawn(inner)
        | TypedExprKind::GenSplice(inner) => seed_expr(inner, generic_fns, queue),
        TypedExprKind::As { expr: e, .. } | TypedExprKind::Ref { expr: e, .. } => {
            seed_expr(e, generic_fns, queue)
        }
        TypedExprKind::Gen { body } => seed_block(body, generic_fns, queue),
        TypedExprKind::Str(segs) => {
            for seg in segs {
                if let TypedStringSegment::Interp(e) = seg {
                    seed_expr(e, generic_fns, queue);
                }
            }
        }
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Ident(_) => {}
    }
}

// ---------------------------------------------------------------------------
// Unification
// ---------------------------------------------------------------------------

fn unify_params(fn_def: &TypedFnDef, arg_tys: &[Ty]) -> HashMap<String, Ty> {
    let mut subst = HashMap::new();
    for (i, param) in fn_def.params.iter().enumerate() {
        if let Some(arg_ty) = arg_tys.get(i) {
            unify_ty(&param.ty, arg_ty, &mut subst);
        }
    }
    subst
}

fn unify_ty(pat: &Ty, concrete: &Ty, subst: &mut HashMap<String, Ty>) {
    match (pat, concrete) {
        (Ty::Named(id, name), _) if *id == TypeId(0) => {
            subst
                .entry(name.clone())
                .or_insert_with(|| concrete.clone());
        }
        (Ty::Vec(p), Ty::Vec(c))
        | (Ty::Set(p), Ty::Set(c))
        | (Ty::Option(p), Ty::Option(c))
        | (Ty::Shared(p), Ty::Shared(c)) => unify_ty(p, c, subst),
        (Ty::Map(kp, vp), Ty::Map(kc, vc)) => {
            unify_ty(kp, kc, subst);
            unify_ty(vp, vc, subst);
        }
        (Ty::Tuple(ps), Ty::Tuple(cs)) => {
            for (p, c) in ps.iter().zip(cs.iter()) {
                unify_ty(p, c, subst);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Specialization
// ---------------------------------------------------------------------------

fn specialize_fn(
    f: &TypedFnDef,
    subst: &HashMap<String, Ty>,
    generic_fns: &HashMap<String, TypedFnDef>,
) -> TypedFnDef {
    let name = specialized_name(&f.name, subst);
    let params: Vec<TypedParam> = f
        .params
        .iter()
        .map(|p| TypedParam {
            name: p.name.clone(),
            ty: subst_ty(&p.ty, subst),
            span: p.span,
        })
        .collect();
    let return_type = subst_ty(&f.return_type, subst);
    let body = subst_block(&f.body, subst, generic_fns);
    TypedFnDef {
        name,
        params,
        variadic: f.variadic.clone(),
        return_type,
        body,
        span: f.span,
    }
}

// ---------------------------------------------------------------------------
// Type substitution
// ---------------------------------------------------------------------------

fn subst_ty(ty: &Ty, subst: &HashMap<String, Ty>) -> Ty {
    match ty {
        Ty::Named(id, name) if *id == TypeId(0) => subst
            .get(name.as_str())
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        Ty::Vec(t) => Ty::Vec(Box::new(subst_ty(t, subst))),
        Ty::Set(t) => Ty::Set(Box::new(subst_ty(t, subst))),
        Ty::Option(t) => Ty::Option(Box::new(subst_ty(t, subst))),
        Ty::Shared(t) => Ty::Shared(Box::new(subst_ty(t, subst))),
        Ty::Ref(t, m) => Ty::Ref(Box::new(subst_ty(t, subst)), *m),
        Ty::Map(k, v) => Ty::Map(Box::new(subst_ty(k, subst)), Box::new(subst_ty(v, subst))),
        Ty::Callable(ps, r) => Ty::Callable(
            ps.iter().map(|p| subst_ty(p, subst)).collect(),
            Box::new(subst_ty(r, subst)),
        ),
        Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| subst_ty(t, subst)).collect()),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// AST traversal: substitute types and rewrite call sites
// ---------------------------------------------------------------------------

fn subst_block(
    block: &TypedBlock,
    subst: &HashMap<String, Ty>,
    generic_fns: &HashMap<String, TypedFnDef>,
) -> TypedBlock {
    TypedBlock {
        stmts: block
            .stmts
            .iter()
            .map(|s| subst_stmt(s, subst, generic_fns))
            .collect(),
        span: block.span,
    }
}

fn subst_stmt(
    stmt: &TypedStmt,
    subst: &HashMap<String, Ty>,
    generic_fns: &HashMap<String, TypedFnDef>,
) -> TypedStmt {
    match stmt {
        TypedStmt::Expr(e) => TypedStmt::Expr(subst_expr(e, subst, generic_fns)),
        TypedStmt::Return { value, span } => TypedStmt::Return {
            value: value.as_ref().map(|v| subst_expr(v, subst, generic_fns)),
            span: *span,
        },
        TypedStmt::Raise { value, span } => TypedStmt::Raise {
            value: value.as_ref().map(|v| subst_expr(v, subst, generic_fns)),
            span: *span,
        },
        TypedStmt::VarDecl {
            name,
            ty,
            value,
            mutable,
            span,
        } => TypedStmt::VarDecl {
            name: name.clone(),
            ty: subst_ty(ty, subst),
            value: subst_expr(value, subst, generic_fns),
            mutable: *mutable,
            span: *span,
        },
        TypedStmt::Assign {
            target,
            value,
            span,
        } => TypedStmt::Assign {
            target: subst_expr(target, subst, generic_fns),
            value: subst_expr(value, subst, generic_fns),
            span: *span,
        },
        TypedStmt::If {
            branches,
            else_branch,
            span,
        } => TypedStmt::If {
            branches: branches
                .iter()
                .map(|(c, b)| {
                    (
                        subst_expr(c, subst, generic_fns),
                        subst_block(b, subst, generic_fns),
                    )
                })
                .collect(),
            else_branch: else_branch
                .as_ref()
                .map(|b| subst_block(b, subst, generic_fns)),
            span: *span,
        },
        TypedStmt::While { cond, body, span } => TypedStmt::While {
            cond: subst_expr(cond, subst, generic_fns),
            body: subst_block(body, subst, generic_fns),
            span: *span,
        },
        TypedStmt::DoWhile { body, cond, span } => TypedStmt::DoWhile {
            body: subst_block(body, subst, generic_fns),
            cond: subst_expr(cond, subst, generic_fns),
            span: *span,
        },
        TypedStmt::For {
            binding,
            binding_ty,
            iterable,
            body,
            span,
        } => TypedStmt::For {
            binding: binding.clone(),
            binding_ty: subst_ty(binding_ty, subst),
            iterable: subst_expr(iterable, subst, generic_fns),
            body: subst_block(body, subst, generic_fns),
            span: *span,
        },
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => TypedStmt::TryCatch {
            body: subst_block(body, subst, generic_fns),
            handlers: handlers
                .iter()
                .map(|h| TypedCatchHandler {
                    ty: subst_ty(&h.ty, subst),
                    binding: h.binding.clone(),
                    body: subst_block(&h.body, subst, generic_fns),
                    span: h.span,
                })
                .collect(),
            finally: finally.as_ref().map(|b| subst_block(b, subst, generic_fns)),
            span: *span,
        },
        TypedStmt::FnDef(f) => TypedStmt::FnDef(TypedFnDef {
            name: f.name.clone(),
            params: f
                .params
                .iter()
                .map(|p| TypedParam {
                    name: p.name.clone(),
                    ty: subst_ty(&p.ty, subst),
                    span: p.span,
                })
                .collect(),
            variadic: f.variadic.clone(),
            return_type: subst_ty(&f.return_type, subst),
            body: subst_block(&f.body, subst, generic_fns),
            span: f.span,
        }),
        TypedStmt::Break(s) => TypedStmt::Break(*s),
        TypedStmt::Continue(s) => TypedStmt::Continue(*s),
    }
}

fn subst_expr(
    expr: &TypedExpr,
    subst: &HashMap<String, Ty>,
    generic_fns: &HashMap<String, TypedFnDef>,
) -> TypedExpr {
    let ty = subst_ty(&expr.ty, subst);
    let kind = match &expr.kind {
        TypedExprKind::Call {
            callee,
            args,
            generic_params,
            generic_bounds,
        } => {
            let new_args: Vec<TypedExpr> = args
                .iter()
                .map(|a| subst_expr(a, subst, generic_fns))
                .collect();
            if !generic_params.is_empty() {
                if let TypedExprKind::Ident(name) = &callee.kind {
                    if let Some(gf) = generic_fns.get(name.as_str()) {
                        let arg_tys: Vec<Ty> = new_args.iter().map(|a| a.ty.clone()).collect();
                        let call_subst = unify_params(gf, &arg_tys);
                        let spec_name = specialized_name(name, &call_subst);
                        let new_callee = TypedExpr {
                            kind: TypedExprKind::Ident(spec_name),
                            ty: subst_ty(&callee.ty, subst),
                            span: callee.span,
                        };
                        return TypedExpr {
                            kind: TypedExprKind::Call {
                                callee: Box::new(new_callee),
                                args: new_args,
                                generic_bounds: vec![],
                                generic_params: vec![],
                            },
                            ty,
                            span: expr.span,
                        };
                    }
                }
            }
            let new_callee = subst_expr(callee, subst, generic_fns);
            TypedExprKind::Call {
                callee: Box::new(new_callee),
                args: new_args,
                generic_bounds: generic_bounds.clone(),
                generic_params: generic_params.clone(),
            }
        }

        TypedExprKind::MethodCall {
            object,
            method_fn,
            args,
        } => {
            let new_obj = subst_expr(object, subst, generic_fns);
            let new_args: Vec<TypedExpr> = args
                .iter()
                .map(|a| subst_expr(a, subst, generic_fns))
                .collect();
            let new_method_fn = rewrite_method_fn(method_fn, subst);
            TypedExprKind::MethodCall {
                object: Box::new(new_obj),
                method_fn: new_method_fn,
                args: new_args,
            }
        }

        TypedExprKind::StaticCall { method_fn, args } => {
            let new_args: Vec<TypedExpr> = args
                .iter()
                .map(|a| subst_expr(a, subst, generic_fns))
                .collect();
            TypedExprKind::StaticCall {
                method_fn: method_fn.clone(),
                args: new_args,
            }
        }

        TypedExprKind::IndirectCall { fat_ptr, args } => {
            let new_fp = subst_expr(fat_ptr, subst, generic_fns);
            let new_args: Vec<TypedExpr> = args
                .iter()
                .map(|a| subst_expr(a, subst, generic_fns))
                .collect();
            TypedExprKind::IndirectCall {
                fat_ptr: Box::new(new_fp),
                args: new_args,
            }
        }

        TypedExprKind::BinOp { op, left, right } => TypedExprKind::BinOp {
            op: op.clone(),
            left: Box::new(subst_expr(left, subst, generic_fns)),
            right: Box::new(subst_expr(right, subst, generic_fns)),
        },

        TypedExprKind::UnOp { op, operand } => TypedExprKind::UnOp {
            op: op.clone(),
            operand: Box::new(subst_expr(operand, subst, generic_fns)),
        },

        TypedExprKind::Field { object, field } => TypedExprKind::Field {
            object: Box::new(subst_expr(object, subst, generic_fns)),
            field: field.clone(),
        },

        TypedExprKind::Index { object, index } => TypedExprKind::Index {
            object: Box::new(subst_expr(object, subst, generic_fns)),
            index: Box::new(subst_expr(index, subst, generic_fns)),
        },

        TypedExprKind::StructLiteral { ty_name, fields } => TypedExprKind::StructLiteral {
            ty_name: ty_name.clone(),
            fields: fields
                .iter()
                .map(|(n, e)| (n.clone(), subst_expr(e, subst, generic_fns)))
                .collect(),
        },

        TypedExprKind::Tuple(elems) => TypedExprKind::Tuple(
            elems
                .iter()
                .map(|e| subst_expr(e, subst, generic_fns))
                .collect(),
        ),

        TypedExprKind::Match { scrutinee, arms } => TypedExprKind::Match {
            scrutinee: Box::new(subst_expr(scrutinee, subst, generic_fns)),
            arms: arms
                .iter()
                .map(|arm| TypedMatchArm {
                    pattern: arm.pattern.clone(),
                    guard: arm
                        .guard
                        .as_ref()
                        .map(|g| subst_expr(g, subst, generic_fns)),
                    body: subst_expr(&arm.body, subst, generic_fns),
                    span: arm.span,
                })
                .collect(),
        },

        TypedExprKind::Closure { params, body } => TypedExprKind::Closure {
            params: params
                .iter()
                .map(|p| TypedParam {
                    name: p.name.clone(),
                    ty: subst_ty(&p.ty, subst),
                    span: p.span,
                })
                .collect(),
            body: match body {
                TypedClosureBody::Expr(e) => {
                    TypedClosureBody::Expr(Box::new(subst_expr(e, subst, generic_fns)))
                }
                TypedClosureBody::Block(b) => {
                    TypedClosureBody::Block(subst_block(b, subst, generic_fns))
                }
            },
        },

        TypedExprKind::Unwrap(inner) => {
            TypedExprKind::Unwrap(Box::new(subst_expr(inner, subst, generic_fns)))
        }
        TypedExprKind::As { expr: e, ty: t } => TypedExprKind::As {
            expr: Box::new(subst_expr(e, subst, generic_fns)),
            ty: subst_ty(t, subst),
        },
        TypedExprKind::Spawn(inner) => {
            TypedExprKind::Spawn(Box::new(subst_expr(inner, subst, generic_fns)))
        }
        TypedExprKind::Ref { mutable, expr: e } => TypedExprKind::Ref {
            mutable: *mutable,
            expr: Box::new(subst_expr(e, subst, generic_fns)),
        },
        TypedExprKind::Gen { body } => TypedExprKind::Gen {
            body: subst_block(body, subst, generic_fns),
        },
        TypedExprKind::GenSplice(inner) => {
            TypedExprKind::GenSplice(Box::new(subst_expr(inner, subst, generic_fns)))
        }
        TypedExprKind::Str(segs) => TypedExprKind::Str(
            segs.iter()
                .map(|seg| match seg {
                    TypedStringSegment::Text(t) => TypedStringSegment::Text(t.clone()),
                    TypedStringSegment::Interp(e) => {
                        TypedStringSegment::Interp(subst_expr(e, subst, generic_fns))
                    }
                })
                .collect(),
        ),
        other => other.clone(),
    };
    TypedExpr {
        kind,
        ty,
        span: expr.span,
    }
}

// ---------------------------------------------------------------------------
// Method function name rewriting
// ---------------------------------------------------------------------------

fn rewrite_method_fn(method_fn: &str, subst: &HashMap<String, Ty>) -> String {
    for (param, concrete_ty) in subst {
        let prefix = format!("{}_", param);
        if method_fn.starts_with(prefix.as_str()) {
            let suffix = &method_fn[prefix.len()..];
            return match type_name_of(concrete_ty) {
                Some(name) => format!("{}_{}", name, suffix),
                None => suffix.to_string(),
            };
        }
    }
    method_fn.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::{Ty, TypeId};

    #[test]
    fn type_mono_name_primitives() {
        assert_eq!(type_mono_name(&Ty::Int), "int");
        assert_eq!(type_mono_name(&Ty::Str), "str");
        assert_eq!(type_mono_name(&Ty::Vec(Box::new(Ty::Int))), "Vec_int");
    }

    #[test]
    fn specialized_name_single_param() {
        let mut subst = HashMap::new();
        subst.insert("T".to_string(), Ty::Int);
        assert_eq!(specialized_name("identity", &subst), "identity__T__int");
    }

    #[test]
    fn rewrite_method_fn_named() {
        let mut subst = HashMap::new();
        subst.insert("T".to_string(), Ty::Named(TypeId(1), "Circle".into()));
        assert_eq!(rewrite_method_fn("T_draw", &subst), "Circle_draw");
    }

    #[test]
    fn rewrite_method_fn_no_match() {
        let subst: HashMap<String, Ty> = HashMap::new();
        assert_eq!(rewrite_method_fn("Vec_add", &subst), "Vec_add");
    }

    #[test]
    fn contains_type_param_named_zero() {
        assert!(contains_type_param(&Ty::Named(TypeId(0), "T".into())));
        assert!(!contains_type_param(&Ty::Named(TypeId(1), "Circle".into())));
        assert!(contains_type_param(&Ty::Vec(Box::new(Ty::Named(
            TypeId(0),
            "T".into()
        )))));
    }
}
