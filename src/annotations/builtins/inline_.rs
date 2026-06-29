use crate::analyzer::typed_ast::{TypedFile, TypedItem};
use crate::annotations::api::{AnnotationArgs, AnnotationTarget};
use crate::annotations::ProcessorRegistry;

pub fn register(registry: &mut ProcessorRegistry) {
    registry.register("inline", process_inline);
}

pub fn process_inline(
    _file: &TypedFile,
    _target: AnnotationTarget,
    _args: AnnotationArgs,
) -> Vec<TypedItem> {
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::Ty;
    use crate::analyzer::typed_ast::{TypedFile, TypedFnDef};
    use crate::annotations::typed_builders::{s, tblock};
    use crate::diagnostics::Span;

    fn empty_file() -> TypedFile {
        TypedFile {
            items: vec![],
            span: Span::new(0, 0),
        }
    }

    #[test]
    fn inline_on_function_produces_no_items() {
        let f = TypedFnDef {
            name: "fast_path".into(),
            params: vec![],
            variadic: None,
            return_type: Ty::Int,
            body: tblock(vec![]),
            is_builtin: false,
            is_inline: false,
            is_declaration: false,
            is_entry: false,
            is_impure: false,
            throws: false,
            span: s(),
        };
        assert!(process_inline(&empty_file(), AnnotationTarget::Function(&f), &[]).is_empty());
    }
}
