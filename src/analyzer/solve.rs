use crate::analyzer::constrain::Constraint;
use crate::analyzer::error::AnalysisError;
use crate::analyzer::infer::type_name_of;
use crate::analyzer::ty::{Ty, TypeRegistry};

/// Returns `true` if `ty` satisfies `iface` according to the registry.
///
/// `Ty::Unknown` always passes to avoid double-reporting after earlier errors.
/// For generic containers (Vec, Set, Map, Option), the check recurses over
/// the element/key/value type against the entry's bounds.
pub fn satisfies(ty: &Ty, iface: &str, registry: &TypeRegistry) -> bool {
    match ty {
        // These always pass: errors elsewhere already cover them.
        Ty::Unknown | Ty::GenericParam(_) | Ty::Interface(_, _) => true,

        // Structural types with no registered conformance.
        Ty::Void | Ty::Tuple(_) | Ty::Callable(_, _) | Ty::Ref(_, _) | Ty::Union(_) => false,

        // Container types recurse over their element/key types.
        Ty::Vec(inner) => satisfies_generic_one("Vec", inner, iface, registry),
        Ty::Set(inner) => satisfies_generic_one("Set", inner, iface, registry),
        Ty::Option(inner) => satisfies_generic_one("Option", inner, iface, registry),
        Ty::Shared(inner) => satisfies_generic_one("Shared", inner, iface, registry),
        Ty::Map(key, val) => satisfies_map(key, val, iface, registry),

        // All remaining concrete types (primitives + user-defined Named) look up
        // conformances by name. Named types additionally check operator shorthand
        // variants. Adding a new primitive only requires updating type_name_of.
        _ => {
            let Some(name) = type_name_of(ty) else {
                return false;
            };
            if let Ty::Named(_, _) = ty {
                // Generic placeholder names have no TypeRegistry entry; treat as passing.
                if registry.lookup_by_name(&name).is_none() {
                    return true;
                }
            }
            let entries = registry.get_conformances(&name, iface);
            if !entries.is_empty() {
                return true;
            }
            // For Named types, also check shorthand operator variants
            // (e.g. `AddableWith[X]` implies `Addable`).
            if let Ty::Named(_, _) = ty {
                if let Some(with_iface) = operator_shorthand_to_with(iface) {
                    if !registry.get_conformances(&name, with_iface).is_empty() {
                        return true;
                    }
                }
            }
            false
        }
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
        entry
            .bounds
            .iter()
            .all(|(_param, bound_iface)| satisfies(inner, bound_iface, registry))
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

/// Maps a shorthand operator interface to its heterogeneous `*With` variant.
/// User-defined types implementing `AddableWith[X]` satisfy the `Addable` bound.
fn operator_shorthand_to_with(iface: &str) -> Option<&'static str> {
    match iface {
        "Addable" => Some("AddableWith"),
        "Subtractable" => Some("SubtractableWith"),
        "Multiplicable" => Some("MultipliableWith"),
        "Divisible" => Some("DividableWith"),
        "Remainder" => Some("RemainderableWith"),
        "Negatable" => None,
        "PartialEq" => Some("EquatableWith"),
        "PartialOrd" | "Ord" => Some("ComparableWith"),
        _ => None,
    }
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
            let (note, note_span) = match &c.reason {
                crate::analyzer::constrain::ConstraintReason::GenericBoundCheck {
                    fn_name,
                    is_explicit,
                    source_span,
                    source_desc,
                    ..
                } if !source_desc.is_empty() => {
                    let verb = if *is_explicit { "required by" } else { "inferred from" };
                    (
                        format!("bound {verb} {source_desc} in `{fn_name}`"),
                        *source_span,
                    )
                }
                _ => (String::new(), None),
            };
            errors.push(AnalysisError::BoundViolation {
                ty: c.ty.to_string(),
                iface: c.iface.clone(),
                context,
                span: c.span,
                note,
                note_span,
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
            ConformanceEntry {
                bounds: vec![("T".into(), "Display".into())],
            },
        );
        r.register_conformance(
            "Vec",
            "Eq",
            ConformanceEntry {
                bounds: vec![("T".into(), "Eq".into())],
            },
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
