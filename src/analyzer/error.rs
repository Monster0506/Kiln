use crate::diagnostics::Span;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum AnalysisError {
    #[error("{span}: undefined name `{name}`")]
    UndefinedName {
        name: String,
        span: Span,
        /// Suggestion: (suggested_name, declaration_span).
        did_you_mean: Option<(String, Option<Span>)>,
    },

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
        /// Notes to display after the error, in order. Each is (text, optional source span).
        notes: Vec<(String, Option<Span>)>,
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

    #[error("{span}: implementation of `{name}` must omit bounds; bounds are declared canonically on the declaration")]
    BoundsOnImplementation { name: String, span: Span },

    #[error("{span}: `{name}` is declared but never implemented")]
    MissingImplementation { name: String, span: Span },

    #[error("{span}: module `{path}` not found")]
    ModuleNotFound { path: String, span: Span },

    #[error("{span}: symbol `{symbol}` is not exported by module `{module}`")]
    SymbolNotExported {
        symbol: String,
        module: String,
        span: Span,
    },

    #[error("{span}: division by zero detected at compile time")]
    DivisionByZero { span: Span },

    #[error("{span}: {message}")]
    RedundantMatchArm { message: String, span: Span },

    #[error("{span}: condition is always true")]
    TautologicalCondition { span: Span },

    #[error("{span}: condition is always false")]
    ContradictoryCondition { span: Span },

    #[error("{span}: const initializer for '{name}' must be a literal value")]
    NonLiteralConst { name: String, span: Span },

    #[error("{span}: cannot assign to const '{name}'")]
    AssignToConst { name: String, span: Span },

    #[error("{span}: processor error: {msg}")]
    ProcessorFail { msg: String, span: Span },

    #[error("{span}: processor warning: {msg}")]
    ProcessorWarn { msg: String, span: Span },

    #[error("{span}: interface `{iface}` has unbound associated types: {unbound}")]
    UnpinnedAssocTypes {
        iface: String,
        /// Comma-joined list of unbound associated type names.
        unbound: String,
        span: Span,
    },

    #[error("{span}: type argument `{found}` is not compatible with `{expected}` (invariant position in `{container}`)")]
    VarianceViolation {
        container: String,
        expected: String,
        found: String,
        span: Span,
    },

    #[error("{span}: `{base_ty}: {base_iface}` -- associated type `{assoc_name}` is `{assoc_ty}`, which does not implement `{required_iface}`")]
    ProjectedBoundViolation {
        base_ty: String,
        base_iface: String,
        assoc_name: String,
        assoc_ty: String,
        required_iface: String,
        span: Span,
    },

    #[error("{span}: type `{ty}` does not implement `Iterable` and cannot be used in a for loop")]
    NotIterable { ty: String, span: Span },

    #[error("{span}: cyclic interface hierarchy: `{iface}` transitively extends itself ({cycle})")]
    CyclicInterface {
        iface: String,
        cycle: String,
        span: Span,
    },

    #[error("{span}: recursive type `{ty}` has infinite size -- field `{field}` refers back to the enclosing type; add `@indirect` to break the cycle")]
    RecursiveTypeWithoutIndirect {
        ty: String,
        field: String,
        span: Span,
    },

    #[error("{span}: hook in struct body requires `@implements[InterfaceName]` annotation")]
    MissingImplementsAnnotation { span: Span },

    #[error("{span}: variable `{name}` is declared but never used")]
    UnusedVariable { name: String, span: Span },

    #[error("{span}: variable `{name}` does not need to be mutable")]
    NeedlessMut { name: String, span: Span },
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
            AnalysisError::BoundsOnImplementation { .. } => "E017",
            AnalysisError::MissingImplementation { .. } => "E018",
            AnalysisError::ModuleNotFound { .. } => "E019",
            AnalysisError::SymbolNotExported { .. } => "E020",
            AnalysisError::DivisionByZero { .. } => "E021",
            AnalysisError::NonLiteralConst { .. } => "E022",
            AnalysisError::AssignToConst { .. } => "E023",
            AnalysisError::RedundantMatchArm { .. } => "W001",
            AnalysisError::TautologicalCondition { .. } => "W002",
            AnalysisError::ContradictoryCondition { .. } => "W003",
            AnalysisError::ProcessorFail { .. } => "E024",
            AnalysisError::ProcessorWarn { .. } => "W004",
            AnalysisError::UnpinnedAssocTypes { .. } => "E025",
            AnalysisError::VarianceViolation { .. } => "E026",
            AnalysisError::ProjectedBoundViolation { .. } => "E027",
            AnalysisError::NotIterable { .. } => "E028",
            AnalysisError::CyclicInterface { .. } => "E029",
            AnalysisError::RecursiveTypeWithoutIndirect { .. } => "E030",
            AnalysisError::MissingImplementsAnnotation { .. } => "E031",
            AnalysisError::UnusedVariable { .. } => "W005",
            AnalysisError::NeedlessMut { .. } => "W006",
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
            AnalysisError::BoundsOnImplementation { .. } => "declaration error",
            AnalysisError::MissingImplementation { .. } => "declaration error",
            AnalysisError::ModuleNotFound { .. } => "module error",
            AnalysisError::SymbolNotExported { .. } => "module error",
            AnalysisError::DivisionByZero { .. } => "arithmetic error",
            AnalysisError::NonLiteralConst { .. } => "const error",
            AnalysisError::AssignToConst { .. } => "mutability error",
            AnalysisError::RedundantMatchArm { .. } => "warning",
            AnalysisError::TautologicalCondition { .. } => "warning",
            AnalysisError::ContradictoryCondition { .. } => "warning",
            AnalysisError::ProcessorFail { .. } => "processor error",
            AnalysisError::ProcessorWarn { .. } => "warning",
            AnalysisError::UnpinnedAssocTypes { .. } => "type error",
            AnalysisError::VarianceViolation { .. } => "type error",
            AnalysisError::ProjectedBoundViolation { .. } => "type error",
            AnalysisError::NotIterable { .. } => "type error",
            AnalysisError::CyclicInterface { .. } => "interface error",
            AnalysisError::RecursiveTypeWithoutIndirect { .. } => "type error",
            AnalysisError::MissingImplementsAnnotation { .. } => "annotation error",
            AnalysisError::UnusedVariable { .. } => "warning",
            AnalysisError::NeedlessMut { .. } => "warning",
        }
    }

    pub fn message(&self) -> String {
        match self {
            AnalysisError::UndefinedName {
                name, did_you_mean, ..
            } => match did_you_mean {
                Some((s, _)) => format!("undefined name `{name}` -- did you mean `{s}`?"),
                None => format!("undefined name `{name}`"),
            },
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
            AnalysisError::BoundsOnImplementation { name, .. } => {
                format!("implementation of `{name}` must omit bounds; bounds are declared canonically on the declaration")
            }
            AnalysisError::MissingImplementation { name, .. } => {
                format!("`{name}` is declared but never implemented")
            }
            AnalysisError::ModuleNotFound { path, .. } => {
                format!("module `{path}` not found")
            }
            AnalysisError::SymbolNotExported { symbol, module, .. } => {
                format!("symbol `{symbol}` is not exported by module `{module}`")
            }
            AnalysisError::DivisionByZero { .. } => {
                "division by zero detected at compile time".into()
            }
            AnalysisError::NonLiteralConst { name, .. } => {
                format!("const initializer for '{name}' must be a literal value")
            }
            AnalysisError::AssignToConst { name, .. } => {
                format!("cannot assign to const '{name}'")
            }
            AnalysisError::RedundantMatchArm { message, .. } => message.clone(),
            AnalysisError::TautologicalCondition { .. } => "condition is always true".into(),
            AnalysisError::ContradictoryCondition { .. } => "condition is always false".into(),
            AnalysisError::ProcessorFail { msg, .. } => msg.clone(),
            AnalysisError::ProcessorWarn { msg, .. } => msg.clone(),
            AnalysisError::UnpinnedAssocTypes { iface, unbound, .. } => {
                format!("interface `{iface}` has unbound associated types: {unbound}")
            }
            AnalysisError::VarianceViolation {
                container,
                expected,
                found,
                ..
            } => {
                format!(
                    "type argument `{found}` is not compatible with `{expected}` (invariant position in `{container}`)"
                )
            }
            AnalysisError::ProjectedBoundViolation {
                base_ty,
                base_iface,
                assoc_name,
                assoc_ty,
                required_iface,
                ..
            } => {
                format!(
                    "`{base_ty}: {base_iface}` -- associated type `{assoc_name}` is `{assoc_ty}`, which does not implement `{required_iface}`"
                )
            }
            AnalysisError::NotIterable { ty, .. } => {
                format!(
                    "type `{ty}` does not implement `Iterable` and cannot be used in a for loop"
                )
            }
            AnalysisError::CyclicInterface { iface, cycle, .. } => {
                format!(
                    "cyclic interface hierarchy: `{iface}` transitively extends itself ({cycle})"
                )
            }
            AnalysisError::RecursiveTypeWithoutIndirect { ty, field, .. } => {
                format!(
                    "recursive type `{ty}` has infinite size -- field `{field}` refers back to the enclosing type; add `@indirect` to break the cycle"
                )
            }
            AnalysisError::MissingImplementsAnnotation { .. } => {
                "hook in struct body requires `@implements[InterfaceName]` annotation".into()
            }
            AnalysisError::UnusedVariable { name, .. } => {
                format!("variable `{name}` is declared but never used")
            }
            AnalysisError::NeedlessMut { name, .. } => {
                format!("variable `{name}` does not need to be mutable")
            }
        }
    }

    /// Returns all `(note_text, note_span)` pairs for this error, in display order.
    pub fn note_info(&self) -> Vec<(String, Option<Span>)> {
        match self {
            AnalysisError::UndefinedName { did_you_mean, .. } => {
                if let Some((name, Some(decl_span))) = did_you_mean {
                    vec![(format!("`{name}` declared here"), Some(*decl_span))]
                } else {
                    vec![]
                }
            }
            AnalysisError::BoundViolation { notes, .. } => notes.clone(),
            AnalysisError::ProcessorFail { .. } | AnalysisError::ProcessorWarn { .. } => vec![],
            _ => vec![],
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
            AnalysisError::BoundsOnImplementation { span, .. } => *span,
            AnalysisError::MissingImplementation { span, .. } => *span,
            AnalysisError::ModuleNotFound { span, .. } => *span,
            AnalysisError::SymbolNotExported { span, .. } => *span,
            AnalysisError::DivisionByZero { span } => *span,
            AnalysisError::NonLiteralConst { span, .. } => *span,
            AnalysisError::AssignToConst { span, .. } => *span,
            AnalysisError::RedundantMatchArm { span, .. } => *span,
            AnalysisError::TautologicalCondition { span } => *span,
            AnalysisError::ContradictoryCondition { span } => *span,
            AnalysisError::ProcessorFail { span, .. } => *span,
            AnalysisError::ProcessorWarn { span, .. } => *span,
            AnalysisError::UnpinnedAssocTypes { span, .. } => *span,
            AnalysisError::VarianceViolation { span, .. } => *span,
            AnalysisError::ProjectedBoundViolation { span, .. } => *span,
            AnalysisError::NotIterable { span, .. } => *span,
            AnalysisError::CyclicInterface { span, .. } => *span,
            AnalysisError::RecursiveTypeWithoutIndirect { span, .. } => *span,
            AnalysisError::MissingImplementsAnnotation { span } => *span,
            AnalysisError::UnusedVariable { span, .. } => *span,
            AnalysisError::NeedlessMut { span, .. } => *span,
        }
    }
}
