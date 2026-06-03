use crate::analyzer::env::Symbol;
use crate::analyzer::ty::Ty;
use crate::diagnostics::Span;
use std::collections::HashMap;

/// Ordered list of (name, type) pairs: function parameters, struct fields, assoc bindings.
pub type ParamList = Vec<(String, Ty)>;

/// Flat list of (name, symbol) pairs snapshotted from an Env scope.
pub type SymbolList = Vec<(String, Symbol)>;

/// Active projection pins: maps (generic_param, assoc_type_name) -> concrete Ty.
pub type ProjectionPins = HashMap<(String, String), Ty>;

/// Diagnostic notes attached to an analysis error: (message, optional source span).
pub type DiagNotes = Vec<(String, Option<Span>)>;
