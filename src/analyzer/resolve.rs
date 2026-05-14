use crate::analyzer::env::{Env, Symbol};
use crate::analyzer::error::AnalysisError;
use crate::analyzer::ty::{Ty, TypeRegistry};
use crate::parser::ast::TypeExpr;

pub fn resolve_type_expr(
    expr: &TypeExpr,
    env: &Env,
    registry: &TypeRegistry,
    errors: &mut Vec<AnalysisError>,
) -> Ty {
    match expr {
        TypeExpr::Named {
            name,
            generics,
            span,
        } => {
            let resolved_generics: Vec<Ty> = generics
                .iter()
                .map(|g| resolve_type_expr(g, env, registry, errors))
                .collect();

            match name.as_str() {
                "int" => Ty::Int,
                "float" => Ty::Float,
                "bool" => Ty::Bool,
                "str" => Ty::Str,
                "void" => Ty::Void,
                "Option" => Ty::Option(Box::new(nth(&resolved_generics, 0))),
                "Vec" => Ty::Vec(Box::new(nth(&resolved_generics, 0))),
                "Set" => Ty::Set(Box::new(nth(&resolved_generics, 0))),
                "Map" => Ty::Map(
                    Box::new(nth(&resolved_generics, 0)),
                    Box::new(nth(&resolved_generics, 1)),
                ),
                "Shared" => Ty::Shared(Box::new(nth(&resolved_generics, 0))),
                other => match env.lookup(other) {
                    Some(Symbol::Type { id, .. }) => Ty::Named(id.clone(), other.to_string()),
                    Some(Symbol::Iface { id, .. }) => {
                        Ty::Interface(id.clone(), other.to_string())
                    }
                    _ => {
                        errors.push(AnalysisError::UndefinedName {
                            name: other.to_string(),
                            span: *span,
                        });
                        Ty::Unknown
                    }
                },
            }
        }
        TypeExpr::Tuple(elems, _) => Ty::Tuple(
            elems
                .iter()
                .map(|e| resolve_type_expr(e, env, registry, errors))
                .collect(),
        ),
        TypeExpr::Callable { params, ret, .. } => {
            let ptys = params
                .iter()
                .map(|p| resolve_type_expr(p, env, registry, errors))
                .collect();
            let rty = resolve_type_expr(ret, env, registry, errors);
            Ty::Callable(ptys, Box::new(rty))
        }
        TypeExpr::Union(variants, _) => Ty::Union(
            variants
                .iter()
                .map(|v| resolve_type_expr(v, env, registry, errors))
                .collect(),
        ),
        TypeExpr::Ref { inner, mutable, .. } => Ty::Ref(
            Box::new(resolve_type_expr(inner, env, registry, errors)),
            *mutable,
        ),
        TypeExpr::GenSplice(_, span) => {
            errors.push(AnalysisError::UndefinedName {
                name: "<gen-splice>".into(),
                span: *span,
            });
            Ty::Unknown
        }
    }
}

fn nth(tys: &[Ty], i: usize) -> Ty {
    tys.get(i).cloned().unwrap_or(Ty::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::env::Env;
    use crate::analyzer::ty::{Ty, TypeRegistry};
    use crate::diagnostics::Span;
    use crate::parser::ast::TypeExpr;
    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    fn check(name: &str, expected: Ty) {
        let env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        let ty = resolve_type_expr(
            &TypeExpr::Named {
                name: name.into(),
                generics: vec![],
                span: s(),
            },
            &env,
            &reg,
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
    fn resolves_option_int() {
        let env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        let ty = resolve_type_expr(
            &TypeExpr::Named {
                name: "Option".into(),
                generics: vec![TypeExpr::Named {
                    name: "int".into(),
                    generics: vec![],
                    span: s(),
                }],
                span: s(),
            },
            &env,
            &reg,
            &mut errs,
        );
        assert_eq!(ty, Ty::Option(Box::new(Ty::Int)));
    }

    #[test]
    fn unknown_type_emits_error() {
        let env = Env::new();
        let reg = TypeRegistry::new();
        let mut errs = vec![];
        let ty = resolve_type_expr(
            &TypeExpr::Named {
                name: "Ghost".into(),
                generics: vec![],
                span: s(),
            },
            &env,
            &reg,
            &mut errs,
        );
        assert_eq!(ty, Ty::Unknown);
        assert_eq!(errs.len(), 1);
    }
}
