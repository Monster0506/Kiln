use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

/// Substitute any `Ty::Projection { base, assoc }` whose `(base, assoc)` key
/// appears in `pins` with its pinned concrete type, recursing into all
/// compound type forms.  Leaves everything else unchanged.
pub fn normalize_ty(ty: &Ty, pins: &HashMap<(String, String), Ty>) -> Ty {
    match ty {
        Ty::Projection { base, assoc } => {
            let key = (base.clone(), assoc.clone());
            if let Some(resolved) = pins.get(&key) {
                normalize_ty(resolved, pins)
            } else {
                ty.clone()
            }
        }
        Ty::Named(id, name, args) => Ty::Named(
            id.clone(),
            name.clone(),
            args.iter().map(|a| normalize_ty(a, pins)).collect(),
        ),
        Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| normalize_ty(t, pins)).collect()),
        Ty::Callable(ps, r) => Ty::Callable(
            ps.iter().map(|p| normalize_ty(p, pins)).collect(),
            Box::new(normalize_ty(r, pins)),
        ),
        Ty::Ref(t, m) => Ty::Ref(Box::new(normalize_ty(t, pins)), *m),
        Ty::Union(ts) => Ty::Union(ts.iter().map(|t| normalize_ty(t, pins)).collect()),
        Ty::Compound(ts) => Ty::Compound(ts.iter().map(|t| normalize_ty(t, pins)).collect()),
        other => other.clone(),
    }
}

/// Computed variance for a type parameter position, inferred from method signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputedVariance {
    /// Appears only in output/return positions.
    Covariant,
    /// Appears only in input/parameter positions.
    Contravariant,
    /// Appears in both positions, or declared invariant.
    Invariant,
    /// Does not appear in any position.
    Bivariant,
}

impl ComputedVariance {
    /// Combine two variance observations at the same parameter.
    /// Invariant contaminates: once any position forces invariant, the param is invariant.
    pub fn combine(self, other: ComputedVariance) -> ComputedVariance {
        match (self, other) {
            (ComputedVariance::Invariant, _) | (_, ComputedVariance::Invariant) => {
                ComputedVariance::Invariant
            }
            (ComputedVariance::Covariant, ComputedVariance::Contravariant)
            | (ComputedVariance::Contravariant, ComputedVariance::Covariant) => {
                ComputedVariance::Invariant
            }
            (ComputedVariance::Bivariant, other) | (other, ComputedVariance::Bivariant) => other,
            (a, _) => a,
        }
    }
}

/// One way a type can satisfy an interface.
/// All bounds must hold for this entry to count.
/// An empty `bounds` vec means unconditional conformance.
#[derive(Debug, Clone)]
pub struct ConformanceEntry {
    /// Each element is `(param_name, interface_name)`.
    /// For generic containers: e.g. `Vec[T]: Display` stores `[("T", "Display")]`.
    /// For concrete types: empty.
    pub bounds: Vec<(String, String)>,
    /// Associated type bindings declared in the impl block: `type Item = int` stores `[("Item", Ty::Int)]`.
    pub bindings: Vec<(String, Ty)>,
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
    /// `Iface1+Iface2` -- compound interface type (value must satisfy all).
    Compound(Vec<Ty>),
    /// `T.Item` -- associated type projection. Emitted when no pin is in scope.
    Projection {
        base: String,
        assoc: String,
    },
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
            Ty::Compound(ts) => {
                let s = ts
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join("+");
                write!(f, "{s}")
            }
            Ty::Projection { base, assoc } => write!(f, "{base}.{assoc}"),
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
    /// iface_name -> direct superinterfaces it extends
    interface_supers: HashMap<String, Vec<String>>,
    /// iface_name -> all transitively reachable superinterfaces (precomputed)
    transitive_supers: HashMap<String, HashSet<String>>,
    /// type_name -> generic parameter names in declaration order
    generic_param_order: HashMap<String, Vec<String>>,
    /// (type_name, param_index) -> computed variance for that position
    type_variance: HashMap<(String, usize), ComputedVariance>,
    /// assoc_type_name -> interfaces that declare it (e.g. "Item" -> ["Iterator", "Collection"])
    assoc_type_index: HashMap<String, Vec<String>>,
    /// iface_name -> associated type names declared in that interface body
    iface_assoc_types: HashMap<String, Vec<String>>,
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

    /// Returns true if `name` is a variant name of any registered enum.
    pub fn is_enum_variant(&self, name: &str) -> bool {
        self.entries.iter().any(|e| {
            matches!(&e.kind, TypeKind::Enum { variant_names } if variant_names.iter().any(|v| v == name))
        })
    }

    /// Returns the `TypeEntry` for the enum that contains `variant_name`, if any.
    pub fn enum_for_variant(&self, variant_name: &str) -> Option<&TypeEntry> {
        self.entries.iter().find(|e| {
            matches!(&e.kind, TypeKind::Enum { variant_names } if variant_names.iter().any(|v| v == variant_name))
        })
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

    /// Returns all directly registered conformance triples as `(type_name, iface_name, entries)`.
    pub fn all_direct_conformances(&self) -> Vec<(String, String, Vec<ConformanceEntry>)> {
        self.conformances
            .iter()
            .map(|((t, i), entries)| (t.clone(), i.clone(), entries.clone()))
            .collect()
    }

    /// Look up the concrete type bound to an associated type in a specific impl.
    /// Returns `Some(ty)` if `type_name` implements `iface_name` with `type assoc_name = ty`.
    pub fn get_assoc_binding(
        &self,
        type_name: &str,
        iface_name: &str,
        assoc_name: &str,
    ) -> Option<Ty> {
        self.conformances
            .get(&(type_name.to_string(), iface_name.to_string()))?
            .iter()
            .find_map(|entry| {
                entry
                    .bindings
                    .iter()
                    .find(|(name, _)| name == assoc_name)
                    .map(|(_, ty)| ty.clone())
            })
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

    pub fn register_interface_supers(&mut self, iface: &str, supers: Vec<String>) {
        self.interface_supers.insert(iface.to_string(), supers);
    }

    /// Precompute the full transitive superinterface closure for every interface
    /// that has direct supers registered. Call once after all interfaces are registered.
    /// After this, `iface_implies` uses O(1) set lookup instead of BFS per query.
    pub fn precompute_transitive_closures(&mut self) {
        let iface_names: Vec<String> = self.interface_supers.keys().cloned().collect();
        for iface in &iface_names {
            if self.transitive_supers.contains_key(iface.as_str()) {
                continue;
            }
            let mut visited = HashSet::new();
            let mut queue = VecDeque::new();
            if let Some(supers) = self.interface_supers.get(iface.as_str()) {
                for s in supers {
                    queue.push_back(s.clone());
                }
            }
            while let Some(current) = queue.pop_front() {
                if !visited.insert(current.clone()) {
                    continue;
                }
                if let Some(supers) = self.interface_supers.get(&current) {
                    for s in supers {
                        queue.push_back(s.clone());
                    }
                }
            }
            self.transitive_supers.insert(iface.clone(), visited);
        }
    }

    /// Returns the precomputed set of all transitive superinterfaces of `iface`,
    /// or `None` if no closure was precomputed for it (e.g. it has no supers).
    pub fn get_transitive_supers(&self, iface: &str) -> Option<&HashSet<String>> {
        self.transitive_supers.get(iface)
    }

    /// True if `from` implies `target` through the superinterface chain.
    /// Uses the precomputed transitive closure when available; falls back to BFS.
    pub fn iface_implies(&self, from: &str, target: &str) -> bool {
        if from == target {
            return true;
        }
        if let Some(set) = self.transitive_supers.get(from) {
            return set.contains(target);
        }
        // Fall back to BFS (before precompute has been called, or for interfaces with no supers).
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(from.to_string());
        while let Some(current) = queue.pop_front() {
            if current == target {
                return true;
            }
            if !visited.insert(current.clone()) {
                continue;
            }
            if let Some(supers) = self.interface_supers.get(&current) {
                for s in supers {
                    queue.push_back(s.clone());
                }
            }
        }
        false
    }

    /// All interfaces for which this type has a direct conformance entry.
    pub fn conformance_ifaces_for(&self, type_name: &str) -> Vec<&str> {
        self.conformances
            .keys()
            .filter(|(t, _)| t == type_name)
            .map(|(_, i)| i.as_str())
            .collect()
    }

    // -----------------------------------------------------------------------
    // Variance infrastructure
    // -----------------------------------------------------------------------

    /// Register the ordered list of generic parameter names for a type.
    /// Must be called before `register_type_variance` for that type.
    pub fn register_generic_param_order(&mut self, type_name: &str, params: Vec<String>) {
        self.generic_param_order
            .insert(type_name.to_string(), params);
    }

    /// Returns the generic parameter names in declaration order, if registered.
    pub fn get_generic_param_order(&self, type_name: &str) -> Option<&[String]> {
        self.generic_param_order
            .get(type_name)
            .map(|v| v.as_slice())
    }

    /// Override the computed variance for a specific parameter position.
    /// The caller is responsible for ensuring this is consistent with the
    /// type's actual usage (use for declared invariance sources like Mutex).
    pub fn register_type_variance(
        &mut self,
        type_name: &str,
        param_idx: usize,
        v: ComputedVariance,
    ) {
        self.type_variance
            .insert((type_name.to_string(), param_idx), v);
    }

    /// Returns the variance for the given parameter index.
    /// Defaults to `Covariant` when not explicitly set, matching the
    /// conservative assumption that the majority of containers are read-through.
    pub fn get_type_variance(&self, type_name: &str, param_idx: usize) -> ComputedVariance {
        self.type_variance
            .get(&(type_name.to_string(), param_idx))
            .copied()
            .unwrap_or(ComputedVariance::Covariant)
    }

    // -----------------------------------------------------------------------
    // Associated type tracking
    // -----------------------------------------------------------------------

    /// Record that `iface_name` declares an associated type `assoc_name`.
    /// Call once per assoc type during interface registration.
    pub fn register_iface_assoc_type(&mut self, iface_name: &str, assoc_name: &str) {
        self.iface_assoc_types
            .entry(iface_name.to_string())
            .or_default()
            .push(assoc_name.to_string());
        self.assoc_type_index
            .entry(assoc_name.to_string())
            .or_default()
            .push(iface_name.to_string());
    }

    /// Names of associated types declared inside `iface_name`, in declaration order.
    pub fn get_iface_assoc_types(&self, iface_name: &str) -> &[String] {
        self.iface_assoc_types
            .get(iface_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Returns `true` if `assoc_name` is an associated type declared in `iface_name`.
    pub fn is_assoc_type_of(&self, assoc_name: &str, iface_name: &str) -> bool {
        self.iface_assoc_types
            .get(iface_name)
            .is_some_and(|v| v.iter().any(|a| a == assoc_name))
    }

    /// All interface names that declare an associated type with `assoc_name`.
    pub fn interfaces_declaring_assoc(&self, assoc_name: &str) -> &[String] {
        self.assoc_type_index
            .get(assoc_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// All `(assoc_name, concrete_ty)` bindings from all conformance entries for `type_name`.
    /// Used during monomorphization to extend substitution maps with associated type resolutions.
    pub fn all_assoc_bindings_for(&self, type_name: &str) -> Vec<(String, Ty)> {
        let mut result = Vec::new();
        for ((t, _), entries) in &self.conformances {
            if t == type_name {
                for entry in entries {
                    for (name, ty) in &entry.bindings {
                        if !result.iter().any(|(n, _)| n == name) {
                            result.push((name.clone(), ty.clone()));
                        }
                    }
                }
            }
        }
        result
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

    #[test]
    fn precompute_makes_iface_implies_correct() {
        let mut reg = TypeRegistry::new();
        reg.register_interface_supers("Ord", vec!["PartialOrd".to_string()]);
        reg.register_interface_supers("PartialOrd", vec!["Eq".to_string()]);
        reg.precompute_transitive_closures();
        assert!(reg.iface_implies("Ord", "PartialOrd"));
        assert!(reg.iface_implies("Ord", "Eq"), "two-hop implication");
        assert!(!reg.iface_implies("Ord", "Display"), "unrelated");
        assert!(reg.iface_implies("PartialOrd", "Eq"));
        assert!(!reg.iface_implies("Eq", "Ord"), "not reflexive upward");
    }

    #[test]
    fn iface_implies_self_is_always_true() {
        let reg = TypeRegistry::new();
        assert!(reg.iface_implies("Ord", "Ord"));
        assert!(reg.iface_implies("Display", "Display"));
    }

    #[test]
    fn get_transitive_supers_returns_full_set() {
        let mut reg = TypeRegistry::new();
        reg.register_interface_supers("Ord", vec!["PartialOrd".to_string()]);
        reg.register_interface_supers("PartialOrd", vec!["Eq".to_string()]);
        reg.precompute_transitive_closures();
        let supers = reg.get_transitive_supers("Ord").unwrap();
        assert!(supers.contains("PartialOrd"));
        assert!(supers.contains("Eq"));
        assert_eq!(supers.len(), 2);
    }

    #[test]
    fn get_transitive_supers_returns_none_for_leaf() {
        let mut reg = TypeRegistry::new();
        reg.register_interface_supers("Ord", vec!["PartialOrd".to_string()]);
        reg.precompute_transitive_closures();
        // PartialOrd has no registered supers, so no closure was computed for it.
        assert!(reg.get_transitive_supers("PartialOrd").is_none());
    }
}
