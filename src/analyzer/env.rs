use crate::analyzer::ty::{InterfaceId, Ty, TypeId};
use crate::diagnostics::Span;
use std::collections::HashMap;

/// A single interface bound on a generic parameter, e.g. `T: Addable`.
#[derive(Debug, Clone)]
pub struct GenericBound {
    pub param: String,
    pub iface: String,
    /// `true` when the bound was written explicitly in the function signature (`[T: Addable]`).
    /// `false` when inferred from body usage.
    pub is_explicit: bool,
    /// Byte span of the expression in the function body that requires this bound.
    /// For explicit bounds this is populated after merging with inferred data.
    pub source_span: Option<crate::diagnostics::Span>,
    /// Human-readable description of the usage site, e.g. "use of `+=` on `T`".
    /// Empty when no body-usage site was found.
    pub source_desc: String,
}

#[derive(Debug, Clone)]
pub struct FnOverload {
    pub generic_params: Vec<String>,
    pub generic_bounds: Vec<GenericBound>,
    /// Bounds inferred from how generic params are used in the function body.
    pub inferred_bounds: Vec<GenericBound>,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    /// The symbol name used in codegen, e.g. "foo__0" for the first overload of "foo".
    pub mangled_name: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
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
        params: Vec<(String, Ty)>,
        ret: Ty,
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
        span: Span,
    },
}

#[derive(Debug, Default, Clone)]
pub struct Env {
    scopes: Vec<HashMap<String, Symbol>>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.scopes.pop();
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
    /// Used to detect duplicate top-level names; NOT an error for VarDecl
    /// (shadowing is allowed in Kiln).
    pub fn would_shadow(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    /// Look up `name` only in the innermost (current) scope.
    pub fn lookup_in_current_scope(&self, name: &str) -> Option<&Symbol> {
        self.scopes.last()?.get(name)
    }

    /// Mutable lookup — searches from innermost scope outward.
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
