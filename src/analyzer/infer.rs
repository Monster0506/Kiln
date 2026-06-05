use crate::analyzer::env::{Env, FnOverload, Symbol};
use crate::analyzer::error::AnalysisError;
use crate::analyzer::resolve::resolve_type_expr;
use crate::analyzer::ty::{Ty, TypeRegistry};
use crate::analyzer::typed_ast::{
    TypedClosureBody, TypedExpr, TypedExprKind, TypedMatchArm, TypedParam, TypedPattern,
    TypedStringSegment,
};
use crate::diagnostics::Span;
use crate::parser::ast::{BinOp, ClosureBody, Expr, Pattern, StringSegment, UnOp};

/// Infer the type of `expr` and produce a fully-annotated `TypedExpr`.
pub fn infer_typed_expr(
    expr: &Expr,
    env: &Env,
    registry: &TypeRegistry,
    errors: &mut Vec<AnalysisError>,
) -> TypedExpr {
    let span = expr.span();
    match expr {
        Expr::Int(n, _) => mk(TypedExprKind::Int(*n), Ty::Int, span),
        Expr::Float(f, _) => mk(TypedExprKind::Float(*f), Ty::Float, span),
        Expr::Bool(b, _) => mk(TypedExprKind::Bool(*b), Ty::Bool, span),

        Expr::Str(segs, _) => {
            let typed_segs = segs
                .iter()
                .map(|s| match s {
                    StringSegment::Text(t) => TypedStringSegment::Text(t.clone()),
                    StringSegment::Interp(e) => {
                        TypedStringSegment::Interp(infer_typed_expr(e, env, registry, errors))
                    }
                })
                .collect();
            mk(TypedExprKind::Str(typed_segs), Ty::Str, span)
        }

        Expr::Ident(name, _) => {
            let ty = match env.lookup(name) {
                Some(Symbol::Var { ty, .. }) => ty.clone(),
                Some(Symbol::Fn { params, ret, .. }) => Ty::Callable(
                    params.iter().map(|(_, t)| t.clone()).collect(),
                    Box::new(ret.clone()),
                ),
                Some(Symbol::FnOverloadSet { .. }) => {
                    // Overloaded function used as first-class value -- ambiguous without a call.
                    Ty::Unknown
                }
                Some(Symbol::Type { id, .. }) => Ty::Named(id.clone(), name.clone(), vec![]),
                Some(Symbol::StructField { ty }) => {
                    let ty = ty.clone();
                    errors.push(AnalysisError::BareFieldAccess {
                        field: name.clone(),
                        span,
                    });
                    ty
                }
                // Inline const value at use site.
                Some(Symbol::Const { ty, value, .. }) => {
                    let ty = ty.clone();
                    let value = value.clone();
                    return mk(value, ty, span);
                }
                _ => {
                    let did_you_mean = crate::diagnostics::suggest::closest_match(
                        name,
                        env.all_names().into_iter(),
                    )
                    .map(|s| {
                        let decl_span = env.span_of(&s);
                        (s, decl_span)
                    });
                    errors.push(AnalysisError::UndefinedName {
                        name: name.clone(),
                        span,
                        did_you_mean,
                    });
                    Ty::Unknown
                }
            };
            mk(TypedExprKind::Ident(name.clone()), ty, span)
        }

        Expr::EnumAccess {
            enum_name,
            variant,
            span: espan,
        } => {
            let (ty, discriminant) = match registry.lookup_by_name(enum_name) {
                Some(entry) => match &entry.kind {
                    crate::analyzer::ty::TypeKind::Enum { variant_names } => {
                        if let Some(idx) = variant_names.iter().position(|v| v == variant) {
                            (
                                Ty::Named(entry.id.clone(), enum_name.clone(), vec![]),
                                idx as i64,
                            )
                        } else {
                            errors.push(AnalysisError::UndefinedName {
                                name: format!("{enum_name}:{variant}"),
                                span: *espan,
                                did_you_mean: None,
                            });
                            (Ty::Unknown, 0)
                        }
                    }
                    _ => {
                        errors.push(AnalysisError::TypeMismatch {
                            expected: "enum".into(),
                            found: enum_name.clone(),
                            span: *espan,
                            decl_span: None,
                        });
                        (Ty::Unknown, 0)
                    }
                },
                None => {
                    errors.push(AnalysisError::UndefinedName {
                        name: enum_name.clone(),
                        span: *espan,
                        did_you_mean: None,
                    });
                    (Ty::Unknown, 0)
                }
            };
            mk(
                TypedExprKind::EnumVariant {
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                    discriminant,
                },
                ty,
                *espan,
            )
        }

        Expr::Tuple(elems, _) => {
            let typed: Vec<TypedExpr> = elems
                .iter()
                .map(|e| infer_typed_expr(e, env, registry, errors))
                .collect();
            let ty = Ty::Tuple(typed.iter().map(|e| e.ty.clone()).collect());
            mk(TypedExprKind::Tuple(typed), ty, span)
        }

        Expr::BinOp {
            op,
            left,
            right,
            span: s,
        } => {
            let tl = infer_typed_expr(left, env, registry, errors);
            let tr = infer_typed_expr(right, env, registry, errors);
            let ty = infer_binop(
                op.clone(),
                tl.ty.clone(),
                tr.ty.clone(),
                s,
                errors,
                registry,
            );
            mk(
                TypedExprKind::BinOp {
                    op: op.clone(),
                    left: Box::new(tl),
                    right: Box::new(tr),
                },
                ty,
                span,
            )
        }

        Expr::UnOp {
            op,
            operand,
            span: s,
        } => {
            let to = infer_typed_expr(operand, env, registry, errors);
            let ty = match op {
                UnOp::Neg => {
                    match &to.ty {
                        Ty::Int | Ty::Float | Ty::Unknown => to.ty.clone(),
                        // Any named type may implement Negatable via a hook.
                        t if type_name_of(t).is_some() => Ty::Unknown,
                        _ => {
                            errors.push(AnalysisError::TypeMismatch {
                                expected: "int, float, or Negatable".into(),
                                found: to.ty.to_string(),
                                span: *s,
                                decl_span: None,
                            });
                            Ty::Unknown
                        }
                    }
                }
                UnOp::Not => {
                    match &to.ty {
                        Ty::Bool | Ty::Unknown => Ty::Bool,
                        // Any named type may implement a bitwise-not hook.
                        t if type_name_of(t).is_some() => Ty::Unknown,
                        _ => {
                            errors.push(AnalysisError::TypeMismatch {
                                expected: "bool or hook !".into(),
                                found: to.ty.to_string(),
                                span: *s,
                                decl_span: None,
                            });
                            Ty::Bool
                        }
                    }
                }
                UnOp::Pos => {
                    match &to.ty {
                        Ty::Int | Ty::Float | Ty::Unknown => to.ty.clone(),
                        // Any named type may implement a pos hook.
                        t if type_name_of(t).is_some() => Ty::Unknown,
                        _ => {
                            errors.push(AnalysisError::TypeMismatch {
                                expected: "numeric or hook +".into(),
                                found: to.ty.to_string(),
                                span: *s,
                                decl_span: None,
                            });
                            Ty::Unknown
                        }
                    }
                }
            };
            mk(
                TypedExprKind::UnOp {
                    op: op.clone(),
                    operand: Box::new(to),
                },
                ty,
                span,
            )
        }

        Expr::As { expr, ty, .. } => {
            let te = infer_typed_expr(expr, env, registry, errors);
            let target = resolve_type_expr(ty, env, errors);
            mk(
                TypedExprKind::As {
                    expr: Box::new(te),
                    ty: target.clone(),
                },
                target,
                span,
            )
        }

        Expr::Unwrap(inner, s) => {
            let ti = infer_typed_expr(inner, env, registry, errors);
            let ty = match &ti.ty {
                Ty::Named(_, name, args) if !registry.get_conformances(name, "Try").is_empty() => {
                    args.first().cloned().unwrap_or(Ty::Unknown)
                }
                Ty::Unknown => Ty::Unknown,
                other => {
                    errors.push(AnalysisError::TypeMismatch {
                        expected: "a type implementing Try".into(),
                        found: other.to_string(),
                        span: *s,
                        decl_span: None,
                    });
                    Ty::Unknown
                }
            };
            mk(TypedExprKind::Unwrap(Box::new(ti)), ty, span)
        }

        Expr::Call {
            callee,
            args,
            span: s,
        } => {
            let typed_args: Vec<TypedExpr> = args
                .iter()
                .map(|a| infer_typed_expr(a, env, registry, errors))
                .collect();

            match callee.as_ref() {
                Expr::Field { object, field, .. } => {
                    infer_call_field(object, field, typed_args, env, registry, errors, *s)
                }
                Expr::Ident(name, _) => {
                    infer_call_ident(name, typed_args, env, registry, errors, *s)
                }
                _ => {
                    let tc = infer_typed_expr(callee, env, registry, errors);
                    let ret = match &tc.ty {
                        Ty::Callable(_, r) => *r.clone(),
                        _ => Ty::Unknown,
                    };
                    mk(
                        TypedExprKind::IndirectCall {
                            fat_ptr: Box::new(tc),
                            args: typed_args,
                        },
                        ret,
                        span,
                    )
                }
            }
        }

        Expr::Field {
            object,
            field,
            span: fspan,
        } => {
            let to = infer_typed_expr(object, env, registry, errors);
            let effective_ty = match &to.ty {
                Ty::Ref(inner, _) => (**inner).clone(),
                other => other.clone(),
            };
            let field_ty = resolve_field_ty(&effective_ty, field, registry);
            let is_annot_args = matches!(&effective_ty, Ty::Named(_, n, _) if n == "AnnotArgs");
            if field_ty == Ty::Unknown && effective_ty != Ty::Unknown && !is_annot_args {
                errors.push(AnalysisError::NoField {
                    ty: effective_ty.to_string(),
                    field: field.clone(),
                    span: *fspan,
                });
            }
            mk(
                TypedExprKind::Field {
                    object: Box::new(to),
                    field: field.clone(),
                },
                field_ty,
                span,
            )
        }

        Expr::Index { object, index, .. } => {
            let to = infer_typed_expr(object, env, registry, errors);
            let ti = infer_typed_expr(index, env, registry, errors);
            let elem_ty = match &to.ty {
                Ty::Named(_, name, args) if name == "Vec" => {
                    args.first().cloned().unwrap_or(Ty::Unknown)
                }
                Ty::Named(_, name, args) if name == "Map" => {
                    args.get(1).cloned().unwrap_or(Ty::Unknown)
                }
                _ => Ty::Unknown,
            };
            mk(
                TypedExprKind::Index {
                    object: Box::new(to),
                    index: Box::new(ti),
                },
                elem_ty,
                span,
            )
        }

        Expr::StructLiteral {
            ty,
            fields,
            span: s,
        } => {
            let (struct_ty, concrete_ty_name) = match env.lookup(ty) {
                Some(Symbol::TypeAlias(alias_ty)) => {
                    let name = match alias_ty {
                        Ty::Named(_, n, _) => n.clone(),
                        _ => ty.clone(),
                    };
                    (alias_ty.clone(), name)
                }
                Some(Symbol::Type { id, .. }) => {
                    (Ty::Named(id.clone(), ty.clone(), vec![]), ty.clone())
                }
                _ => {
                    // Check if ty is an enum variant name (e.g. "Some" from Option:Some { ... }).
                    if let Some(entry) = registry.enum_for_variant(ty) {
                        let enum_ty = Ty::Named(entry.id.clone(), entry.name.clone(), vec![]);
                        (enum_ty, ty.clone())
                    } else {
                        errors.push(AnalysisError::UndefinedName {
                            name: ty.clone(),
                            span: *s,
                            did_you_mean: None,
                        });
                        (Ty::Unknown, ty.clone())
                    }
                }
            };
            let known_fields: Option<Vec<String>> = registry
                .get_struct_fields(&concrete_ty_name)
                .map(|fs| fs.iter().map(|(n, _)| n.clone()).collect());
            let typed_fields = fields
                .iter()
                .map(|(fname, expr)| {
                    if let Some(ref kf) = known_fields {
                        if !kf.iter().any(|n| n == fname) {
                            errors.push(AnalysisError::NoField {
                                ty: concrete_ty_name.clone(),
                                field: fname.clone(),
                                span,
                            });
                        }
                    }
                    (fname.clone(), infer_typed_expr(expr, env, registry, errors))
                })
                .collect();
            mk(
                TypedExprKind::StructLiteral {
                    ty_name: concrete_ty_name,
                    fields: typed_fields,
                },
                struct_ty,
                span,
            )
        }

        Expr::Match {
            scrutinee, arms, ..
        } => {
            let ts = infer_typed_expr(scrutinee, env, registry, errors);
            let typed_arms: Vec<TypedMatchArm> = arms
                .iter()
                .map(|arm| {
                    // Clone env and add struct/variant pattern field bindings for the arm body.
                    let mut arm_env = env.clone();
                    if let Pattern::Struct {
                        variant, fields, ..
                    } = &arm.pattern
                    {
                        arm_env.push_scope();
                        // Build substitution from the scrutinee's concrete type args so
                        // that match-bound vars like `v` in `Some { value: v }` get type
                        // `int` instead of `Unknown` when the scrutinee is `Option[int]`.
                        let subst: std::collections::HashMap<String, Ty> = {
                            let mut m = std::collections::HashMap::new();
                            if let Ty::Named(_, _, concrete_args) = &ts.ty {
                                if !concrete_args.is_empty() {
                                    let enum_name = registry
                                        .enum_for_variant(variant)
                                        .map(|e| e.name.clone())
                                        .or_else(|| {
                                            if let Ty::Named(_, n, _) = &ts.ty {
                                                Some(n.clone())
                                            } else {
                                                None
                                            }
                                        });
                                    if let Some(ename) = enum_name {
                                        if let Some(params) =
                                            registry.get_generic_param_order(&ename)
                                        {
                                            for (pname, arg) in
                                                params.iter().zip(concrete_args.iter())
                                            {
                                                m.insert(pname.clone(), arg.clone());
                                            }
                                        }
                                    }
                                }
                            }
                            m
                        };
                        for (field_name, binding_name) in fields {
                            if binding_name != "_" {
                                let raw_ty = registry
                                    .get_struct_fields(variant)
                                    .and_then(|fs| {
                                        fs.iter()
                                            .find(|(n, _)| n == field_name)
                                            .map(|(_, t)| t.clone())
                                    })
                                    .unwrap_or(Ty::Unknown);
                                let field_ty = if subst.is_empty() {
                                    raw_ty
                                } else {
                                    subst_generic_ty(&raw_ty, &subst)
                                };
                                arm_env.define(
                                    binding_name,
                                    Symbol::Var {
                                        ty: field_ty,
                                        mutable: false,
                                        span: arm.span,
                                    },
                                );
                            }
                        }
                    }
                    if let Pattern::TypeBinding { name, ty, .. } = &arm.pattern {
                        if name != "_" && ty == "_" && !registry.is_enum_variant(name) {
                            arm_env.push_scope();
                            arm_env.define(
                                name,
                                Symbol::Var {
                                    ty: Ty::Unknown,
                                    mutable: false,
                                    span: arm.span,
                                },
                            );
                        }
                    }
                    let body = infer_typed_expr(&arm.body, &arm_env, registry, errors);
                    let guard = arm
                        .guard
                        .as_ref()
                        .map(|g| infer_typed_expr(g, &arm_env, registry, errors));
                    TypedMatchArm {
                        pattern: lower_pattern(&arm.pattern, env, registry, errors),
                        guard,
                        body,
                        narrowed_discriminant: None,
                        span: arm.span,
                    }
                })
                .collect();
            let ty = typed_arms
                .iter()
                .map(|a| a.body.ty.clone())
                .find(|t| *t != Ty::Unknown)
                .unwrap_or(Ty::Unknown);
            mk(
                TypedExprKind::Match {
                    scrutinee: Box::new(ts),
                    arms: typed_arms,
                },
                ty,
                span,
            )
        }

        Expr::Closure { params, body, .. } => {
            let typed_params: Vec<TypedParam> = params
                .iter()
                .map(|p| TypedParam {
                    name: p.name.clone(),
                    ty: resolve_type_expr(&p.ty, env, errors),
                    mutable: p.mutable,
                    span: p.span,
                })
                .collect();
            let param_tys: Vec<Ty> = typed_params.iter().map(|p| p.ty.clone()).collect();
            let (typed_body, ret_ty) = match body {
                ClosureBody::Expr(e) => {
                    let mut closure_env = env.clone();
                    closure_env.push_scope();
                    for p in &typed_params {
                        closure_env.define(
                            &p.name,
                            Symbol::Var {
                                ty: p.ty.clone(),
                                mutable: false,
                                span: p.span,
                            },
                        );
                    }
                    let te = infer_typed_expr(e, &closure_env, registry, errors);
                    let r = te.ty.clone();
                    (TypedClosureBody::Expr(Box::new(te)), r)
                }
                ClosureBody::Block(b) => {
                    let mut closure_env = env.clone();
                    closure_env.push_scope();
                    for p in &typed_params {
                        closure_env.define(
                            &p.name,
                            Symbol::Var {
                                ty: p.ty.clone(),
                                mutable: false,
                                span: p.span,
                            },
                        );
                    }
                    let typed_stmts: Vec<_> = b
                        .stmts
                        .iter()
                        .map(|s| lower_stmt_shallow(s, &closure_env, registry, errors))
                        .collect();
                    let tb = crate::analyzer::typed_ast::TypedBlock {
                        stmts: typed_stmts,
                        span: b.span,
                    };
                    (TypedClosureBody::Block(tb), Ty::Unknown)
                }
            };
            let ty = Ty::Callable(param_tys, Box::new(ret_ty));
            mk(
                TypedExprKind::Closure {
                    params: typed_params,
                    body: typed_body,
                },
                ty,
                span,
            )
        }

        Expr::Spawn(inner, _) => {
            let ti = infer_typed_expr(inner, env, registry, errors);
            mk(TypedExprKind::Spawn(Box::new(ti)), Ty::Unknown, span)
        }

        Expr::Ref { mutable, expr, .. } => {
            let te = infer_typed_expr(expr, env, registry, errors);
            let inner = te.ty.clone();
            let ty = Ty::Ref(Box::new(inner), *mutable);
            mk(
                TypedExprKind::Ref {
                    mutable: *mutable,
                    expr: Box::new(te),
                },
                ty,
                span,
            )
        }

        Expr::Gen { body, .. } => {
            let mut gen_env = env.clone();
            gen_env.push_scope();
            let mut typed_stmts: Vec<crate::analyzer::typed_ast::TypedStmt> = Vec::new();
            for s in &body.stmts {
                let ts = lower_stmt_shallow(s, &gen_env, registry, errors);
                if let crate::analyzer::typed_ast::TypedStmt::VarDecl {
                    ref name,
                    ref ty,
                    mutable,
                    span: vspan,
                    ..
                } = ts
                {
                    gen_env.define(
                        name,
                        crate::analyzer::env::Symbol::Var {
                            ty: ty.clone(),
                            mutable,
                            span: vspan,
                        },
                    );
                }
                typed_stmts.push(ts);
            }
            gen_env.pop_scope();
            let tb = crate::analyzer::typed_ast::TypedBlock {
                stmts: typed_stmts,
                span: body.span,
            };
            let block_ty = match env.lookup("Block") {
                Some(crate::analyzer::env::Symbol::Type { id, .. }) => {
                    Ty::Named(id.clone(), "Block".into(), vec![])
                }
                _ => Ty::Unknown,
            };
            mk(TypedExprKind::Gen { body: tb }, block_ty, span)
        }

        Expr::Array(elems, _) => {
            let typed: Vec<_> = elems
                .iter()
                .map(|e| infer_typed_expr(e, env, registry, errors))
                .collect();
            mk(TypedExprKind::Array(typed), Ty::Unknown, span)
        }

        Expr::GenSplice(e, _) => {
            let te = infer_typed_expr(e, env, registry, errors);
            let ty = te.ty.clone();
            mk(TypedExprKind::GenSplice(Box::new(te)), ty, span)
        }

        Expr::Block(stmts, _) => {
            let typed_stmts = stmts
                .iter()
                .map(|s| lower_stmt_shallow(s, env, registry, errors))
                .collect();
            mk(TypedExprKind::Block(typed_stmts), Ty::Void, span)
        }
    }
}

// ---------------------------------------------------------------------------
// Call resolution helpers
// ---------------------------------------------------------------------------

/// Resolve a single type-argument expression (in expression position) to a Ty.
/// Used for `Type[TypeArg].method()` calls where the type arg is an identifier.
fn resolve_expr_as_ty(expr: &Expr, env: &Env) -> Option<Ty> {
    if let Expr::Ident(name, _) = expr {
        return Some(match name.as_str() {
            "int" => Ty::Int,
            "float" => Ty::Float,
            "bool" => Ty::Bool,
            "str" => Ty::Str,
            _ => match env.lookup(name) {
                Some(Symbol::Type { id, .. }) => Ty::Named(id.clone(), name.clone(), vec![]),
                _ => return None,
            },
        });
    }
    None
}

fn infer_call_field(
    object: &Expr,
    field: &str,
    typed_args: Vec<TypedExpr>,
    env: &Env,
    registry: &TypeRegistry,
    errors: &mut Vec<AnalysisError>,
    span: Span,
) -> TypedExpr {
    // Type[TypeArg].method() -- type-qualified static call with explicit type argument.
    // The parser sees this as Index(Ident(type_name), Ident(type_arg)).method(), so we
    // intercept it here before falling through to the instance-call path.
    if let Expr::Index {
        object: type_obj,
        index: type_arg_expr,
        ..
    } = object
    {
        if let Expr::Ident(type_name, _) = type_obj.as_ref() {
            let is_type = !matches!(
                env.lookup(type_name),
                Some(Symbol::Var { .. }) | Some(Symbol::Fn { .. })
            );
            if is_type {
                if let Some(type_arg_ty) = resolve_expr_as_ty(type_arg_expr, env) {
                    let method_fn = format!("{}_{}", type_name, field);
                    let raw_ret = registry
                        .find_method(type_name, field)
                        .map(|m| m.ret.clone())
                        .unwrap_or(Ty::Unknown);
                    // Substitute all GenericParams in the return type with the concrete arg.
                    let concrete_ret = substitute_t(&raw_ret, &type_arg_ty);
                    return mk(
                        TypedExprKind::StaticCall {
                            method_fn,
                            args: typed_args,
                        },
                        concrete_ret,
                        span,
                    );
                }
            }
        }
    }

    // Determine whether object is a type-namespace identifier.
    if let Expr::Ident(type_name, _) = object {
        let is_static = match env.lookup(type_name) {
            Some(Symbol::Var { .. }) | Some(Symbol::Fn { .. }) => false,
            _ => true, // Type, Iface, builtin (Vec/Map/...), or not in env
        };
        if is_static {
            let method_fn = format!("{}_{}", type_name, field);
            let ret = registry
                .find_method(type_name, field)
                .map(|m| m.ret.clone())
                .unwrap_or(Ty::Unknown);
            return mk(
                TypedExprKind::StaticCall {
                    method_fn,
                    args: typed_args,
                },
                ret,
                span,
            );
        }
    }

    // Instance method call.
    let to = infer_typed_expr(object, env, registry, errors);
    let obj_type = type_name_of(&to.ty);
    if let Some(tname) = &obj_type {
        // Check if this is a callable struct field (not a registered method).
        if registry.find_method(tname, field).is_none() {
            let field_ty = registry
                .get_struct_fields(tname)
                .and_then(|fs| fs.iter().find(|(n, _)| n == field))
                .map(|(_, t)| t.clone());
            if let Some(Ty::Callable(_, ret_box)) = field_ty {
                let ret_ty = *ret_box;
                let fat_ptr = mk(
                    TypedExprKind::Field {
                        object: Box::new(to),
                        field: field.to_string(),
                    },
                    Ty::Callable(vec![], Box::new(ret_ty.clone())),
                    span,
                );
                return mk(
                    TypedExprKind::IndirectCall {
                        fat_ptr: Box::new(fat_ptr),
                        args: typed_args,
                    },
                    ret_ty,
                    span,
                );
            }
        }
    }
    // .unwrap() on a Try type is equivalent to the ! postfix operator.
    if field == "unwrap" {
        if let Ty::Named(_, name, args) = &to.ty {
            if !registry.get_conformances(name, "Try").is_empty() {
                let inner_ty = args.first().cloned().unwrap_or(Ty::Unknown);
                return mk(TypedExprKind::Unwrap(Box::new(to)), inner_ty, span);
            }
        }
    }

    let (method_fn, ret) = if let Some(tname) = &obj_type {
        let qfn = format!("{}_{}", tname, field);

        // For any single-argument generic type (Vec[X], Option[X], Set[X], ...),
        // substitute the concrete element type for T throughout the method signature
        // so argument types and the return type are concrete.
        let elem_ty: Option<Ty> = match &to.ty {
            Ty::Named(_, _, args) if args.len() == 1 => args.first().cloned(),
            _ => None,
        };

        if let (Some(method), Some(elem)) = (registry.find_method(tname, field), &elem_ty) {
            // Check each argument against the substituted parameter type.
            // method.params does NOT include self -- it matches typed_args 1:1.
            let expected_params: Vec<Ty> = method
                .params
                .iter()
                .map(|(_, pt)| substitute_t(pt, elem))
                .collect();
            for (arg, expected) in typed_args.iter().zip(expected_params.iter()) {
                check_assignable(expected, &arg.ty, &arg.span, errors);
            }
            let ret = substitute_t(&method.ret, elem);
            return mk(
                TypedExprKind::MethodCall {
                    object: Box::new(to),
                    method_fn: qfn,
                    args: typed_args,
                },
                ret,
                span,
            );
        }

        let r = registry
            .find_method(tname, field)
            .map(|m| m.ret.clone())
            .unwrap_or(Ty::Unknown);
        // When the type name is a generic parameter (e.g. "T") and the registry has
        // no entry for it, fall back to the interface-bound lookup so the return type
        // is concrete (e.g. Str for to_str) rather than Unknown.
        let r = if r == Ty::Unknown {
            if let Ty::GenericParam(param_name) = &to.ty {
                let ifaces = env.get_param_ifaces(param_name);
                let mut found = Ty::Unknown;
                for iface_name in &ifaces {
                    if let Some(method) = registry.get_interface_method(iface_name, field) {
                        found = project_return_ty(&method.ret, param_name, iface_name, registry);
                        break;
                    }
                }
                found
            } else {
                r
            }
        } else {
            r
        };
        (qfn, r)
    } else if let Ty::Interface(_, iface_name) = &to.ty {
        // Interface dispatch: method_fn is the unqualified name; codegen emits
        // a vtable switch. Return type comes from the interface method signature.
        let ret = registry
            .get_interface_method(iface_name, field)
            .map(|m| m.ret.clone())
            .unwrap_or(Ty::Unknown);
        (field.to_string(), ret)
    } else if let Ty::GenericParam(param_name) = &to.ty {
        // Generic param dispatch: look up which interface declares this method
        // through the param's bounds, then return a projection type for any
        // associated-type return values.
        let ifaces = env.get_param_ifaces(param_name);
        let mut found_ret = Ty::Unknown;
        for iface_name in &ifaces {
            if let Some(method) = registry.get_interface_method(iface_name, field) {
                found_ret = project_return_ty(&method.ret, param_name, iface_name, registry);
                break;
            }
        }
        let method_fn = format!("{param_name}_{field}");
        return mk(
            TypedExprKind::StaticCall {
                method_fn,
                args: typed_args,
            },
            found_ret,
            span,
        );
    } else {
        (field.to_string(), Ty::Unknown)
    };
    mk(
        TypedExprKind::MethodCall {
            object: Box::new(to),
            method_fn,
            args: typed_args,
        },
        ret,
        span,
    )
}

fn infer_call_ident(
    name: &str,
    typed_args: Vec<TypedExpr>,
    env: &Env,
    registry: &TypeRegistry,
    errors: &mut Vec<AnalysisError>,
    span: Span,
) -> TypedExpr {
    match env.lookup(name) {
        Some(Symbol::Var {
            ty: Ty::Callable(_, ret),
            ..
        }) => {
            let ret_ty = *ret.clone();
            let callee = mk(
                TypedExprKind::Ident(name.to_string()),
                Ty::Callable(vec![], Box::new(ret_ty.clone())),
                span,
            );
            mk(
                TypedExprKind::IndirectCall {
                    fat_ptr: Box::new(callee),
                    args: typed_args,
                },
                ret_ty,
                span,
            )
        }
        Some(Symbol::Fn {
            params,
            ret,
            generic_params: gparams,
            generic_bounds: gbounds,
            inferred_bounds: ibounds,
            span: fn_def_span,
            ..
        }) => {
            let fn_def_span = *fn_def_span;
            if params.len() != typed_args.len() && !params.iter().any(|(_, t)| *t == Ty::Unknown) {
                errors.push(AnalysisError::ArityMismatch {
                    expected: params.len(),
                    found: typed_args.len(),
                    span,
                    fn_span: Some(fn_def_span),
                });
            }
            // Check each argument type against the declared parameter type.
            // types_compatible allows int -> float coercion and GenericParam wildcards.
            for ((_, param_ty), arg) in params.iter().zip(typed_args.iter()) {
                check_assignable(param_ty, &arg.ty, &arg.span, errors);
            }
            let ret_ty = ret.clone();
            let callee_ty = Ty::Callable(
                params.iter().map(|(_, t)| t.clone()).collect(),
                Box::new(ret_ty.clone()),
            );
            let callee = mk(TypedExprKind::Ident(name.to_string()), callee_ty, span);
            let mut all_bounds = gbounds.clone();
            for ib in ibounds {
                if let Some(existing) = all_bounds
                    .iter_mut()
                    .find(|b| b.param == ib.param && b.iface == ib.iface)
                {
                    if existing.source_span.is_none() && ib.source_span.is_some() {
                        existing.source_span = ib.source_span;
                        existing.source_desc = ib.source_desc.clone();
                    }
                } else {
                    all_bounds.push(ib.clone());
                }
            }
            let declared_param_tys: Vec<Ty> = params.iter().map(|(_, t)| t.clone()).collect();
            mk(
                TypedExprKind::Call {
                    callee: Box::new(callee),
                    args: typed_args,
                    fn_name: name.to_string(),
                    generic_bounds: all_bounds,
                    generic_params: gparams.clone(),
                    param_tys: declared_param_tys,
                },
                ret_ty,
                span,
            )
        }
        Some(Symbol::FnOverloadSet { overloads }) => {
            let overloads = overloads.clone();
            match find_best_overload(&overloads, &typed_args) {
                Some(overload) => {
                    let ret_ty = overload.ret.clone();
                    let callee_ty = Ty::Callable(
                        overload.params.iter().map(|(_, t)| t.clone()).collect(),
                        Box::new(ret_ty.clone()),
                    );
                    let callee = mk(
                        TypedExprKind::Ident(overload.mangled_name.clone()),
                        callee_ty,
                        span,
                    );
                    let mut all_bounds = overload.generic_bounds.clone();
                    for ib in &overload.inferred_bounds {
                        if let Some(existing) = all_bounds
                            .iter_mut()
                            .find(|b| b.param == ib.param && b.iface == ib.iface)
                        {
                            if existing.source_span.is_none() && ib.source_span.is_some() {
                                existing.source_span = ib.source_span;
                                existing.source_desc = ib.source_desc.clone();
                            }
                        } else {
                            all_bounds.push(ib.clone());
                        }
                    }
                    let declared_param_tys: Vec<Ty> =
                        overload.params.iter().map(|(_, t)| t.clone()).collect();
                    mk(
                        TypedExprKind::Call {
                            callee: Box::new(callee),
                            args: typed_args,
                            fn_name: name.to_string(),
                            generic_bounds: all_bounds,
                            generic_params: overload.generic_params.clone(),
                            param_tys: declared_param_tys,
                        },
                        ret_ty,
                        span,
                    )
                }
                None => {
                    errors.push(AnalysisError::NoMatchingOverload {
                        name: name.to_string(),
                        span,
                    });
                    let callee = mk(TypedExprKind::Ident(name.to_string()), Ty::Unknown, span);
                    mk(
                        TypedExprKind::Call {
                            callee: Box::new(callee),
                            args: typed_args,
                            fn_name: name.to_string(),
                            generic_bounds: vec![],
                            generic_params: vec![],
                            param_tys: vec![],
                        },
                        Ty::Unknown,
                        span,
                    )
                }
            }
        }
        Some(Symbol::Var { .. }) => {
            // Callable variable whose type wasn't Callable above -- indirect.
            let callee =
                infer_typed_expr(&Expr::Ident(name.to_string(), span), env, registry, errors);
            mk(
                TypedExprKind::IndirectCall {
                    fat_ptr: Box::new(callee),
                    args: typed_args,
                },
                Ty::Unknown,
                span,
            )
        }
        _ => {
            // Unknown name or type -- emit as a plain Call with Unknown type.
            let callee = mk(TypedExprKind::Ident(name.to_string()), Ty::Unknown, span);
            mk(
                TypedExprKind::Call {
                    callee: Box::new(callee),
                    args: typed_args,
                    fn_name: name.to_string(),
                    generic_bounds: vec![],
                    generic_params: vec![],
                    param_tys: vec![],
                },
                Ty::Unknown,
                span,
            )
        }
    }
}

fn param_matches_arg(pt: &Ty, arg_ty: &Ty) -> bool {
    match pt {
        Ty::Unknown | Ty::GenericParam(_) => true,
        Ty::Named(_, pname, pargs) => match arg_ty {
            Ty::Named(_, aname, aargs) => {
                pname == aname
                    && pargs.len() == aargs.len()
                    && pargs
                        .iter()
                        .zip(aargs)
                        .all(|(p, a)| param_matches_arg(p, a))
            }
            _ => false,
        },
        _ => *pt == *arg_ty,
    }
}

fn find_best_overload<'a>(
    overloads: &'a [FnOverload],
    args: &[TypedExpr],
) -> Option<&'a FnOverload> {
    // Exact/generic match: GenericParam and Unknown on the param side are wildcards.
    for o in overloads {
        if o.params.len() != args.len() {
            continue;
        }
        let matches = o
            .params
            .iter()
            .zip(args.iter())
            .all(|((_, pt), arg)| param_matches_arg(pt, &arg.ty));
        if matches {
            return Some(o);
        }
    }
    // Relaxed match: also allow Unknown on the arg side.
    for o in overloads {
        if o.params.len() != args.len() {
            continue;
        }
        let matches = o
            .params
            .iter()
            .zip(args.iter())
            .all(|((_, pt), arg)| param_matches_arg(pt, &arg.ty) || arg.ty == Ty::Unknown);
        if matches {
            return Some(o);
        }
    }
    // Coercion pass: allow numeric widening (int -> float) and other compatible types.
    // This runs after exact and Unknown passes so exact matches always win.
    for o in overloads {
        if o.params.len() != args.len() {
            continue;
        }
        let matches = o
            .params
            .iter()
            .zip(args.iter())
            .all(|((_, pt), arg)| types_compatible(pt, &arg.ty));
        if matches {
            return Some(o);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Generic-param method dispatch helpers
// ---------------------------------------------------------------------------

/// Rewrite an interface method return type so that any `GenericParam(name)` that
/// is an associated type of `iface_name` becomes `Ty::Projection { base: param, assoc: name }`.
/// Everything else (regular generic params, concrete types) passes through unchanged.
fn project_return_ty(ty: &Ty, param: &str, iface_name: &str, registry: &TypeRegistry) -> Ty {
    match ty {
        Ty::GenericParam(name) if registry.is_assoc_type_of(name, iface_name) => Ty::Projection {
            base: param.to_string(),
            assoc: name.clone(),
        },
        Ty::Named(id, n, args) => Ty::Named(
            id.clone(),
            n.clone(),
            args.iter()
                .map(|a| project_return_ty(a, param, iface_name, registry))
                .collect(),
        ),
        Ty::Tuple(ts) => Ty::Tuple(
            ts.iter()
                .map(|t| project_return_ty(t, param, iface_name, registry))
                .collect(),
        ),
        Ty::Callable(ps, r) => Ty::Callable(
            ps.iter()
                .map(|p| project_return_ty(p, param, iface_name, registry))
                .collect(),
            Box::new(project_return_ty(r, param, iface_name, registry)),
        ),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Field type resolution
// ---------------------------------------------------------------------------

fn resolve_field_ty(obj_ty: &Ty, field: &str, registry: &TypeRegistry) -> Ty {
    let tname = match type_name_of(obj_ty) {
        Some(n) => n,
        None => return Ty::Unknown,
    };
    let raw_ty = registry
        .get_struct_fields(&tname)
        .and_then(|fields| fields.iter().find(|(n, _)| n == field))
        .map(|(_, ty)| ty.clone())
        .unwrap_or(Ty::Unknown);
    // Substitute concrete type args (e.g. ListNode[int].value: T -> int).
    if let Ty::Named(_, _, concrete_args) = obj_ty {
        if !concrete_args.is_empty() {
            if let Some(param_names) = registry.get_generic_param_order(&tname) {
                let subst: std::collections::HashMap<String, Ty> = param_names
                    .iter()
                    .zip(concrete_args.iter())
                    .map(|(name, ty)| (name.clone(), ty.clone()))
                    .collect();
                return subst_generic_ty(&raw_ty, &subst);
            }
        }
    }
    raw_ty
}

pub fn subst_generic_ty(ty: &Ty, subst: &std::collections::HashMap<String, Ty>) -> Ty {
    match ty {
        Ty::GenericParam(p) => subst.get(p).cloned().unwrap_or_else(|| ty.clone()),
        Ty::Named(id, name, args) => {
            let new_args = args.iter().map(|a| subst_generic_ty(a, subst)).collect();
            Ty::Named(id.clone(), name.clone(), new_args)
        }
        Ty::Callable(params, ret) => Ty::Callable(
            params.iter().map(|p| subst_generic_ty(p, subst)).collect(),
            Box::new(subst_generic_ty(ret, subst)),
        ),
        Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| subst_generic_ty(t, subst)).collect()),
        other => other.clone(),
    }
}

pub fn type_name_of(ty: &Ty) -> Option<String> {
    match ty {
        Ty::Int => Some("int".into()),
        Ty::Float => Some("float".into()),
        Ty::Bool => Some("bool".into()),
        Ty::Str => Some("str".into()),
        Ty::Named(_, name, _) | Ty::GenericParam(name) => Some(name.clone()),
        // These types have no single dispatch name.
        Ty::Void
        | Ty::Unknown
        | Ty::Tuple(_)
        | Ty::Callable(_, _)
        | Ty::Ref(_, _)
        | Ty::Union(_)
        | Ty::Interface(_, _)
        | Ty::Compound(_)
        | Ty::Projection { .. } => None,
    }
}

// ---------------------------------------------------------------------------
// Pattern lowering
// ---------------------------------------------------------------------------

fn lower_pattern(
    pat: &Pattern,
    env: &Env,
    registry: &TypeRegistry,
    errors: &mut Vec<AnalysisError>,
) -> TypedPattern {
    match pat {
        Pattern::Wildcard(s) => TypedPattern::Wildcard(*s),
        Pattern::Literal(e) => TypedPattern::Literal(infer_typed_expr(e, env, registry, errors)),
        Pattern::TypeBinding { ty, name, span } => {
            // A bare-ident pattern like `Less` (parsed as TypeBinding with ty="_")
            // may actually be a unit enum variant. Promote it to a Struct pattern
            // so the match codegen can do a proper discriminant comparison.
            if ty == "_" && registry.is_enum_variant(name) {
                TypedPattern::Struct {
                    variant: name.clone(),
                    fields: vec![],
                    has_rest: false,
                    span: *span,
                }
            } else {
                TypedPattern::TypeBinding {
                    ty: ty.clone(),
                    name: name.clone(),
                    span: *span,
                }
            }
        }
        Pattern::InterfaceGuard {
            interface,
            name,
            span,
        } => TypedPattern::InterfaceGuard {
            interface: interface.clone(),
            name: name.clone(),
            span: *span,
        },
        Pattern::Struct {
            variant,
            fields,
            has_rest,
            span,
        } => TypedPattern::Struct {
            variant: variant.clone(),
            fields: fields.clone(),
            has_rest: *has_rest,
            span: *span,
        },
        Pattern::Tuple(pats, s) => TypedPattern::Tuple(
            pats.iter()
                .map(|p| lower_pattern(p, env, registry, errors))
                .collect(),
            *s,
        ),
    }
}

// ---------------------------------------------------------------------------
// Shallow stmt lowering (for closure/gen blocks where env is immutable)
// ---------------------------------------------------------------------------

fn lower_stmt_shallow(
    stmt: &crate::parser::ast::Stmt,
    env: &Env,
    registry: &TypeRegistry,
    errors: &mut Vec<AnalysisError>,
) -> crate::analyzer::typed_ast::TypedStmt {
    use crate::analyzer::typed_ast::TypedStmt;
    use crate::parser::ast::Stmt;
    match stmt {
        Stmt::Expr(e) => TypedStmt::Expr(infer_typed_expr(e, env, registry, errors)),
        Stmt::Return { value, span } => TypedStmt::Return {
            value: value
                .as_ref()
                .map(|v| infer_typed_expr(v, env, registry, errors)),
            span: *span,
        },
        Stmt::Break(s) => TypedStmt::Break(*s),
        Stmt::Continue(s) => TypedStmt::Continue(*s),
        Stmt::Raise { value, span } => TypedStmt::Raise {
            value: value
                .as_ref()
                .map(|v| infer_typed_expr(v, env, registry, errors)),
            span: *span,
        },
        Stmt::VarDecl {
            name,
            ty,
            value,
            mutable,
            span,
        } => {
            let declared = resolve_type_expr(ty, env, errors);
            let typed_val = infer_typed_expr(value, env, registry, errors);
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
        } => TypedStmt::Assign {
            target: infer_typed_expr(target, env, registry, errors),
            value: infer_typed_expr(value, env, registry, errors),
            span: *span,
        },
        Stmt::CompoundAssign {
            target,
            op,
            rhs,
            span,
        } => TypedStmt::CompoundAssign {
            target: infer_typed_expr(target, env, registry, errors),
            op: op.clone(),
            rhs: infer_typed_expr(rhs, env, registry, errors),
            span: *span,
        },
        Stmt::If {
            branches,
            else_branch,
            span,
        } => {
            let typed_branches = branches
                .iter()
                .map(|(cond, block)| {
                    let tc = infer_typed_expr(cond, env, registry, errors);
                    let tb = shallow_block(block, env, registry, errors);
                    (tc, tb)
                })
                .collect();
            let else_typed = else_branch
                .as_ref()
                .map(|b| shallow_block(b, env, registry, errors));
            TypedStmt::If {
                branches: typed_branches,
                else_branch: else_typed,
                span: *span,
            }
        }
        Stmt::While { cond, body, span } => TypedStmt::While {
            cond: infer_typed_expr(cond, env, registry, errors),
            body: shallow_block(body, env, registry, errors),
            span: *span,
        },
        Stmt::DoWhile { body, cond, span } => TypedStmt::DoWhile {
            body: shallow_block(body, env, registry, errors),
            cond: infer_typed_expr(cond, env, registry, errors),
            span: *span,
        },
        Stmt::For {
            binding,
            binding_ty,
            iterable,
            body,
            span,
        } => {
            let bt = binding_ty
                .as_ref()
                .map(|t| resolve_type_expr(t, env, errors))
                .unwrap_or(Ty::Unknown);
            TypedStmt::For {
                binding: binding.clone(),
                binding_ty: bt,
                iterable: infer_typed_expr(iterable, env, registry, errors),
                body: shallow_block(body, env, registry, errors),
                iter_ty: None,
                span: *span,
            }
        }
        Stmt::TryCatch {
            body,
            handlers,
            finally,
            span,
        } => {
            use crate::analyzer::typed_ast::TypedCatchHandler;
            TypedStmt::TryCatch {
                body: shallow_block(body, env, registry, errors),
                handlers: handlers
                    .iter()
                    .map(|h| {
                        let handler_ty = resolve_type_expr(&h.ty, env, errors);
                        let mut handler_env = env.clone();
                        handler_env.push_scope();
                        handler_env.define(
                            &h.binding,
                            crate::analyzer::env::Symbol::Var {
                                ty: handler_ty.clone(),
                                mutable: false,
                                span: h.span,
                            },
                        );
                        let handler_body = shallow_block(&h.body, &handler_env, registry, errors);
                        handler_env.pop_scope();
                        TypedCatchHandler {
                            ty: handler_ty,
                            binding: h.binding.clone(),
                            body: handler_body,
                            span: h.span,
                        }
                    })
                    .collect(),
                finally: finally
                    .as_ref()
                    .map(|b| shallow_block(b, env, registry, errors)),
                span: *span,
            }
        }
        Stmt::FnDef(f) => {
            // Nested fn in closure: produce a minimal typed def
            let ret = resolve_type_expr(&f.return_type, env, errors);
            let params: Vec<TypedParam> = f
                .params
                .iter()
                .map(|p| TypedParam {
                    name: p.name.clone(),
                    ty: resolve_type_expr(&p.ty, env, errors),
                    mutable: p.mutable,
                    span: p.span,
                })
                .collect();
            let body = shallow_block(&f.body, env, registry, errors);
            TypedStmt::FnDef(crate::analyzer::typed_ast::TypedFnDef {
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
                span: f.span,
            })
        }
    }
}

fn shallow_block(
    block: &crate::parser::ast::Block,
    env: &Env,
    registry: &TypeRegistry,
    errors: &mut Vec<AnalysisError>,
) -> crate::analyzer::typed_ast::TypedBlock {
    crate::analyzer::typed_ast::TypedBlock {
        stmts: block
            .stmts
            .iter()
            .map(|s| lower_stmt_shallow(s, env, registry, errors))
            .collect(),
        span: block.span,
    }
}

// ---------------------------------------------------------------------------
// Binop type inference
// ---------------------------------------------------------------------------

fn infer_binop(
    op: BinOp,
    lt: Ty,
    rt: Ty,
    span: &Span,
    errors: &mut Vec<AnalysisError>,
    registry: &TypeRegistry,
) -> Ty {
    use BinOp::*;
    match op {
        Add | Sub | Mul | Div | Mod => match (&lt, &rt) {
            (Ty::Int, Ty::Int) => Ty::Int,
            (Ty::Float, Ty::Float) => Ty::Float,
            (Ty::Int, Ty::Float) | (Ty::Float, Ty::Int) => Ty::Float,
            (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
            // Named left operand: look up the actual return type from the hook registry
            // so expressions like `p1 + p2` carry the correct result type downstream.
            (Ty::Named(_, ln, _), _) => {
                let op_str = match op {
                    Add => "+",
                    Sub => "-",
                    Mul => "*",
                    Div => "/",
                    Mod => "%",
                    _ => "",
                };
                registry
                    .find_method(ln, op_str)
                    .map(|m| m.ret.clone())
                    .unwrap_or(Ty::Unknown)
            }
            (Ty::GenericParam(_), _) | (_, Ty::Named(_, _, _)) | (_, Ty::GenericParam(_)) => {
                Ty::Unknown
            }
            (l, r) if l == r && type_name_of(l).is_some() => l.clone(),
            _ => {
                errors.push(AnalysisError::TypeMismatch {
                    expected: "numeric or operator-overloaded types".into(),
                    found: format!("{lt} and {rt}"),
                    span: *span,
                    decl_span: None,
                });
                Ty::Unknown
            }
        },
        Eq | Ne | Lt | Gt | LtEq | GtEq => Ty::Bool,
        // Spaceship returns Ordering.
        Spaceship => match (&lt, &rt) {
            (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
            _ => match registry.lookup_by_name("Ordering") {
                Some(e) => Ty::Named(e.id.clone(), "Ordering".into(), vec![]),
                None => Ty::Int,
            },
        },
        And | Or => {
            if !matches!(lt, Ty::Bool | Ty::Unknown) || !matches!(rt, Ty::Bool | Ty::Unknown) {
                errors.push(AnalysisError::TypeMismatch {
                    expected: "bool".into(),
                    found: format!("{lt} and {rt}"),
                    span: *span,
                    decl_span: None,
                });
            }
            Ty::Bool
        }
        Pipe => Ty::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Generic substitution (replaces GenericParam wildcards with a concrete type)
// ---------------------------------------------------------------------------

pub fn substitute_t(ty: &Ty, concrete: &Ty) -> Ty {
    match ty {
        Ty::GenericParam(_) => concrete.clone(),
        Ty::Named(id, name, args) => Ty::Named(
            id.clone(),
            name.clone(),
            args.iter().map(|a| substitute_t(a, concrete)).collect(),
        ),
        Ty::Callable(params, ret) => Ty::Callable(
            params.iter().map(|p| substitute_t(p, concrete)).collect(),
            Box::new(substitute_t(ret, concrete)),
        ),
        Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|e| substitute_t(e, concrete)).collect()),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// Assignability check (used by check.rs)
// ---------------------------------------------------------------------------

pub fn check_assignable(expected: &Ty, found: &Ty, span: &Span, errors: &mut Vec<AnalysisError>) {
    if !types_compatible(expected, found) {
        errors.push(AnalysisError::TypeMismatch {
            expected: expected.to_string(),
            found: found.to_string(),
            span: *span,
            decl_span: None,
        });
    }
}

/// Like `check_assignable`, but attaches a secondary span pointing at where the
/// expected type was declared.  Used by VarDecl to show the declaration site.
pub fn check_assignable_with_decl_span(
    expected: &Ty,
    found: &Ty,
    span: &Span,
    decl_span: Option<Span>,
    errors: &mut Vec<AnalysisError>,
) {
    if !types_compatible(expected, found) {
        errors.push(AnalysisError::TypeMismatch {
            expected: expected.to_string(),
            found: found.to_string(),
            span: *span,
            decl_span,
        });
    }
}

/// Like `check_assignable`, but normalizes both sides through the active projection
/// pin table before comparing.  Call this whenever `env` is in scope and either
/// side might contain `Ty::Projection` that has a concrete resolution.
pub fn check_assignable_normalized(
    expected: &Ty,
    found: &Ty,
    env: &crate::analyzer::env::Env,
    span: &Span,
    errors: &mut Vec<AnalysisError>,
) {
    let pins = env.get_active_pins();
    let norm_expected = crate::analyzer::ty::normalize_ty(expected, &pins);
    let norm_found = crate::analyzer::ty::normalize_ty(found, &pins);
    if !types_compatible(&norm_expected, &norm_found) {
        errors.push(AnalysisError::TypeMismatch {
            expected: norm_expected.to_string(),
            found: norm_found.to_string(),
            span: *span,
            decl_span: None,
        });
    }
}

pub fn types_compatible(expected: &Ty, found: &Ty) -> bool {
    if matches!(expected, Ty::Unknown) || matches!(found, Ty::Unknown) {
        return true;
    }
    // GenericParam and Projection are wildcards; concrete type resolved later.
    if matches!(expected, Ty::GenericParam(_) | Ty::Projection { .. })
        || matches!(found, Ty::GenericParam(_) | Ty::Projection { .. })
    {
        return true;
    }
    if *expected == Ty::Float && *found == Ty::Int {
        return true;
    }
    match (expected, found) {
        // Interface and compound interface types are supertypes of any concrete type;
        // runtime dispatch checks actual conformance. Specific conformance is
        // verified separately by check.rs where the registry is available.
        (Ty::Interface(_, _), _) | (Ty::Compound(_), _) => true,
        (_, Ty::Interface(_, _)) | (_, Ty::Compound(_)) => true,

        // References are transparent: &T is compatible with &U iff T compatible with U,
        // and T is assignable to &T (and vice versa) for struct-like heap types.
        (Ty::Ref(ei, _), Ty::Ref(fi, _)) => types_compatible(ei, fi),
        (Ty::Ref(inner, _), other) | (other, Ty::Ref(inner, _))
            if !matches!(other, Ty::Ref(_, _)) =>
        {
            types_compatible(inner, other)
        }

        (Ty::Named(_, en, ea), Ty::Named(_, fn_, fa)) if en == fn_ => ea
            .iter()
            .zip(fa.iter())
            .all(|(e, f)| types_compatible(e, f)),
        (Ty::Callable(ep, er), Ty::Callable(fp, fr)) => {
            ep.len() == fp.len()
                && ep
                    .iter()
                    .zip(fp.iter())
                    .all(|(e, f)| types_compatible(e, f))
                && types_compatible(er, fr)
        }
        (Ty::Tuple(es), Ty::Tuple(fs)) => {
            es.len() == fs.len()
                && es
                    .iter()
                    .zip(fs.iter())
                    .all(|(e, f)| types_compatible(e, f))
        }
        _ => expected == found,
    }
}

// ---------------------------------------------------------------------------
// Constructor helper
// ---------------------------------------------------------------------------

fn mk(kind: TypedExprKind, ty: Ty, span: Span) -> TypedExpr {
    TypedExpr { kind, ty, span }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::env::Env;
    use crate::analyzer::ty::{Ty, TypeRegistry};
    use crate::diagnostics::Span;
    use crate::parser::ast::Expr;

    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    fn fresh() -> (Env, TypeRegistry) {
        (Env::new(), TypeRegistry::new())
    }

    #[test]
    fn int_literal() {
        let (env, reg) = fresh();
        let mut errs = vec![];
        let te = infer_typed_expr(&Expr::Int(1, s()), &env, &reg, &mut errs);
        assert_eq!(te.ty, Ty::Int);
        assert!(errs.is_empty());
    }

    #[test]
    fn float_literal() {
        let (env, reg) = fresh();
        let mut errs = vec![];
        let te = infer_typed_expr(&Expr::Float(1.0, s()), &env, &reg, &mut errs);
        assert_eq!(te.ty, Ty::Float);
    }

    #[test]
    fn bool_literal() {
        let (env, reg) = fresh();
        let mut errs = vec![];
        let te = infer_typed_expr(&Expr::Bool(true, s()), &env, &reg, &mut errs);
        assert_eq!(te.ty, Ty::Bool);
    }

    #[test]
    fn int_add_is_int() {
        let (env, reg) = fresh();
        let mut errs = vec![];
        let expr = Expr::BinOp {
            op: BinOp::Add,
            left: Box::new(Expr::Int(1, s())),
            right: Box::new(Expr::Int(2, s())),
            span: s(),
        };
        let te = infer_typed_expr(&expr, &env, &reg, &mut errs);
        assert_eq!(te.ty, Ty::Int);
        assert!(errs.is_empty());
    }

    #[test]
    fn comparison_yields_bool() {
        let (env, reg) = fresh();
        let mut errs = vec![];
        let expr = Expr::BinOp {
            op: BinOp::Eq,
            left: Box::new(Expr::Int(1, s())),
            right: Box::new(Expr::Int(1, s())),
            span: s(),
        };
        let te = infer_typed_expr(&expr, &env, &reg, &mut errs);
        assert_eq!(te.ty, Ty::Bool);
    }
}
