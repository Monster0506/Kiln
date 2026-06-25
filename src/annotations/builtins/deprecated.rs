use crate::analyzer::typed_ast::{TypedFile, TypedItem};
use crate::annotations::api::{AnnotationArgs, AnnotationTarget};
use crate::annotations::ProcessorRegistry;

pub fn register(registry: &mut ProcessorRegistry) {
    registry.register("deprecated", process_deprecated);
}

pub fn process_deprecated(
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
    fn deprecated_produces_no_new_items() {
        let f = TypedFnDef {
            name: "old_api".into(),
            params: vec![],
            variadic: None,
            return_type: Ty::Void,
            body: tblock(vec![]),
            is_builtin: false,
            is_inline: false,
            is_declaration: false,
            is_entry: false,
            is_impure: false,
            span: s(),
        };
        assert!(process_deprecated(&empty_file(), AnnotationTarget::Function(&f), &[]).is_empty());
    }
}
