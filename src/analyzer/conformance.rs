use crate::analyzer::error::AnalysisError;
use crate::diagnostics::Span;
use crate::parser::ast::{
    EnumDef, Field, FnDef, HookDef, ImplBlock, InterfaceDef, InterfaceItemKind, StructDef, TypeExpr,
};

/// Check that `st` satisfies every interface it declares.
pub fn check_struct_conformance(
    st: &StructDef,
    all_interfaces: &[InterfaceDef],
    all_impls: &[ImplBlock],
    errors: &mut Vec<AnalysisError>,
) {
    for iface_ty in &st.interfaces {
        let iface_name = type_expr_name(iface_ty);
        let hooks = collect_impl_hooks(&st.name, &iface_name, all_impls);
        check_against_iface(
            &st.name,
            &st.fields,
            &st.methods,
            &hooks,
            &iface_name,
            all_interfaces,
            all_impls,
            &st.span,
            errors,
        );
    }
}

/// Check that `en` satisfies every interface it declares.
pub fn check_enum_conformance(
    en: &EnumDef,
    all_interfaces: &[InterfaceDef],
    all_impls: &[ImplBlock],
    errors: &mut Vec<AnalysisError>,
) {
    for iface_ty in &en.interfaces {
        let iface_name = type_expr_name(iface_ty);
        let hooks = collect_impl_hooks(&en.name, &iface_name, all_impls);
        check_against_iface(
            &en.name,
            &[],
            &en.methods,
            &hooks,
            &iface_name,
            all_interfaces,
            all_impls,
            &en.span,
            errors,
        );
    }
}

/// Returns the display name for a TypeExpr, substituting `Self` and generic params.
fn type_expr_display(
    ty: &TypeExpr,
    self_name: &str,
    subst: &std::collections::HashMap<String, String>,
) -> String {
    match ty {
        TypeExpr::Named { name, .. } if name == "Self" => self_name.to_string(),
        TypeExpr::Named { name, .. } => subst
            .get(name.as_str())
            .cloned()
            .unwrap_or_else(|| name.clone()),
        _ => format!("{ty:?}"),
    }
}

/// Returns true if the impl's TypeExpr is compatible with the interface's TypeExpr.
/// `Self` and generic params in the interface are substituted before comparison.
/// Names in `assoc_types` are treated as wildcards (the impl may use any concrete type).
fn hook_param_ty_compatible(
    impl_ty: &TypeExpr,
    iface_ty: &TypeExpr,
    type_name: &str,
    self_alias: &Option<String>,
    subst: &std::collections::HashMap<String, String>,
    assoc_types: &std::collections::HashSet<String>,
) -> bool {
    let iface_name = match iface_ty {
        TypeExpr::Named { name, .. } => name.as_str(),
        _ => return true,
    };
    let impl_name = match impl_ty {
        TypeExpr::Named { name, .. } => name.as_str(),
        _ => return false,
    };
    // Associated types are wildcards: the impl defines them, so any concrete type is valid.
    if assoc_types.contains(iface_name) {
        return true;
    }
    // Resolve the interface-side name through Self and generic param substitution.
    let resolved_iface = if iface_name == "Self" {
        type_name
    } else if let Some(concrete) = subst.get(iface_name) {
        concrete.as_str()
    } else {
        iface_name
    };
    // impl may write `Self`, the concrete type name, or the self-alias
    let impl_matches = |expected: &str| {
        impl_name == expected
            || (expected == type_name
                && (impl_name == "Self" || self_alias.as_deref() == Some(impl_name)))
    };
    impl_matches(resolved_iface)
}

/// Check that an impl block provides all required items from the interface it claims to implement,
/// including verifying associated-type bindings declared in the interface's extends clause.
pub fn check_impl_completeness(
    impl_block: &ImplBlock,
    all_interfaces: &[InterfaceDef],
    all_impls: &[ImplBlock],
    errors: &mut Vec<AnalysisError>,
) {
    let iface_name = type_expr_name(&impl_block.interface);
    let type_name = type_expr_name(&impl_block.for_type);
    let iface = match all_interfaces.iter().find(|i| i.name == iface_name) {
        Some(i) => i,
        None => return,
    };

    // Build a substitution map from the interface's generic param names to the concrete
    // type arguments supplied in the impl's interface type expression.
    // e.g. `impl AddableWith[int] for Item` gives { "Rhs" -> "int" }.
    let generic_subst: std::collections::HashMap<String, String> = {
        let iface_args = match &impl_block.interface {
            TypeExpr::Named { generics, .. } => generics.as_slice(),
            _ => &[],
        };
        iface
            .generic_params
            .iter()
            .zip(iface_args.iter())
            .filter_map(|(param, arg)| {
                if let TypeExpr::Named { name: arg_name, .. } = arg {
                    Some((param.name.clone(), arg_name.clone()))
                } else {
                    None
                }
            })
            .collect()
    };

    // Collect associated type names so they can be treated as wildcards in signature checks.
    let assoc_type_names: std::collections::HashSet<String> = iface
        .items
        .iter()
        .filter_map(|item| {
            if let InterfaceItemKind::AssocType { name, .. } = &item.kind {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();

    // Check that required hooks/methods are present and have compatible signatures.
    for item in &iface.items {
        match &item.kind {
            InterfaceItemKind::Hook {
                name,
                params: iface_params,
                return_type: iface_ret,
                default,
                ..
            } if default.is_none() => {
                let hook_label = match name {
                    crate::parser::ast::HookName::Op(s) => s.clone(),
                    crate::parser::ast::HookName::Named(s) => s.clone(),
                };
                match impl_block.hooks.iter().find(|h| &h.name == name) {
                    None => {
                        errors.push(AnalysisError::MissingConformance {
                            ty: type_name.clone(),
                            iface: iface_name.clone(),
                            detail: format!("impl block missing required hook `{hook_label}`"),
                            span: impl_block.span,
                        });
                    }
                    Some(impl_hook) => {
                        // Check parameter count.
                        if impl_hook.params.len() != iface_params.len() {
                            errors.push(AnalysisError::MissingConformance {
                                ty: type_name.clone(),
                                iface: iface_name.clone(),
                                detail: format!(
                                    "hook `{hook_label}` has {} parameter(s) but `{}` requires {}",
                                    impl_hook.params.len(),
                                    iface_name,
                                    iface_params.len()
                                ),
                                span: impl_hook.span,
                            });
                        } else {
                            for (impl_p, iface_p) in
                                impl_hook.params.iter().zip(iface_params.iter())
                            {
                                if !hook_param_ty_compatible(
                                    &impl_p.ty,
                                    &iface_p.ty,
                                    &type_name,
                                    &impl_block.self_alias,
                                    &generic_subst,
                                    &assoc_type_names,
                                ) {
                                    errors.push(AnalysisError::MissingConformance {
                                        ty: type_name.clone(),
                                        iface: iface_name.clone(),
                                        detail: format!(
                                            "hook `{hook_label}` parameter `{}`: expected `{}`, found `{}`",
                                            impl_p.name,
                                            type_expr_display(&iface_p.ty, &type_name, &generic_subst),
                                            type_expr_display(&impl_p.ty, &type_name, &generic_subst),
                                        ),
                                        span: impl_hook.span,
                                    });
                                }
                            }
                            if let Some(iface_ret_ty) = iface_ret {
                                if let Some(impl_ret_ty) = &impl_hook.return_type {
                                    if !hook_param_ty_compatible(
                                        impl_ret_ty,
                                        iface_ret_ty,
                                        &type_name,
                                        &impl_block.self_alias,
                                        &generic_subst,
                                        &assoc_type_names,
                                    ) {
                                        errors.push(AnalysisError::MissingConformance {
                                            ty: type_name.clone(),
                                            iface: iface_name.clone(),
                                            detail: format!(
                                                "hook `{hook_label}` return type: expected `{}`, found `{}`",
                                                type_expr_display(iface_ret_ty, &type_name, &generic_subst),
                                                type_expr_display(impl_ret_ty, &type_name, &generic_subst),
                                            ),
                                            span: impl_hook.span,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            InterfaceItemKind::Method(sig) if sig.body.stmts.is_empty() => {
                if !impl_block.methods.iter().any(|m| m.name == sig.name) {
                    errors.push(AnalysisError::MissingConformance {
                        ty: type_name.clone(),
                        iface: iface_name.clone(),
                        detail: format!("impl block missing required method `{}`", sig.name),
                        span: impl_block.span,
                    });
                }
            }
            _ => {}
        }
    }

    // Check parent interfaces transitively, using ALL hooks/methods from ALL
    // impl blocks for this type so that hooks split across multiple impl blocks
    // (e.g. hook lives in impl WithHook rather than impl Wrapper) are found.
    let all_hooks = collect_all_hooks_for_type(&type_name, all_impls);
    let all_methods = collect_all_methods_for_type(&type_name, all_impls);

    for parent_ty in &iface.extends {
        let pname = type_expr_name(parent_ty);

        if let TypeExpr::Named { bindings, .. } = parent_ty {
            if !bindings.is_empty() {
                check_assoc_bindings(
                    &type_name,
                    &pname,
                    bindings,
                    all_interfaces,
                    all_impls,
                    &impl_block.span,
                    errors,
                );
            }
        }

        check_against_iface(
            &type_name,
            &[],
            &all_methods,
            &all_hooks,
            &pname,
            all_interfaces,
            all_impls,
            &impl_block.span,
            errors,
        );
    }
}

/// Verify that, for a type implementing an interface that extends `parent_iface` with
/// `bindings` (e.g. `Output=Self`), the type's impl of `parent_iface` actually satisfies
/// those bindings.
///
/// For each binding `(assoc_name, bound_ty)`:
///   - Resolve `bound_ty` by substituting `Self` -> `type_name`.
///   - Find the hook in `parent_iface` whose declared return type equals `assoc_name`.
///   - Look up the return type of that hook in `type_name`'s impl of `parent_iface`.
///   - Require that it equals the resolved binding type.
fn check_assoc_bindings(
    type_name: &str,
    parent_iface_name: &str,
    bindings: &[(String, TypeExpr)],
    all_interfaces: &[InterfaceDef],
    all_impls: &[ImplBlock],
    span: &Span,
    errors: &mut Vec<AnalysisError>,
) {
    let parent_iface = match all_interfaces.iter().find(|i| i.name == parent_iface_name) {
        Some(i) => i,
        None => return,
    };

    // Find the impl of parent_iface for type_name.
    let impl_block = all_impls.iter().find(|b| {
        type_expr_name(&b.for_type) == type_name
            && type_expr_name(&b.interface) == parent_iface_name
    });

    for (assoc_name, bound_ty_expr) in bindings {
        // Resolve bound type: substitute "Self" -> type_name.
        let expected = resolve_binding_name(bound_ty_expr, type_name);

        // Find which hook in parent_iface declares its return type as the assoc type name.
        for item in &parent_iface.items {
            let (hook_name, ret_ty_expr) = match &item.kind {
                InterfaceItemKind::Hook {
                    name,
                    return_type: Some(ret),
                    ..
                } => (name, ret),
                _ => continue,
            };

            let ret_ty_name = match ret_ty_expr {
                TypeExpr::Named { name, .. } => name.as_str(),
                _ => continue,
            };

            if ret_ty_name != assoc_name.as_str() {
                continue;
            }

            // This hook defines the assoc type. Check the impl's version.
            if let Some(b) = impl_block {
                if let Some(h) = b.hooks.iter().find(|h| &h.name == hook_name) {
                    let actual = h
                        .return_type
                        .as_ref()
                        .map(|t| resolve_binding_name(t, type_name))
                        .unwrap_or_else(|| "void".to_string());

                    if actual != expected {
                        errors.push(AnalysisError::MissingConformance {
                            ty: type_name.to_string(),
                            iface: parent_iface_name.to_string(),
                            detail: format!(
                                "associated type `{assoc_name}` must be `{expected}`, found `{actual}`"
                            ),
                            span: *span,
                        });
                    }
                }
            }
        }
    }
}

/// Resolve a binding type expression to a concrete name string, substituting `Self` -> `self_ty`.
fn resolve_binding_name(ty: &TypeExpr, self_ty: &str) -> String {
    match ty {
        TypeExpr::Named { name, .. } => {
            if name == "Self" {
                self_ty.to_string()
            } else {
                name.clone()
            }
        }
        _ => "<complex>".to_string(),
    }
}

fn collect_impl_hooks(type_name: &str, iface_name: &str, all_impls: &[ImplBlock]) -> Vec<HookDef> {
    for impl_block in all_impls {
        let impl_type = type_expr_name(&impl_block.for_type);
        let impl_iface = type_expr_name(&impl_block.interface);
        if impl_type == type_name && impl_iface == iface_name {
            return impl_block.hooks.clone();
        }
    }
    vec![]
}

fn collect_all_hooks_for_type(type_name: &str, all_impls: &[ImplBlock]) -> Vec<HookDef> {
    all_impls
        .iter()
        .filter(|b| type_expr_name(&b.for_type) == type_name)
        .flat_map(|b| b.hooks.iter().cloned())
        .collect()
}

fn collect_all_methods_for_type(type_name: &str, all_impls: &[ImplBlock]) -> Vec<FnDef> {
    all_impls
        .iter()
        .filter(|b| type_expr_name(&b.for_type) == type_name)
        .flat_map(|b| b.methods.iter().cloned())
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn check_against_iface(
    type_name: &str,
    fields: &[Field],
    methods: &[FnDef],
    hooks: &[HookDef],
    iface_name: &str,
    all_interfaces: &[InterfaceDef],
    all_impls: &[ImplBlock],
    span: &Span,
    errors: &mut Vec<AnalysisError>,
) {
    let iface = match all_interfaces.iter().find(|i| i.name == iface_name) {
        Some(i) => i,
        None => {
            errors.push(AnalysisError::MissingConformance {
                ty: type_name.to_string(),
                iface: iface_name.to_string(),
                detail: "interface not found".into(),
                span: *span,
            });
            return;
        }
    };

    for parent in &iface.extends {
        let pname = type_expr_name(parent);

        // Check associated-type bindings on this parent reference.
        if let TypeExpr::Named { bindings, .. } = parent {
            if !bindings.is_empty() {
                check_assoc_bindings(
                    type_name,
                    &pname,
                    bindings,
                    all_interfaces,
                    all_impls,
                    span,
                    errors,
                );
            }
        }

        // Merge hooks from the current impl with hooks from the parent's own
        // impl block, so that either source can satisfy the parent's requirements.
        let mut parent_hooks = hooks.to_vec();
        parent_hooks.extend(collect_impl_hooks(type_name, &pname, all_impls));

        check_against_iface(
            type_name,
            fields,
            methods,
            &parent_hooks,
            &pname,
            all_interfaces,
            all_impls,
            span,
            errors,
        );
    }

    for item in &iface.items {
        match &item.kind {
            InterfaceItemKind::Field { name, .. } => {
                if !fields.iter().any(|f| &f.name == name) {
                    errors.push(AnalysisError::MissingConformance {
                        ty: type_name.to_string(),
                        iface: iface_name.to_string(),
                        detail: format!("missing required field `{name}`"),
                        span: *span,
                    });
                }
            }
            InterfaceItemKind::Hook { name, default, .. } if default.is_none() => {
                if !hooks.iter().any(|h| &h.name == name) {
                    errors.push(AnalysisError::MissingConformance {
                        ty: type_name.to_string(),
                        iface: iface_name.to_string(),
                        detail: format!("missing required hook `{name:?}`"),
                        span: *span,
                    });
                }
            }
            InterfaceItemKind::Method(sig) if sig.body.stmts.is_empty() => {
                if !methods.iter().any(|m| m.name == sig.name) {
                    errors.push(AnalysisError::MissingConformance {
                        ty: type_name.to_string(),
                        iface: iface_name.to_string(),
                        detail: format!("missing required method `{}`", sig.name),
                        span: *span,
                    });
                }
            }
            _ => {}
        }
    }
}

fn type_expr_name(ty: &TypeExpr) -> String {
    match ty {
        TypeExpr::Named { name, .. } => name.clone(),
        _ => "<unknown>".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Span;
    use crate::parser::ast::*;
    fn s() -> Span {
        Span { start: 0, end: 0 }
    }
    fn named(n: &str) -> TypeExpr {
        TypeExpr::Named {
            name: n.into(),
            generics: vec![],
            bindings: vec![],
            span: s(),
        }
    }

    fn exception_iface() -> InterfaceDef {
        InterfaceDef {
            name: "Exception".into(),
            generic_params: vec![],
            extends: vec![],
            items: vec![InterfaceItem {
                kind: InterfaceItemKind::Field {
                    name: "message".into(),
                    ty: named("str"),
                },
                span: s(),
            }],
            span: s(),
        }
    }

    fn addable_iface() -> InterfaceDef {
        InterfaceDef {
            name: "Addable".into(),
            generic_params: vec![],
            extends: vec![],
            items: vec![InterfaceItem {
                kind: InterfaceItemKind::Hook {
                    annotations: vec![],
                    name: HookName::Op("+".into()),
                    params: vec![],
                    return_type: None,
                    default: None,
                },
                span: s(),
            }],
            span: s(),
        }
    }

    fn point_struct() -> StructDef {
        StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "Point".into(),
            generic_params: vec![],
            interfaces: vec![named("Addable")],
            fields: vec![],
            methods: vec![],
            decls: vec![],
            span: s(),
        }
    }

    fn addable_impl_for_point() -> ImplBlock {
        ImplBlock {
            self_alias: None,
            generic_params: vec![],
            interface: named("Addable"),
            for_type: named("Point"),
            methods: vec![],
            hooks: vec![HookDef {
                annotations: vec![],
                name: HookName::Op("+".into()),
                params: vec![],
                return_type: None,
                body: Block {
                    stmts: vec![],
                    span: s(),
                },
                span: s(),
            }],
            assoc_bindings: vec![],
            kind: ImplKind::Plain,
            span: s(),
        }
    }

    #[test]
    fn ok_when_required_field_present() {
        let st = StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "ValueError".into(),
            generic_params: vec![],
            interfaces: vec![named("Exception")],
            fields: vec![Field {
                annotations: vec![],
                is_priv: false,
                name: "message".into(),
                ty: named("str"),
                default: None,
                span: s(),
            }],
            methods: vec![],
            decls: vec![],
            span: s(),
        };
        let mut errs = vec![];
        check_struct_conformance(&st, &[exception_iface()], &[], &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn error_when_required_field_missing() {
        let st = StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "BadError".into(),
            generic_params: vec![],
            interfaces: vec![named("Exception")],
            fields: vec![],
            methods: vec![],
            decls: vec![],
            span: s(),
        };
        let mut errs = vec![];
        check_struct_conformance(&st, &[exception_iface()], &[], &mut errs);
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn error_when_required_hook_missing() {
        let st = point_struct();
        let mut errs = vec![];
        check_struct_conformance(&st, &[addable_iface()], &[], &mut errs);
        assert_eq!(errs.len(), 1, "expected one error for missing hook");
        let detail = match &errs[0] {
            AnalysisError::MissingConformance { detail, .. } => detail.clone(),
            _ => panic!("wrong error kind: {:?}", errs[0]),
        };
        assert!(
            detail.contains('+'),
            "detail should mention the hook name: {detail}"
        );
    }

    #[test]
    fn ok_when_required_hook_provided_in_impl_block() {
        let st = point_struct();
        let impl_block = addable_impl_for_point();
        let mut errs = vec![];
        check_struct_conformance(&st, &[addable_iface()], &[impl_block], &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn regression_bug3_hook_conformance_validated() {
        // Bug 3: check_struct_conformance always passed &[] for hooks, so a
        // completely empty impl block would not produce any error for missing hooks.
        let st = point_struct();
        let empty_impl = ImplBlock {
            self_alias: None,
            generic_params: vec![],
            interface: named("Addable"),
            for_type: named("Point"),
            methods: vec![],
            hooks: vec![],
            assoc_bindings: vec![],
            kind: ImplKind::Plain,
            span: s(),
        };
        let mut errs = vec![];
        check_struct_conformance(&st, &[addable_iface()], &[empty_impl], &mut errs);
        assert_eq!(
            errs.len(),
            1,
            "should detect missing hook even in empty impl block"
        );
    }

    fn printable_iface() -> InterfaceDef {
        InterfaceDef {
            name: "Printable".into(),
            generic_params: vec![],
            extends: vec![],
            items: vec![InterfaceItem {
                kind: InterfaceItemKind::Method(FnDef {
                    annotations: vec![],
                    name: "print".into(),
                    generic_params: vec![],
                    params: vec![],
                    variadic: None,
                    return_type: named("void"),
                    body: Block {
                        stmts: vec![],
                        span: s(),
                    },
                    is_declaration: false,
                    span: s(),
                }),
                span: s(),
            }],
            span: s(),
        }
    }

    #[test]
    fn error_when_impl_block_missing_required_hook() {
        let empty_impl = ImplBlock {
            self_alias: None,
            generic_params: vec![],
            interface: named("Addable"),
            for_type: named("Point"),
            methods: vec![],
            hooks: vec![],
            assoc_bindings: vec![],
            kind: ImplKind::Plain,
            span: s(),
        };
        let mut errs = vec![];
        check_impl_completeness(&empty_impl, &[addable_iface()], &[], &mut errs);
        assert_eq!(
            errs.len(),
            1,
            "expected error for missing hook in impl block"
        );
    }

    #[test]
    fn ok_when_impl_block_has_required_hook() {
        let impl_block = addable_impl_for_point();
        let mut errs = vec![];
        check_impl_completeness(&impl_block, &[addable_iface()], &[], &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn error_when_impl_block_missing_required_method() {
        let empty_impl = ImplBlock {
            self_alias: None,
            generic_params: vec![],
            interface: named("Printable"),
            for_type: named("Point"),
            methods: vec![],
            hooks: vec![],
            assoc_bindings: vec![],
            kind: ImplKind::Plain,
            span: s(),
        };
        let mut errs = vec![];
        check_impl_completeness(&empty_impl, &[printable_iface()], &[], &mut errs);
        assert_eq!(
            errs.len(),
            1,
            "expected error for missing method in impl block"
        );
    }

    #[test]
    fn ok_when_impl_block_has_required_method() {
        let impl_with_method = ImplBlock {
            self_alias: None,
            generic_params: vec![],
            interface: named("Printable"),
            for_type: named("Point"),
            methods: vec![FnDef {
                annotations: vec![],
                name: "print".into(),
                generic_params: vec![],
                params: vec![],
                variadic: None,
                return_type: named("void"),
                body: Block {
                    stmts: vec![],
                    span: s(),
                },
                is_declaration: false,
                span: s(),
            }],
            hooks: vec![],
            assoc_bindings: vec![],
            kind: ImplKind::Plain,
            span: s(),
        };
        let mut errs = vec![];
        check_impl_completeness(&impl_with_method, &[printable_iface()], &[], &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn regression_bug4_impl_block_completeness_validated() {
        // Bug 4: impl blocks were type-checked but never validated for completeness
        // against the interface. An empty impl block should error.
        let empty_impl = ImplBlock {
            self_alias: None,
            generic_params: vec![],
            interface: named("Addable"),
            for_type: named("Point"),
            methods: vec![],
            hooks: vec![],
            assoc_bindings: vec![],
            kind: ImplKind::Plain,
            span: s(),
        };
        let mut errs = vec![];
        check_impl_completeness(&empty_impl, &[addable_iface()], &[], &mut errs);
        assert!(
            !errs.is_empty(),
            "empty impl block should fail completeness check"
        );
    }

    // ---- Binding enforcement tests -------------------------------------------

    fn addable_with_iface() -> InterfaceDef {
        // interface AddableWith[Rhs] { type Output; hook +(rhs: Rhs) -> Output }
        InterfaceDef {
            name: "AddableWith".into(),
            generic_params: vec![GenericParam {
                kind: GenericParamKind::Type,
                name: "Rhs".into(),
                variance: Variance::Invariant,
                bounds: vec![],
                span: s(),
            }],
            extends: vec![],
            items: vec![
                InterfaceItem {
                    kind: InterfaceItemKind::AssocType {
                        name: "Output".into(),
                        bounds: vec![],
                    },
                    span: s(),
                },
                InterfaceItem {
                    kind: InterfaceItemKind::Hook {
                        annotations: vec![],
                        name: HookName::Op("+".into()),
                        params: vec![Param {
                            name: "rhs".into(),
                            ty: named("Rhs"),
                            mutable: false,
                            span: s(),
                        }],
                        return_type: Some(named("Output")),
                        default: None,
                    },
                    span: s(),
                },
            ],
            span: s(),
        }
    }

    fn closed_addable_iface() -> InterfaceDef {
        // interface Addable: AddableWith[Self, Output=Self] { hook +(rhs: Self) -> Self }
        InterfaceDef {
            name: "Addable".into(),
            generic_params: vec![],
            extends: vec![TypeExpr::Named {
                name: "AddableWith".into(),
                generics: vec![named("Self")],
                bindings: vec![("Output".into(), named("Self"))],
                span: s(),
            }],
            items: vec![InterfaceItem {
                kind: InterfaceItemKind::Hook {
                    annotations: vec![],
                    name: HookName::Op("+".into()),
                    params: vec![Param {
                        name: "rhs".into(),
                        ty: named("Self"),
                        mutable: false,
                        span: s(),
                    }],
                    return_type: Some(named("Self")),
                    default: None,
                },
                span: s(),
            }],
            span: s(),
        }
    }

    fn addable_with_impl_returning(for_ty: &str, ret: &str) -> ImplBlock {
        // impl AddableWith[for_ty] for for_ty { hook +(rhs: for_ty) -> ret {} }
        ImplBlock {
            self_alias: None,
            generic_params: vec![],
            interface: TypeExpr::Named {
                name: "AddableWith".into(),
                generics: vec![named(for_ty)],
                bindings: vec![],
                span: s(),
            },
            for_type: named(for_ty),
            methods: vec![],
            hooks: vec![HookDef {
                annotations: vec![],
                name: HookName::Op("+".into()),
                params: vec![Param {
                    name: "rhs".into(),
                    ty: named(for_ty),
                    mutable: false,
                    span: s(),
                }],
                return_type: Some(named(ret)),
                body: Block {
                    stmts: vec![],
                    span: s(),
                },
                span: s(),
            }],
            assoc_bindings: vec![],
            kind: ImplKind::Plain,
            span: s(),
        }
    }

    fn addable_impl_for(ty: &str) -> ImplBlock {
        ImplBlock {
            self_alias: None,
            generic_params: vec![],
            interface: named("Addable"),
            for_type: named(ty),
            methods: vec![],
            hooks: vec![HookDef {
                annotations: vec![],
                name: HookName::Op("+".into()),
                params: vec![Param {
                    name: "rhs".into(),
                    ty: named(ty),
                    mutable: false,
                    span: s(),
                }],
                return_type: Some(named(ty)),
                body: Block {
                    stmts: vec![],
                    span: s(),
                },
                span: s(),
            }],
            assoc_bindings: vec![],
            kind: ImplKind::Plain,
            span: s(),
        }
    }

    // ---- Parent-impl hook merge tests ------------------------------------------

    fn with_hook_iface() -> InterfaceDef {
        // interface WithHook { hook do_thing() -> void }
        InterfaceDef {
            name: "WithHook".into(),
            generic_params: vec![],
            extends: vec![],
            items: vec![InterfaceItem {
                kind: InterfaceItemKind::Hook {
                    annotations: vec![],
                    name: HookName::Named("do_thing".into()),
                    params: vec![],
                    return_type: None,
                    default: None,
                },
                span: s(),
            }],
            span: s(),
        }
    }

    fn wrapper_iface() -> InterfaceDef {
        // interface Wrapper: WithHook {}
        InterfaceDef {
            name: "Wrapper".into(),
            generic_params: vec![],
            extends: vec![named("WithHook")],
            items: vec![],
            span: s(),
        }
    }

    fn foo_struct() -> StructDef {
        StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "Foo".into(),
            generic_params: vec![],
            interfaces: vec![named("Wrapper")],
            fields: vec![],
            methods: vec![],
            decls: vec![],
            span: s(),
        }
    }

    fn with_hook_impl_for_foo() -> ImplBlock {
        // impl WithHook for Foo { hook do_thing() {} }
        ImplBlock {
            self_alias: None,
            generic_params: vec![],
            interface: named("WithHook"),
            for_type: named("Foo"),
            methods: vec![],
            hooks: vec![HookDef {
                annotations: vec![],
                name: HookName::Named("do_thing".into()),
                params: vec![],
                return_type: None,
                body: Block {
                    stmts: vec![],
                    span: s(),
                },
                span: s(),
            }],
            assoc_bindings: vec![],
            kind: ImplKind::Plain,
            span: s(),
        }
    }

    #[test]
    fn parent_iface_hooks_satisfied_by_separate_impl() {
        // struct Foo: Wrapper {}  +  impl WithHook for Foo { hook do_thing {} }
        // The hook lives in the parent-interface impl, not the child impl.
        // Conformance check should locate it and pass.
        let st = foo_struct();
        let ifaces = vec![wrapper_iface(), with_hook_iface()];
        let impls = vec![with_hook_impl_for_foo()];
        let mut errs = vec![];
        check_struct_conformance(&st, &ifaces, &impls, &mut errs);
        assert!(
            errs.is_empty(),
            "hook in parent-iface impl should satisfy parent requirement: {errs:?}"
        );
    }

    #[test]
    fn parent_iface_hooks_still_fail_when_missing() {
        // Same setup but no impl at all -- must still error.
        let st = foo_struct();
        let ifaces = vec![wrapper_iface(), with_hook_iface()];
        let mut errs = vec![];
        check_struct_conformance(&st, &ifaces, &[], &mut errs);
        assert!(
            !errs.is_empty(),
            "missing hook should still produce an error"
        );
    }

    #[test]
    fn check_impl_completeness_walks_parent_interfaces() {
        // impl Wrapper for Foo {} -- empty, but Wrapper: WithHook requires do_thing.
        // check_impl_completeness must walk parent interfaces and error when
        // do_thing is not provided in any impl block for Foo.
        let empty_wrapper_impl = ImplBlock {
            self_alias: None,
            generic_params: vec![],
            interface: named("Wrapper"),
            for_type: named("Foo"),
            methods: vec![],
            hooks: vec![],
            assoc_bindings: vec![],
            kind: ImplKind::Plain,
            span: s(),
        };
        let ifaces = vec![wrapper_iface(), with_hook_iface()];
        let mut errs = vec![];
        check_impl_completeness(&empty_wrapper_impl, &ifaces, &[], &mut errs);
        assert!(
            !errs.is_empty(),
            "check_impl_completeness must error when parent-required hook is missing"
        );
    }

    #[test]
    fn check_impl_completeness_walks_parent_interfaces_satisfied_by_separate_impl() {
        // impl Wrapper for Foo {} + impl WithHook for Foo { hook do_thing {} }
        // check_impl_completeness should pass because do_thing exists in another impl.
        let empty_wrapper_impl = ImplBlock {
            self_alias: None,
            generic_params: vec![],
            interface: named("Wrapper"),
            for_type: named("Foo"),
            methods: vec![],
            hooks: vec![],
            assoc_bindings: vec![],
            kind: ImplKind::Plain,
            span: s(),
        };
        let ifaces = vec![wrapper_iface(), with_hook_iface()];
        let impls = vec![with_hook_impl_for_foo()];
        let mut errs = vec![];
        check_impl_completeness(&empty_wrapper_impl, &ifaces, &impls, &mut errs);
        assert!(
            errs.is_empty(),
            "parent hook satisfied by separate impl should pass: {errs:?}"
        );
    }

    #[test]
    fn binding_satisfied_when_output_equals_self() {
        // impl AddableWith[MyNum] for MyNum { hook + -> MyNum } -- Output=MyNum matches Self=MyNum
        let correct_with_impl = addable_with_impl_returning("MyNum", "MyNum");
        let addable_impl = addable_impl_for("MyNum");
        let mut errs = vec![];
        check_impl_completeness(
            &addable_impl,
            &[closed_addable_iface(), addable_with_iface()],
            &[correct_with_impl],
            &mut errs,
        );
        assert!(
            errs.is_empty(),
            "Output=MyNum should satisfy Output=Self for MyNum: {errs:?}"
        );
    }

    #[test]
    fn binding_violated_when_output_differs_from_self() {
        // impl AddableWith[MyNum] for MyNum { hook + -> str } -- Output=str, but Self=MyNum
        let wrong_with_impl = addable_with_impl_returning("MyNum", "str");
        let addable_impl = addable_impl_for("MyNum");
        let mut errs = vec![];
        check_impl_completeness(
            &addable_impl,
            &[closed_addable_iface(), addable_with_iface()],
            &[wrong_with_impl],
            &mut errs,
        );
        assert_eq!(
            errs.len(),
            1,
            "Output=str violates Output=Self for MyNum: {errs:?}"
        );
        let detail = match &errs[0] {
            AnalysisError::MissingConformance { detail, .. } => detail.clone(),
            _ => panic!("wrong error kind: {:?}", errs[0]),
        };
        assert!(
            detail.contains("Output"),
            "error should mention assoc type name: {detail}"
        );
    }
}
