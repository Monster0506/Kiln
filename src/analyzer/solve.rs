use crate::analyzer::constrain::Constraint;
use crate::analyzer::error::AnalysisError;
use crate::analyzer::infer::type_name_of;
use crate::analyzer::ty::{Ty, TypeRegistry};
use crate::diagnostics::Span;

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

        // Named types (including generic containers like Vec, Option, Map).
        Ty::Named(_, name, args) => {
            // Generic placeholder names have no TypeRegistry entry; treat as passing.
            if registry.lookup_by_name(name.as_str()).is_none() {
                return true;
            }
            let entries = registry.get_conformances(name.as_str(), iface);
            if !entries.is_empty() {
                let ok = if args.is_empty() {
                    true
                } else if args.len() == 1 {
                    // Single-arg container: all bounds checked against args[0].
                    let inner = &args[0];
                    entries.iter().any(|entry| {
                        entry
                            .bounds
                            .iter()
                            .all(|(_, bound_iface)| satisfies(inner, bound_iface, registry))
                    })
                } else {
                    // Multi-arg container: bounds matched positionally to args.
                    entries.iter().any(|entry| {
                        entry
                            .bounds
                            .iter()
                            .enumerate()
                            .all(|(i, (_, bound_iface))| {
                                args.get(i)
                                    .is_none_or(|a| satisfies(a, bound_iface, registry))
                            })
                    })
                };
                if ok {
                    return true;
                }
            }
            // Check shorthand operator variants (e.g. `AddableWith[X]` implies `Addable`).
            if let Some(with_iface) = operator_shorthand_to_with(iface) {
                if !registry
                    .get_conformances(name.as_str(), with_iface)
                    .is_empty()
                {
                    return true;
                }
            }
            false
        }

        // Primitives look up conformances by name.
        _ => {
            let Some(name) = type_name_of(ty) else {
                return false;
            };
            let entries = registry.get_conformances(&name, iface);
            if !entries.is_empty() {
                return true;
            }
            false
        }
    }
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
            let notes: Vec<(String, Option<Span>)> = match &c.reason {
                crate::analyzer::constrain::ConstraintReason::GenericBoundCheck {
                    fn_name,
                    is_explicit,
                    decl_span,
                    source_span,
                    source_desc,
                    ..
                } => {
                    let mut ns = Vec::new();
                    if !source_desc.is_empty() {
                        let verb = if *is_explicit {
                            "required by"
                        } else {
                            "inferred from"
                        };
                        ns.push((
                            format!("bound {verb} {source_desc} in `{fn_name}`"),
                            *source_span,
                        ));
                    }
                    if *is_explicit {
                        ns.push(("bound declared here".to_string(), *decl_span));
                    }
                    ns
                }
                _ => vec![],
            };
            errors.push(AnalysisError::BoundViolation {
                ty: c.ty.to_string(),
                iface: c.iface.clone(),
                context,
                span: c.span,
                notes,
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
        let ty = Ty::Named(crate::analyzer::ty::TypeId(99), "Vec".into(), vec![Ty::Int]);
        assert!(satisfies(&ty, "Display", &r));
    }

    #[test]
    fn vec_vec_int_satisfies_display() {
        let r = make_registry();
        let inner = Ty::Named(crate::analyzer::ty::TypeId(99), "Vec".into(), vec![Ty::Int]);
        let outer = Ty::Named(crate::analyzer::ty::TypeId(99), "Vec".into(), vec![inner]);
        assert!(satisfies(&outer, "Display", &r));
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
