use crate::analyzer::ty::Ty;
use crate::analyzer::typed_ast::{TypedEnumDef, TypedStructDef};
use crate::parser::ast::{EnumDef, EnumVariant, StructDef, TypeExpr};
use cranelift_module::FuncId;
use std::collections::{HashMap, HashSet};

fn ty_size(ty: &Ty) -> u32 {
    match ty {
        Ty::Bool => 1,
        _ => 8,
    }
}

fn ty_align(ty: &Ty) -> u32 {
    match ty {
        Ty::Bool => 1,
        _ => 8,
    }
}

pub struct FieldInfo {
    pub offset: u32,
    pub size: u32,
}

pub struct StructInfo {
    fields: Vec<(String, FieldInfo)>,
    indirect_fields: HashSet<String>,
    pub size: u32,
}

impl StructInfo {
    pub fn field_offset(&self, name: &str) -> Option<u32> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, f)| f.offset)
    }

    pub fn field_info(&self, name: &str) -> Option<&FieldInfo> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, f)| f)
    }

    pub fn fields(&self) -> &[(String, FieldInfo)] {
        &self.fields
    }

    pub fn is_indirect(&self, field_name: &str) -> bool {
        self.indirect_fields.contains(field_name)
    }
}

/// Per-variant layout within an enum.
pub struct EnumVariantLayout {
    pub discriminant: u32,
    /// (field_name, absolute_byte_offset_from_enum_base)
    pub fields: Vec<(String, u32)>,
}

pub struct EnumInfo {
    /// variant_name -> layout
    pub variants: HashMap<String, EnumVariantLayout>,
    pub discriminant_size: u32,
    pub payload_offset: u32,
    pub max_payload_size: u32,
    pub size: u32,
}

pub struct StructLayouts {
    structs: HashMap<String, StructInfo>,
    enums: HashMap<String, EnumInfo>,
    /// type_name -> integer type ID (for runtime vtable dispatch)
    type_ids: HashMap<String, u32>,
    next_type_id: u32,
    /// method_name -> [(type_id, func_id)] for vtable dispatch
    vtable_entries: HashMap<String, Vec<(u32, FuncId)>>,
    /// iface_name -> [type_ids] that implement the interface (for implements())
    iface_conformance: HashMap<String, Vec<u32>>,
}

impl Default for StructLayouts {
    fn default() -> Self {
        Self::new()
    }
}

impl StructLayouts {
    pub fn new() -> Self {
        let mut type_ids = HashMap::new();
        // Assign stable type IDs to primitives so they don't all share ID 0.
        type_ids.insert("int".to_string(), 1u32);
        type_ids.insert("float".to_string(), 2u32);
        type_ids.insert("bool".to_string(), 3u32);
        type_ids.insert("str".to_string(), 4u32);
        Self {
            structs: HashMap::new(),
            enums: HashMap::new(),
            type_ids,
            next_type_id: 5,
            vtable_entries: HashMap::new(),
            iface_conformance: HashMap::new(),
        }
    }

    pub fn register_typed_struct(&mut self, st: &TypedStructDef) {
        let type_id = self.next_type_id;
        self.next_type_id += 1;
        self.type_ids.insert(st.name.clone(), type_id);
        let mut offset: u32 = 8;
        let mut fields = Vec::new();
        let mut indirect_fields = HashSet::new();
        for f in &st.fields {
            // @indirect fields are always pointer-sized regardless of actual type.
            let size = if f.is_indirect { 8 } else { ty_size(&f.ty) };
            let align = if f.is_indirect { 8 } else { ty_align(&f.ty) };
            offset = align_up(offset, align);
            fields.push((f.name.clone(), FieldInfo { offset, size }));
            if f.is_indirect {
                indirect_fields.insert(f.name.clone());
            }
            offset += size;
        }
        let size = align_up(offset, 8);
        self.structs.insert(
            st.name.clone(),
            StructInfo {
                fields,
                indirect_fields,
                size,
            },
        );
    }

    pub fn register_typed_enum(&mut self, en: &TypedEnumDef) {
        let type_id = self.next_type_id;
        self.next_type_id += 1;
        self.type_ids.insert(en.name.clone(), type_id);
        let discriminant_size: u32 = 4;
        let payload_offset = align_up(discriminant_size, 8);
        let max_payload: u32 = en
            .variants
            .iter()
            .map(|v| v.fields.iter().map(|f| ty_size(&f.ty)).sum::<u32>())
            .max()
            .unwrap_or(0);
        let mut variants = std::collections::HashMap::new();
        for (i, v) in en.variants.iter().enumerate() {
            let disc = v.discriminant.map(|d| d as u32).unwrap_or(i as u32);
            let mut field_offset = payload_offset;
            let mut fields_layout = Vec::new();
            for f in &v.fields {
                let align = ty_align(&f.ty);
                field_offset = align_up(field_offset, align);
                fields_layout.push((f.name.clone(), field_offset));
                field_offset += ty_size(&f.ty);
            }
            variants.insert(
                v.name.clone(),
                EnumVariantLayout {
                    discriminant: disc,
                    fields: fields_layout,
                },
            );
        }
        let size = align_up(payload_offset + max_payload, 8);
        self.enums.insert(
            en.name.clone(),
            EnumInfo {
                variants,
                discriminant_size,
                payload_offset,
                max_payload_size: max_payload,
                size,
            },
        );
    }

    pub fn register_struct(&mut self, st: &StructDef) {
        let type_id = self.next_type_id;
        self.next_type_id += 1;
        self.type_ids.insert(st.name.clone(), type_id);

        // First 8 bytes are the type tag; user fields start at offset 8.
        let mut offset: u32 = 8;
        let mut fields = Vec::new();
        for f in &st.fields {
            let size = type_expr_size(&f.ty);
            let align = type_expr_align(&f.ty);
            offset = align_up(offset, align);
            fields.push((f.name.clone(), FieldInfo { offset, size }));
            offset += size;
        }
        let size = align_up(offset, 8);
        self.structs.insert(
            st.name.clone(),
            StructInfo {
                fields,
                indirect_fields: HashSet::new(),
                size,
            },
        );
    }

    pub fn register_enum(&mut self, en: &EnumDef) {
        let type_id = self.next_type_id;
        self.next_type_id += 1;
        self.type_ids.insert(en.name.clone(), type_id);

        let discriminant_size: u32 = 4;
        let payload_offset = align_up(discriminant_size, 8);

        let max_payload = en
            .variants
            .iter()
            .map(variant_payload_size)
            .max()
            .unwrap_or(0);

        let mut variants: HashMap<String, EnumVariantLayout> = HashMap::new();
        for (i, v) in en.variants.iter().enumerate() {
            let disc = v.discriminant.map(|d| d as u32).unwrap_or(i as u32);
            // Compute per-field absolute offsets within this variant.
            let mut field_offset = payload_offset;
            let mut fields_layout: Vec<(String, u32)> = Vec::new();
            for f in &v.fields {
                let align = type_expr_align(&f.ty);
                field_offset = align_up(field_offset, align);
                fields_layout.push((f.name.clone(), field_offset));
                field_offset += type_expr_size(&f.ty);
            }
            variants.insert(
                v.name.clone(),
                EnumVariantLayout {
                    discriminant: disc,
                    fields: fields_layout,
                },
            );
        }

        let size = align_up(payload_offset + max_payload, 8);

        self.enums.insert(
            en.name.clone(),
            EnumInfo {
                variants,
                discriminant_size,
                payload_offset,
                max_payload_size: max_payload,
                size,
            },
        );
    }

    pub fn get_struct(&self, name: &str) -> Option<&StructInfo> {
        self.structs.get(name)
    }

    pub fn get_enum(&self, name: &str) -> Option<&EnumInfo> {
        self.enums.get(name)
    }

    /// Find an enum variant by name across all registered enums.
    /// Returns `(enum_info, variant_layout)`.
    pub fn get_enum_variant(&self, variant_name: &str) -> Option<(&EnumInfo, &EnumVariantLayout)> {
        for info in self.enums.values() {
            if let Some(vl) = info.variants.get(variant_name) {
                return Some((info, vl));
            }
        }
        None
    }

    /// Search all registered structs for any struct that has a field with the
    /// given name, and return that field's byte offset.
    pub fn find_field_offset(&self, field_name: &str) -> Option<u32> {
        for info in self.structs.values() {
            if let Some(offset) = info.field_offset(field_name) {
                return Some(offset);
            }
        }
        None
    }

    /// Find the offset of `field_name` within struct `type_name`.
    pub fn field_offset_for_type(&self, type_name: &str, field_name: &str) -> Option<u32> {
        self.structs.get(type_name)?.field_offset(field_name)
    }

    /// Return the runtime type ID assigned to a named type.
    pub fn get_type_id(&self, name: &str) -> Option<u32> {
        self.type_ids.get(name).copied()
    }

    /// Register a vtable entry: this `func_id` implements `method_name` for objects
    /// whose type tag equals `type_id`.
    pub fn register_vtable_entry(&mut self, method_name: &str, type_id: u32, func_id: FuncId) {
        self.vtable_entries
            .entry(method_name.to_string())
            .or_default()
            .push((type_id, func_id));
    }

    /// Record that `type_id` implements interface `iface_name`.
    pub fn register_conformance(&mut self, iface_name: &str, type_id: u32) {
        let ids = self
            .iface_conformance
            .entry(iface_name.to_string())
            .or_default();
        if !ids.contains(&type_id) {
            ids.push(type_id);
        }
    }

    /// Return the type name for a given type ID (reverse of get_type_id).
    pub fn type_name_for_id(&self, id: u32) -> Option<&str> {
        self.type_ids
            .iter()
            .find(|(_, &v)| v == id)
            .map(|(k, _)| k.as_str())
    }

    /// Iterate over all registered (name, type_id) pairs.
    pub fn all_type_ids(&self) -> impl Iterator<Item = (&str, u32)> {
        self.type_ids.iter().map(|(k, &v)| (k.as_str(), v))
    }

    /// Return all type IDs whose concrete type implements `iface_name`.
    pub fn type_ids_for_iface(&self, iface_name: &str) -> &[u32] {
        self.iface_conformance
            .get(iface_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Return all (type_id, func_id) pairs that implement `method_name`.
    pub fn all_impls_for_method(&self, method_name: &str) -> &[(u32, FuncId)] {
        self.vtable_entries
            .get(method_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Iterate over all registered struct names and their layouts.
    pub fn struct_names(&self) -> impl Iterator<Item = &str> {
        self.structs.keys().map(|s| s.as_str())
    }

    /// Return true if the named field of the named struct is @indirect.
    pub fn is_indirect_field(&self, type_name: &str, field_name: &str) -> bool {
        self.structs
            .get(type_name)
            .is_some_and(|info| info.is_indirect(field_name))
    }
}

pub fn type_expr_size(ty: &TypeExpr) -> u32 {
    match ty {
        TypeExpr::Named { name, .. } => match name.as_str() {
            "bool" => 1,
            "int" | "float" => 8,
            "str" => 16,
            _ => 8,
        },
        TypeExpr::Tuple(elems, _) => elems.iter().map(type_expr_size).sum(),
        _ => 8,
    }
}

pub fn type_expr_align(ty: &TypeExpr) -> u32 {
    match ty {
        TypeExpr::Named { name, .. } => match name.as_str() {
            "bool" => 1,
            _ => 8,
        },
        _ => 8,
    }
}

fn variant_payload_size(v: &EnumVariant) -> u32 {
    v.fields.iter().map(|f| type_expr_size(&f.ty)).sum()
}

fn align_up(offset: u32, align: u32) -> u32 {
    (offset + align - 1) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Span;
    use crate::parser::ast::{Field, StructDef, TypeExpr};

    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    fn named_ty(n: &str) -> TypeExpr {
        TypeExpr::Named {
            name: n.into(),
            generics: vec![],
            bindings: vec![],
            span: s(),
        }
    }

    fn field(name: &str, ty: &str) -> Field {
        Field {
            annotations: vec![],
            is_priv: false,
            name: name.into(),
            ty: named_ty(ty),
            default: None,
            span: s(),
        }
    }

    #[test]
    fn struct_field_offsets_computed() {
        let st = StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "Point".into(),
            generic_params: vec![],
            interfaces: vec![],
            fields: vec![field("x", "float"), field("y", "float")],
            methods: vec![],
            decls: vec![],
            inline_hooks: vec![],
            span: s(),
        };
        let mut layouts = StructLayouts::new();
        layouts.register_struct(&st);
        let info = layouts.get_struct("Point").unwrap();
        // Fields start at offset 8 (type tag occupies bytes 0..7).
        assert_eq!(info.field_offset("x"), Some(8));
        assert_eq!(info.field_offset("y"), Some(16));
        assert_eq!(info.size, 24);
    }

    #[test]
    fn bool_field_padded_to_alignment() {
        let st = StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "Flags".into(),
            generic_params: vec![],
            interfaces: vec![],
            fields: vec![field("a", "bool"), field("b", "int")],
            methods: vec![],
            decls: vec![],
            inline_hooks: vec![],
            span: s(),
        };
        let mut layouts = StructLayouts::new();
        layouts.register_struct(&st);
        let info = layouts.get_struct("Flags").unwrap();
        // bool at 8, int aligned to 8 -> at 16
        assert_eq!(info.field_offset("a"), Some(8));
        assert_eq!(info.field_offset("b"), Some(16));
        assert_eq!(info.size, 24);
    }

    #[test]
    fn type_ids_are_assigned() {
        let st = StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "MyStruct".into(),
            generic_params: vec![],
            interfaces: vec![],
            fields: vec![],
            methods: vec![],
            decls: vec![],
            inline_hooks: vec![],
            span: s(),
        };
        let mut layouts = StructLayouts::new();
        layouts.register_struct(&st);
        // Primitives reserve IDs 1-4; first user struct gets 5.
        assert_eq!(layouts.get_type_id("MyStruct"), Some(5));
    }

    #[test]
    fn option_layout_none_discriminant_and_some_value_offset() {
        use crate::analyzer::ty::Ty;
        use crate::analyzer::typed_ast::{TypedEnumDef, TypedEnumVariant, TypedField};

        let span = s();
        let opt = TypedEnumDef {
            name: "Option".into(),
            variants: vec![
                TypedEnumVariant {
                    name: "Some".into(),
                    fields: vec![TypedField {
                        name: "value".into(),
                        ty: Ty::Unknown,
                        is_indirect: false,
                        is_priv: false,
                        span,
                    }],
                    discriminant: None,
                    span,
                },
                TypedEnumVariant {
                    name: "None".into(),
                    fields: vec![],
                    discriminant: None,
                    span,
                },
            ],
            span,
        };
        let mut layouts = StructLayouts::new();
        layouts.register_typed_enum(&opt);
        let info = layouts.get_enum("Option").unwrap();

        let none_layout = info.variants.get("None").unwrap();
        let some_layout = info.variants.get("Some").unwrap();
        let value_offset = some_layout
            .fields
            .iter()
            .find(|(n, _)| n == "value")
            .unwrap()
            .1;

        // lower_for_iterable queries these from the layout system.
        // If they change, both this test and the hardcoded constants must be updated together.
        assert_eq!(
            none_layout.discriminant, 1,
            "None is the second variant (index 1); lower_for_iterable relies on this"
        );
        assert_eq!(
            value_offset, info.payload_offset,
            "Some.value is at payload_offset (first field of first-payload variant)"
        );
        assert_eq!(
            info.payload_offset, 8,
            "payload_offset must be 8 (discriminant_size=4, aligned to 8)"
        );
    }
}
