use crate::analyzer::error::AnalysisError;
use crate::diagnostics::Span;
use crate::parser::ast::{Expr, MatchArm, Pattern};

/// Describes the set of values the match scrutinee can take.
#[derive(Debug)]
pub enum MatchDomain {
    /// int, float, str -- infinite; requires a wildcard.
    Primitive,
    Bool,
    Enum {
        variants: Vec<String>,
    },
    /// A | B | C union type.
    Union {
        types: Vec<String>,
    },
}

pub fn check_exhaustiveness(
    arms: &[MatchArm],
    domain: &MatchDomain,
    span: &Span,
    errors: &mut Vec<AnalysisError>,
) {
    if arms_have_wildcard(arms) {
        return;
    }

    match domain {
        MatchDomain::Primitive => {
            errors.push(AnalysisError::NonExhaustiveMatch { span: *span });
        }
        MatchDomain::Bool => {
            let has_true = arms.iter().any(|a| is_bool_literal(&a.pattern, true));
            let has_false = arms.iter().any(|a| is_bool_literal(&a.pattern, false));
            if !has_true || !has_false {
                errors.push(AnalysisError::NonExhaustiveMatch { span: *span });
            }
        }
        MatchDomain::Enum { variants } => {
            for variant in variants {
                let covered = arms.iter().any(|a| match &a.pattern {
                    Pattern::Struct { variant: v, .. } => v == variant,
                    Pattern::TypeBinding { ty, .. } => ty == variant,
                    _ => false,
                });
                if !covered {
                    errors.push(AnalysisError::NonExhaustiveMatch { span: *span });
                    return;
                }
            }
        }
        MatchDomain::Union { types } => {
            for ty_name in types {
                let covered = arms.iter().any(|a| {
                    matches!(&a.pattern,
                    Pattern::TypeBinding { ty, .. } if ty == ty_name)
                });
                if !covered {
                    errors.push(AnalysisError::NonExhaustiveMatch { span: *span });
                    return;
                }
            }
        }
    }
}

fn arms_have_wildcard(arms: &[MatchArm]) -> bool {
    arms.iter()
        .any(|a| matches!(a.pattern, Pattern::Wildcard(_)))
}

fn is_bool_literal(pat: &Pattern, val: bool) -> bool {
    matches!(pat, Pattern::Literal(Expr::Bool(b, _)) if *b == val)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Span;
    use crate::parser::ast::{Expr, MatchArm, Pattern};
    fn s() -> Span {
        Span { start: 0, end: 0 }
    }
    fn dummy_body() -> Expr {
        Expr::Bool(true, s())
    }

    #[test]
    fn wildcard_covers_all() {
        let arms = vec![MatchArm {
            pattern: Pattern::Wildcard(s()),
            guard: None,
            body: dummy_body(),
            span: s(),
        }];
        let mut errs = vec![];
        check_exhaustiveness(&arms, &MatchDomain::Primitive, &s(), &mut errs);
        assert!(errs.is_empty());
    }

    #[test]
    fn bool_needs_both_cases() {
        let arms = vec![
            MatchArm {
                pattern: Pattern::Literal(Expr::Bool(true, s())),
                guard: None,
                body: dummy_body(),
                span: s(),
            },
            MatchArm {
                pattern: Pattern::Literal(Expr::Bool(false, s())),
                guard: None,
                body: dummy_body(),
                span: s(),
            },
        ];
        let mut errs = vec![];
        check_exhaustiveness(&arms, &MatchDomain::Bool, &s(), &mut errs);
        assert!(errs.is_empty());
    }

    #[test]
    fn bool_missing_false_is_error() {
        let arms = vec![MatchArm {
            pattern: Pattern::Literal(Expr::Bool(true, s())),
            guard: None,
            body: dummy_body(),
            span: s(),
        }];
        let mut errs = vec![];
        check_exhaustiveness(&arms, &MatchDomain::Bool, &s(), &mut errs);
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn enum_all_variants_covered() {
        let arms = vec![
            MatchArm {
                pattern: Pattern::Struct {
                    variant: "North".into(),
                    fields: vec![],
                    span: s(),
                },
                guard: None,
                body: dummy_body(),
                span: s(),
            },
            MatchArm {
                pattern: Pattern::Struct {
                    variant: "South".into(),
                    fields: vec![],
                    span: s(),
                },
                guard: None,
                body: dummy_body(),
                span: s(),
            },
        ];
        let domain = MatchDomain::Enum {
            variants: vec!["North".into(), "South".into()],
        };
        let mut errs = vec![];
        check_exhaustiveness(&arms, &domain, &s(), &mut errs);
        assert!(errs.is_empty());
    }

    #[test]
    fn enum_missing_variant_is_error() {
        let arms = vec![MatchArm {
            pattern: Pattern::Struct {
                variant: "North".into(),
                fields: vec![],
                span: s(),
            },
            guard: None,
            body: dummy_body(),
            span: s(),
        }];
        let domain = MatchDomain::Enum {
            variants: vec!["North".into(), "South".into()],
        };
        let mut errs = vec![];
        check_exhaustiveness(&arms, &domain, &s(), &mut errs);
        assert_eq!(errs.len(), 1);
    }

    #[test]
    fn option_some_struct_and_none_struct_exhaustive() {
        let arms = vec![
            MatchArm {
                pattern: Pattern::Struct {
                    variant: "Some".into(),
                    fields: vec![],
                    span: s(),
                },
                guard: None,
                body: dummy_body(),
                span: s(),
            },
            MatchArm {
                pattern: Pattern::Struct {
                    variant: "None".into(),
                    fields: vec![],
                    span: s(),
                },
                guard: None,
                body: dummy_body(),
                span: s(),
            },
        ];
        let domain = MatchDomain::Enum {
            variants: vec!["Some".into(), "None".into()],
        };
        let mut errs = vec![];
        check_exhaustiveness(&arms, &domain, &s(), &mut errs);
        assert!(
            errs.is_empty(),
            "Some + None (both Struct) must be exhaustive"
        );
    }

    #[test]
    fn option_some_struct_and_bare_none_typebinding_exhaustive() {
        // Bare `None` in a match arm is parsed as TypeBinding { ty: "None" }.
        // The Enum path must accept it as covering the None variant.
        let arms = vec![
            MatchArm {
                pattern: Pattern::Struct {
                    variant: "Some".into(),
                    fields: vec![],
                    span: s(),
                },
                guard: None,
                body: dummy_body(),
                span: s(),
            },
            MatchArm {
                pattern: Pattern::TypeBinding {
                    ty: "None".into(),
                    name: "_".into(),
                    span: s(),
                },
                guard: None,
                body: dummy_body(),
                span: s(),
            },
        ];
        let domain = MatchDomain::Enum {
            variants: vec!["Some".into(), "None".into()],
        };
        let mut errs = vec![];
        check_exhaustiveness(&arms, &domain, &s(), &mut errs);
        assert!(
            errs.is_empty(),
            "Some (Struct) + None (TypeBinding) must be exhaustive: {errs:?}"
        );
    }

    #[test]
    fn option_missing_none_via_enum_is_error() {
        let arms = vec![MatchArm {
            pattern: Pattern::Struct {
                variant: "Some".into(),
                fields: vec![],
                span: s(),
            },
            guard: None,
            body: dummy_body(),
            span: s(),
        }];
        let domain = MatchDomain::Enum {
            variants: vec!["Some".into(), "None".into()],
        };
        let mut errs = vec![];
        check_exhaustiveness(&arms, &domain, &s(), &mut errs);
        assert_eq!(
            errs.len(),
            1,
            "Some-only Option match must be non-exhaustive"
        );
    }

    #[test]
    fn option_missing_some_via_enum_is_error() {
        let arms = vec![MatchArm {
            pattern: Pattern::TypeBinding {
                ty: "None".into(),
                name: "_".into(),
                span: s(),
            },
            guard: None,
            body: dummy_body(),
            span: s(),
        }];
        let domain = MatchDomain::Enum {
            variants: vec!["Some".into(), "None".into()],
        };
        let mut errs = vec![];
        check_exhaustiveness(&arms, &domain, &s(), &mut errs);
        assert_eq!(
            errs.len(),
            1,
            "None-only Option match must be non-exhaustive"
        );
    }
}
