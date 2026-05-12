use crate::diagnostics::Span;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum AnalysisError {
    #[error("{span}: undefined name `{name}`")]
    UndefinedName { name: String, span: Span },

    #[error("{span}: type mismatch: expected `{expected}`, found `{found}`")]
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

    #[error("{span}: generic bound violated: `{ty}` does not satisfy `{bound}`")]
    BoundViolation {
        ty: String,
        bound: String,
        span: Span,
    },

    #[error("{span}: field `{field}` is private")]
    PrivateField { field: String, span: Span },
}

impl AnalysisError {
    pub fn code(&self) -> &'static str {
        match self {
            AnalysisError::UndefinedName { .. } => "E001",
            AnalysisError::TypeMismatch { .. } => "E002",
            AnalysisError::AssignToImmutable { .. } => "E003",
            AnalysisError::DuplicateName { .. } => "E004",
            AnalysisError::MissingReturn { .. } => "E005",
            AnalysisError::NonExhaustiveMatch { .. } => "E006",
            AnalysisError::MissingConformance { .. } => "E007",
            AnalysisError::BoundViolation { .. } => "E008",
            AnalysisError::PrivateField { .. } => "E009",
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            AnalysisError::UndefinedName { .. } => "name error",
            AnalysisError::TypeMismatch { .. } => "type error",
            AnalysisError::AssignToImmutable { .. } => "mutability error",
            AnalysisError::DuplicateName { .. } => "name error",
            AnalysisError::MissingReturn { .. } => "control flow error",
            AnalysisError::NonExhaustiveMatch { .. } => "exhaustiveness error",
            AnalysisError::MissingConformance { .. } => "conformance error",
            AnalysisError::BoundViolation { .. } => "type error",
            AnalysisError::PrivateField { .. } => "visibility error",
        }
    }

    pub fn message(&self) -> String {
        match self {
            AnalysisError::UndefinedName { name, .. } => {
                format!("undefined name `{name}`")
            }
            AnalysisError::TypeMismatch {
                expected, found, ..
            } => {
                format!("type mismatch: expected `{expected}`, found `{found}`")
            }
            AnalysisError::AssignToImmutable { name, .. } => {
                format!("cannot assign to immutable binding `{name}`")
            }
            AnalysisError::DuplicateName { name, .. } => {
                format!("duplicate top-level name `{name}`")
            }
            AnalysisError::MissingReturn { name, .. } => {
                format!("function `{name}` does not return a value on all paths")
            }
            AnalysisError::NonExhaustiveMatch { .. } => "match is not exhaustive".into(),
            AnalysisError::MissingConformance {
                ty, iface, detail, ..
            } => {
                format!("type `{ty}` does not satisfy `{iface}`: {detail}")
            }
            AnalysisError::BoundViolation { ty, bound, .. } => {
                format!("generic bound violated: `{ty}` does not satisfy `{bound}`")
            }
            AnalysisError::PrivateField { field, .. } => {
                format!("field `{field}` is private")
            }
        }
    }

    pub fn span(&self) -> Span {
        match self {
            AnalysisError::UndefinedName { span, .. } => *span,
            AnalysisError::TypeMismatch { span, .. } => *span,
            AnalysisError::AssignToImmutable { span, .. } => *span,
            AnalysisError::DuplicateName { span, .. } => *span,
            AnalysisError::MissingReturn { span, .. } => *span,
            AnalysisError::NonExhaustiveMatch { span } => *span,
            AnalysisError::MissingConformance { span, .. } => *span,
            AnalysisError::BoundViolation { span, .. } => *span,
            AnalysisError::PrivateField { span, .. } => *span,
        }
    }
}
