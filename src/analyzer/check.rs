use crate::analyzer::env::{Env, Symbol};
use crate::analyzer::error::AnalysisError;
use crate::analyzer::infer::{
    check_assignable, check_assignable_with_decl_span, infer_typed_expr, subst_generic_ty,
    substitute_t,
};
use crate::analyzer::resolve::resolve_type_expr;
use crate::analyzer::ty::{ComputedVariance, Ty, TypeKind, TypeRegistry};
use crate::diagnostics::Span;

use crate::analyzer::typed_ast::{
    TypedBlock, TypedCatchHandler, TypedClosureBody, TypedExprKind, TypedFnDef, TypedParam,
    TypedStmt, TypedStringSegment,
};
use crate::parser::ast::{Block, Expr, FnDef, Stmt};

/// Check all statements in `block` and return a typed block.
pub fn check_typed_block(
    block: &Block,
    env: &mut Env,
    registry: &TypeRegistry,
    return_ty: &Ty,
    errors: &mut Vec<AnalysisError>,
) -> TypedBlock {
    env.push_scope();
    let mut stmts: Vec<TypedStmt> = Vec::new();
    let mut terminator_span: Option<Span> = None;

    for s in &block.stmts {
        if let Some(term_span) = terminator_span {
            errors.push(AnalysisError::UnreachableCode {
                span: ast_stmt_span(s),
                terminator_span: term_span,
            });
            break;
        }
        let typed = check_typed_stmt(s, env, registry, return_ty, errors);
        if is_definite_terminator(&typed) {
            terminator_span = Some(typed_stmt_span(&typed));
        }
        stmts.push(typed);
    }

    emit_unused_var_warnings(env, &stmts, errors);
    env.pop_scope();
    TypedBlock {
        stmts,
        span: block.span,
    }
}

/// If `cond` is `implements(varname, Iface)`, return the var name and its
/// narrowed compound type for use in the true branch.
fn detect_implements_narrowing(
    cond: &crate::analyzer::typed_ast::TypedExpr,
    env: &crate::analyzer::env::Env,
) -> Option<(String, crate::analyzer::ty::Ty)> {
    use crate::analyzer::env::Symbol;
    use crate::analyzer::ty::Ty;
    use crate::analyzer::typed_ast::TypedExprKind;
    if let TypedExprKind::Implements { expr, iface_name } = &cond.kind {
        if let TypedExprKind::Ident(var_name) = &expr.kind {
            if let Some(Symbol::Var {
                ty: original_ty, ..
            }) = env.lookup(var_name)
            {
                if let Some(Symbol::Iface { id, .. }) = env.lookup(iface_name) {
                    let iface_ty = Ty::Interface(id.clone(), iface_name.clone());
                    let narrowed = match original_ty {
                        Ty::Compound(parts) => {
                            let mut p = parts.clone();
                            if !p.contains(&iface_ty) {
                                p.push(iface_ty);
                            }
                            Ty::Compound(p)
                        }
                        other => Ty::Compound(vec![other.clone(), iface_ty]),
                    };
                    return Some((var_name.clone(), narrowed));
                }
            }
        }
    }
    None
}

fn is_definite_terminator(ts: &TypedStmt) -> bool {
    matches!(
        ts,
        TypedStmt::Return { .. }
            | TypedStmt::Break(_)
            | TypedStmt::Continue(_)
            | TypedStmt::Raise { .. }
            | TypedStmt::Abandon { .. }
    )
}

fn typed_stmt_span(ts: &TypedStmt) -> Span {
    match ts {
        TypedStmt::VarDecl { span, .. }
        | TypedStmt::Assign { span, .. }
        | TypedStmt::CompoundAssign { span, .. }
        | TypedStmt::Return { span, .. }
        | TypedStmt::Raise { span, .. }
        | TypedStmt::Abandon { span, .. }
        | TypedStmt::If { span, .. }
        | TypedStmt::While { span, .. }
        | TypedStmt::DoWhile { span, .. }
        | TypedStmt::For { span, .. }
        | TypedStmt::TryCatch { span, .. } => *span,
        TypedStmt::Break(s) | TypedStmt::Continue(s) => *s,
        TypedStmt::FnDef(f) => f.span,
        TypedStmt::Expr(e) => e.span,
    }
}

fn ast_stmt_span(s: &Stmt) -> Span {
    match s {
        Stmt::VarDecl { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::CompoundAssign { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::Raise { span, .. }
        | Stmt::Abandon { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::DoWhile { span, .. }
        | Stmt::For { span, .. }
        | Stmt::TryCatch { span, .. } => *span,
        Stmt::Break(s) | Stmt::Continue(s) => *s,
        Stmt::FnDef(f) => f.span,
        Stmt::Expr(e) => e.span(),
    }
}

fn check_typed_stmt(
    stmt: &Stmt,
    env: &mut Env,
    registry: &TypeRegistry,
    return_ty: &Ty,
    errors: &mut Vec<AnalysisError>,
) -> TypedStmt {
    match stmt {
        Stmt::VarDecl {
            name,
            ty,
            value,
            mutable,
            span,
        } => {
            let declared = resolve_type_expr(ty, env, errors);
            let typed_val = infer_typed_expr(value, env, registry, errors);
            let typed_val =
                crate::analyzer::infer::coerce_generic_to_declared(typed_val, &declared);
            if let Ty::Interface(_, iface_name) = &declared {
                // Verify the assigned type implements the interface. Unknown is
                // allowed to avoid double-reporting after earlier errors.
                if let Some(type_name) = crate::analyzer::infer::type_name_of(&typed_val.ty) {
                    if registry.get_conformances(&type_name, iface_name).is_empty() {
                        errors.push(AnalysisError::TypeMismatch {
                            expected: iface_name.clone(),
                            found: type_name,
                            span: typed_val.span,
                            decl_span: Some(*span),
                        });
                    }
                }
            } else if let Ty::Compound(parts) = &declared {
                // Verify the assigned type implements ALL constituent interfaces.
                if let Some(type_name) = crate::analyzer::infer::type_name_of(&typed_val.ty) {
                    for part in parts {
                        if let Ty::Interface(_, iface_name) = part {
                            if registry.get_conformances(&type_name, iface_name).is_empty() {
                                errors.push(AnalysisError::TypeMismatch {
                                    expected: iface_name.clone(),
                                    found: type_name.clone(),
                                    span: typed_val.span,
                                    decl_span: Some(*span),
                                });
                            }
                        }
                    }
                }
            } else {
                check_assignable_with_decl_span(
                    &declared,
                    &typed_val.ty,
                    &typed_val.span,
                    Some(*span),
                    errors,
                );
                // Additional check: enforce invariant variance where declared.
                check_variance_assignment(
                    &declared,
                    &typed_val.ty,
                    registry,
                    typed_val.span,
                    errors,
                );
            }
            env.define(
                name,
                Symbol::Var {
                    ty: declared.clone(),
                    mutable: *mutable,
                    span: *span,
                },
            );
            TypedStmt::VarDecl {
                name: name.clone(),
                ty: declared,
                value: typed_val,
                mutable: *mutable,
                span: *span,
            }
        }

        Stmt::Assign {
            target,
            value,
            span,
        } => {
            if let Expr::Ident(name, ident_span) = target {
                match env.lookup(name) {
                    Some(Symbol::Var {
                        mutable: false,
                        span: decl_span,
                        ..
                    }) => {
                        errors.push(AnalysisError::AssignToImmutable {
                            name: name.clone(),
                            span: *ident_span,
                            decl_span: Some(*decl_span),
                        });
                    }
                    Some(Symbol::Const { .. }) => {
                        errors.push(AnalysisError::AssignToConst {
                            name: name.clone(),
                            span: *ident_span,
                        });
                    }
                    None => {
                        errors.push(AnalysisError::UndefinedName {
                            name: name.clone(),
                            span: *ident_span,
                            did_you_mean: None,
                        });
                    }
                    _ => {}
                }
            }
            let typed_target = infer_typed_expr(target, env, registry, errors);
            let typed_val = infer_typed_expr(value, env, registry, errors);
            check_assignable(&typed_target.ty, &typed_val.ty, &typed_val.span, errors);
            TypedStmt::Assign {
                target: typed_target,
                value: typed_val,
                span: *span,
            }
        }

        Stmt::CompoundAssign {
            target,
            op,
            rhs,
            span,
        } => {
            if let Expr::Ident(name, ident_span) = target {
                match env.lookup(name) {
                    Some(Symbol::Var {
                        mutable: false,
                        span: decl_span,
                        ..
                    }) => {
                        errors.push(AnalysisError::AssignToImmutable {
                            name: name.clone(),
                            span: *ident_span,
                            decl_span: Some(*decl_span),
                        });
                    }
                    Some(Symbol::Const { .. }) => {
                        errors.push(AnalysisError::AssignToConst {
                            name: name.clone(),
                            span: *ident_span,
                        });
                    }
                    None => {
                        errors.push(AnalysisError::UndefinedName {
                            name: name.clone(),
                            span: *ident_span,
                            did_you_mean: None,
                        });
                    }
                    _ => {}
                }
            }
            TypedStmt::CompoundAssign {
                target: infer_typed_expr(target, env, registry, errors),
                op: op.clone(),
                rhs: infer_typed_expr(rhs, env, registry, errors),
                span: *span,
            }
        }

        Stmt::Return { value, span } => {
            let (typed, val_span) = match value {
                Some(v) => {
                    let te = infer_typed_expr(v, env, registry, errors);
                    let s = te.span;
                    (Some(te), s)
                }
                None => (None, *span),
            };
            let found = typed.as_ref().map(|t| t.ty.clone()).unwrap_or(Ty::Void);
            check_assignable(return_ty, &found, &val_span, errors);
            TypedStmt::Return {
                value: typed,
                span: *span,
            }
        }

        Stmt::Raise { value, span } => {
            let typed = value
                .as_ref()
                .map(|v| infer_typed_expr(v, env, registry, errors));
            TypedStmt::Raise {
                value: typed,
                span: *span,
            }
        }

        Stmt::Abandon { message, span } => {
            let typed = message
                .as_ref()
                .map(|v| infer_typed_expr(v, env, registry, errors));
            TypedStmt::Abandon {
                message: typed,
                span: *span,
            }
        }

        Stmt::Expr(expr) => TypedStmt::Expr(infer_typed_expr(expr, env, registry, errors)),

        Stmt::If {
            branches,
            else_branch,
            span,
        } => {
            let mut typed_branches = Vec::new();
            for (cond, block) in branches {
                let tc = infer_typed_expr(cond, env, registry, errors);
                if !matches!(tc.ty, Ty::Bool | Ty::Unknown) {
                    errors.push(AnalysisError::TypeMismatch {
                        expected: "bool".into(),
                        found: tc.ty.to_string(),
                        span: cond.span(),
                        decl_span: None,
                    });
                }
                let narrowing = detect_implements_narrowing(&tc, env);
                if let Some((ref vname, ref nty)) = narrowing {
                    env.push_scope();
                    env.define(
                        vname,
                        crate::analyzer::env::Symbol::Var {
                            ty: nty.clone(),
                            mutable: false,
                            span: *span,
                        },
                    );
                }
                let tb = check_typed_block(block, env, registry, return_ty, errors);
                if narrowing.is_some() {
                    env.pop_scope();
                }
                typed_branches.push((tc, tb));
            }
            let else_typed = else_branch
                .as_ref()
                .map(|b| check_typed_block(b, env, registry, return_ty, errors));
            TypedStmt::If {
                branches: typed_branches,
                else_branch: else_typed,
                span: *span,
            }
        }

        Stmt::While { cond, body, span } => {
            let tc = infer_typed_expr(cond, env, registry, errors);
            if !matches!(tc.ty, Ty::Bool | Ty::Unknown) {
                errors.push(AnalysisError::TypeMismatch {
                    expected: "bool".into(),
                    found: tc.ty.to_string(),
                    span: *span,
                    decl_span: None,
                });
            }
            let tb = check_typed_block(body, env, registry, return_ty, errors);
            TypedStmt::While {
                cond: tc,
                body: tb,
                span: *span,
            }
        }

        Stmt::DoWhile { body, cond, span } => {
            let tb = check_typed_block(body, env, registry, return_ty, errors);
            let tc = infer_typed_expr(cond, env, registry, errors);
            if !matches!(tc.ty, Ty::Bool | Ty::Unknown) {
                errors.push(AnalysisError::TypeMismatch {
                    expected: "bool".into(),
                    found: tc.ty.to_string(),
                    span: *span,
                    decl_span: None,
                });
            }
            TypedStmt::DoWhile {
                body: tb,
                cond: tc,
                span: *span,
            }
        }

        Stmt::For {
            binding,
            binding_ty,
            iterable,
            body,
            span,
        } => {
            let ti = infer_typed_expr(iterable, env, registry, errors);
            // Determine element type and optional iterator type for custom Iterable dispatch.
            let mut iter_ty: Option<Ty> = None;
            let elem_ty = match &ti.ty {
                Ty::Named(_, name, args) if name == "Vec" => {
                    let elem = args.first().cloned().unwrap_or(Ty::Unknown);
                    if let Some(iter_method) = registry.find_method("Vec", "iter") {
                        iter_ty = Some(substitute_t(&iter_method.ret, &elem));
                    }
                    elem
                }
                Ty::Named(_, name, args) if name == "Set" => {
                    args.first().cloned().unwrap_or(Ty::Unknown)
                }
                Ty::Str => Ty::Str,
                Ty::Named(_, name, args) if args.is_empty() => {
                    match registry.lookup_by_name(name) {
                        Some(entry) if matches!(&entry.kind, TypeKind::Enum { .. }) => {
                            ti.ty.clone()
                        }
                        _ => {
                            // Check for custom Iterable: look for an iter() method.
                            if let Some(iter_method) = registry.find_method(name, "iter") {
                                let it = iter_method.ret.clone();
                                // Derive element type from Iterator::next() -> Option[Item].
                                let item_ty = if let Ty::Named(_, iter_name, _) = &it {
                                    registry
                                        .find_method(iter_name, "next")
                                        .and_then(|m| {
                                            if let Ty::Named(_, opt_name, opt_args) = &m.ret {
                                                if opt_name == "Option" {
                                                    opt_args.first().cloned()
                                                } else {
                                                    None
                                                }
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or(Ty::Unknown)
                                } else {
                                    Ty::Unknown
                                };
                                iter_ty = Some(it);
                                item_ty
                            } else {
                                Ty::Unknown
                            }
                        }
                    }
                }
                // Generic custom Iterable, e.g. LinkedList[int].
                Ty::Named(_, name, args) => {
                    if let Some(iter_method) = registry.find_method(name, "iter") {
                        // Build substitution map: generic param names -> concrete type args.
                        let subst: std::collections::HashMap<String, Ty> = registry
                            .get_generic_param_order(name)
                            .map(|params| {
                                params
                                    .iter()
                                    .zip(args.iter())
                                    .map(|(p, a)| (p.clone(), a.clone()))
                                    .collect()
                            })
                            .unwrap_or_default();
                        // Apply substitution to the iter() return type to get concrete iterator type.
                        let raw_iter_ret = iter_method.ret.clone();
                        let concrete_iter_ty = if subst.is_empty() {
                            raw_iter_ret
                        } else {
                            subst_generic_ty(&raw_iter_ret, &subst)
                        };
                        // Derive element type from Iterator::next() -> Option[Item] on the
                        // concrete iterator type, applying its own generic substitution.
                        let item_ty = if let Ty::Named(_, iter_name, iter_args) = &concrete_iter_ty
                        {
                            let iter_subst: std::collections::HashMap<String, Ty> = registry
                                .get_generic_param_order(iter_name)
                                .map(|params| {
                                    params
                                        .iter()
                                        .zip(iter_args.iter())
                                        .map(|(p, a)| (p.clone(), a.clone()))
                                        .collect()
                                })
                                .unwrap_or_default();
                            registry
                                .find_method(iter_name, "next")
                                .and_then(|m| {
                                    let concrete_ret = if iter_subst.is_empty() {
                                        m.ret.clone()
                                    } else {
                                        subst_generic_ty(&m.ret, &iter_subst)
                                    };
                                    if let Ty::Named(_, opt_name, opt_args) = &concrete_ret {
                                        if opt_name == "Option" {
                                            opt_args.first().cloned()
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(Ty::Unknown)
                        } else {
                            Ty::Unknown
                        };
                        iter_ty = Some(concrete_iter_ty);
                        item_ty
                    } else {
                        Ty::Unknown
                    }
                }
                // Primitives are not Ty::Named; check for a user-defined extension impl
                // that registered iter() under the primitive's type name.
                other => {
                    let prim_name: &str = match other {
                        Ty::Int => "int",
                        Ty::Float => "float",
                        Ty::Bool => "bool",
                        _ => "",
                    };
                    if prim_name.is_empty() {
                        Ty::Unknown
                    } else if let Some(iter_method) = registry.find_method(prim_name, "iter") {
                        let it = iter_method.ret.clone();
                        let item_ty = if let Ty::Named(_, iter_name, _) = &it {
                            registry
                                .find_method(iter_name, "next")
                                .and_then(|m| {
                                    if let Ty::Named(_, opt_name, opt_args) = &m.ret {
                                        if opt_name == "Option" {
                                            opt_args.first().cloned()
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or(Ty::Unknown)
                        } else {
                            Ty::Unknown
                        };
                        iter_ty = Some(it);
                        item_ty
                    } else {
                        Ty::Unknown
                    }
                }
            };
            // Emit a compile-time error if the iterable type doesn't implement Iterable.
            // Guard on ti.ty != Unknown to avoid cascading errors from already-reported issues.
            if elem_ty == Ty::Unknown && ti.ty != Ty::Unknown {
                errors.push(AnalysisError::NotIterable {
                    ty: ti.ty.to_string(),
                    span: *span,
                });
            }
            let ann_ty = if let Some(ann) = binding_ty {
                let at = resolve_type_expr(ann, env, errors);
                check_assignable(&at, &elem_ty, span, errors);
                at
            } else {
                elem_ty.clone()
            };
            env.push_scope();
            env.define(
                binding,
                Symbol::Var {
                    ty: elem_ty,
                    mutable: false,
                    span: *span,
                },
            );
            let mut tb_stmts = Vec::new();
            for s in &body.stmts {
                tb_stmts.push(check_typed_stmt(s, env, registry, return_ty, errors));
            }
            env.pop_scope();
            let typed_body = TypedBlock {
                stmts: tb_stmts,
                span: body.span,
            };
            TypedStmt::For {
                binding: binding.clone(),
                binding_ty: ann_ty,
                iterable: ti,
                body: typed_body,
                iter_ty,
                span: *span,
            }
        }

        Stmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => {
            let typed_body = check_typed_block(body, env, registry, return_ty, errors);
            let mut typed_handlers: Vec<TypedCatchHandler> = Vec::new();
            for h in handlers {
                let exc_ty = resolve_type_expr(&h.ty, env, errors);
                env.push_scope();
                env.define(
                    &h.binding,
                    Symbol::Var {
                        ty: exc_ty.clone(),
                        mutable: false,
                        span: h.span,
                    },
                );
                let hb = check_typed_block(&h.body, env, registry, return_ty, errors);
                env.pop_scope();
                typed_handlers.push(TypedCatchHandler {
                    ty: exc_ty,
                    binding: h.binding.clone(),
                    body: hb,
                    span: h.span,
                });
            }
            let typed_finally = finally
                .as_ref()
                .map(|b| check_typed_block(b, env, registry, return_ty, errors));
            TypedStmt::TryCatch {
                body: typed_body,
                handlers: typed_handlers,
                finally: typed_finally,
                span: *span,
            }
        }

        Stmt::FnDef(f) => TypedStmt::FnDef(check_fn_def(f, env, registry, errors)),

        Stmt::Break(s) => TypedStmt::Break(*s),
        Stmt::Continue(s) => TypedStmt::Continue(*s),
    }
}

/// Check a function definition and produce a typed one.
pub fn check_fn_def(
    f: &FnDef,
    env: &mut Env,
    registry: &TypeRegistry,
    errors: &mut Vec<AnalysisError>,
) -> TypedFnDef {
    let ret = resolve_type_expr(&f.return_type, env, errors);
    let mut params: Vec<TypedParam> = Vec::new();
    for p in &f.params {
        params.push(TypedParam {
            name: p.name.clone(),
            ty: resolve_type_expr(&p.ty, env, errors),
            mutable: p.mutable,
            span: p.span,
        });
    }

    let mut generic_bounds = Vec::new();
    for g in &f.generic_params {
        for b in &g.bounds {
            if let crate::parser::ast::TypeExpr::Named {
                name: iface_name, ..
            } = b
            {
                generic_bounds.push(crate::analyzer::env::GenericBound {
                    param: g.name.clone(),
                    iface: iface_name.clone(),
                    assoc_bindings: vec![],
                    is_explicit: true,
                    decl_span: Some(g.span),
                    source_span: None,
                    source_desc: String::new(),
                });
            }
        }
    }
    env.define(
        &f.name,
        Symbol::Fn {
            generic_params: f.generic_params.iter().map(|g| g.name.clone()).collect(),
            generic_bounds,
            inferred_bounds: vec![],
            params: params
                .iter()
                .map(|p| (p.name.clone(), p.ty.clone()))
                .collect(),
            ret: ret.clone(),
            throws: f.throws,
            span: f.span,
        },
    );

    let outer_throws = env.throws_context;
    env.throws_context = f.throws;
    env.push_scope();
    for p in &params {
        env.define(
            &p.name,
            Symbol::Var {
                ty: p.ty.clone(),
                mutable: false,
                span: p.span,
            },
        );
    }
    let body = check_typed_block(&f.body, env, registry, &ret, errors);
    env.pop_scope();
    env.throws_context = outer_throws;

    TypedFnDef {
        name: f.name.clone(),
        params,
        variadic: f.variadic.as_ref().map(|v| v.name.clone()),
        return_type: ret,
        body,
        is_builtin: false,
        is_inline: f.annotations.iter().any(|a| a.name == "inline"),
        is_declaration: false,
        is_entry: f.annotations.iter().any(|a| a.name == "entry"),
        is_impure: f.annotations.iter().any(|a| a.name == "impure"),
        throws: f.throws,
        span: f.span,
    }
}

/// For invariant type parameters on `Ty::Named`, verify the assigned type's
/// corresponding argument matches exactly (e.g. rejects Mutex[Animal] = mutex_of_dog).
fn check_variance_assignment(
    expected: &Ty,
    found: &Ty,
    registry: &TypeRegistry,
    span: Span,
    errors: &mut Vec<AnalysisError>,
) {
    if let (Ty::Named(_, en, ea), Ty::Named(_, fn_, fa)) = (expected, found) {
        if en != fn_ || ea.len() != fa.len() {
            return;
        }
        for (i, (exp_arg, found_arg)) in ea.iter().zip(fa.iter()).enumerate() {
            if registry.get_type_variance(en, i) == ComputedVariance::Invariant
                && exp_arg != found_arg
                && !matches!(exp_arg, Ty::Unknown)
                && !matches!(found_arg, Ty::Unknown)
            {
                errors.push(AnalysisError::VarianceViolation {
                    container: en.clone(),
                    expected: exp_arg.to_string(),
                    found: found_arg.to_string(),
                    span,
                });
            }
        }
    }
}

/// Emit W005/W006 warnings for variables in the current scope that are never read,
/// or mutable variables that are never reassigned. Called before each scope pop.
fn emit_unused_var_warnings(env: &Env, stmts: &[TypedStmt], errors: &mut Vec<AnalysisError>) {
    let scope_vars = env.current_scope_vars();
    if scope_vars.is_empty() {
        return;
    }
    let reads = collect_ident_reads(stmts);
    let writes = collect_reassigned_names(stmts);
    for (name, is_mut, span) in scope_vars {
        if !reads.contains(&name) {
            errors.push(AnalysisError::UnusedVariable {
                name: name.clone(),
                span,
            });
        } else if is_mut && !writes.contains(&name) {
            errors.push(AnalysisError::NeedlessMut { name, span });
        }
    }
}

/// Collect identifier names read (not just assigned) within `stmts` and nested blocks.
/// Bare `Ident` assignment targets are excluded as pure writes.
fn collect_ident_reads(stmts: &[TypedStmt]) -> std::collections::HashSet<String> {
    let mut reads = std::collections::HashSet::new();
    for stmt in stmts {
        read_stmt(stmt, &mut reads);
    }
    reads
}

fn collect_reassigned_names(stmts: &[TypedStmt]) -> std::collections::HashSet<String> {
    let mut writes = std::collections::HashSet::new();
    for stmt in stmts {
        write_stmt(stmt, &mut writes);
    }
    writes
}

fn read_stmt(stmt: &TypedStmt, reads: &mut std::collections::HashSet<String>) {
    match stmt {
        TypedStmt::VarDecl { value, .. } => read_expr(&value.kind, reads),
        TypedStmt::Assign { target, value, .. } => {
            if !matches!(&target.kind, TypedExprKind::Ident(_)) {
                read_expr(&target.kind, reads);
            }
            read_expr(&value.kind, reads);
        }
        TypedStmt::CompoundAssign { target, rhs, .. } => {
            read_expr(&target.kind, reads);
            read_expr(&rhs.kind, reads);
        }
        TypedStmt::Return { value, .. } => {
            if let Some(v) = value {
                read_expr(&v.kind, reads);
            }
        }
        TypedStmt::Raise { value, .. } | TypedStmt::Abandon { message: value, .. } => {
            if let Some(v) = value {
                read_expr(&v.kind, reads);
            }
        }
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (cond, body) in branches {
                read_expr(&cond.kind, reads);
                read_block(body, reads);
            }
            if let Some(eb) = else_branch {
                read_block(eb, reads);
            }
        }
        TypedStmt::While { cond, body, .. } => {
            read_expr(&cond.kind, reads);
            read_block(body, reads);
        }
        TypedStmt::DoWhile { body, cond, .. } => {
            read_block(body, reads);
            read_expr(&cond.kind, reads);
        }
        TypedStmt::For { iterable, body, .. } => {
            read_expr(&iterable.kind, reads);
            read_block(body, reads);
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            read_block(body, reads);
            for h in handlers {
                read_block(&h.body, reads);
            }
            if let Some(f) = finally {
                read_block(f, reads);
            }
        }
        TypedStmt::FnDef(_) => {}
        TypedStmt::Expr(e) => read_expr(&e.kind, reads),
        TypedStmt::Break(_) | TypedStmt::Continue(_) => {}
    }
}

fn read_block(block: &TypedBlock, reads: &mut std::collections::HashSet<String>) {
    for stmt in &block.stmts {
        read_stmt(stmt, reads);
    }
}

fn read_expr(kind: &TypedExprKind, reads: &mut std::collections::HashSet<String>) {
    match kind {
        TypedExprKind::Ident(name) => {
            reads.insert(name.clone());
        }
        TypedExprKind::Call { callee, args, .. } => {
            read_expr(&callee.kind, reads);
            for a in args {
                read_expr(&a.kind, reads);
            }
        }
        TypedExprKind::MethodCall { object, args, .. } => {
            read_expr(&object.kind, reads);
            for a in args {
                read_expr(&a.kind, reads);
            }
        }
        TypedExprKind::StaticCall { args, .. } => {
            for a in args {
                read_expr(&a.kind, reads);
            }
        }
        TypedExprKind::IndirectCall { fat_ptr, args } => {
            read_expr(&fat_ptr.kind, reads);
            for a in args {
                read_expr(&a.kind, reads);
            }
        }
        TypedExprKind::Field { object, .. } => read_expr(&object.kind, reads),
        TypedExprKind::Index { object, index } => {
            read_expr(&object.kind, reads);
            read_expr(&index.kind, reads);
        }
        TypedExprKind::BinOp { left, right, .. } => {
            read_expr(&left.kind, reads);
            read_expr(&right.kind, reads);
        }
        TypedExprKind::UnOp { operand, .. } => read_expr(&operand.kind, reads),
        TypedExprKind::Unwrap(e) => read_expr(&e.kind, reads),
        TypedExprKind::As { expr, .. } => read_expr(&expr.kind, reads),
        TypedExprKind::Tuple(es) | TypedExprKind::Array(es) => {
            for e in es {
                read_expr(&e.kind, reads);
            }
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, e) in fields {
                read_expr(&e.kind, reads);
            }
        }
        TypedExprKind::Match { scrutinee, arms } => {
            read_expr(&scrutinee.kind, reads);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    read_expr(&guard.kind, reads);
                }
                read_expr(&arm.body.kind, reads);
            }
        }
        TypedExprKind::Closure { body, .. } => match body {
            TypedClosureBody::Expr(e) => read_expr(&e.kind, reads),
            TypedClosureBody::Block(b) => read_block(b, reads),
        },
        TypedExprKind::Spawn(e) | TypedExprKind::Try(e) | TypedExprKind::Ignore(e) => {
            read_expr(&e.kind, reads)
        }
        TypedExprKind::Implements { expr, .. } => read_expr(&expr.kind, reads),
        TypedExprKind::Ref { expr, .. } => read_expr(&expr.kind, reads),
        TypedExprKind::Gen { body } => read_block(body, reads),
        TypedExprKind::GenSplice(e) => read_expr(&e.kind, reads),
        TypedExprKind::Block(stmts) => {
            for stmt in stmts {
                read_stmt(stmt, reads);
            }
        }
        TypedExprKind::Str(segs) => {
            for seg in segs {
                if let TypedStringSegment::Interp(e) = seg {
                    read_expr(&e.kind, reads);
                }
            }
        }
        TypedExprKind::BoundMethod { object, .. } => read_expr(&object.kind, reads),
        TypedExprKind::PrimTypeRef { .. } => {}
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::EnumVariant { .. } => {}
    }
}

fn write_stmt(stmt: &TypedStmt, writes: &mut std::collections::HashSet<String>) {
    match stmt {
        TypedStmt::Assign { target, .. } => {
            if let TypedExprKind::Ident(name) = &target.kind {
                writes.insert(name.clone());
            }
        }
        TypedStmt::CompoundAssign { target, .. } => {
            if let TypedExprKind::Ident(name) = &target.kind {
                writes.insert(name.clone());
            }
        }
        TypedStmt::If {
            branches,
            else_branch,
            ..
        } => {
            for (_, body) in branches {
                for s in &body.stmts {
                    write_stmt(s, writes);
                }
            }
            if let Some(eb) = else_branch {
                for s in &eb.stmts {
                    write_stmt(s, writes);
                }
            }
        }
        TypedStmt::While { body, .. } | TypedStmt::DoWhile { body, .. } => {
            for s in &body.stmts {
                write_stmt(s, writes);
            }
        }
        TypedStmt::For { body, .. } => {
            for s in &body.stmts {
                write_stmt(s, writes);
            }
        }
        TypedStmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            for s in &body.stmts {
                write_stmt(s, writes);
            }
            for h in handlers {
                for s in &h.body.stmts {
                    write_stmt(s, writes);
                }
            }
            if let Some(f) = finally {
                for s in &f.stmts {
                    write_stmt(s, writes);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::env::Env;
    use crate::analyzer::ty::{Ty, TypeRegistry};
    use crate::diagnostics::Span;
    use crate::parser::ast::*;

    fn s() -> Span {
        Span { start: 0, end: 0 }
    }
    fn int_ty() -> TypeExpr {
        TypeExpr::Named {
            name: "int".into(),
            generics: vec![],
            bindings: vec![],
            span: s(),
        }
    }
    fn bool_ty() -> TypeExpr {
        TypeExpr::Named {
            name: "bool".into(),
            generics: vec![],
            bindings: vec![],
            span: s(),
        }
    }

    #[test]
    fn var_decl_ok() {
        let block = Block {
            stmts: vec![Stmt::VarDecl {
                name: "_x".into(),
                ty: int_ty(),
                value: Expr::Int(42, s()),
                mutable: false,
                span: s(),
            }],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn var_decl_type_mismatch() {
        let block = Block {
            stmts: vec![Stmt::VarDecl {
                name: "_x".into(),
                ty: bool_ty(),
                value: Expr::Int(1, s()),
                mutable: false,
                span: s(),
            }],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            errs.iter()
                .any(|e| matches!(e, AnalysisError::TypeMismatch { .. })),
            "{errs:?}"
        );
    }

    #[test]
    fn assign_to_immutable_is_error() {
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_ty(),
                    value: Expr::Int(1, s()),
                    mutable: false,
                    span: s(),
                },
                Stmt::Assign {
                    target: Expr::Ident("x".into(), s()),
                    value: Expr::Int(2, s()),
                    span: s(),
                },
            ],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            errs.iter()
                .any(|e| matches!(e, AnalysisError::AssignToImmutable { .. })),
            "{errs:?}"
        );
    }

    #[test]
    fn assign_to_mut_is_ok() {
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_ty(),
                    value: Expr::Int(1, s()),
                    mutable: true,
                    span: s(),
                },
                Stmt::Assign {
                    target: Expr::Ident("x".into(), s()),
                    value: Expr::Int(2, s()),
                    span: s(),
                },
            ],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            !errs
                .iter()
                .any(|e| matches!(e, AnalysisError::AssignToImmutable { .. })),
            "{errs:?}"
        );
    }

    // -- Unused-variable warnings (W005, W006) ----------------------------------

    #[test]
    fn unused_variable_produces_warning() {
        let block = Block {
            stmts: vec![Stmt::VarDecl {
                name: "x".into(),
                ty: int_ty(),
                value: Expr::Int(1, s()),
                mutable: false,
                span: s(),
            }],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            errs.iter()
                .any(|e| matches!(e, AnalysisError::UnusedVariable { name, .. } if name == "x")),
            "{errs:?}"
        );
    }

    #[test]
    fn used_variable_produces_no_warning() {
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_ty(),
                    value: Expr::Int(1, s()),
                    mutable: false,
                    span: s(),
                },
                Stmt::Return {
                    value: Some(Expr::Ident("x".into(), s())),
                    span: s(),
                },
            ],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Int, &mut errs);
        assert!(
            !errs
                .iter()
                .any(|e| matches!(e, AnalysisError::UnusedVariable { .. })),
            "{errs:?}"
        );
    }

    #[test]
    fn underscore_prefix_suppresses_warning() {
        let block = Block {
            stmts: vec![Stmt::VarDecl {
                name: "_unused".into(),
                ty: int_ty(),
                value: Expr::Int(1, s()),
                mutable: false,
                span: s(),
            }],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            !errs
                .iter()
                .any(|e| matches!(e, AnalysisError::UnusedVariable { .. })),
            "{errs:?}"
        );
    }

    #[test]
    fn unused_mut_produces_needless_mut_warning() {
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_ty(),
                    value: Expr::Int(1, s()),
                    mutable: true,
                    span: s(),
                },
                Stmt::Return {
                    value: Some(Expr::Ident("x".into(), s())),
                    span: s(),
                },
            ],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Int, &mut errs);
        assert!(
            errs.iter()
                .any(|e| matches!(e, AnalysisError::NeedlessMut { name, .. } if name == "x")),
            "{errs:?}"
        );
    }

    #[test]
    fn variable_used_only_in_assignment_lhs_is_unused() {
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_ty(),
                    value: Expr::Int(1, s()),
                    mutable: true,
                    span: s(),
                },
                Stmt::Assign {
                    target: Expr::Ident("x".into(), s()),
                    value: Expr::Int(2, s()),
                    span: s(),
                },
            ],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            errs.iter()
                .any(|e| matches!(e, AnalysisError::UnusedVariable { name, .. } if name == "x")),
            "{errs:?}"
        );
    }

    // ----- multi-span diagnostic tests (item 25) ----

    /// TypeMismatch with a decl_span produces non-empty note_info().
    #[test]
    fn type_mismatch_with_decl_span_produces_note() {
        use crate::analyzer::error::AnalysisError;
        let decl = Span { start: 1, end: 5 };
        let err = AnalysisError::TypeMismatch {
            expected: "int".into(),
            found: "str".into(),
            span: Span { start: 10, end: 15 },
            decl_span: Some(decl),
        };
        let notes = err.note_info();
        assert!(
            !notes.is_empty(),
            "expected notes for TypeMismatch with decl_span"
        );
        assert_eq!(notes[0].0, "expected type declared here");
        assert_eq!(notes[0].1, Some(decl));
    }

    /// TypeMismatch without a decl_span produces empty note_info().
    #[test]
    fn type_mismatch_without_decl_span_produces_no_note() {
        use crate::analyzer::error::AnalysisError;
        let err = AnalysisError::TypeMismatch {
            expected: "int".into(),
            found: "str".into(),
            span: Span { start: 10, end: 15 },
            decl_span: None,
        };
        let notes = err.note_info();
        assert!(
            notes.is_empty(),
            "expected no notes for TypeMismatch without decl_span"
        );
    }

    /// ArityMismatch with a fn_span produces non-empty note_info().
    #[test]
    fn arity_mismatch_with_fn_span_produces_note() {
        use crate::analyzer::error::AnalysisError;
        let fn_span = Span { start: 20, end: 30 };
        let err = AnalysisError::ArityMismatch {
            expected: 2,
            found: 3,
            span: Span { start: 40, end: 50 },
            fn_span: Some(fn_span),
        };
        let notes = err.note_info();
        assert!(
            !notes.is_empty(),
            "expected notes for ArityMismatch with fn_span"
        );
        assert_eq!(notes[0].0, "function defined here");
        assert_eq!(notes[0].1, Some(fn_span));
    }

    /// ArityMismatch without a fn_span produces empty note_info().
    #[test]
    fn arity_mismatch_without_fn_span_produces_no_note() {
        use crate::analyzer::error::AnalysisError;
        let err = AnalysisError::ArityMismatch {
            expected: 2,
            found: 3,
            span: Span { start: 40, end: 50 },
            fn_span: None,
        };
        let notes = err.note_info();
        assert!(
            notes.is_empty(),
            "expected no notes for ArityMismatch without fn_span"
        );
    }

    /// MissingConformance with an iface_span produces non-empty note_info().
    #[test]
    fn missing_conformance_with_iface_span_produces_note() {
        use crate::analyzer::error::AnalysisError;
        let iface_span = Span { start: 5, end: 10 };
        let err = AnalysisError::MissingConformance {
            ty: "MyStruct".into(),
            iface: "Addable".into(),
            detail: "missing required hook `+`".into(),
            span: Span { start: 50, end: 60 },
            iface_span: Some(iface_span),
        };
        let notes = err.note_info();
        assert!(
            !notes.is_empty(),
            "expected notes for MissingConformance with iface_span"
        );
        assert_eq!(notes[0].0, "interface required here");
        assert_eq!(notes[0].1, Some(iface_span));
    }

    /// MissingConformance without an iface_span produces empty note_info().
    #[test]
    fn missing_conformance_without_iface_span_produces_no_note() {
        use crate::analyzer::error::AnalysisError;
        let err = AnalysisError::MissingConformance {
            ty: "MyStruct".into(),
            iface: "Addable".into(),
            detail: "missing required hook `+`".into(),
            span: Span { start: 50, end: 60 },
            iface_span: None,
        };
        let notes = err.note_info();
        assert!(
            notes.is_empty(),
            "expected no notes for MissingConformance without iface_span"
        );
    }

    /// VarDecl with a type mismatch threads the declaration span into the error.
    #[test]
    fn var_decl_type_mismatch_includes_decl_span() {
        let decl_span = Span { start: 0, end: 9 };
        let val_span = Span { start: 12, end: 15 };
        let block = Block {
            stmts: vec![Stmt::VarDecl {
                name: "_x".into(),
                ty: bool_ty(),
                value: Expr::Int(1, val_span),
                mutable: false,
                span: decl_span,
            }],
            span: Span { start: 0, end: 20 },
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        let mismatch = errs
            .iter()
            .find(|e| matches!(e, AnalysisError::TypeMismatch { .. }))
            .expect("expected a TypeMismatch error");
        match mismatch {
            AnalysisError::TypeMismatch { decl_span: ds, .. } => {
                assert_eq!(*ds, Some(decl_span), "decl_span should be the VarDecl span");
            }
            _ => panic!("expected TypeMismatch"),
        }
    }

    /// Assigning to an immutable variable threads the declaration span into the error.
    #[test]
    fn assign_to_immutable_includes_decl_span() {
        let decl_span = Span { start: 0, end: 9 };
        let assign_span = Span { start: 20, end: 25 };
        let block = Block {
            stmts: vec![
                Stmt::VarDecl {
                    name: "x".into(),
                    ty: int_ty(),
                    value: Expr::Int(1, s()),
                    mutable: false,
                    span: decl_span,
                },
                Stmt::Assign {
                    target: Expr::Ident("x".into(), assign_span),
                    value: Expr::Int(2, s()),
                    span: assign_span,
                },
            ],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        let err = errs
            .iter()
            .find(|e| matches!(e, AnalysisError::AssignToImmutable { .. }))
            .expect("expected AssignToImmutable error");
        let notes = err.note_info();
        assert!(!notes.is_empty(), "expected a note pointing at declaration");
        assert_eq!(
            notes[0].1,
            Some(decl_span),
            "note should point at decl_span"
        );
    }

    // ----- unreachable code warning tests (item 28) ----

    fn return_stmt(span: Span) -> Stmt {
        Stmt::Return { value: None, span }
    }

    fn break_stmt(span: Span) -> Stmt {
        Stmt::Break(span)
    }

    fn raise_stmt(span: Span) -> Stmt {
        Stmt::Raise {
            value: Some(Expr::Int(1, span)),
            span,
        }
    }

    fn expr_stmt(span: Span) -> Stmt {
        Stmt::Expr(Expr::Int(0, span))
    }

    #[test]
    fn return_followed_by_stmt_produces_unreachable_warning() {
        let ret_span = Span { start: 0, end: 8 };
        let dead_span = Span { start: 10, end: 20 };
        let block = Block {
            stmts: vec![return_stmt(ret_span), expr_stmt(dead_span)],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        let warn = errs
            .iter()
            .find(|e| matches!(e, AnalysisError::UnreachableCode { .. }))
            .expect("expected UnreachableCode warning");
        match warn {
            AnalysisError::UnreachableCode {
                span,
                terminator_span,
            } => {
                assert_eq!(*span, dead_span, "span should point at dead statement");
                assert_eq!(
                    *terminator_span, ret_span,
                    "terminator_span should point at return"
                );
            }
            _ => panic!("expected UnreachableCode"),
        }
    }

    #[test]
    fn break_followed_by_stmt_produces_unreachable_warning() {
        let break_span = Span { start: 0, end: 5 };
        let dead_span = Span { start: 10, end: 20 };
        let block = Block {
            stmts: vec![break_stmt(break_span), expr_stmt(dead_span)],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            errs.iter()
                .any(|e| matches!(e, AnalysisError::UnreachableCode { .. })),
            "expected UnreachableCode warning after break: {errs:?}"
        );
    }

    #[test]
    fn raise_followed_by_stmt_produces_unreachable_warning() {
        let raise_span = Span { start: 0, end: 7 };
        let dead_span = Span { start: 10, end: 20 };
        let block = Block {
            stmts: vec![raise_stmt(raise_span), expr_stmt(dead_span)],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            errs.iter()
                .any(|e| matches!(e, AnalysisError::UnreachableCode { .. })),
            "expected UnreachableCode warning after raise: {errs:?}"
        );
    }

    #[test]
    fn conditional_return_does_not_produce_unreachable_warning() {
        // Only a return inside an if-branch; the statement after the if is reachable.
        let block = Block {
            stmts: vec![
                Stmt::If {
                    branches: vec![(
                        Expr::Bool(true, s()),
                        Block {
                            stmts: vec![return_stmt(s())],
                            span: s(),
                        },
                    )],
                    else_branch: None,
                    span: s(),
                },
                expr_stmt(s()),
            ],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            !errs
                .iter()
                .any(|e| matches!(e, AnalysisError::UnreachableCode { .. })),
            "should not warn for conditional return: {errs:?}"
        );
    }

    #[test]
    fn only_first_unreachable_stmt_is_warned() {
        let ret_span = Span { start: 0, end: 8 };
        let block = Block {
            stmts: vec![
                return_stmt(ret_span),
                expr_stmt(Span { start: 10, end: 20 }),
                expr_stmt(Span { start: 25, end: 35 }),
                expr_stmt(Span { start: 40, end: 50 }),
            ],
            span: s(),
        };
        let mut env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        let count = errs
            .iter()
            .filter(|e| matches!(e, AnalysisError::UnreachableCode { .. }))
            .count();
        assert_eq!(
            count, 1,
            "should emit exactly one unreachable warning, got {count}"
        );
    }

    fn call_throws_fn_block() -> Block {
        Block {
            stmts: vec![Stmt::Expr(Expr::Call {
                callee: Box::new(Expr::Ident("risky".into(), s())),
                args: vec![],
                span: s(),
            })],
            span: s(),
        }
    }

    fn throws_fn_symbol() -> Symbol {
        Symbol::Fn {
            generic_params: vec![],
            generic_bounds: vec![],
            inferred_bounds: vec![],
            params: vec![],
            ret: Ty::Void,
            throws: true,
            span: s(),
        }
    }

    #[test]
    fn try_expr_allows_throws_fn_in_clean_context() {
        // try f() in a clean context should NOT produce ThrowsInCleanContext
        let block = Block {
            stmts: vec![Stmt::Expr(Expr::Try(
                Box::new(Expr::Call {
                    callee: Box::new(Expr::Ident("risky".into(), s())),
                    args: vec![],
                    span: s(),
                }),
                s(),
            ))],
            span: s(),
        };
        let mut env = Env::new();
        env.push_scope();
        env.define("risky", throws_fn_symbol());
        env.throws_context = false;
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            !errs
                .iter()
                .any(|e| matches!(e, AnalysisError::ThrowsInCleanContext { .. })),
            "try should suppress ThrowsInCleanContext: {errs:?}"
        );
    }

    #[test]
    fn call_throws_fn_in_clean_context_errors() {
        let block = call_throws_fn_block();
        let mut env = Env::new();
        env.push_scope();
        env.define("risky", throws_fn_symbol());
        env.throws_context = false;
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            errs.iter()
                .any(|e| matches!(e, AnalysisError::ThrowsInCleanContext { .. })),
            "expected ThrowsInCleanContext error: {errs:?}"
        );
    }

    #[test]
    fn call_throws_fn_in_throws_context_ok() {
        let block = call_throws_fn_block();
        let mut env = Env::new();
        env.push_scope();
        env.define("risky", throws_fn_symbol());
        env.throws_context = true;
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            !errs
                .iter()
                .any(|e| matches!(e, AnalysisError::ThrowsInCleanContext { .. })),
            "should not error when calling throws fn from throws context: {errs:?}"
        );
    }

    #[test]
    fn abandon_is_definite_terminator() {
        use crate::analyzer::typed_ast::TypedStmt;
        use crate::diagnostics::span::Span;
        let stmt = TypedStmt::Abandon {
            message: None,
            span: Span::new(0, 0),
        };
        assert!(is_definite_terminator(&stmt));
    }

    #[test]
    fn abandon_no_message_infers_ok() {
        let block = Block {
            stmts: vec![Stmt::Abandon {
                message: None,
                span: s(),
            }],
            span: s(),
        };
        let mut env = Env::new();
        env.push_scope();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            errs.is_empty(),
            "abandon should not produce errors: {errs:?}"
        );
    }

    #[test]
    fn abandon_with_message_infers_ok() {
        let block = Block {
            stmts: vec![Stmt::Abandon {
                message: Some(Expr::Int(42, s())),
                span: s(),
            }],
            span: s(),
        };
        let mut env = Env::new();
        env.push_scope();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        check_typed_block(&block, &mut env, &reg, &Ty::Void, &mut errs);
        assert!(
            errs.is_empty(),
            "abandon with message should not produce errors: {errs:?}"
        );
    }
}
