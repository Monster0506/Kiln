use crate::parser::ast::{EnumDef, FnDef, StructDef};

/// A read-only view of the AST node an annotation is applied to.
#[derive(Debug, Clone)]
pub enum AnnotationTarget<'a> {
    Function(&'a FnDef),
    Struct(&'a StructDef),
    Enum(&'a EnumDef),
}

/// The args from `@Foo { field: val }` or `@Foo(Name, Other)`.
pub type AnnotationArgs<'a> = &'a [(String, crate::parser::ast::Expr)];
