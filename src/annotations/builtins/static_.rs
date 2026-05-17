use crate::annotations::api::{AnnotationArgs, AnnotationTarget};
use crate::annotations::ProcessorRegistry;
use crate::parser::ast::Item;

pub fn register(registry: &mut ProcessorRegistry) {
    registry.register("static", process_static);
}

pub fn process_static(target: AnnotationTarget, _args: AnnotationArgs) -> Vec<Item> {
    // @static is a field-level annotation on methods inside struct bodies.
    // The codegen and analyzer read it directly from the FnDef annotations.
    // No new items are generated.
    let _ = target;
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

    #[test]
    fn static_on_function_produces_no_new_items() {
        let fn_def = FnDef {
            annotations: vec![],
            name: "new".into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: TypeExpr::Named {
                name: "void".into(),
                generics: vec![],
                bindings: vec![],
                span: s(),
            },
            body: Block {
                stmts: vec![],
                span: s(),
            },
            span: s(),
        };
        let result = process_static(AnnotationTarget::Function(&fn_def), &[]);
        assert!(result.is_empty());
    }
}
