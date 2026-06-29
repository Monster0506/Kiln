use crate::annotations::api::{AnnotationArgs, SourceAnnotationTarget};
use crate::annotations::source_builders::*;
use crate::annotations::ProcessorRegistry;
use crate::parser::ast::{BinOp, EnumDef, Field, HookName, ImplBlock, Item, SourceFile, StructDef};

pub fn register(registry: &mut ProcessorRegistry) {
    registry.register_source("derive", process_derive);
}

pub fn process_derive(
    _file: &SourceFile,
    target: SourceAnnotationTarget,
    args: AnnotationArgs,
) -> Vec<Item> {
    let mut items = Vec::new();
    match target {
        SourceAnnotationTarget::Struct(st) => {
            for (trait_name, _) in args {
                match trait_name.as_str() {
                    "Eq" => items.extend(derive_eq_struct(st)),
                    "Display" => items.push(derive_display_struct(st)),
                    "Comparable" => items.extend(derive_comparable_struct(st)),
                    _ => {}
                }
            }
        }
        SourceAnnotationTarget::Enum(en) => {
            for (trait_name, _) in args {
                match trait_name.as_str() {
                    "Eq" => items.extend(derive_eq_enum(en)),
                    "Comparable" => items.extend(derive_comparable_enum(en)),
                    _ => {}
                }
            }
        }
        _ => {}
    }
    items
}

fn derive_eq_struct(st: &StructDef) -> Vec<Item> {
    let expr = if st.fields.is_empty() {
        sbool(true)
    } else {
        let mut it = st.fields.iter();
        let first = it.next().unwrap();
        let init = field_eq_expr(first);
        it.fold(init, |acc, f| sbinop(BinOp::And, acc, field_eq_expr(f)))
    };
    let hook = shook(
        HookName::Op("==".into()),
        vec![sparam("other", stype_named(&st.name))],
        stype_bool(),
        sblock(vec![sreturn(Some(expr))]),
    );
    vec![
        Item::ImplBlock(simpl("PartialEq", &st.name, vec![hook])),
        Item::ImplBlock(simpl("Eq", &st.name, vec![])),
    ]
}

fn field_eq_expr(f: &Field) -> crate::parser::ast::Expr {
    sbinop(
        BinOp::Eq,
        sfield_access("self", &f.name),
        sfield_access("other", &f.name),
    )
}

fn derive_display_struct(st: &StructDef) -> Item {
    use crate::parser::ast::StringSegment;
    let mut segs = vec![StringSegment::Text(format!("{} {{ ", st.name))];
    for (i, f) in st.fields.iter().enumerate() {
        if i > 0 {
            segs.push(StringSegment::Text(", ".into()));
        }
        segs.push(StringSegment::Text(format!("{}: ", f.name)));
        segs.push(StringSegment::Interp(sfield_access("self", &f.name)));
    }
    segs.push(StringSegment::Text(" }".into()));
    let body_expr = crate::parser::ast::Expr::Str(segs, s());
    let hook = shook(
        HookName::Named("to_str".into()),
        vec![],
        stype_str(),
        sblock(vec![sreturn(Some(body_expr))]),
    );
    Item::ImplBlock(simpl("Display", &st.name, vec![hook]))
}

fn derive_comparable_struct(st: &StructDef) -> Vec<Item> {
    let cmp_body = if st.fields.is_empty() {
        sblock(vec![sreturn(Some(senum_access("Ordering", "Equal")))])
    } else {
        let mut stmts = vec![];
        for f in &st.fields {
            let sf = sfield_access("self", &f.name);
            let of = sfield_access("other", &f.name);
            stmts.push(sif(
                vec![(
                    sbinop(BinOp::Lt, sf.clone(), of.clone()),
                    sblock(vec![sreturn(Some(senum_access("Ordering", "Less")))]),
                )],
                None,
            ));
            stmts.push(sif(
                vec![(
                    sbinop(BinOp::Lt, of, sf),
                    sblock(vec![sreturn(Some(senum_access("Ordering", "Greater")))]),
                )],
                None,
            ));
        }
        stmts.push(sreturn(Some(senum_access("Ordering", "Equal"))));
        sblock(stmts)
    };
    let cmp_hook = shook(
        HookName::Op("<=>".into()),
        vec![sparam("other", stype_named(&st.name))],
        stype_named("Ordering"),
        cmp_body,
    );
    vec![
        Item::ImplBlock(simpl("Ord", &st.name, vec![cmp_hook])),
        Item::ImplBlock(partial_ord_impl_src(&st.name)),
        Item::ImplBlock(simpl("Comparable", &st.name, vec![])),
    ]
}

fn partial_ord_impl_src(type_name: &str) -> ImplBlock {
    let lt_body = sblock(vec![sreturn(Some(sbinop(
        BinOp::Eq,
        sbinop(BinOp::Spaceship, sident("self"), sident("other")),
        senum_access("Ordering", "Less"),
    )))]);
    let lt_hook = shook(
        HookName::Op("<".into()),
        vec![sparam("other", stype_named(type_name))],
        stype_bool(),
        lt_body,
    );
    simpl("PartialOrd", type_name, vec![lt_hook])
}

fn enum_discriminant_expr(
    scrutinee: crate::parser::ast::Expr,
    en: &EnumDef,
) -> crate::parser::ast::Expr {
    let arms = en
        .variants
        .iter()
        .enumerate()
        .map(|(i, v)| smatch_arm(spattern_struct(&v.name, true), sint(i as i64)))
        .collect();
    smatch(scrutinee, arms)
}

fn derive_eq_enum(en: &EnumDef) -> Vec<Item> {
    let self_d = enum_discriminant_expr(sident("self"), en);
    let other_d = enum_discriminant_expr(sident("other"), en);
    let body = sblock(vec![sreturn(Some(sbinop(BinOp::Eq, self_d, other_d)))]);
    let hook = shook(
        HookName::Op("==".into()),
        vec![sparam("other", stype_named(&en.name))],
        stype_bool(),
        body,
    );
    vec![
        Item::ImplBlock(simpl("PartialEq", &en.name, vec![hook])),
        Item::ImplBlock(simpl("Eq", &en.name, vec![])),
    ]
}

fn derive_comparable_enum(en: &EnumDef) -> Vec<Item> {
    let self_d = enum_discriminant_expr(sident("self"), en);
    let other_d = enum_discriminant_expr(sident("other"), en);
    let stmts = vec![
        svar_decl("self_d", stype_int(), self_d, false),
        svar_decl("other_d", stype_int(), other_d, false),
        sif(
            vec![(
                sbinop(BinOp::Lt, sident("self_d"), sident("other_d")),
                sblock(vec![sreturn(Some(senum_access("Ordering", "Less")))]),
            )],
            None,
        ),
        sif(
            vec![(
                sbinop(BinOp::Lt, sident("other_d"), sident("self_d")),
                sblock(vec![sreturn(Some(senum_access("Ordering", "Greater")))]),
            )],
            None,
        ),
        sreturn(Some(senum_access("Ordering", "Equal"))),
    ];
    let cmp_hook = shook(
        HookName::Op("<=>".into()),
        vec![sparam("other", stype_named(&en.name))],
        stype_named("Ordering"),
        sblock(stmts),
    );
    vec![
        Item::ImplBlock(simpl("Ord", &en.name, vec![cmp_hook])),
        Item::ImplBlock(partial_ord_impl_src(&en.name)),
        Item::ImplBlock(simpl("Comparable", &en.name, vec![])),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Span;
    use crate::parser::ast::{EnumVariant, Field, Item, SourceFile, StructDef};

    fn sp() -> Span {
        Span::new(0, 0)
    }

    fn empty_file() -> SourceFile {
        SourceFile {
            items: vec![],
            span: sp(),
        }
    }

    fn make_field(name: &str) -> Field {
        Field {
            annotations: vec![],
            is_priv: false,
            name: name.into(),
            ty: stype_str(),
            default: None,
            span: sp(),
        }
    }

    fn make_struct(name: &str, fields: Vec<Field>) -> StructDef {
        StructDef {
            annotations: vec![],
            is_builtin: false,
            name: name.into(),
            generic_params: vec![],
            interfaces: vec![],
            fields,
            methods: vec![],
            decls: vec![],
            inline_hooks: vec![],
            span: sp(),
        }
    }

    fn make_enum(name: &str, variants: Vec<&str>) -> EnumDef {
        EnumDef {
            annotations: vec![],
            name: name.into(),
            generic_params: vec![],
            interfaces: vec![],
            variants: variants
                .into_iter()
                .map(|v| EnumVariant {
                    name: v.into(),
                    fields: vec![],
                    discriminant: None,
                    span: sp(),
                })
                .collect(),
            methods: vec![],
            span: sp(),
        }
    }

    fn derive_args(traits: &[&str]) -> Vec<(String, crate::parser::ast::Expr)> {
        traits
            .iter()
            .map(|t| (t.to_string(), crate::parser::ast::Expr::Bool(true, sp())))
            .collect()
    }

    #[test]
    fn derive_eq_struct_generates_partial_eq_and_eq() {
        let st = make_struct("Point", vec![make_field("x"), make_field("y")]);
        let args = derive_args(&["Eq"]);
        let items = process_derive(&empty_file(), SourceAnnotationTarget::Struct(&st), &args);
        assert_eq!(items.len(), 2);
        let ifaces: Vec<_> = items
            .iter()
            .filter_map(|i| {
                if let Item::ImplBlock(b) = i {
                    Some(&b.interface)
                } else {
                    None
                }
            })
            .collect();
        assert!(ifaces.iter().any(
            |t| matches!(t, crate::parser::ast::TypeExpr::Named { name, .. } if name == "PartialEq")
        ));
        assert!(ifaces.iter().any(
            |t| matches!(t, crate::parser::ast::TypeExpr::Named { name, .. } if name == "Eq")
        ));
    }

    #[test]
    fn derive_comparable_struct_generates_three_impls() {
        let st = make_struct("Point", vec![make_field("x")]);
        let args = derive_args(&["Comparable"]);
        let items = process_derive(&empty_file(), SourceAnnotationTarget::Struct(&st), &args);
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn derive_display_struct_generates_one_impl() {
        let st = make_struct("Point", vec![make_field("x")]);
        let args = derive_args(&["Display"]);
        let items = process_derive(&empty_file(), SourceAnnotationTarget::Struct(&st), &args);
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn derive_eq_enum_generates_two_impls() {
        let en = make_enum("Color", vec!["Red", "Green", "Blue"]);
        let args = derive_args(&["Eq"]);
        let items = process_derive(&empty_file(), SourceAnnotationTarget::Enum(&en), &args);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn derive_comparable_enum_generates_three_impls() {
        let en = make_enum("Priority", vec!["Low", "Medium", "High"]);
        let args = derive_args(&["Comparable"]);
        let items = process_derive(&empty_file(), SourceAnnotationTarget::Enum(&en), &args);
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn unknown_trait_produces_no_items() {
        let st = make_struct("Foo", vec![]);
        let args = derive_args(&["Hash"]);
        let items = process_derive(&empty_file(), SourceAnnotationTarget::Struct(&st), &args);
        assert!(items.is_empty());
    }

    #[test]
    fn derive_on_function_produces_no_items() {
        use crate::parser::ast::FnDef;
        let fn_def = FnDef {
            annotations: vec![],
            name: "foo".into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: stype_named("void"),
            throws: false,
            body: sblock(vec![]),
            is_declaration: false,
            span: sp(),
        };
        let args = derive_args(&["Eq"]);
        let items = process_derive(
            &empty_file(),
            SourceAnnotationTarget::Function(&fn_def),
            &args,
        );
        assert!(items.is_empty());
    }

    #[test]
    fn eq_struct_hook_has_correct_op_name() {
        let st = make_struct("Point", vec![make_field("x")]);
        let args = derive_args(&["Eq"]);
        let items = process_derive(&empty_file(), SourceAnnotationTarget::Struct(&st), &args);
        let Item::ImplBlock(b) = &items[0] else {
            panic!()
        };
        assert_eq!(b.hooks.len(), 1);
        assert_eq!(b.hooks[0].name, HookName::Op("==".into()));
    }

    #[test]
    fn eq_empty_struct_body_returns_true() {
        let st = make_struct("Unit", vec![]);
        let args = derive_args(&["Eq"]);
        let items = process_derive(&empty_file(), SourceAnnotationTarget::Struct(&st), &args);
        let Item::ImplBlock(b) = &items[0] else {
            panic!()
        };
        let body_stmt = &b.hooks[0].body.stmts[0];
        assert!(
            matches!(
                body_stmt,
                crate::parser::ast::Stmt::Return {
                    value: Some(crate::parser::ast::Expr::Bool(true, _)),
                    ..
                }
            ),
            "empty struct eq body should return true"
        );
    }
}
