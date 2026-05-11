use crate::analyzer::ty::{InterfaceId, Ty, TypeId};
use crate::diagnostics::Span;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Symbol {
    Var {
        ty: Ty,
        mutable: bool,
        span: Span,
    },
    Const {
        ty: Ty,
        span: Span,
    },
    Fn {
        generic_params: Vec<String>,
        params: Vec<(String, Ty)>,
        ret: Ty,
        span: Span,
    },
    Type {
        id: TypeId,
        span: Span,
    },
    Iface {
        id: InterfaceId,
        span: Span,
    },
}

#[derive(Debug, Default)]
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
