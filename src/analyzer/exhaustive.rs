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
    /// Option[T] -- must cover Some(_) and None.
    Option,
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
        MatchDomain::Option => {
            let has_some = arms.iter().any(|a| {
                matches!(&a.pattern,
                Pattern::Struct { variant, .. } if variant == "Some")
            });
            let has_none = arms.iter().any(|a| {
                matches!(&a.pattern, Pattern::Struct { variant, .. } if variant == "None")
                    || matches!(&a.pattern, Pattern::TypeBinding { ty, .. } if ty == "None")
            });
            if !has_some || !has_none {
                errors.push(AnalysisError::NonExhaustiveMatch { span: *span });
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
}
