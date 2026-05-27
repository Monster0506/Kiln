use crate::annotations::api::{AnnotationArgs, AnnotationTarget};
use crate::annotations::ProcessorRegistry;
use crate::parser::ast::Item;

pub fn register(registry: &mut ProcessorRegistry) {
    registry.register("inline", process_inline);
}

pub fn process_inline(_target: AnnotationTarget, _args: AnnotationArgs) -> Vec<Item> {
    // @inline instructs codegen to apply an always-inline attribute to the function.
    // The codegen pass reads this annotation directly from the FnDef node.
    // No new items are generated.
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
    fn inline_on_function_produces_no_items() {
        let f = FnDef {
            annotations: vec![],
            name: "fast_path".into(),
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
        let result = process_inline(AnnotationTarget::Function(&f), &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn inline_on_struct_produces_no_items() {
        let st = StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "Tiny".into(),
            generic_params: vec![],
            interfaces: vec![],
            fields: vec![],
            methods: vec![],
            decls: vec![],
            inline_hooks: vec![],
            span: s(),
        };
        let result = process_inline(AnnotationTarget::Struct(&st), &[]);
        assert!(result.is_empty());
    }
}
