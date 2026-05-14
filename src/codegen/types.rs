use crate::analyzer::ty::Ty;
use crate::parser::ast::TypeExpr;
use cranelift_codegen::ir::types::{self, Type};

/// Lower a Kiln `Ty` to a Cranelift scalar `Type`.
///
/// Returns `None` for `Ty::Void` (no value) and for composite types that
/// require out-of-band handling (str, tuples, structs) — callers that may
/// receive those must check first.
///
/// Heap-allocated composite types (Vec, Map, Set, Shared, Ref, Named) are
/// represented as opaque `I64` pointers at the IR level.
pub fn clif_type(ty: &Ty) -> Option<Type> {
    match ty {
        Ty::Int => Some(types::I64),
        Ty::Float => Some(types::F64),
        Ty::Bool => Some(types::I8),
        Ty::Void => None,

        // Composite scalar-ish: represented as a pointer word.
        Ty::Str
        | Ty::Shared(_)
        | Ty::Ref(_, _)
        | Ty::Vec(_)
        | Ty::Set(_)
        | Ty::Map(_, _)
        | Ty::Option(_)
        | Ty::Tuple(_)
        | Ty::Callable(_, _)
        | Ty::Named(_, _)
        | Ty::Interface(_, _)
        | Ty::Union(_)
        | Ty::GenericParam(_)
        | Ty::Unknown => Some(types::I64),
    }
}

pub fn is_void(ty: &Ty) -> bool {
    matches!(ty, Ty::Void)
}

/// Convert a `TypeExpr` directly to a Cranelift type without going through
/// the analyzer. Handles primitive names; everything else is an `I64` pointer.
pub fn type_expr_to_clif(ty: &TypeExpr) -> Option<Type> {
    match ty {
        TypeExpr::Named { name, .. } => match name.as_str() {
            "void" => None,
            "int" => Some(types::I64),
            "float" => Some(types::F64),
            "bool" => Some(types::I8),
            "str" => Some(types::I64),
            _ => Some(types::I64),
        },
        TypeExpr::Tuple(elems, _) if elems.is_empty() => None,
        TypeExpr::Tuple(_, _) => Some(types::I64),
        _ => Some(types::I64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::Ty;

    #[test]
    fn int_lowers_to_i64() {
        assert_eq!(clif_type(&Ty::Int), Some(types::I64));
    }

    #[test]
    fn float_lowers_to_f64() {
        assert_eq!(clif_type(&Ty::Float), Some(types::F64));
    }

    #[test]
    fn bool_lowers_to_i8() {
        assert_eq!(clif_type(&Ty::Bool), Some(types::I8));
    }

    #[test]
    fn void_returns_none() {
        assert_eq!(clif_type(&Ty::Void), None);
    }

    #[test]
    fn pointer_types_lower_to_i64() {
        assert_eq!(clif_type(&Ty::Shared(Box::new(Ty::Int))), Some(types::I64));
        assert_eq!(
            clif_type(&Ty::Ref(Box::new(Ty::Int), false)),
            Some(types::I64)
        );
        assert_eq!(clif_type(&Ty::Vec(Box::new(Ty::Int))), Some(types::I64));
    }
}
