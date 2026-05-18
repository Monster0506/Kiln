use crate::annotations::api::{AnnotationArgs, AnnotationTarget};
use crate::annotations::ProcessorRegistry;
use crate::parser::ast::Item;

pub fn register(registry: &mut ProcessorRegistry) {
    registry.register("indirect", process_indirect);
}

pub fn process_indirect(_target: AnnotationTarget, _args: AnnotationArgs) -> Vec<Item> {
    // @indirect is a field-level annotation read directly by the codegen memory
    // pass when laying out struct fields. No new items are generated.
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
    fn indirect_on_function_produces_no_items() {
        let f = FnDef {
            annotations: vec![],
            name: "foo".into(),
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
        let result = process_indirect(AnnotationTarget::Function(&f), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn indirect_on_struct_produces_no_items() {
        let st = StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "Node".into(),
            generic_params: vec![],
            interfaces: vec![],
            fields: vec![],
            methods: vec![],
            decls: vec![],
            span: s(),
        };
        let result = process_indirect(AnnotationTarget::Struct(&st), &[]);
        assert!(result.is_empty());
    }
}
