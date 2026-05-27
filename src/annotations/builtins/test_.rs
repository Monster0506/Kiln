use crate::annotations::api::{AnnotationArgs, AnnotationTarget};
use crate::annotations::ProcessorRegistry;
use crate::parser::ast::Item;

pub fn register(registry: &mut ProcessorRegistry) {
    registry.register("test", process_test);
}

pub fn process_test(_target: AnnotationTarget, _args: AnnotationArgs) -> Vec<Item> {
    // @test is a marker annotation. The test harness (inject_harness) scans
    // for @test annotations on functions directly. No new items needed here.
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
    fn test_on_void_function_emits_nothing() {
        let f = FnDef {
            annotations: vec![AnnotationUse {
                name: "test".into(),
                args: vec![],
                span: s(),
            }],
            name: "addition_works".into(),
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
        let items = process_test(AnnotationTarget::Function(&f), &[]);
        // marker: no items emitted, harness collects directly
        assert!(items.is_empty());
    }

    #[test]
    fn test_on_non_void_function_emits_nothing() {
        let f = FnDef {
            annotations: vec![],
            name: "bad_test".into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: named("int"),
            body: Block {
                stmts: vec![],
                span: s(),
            },
            is_declaration: false,
            span: s(),
        };
        let items = process_test(AnnotationTarget::Function(&f), &[]);
        assert!(items.is_empty());
    }

    #[test]
    fn test_on_struct_emits_nothing() {
        let st = StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "Foo".into(),
            generic_params: vec![],
            interfaces: vec![],
            fields: vec![],
            methods: vec![],
            decls: vec![],
            inline_hooks: vec![],
            span: s(),
        };
        let items = process_test(AnnotationTarget::Struct(&st), &[]);
        assert!(items.is_empty());
    }
}
