use crate::analyzer::constrain::{Constraint, ConstraintKind};
use crate::analyzer::error::AnalysisError;
use crate::analyzer::infer::type_name_of;
use crate::analyzer::ty::{Ty, TypeRegistry};
use crate::analyzer::types::DiagNotes;

/// Returns `true` if `ty` satisfies `iface`. `Ty::Unknown` always passes to avoid
/// double-reporting. Generic container checks recurse over element/key/value bounds.
pub fn satisfies(ty: &Ty, iface: &str, registry: &TypeRegistry) -> bool {
    match ty {
        // These always pass: errors elsewhere already cover them.
        // Ty::Projection is abstract at definition sites; concrete checks happen at call sites.
        Ty::Unknown | Ty::GenericParam(_) | Ty::Interface(_, _) | Ty::Projection { .. } => true,

        // A compound type satisfies an interface when any of its constituents satisfies it
        // (or implies it through the superinterface hierarchy).
        Ty::Compound(parts) => parts.iter().any(|p| satisfies(p, iface, registry)),

        // Structural types with no registered conformance.
        Ty::Void | Ty::Tuple(_) | Ty::Callable(_, _) | Ty::Union(_) => false,

        // References are transparent: &T satisfies whatever T satisfies.
        Ty::Ref(inner, _) => satisfies(inner, iface, registry),

        // Named types (including generic containers like Vec, Option, Map).
        Ty::Named(_, name, args) => {
            // Generic placeholder names have no TypeRegistry entry; treat as passing.
            if registry.lookup_by_name(name.as_str()).is_none() {
                return true;
            }
            let entries = registry.get_conformances(name.as_str(), iface);
            if !entries.is_empty() {
                // Resolve bounds by named param via generic_param_order, so multi-bound
                // params (T: Eq + Hash) and multi-arg types both work correctly.
                let param_order = registry.get_generic_param_order(name.as_str());
                let ok = entries.iter().any(|entry| {
                    if entry.bounds.is_empty() || args.is_empty() {
                        return true;
                    }
                    entry.bounds.iter().all(|(param_name, bound_iface)| {
                        let idx = param_order
                            .and_then(|order| order.iter().position(|p| p == param_name));
                        match idx {
                            Some(i) => args
                                .get(i)
                                .is_none_or(|a| satisfies(a, bound_iface, registry)),
                            None => {
                                // No param order registered: fall back to first arg (single-param containers).
                                args.first()
                                    .is_none_or(|a| satisfies(a, bound_iface, registry))
                            }
                        }
                    })
                });
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
            // Fall back to superinterface implication.
            satisfies_by_implication(name.as_str(), iface, registry)
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
            satisfies_by_implication(&name, iface, registry)
        }
    }
}

/// Returns true if any directly registered conformance for `type_name` implies
/// `iface` through the superinterface hierarchy.
fn satisfies_by_implication(type_name: &str, iface: &str, registry: &TypeRegistry) -> bool {
    registry
        .conformance_ifaces_for(type_name)
        .iter()
        .any(|known| registry.iface_implies(known, iface))
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

// Solver

/// Solve all constraints, returning a `BoundViolation` or `ProjectedBoundViolation` for each failure.
/// Never short-circuits -- all constraints are checked.
pub fn solve(constraints: &[Constraint], registry: &TypeRegistry) -> Vec<AnalysisError> {
    let mut errors = Vec::new();
    for c in constraints {
        match &c.kind {
            ConstraintKind::Bound { ty, iface } => {
                if !satisfies(ty, iface, registry) {
                    let context = c.reason.context_string();
                    let mut notes: DiagNotes = match &c.reason {
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
                    if let Some(suggestion) = weaker_bound_suggestion(ty, iface, registry) {
                        notes.push((suggestion, None));
                    }
                    errors.push(AnalysisError::BoundViolation {
                        ty: ty.to_string(),
                        iface: iface.clone(),
                        context,
                        span: c.span,
                        notes,
                    });
                }
            }
            ConstraintKind::ProjectedBound {
                base_ty,
                base_iface,
                assoc_name,
                required_iface,
            } => {
                // Resolve the associated type for base_ty through base_iface, then check
                // that the resolved type satisfies required_iface.
                let base_name = type_name_of(base_ty);
                let resolved = base_name
                    .and_then(|name| registry.get_assoc_binding(&name, base_iface, assoc_name));
                let passes = match &resolved {
                    Some(assoc_ty) => satisfies(assoc_ty, required_iface, registry),
                    // If we can't resolve the assoc type, pass silently -- a missing impl
                    // would already be caught by the base Bound constraint.
                    None => true,
                };
                if !passes {
                    let assoc_ty = resolved.unwrap();
                    errors.push(AnalysisError::ProjectedBoundViolation {
                        base_ty: base_ty.to_string(),
                        base_iface: base_iface.clone(),
                        assoc_name: assoc_name.clone(),
                        assoc_ty: assoc_ty.to_string(),
                        required_iface: required_iface.clone(),
                        span: c.span,
                    });
                }
            }
        }
    }
    errors
}

/// When `ty: iface` fails, suggest the most specific satisfied superinterface of `iface`
/// (the one not implied by any other satisfied superinterface).
fn weaker_bound_suggestion(ty: &Ty, iface: &str, registry: &TypeRegistry) -> Option<String> {
    let supers = registry.get_transitive_supers(iface)?;
    let satisfied: Vec<&str> = supers
        .iter()
        .filter(|sup| satisfies(ty, sup.as_str(), registry))
        .map(|s| s.as_str())
        .collect();
    if satisfied.is_empty() {
        return None;
    }
    // Keep only the most specific: drop any that are implied by another satisfied member.
    let best = satisfied.iter().find(|s| {
        !satisfied
            .iter()
            .any(|other| *other != **s && registry.iface_implies(other, s))
    });
    best.map(|s| {
        format!(
            "`{ty}` does implement `{s}` -- consider relaxing the bound from `{iface}` to `{s}`"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::{ConformanceEntry, TypeRegistry};

    fn make_registry() -> TypeRegistry {
        let mut r = TypeRegistry::new();
        // int: Addable, Display
        r.register_conformance(
            "int",
            "Addable",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        r.register_conformance(
            "int",
            "Display",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        r.register_conformance(
            "int",
            "Eq",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        r.register_conformance(
            "int",
            "Ord",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        // float: Display (not Eq/Ord due to NaN)
        r.register_conformance(
            "float",
            "Display",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        r.register_conformance(
            "float",
            "Addable",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        // str: Display, Addable
        r.register_conformance(
            "str",
            "Display",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        r.register_conformance(
            "str",
            "Addable",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        // bool: Display
        r.register_conformance(
            "bool",
            "Display",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        // Vec[T]: Display if T: Display
        r.register_conformance(
            "Vec",
            "Display",
            ConformanceEntry {
                bounds: vec![("T".into(), "Display".into())],
                bindings: vec![],
            },
        );
        r.register_conformance(
            "Vec",
            "Eq",
            ConformanceEntry {
                bounds: vec![("T".into(), "Eq".into())],
                bindings: vec![],
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

    // Superinterface implication: int has Ord, and Ord extends PartialOrd,
    // so int should satisfy PartialOrd without a direct conformance entry.
    #[test]
    fn int_satisfies_partial_ord_via_ord() {
        let mut r = TypeRegistry::new();
        r.register_conformance(
            "int",
            "Ord",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        // Ord directly extends PartialOrd.
        r.register_interface_supers("Ord", vec!["PartialOrd".to_string()]);
        // No direct (int, PartialOrd) entry -- must derive via implication.
        assert!(
            !r.get_conformances("int", "PartialOrd").is_empty() || {
                // The real test: satisfies() uses the implication path.
                satisfies(&Ty::Int, "PartialOrd", &r)
            }
        );
    }

    // Transitive implication: Ord -> PartialOrd -> Eq (two hops).
    #[test]
    fn int_satisfies_eq_transitively_via_ord() {
        let mut r = TypeRegistry::new();
        r.register_conformance(
            "int",
            "Ord",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        r.register_interface_supers("Ord", vec!["PartialOrd".to_string()]);
        r.register_interface_supers("PartialOrd", vec!["Eq".to_string()]);
        assert!(satisfies(&Ty::Int, "Eq", &r));
    }

    // No spurious implication: int has Ord but not Display via implication.
    #[test]
    fn int_does_not_satisfy_display_via_ord() {
        let mut r = TypeRegistry::new();
        r.register_conformance(
            "int",
            "Ord",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        r.register_interface_supers("Ord", vec!["PartialOrd".to_string()]);
        assert!(!satisfies(&Ty::Int, "Display", &r));
    }

    // Hierarchy-aware suggestion: float doesn't satisfy Ord but does satisfy PartialOrd.
    #[test]
    fn weaker_bound_suggested_when_type_satisfies_superinterface() {
        let mut r = TypeRegistry::new();
        r.register_conformance(
            "float",
            "PartialOrd",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        r.register_interface_supers("Ord", vec!["PartialOrd".to_string()]);
        r.precompute_transitive_closures();
        let suggestion = weaker_bound_suggestion(&Ty::Float, "Ord", &r);
        assert!(
            suggestion.is_some(),
            "should suggest PartialOrd as a weaker alternative"
        );
        let msg = suggestion.unwrap();
        assert!(
            msg.contains("PartialOrd"),
            "suggestion should mention PartialOrd: {msg}"
        );
    }

    // No suggestion when type satisfies nothing in the hierarchy.
    #[test]
    fn no_suggestion_when_type_satisfies_nothing() {
        let mut r = TypeRegistry::new();
        r.register_interface_supers("Ord", vec!["PartialOrd".to_string()]);
        r.precompute_transitive_closures();
        let suggestion = weaker_bound_suggestion(&Ty::Float, "Ord", &r);
        assert!(suggestion.is_none());
    }

    // Most-specific suggestion: two-level hierarchy; pick the closer one.
    #[test]
    fn most_specific_superinterface_is_suggested() {
        let mut r = TypeRegistry::new();
        // float satisfies both PartialOrd and Eq, but not Ord.
        r.register_conformance(
            "float",
            "PartialOrd",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        r.register_conformance(
            "float",
            "Eq",
            ConformanceEntry {
                bounds: vec![],
                bindings: vec![],
            },
        );
        r.register_interface_supers("Ord", vec!["PartialOrd".to_string()]);
        r.register_interface_supers("PartialOrd", vec!["Eq".to_string()]);
        r.precompute_transitive_closures();
        let suggestion = weaker_bound_suggestion(&Ty::Float, "Ord", &r);
        let msg = suggestion.expect("should have a suggestion");
        // PartialOrd is more specific than Eq (PartialOrd implies Eq), so PartialOrd wins.
        assert!(
            msg.contains("PartialOrd"),
            "should suggest PartialOrd not Eq: {msg}"
        );
    }
}
