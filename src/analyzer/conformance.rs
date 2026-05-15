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
            &en.span,
            errors,
        );
    }
}

/// Check that an impl block provides all required items from the interface it claims to implement.
pub fn check_impl_completeness(
    impl_block: &ImplBlock,
    all_interfaces: &[InterfaceDef],
    errors: &mut Vec<AnalysisError>,
) {
    let iface_name = type_expr_name(&impl_block.interface);
    let type_name = type_expr_name(&impl_block.for_type);
    let iface = match all_interfaces.iter().find(|i| i.name == iface_name) {
        Some(i) => i,
        None => return,
    };

    for item in &iface.items {
        match &item.kind {
            InterfaceItemKind::Hook { name, default, .. } if default.is_none() => {
                if !impl_block.hooks.iter().any(|h| &h.name == name) {
                    let hook_label = match name {
                        crate::parser::ast::HookName::Op(s) => s.clone(),
                        crate::parser::ast::HookName::Named(s) => s.clone(),
                    };
                    errors.push(AnalysisError::MissingConformance {
                        ty: type_name.clone(),
                        iface: iface_name.clone(),
                        detail: format!("impl block missing required hook `{hook_label}`"),
                        span: impl_block.span,
                    });
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

fn check_against_iface(
    type_name: &str,
    fields: &[Field],
    methods: &[FnDef],
    hooks: &[HookDef],
    iface_name: &str,
    all_interfaces: &[InterfaceDef],
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
        check_against_iface(
            type_name,
            fields,
            methods,
            hooks,
            &pname,
            all_interfaces,
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
            generic_params: vec![],
            interface: named("Addable"),
            for_type: named("Point"),
            methods: vec![],
            hooks: vec![HookDef {
                name: HookName::Op("+".into()),
                params: vec![],
                return_type: None,
                body: Block {
                    stmts: vec![],
                    span: s(),
                },
                span: s(),
            }],
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
            generic_params: vec![],
            interface: named("Addable"),
            for_type: named("Point"),
            methods: vec![],
            hooks: vec![],
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
            generic_params: vec![],
            interface: named("Addable"),
            for_type: named("Point"),
            methods: vec![],
            hooks: vec![],
            kind: ImplKind::Plain,
            span: s(),
        };
        let mut errs = vec![];
        check_impl_completeness(&empty_impl, &[addable_iface()], &mut errs);
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
        check_impl_completeness(&impl_block, &[addable_iface()], &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn error_when_impl_block_missing_required_method() {
        let empty_impl = ImplBlock {
            generic_params: vec![],
            interface: named("Printable"),
            for_type: named("Point"),
            methods: vec![],
            hooks: vec![],
            kind: ImplKind::Plain,
            span: s(),
        };
        let mut errs = vec![];
        check_impl_completeness(&empty_impl, &[printable_iface()], &mut errs);
        assert_eq!(
            errs.len(),
            1,
            "expected error for missing method in impl block"
        );
    }

    #[test]
    fn ok_when_impl_block_has_required_method() {
        let impl_with_method = ImplBlock {
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
                span: s(),
            }],
            hooks: vec![],
            kind: ImplKind::Plain,
            span: s(),
        };
        let mut errs = vec![];
        check_impl_completeness(&impl_with_method, &[printable_iface()], &mut errs);
        assert!(errs.is_empty(), "{errs:?}");
    }

    #[test]
    fn regression_bug4_impl_block_completeness_validated() {
        // Bug 4: impl blocks were type-checked but never validated for completeness
        // against the interface. An empty impl block should error.
        let empty_impl = ImplBlock {
            generic_params: vec![],
            interface: named("Addable"),
            for_type: named("Point"),
            methods: vec![],
            hooks: vec![],
            kind: ImplKind::Plain,
            span: s(),
        };
        let mut errs = vec![];
        check_impl_completeness(&empty_impl, &[addable_iface()], &mut errs);
        assert!(
            !errs.is_empty(),
            "empty impl block should fail completeness check"
        );
    }
}
