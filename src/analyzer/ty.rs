use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InterfaceId(pub u32);

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Int,
    Float,
    Bool,
    Str,
    Void,
    Option(Box<Ty>),
    Vec(Box<Ty>),
    Set(Box<Ty>),
    Map(Box<Ty>, Box<Ty>),
    Tuple(Vec<Ty>),
    Callable(Vec<Ty>, Box<Ty>),
    Shared(Box<Ty>),
    Ref(Box<Ty>, bool),
    Union(Vec<Ty>),
    Named(TypeId, String),
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
            Ty::Option(t) => write!(f, "Option[{t}]"),
            Ty::Vec(t) => write!(f, "Vec[{t}]"),
            Ty::Set(t) => write!(f, "Set[{t}]"),
            Ty::Map(k, v) => write!(f, "Map[{k}, {v}]"),
            Ty::Shared(t) => write!(f, "Shared[{t}]"),
            Ty::Ref(t, true) => write!(f, "&mut {t}"),
            Ty::Ref(t, false) => write!(f, "&{t}"),
            Ty::Named(_, name) => write!(f, "{name}"),
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
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
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
        self.entries.get(id.0 as usize)
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
    fn ty_display_option() {
        assert_eq!(Ty::Option(Box::new(Ty::Int)).to_string(), "Option[int]");
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
}
