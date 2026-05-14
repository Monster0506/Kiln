use crate::analyzer::constrain::Constraint;
use crate::analyzer::error::AnalysisError;
use crate::analyzer::ty::{Ty, TypeRegistry};

/// Returns `true` if `ty` satisfies `iface` according to the registry.
///
/// `Ty::Unknown` always passes to avoid double-reporting after earlier errors.
/// For generic containers (Vec, Set, Map, Option), the check recurses over
/// the element/key/value type against the entry's bounds.
pub fn satisfies(ty: &Ty, iface: &str, registry: &TypeRegistry) -> bool {
    match ty {
        Ty::Unknown => true,

        Ty::Int => !registry.get_conformances("int", iface).is_empty(),
        Ty::Float => !registry.get_conformances("float", iface).is_empty(),
        Ty::Bool => !registry.get_conformances("bool", iface).is_empty(),
        Ty::Str => !registry.get_conformances("str", iface).is_empty(),
        Ty::Void => false,

        Ty::Vec(inner) => satisfies_generic_one("Vec", inner, iface, registry),
        Ty::Set(inner) => satisfies_generic_one("Set", inner, iface, registry),
        Ty::Option(inner) => satisfies_generic_one("Option", inner, iface, registry),
        Ty::Shared(inner) => satisfies_generic_one("Shared", inner, iface, registry),

        Ty::Map(key, val) => satisfies_map(key, val, iface, registry),

        Ty::Named(_, name) => {
            // If the name has no entry in the TypeRegistry it is a generic type
            // parameter placeholder (e.g. `T` in `def f[T: Eq](...)`). Those are
            // already checked at call-site via explicit bounds, so we pass them here.
            if registry.lookup_by_name(name).is_none() {
                return true;
            }
            let entries = registry.get_conformances(name, iface);
            if entries.is_empty() {
                return false;
            }
            // For a concrete named type, bounds in entries are always empty.
            true
        }

        Ty::GenericParam(_) => {
            // Generic params are checked at call sites via explicit bounds;
            // if one appears here it means the constraint was already emitted
            // with a concrete type. Treat as unknown.
            true
        }

        // Interface types satisfy all constraints — runtime dispatch handles
        // actual conformance, so we cannot reject statically.
        Ty::Interface(_, _) => true,

        // Compound / structural types with no registered conformance.
        Ty::Tuple(_) | Ty::Callable(_, _) | Ty::Ref(_, _) | Ty::Union(_) => false,
    }
}

/// Check conformance for a single-parameter generic container (Vec, Set, Option, Shared).
fn satisfies_generic_one(
    type_name: &str,
    inner: &Ty,
    iface: &str,
    registry: &TypeRegistry,
) -> bool {
    let entries = registry.get_conformances(type_name, iface);
    if entries.is_empty() {
        return false;
    }
    // Any one entry with all bounds satisfied is enough.
    entries.iter().any(|entry| {
        entry.bounds.iter().all(|(_param, bound_iface)| satisfies(inner, bound_iface, registry))
    })
}

/// Check conformance for Map[K, V].
fn satisfies_map(key: &Ty, val: &Ty, iface: &str, registry: &TypeRegistry) -> bool {
    let entries = registry.get_conformances("Map", iface);
    if entries.is_empty() {
        return false;
    }
    // Bounds are ordered: first bound applies to K, second to V (by convention).
    entries.iter().any(|entry| {
        let bounds = &entry.bounds;
        match bounds.as_slice() {
            [] => true,
            [(_, k_iface)] => satisfies(key, k_iface, registry),
            [(_, k_iface), (_, v_iface)] => {
                satisfies(key, k_iface, registry) && satisfies(val, v_iface, registry)
            }
            _ => false,
        }
    })
}

// ---------------------------------------------------------------------------
// Solver
// ---------------------------------------------------------------------------

/// Solve all constraints, returning a `BoundViolation` for each failure.
/// Never short-circuits — all constraints are checked.
pub fn solve(constraints: &[Constraint], registry: &TypeRegistry) -> Vec<AnalysisError> {
    let mut errors = Vec::new();
    for c in constraints {
        if !satisfies(&c.ty, &c.iface, registry) {
            let context = c.reason.context_string();
            errors.push(AnalysisError::BoundViolation {
                ty: c.ty.to_string(),
                iface: c.iface.clone(),
                context,
                span: c.span,
            });
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::{ConformanceEntry, TypeRegistry};

    fn make_registry() -> TypeRegistry {
        let mut r = TypeRegistry::new();
        // int: Addable, Display
        r.register_conformance("int", "Addable", ConformanceEntry { bounds: vec![] });
        r.register_conformance("int", "Display", ConformanceEntry { bounds: vec![] });
        r.register_conformance("int", "Eq", ConformanceEntry { bounds: vec![] });
        r.register_conformance("int", "Ord", ConformanceEntry { bounds: vec![] });
        // float: Display (not Eq/Ord due to NaN)
        r.register_conformance("float", "Display", ConformanceEntry { bounds: vec![] });
        r.register_conformance("float", "Addable", ConformanceEntry { bounds: vec![] });
        // str: Display, Addable
        r.register_conformance("str", "Display", ConformanceEntry { bounds: vec![] });
        r.register_conformance("str", "Addable", ConformanceEntry { bounds: vec![] });
        // bool: Display
        r.register_conformance("bool", "Display", ConformanceEntry { bounds: vec![] });
        // Vec[T]: Display if T: Display
        r.register_conformance(
            "Vec",
            "Display",
            ConformanceEntry { bounds: vec![("T".into(), "Display".into())] },
        );
        r.register_conformance(
            "Vec",
            "Eq",
            ConformanceEntry { bounds: vec![("T".into(), "Eq".into())] },
        );
        r
    }

    #[test]
    fn int_satisfies_addable() {
        let r = make_registry();
        assert!(satisfies(&Ty::Int, "Addable", &r));
    }

    #[test]
    fn int_does_not_satisfy_unknown_iface() {
        let r = make_registry();
        assert!(!satisfies(&Ty::Int, "Iterable", &r));
    }

    #[test]
    fn vec_int_satisfies_display() {
        let r = make_registry();
        assert!(satisfies(&Ty::Vec(Box::new(Ty::Int)), "Display", &r));
    }

    #[test]
    fn vec_vec_int_satisfies_display() {
        let r = make_registry();
        let inner = Ty::Vec(Box::new(Ty::Int));
        assert!(satisfies(&Ty::Vec(Box::new(inner)), "Display", &r));
    }

    #[test]
    fn float_does_not_satisfy_eq() {
        let r = make_registry();
        assert!(!satisfies(&Ty::Float, "Eq", &r));
    }

    #[test]
    fn unknown_always_satisfies() {
        let r = make_registry();
        assert!(satisfies(&Ty::Unknown, "Eq", &r));
        assert!(satisfies(&Ty::Unknown, "SomeFakeIface", &r));
    }

    #[test]
    fn solve_empty_returns_no_errors() {
        let r = make_registry();
        assert!(solve(&[], &r).is_empty());
    }
}
