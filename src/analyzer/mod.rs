pub mod check;
pub mod collect;
pub mod conformance;
pub mod constrain;
pub mod env;
pub mod error;
pub mod exhaustive;
pub mod infer;
pub mod resolve;
pub mod returns;
pub mod solve;
pub mod ty;
pub mod typed_ast;

pub use error::AnalysisError;
pub use ty::{Ty, TypeId, TypeRegistry};
pub use typed_ast::TypedFile;

use crate::analyzer::env::{Env, FnOverload, GenericBound, Symbol};
use crate::analyzer::resolve::resolve_type_expr;
use crate::analyzer::ty::{MethodEntry, TypeKind};
use crate::analyzer::typed_ast::{
    TypedEnumDef, TypedEnumVariant, TypedField, TypedFnDef, TypedHookDef, TypedImplBlock,
    TypedInterfaceDef, TypedInterfaceMethod, TypedItem, TypedParam, TypedStructDef,
};
use crate::diagnostics::Span;
use crate::parser::ast::{HookName, ImplKind, Item, SourceFile, TypeExpr};

fn register_builtins(env: &mut Env, registry: &mut ty::TypeRegistry) {
    use ty::ConformanceEntry as CE;
    let s = Span::new(0, 0);
    let unc = || CE { bounds: vec![] }; // unconditional entry
    let with = |param: &str, iface: &str| CE {
        bounds: vec![(param.into(), iface.into())],
    };
    let with2 = |p1: &str, i1: &str, p2: &str, i2: &str| CE {
        bounds: vec![(p1.into(), i1.into()), (p2.into(), i2.into())],
    };

    // int
    for iface in &[
        "Copy",
        "Clone",
        "Eq",
        "Hash",
        "Ord",
        "PartialEq",
        "PartialOrd",
        "Debug",
        "Display",
        "Default",
        "Zero",
        "One",
        "Negatable",
        "Abs",
        "Signum",
        "Addable",
        "Subtractable",
        "Multiplicable",
        "Divisible",
        "Remainder",
        "Bitwise",
        "Numeric",
        "Integral",
    ] {
        registry.register_conformance("int", iface, unc());
    }

    // float (not Eq, not Ord, not Remainder, not Bitwise, not Integral due to NaN / IEEE754)
    for iface in &[
        "Copy",
        "Clone",
        "PartialEq",
        "PartialOrd",
        "Debug",
        "Display",
        "Default",
        "Zero",
        "One",
        "Negatable",
        "Abs",
        "Signum",
        "Addable",
        "Subtractable",
        "Multiplicable",
        "Divisible",
        "Fractional",
        "Numeric",
    ] {
        registry.register_conformance("float", iface, unc());
    }

    // bool
    for iface in &[
        "Copy",
        "Clone",
        "Eq",
        "Hash",
        "Ord",
        "PartialEq",
        "PartialOrd",
        "Debug",
        "Display",
        "Default",
    ] {
        registry.register_conformance("bool", iface, unc());
    }

    // str
    for iface in &[
        "Clone",
        "Eq",
        "Hash",
        "Ord",
        "PartialEq",
        "PartialOrd",
        "Debug",
        "Display",
        "Default",
        "Semigroup",
        "Monoid",
        "Addable",
    ] {
        registry.register_conformance("str", iface, unc());
    }
    registry.register_conformance("str", "Indexable", unc());

    // Vec[T] -- conditional on T
    for (iface, bound) in &[
        ("Clone", "Clone"),
        ("Eq", "Eq"),
        ("Hash", "Hash"),
        ("Ord", "Ord"),
        ("PartialEq", "PartialEq"),
        ("PartialOrd", "PartialOrd"),
        ("Debug", "Debug"),
        ("Display", "Display"),
    ] {
        registry.register_conformance("Vec", iface, with("T", bound));
    }
    for iface in &[
        "Default",
        "Semigroup",
        "Monoid",
        "Iterable",
        "Foldable",
        "Indexable",
    ] {
        registry.register_conformance("Vec", iface, unc());
    }

    // Set[T: Hash]
    for (iface, bound) in &[
        ("Clone", "Clone"),
        ("Eq", "Eq"),
        ("Debug", "Debug"),
        ("Display", "Display"),
    ] {
        registry.register_conformance("Set", iface, with("T", bound));
    }
    for iface in &["Default", "Semigroup", "Monoid", "Iterable", "Foldable"] {
        registry.register_conformance("Set", iface, unc());
    }

    // Map[K: Hash, V]
    for (iface, k_bound, v_bound) in &[
        ("Clone", "Clone", "Clone"),
        ("Eq", "Eq", "Eq"),
        ("Debug", "Debug", "Debug"),
        ("Display", "Display", "Display"),
    ] {
        registry.register_conformance("Map", iface, with2("K", k_bound, "V", v_bound));
    }
    for iface in &["Default", "Iterable", "Foldable", "Indexable"] {
        registry.register_conformance("Map", iface, unc());
    }

    // Option[T]
    for (iface, bound) in &[
        ("Clone", "Clone"),
        ("Eq", "Eq"),
        ("Hash", "Hash"),
        ("Ord", "Ord"),
        ("PartialEq", "PartialEq"),
        ("PartialOrd", "PartialOrd"),
        ("Debug", "Debug"),
        ("Display", "Display"),
    ] {
        registry.register_conformance("Option", iface, with("T", bound));
    }
    for iface in &["Default", "Iterable", "Foldable"] {
        registry.register_conformance("Option", iface, unc());
    }

    // Shared[T]
    registry.register_conformance("Shared", "Clone", unc());

    let exc_id = registry.register("Exception".into(), TypeKind::Struct);
    env.define(
        "Exception",
        Symbol::Type {
            id: exc_id,
            span: s,
        },
    );

    let ord_id = registry.register(
        "Ordering".into(),
        TypeKind::Enum {
            variant_names: vec!["Less".into(), "Equal".into(), "Greater".into()],
        },
    );
    env.define(
        "Ordering",
        Symbol::Type {
            id: ord_id,
            span: s,
        },
    );
    for iface in &[
        "Copy",
        "Clone",
        "Eq",
        "Hash",
        "Ord",
        "PartialEq",
        "PartialOrd",
        "Debug",
        "Display",
    ] {
        registry.register_conformance("Ordering", iface, unc());
    }

    // len, panic, assert, clock_ms: no interface requirements
    // (print/println are defined in the prelude as def print[T: Display])
    let fns: &[(&str, &[Ty], Ty)] = &[
        ("len", &[Ty::Unknown], Ty::Int),
        ("panic", &[Ty::Unknown], Ty::Void),
        ("assert", &[Ty::Unknown], Ty::Void),
        ("clock_ms", &[], Ty::Int),
    ];
    for (name, params, ret) in fns {
        env.define(
            name,
            Symbol::Fn {
                generic_params: vec![],
                generic_bounds: vec![],
                params: params
                    .iter()
                    .enumerate()
                    .map(|(i, t)| (format!("_{i}"), t.clone()))
                    .collect(),
                ret: ret.clone(),
                span: s,
            },
        );
    }
}

/// Analyze `source`, producing a `TypedFile` or a list of errors.
pub fn analyze(source: &SourceFile) -> Result<TypedFile, Vec<AnalysisError>> {
    let mut errors: Vec<AnalysisError> = Vec::new();
    let mut env = Env::new();
    let mut registry = ty::TypeRegistry::new();

    env.push_scope();
    register_builtins(&mut env, &mut registry);

    // Pass 1: collect top-level names.
    errors.extend(collect::collect_top_level(source, &mut env, &mut registry));

    // Pass 1b: resolve top-level function signatures, grouping overloads.
    {
        // Collect function items grouped by name, preserving first-appearance order.
        use std::collections::HashMap;
        let mut fn_order: Vec<String> = Vec::new();
        let mut fn_groups: HashMap<String, Vec<usize>> = HashMap::new();
        let fns: Vec<_> = source
            .items
            .iter()
            .filter_map(|i| {
                if let Item::Function(f) = i {
                    Some(f)
                } else {
                    None
                }
            })
            .collect();
        for (idx, f) in fns.iter().enumerate() {
            if !fn_groups.contains_key(&f.name) {
                fn_order.push(f.name.clone());
            }
            fn_groups.entry(f.name.clone()).or_default().push(idx);
        }

        for name in &fn_order {
            let indices = &fn_groups[name];
            if indices.len() == 1 {
                let f = fns[indices[0]];
                let has_generics = !f.generic_params.is_empty();
                if has_generics {
                    env.push_scope();
                    for gp in &f.generic_params {
                        env.define(
                            &gp.name,
                            Symbol::Type {
                                id: TypeId(0),
                                span: gp.span,
                            },
                        );
                    }
                }
                let ret = resolve_type_expr(&f.return_type, &env, &mut errors);
                let params: Vec<(String, Ty)> = f
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                    .collect();
                if has_generics {
                    env.pop_scope();
                }
                let generic_bounds = f
                    .generic_params
                    .iter()
                    .flat_map(|g| {
                        g.bounds.iter().map(move |b| GenericBound {
                            param: g.name.clone(),
                            iface: b.clone(),
                        })
                    })
                    .collect();
                env.define(
                    name,
                    Symbol::Fn {
                        generic_params: f.generic_params.iter().map(|g| g.name.clone()).collect(),
                        generic_bounds,
                        params,
                        ret,
                        span: f.span,
                    },
                );
            } else {
                let mut overloads: Vec<FnOverload> = Vec::new();
                for (local_idx, &global_idx) in indices.iter().enumerate() {
                    let f = fns[global_idx];
                    let mangled_name = format!("{}__{}", name, local_idx);
                    let has_generics = !f.generic_params.is_empty();
                    if has_generics {
                        env.push_scope();
                        for gp in &f.generic_params {
                            env.define(
                                &gp.name,
                                Symbol::Type {
                                    id: TypeId(0),
                                    span: gp.span,
                                },
                            );
                        }
                    }
                    let ret = resolve_type_expr(&f.return_type, &env, &mut errors);
                    let params: Vec<(String, Ty)> = f
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                        .collect();
                    if has_generics {
                        env.pop_scope();
                    }
                    let generic_bounds = f
                        .generic_params
                        .iter()
                        .flat_map(|g| {
                            g.bounds.iter().map(move |b| GenericBound {
                                param: g.name.clone(),
                                iface: b.clone(),
                            })
                        })
                        .collect();
                    overloads.push(FnOverload {
                        generic_params: f.generic_params.iter().map(|g| g.name.clone()).collect(),
                        generic_bounds,
                        params,
                        ret,
                        mangled_name,
                        span: f.span,
                    });
                }
                env.define(name, Symbol::FnOverloadSet { overloads });
            }
        }
    }

    // Pass 1c: register struct fields (and builtin method decls) into the registry.
    for item in &source.items {
        if let Item::Struct(s) = item {
            if s.is_builtin {
                // Builtin structs declare their methods as FnDecl entries.
                // Register them so the analyzer can type-check calls, using the
                // real source spans from the prelude file.
                let has_generics = !s.generic_params.is_empty();
                if has_generics {
                    env.push_scope();
                    for gp in &s.generic_params {
                        env.define(
                            &gp.name,
                            Symbol::Type {
                                id: TypeId(0),
                                span: gp.span,
                            },
                        );
                    }
                }
                for decl in &s.decls {
                    let params: Vec<(String, Ty)> = decl
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                        .collect();
                    let ret = resolve_type_expr(&decl.return_type, &env, &mut errors);
                    let qualified_fn = format!("{}_{}", s.name, decl.name);
                    registry.register_method(
                        &s.name,
                        ty::MethodEntry {
                            method_name: decl.name.clone(),
                            qualified_fn,
                            params,
                            ret,
                        },
                    );
                }
                if has_generics {
                    env.pop_scope();
                }
            } else {
                let fields: Vec<(String, Ty)> = s
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), resolve_type_expr(&f.ty, &env, &mut errors)))
                    .collect();
                registry.register_struct_fields(&s.name, fields);
            }
        }
    }

    // Pass 1d: register impl block methods and conformances into the registry.
    for item in &source.items {
        if let Item::ImplBlock(impl_block) = item {
            let type_name = match &impl_block.for_type {
                TypeExpr::Named { name, .. } => name.clone(),
                _ => continue,
            };
            let iface_name = match &impl_block.interface {
                TypeExpr::Named { name, .. } => name.clone(),
                _ => String::new(),
            };

            // Register conformance entry. Bounds come from impl-level generic params.
            if !iface_name.is_empty() {
                let bounds: Vec<(String, String)> = impl_block
                    .generic_params
                    .iter()
                    .flat_map(|gp| gp.bounds.iter().map(move |b| (gp.name.clone(), b.clone())))
                    .collect();
                registry.register_conformance(
                    &type_name,
                    &iface_name,
                    ty::ConformanceEntry { bounds },
                );
            }

            // Push scope for generic params. Prefer explicit impl-level params; fall
            // back to extracting them from the for_type generics list.
            let scope_params: Vec<String> = if !impl_block.generic_params.is_empty() {
                impl_block
                    .generic_params
                    .iter()
                    .map(|g| g.name.clone())
                    .collect()
            } else {
                match &impl_block.for_type {
                    TypeExpr::Named { generics, .. } => generics
                        .iter()
                        .filter_map(|g| {
                            if let TypeExpr::Named { name, .. } = g {
                                Some(name.clone())
                            } else {
                                None
                            }
                        })
                        .collect(),
                    _ => vec![],
                }
            };
            let has_generics = !scope_params.is_empty();
            if has_generics {
                env.push_scope();
                let dummy_span = impl_block.span;
                for gname in &scope_params {
                    env.define(
                        gname,
                        Symbol::Type {
                            id: TypeId(0),
                            span: dummy_span,
                        },
                    );
                }
            }
            for method in &impl_block.methods {
                let params: Vec<(String, Ty)> = method
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                    .collect();
                let ret = resolve_type_expr(&method.return_type, &env, &mut errors);
                let qualified_fn = format!("{}_{}", type_name, method.name);
                registry.register_method(
                    &type_name,
                    MethodEntry {
                        method_name: method.name.clone(),
                        qualified_fn,
                        params,
                        ret,
                    },
                );
            }
            for hook in &impl_block.hooks {
                let hook_name = match &hook.name {
                    HookName::Named(n) => n.clone(),
                    HookName::Op(op) => op.clone(),
                };
                let params: Vec<(String, Ty)> = hook
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                    .collect();
                let ret = hook
                    .return_type
                    .as_ref()
                    .map(|r| resolve_type_expr(r, &env, &mut errors))
                    .unwrap_or(Ty::Void);
                let qualified_fn = format!("{}__hook__{}", type_name, hook_name);
                registry.register_method(
                    &type_name,
                    MethodEntry {
                        method_name: hook_name,
                        qualified_fn,
                        params,
                        ret,
                    },
                );
            }
            if has_generics {
                env.pop_scope();
            }
        }
    }

    let interfaces: Vec<_> = source
        .items
        .iter()
        .filter_map(|i| {
            if let Item::Interface(iface) = i {
                Some(iface.clone())
            } else {
                None
            }
        })
        .collect();

    let all_impls: Vec<_> = source
        .items
        .iter()
        .filter_map(|i| {
            if let Item::ImplBlock(b) = i {
                Some(b.clone())
            } else {
                None
            }
        })
        .collect();

    // Pass 1e: register interface method/hook signatures for use by infer.rs.
    for item in &source.items {
        if let Item::Interface(iface) = item {
            use crate::parser::ast::InterfaceItemKind;
            // Push Self, generic params, and associated types so that hook/method
            // signatures using them (e.g. `-> Output`, `rhs: Self`) resolve cleanly.
            env.push_scope();
            let dummy_span = iface.span;
            env.define(
                "Self",
                Symbol::Type {
                    id: TypeId(0),
                    span: dummy_span,
                },
            );
            for gp in &iface.generic_params {
                env.define(
                    &gp.name,
                    Symbol::Type {
                        id: TypeId(0),
                        span: gp.span,
                    },
                );
            }
            for iitem in &iface.items {
                if let InterfaceItemKind::AssocType { name, .. } = &iitem.kind {
                    env.define(
                        name,
                        Symbol::Type {
                            id: TypeId(0),
                            span: iitem.span,
                        },
                    );
                }
            }
            for iitem in &iface.items {
                match &iitem.kind {
                    InterfaceItemKind::Method(method) => {
                        let params: Vec<(String, Ty)> = method
                            .params
                            .iter()
                            .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                            .collect();
                        let ret = resolve_type_expr(&method.return_type, &env, &mut errors);
                        registry.register_interface_method(
                            &iface.name,
                            MethodEntry {
                                method_name: method.name.clone(),
                                qualified_fn: method.name.clone(),
                                params,
                                ret,
                            },
                        );
                    }
                    InterfaceItemKind::Hook {
                        name,
                        params,
                        return_type,
                        ..
                    } => {
                        let hook_name = match name {
                            HookName::Named(n) => n.clone(),
                            HookName::Op(op) => op.clone(),
                        };
                        let resolved_params: Vec<(String, Ty)> = params
                            .iter()
                            .map(|p| (p.name.clone(), resolve_type_expr(&p.ty, &env, &mut errors)))
                            .collect();
                        let ret = return_type
                            .as_ref()
                            .map(|r| resolve_type_expr(r, &env, &mut errors))
                            .unwrap_or(Ty::Void);
                        registry.register_interface_method(
                            &iface.name,
                            MethodEntry {
                                method_name: hook_name.clone(),
                                qualified_fn: hook_name,
                                params: resolved_params,
                                ret,
                            },
                        );
                    }
                    InterfaceItemKind::Field { .. } => {}
                    InterfaceItemKind::AssocType { .. } => {}
                }
            }
            env.pop_scope();
        }
    }

    // Pass 2: check each item and produce typed items.
    let mut typed_items: Vec<TypedItem> = Vec::new();
    let mut plain_impls_seen: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for item in &source.items {
        match item {
            Item::Function(f) => {
                // Determine the mangled name: bare name for single-def functions,
                // "name__N" for overloads.
                let mangled_name = match env.lookup(&f.name) {
                    Some(Symbol::FnOverloadSet { overloads }) => overloads
                        .iter()
                        .find(|o| o.span == f.span)
                        .map(|o| o.mangled_name.clone())
                        .unwrap_or(f.name.clone()),
                    _ => f.name.clone(),
                };

                let has_generics = !f.generic_params.is_empty();
                if has_generics {
                    env.push_scope();
                    for gp in &f.generic_params {
                        env.define(
                            &gp.name,
                            Symbol::Type {
                                id: TypeId(0),
                                span: gp.span,
                            },
                        );
                    }
                }
                let ret = resolve_type_expr(&f.return_type, &env, &mut errors);
                let mut params: Vec<TypedParam> = Vec::new();
                env.push_scope();
                for p in &f.params {
                    let pty = resolve_type_expr(&p.ty, &env, &mut errors);
                    env.define(
                        &p.name,
                        Symbol::Var {
                            ty: pty.clone(),
                            mutable: false,
                            span: p.span,
                        },
                    );
                    params.push(TypedParam {
                        name: p.name.clone(),
                        ty: pty,
                        span: p.span,
                    });
                }
                let body =
                    check::check_typed_block(&f.body, &mut env, &registry, &ret, &mut errors);
                env.pop_scope();
                if has_generics {
                    env.pop_scope();
                }
                if ret != Ty::Void && !f.body.stmts.is_empty() && !returns::always_returns(&f.body)
                {
                    errors.push(AnalysisError::MissingReturn {
                        name: f.name.clone(),
                        span: f.span,
                    });
                }
                typed_items.push(TypedItem::Function(TypedFnDef {
                    name: mangled_name,
                    params,
                    variadic: f.variadic.as_ref().map(|v| v.name.clone()),
                    return_type: ret,
                    body,
                    is_builtin: f.annotations.iter().any(|a| a.name == "builtin"),
                    span: f.span,
                }));
            }

            Item::Struct(s) => {
                if !s.is_builtin {
                    conformance::check_struct_conformance(s, &interfaces, &all_impls, &mut errors);
                }
                let fields: Vec<TypedField> = s
                    .fields
                    .iter()
                    .map(|f| TypedField {
                        name: f.name.clone(),
                        ty: resolve_type_expr(&f.ty, &env, &mut errors),
                        is_priv: f.is_priv,
                        span: f.span,
                    })
                    .collect();
                typed_items.push(TypedItem::Struct(TypedStructDef {
                    name: s.name.clone(),
                    is_builtin: s.is_builtin,
                    fields,
                    span: s.span,
                }));
            }

            Item::Enum(e) => {
                conformance::check_enum_conformance(e, &interfaces, &all_impls, &mut errors);
                let variants: Vec<TypedEnumVariant> = e
                    .variants
                    .iter()
                    .map(|v| TypedEnumVariant {
                        name: v.name.clone(),
                        fields: v
                            .fields
                            .iter()
                            .map(|f| TypedField {
                                name: f.name.clone(),
                                ty: resolve_type_expr(&f.ty, &env, &mut errors),
                                is_priv: f.is_priv,
                                span: f.span,
                            })
                            .collect(),
                        discriminant: v.discriminant,
                        span: v.span,
                    })
                    .collect();
                typed_items.push(TypedItem::Enum(TypedEnumDef {
                    name: e.name.clone(),
                    variants,
                    span: e.span,
                }));
            }

            Item::ImplBlock(impl_block) => {
                conformance::check_impl_completeness(impl_block, &interfaces, &mut errors);
                if impl_block.kind == ImplKind::Plain {
                    let key_ty = match &impl_block.for_type {
                        TypeExpr::Named { name, .. } => name.clone(),
                        _ => String::new(),
                    };
                    let key_iface = match &impl_block.interface {
                        TypeExpr::Named { name, .. } => name.clone(),
                        _ => String::new(),
                    };
                    if !key_ty.is_empty() && !key_iface.is_empty() {
                        let key = (key_ty.clone(), key_iface.clone());
                        if plain_impls_seen.contains(&key) {
                            errors.push(error::AnalysisError::DuplicateImpl {
                                ty: key_ty,
                                iface: key_iface,
                                span: impl_block.span,
                            });
                        } else {
                            plain_impls_seen.insert(key);
                        }
                    }
                }
                let type_name = match &impl_block.for_type {
                    TypeExpr::Named { name, .. } => name.clone(),
                    _ => "Unknown".to_string(),
                };
                let interface_name = match &impl_block.interface {
                    TypeExpr::Named { name, .. } => name.clone(),
                    _ => String::new(),
                };

                let scope_params: Vec<String> = if !impl_block.generic_params.is_empty() {
                    impl_block
                        .generic_params
                        .iter()
                        .map(|g| g.name.clone())
                        .collect()
                } else {
                    match &impl_block.for_type {
                        TypeExpr::Named { generics, .. } => generics
                            .iter()
                            .filter_map(|g| {
                                if let TypeExpr::Named { name, .. } = g {
                                    Some(name.clone())
                                } else {
                                    None
                                }
                            })
                            .collect(),
                        _ => vec![],
                    }
                };
                let has_impl_generics = !scope_params.is_empty();
                if has_impl_generics {
                    env.push_scope();
                    let dummy_span = impl_block.span;
                    for gname in &scope_params {
                        env.define(
                            gname,
                            Symbol::Type {
                                id: TypeId(0),
                                span: dummy_span,
                            },
                        );
                    }
                }

                // Push `Self` and all associated type names from the interface into scope
                // so that hook/method signatures like `-> Output` resolve without errors.
                let dummy_span = impl_block.span;
                env.push_scope();
                env.define(
                    "Self",
                    Symbol::Type {
                        id: TypeId(0),
                        span: dummy_span,
                    },
                );
                if let Some(iface_def) = interfaces.iter().find(|i| i.name == interface_name) {
                    use crate::parser::ast::InterfaceItemKind;
                    for iitem in &iface_def.items {
                        if let InterfaceItemKind::AssocType { name, .. } = &iitem.kind {
                            env.define(
                                name,
                                Symbol::Type {
                                    id: TypeId(0),
                                    span: iitem.span,
                                },
                            );
                        }
                    }
                }

                let self_ty = resolve_type_expr(&impl_block.for_type, &env, &mut errors);

                let mut typed_methods: Vec<TypedFnDef> = Vec::new();
                for method in &impl_block.methods {
                    env.push_scope();
                    env.define(
                        "self",
                        Symbol::Var {
                            ty: self_ty.clone(),
                            mutable: false,
                            span: impl_block.span,
                        },
                    );
                    let ret = resolve_type_expr(&method.return_type, &env, &mut errors);
                    let mut params: Vec<TypedParam> = Vec::new();
                    for p in &method.params {
                        let pty = resolve_type_expr(&p.ty, &env, &mut errors);
                        env.define(
                            &p.name,
                            Symbol::Var {
                                ty: pty.clone(),
                                mutable: false,
                                span: p.span,
                            },
                        );
                        params.push(TypedParam {
                            name: p.name.clone(),
                            ty: pty,
                            span: p.span,
                        });
                    }
                    let body = check::check_typed_block(
                        &method.body,
                        &mut env,
                        &registry,
                        &ret,
                        &mut errors,
                    );
                    env.pop_scope();
                    typed_methods.push(TypedFnDef {
                        name: method.name.clone(),
                        params,
                        variadic: method.variadic.as_ref().map(|v| v.name.clone()),
                        return_type: ret,
                        body,
                        is_builtin: false,
                        span: method.span,
                    });
                }

                let mut typed_hooks: Vec<TypedHookDef> = Vec::new();
                for hook in &impl_block.hooks {
                    env.push_scope();
                    env.define(
                        "self",
                        Symbol::Var {
                            ty: self_ty.clone(),
                            mutable: false,
                            span: impl_block.span,
                        },
                    );
                    let ret = hook
                        .return_type
                        .as_ref()
                        .map(|r| resolve_type_expr(r, &env, &mut errors))
                        .unwrap_or(Ty::Void);
                    let mut params: Vec<TypedParam> = Vec::new();
                    for p in &hook.params {
                        let pty = resolve_type_expr(&p.ty, &env, &mut errors);
                        env.define(
                            &p.name,
                            Symbol::Var {
                                ty: pty.clone(),
                                mutable: false,
                                span: p.span,
                            },
                        );
                        params.push(TypedParam {
                            name: p.name.clone(),
                            ty: pty,
                            span: p.span,
                        });
                    }
                    let body = check::check_typed_block(
                        &hook.body,
                        &mut env,
                        &registry,
                        &ret,
                        &mut errors,
                    );
                    env.pop_scope();
                    typed_hooks.push(TypedHookDef {
                        name: hook.name.clone(),
                        params,
                        return_type: ret,
                        body,
                        span: hook.span,
                    });
                }

                env.pop_scope(); // assoc-types + Self scope
                if has_impl_generics {
                    env.pop_scope();
                }
                typed_items.push(TypedItem::ImplBlock(TypedImplBlock {
                    interface: interface_name,
                    for_type: type_name,
                    kind: impl_block.kind.clone(),
                    methods: typed_methods,
                    hooks: typed_hooks,
                    span: impl_block.span,
                }));
            }

            Item::Interface(iface) => {
                use crate::parser::ast::InterfaceItemKind;
                env.push_scope();
                let dummy_span = iface.span;
                // `Self` stands for the implementing type inside an interface body.
                env.define(
                    "Self",
                    Symbol::Type {
                        id: TypeId(0),
                        span: dummy_span,
                    },
                );
                // Generic type params declared on the interface (e.g. `interface Add[Rhs]`).
                for gp in &iface.generic_params {
                    env.define(
                        &gp.name,
                        Symbol::Type {
                            id: TypeId(0),
                            span: gp.span,
                        },
                    );
                }
                // Associated types declared in the interface body.
                for iitem in &iface.items {
                    if let InterfaceItemKind::AssocType { name, .. } = &iitem.kind {
                        env.define(
                            name,
                            Symbol::Type {
                                id: TypeId(0),
                                span: iitem.span,
                            },
                        );
                    }
                }
                let typed_methods: Vec<TypedInterfaceMethod> = iface
                    .items
                    .iter()
                    .filter_map(|iitem| {
                        if let InterfaceItemKind::Method(m) = &iitem.kind {
                            let params: Vec<TypedParam> = m
                                .params
                                .iter()
                                .map(|p| TypedParam {
                                    name: p.name.clone(),
                                    ty: resolve_type_expr(&p.ty, &env, &mut errors),
                                    span: p.span,
                                })
                                .collect();
                            let return_type = resolve_type_expr(&m.return_type, &env, &mut errors);
                            Some(TypedInterfaceMethod {
                                name: m.name.clone(),
                                params,
                                return_type,
                                span: m.span,
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                env.pop_scope();
                typed_items.push(TypedItem::Interface(TypedInterfaceDef {
                    name: iface.name.clone(),
                    methods: typed_methods,
                    span: iface.span,
                }));
            }

            _ => {}
        }
    }

    let typed_file = TypedFile {
        items: typed_items,
        span: source.span,
    };

    // Pass 3: constraint collection + solving (interface bound checks).
    let constraints = constrain::collect_constraints(&typed_file);
    errors.extend(solve::solve(&constraints, &registry));

    // Pass 4: object safety — check that interfaces used as dynamic types are object-safe.
    {
        use crate::parser::ast::InterfaceItemKind;
        // Compute which interfaces are NOT object-safe: any method that returns Self
        // or has Self in a parameter type position makes the interface non-object-safe.
        let non_object_safe: std::collections::HashMap<String, String> = interfaces
            .iter()
            .filter_map(|iface| {
                for iitem in &iface.items {
                    if let InterfaceItemKind::Method(m) = &iitem.kind {
                        if type_expr_uses_self(&m.return_type)
                            || m.params.iter().any(|p| type_expr_uses_self(&p.ty))
                        {
                            return Some((iface.name.clone(), m.name.clone()));
                        }
                    }
                }
                None
            })
            .collect();
        // Scan all function params in the typed file for dynamic interface types.
        for item in &typed_file.items {
            match item {
                TypedItem::Function(f) => {
                    check_params_object_safe(&f.params, &non_object_safe, &mut errors, f.span);
                }
                TypedItem::ImplBlock(ib) => {
                    for m in &ib.methods {
                        check_params_object_safe(&m.params, &non_object_safe, &mut errors, m.span);
                    }
                }
                _ => {}
            }
        }
    }

    if errors.is_empty() {
        Ok(typed_file)
    } else {
        Err(errors)
    }
}

fn type_expr_uses_self(ty: &TypeExpr) -> bool {
    match ty {
        TypeExpr::Named { name, generics, .. } => {
            name == "Self" || generics.iter().any(type_expr_uses_self)
        }
        TypeExpr::Projection { base, .. } => base == "Self",
        TypeExpr::Union(vs, _) => vs.iter().any(type_expr_uses_self),
        TypeExpr::Tuple(ts, _) => ts.iter().any(type_expr_uses_self),
        TypeExpr::Callable { params, ret, .. } => {
            params.iter().any(type_expr_uses_self) || type_expr_uses_self(ret)
        }
        TypeExpr::Ref { inner, .. } => type_expr_uses_self(inner),
        TypeExpr::GenSplice(..) => false,
    }
}

fn check_params_object_safe(
    params: &[typed_ast::TypedParam],
    non_object_safe: &std::collections::HashMap<String, String>,
    errors: &mut Vec<error::AnalysisError>,
    span: Span,
) {
    for p in params {
        if let ty::Ty::Interface(_, ref iface_name) = p.ty {
            if let Some(method_name) = non_object_safe.get(iface_name) {
                errors.push(error::AnalysisError::NonObjectSafeInterface {
                    iface: iface_name.clone(),
                    method: method_name.clone(),
                    span,
                });
            }
        }
    }
}
