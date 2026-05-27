use crate::annotations::api::{AnnotationArgs, AnnotationTarget};
use crate::annotations::ProcessorRegistry;
use crate::parser::ast::Item;

pub fn register(registry: &mut ProcessorRegistry) {
    registry.register("deprecated", process_deprecated);
}

pub fn process_deprecated(_target: AnnotationTarget, _args: AnnotationArgs) -> Vec<Item> {
    // @deprecated is a marker annotation. The analyzer emits a warning at every
    // call site that references a @deprecated symbol. No new items are generated.
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::api::AnnotationTarget;
    use crate::diagnostics::Span;
    use crate::parser::ast::*;

    fn s() -> Span {
        Span::new(0, 0)
    }
    fn named(n: &str) -> TypeExpr {
        TypeExpr::Named {
            name: n.into(),
            generics: vec![],
            bindings: vec![],
            span: s(),
        }
    }

    #[test]
    fn deprecated_produces_no_new_items() {
        let f = FnDef {
            annotations: vec![],
            name: "old_api".into(),
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
        };
        let result = process_deprecated(AnnotationTarget::Function(&f), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn deprecated_on_struct_produces_no_items() {
        let st = StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "OldStruct".into(),
            generic_params: vec![],
            interfaces: vec![],
            fields: vec![],
            methods: vec![],
            decls: vec![],
            inline_hooks: vec![],
            span: s(),
        };
        let result = process_deprecated(AnnotationTarget::Struct(&st), &[]);
        assert!(result.is_empty());
    }
}
