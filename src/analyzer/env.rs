use crate::analyzer::ty::{InterfaceId, Ty, TypeId};
use crate::analyzer::types::{ParamList, ProjectionPins, SymbolList};
use crate::diagnostics::Span;
use std::collections::HashMap;

/// A single interface bound on a generic parameter, e.g. `T: Addable`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GenericBound {
    pub param: String,
    pub iface: String,
    /// Associated type bindings from this bound, e.g. `T: Iterator[Item=int]` stores
    /// `[("Item", Ty::Int)]`. Interface RHS (e.g. `Item=Display`) stores `Ty::Interface`.
    pub assoc_bindings: ParamList,
    /// `true` when the bound was written explicitly in the function signature (`[T: Addable]`).
    /// `false` when inferred from body usage.
    pub is_explicit: bool,
    /// Span of the generic parameter in the function signature where this bound was declared.
    /// Set for explicit bounds; `None` for inferred bounds.
    pub decl_span: Option<crate::diagnostics::Span>,
    /// Byte span of the expression in the function body that requires this bound.
    /// For explicit bounds this is populated after merging with inferred data.
    pub source_span: Option<crate::diagnostics::Span>,
    /// Human-readable description of the usage site, e.g. "use of `+=` on `T`".
    /// Empty when no body-usage site was found.
    pub source_desc: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FnOverload {
    pub generic_params: Vec<String>,
    pub generic_bounds: Vec<GenericBound>,
    /// Bounds inferred from how generic params are used in the function body.
    pub inferred_bounds: Vec<GenericBound>,
    pub params: ParamList,
    pub ret: Ty,
    pub throws: bool,
    /// The symbol name used in codegen, e.g. "foo__0" for the first overload of "foo".
    pub mangled_name: String,
    pub span: Span,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Symbol {
    Var {
        ty: Ty,
        mutable: bool,
        span: Span,
    },
    Fn {
        generic_params: Vec<String>,
        generic_bounds: Vec<GenericBound>,
        /// Bounds inferred from how generic params are used in the function body.
        inferred_bounds: Vec<GenericBound>,
        params: ParamList,
        ret: Ty,
        throws: bool,
        span: Span,
    },
    /// Multiple definitions of the same function name (overloads).
    FnOverloadSet {
        overloads: Vec<FnOverload>,
    },
    Type {
        id: TypeId,
        span: Span,
    },
    /// A transparent alias to a fully-resolved type (used for `Self` and user self-aliases).
    TypeAlias(Ty),
    /// A struct field name injected into method/hook scope. Resolving a bare name
    /// against this variant is an error: the user must write `self.field` explicitly.
    StructField {
        ty: Ty,
    },
    Iface {
        id: InterfaceId,
        /// Names of associated types declared in this interface (e.g. `type Item`).
        assoc_types: Vec<String>,
        span: Span,
    },
    /// A compile-time constant. Uses of the name are inlined as the literal value.
    Const {
        ty: Ty,
        value: crate::analyzer::typed_ast::TypedExprKind,
        span: Span,
    },
}

#[derive(Debug, Default, Clone)]
pub struct Env {
    scopes: Vec<HashMap<String, Symbol>>,
    /// Projection pin table, scoped in parallel with symbol scopes.
    /// Each layer maps (param_name, assoc_name) -> concrete Ty.
    projection_pins: Vec<ProjectionPins>,
    /// Generic param -> [iface names] for each scope level.
    /// Lets `infer_call_field` look up which interfaces a generic param is bound by.
    generic_param_bounds: Vec<HashMap<String, Vec<String>>>,
    /// Whether the current function being analyzed is declared `throws`.
    pub throws_context: bool,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.projection_pins.push(HashMap::new());
        self.generic_param_bounds.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
        self.projection_pins.pop();
        self.generic_param_bounds.pop();
    }

    /// Register a projection pin: `param.assoc = ty` in the innermost scope.
    pub fn pin_projection(&mut self, param: &str, assoc: &str, ty: Ty) {
        if let Some(layer) = self.projection_pins.last_mut() {
            layer.insert((param.to_string(), assoc.to_string()), ty);
        }
    }

    /// Look up a projection pin from innermost scope outward.
    pub fn resolve_projection(&self, param: &str, assoc: &str) -> Option<&Ty> {
        for layer in self.projection_pins.iter().rev() {
            if let Some(ty) = layer.get(&(param.to_string(), assoc.to_string())) {
                return Some(ty);
            }
        }
        None
    }

    /// Flatten all active pin layers (outer-to-inner so inner overrides outer).
    pub fn get_active_pins(&self) -> ProjectionPins {
        let mut result = ProjectionPins::new();
        for layer in self.projection_pins.iter() {
            result.extend(layer.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        result
    }

    /// Register that generic param `param` is bounded by `iface` in the current scope.
    pub fn register_param_iface(&mut self, param: &str, iface: &str) {
        if let Some(layer) = self.generic_param_bounds.last_mut() {
            layer
                .entry(param.to_string())
                .or_default()
                .push(iface.to_string());
        }
    }

    /// Returns all interface names that `param` is bounded by (innermost scope wins, all layers merged).
    pub fn get_param_ifaces(&self, param: &str) -> Vec<String> {
        let mut result = Vec::new();
        for layer in self.generic_param_bounds.iter() {
            if let Some(ifaces) = layer.get(param) {
                for iface in ifaces {
                    if !result.contains(iface) {
                        result.push(iface.clone());
                    }
                }
            }
        }
        result
    }

    /// Define a symbol in the innermost scope.
    pub fn define(&mut self, name: &str, sym: Symbol) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), sym);
        }
    }

    /// Search from innermost scope outward.
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(sym) = scope.get(name) {
                return Some(sym);
            }
        }
        None
    }

    /// Returns true when `name` is already visible in any enclosing scope.
    /// Used only for duplicate top-level detection; VarDecl shadowing is allowed.
    pub fn would_shadow(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    /// Look up `name` only in the innermost (current) scope.
    pub fn lookup_in_current_scope(&self, name: &str) -> Option<&Symbol> {
        self.scopes.last()?.get(name)
    }

    /// Collect all visible names for did-you-mean suggestions.
    /// Excludes `StructField` symbols (require `self.`) and names starting with `_`.
    pub fn all_names(&self) -> Vec<&str> {
        let mut seen = std::collections::HashSet::new();
        let mut names = Vec::new();
        for scope in self.scopes.iter().rev() {
            for (name, sym) in scope {
                if seen.contains(name.as_str()) {
                    continue;
                }
                seen.insert(name.as_str());
                if name.starts_with('_') {
                    continue;
                }
                if matches!(sym, Symbol::StructField { .. }) {
                    continue;
                }
                names.push(name.as_str());
            }
        }
        names
    }

    /// Returns all (name, is_mutable, decl_span) for Symbol::Var entries in the innermost
    /// scope whose names do not start with `_`. Used to emit unused-variable warnings.
    pub fn current_scope_vars(&self) -> Vec<(String, bool, Span)> {
        match self.scopes.last() {
            Some(scope) => scope
                .iter()
                .filter_map(|(name, sym)| match sym {
                    Symbol::Var { mutable, span, .. } if !name.starts_with('_') => {
                        Some((name.clone(), *mutable, *span))
                    }
                    _ => None,
                })
                .collect(),
            None => vec![],
        }
    }

    /// Return the declaration span of a visible symbol, if it has one.
    pub fn span_of(&self, name: &str) -> Option<Span> {
        match self.lookup(name)? {
            Symbol::Var { span, .. } => Some(*span),
            Symbol::Fn { span, .. } => Some(*span),
            Symbol::FnOverloadSet { overloads } => overloads.first().map(|o| o.span),
            Symbol::Type { span, .. } => Some(*span),
            Symbol::Iface { span, .. } => Some(*span),
            Symbol::Const { span, .. } => Some(*span),
            Symbol::TypeAlias(_) | Symbol::StructField { .. } => None,
        }
    }

    /// Returns all (name, symbol) pairs visible across all active scopes,
    /// innermost taking precedence. Used to snapshot prelude symbols for the cache.
    pub fn get_global_scope_symbols(&self) -> SymbolList {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for scope in self.scopes.iter().rev() {
            for (k, v) in scope {
                if seen.insert(k.clone()) {
                    result.push((k.clone(), v.clone()));
                }
            }
        }
        result
    }

    /// Mutable lookup -- searches from innermost scope outward.
    pub fn lookup_mut(&mut self, name: &str) -> Option<&mut Symbol> {
        for scope in self.scopes.iter_mut().rev() {
            if scope.contains_key(name) {
                return scope.get_mut(name);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::Ty;
    use crate::diagnostics::Span;
    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    #[test]
    fn lookup_finds_current_scope() {
        let mut env = Env::new();
        env.push_scope();
        env.define(
            "x",
            Symbol::Var {
                ty: Ty::Int,
                mutable: false,
                span: s(),
            },
        );
        assert!(matches!(
            env.lookup("x"),
            Some(Symbol::Var { ty: Ty::Int, .. })
        ));
        env.pop_scope();
    }

    #[test]
    fn lookup_finds_outer_scope() {
        let mut env = Env::new();
        env.push_scope();
        env.define(
            "x",
            Symbol::Var {
                ty: Ty::Int,
                mutable: false,
                span: s(),
            },
        );
        env.push_scope();
        assert!(env.lookup("x").is_some());
        env.pop_scope();
        env.pop_scope();
    }

    #[test]
    fn would_shadow_detects_outer_binding() {
        let mut env = Env::new();
        env.push_scope();
        env.define(
            "x",
            Symbol::Var {
                ty: Ty::Int,
                mutable: false,
                span: s(),
            },
        );
        env.push_scope();
        assert!(env.would_shadow("x"));
        assert!(!env.would_shadow("y"));
        env.pop_scope();
        env.pop_scope();
    }
}
