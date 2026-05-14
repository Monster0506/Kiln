use crate::analyzer::error::AnalysisError;
use crate::diagnostics::Span;
use crate::parser::ast::{
    EnumDef, Field, FnDef, HookDef, InterfaceDef, InterfaceItemKind, StructDef, TypeExpr,
};

/// Check that `st` satisfies every interface it declares.
pub fn check_struct_conformance(
    st: &StructDef,
    all_interfaces: &[InterfaceDef],
    errors: &mut Vec<AnalysisError>,
) {
    for iface_ty in &st.interfaces {
        let iface_name = type_expr_name(iface_ty);
        check_against_iface(
            &st.name,
            &st.fields,
            &st.methods,
            &[],
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
    errors: &mut Vec<AnalysisError>,
) {
    for iface_ty in &en.interfaces {
        let iface_name = type_expr_name(iface_ty);
        check_against_iface(
            &en.name,
            &[],
            &en.methods,
            &[],
            &iface_name,
            all_interfaces,
            &en.span,
            errors,
        );
    }
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
        check_struct_conformance(&st, &[exception_iface()], &mut errs);
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
        check_struct_conformance(&st, &[exception_iface()], &mut errs);
        assert_eq!(errs.len(), 1);
    }
}
