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

    #[error("{span}: type `{ty}` does not implement `{iface}`{context}")]
    BoundViolation {
        ty: String,
        iface: String,
        context: String,
        span: Span,
        /// Span in the generic function body where this bound was inferred (for the note line).
        note_span: Option<Span>,
        /// Human-readable explanation for the note, e.g. "required because `sum` uses `+=` on `T`".
        note: String,
    },

    #[error("{span}: field `{field}` is private")]
    PrivateField { field: String, span: Span },

    #[error("{span}: no matching overload for `{name}`")]
    NoMatchingOverload { name: String, span: Span },

    #[error("{span}: duplicate plain impl of `{iface}` for `{ty}`")]
    DuplicateImpl {
        ty: String,
        iface: String,
        span: Span,
    },

    #[error("{span}: interface `{iface}` is not object-safe (method `{method}` uses `Self`)")]
    NonObjectSafeInterface {
        iface: String,
        method: String,
        span: Span,
    },

    #[error("{span}: type `{ty}` has no field `{field}`")]
    NoField {
        ty: String,
        field: String,
        span: Span,
    },

    #[error("{span}: unknown annotation `{name}`")]
    UnknownAnnotation { name: String, span: Span },

    #[error("{span}: `{field}` is a struct field; write `self.{field}` to access it")]
    BareFieldAccess { field: String, span: Span },

    #[error("{span}: duplicate definition of `{name}` with the same parameter signature")]
    DuplicateSignature { name: String, span: Span },
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
            AnalysisError::NoMatchingOverload { .. } => "E010",
            AnalysisError::DuplicateImpl { .. } => "E011",
            AnalysisError::NonObjectSafeInterface { .. } => "E012",
            AnalysisError::NoField { .. } => "E013",
            AnalysisError::UnknownAnnotation { .. } => "E014",
            AnalysisError::BareFieldAccess { .. } => "E015",
            AnalysisError::DuplicateSignature { .. } => "E016",
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
            AnalysisError::NoMatchingOverload { .. } => "type error",
            AnalysisError::DuplicateImpl { .. } => "impl error",
            AnalysisError::NonObjectSafeInterface { .. } => "object safety error",
            AnalysisError::NoField { .. } => "type error",
            AnalysisError::UnknownAnnotation { .. } => "annotation error",
            AnalysisError::BareFieldAccess { .. } => "name error",
            AnalysisError::DuplicateSignature { .. } => "name error",
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
            AnalysisError::BoundViolation {
                ty, iface, context, ..
            } => {
                format!("`{ty}` does not implement `{iface}`{context}")
            }
            AnalysisError::PrivateField { field, .. } => {
                format!("field `{field}` is private")
            }
            AnalysisError::NoMatchingOverload { name, .. } => {
                format!("no matching overload for `{name}`")
            }
            AnalysisError::DuplicateImpl { ty, iface, .. } => {
                format!("duplicate plain impl of `{iface}` for `{ty}`")
            }
            AnalysisError::NonObjectSafeInterface { iface, method, .. } => {
                format!("interface `{iface}` is not object-safe (method `{method}` uses `Self`)")
            }
            AnalysisError::NoField { ty, field, .. } => {
                format!("type `{ty}` has no field `{field}`")
            }
            AnalysisError::UnknownAnnotation { name, .. } => {
                format!("unknown annotation `{name}`")
            }
            AnalysisError::BareFieldAccess { field, .. } => {
                format!("`{field}` is a struct field; write `self.{field}` to access it")
            }
            AnalysisError::DuplicateSignature { name, .. } => {
                format!("duplicate definition of `{name}` with the same parameter signature")
            }
        }
    }

    /// Returns `(note_text, note_span)` for errors that carry a secondary location.
    pub fn note_info(&self) -> Option<(String, Option<Span>)> {
        match self {
            AnalysisError::BoundViolation {
                note, note_span, ..
            } if !note.is_empty() => Some((note.clone(), *note_span)),
            _ => None,
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
            AnalysisError::NoMatchingOverload { span, .. } => *span,
            AnalysisError::DuplicateImpl { span, .. } => *span,
            AnalysisError::NonObjectSafeInterface { span, .. } => *span,
            AnalysisError::NoField { span, .. } => *span,
            AnalysisError::UnknownAnnotation { span, .. } => *span,
            AnalysisError::BareFieldAccess { span, .. } => *span,
            AnalysisError::DuplicateSignature { span, .. } => *span,
        }
    }
}
