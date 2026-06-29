use crate::analyzer::env::{Env, Symbol};
use crate::analyzer::error::AnalysisError;
use crate::analyzer::ty::{InterfaceId, Ty, TypeKind, TypeRegistry};
use crate::parser::ast::{Item, SourceFile};

/// Pass 1: register all top-level names.
/// Returns a list of errors (duplicate names). Does not type-check bodies.
pub fn collect_top_level(
    source: &SourceFile,
    env: &mut Env,
    registry: &mut TypeRegistry,
) -> Vec<AnalysisError> {
    let mut errors = Vec::new();
    let mut next_iface_id: u32 = 0;
    env.push_scope(); // module-level scope

    for item in &source.items {
        let (name, span) = match item {
            Item::Struct(s) => (s.name.clone(), s.span),
            Item::Enum(e) => (e.name.clone(), e.span),
            Item::Interface(i) => (i.name.clone(), i.span),
            Item::TypeAlias(t) => (t.name.clone(), t.span),
            Item::Function(f) => (f.name.clone(), f.span),
            // ImplBlock, Import, Export, AnnotationDef, ProcessorDef have no top-level name to bind
            _ => continue,
        };

        if env.would_shadow(&name) {
            // Allow multiple definitions of the same function name at the same scope
            // level -- they become overloads, resolved in Pass 1b.
            let in_same_scope = matches!(
                env.lookup_in_current_scope(&name),
                Some(Symbol::Fn { .. }) | Some(Symbol::FnOverloadSet { .. })
            );
            let is_fn_item = matches!(item, Item::Function(_));
            if is_fn_item && in_same_scope {
                continue;
            }
            errors.push(AnalysisError::DuplicateName { name, span });
            continue;
        }

        let sym = match item {
            Item::Struct(s) => {
                let id = registry.register(s.name.clone(), TypeKind::Struct);
                Symbol::Type { id, span }
            }
            Item::Enum(e) => {
                let variants = e.variants.iter().map(|v| v.name.clone()).collect();
                let id = registry.register(
                    e.name.clone(),
                    TypeKind::Enum {
                        variant_names: variants,
                    },
                );
                Symbol::Type { id, span }
            }
            Item::Interface(iface) => {
                let id = InterfaceId(next_iface_id);
                next_iface_id += 1;
                let assoc_types: Vec<String> = iface
                    .items
                    .iter()
                    .filter_map(|it| {
                        if let crate::parser::ast::InterfaceItemKind::AssocType { name, .. } =
                            &it.kind
                        {
                            Some(name.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                Symbol::Iface {
                    id,
                    assoc_types,
                    span,
                }
            }
            Item::TypeAlias(t) => {
                let id = registry.register(t.name.clone(), TypeKind::Alias(Ty::Unknown));
                Symbol::Type { id, span }
            }
            Item::Function(f) => Symbol::Fn {
                generic_params: f.generic_params.iter().map(|g| g.name.clone()).collect(),
                generic_bounds: vec![],  // resolved in pass 1b
                inferred_bounds: vec![], // populated after body analysis
                params: vec![],          // resolved in pass 1b
                ret: Ty::Unknown,
                throws: f.throws,
                span,
            },
            _ => continue,
        };

        env.define(&name, sym);
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::env::Env;
    use crate::analyzer::ty::TypeRegistry;
    use crate::diagnostics::Span;
    use crate::parser::ast::*;
    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    fn make_struct(name: &str) -> Item {
        Item::Struct(StructDef {
            annotations: vec![],
            is_builtin: false,
            name: name.to_string(),
            generic_params: vec![],
            interfaces: vec![],
            fields: vec![],
            methods: vec![],
            decls: vec![],
            inline_hooks: vec![],
            span: s(),
        })
    }

    #[test]
    fn collects_struct_and_function() {
        let source = SourceFile {
            items: vec![
                make_struct("Point"),
                Item::Function(FnDef {
                    annotations: vec![],
                    name: "add".into(),
                    generic_params: vec![],
                    params: vec![],
                    variadic: None,
                    return_type: TypeExpr::Named {
                        name: "int".into(),
                        generics: vec![],
                        bindings: vec![],
                        span: s(),
                    },
                    throws: false,
                    body: Block {
                        stmts: vec![],
                        span: s(),
                    },
                    is_declaration: false,
                    span: s(),
                }),
            ],
            span: s(),
        };
        let mut env = Env::new();
        let mut reg = TypeRegistry::new();
        let errors = collect_top_level(&source, &mut env, &mut reg);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(env.lookup("Point").is_some());
        assert!(env.lookup("add").is_some());
    }

    #[test]
    fn duplicate_name_is_error() {
        let source = SourceFile {
            items: vec![make_struct("Foo"), make_struct("Foo")],
            span: s(),
        };
        let mut env = Env::new();
        let mut reg = TypeRegistry::new();
        let errors = collect_top_level(&source, &mut env, &mut reg);
        assert_eq!(errors.len(), 1);
    }
}
