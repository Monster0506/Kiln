use crate::analyzer::env::{Env, Symbol};
use crate::analyzer::error::AnalysisError;
use crate::analyzer::ty::Ty;
use crate::parser::ast::TypeExpr;

/// Maps a bare primitive type name to its `Ty` without needing analyzer context.
/// Returns `None` for non-primitive names (containers, user types, etc.).
pub fn resolve_primitive_name(name: &str) -> Option<Ty> {
    match name {
        "int" => Some(Ty::Int),
        "float" => Some(Ty::Float),
        "bool" => Some(Ty::Bool),
        "str" => Some(Ty::Str),
        "void" => Some(Ty::Void),
        _ => None,
    }
}

pub fn resolve_type_expr(
    expr: &TypeExpr,
    env: &Env,
    //    registry: &TypeRegistry,
    errors: &mut Vec<AnalysisError>,
) -> Ty {
    match expr {
        TypeExpr::Named {
            name,
            generics,
            span,
            ..
        } => {
            let resolved_generics: Vec<Ty> = generics
                .iter()
                .map(|g| resolve_type_expr(g, env, errors))
                .collect();

            if let Some(prim) = resolve_primitive_name(name) {
                return prim;
            }
            match env.lookup(name.as_str()) {
                Some(Symbol::TypeAlias(ty)) => ty.clone(),
                Some(Symbol::Type { id, .. }) if *id == crate::analyzer::ty::TypeId(0) => {
                    Ty::GenericParam(name.to_string())
                }
                Some(Symbol::Type { id, .. }) => {
                    Ty::Named(id.clone(), name.to_string(), resolved_generics)
                }
                Some(Symbol::Iface { id, .. }) => Ty::Interface(id.clone(), name.to_string()),
                _ => {
                    errors.push(AnalysisError::UndefinedName {
                        name: name.to_string(),
                        span: *span,
                    });
                    Ty::Unknown
                }
            }
        }
        TypeExpr::Tuple(elems, _) => Ty::Tuple(
            elems
                .iter()
                .map(|e| resolve_type_expr(e, env, errors))
                .collect(),
        ),
        TypeExpr::Callable { params, ret, .. } => {
            let ptys = params
                .iter()
                .map(|p| resolve_type_expr(p, env, errors))
                .collect();
            let rty = resolve_type_expr(ret, env, errors);
            Ty::Callable(ptys, Box::new(rty))
        }
        TypeExpr::Union(variants, _) => Ty::Union(
            variants
                .iter()
                .map(|v| resolve_type_expr(v, env, errors))
                .collect(),
        ),
        TypeExpr::Ref { inner, mutable, .. } => {
            Ty::Ref(Box::new(resolve_type_expr(inner, env, errors)), *mutable)
        }
        TypeExpr::Projection { base, assoc, .. } => {
            // `Base.Assoc` — treated as an opaque generic param for now.
            // Full projection resolution requires type-level evaluation.
            Ty::GenericParam(format!("{base}.{assoc}"))
        }
        TypeExpr::GenSplice(_, span) => {
            errors.push(AnalysisError::UndefinedName {
                name: "<gen-splice>".into(),
                span: *span,
            });
            Ty::Unknown
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::env::Env;
    use crate::analyzer::ty::Ty;
    use crate::diagnostics::Span;
    use crate::parser::ast::TypeExpr;
    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    fn check(name: &str, expected: Ty) {
        let env = Env::new();
        let mut errs = vec![];
        let ty = resolve_type_expr(
            &TypeExpr::Named {
                name: name.into(),
                generics: vec![],
                bindings: vec![],
                span: s(),
            },
            &env,
            &mut errs,
        );
        assert_eq!(ty, expected);
        assert!(errs.is_empty());
    }

    #[test]
    fn resolves_int() {
        check("int", Ty::Int);
    }
    #[test]
    fn resolves_float() {
        check("float", Ty::Float);
    }
    #[test]
    fn resolves_str() {
        check("str", Ty::Str);
    }
    #[test]
    fn resolves_void() {
        check("void", Ty::Void);
    }

    #[test]
    fn resolves_registered_generic_type() {
        use crate::analyzer::env::Symbol;
        use crate::analyzer::ty::TypeKind;
        let mut env = Env::new();
        let mut reg = crate::analyzer::ty::TypeRegistry::new();
        // TypeId(0) is reserved as the generic-param sentinel; new() starts at 1.
        let id = reg.register("Box".into(), TypeKind::Struct);
        env.push_scope();
        env.define(
            "Box",
            Symbol::Type {
                id: id.clone(),
                span: s(),
            },
        );
        let mut errs = vec![];
        let ty = resolve_type_expr(
            &TypeExpr::Named {
                name: "Box".into(),
                generics: vec![TypeExpr::Named {
                    name: "int".into(),
                    generics: vec![],
                    bindings: vec![],
                    span: s(),
                }],
                bindings: vec![],
                span: s(),
            },
            &env,
            &mut errs,
        );
        assert_eq!(ty, Ty::Named(id, "Box".into(), vec![Ty::Int]));
        assert!(errs.is_empty());
    }

    #[test]
    fn unknown_type_emits_error() {
        let env = Env::new();
        let mut errs = vec![];
        let ty = resolve_type_expr(
            &TypeExpr::Named {
                name: "Ghost".into(),
                generics: vec![],
                bindings: vec![],
                span: s(),
            },
            &env,
            &mut errs,
        );
        assert_eq!(ty, Ty::Unknown);
        assert_eq!(errs.len(), 1);
    }
}
