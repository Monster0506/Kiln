use crate::analyzer::typed_ast::{TypedEnumDef, TypedFnDef, TypedStructDef};
use crate::parser::ast::{EnumDef, FnDef, StructDef};

/// The typed AST node that an annotation is applied to (post-analysis).
#[derive(Debug, Clone)]
pub enum AnnotationTarget<'a> {
    Function(&'a TypedFnDef),
    Struct(&'a TypedStructDef),
    Enum(&'a TypedEnumDef),
}

/// The untyped AST node that an annotation is applied to (pre-analysis).
#[derive(Debug, Clone)]
pub enum SourceAnnotationTarget<'a> {
    Function(&'a FnDef),
    Struct(&'a StructDef),
    Enum(&'a EnumDef),
}

pub type AnnotationArgs<'a> = &'a [(String, crate::parser::ast::Expr)];
