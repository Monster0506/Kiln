use std::collections::{HashMap, HashSet, VecDeque};

use crate::analyzer::infer::type_name_of;
use crate::analyzer::ty::{Ty, TypeRegistry};
use crate::analyzer::typed_ast::{
    TypedBlock, TypedCatchHandler, TypedClosureBody, TypedExpr, TypedExprKind, TypedFile,
    TypedFnDef, TypedHookDef, TypedImplBlock, TypedItem, TypedMatchArm, TypedParam, TypedStmt,
    TypedStringSegment,
};
use crate::parser::ast::HookName;

// (for_type base name, hook_suffix, concrete receiver type)
// e.g. ("Vec", "to_str", Vec(Named("Item")))
type ImplHookReq = (String, String, Ty);

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn monomorphize(file: TypedFile, registry: &TypeRegistry) -> TypedFile {
    let mut generic_fns: HashMap<String, TypedFnDef> = HashMap::new();
    for item in &file.items {
        if let TypedItem::Function(f) = item {
            if is_generic_fn(f) {
                generic_fns.insert(f.name.clone(), f.clone());
            }
        }
    }

    // Collect generic impl hooks from parameterized types (e.g. impl[T] Iface for Vec[T]).
    // Key: (for_type base name, hook_suffix). Value: (hook, for_type_ty template).
    let mut generic_impl_hooks: HashMap<(String, String), (TypedHookDef, Ty)> = HashMap::new();
    for item in &file.items {
        if let TypedItem::ImplBlock(ib) = item {
            if contains_type_param(&ib.for_type_ty) {
                for hook in &ib.hooks {
                    let suffix = hook_suffix(hook);
                    generic_impl_hooks.insert(
                        (ib.for_type.clone(), suffix),
                        (hook.clone(), ib.for_type_ty.clone()),
                    );
                }
            }
        }
    }

    if generic_fns.is_empty() && generic_impl_hooks.is_empty() {
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
    let mut impl_reqs: Vec<ImplHookReq> = Vec::new();

    while let Some((fn_name, subst)) = queue.pop_front() {
        let key = make_done_key(&fn_name, &subst);
        if !done.insert(key) {
            continue;
        }
        let Some(generic_fn) = generic_fns.get(&fn_name).cloned() else {
            continue;
        };
        let specialized = specialize_fn(
            &generic_fn,
            &subst,
            registry,
            &generic_fns,
            &generic_impl_hooks,
            &mut impl_reqs,
        );
        seed_block(&specialized.body, &generic_fns, &mut queue);
        new_fns.push(specialized);
    }

    // Process impl hook specializations (e.g. Vec_Item_to_str from Vec[T]_to_str with T=Item).
    // Iterate to handle transitive specializations (e.g. Vec[Vec[int]]).
    let mut impl_done: HashSet<String> = HashSet::new();
    loop {
        let pending: Vec<ImplHookReq> = std::mem::take(&mut impl_reqs);
        if pending.is_empty() {
            break;
        }
        for (base, method_suffix, concrete_ty) in pending {
            let fn_name = format!("{}_{}", type_mono_name(&concrete_ty), method_suffix);
            if impl_done.contains(&fn_name) {
                continue;
            }
            impl_done.insert(fn_name.clone());
            if let Some((hook, for_type_ty)) = generic_impl_hooks.get(&(base, method_suffix)) {
                let inner_subst = derive_impl_subst(for_type_ty, &concrete_ty);
                let fn_def = specialize_hook_as_fn(
                    &fn_name,
                    hook,
                    &inner_subst,
                    &concrete_ty,
                    &generic_fns,
                    &generic_impl_hooks,
                    &mut impl_reqs,
                );
                new_fns.push(fn_def);
            }
        }
    }

    let empty: HashMap<String, Ty> = HashMap::new();
    let mut new_items: Vec<TypedItem> = Vec::new();
    for item in file.items {
        match item {
            TypedItem::Function(f) if is_generic_fn(&f) => {}
            TypedItem::Function(f) => {
                let body = subst_block(
                    &f.body,
                    &empty,
                    &generic_fns,
                    &generic_impl_hooks,
                    &mut impl_reqs,
                );
                new_items.push(TypedItem::Function(TypedFnDef { body, ..f }));
            }
            TypedItem::ImplBlock(ib) => {
                let methods: Vec<TypedFnDef> = ib
                    .methods
                    .into_iter()
                    .map(|m| {
                        let body = subst_block(
                            &m.body,
                            &empty,
                            &generic_fns,
                            &generic_impl_hooks,
                            &mut impl_reqs,
                        );
                        TypedFnDef { body, ..m }
                    })
                    .collect();
                let hooks: Vec<TypedHookDef> = ib
                    .hooks
                    .into_iter()
                    .map(|h| {
                        let body = subst_block(
                            &h.body,
                            &empty,
                            &generic_fns,
                            &generic_impl_hooks,
                            &mut impl_reqs,
                        );
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

    // Process any impl_reqs added by For-statement dispatch (iter/next hooks).
    let mut extra_fns: Vec<TypedFnDef> = Vec::new();
    loop {
        let pending: Vec<ImplHookReq> = std::mem::take(&mut impl_reqs);
        if pending.is_empty() {
            break;
        }
        for (base, method_suffix, concrete_ty) in pending {
            let fn_name = format!("{}_{}", type_mono_name(&concrete_ty), method_suffix);
            if impl_done.contains(&fn_name) {
                continue;
            }
            impl_done.insert(fn_name.clone());
            if let Some((hook, for_type_ty)) =
                generic_impl_hooks.get(&(base.clone(), method_suffix.clone()))
            {
                let inner_subst = derive_impl_subst(for_type_ty, &concrete_ty);
                let fn_def = specialize_hook_as_fn(
                    &fn_name,
                    hook,
                    &inner_subst,
                    &concrete_ty,
                    &generic_fns,
                    &generic_impl_hooks,
                    &mut impl_reqs,
                );
                extra_fns.push(fn_def);
            }
        }
    }
    for f in extra_fns {
        new_items.push(TypedItem::Function(f));
    }

    TypedFile {
        items: new_items,
        span: file.span,
    }
}

// ---------------------------------------------------------------------------
// Generic impl hook helpers
// ---------------------------------------------------------------------------

fn hook_suffix(hook: &TypedHookDef) -> String {
    match &hook.name {
        HookName::Named(n) => n.clone(),
        HookName::Op(op) => {
            if hook.params.is_empty() {
                crate::codegen::names::encode_unary_op(op).to_string()
            } else {
                crate::codegen::names::encode_op(op)
            }
        }
    }
}

/// Derive the generic-param substitution by unifying the impl's generic self
/// type (e.g. `Vec(GenericParam("T"))`) against the concrete receiver type
/// (e.g. `Vec(Named("Item"))`).  Returns an empty map when the impl is not
/// generic or the types don't match.
fn derive_impl_subst(for_type_ty: &Ty, concrete_ty: &Ty) -> HashMap<String, Ty> {
    let mut subst = HashMap::new();
    unify_ty(for_type_ty, concrete_ty, &mut subst);
    subst
}

fn specialize_hook_as_fn(
    fn_name: &str,
    hook: &TypedHookDef,
    subst: &HashMap<String, Ty>,
    concrete_self_ty: &Ty,
    generic_fns: &HashMap<String, TypedFnDef>,
    generic_impl_hooks: &HashMap<(String, String), (TypedHookDef, Ty)>,
    impl_reqs: &mut Vec<ImplHookReq>,
) -> TypedFnDef {
    let mut params = if hook.is_static {
        vec![]
    } else {
        vec![TypedParam {
            name: "__self".to_string(),
            ty: concrete_self_ty.clone(),
            mutable: false,
            span: hook.span,
        }]
    };
    for p in &hook.params {
        params.push(TypedParam {
            name: p.name.clone(),
            ty: subst_ty(&p.ty, subst),
            mutable: p.mutable,
            span: p.span,
        });
    }
    let body = subst_block(
        &hook.body,
        subst,
        generic_fns,
        generic_impl_hooks,
        impl_reqs,
    );
    TypedFnDef {
        name: fn_name.to_string(),
        params,
        variadic: None,
        return_type: subst_ty(&hook.return_type, subst),
        body,
        is_builtin: false,
        is_inline: false,
        is_declaration: false,
        is_entry: false,
        is_impure: hook.is_impure,
        span: hook.span,
    }
}

// ---------------------------------------------------------------------------
// Generic detection
// ---------------------------------------------------------------------------

fn is_generic_fn(f: &TypedFnDef) -> bool {
    if f.is_builtin {
        return false;
    }
    f.params.iter().any(|p| contains_type_param(&p.ty))
}

pub fn contains_type_param_pub(ty: &Ty) -> bool {
    contains_type_param(ty)
}

fn contains_type_param(ty: &Ty) -> bool {
    match ty {
        Ty::GenericParam(_) => true,
        Ty::Named(_, _, args) => args.iter().any(contains_type_param),
        Ty::Ref(t, _) => contains_type_param(t),
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
        Ty::Void => "void".into(),
        Ty::Ref(inner, _) => type_mono_name(inner),
        Ty::Named(_, name, args) if args.is_empty() => name.clone(),
        Ty::Named(_, name, args) => {
            let arg_names: Vec<String> = args.iter().map(type_mono_name).collect();
            format!("{}_{}", name, arg_names.join("_"))
        }
        Ty::Interface(_, name) | Ty::GenericParam(name) => name.clone(),
        Ty::Tuple(ts) => {
            let inner: Vec<String> = ts.iter().map(type_mono_name).collect();
            format!("Tuple_{}", inner.join("_"))
        }
        _ => type_name_of(ty).unwrap_or_else(|| "unknown".into()),
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
        TypedStmt::CompoundAssign { target, rhs, .. } => {
            seed_expr(target, generic_fns, queue);
            seed_expr(rhs, generic_fns, queue);
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

// Resolve the effective type of an expression, recursively resolving generic call return types.
// When an arg's type is still a GenericParam (e.g. the return of an unsubstituted generic call),
// this looks at the call's arg types to compute the concrete return type via unification.
fn resolve_expr_ty(expr: &TypedExpr, generic_fns: &HashMap<String, TypedFnDef>) -> Ty {
    if matches!(&expr.ty, Ty::GenericParam(_)) {
        if let TypedExprKind::Call {
            callee,
            args,
            generic_params,
            ..
        } = &expr.kind
        {
            if !generic_params.is_empty() {
                if let TypedExprKind::Ident(name) = &callee.kind {
                    if let Some(gf) = generic_fns.get(name.as_str()) {
                        let arg_tys: Vec<Ty> = args
                            .iter()
                            .map(|a| resolve_expr_ty(a, generic_fns))
                            .collect();
                        let call_subst = unify_params(gf, &arg_tys);
                        let resolved = subst_ty(&gf.return_type, &call_subst);
                        if !matches!(resolved, Ty::GenericParam(_)) {
                            return resolved;
                        }
                    }
                }
            }
        }
    }
    expr.ty.clone()
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
                        let arg_tys: Vec<Ty> = args
                            .iter()
                            .map(|a| resolve_expr_ty(a, generic_fns))
                            .collect();
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
        TypedExprKind::Array(elems) => {
            for e in elems {
                seed_expr(e, generic_fns, queue);
            }
        }
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
        TypedExprKind::Block(stmts) => {
            for s in stmts {
                seed_stmt(s, generic_fns, queue);
            }
        }
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Ident(_)
        | TypedExprKind::EnumVariant { .. } => {}
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
        (Ty::GenericParam(name), _) => {
            subst
                .entry(name.clone())
                .or_insert_with(|| concrete.clone());
        }
        (Ty::Named(_, pname, pargs), Ty::Named(_, cname, cargs)) if pname == cname => {
            for (pa, ca) in pargs.iter().zip(cargs.iter()) {
                unify_ty(pa, ca, subst);
            }
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
    registry: &TypeRegistry,
    generic_fns: &HashMap<String, TypedFnDef>,
    generic_impl_hooks: &HashMap<(String, String), (TypedHookDef, Ty)>,
    impl_reqs: &mut Vec<ImplHookReq>,
) -> TypedFnDef {
    // Extend substitution with associated type bindings so that Ty::Projection
    // values in the function body and return type resolve correctly.
    let extended_subst = extend_with_assoc_bindings(subst, registry);
    let subst = &extended_subst;
    let name = specialized_name(&f.name, subst);
    let params: Vec<TypedParam> = f
        .params
        .iter()
        .map(|p| TypedParam {
            name: p.name.clone(),
            ty: subst_ty(&p.ty, subst),
            mutable: p.mutable,
            span: p.span,
        })
        .collect();
    let return_type = subst_ty(&f.return_type, subst);
    let body = subst_block(&f.body, subst, generic_fns, generic_impl_hooks, impl_reqs);
    TypedFnDef {
        name,
        params,
        variadic: f.variadic.clone(),
        return_type,
        body,
        is_builtin: false,
        is_inline: f.is_inline,
        is_declaration: false,
        is_entry: f.is_entry,
        is_impure: f.is_impure,
        span: f.span,
    }
}

// ---------------------------------------------------------------------------
// Type substitution
// ---------------------------------------------------------------------------

fn subst_ty(ty: &Ty, subst: &HashMap<String, Ty>) -> Ty {
    match ty {
        Ty::GenericParam(name) => subst
            .get(name.as_str())
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        // Resolve projections: if base param is substituted, look up the assoc binding
        // from the extended substitution (which includes assoc type bindings added by
        // `extend_with_assoc_bindings`).
        Ty::Projection { base, assoc } => {
            let key = format!("{}::{}", base, assoc);
            if let Some(resolved) = subst.get(&key) {
                resolved.clone()
            } else {
                ty.clone()
            }
        }
        Ty::Named(id, name, args) => Ty::Named(
            id.clone(),
            name.clone(),
            args.iter().map(|a| subst_ty(a, subst)).collect(),
        ),
        Ty::Ref(t, m) => Ty::Ref(Box::new(subst_ty(t, subst)), *m),
        Ty::Callable(ps, r) => Ty::Callable(
            ps.iter().map(|p| subst_ty(p, subst)).collect(),
            Box::new(subst_ty(r, subst)),
        ),
        Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| subst_ty(t, subst)).collect()),
        other => other.clone(),
    }
}

/// Extend a generic-param substitution with associated type bindings from all
/// conformance entries for each concrete type in the substitution.
/// Keys for assoc bindings use the format `"ParamName::AssocName"` to avoid
/// collisions with regular generic param names.
fn extend_with_assoc_bindings(
    subst: &HashMap<String, Ty>,
    registry: &TypeRegistry,
) -> HashMap<String, Ty> {
    let mut result = subst.clone();
    for (param_name, concrete_ty) in subst {
        let type_name = match concrete_ty {
            Ty::Named(_, name, _) => name.clone(),
            Ty::Int => "int".to_string(),
            Ty::Float => "float".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Str => "str".to_string(),
            _ => continue,
        };
        for (assoc_name, assoc_ty) in registry.all_assoc_bindings_for(&type_name) {
            let key = format!("{}::{}", param_name, assoc_name);
            result.entry(key).or_insert(assoc_ty);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// AST traversal: substitute types and rewrite call sites
// ---------------------------------------------------------------------------

fn subst_block(
    block: &TypedBlock,
    subst: &HashMap<String, Ty>,
    generic_fns: &HashMap<String, TypedFnDef>,
    generic_impl_hooks: &HashMap<(String, String), (TypedHookDef, Ty)>,
    impl_reqs: &mut Vec<ImplHookReq>,
) -> TypedBlock {
    TypedBlock {
        stmts: block
            .stmts
            .iter()
            .map(|s| subst_stmt(s, subst, generic_fns, generic_impl_hooks, impl_reqs))
            .collect(),
        span: block.span,
    }
}

fn subst_stmt(
    stmt: &TypedStmt,
    subst: &HashMap<String, Ty>,
    generic_fns: &HashMap<String, TypedFnDef>,
    generic_impl_hooks: &HashMap<(String, String), (TypedHookDef, Ty)>,
    impl_reqs: &mut Vec<ImplHookReq>,
) -> TypedStmt {
    macro_rules! se {
        ($e:expr) => {
            subst_expr($e, subst, generic_fns, generic_impl_hooks, impl_reqs)
        };
    }
    macro_rules! sb {
        ($b:expr) => {
            subst_block($b, subst, generic_fns, generic_impl_hooks, impl_reqs)
        };
    }
    match stmt {
        TypedStmt::Expr(e) => TypedStmt::Expr(se!(e)),
        TypedStmt::Return { value, span } => TypedStmt::Return {
            value: value.as_ref().map(|v| se!(v)),
            span: *span,
        },
        TypedStmt::Raise { value, span } => TypedStmt::Raise {
            value: value.as_ref().map(|v| se!(v)),
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
            value: se!(value),
            mutable: *mutable,
            span: *span,
        },
        TypedStmt::Assign {
            target,
            value,
            span,
        } => TypedStmt::Assign {
            target: se!(target),
            value: se!(value),
            span: *span,
        },
        TypedStmt::CompoundAssign {
            target,
            op,
            rhs,
            span,
        } => TypedStmt::CompoundAssign {
            target: se!(target),
            op: op.clone(),
            rhs: se!(rhs),
            span: *span,
        },
        TypedStmt::If {
            branches,
            else_branch,
            span,
        } => TypedStmt::If {
            branches: branches.iter().map(|(c, b)| (se!(c), sb!(b))).collect(),
            else_branch: else_branch.as_ref().map(|b| sb!(b)),
            span: *span,
        },
        TypedStmt::While { cond, body, span } => TypedStmt::While {
            cond: se!(cond),
            body: sb!(body),
            span: *span,
        },
        TypedStmt::DoWhile { body, cond, span } => TypedStmt::DoWhile {
            body: sb!(body),
            cond: se!(cond),
            span: *span,
        },
        TypedStmt::For {
            binding,
            binding_ty,
            iterable,
            body,
            iter_ty,
            span,
        } => {
            let new_iterable = se!(iterable);
            let new_iter_ty = iter_ty.as_ref().map(|t| subst_ty(t, subst));
            // Emit impl_reqs for iter() and next() so monomorphized hooks are compiled.
            if let Some(it) = &new_iter_ty {
                impl_reqs.push((type_base_name(it), "next".to_string(), it.clone()));
                impl_reqs.push((
                    type_base_name(&new_iterable.ty),
                    "iter".to_string(),
                    new_iterable.ty.clone(),
                ));
            }
            TypedStmt::For {
                binding: binding.clone(),
                binding_ty: subst_ty(binding_ty, subst),
                iterable: new_iterable,
                body: sb!(body),
                iter_ty: new_iter_ty,
                span: *span,
            }
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => TypedStmt::TryCatch {
            body: sb!(body),
            handlers: handlers
                .iter()
                .map(|h| TypedCatchHandler {
                    ty: subst_ty(&h.ty, subst),
                    binding: h.binding.clone(),
                    body: sb!(&h.body),
                    span: h.span,
                })
                .collect(),
            finally: finally.as_ref().map(|b| sb!(b)),
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
                    mutable: p.mutable,
                    span: p.span,
                })
                .collect(),
            variadic: f.variadic.clone(),
            return_type: subst_ty(&f.return_type, subst),
            body: sb!(&f.body),
            is_builtin: f.is_builtin,
            is_inline: f.is_inline,
            is_declaration: f.is_declaration,
            is_entry: f.is_entry,
            is_impure: f.is_impure,
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
    generic_impl_hooks: &HashMap<(String, String), (TypedHookDef, Ty)>,
    impl_reqs: &mut Vec<ImplHookReq>,
) -> TypedExpr {
    macro_rules! se {
        ($e:expr) => {
            subst_expr($e, subst, generic_fns, generic_impl_hooks, impl_reqs)
        };
    }
    macro_rules! sb {
        ($b:expr) => {
            subst_block($b, subst, generic_fns, generic_impl_hooks, impl_reqs)
        };
    }
    let ty = subst_ty(&expr.ty, subst);
    let kind = match &expr.kind {
        TypedExprKind::Call {
            callee,
            args,
            fn_name,
            generic_params,
            generic_bounds,
            param_tys,
        } => {
            let new_args: Vec<TypedExpr> = args.iter().map(|a| se!(a)).collect();
            // Emit to_str impl_reqs for println/print so concrete generic types
            // (e.g. ListNode[int]) get their to_str hook compiled.
            if let TypedExprKind::Ident(call_name) = &callee.kind {
                if (call_name == "println" || call_name == "print") && new_args.len() == 1 {
                    let arg_ty = &new_args[0].ty;
                    let inner_ty = match arg_ty {
                        Ty::Ref(inner, _) => inner.as_ref(),
                        other => other,
                    };
                    if let Ty::Named(_, base, type_args) = inner_ty {
                        if !type_args.is_empty()
                            && generic_impl_hooks
                                .contains_key(&(base.clone(), "to_str".to_string()))
                        {
                            impl_reqs.push((base.clone(), "to_str".to_string(), inner_ty.clone()));
                        }
                    }
                }
            }
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
                        // Use call_subst (not outer subst) to resolve the return type,
                        // so a generic return like T in sum[T]->T gets resolved to int.
                        let resolved_ty = subst_ty(&expr.ty, &call_subst);
                        return TypedExpr {
                            kind: TypedExprKind::Call {
                                callee: Box::new(new_callee),
                                args: new_args,
                                fn_name: fn_name.clone(),
                                generic_bounds: vec![],
                                generic_params: vec![],
                                param_tys: vec![],
                            },
                            ty: resolved_ty,
                            span: expr.span,
                        };
                    }
                }
            }
            let new_callee = se!(callee);
            TypedExprKind::Call {
                callee: Box::new(new_callee),
                args: new_args,
                fn_name: fn_name.clone(),
                generic_bounds: generic_bounds.clone(),
                generic_params: generic_params.clone(),
                param_tys: param_tys.iter().map(|t| subst_ty(t, subst)).collect(),
            }
        }

        TypedExprKind::MethodCall {
            object,
            method_fn,
            args,
        } => {
            let new_obj = se!(object);
            let new_args: Vec<TypedExpr> = args.iter().map(|a| se!(a)).collect();
            let rewritten = rewrite_method_fn(method_fn, subst, generic_impl_hooks, impl_reqs);
            // When the receiver is a concrete parameterized type (e.g. ListNode[int])
            // and the method targets a generic impl hook (e.g. ListNode_to_str),
            // rewrite to the monomorphized name (ListNode_int_to_str) and emit an impl_req.
            let final_method_fn = {
                let inner_ty = match &new_obj.ty {
                    Ty::Ref(inner, _) => inner.as_ref(),
                    other => other,
                };
                if let Ty::Named(_, base, type_args) = inner_ty {
                    if !type_args.is_empty() && !contains_type_param(inner_ty) {
                        let prefix = format!("{}_", base);
                        if let Some(suffix) = rewritten.strip_prefix(prefix.as_str()) {
                            if generic_impl_hooks.contains_key(&(base.clone(), suffix.to_string()))
                            {
                                impl_reqs.push((
                                    base.clone(),
                                    suffix.to_string(),
                                    inner_ty.clone(),
                                ));
                                format!("{}_{}", type_mono_name(inner_ty), suffix)
                            } else {
                                rewritten
                            }
                        } else {
                            rewritten
                        }
                    } else {
                        rewritten
                    }
                } else {
                    rewritten
                }
            };
            TypedExprKind::MethodCall {
                object: Box::new(new_obj),
                method_fn: final_method_fn,
                args: new_args,
            }
        }

        TypedExprKind::StaticCall { method_fn, args } => {
            let new_args: Vec<TypedExpr> = args.iter().map(|a| se!(a)).collect();
            let rewritten = rewrite_method_fn(method_fn, subst, generic_impl_hooks, impl_reqs);
            // If no param substitution happened but the expression's return type is
            // fully concrete, check whether the call targets a generic impl hook and
            // emit the required specialization (e.g. ListNode[int].new()).
            let final_fn = if rewritten == *method_fn && !contains_type_param(&ty) {
                let mut specialized = None;
                for ((base, suffix), _) in generic_impl_hooks.iter() {
                    if *method_fn == format!("{}_{}", base, suffix) {
                        impl_reqs.push((base.clone(), suffix.clone(), ty.clone()));
                        specialized = Some(format!("{}_{}", type_mono_name(&ty), suffix));
                        break;
                    }
                }
                specialized.unwrap_or(rewritten)
            } else {
                rewritten
            };
            TypedExprKind::StaticCall {
                method_fn: final_fn,
                args: new_args,
            }
        }

        TypedExprKind::IndirectCall { fat_ptr, args } => {
            let new_fp = se!(fat_ptr);
            let new_args: Vec<TypedExpr> = args.iter().map(|a| se!(a)).collect();
            TypedExprKind::IndirectCall {
                fat_ptr: Box::new(new_fp),
                args: new_args,
            }
        }

        TypedExprKind::BinOp { op, left, right } => TypedExprKind::BinOp {
            op: op.clone(),
            left: Box::new(se!(left)),
            right: Box::new(se!(right)),
        },

        TypedExprKind::UnOp { op, operand } => TypedExprKind::UnOp {
            op: op.clone(),
            operand: Box::new(se!(operand)),
        },

        TypedExprKind::Field { object, field } => TypedExprKind::Field {
            object: Box::new(se!(object)),
            field: field.clone(),
        },

        TypedExprKind::Index { object, index } => TypedExprKind::Index {
            object: Box::new(se!(object)),
            index: Box::new(se!(index)),
        },

        TypedExprKind::StructLiteral { ty_name, fields } => TypedExprKind::StructLiteral {
            ty_name: ty_name.clone(),
            fields: fields.iter().map(|(n, e)| (n.clone(), se!(e))).collect(),
        },

        TypedExprKind::Tuple(elems) => TypedExprKind::Tuple(elems.iter().map(|e| se!(e)).collect()),

        TypedExprKind::Match { scrutinee, arms } => TypedExprKind::Match {
            scrutinee: Box::new(se!(scrutinee)),
            arms: arms
                .iter()
                .map(|arm| TypedMatchArm {
                    pattern: arm.pattern.clone(),
                    guard: arm.guard.as_ref().map(|g| se!(g)),
                    body: se!(&arm.body),
                    narrowed_discriminant: arm.narrowed_discriminant,
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
                    mutable: p.mutable,
                    span: p.span,
                })
                .collect(),
            body: match body {
                TypedClosureBody::Expr(e) => TypedClosureBody::Expr(Box::new(se!(e))),
                TypedClosureBody::Block(b) => TypedClosureBody::Block(sb!(b)),
            },
        },

        TypedExprKind::Unwrap(inner) => TypedExprKind::Unwrap(Box::new(se!(inner))),
        TypedExprKind::As { expr: e, ty: t } => TypedExprKind::As {
            expr: Box::new(se!(e)),
            ty: subst_ty(t, subst),
        },
        TypedExprKind::Spawn(inner) => TypedExprKind::Spawn(Box::new(se!(inner))),
        TypedExprKind::Ref { mutable, expr: e } => TypedExprKind::Ref {
            mutable: *mutable,
            expr: Box::new(se!(e)),
        },
        TypedExprKind::Array(elems) => TypedExprKind::Array(elems.iter().map(|e| se!(e)).collect()),
        TypedExprKind::Gen { body } => TypedExprKind::Gen { body: sb!(body) },
        TypedExprKind::GenSplice(inner) => TypedExprKind::GenSplice(Box::new(se!(inner))),
        TypedExprKind::Block(stmts) => TypedExprKind::Block(
            stmts
                .iter()
                .map(|s| subst_stmt(s, subst, generic_fns, generic_impl_hooks, impl_reqs))
                .collect(),
        ),
        TypedExprKind::Str(segs) => {
            let new_segs: Vec<TypedStringSegment> = segs
                .iter()
                .map(|seg| match seg {
                    TypedStringSegment::Text(t) => TypedStringSegment::Text(t.clone()),
                    TypedStringSegment::Interp(e) => {
                        let substed = se!(e);
                        // If the interpolated value has a concrete Named type with
                        // a registered to_str hook, emit an impl_req so the
                        // monomorphized to_str function gets compiled.
                        if let Ty::Named(_, base, args) = &substed.ty {
                            if !args.is_empty()
                                && generic_impl_hooks
                                    .contains_key(&(base.clone(), "to_str".to_string()))
                            {
                                impl_reqs.push((
                                    base.clone(),
                                    "to_str".to_string(),
                                    substed.ty.clone(),
                                ));
                            }
                        }
                        TypedStringSegment::Interp(substed)
                    }
                })
                .collect();
            TypedExprKind::Str(new_segs)
        }
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

pub fn type_base_name(ty: &Ty) -> String {
    match ty {
        Ty::Named(_, name, _) | Ty::GenericParam(name) => name.clone(),
        _ => type_mono_name(ty),
    }
}

fn rewrite_method_fn(
    method_fn: &str,
    subst: &HashMap<String, Ty>,
    generic_impl_hooks: &HashMap<(String, String), (TypedHookDef, Ty)>,
    impl_reqs: &mut Vec<ImplHookReq>,
) -> String {
    for (param, concrete_ty) in subst {
        let prefix = format!("{}_", param);
        if method_fn.starts_with(prefix.as_str()) {
            let suffix = &method_fn[prefix.len()..];
            let concrete_name = type_mono_name(concrete_ty);
            let base = type_base_name(concrete_ty);
            if generic_impl_hooks.contains_key(&(base.clone(), suffix.to_string())) {
                impl_reqs.push((base, suffix.to_string(), concrete_ty.clone()));
            }
            return format!("{}_{}", concrete_name, suffix);
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
        assert_eq!(
            type_mono_name(&Ty::Named(TypeId(99), "Vec".into(), vec![Ty::Int])),
            "Vec_int"
        );
    }

    #[test]
    fn specialized_name_single_param() {
        let mut subst = HashMap::new();
        subst.insert("T".to_string(), Ty::Int);
        assert_eq!(specialized_name("identity", &subst), "identity__T__int");
    }

    #[test]
    fn rewrite_method_fn_named() {
        let hooks: HashMap<(String, String), (TypedHookDef, Ty)> = HashMap::new();
        let mut reqs: Vec<ImplHookReq> = Vec::new();
        let mut subst = HashMap::new();
        subst.insert(
            "T".to_string(),
            Ty::Named(TypeId(1), "Circle".into(), vec![]),
        );
        assert_eq!(
            rewrite_method_fn("T_draw", &subst, &hooks, &mut reqs),
            "Circle_draw"
        );
    }

    #[test]
    fn rewrite_method_fn_no_match() {
        let hooks: HashMap<(String, String), (TypedHookDef, Ty)> = HashMap::new();
        let mut reqs: Vec<ImplHookReq> = Vec::new();
        let subst: HashMap<String, Ty> = HashMap::new();
        assert_eq!(
            rewrite_method_fn("Vec_add", &subst, &hooks, &mut reqs),
            "Vec_add"
        );
    }

    #[test]
    fn rewrite_method_fn_vec_uses_mono_name() {
        let hooks: HashMap<(String, String), (TypedHookDef, Ty)> = HashMap::new();
        let mut reqs: Vec<ImplHookReq> = Vec::new();
        let mut subst = HashMap::new();
        subst.insert(
            "T".to_string(),
            Ty::Named(
                TypeId(99),
                "Vec".into(),
                vec![Ty::Named(TypeId(1), "Item".into(), vec![])],
            ),
        );
        assert_eq!(
            rewrite_method_fn("T_to_str", &subst, &hooks, &mut reqs),
            "Vec_Item_to_str"
        );
    }

    #[test]
    fn contains_type_param_generic_param() {
        assert!(contains_type_param(&Ty::GenericParam("T".into())));
        assert!(!contains_type_param(&Ty::Named(
            TypeId(1),
            "Circle".into(),
            vec![]
        )));
        assert!(contains_type_param(&Ty::Named(
            TypeId(99),
            "Vec".into(),
            vec![Ty::GenericParam("T".into())]
        )));
    }
}
