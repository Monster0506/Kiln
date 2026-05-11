use crate::diagnostics::Span;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum AnalysisError {
    #[error("{span}: undefined name `{name}`")]
    UndefinedName { name: String, span: Span },

    #[error("{span}: type mismatch -- expected `{expected}`, found `{found}`")]
    TypeMismatch {
        expected: String,
        found: String,
        span: Span,
    },

    #[error("{span}: cannot assign to immutable binding `{name}`")]
    AssignToImmutable { name: String, span: Span },

    #[error("{span}: duplicate top-level name `{name}`")]
    DuplicateName { name: String, span: Span },

    #[error("{span}: function `{name}` does not return a value on all paths")]
    MissingReturn { name: String, span: Span },

    #[error("{span}: match is not exhaustive")]
    NonExhaustiveMatch { span: Span },

    #[error("{span}: type `{ty}` does not satisfy `{iface}`: {detail}")]
    MissingConformance {
        ty: String,
        iface: String,
        detail: String,
        span: Span,
    },

    #[error("{span}: generic bound violated -- `{ty}` does not satisfy `{bound}`")]
    BoundViolation {
        ty: String,
        bound: String,
        span: Span,
    },

    #[error("{span}: field `{field}` is private")]
    PrivateField { field: String, span: Span },
}
