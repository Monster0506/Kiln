use std::collections::HashMap;
use std::fmt;

/// One way a type can satisfy an interface.
/// All bounds must hold for this entry to count.
/// An empty `bounds` vec means unconditional conformance.
#[derive(Debug, Clone)]
pub struct ConformanceEntry {
    /// Each element is `(param_name, interface_name)`.
    /// For generic containers: e.g. `Vec[T]: Display` stores `[("T", "Display")]`.
    /// For concrete types: empty.
    pub bounds: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InterfaceId(pub u32);

/// A method registered for a named type via an impl block.
#[derive(Debug, Clone)]
pub struct MethodEntry {
    pub method_name: String,
    /// Fully-qualified function name as registered in codegen's func_ids (e.g. "Vec_new").
    pub qualified_fn: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Int,
    Float,
    Bool,
    Str,
    Void,
    Tuple(Vec<Ty>),
    Callable(Vec<Ty>, Box<Ty>),
    Ref(Box<Ty>, bool),
    Union(Vec<Ty>),
    /// A named (possibly generic) type. `args` holds concrete type arguments.
    /// Plain named types have an empty `args` vec.
    Named(TypeId, String, Vec<Ty>),
    /// An interface name used as a type. Dispatches via type-tag vtable at runtime.
    Interface(InterfaceId, String),
    GenericParam(String),
    Unknown,
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Int => write!(f, "int"),
            Ty::Float => write!(f, "float"),
            Ty::Bool => write!(f, "bool"),
            Ty::Str => write!(f, "str"),
            Ty::Void => write!(f, "void"),
            Ty::Ref(t, true) => write!(f, "&mut {t}"),
            Ty::Ref(t, false) => write!(f, "&{t}"),
            Ty::Named(_, name, args) if args.is_empty() => write!(f, "{name}"),
            Ty::Named(_, name, args) => {
                let s = args
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{name}[{s}]")
            }
            Ty::Interface(_, name) => write!(f, "{name}"),
            Ty::GenericParam(n) => write!(f, "{n}"),
            Ty::Unknown => write!(f, "<unknown>"),
            Ty::Tuple(ts) => {
                let s = ts
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "({s})")
            }
            Ty::Callable(ps, r) => {
                let s = ps
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "Callable[({s}), {r}]")
            }
            Ty::Union(ts) => {
                let s = ts
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(" | ");
                write!(f, "{s}")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum TypeKind {
    Struct,
    Enum { variant_names: Vec<String> },
    Alias(Ty),
}

#[derive(Debug, Clone)]
pub struct TypeEntry {
    pub id: TypeId,
    pub name: String,
    pub kind: TypeKind,
}

#[derive(Debug, Default)]
pub struct TypeRegistry {
    next_id: u32,
    entries: Vec<TypeEntry>,
    /// type_name -> ordered list of (field_name, Ty)
    struct_fields: HashMap<String, Vec<(String, Ty)>>,
    /// type_name -> registered methods
    type_methods: HashMap<String, Vec<MethodEntry>>,
    /// (type_name, iface_name) -> list of conformance entries (any one suffices)
    conformances: HashMap<(String, String), Vec<ConformanceEntry>>,
    /// iface_name -> method signatures declared in that interface
    interface_methods: HashMap<String, Vec<MethodEntry>>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self {
            next_id: 1, // TypeId(0) is reserved as the generic-param sentinel
            ..Self::default()
        }
    }

    pub fn register_struct_fields(&mut self, type_name: &str, fields: Vec<(String, Ty)>) {
        self.struct_fields.insert(type_name.to_string(), fields);
    }

    pub fn get_struct_fields(&self, type_name: &str) -> Option<&[(String, Ty)]> {
        self.struct_fields.get(type_name).map(|v| v.as_slice())
    }

    pub fn register_method(&mut self, type_name: &str, entry: MethodEntry) {
        self.type_methods
            .entry(type_name.to_string())
            .or_default()
            .push(entry);
    }

    pub fn find_method(&self, type_name: &str, method_name: &str) -> Option<&MethodEntry> {
        self.type_methods
            .get(type_name)?
            .iter()
            .find(|m| m.method_name == method_name)
    }

    pub fn register(&mut self, name: String, kind: TypeKind) -> TypeId {
        let id = TypeId(self.next_id);
        self.next_id += 1;
        self.entries.push(TypeEntry {
            id: id.clone(),
            name,
            kind,
        });
        id
    }

    pub fn lookup_by_name(&self, name: &str) -> Option<&TypeEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    pub fn lookup_by_id(&self, id: &TypeId) -> Option<&TypeEntry> {
        self.entries.iter().find(|e| e.id == *id)
    }

    pub fn register_conformance(
        &mut self,
        type_name: &str,
        iface_name: &str,
        entry: ConformanceEntry,
    ) {
        self.conformances
            .entry((type_name.to_string(), iface_name.to_string()))
            .or_default()
            .push(entry);
    }

    pub fn get_conformances(&self, type_name: &str, iface_name: &str) -> &[ConformanceEntry] {
        self.conformances
            .get(&(type_name.to_string(), iface_name.to_string()))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn register_interface_method(&mut self, iface_name: &str, entry: MethodEntry) {
        self.interface_methods
            .entry(iface_name.to_string())
            .or_default()
            .push(entry);
    }

    pub fn get_interface_method(
        &self,
        iface_name: &str,
        method_name: &str,
    ) -> Option<&MethodEntry> {
        self.interface_methods
            .get(iface_name)?
            .iter()
            .find(|m| m.method_name == method_name)
    }

    pub fn list_interface_methods(&self, iface_name: &str) -> &[MethodEntry] {
        self.interface_methods
            .get(iface_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Returns all interface names that declare a hook or method with `hook_name`.
    pub fn interfaces_for_hook(&self, hook_name: &str) -> Vec<String> {
        self.interface_methods
            .iter()
            .filter(|(_, methods)| methods.iter().any(|m| m.method_name == hook_name))
            .map(|(iface, _)| iface.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ty_display_primitives() {
        assert_eq!(Ty::Int.to_string(), "int");
        assert_eq!(Ty::Float.to_string(), "float");
        assert_eq!(Ty::Bool.to_string(), "bool");
        assert_eq!(Ty::Str.to_string(), "str");
        assert_eq!(Ty::Void.to_string(), "void");
    }

    #[test]
    fn ty_display_named_with_args() {
        let ty = Ty::Named(TypeId(0), "Option".into(), vec![Ty::Int]);
        assert_eq!(ty.to_string(), "Option[int]");
    }

    #[test]
    fn ty_display_callable() {
        let c = Ty::Callable(vec![Ty::Int, Ty::Str], Box::new(Ty::Bool));
        assert_eq!(c.to_string(), "Callable[(int, str), bool]");
    }

    #[test]
    fn type_registry_roundtrip() {
        let mut reg = TypeRegistry::new();
        let id = reg.register("Point".into(), TypeKind::Struct);
        let entry = reg.lookup_by_id(&id).unwrap();
        assert_eq!(entry.name, "Point");
    }

    #[test]
    fn interfaces_for_hook_finds_single() {
        let mut reg = TypeRegistry::new();
        reg.register_interface_method(
            "Zero",
            MethodEntry {
                method_name: "zero".into(),
                qualified_fn: "zero".into(),
                params: vec![],
                ret: Ty::Unknown,
            },
        );
        let ifaces = reg.interfaces_for_hook("zero");
        assert_eq!(ifaces, vec!["Zero".to_string()]);
    }

    #[test]
    fn interfaces_for_hook_returns_empty_when_not_found() {
        let reg = TypeRegistry::new();
        assert!(reg.interfaces_for_hook("nonexistent").is_empty());
    }

    #[test]
    fn interfaces_for_hook_finds_multiple() {
        let mut reg = TypeRegistry::new();
        for iface in &["Display", "Debug"] {
            reg.register_interface_method(
                iface,
                MethodEntry {
                    method_name: "to_str".into(),
                    qualified_fn: "to_str".into(),
                    params: vec![],
                    ret: Ty::Str,
                },
            );
        }
        let mut ifaces = reg.interfaces_for_hook("to_str");
        ifaces.sort();
        assert_eq!(ifaces, vec!["Debug".to_string(), "Display".to_string()]);
    }
}
