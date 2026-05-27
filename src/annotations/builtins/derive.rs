use crate::annotations::api::{AnnotationArgs, AnnotationTarget};
use crate::annotations::ProcessorRegistry;
use crate::diagnostics::Span;
use crate::parser::ast::*;

pub fn register(registry: &mut ProcessorRegistry) {
    registry.register("derive", process_derive);
}

/// Generate impl blocks for each requested trait.
/// Returns multiple items per trait:
///   Eq         -> impl PartialEq (with == hook), impl Eq (marker)
///   Display    -> impl Display   (with to_str hook, no explicit params)
///   Comparable -> impl Ord       (with <=> hook returning Ordering), impl Comparable (marker)
pub fn process_derive(target: AnnotationTarget, args: AnnotationArgs) -> Vec<Item> {
    let mut items = Vec::new();
    match target {
        AnnotationTarget::Struct(st) => {
            for (trait_name, _) in args {
                match trait_name.as_str() {
                    "Eq" => items.extend(derive_eq(st)),
                    "Display" => items.push(derive_display(st)),
                    "Comparable" => items.extend(derive_comparable(st)),
                    _ => {}
                }
            }
        }
        AnnotationTarget::Enum(en) => {
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

fn s() -> Span {
    Span::new(0, 0)
}

fn named(n: &str) -> TypeExpr {
    TypeExpr::Named {
        name: n.into(),
        generics: vec![],
        bindings: vec![],
        span: s(),
    }
}

fn ordering() -> TypeExpr {
    named("Ordering")
}

fn other_param(st: &StructDef) -> Param {
    Param {
        name: "other".into(),
        ty: named(&st.name),
        mutable: false,
        span: s(),
    }
}

fn other_param_named(type_name: &str) -> Param {
    Param {
        name: "other".into(),
        ty: named(type_name),
        mutable: false,
        span: s(),
    }
}

fn field_access(obj: &str, field: &str) -> Expr {
    Expr::Field {
        object: Box::new(Expr::Ident(obj.into(), s())),
        field: field.into(),
        span: s(),
    }
}

fn ordering_equal() -> Expr {
    Expr::EnumAccess {
        enum_name: "Ordering".into(),
        variant: "Equal".into(),
        span: s(),
    }
}

fn plain_impl(interface: &str, for_type: &str, hooks: Vec<HookDef>) -> Item {
    Item::ImplBlock(ImplBlock {
        generic_params: vec![],
        interface: named(interface),
        for_type: named(for_type),
        self_alias: None,
        methods: vec![],
        hooks,
        assoc_bindings: vec![],
        kind: ImplKind::Plain,
        span: s(),
    })
}

fn marker_impl(interface: &str, for_type: &str) -> Item {
    plain_impl(interface, for_type, vec![])
}

// ---- Enum helpers -------------------------------------------------------------

/// Generates `match scrutinee { Var0 => 0, Var1 => 1, ... }`
fn enum_discriminant_match(scrutinee: Expr, variants: &[EnumVariant]) -> Expr {
    let arms = variants
        .iter()
        .enumerate()
        .map(|(i, v)| MatchArm {
            pattern: Pattern::Struct {
                variant: v.name.clone(),
                fields: vec![],
                span: s(),
            },
            guard: None,
            body: Expr::Int(i as i64, s()),
            span: s(),
        })
        .collect();
    Expr::Match {
        scrutinee: Box::new(scrutinee),
        arms,
        span: s(),
    }
}

// ---- Eq (enum) ----------------------------------------------------------------

fn derive_eq_enum(en: &EnumDef) -> Vec<Item> {
    let self_d = enum_discriminant_match(Expr::Ident("self".into(), s()), &en.variants);
    let other_d = enum_discriminant_match(Expr::Ident("other".into(), s()), &en.variants);
    let body = Block {
        stmts: vec![Stmt::Return {
            value: Some(Expr::BinOp {
                op: BinOp::Eq,
                left: Box::new(self_d),
                right: Box::new(other_d),
                span: s(),
            }),
            span: s(),
        }],
        span: s(),
    };
    let hook = HookDef {
        annotations: vec![],
        name: HookName::Op("==".into()),
        params: vec![other_param_named(&en.name)],
        return_type: Some(named("bool")),
        body,
        span: s(),
    };
    vec![
        plain_impl("PartialEq", &en.name, vec![hook]),
        marker_impl("Eq", &en.name),
    ]
}

// ---- Comparable (enum) --------------------------------------------------------

fn derive_comparable_enum(en: &EnumDef) -> Vec<Item> {
    let self_d = enum_discriminant_match(Expr::Ident("self".into(), s()), &en.variants);
    let other_d = enum_discriminant_match(Expr::Ident("other".into(), s()), &en.variants);
    let stmts = vec![
        Stmt::VarDecl {
            name: "self_d".into(),
            ty: named("int"),
            value: self_d,
            mutable: false,
            span: s(),
        },
        Stmt::VarDecl {
            name: "other_d".into(),
            ty: named("int"),
            value: other_d,
            mutable: false,
            span: s(),
        },
        Stmt::If {
            branches: vec![(
                Expr::BinOp {
                    op: BinOp::Lt,
                    left: Box::new(Expr::Ident("self_d".into(), s())),
                    right: Box::new(Expr::Ident("other_d".into(), s())),
                    span: s(),
                },
                Block {
                    stmts: vec![Stmt::Return {
                        value: Some(Expr::EnumAccess {
                            enum_name: "Ordering".into(),
                            variant: "Less".into(),
                            span: s(),
                        }),
                        span: s(),
                    }],
                    span: s(),
                },
            )],
            else_branch: None,
            span: s(),
        },
        Stmt::If {
            branches: vec![(
                Expr::BinOp {
                    op: BinOp::Lt,
                    left: Box::new(Expr::Ident("other_d".into(), s())),
                    right: Box::new(Expr::Ident("self_d".into(), s())),
                    span: s(),
                },
                Block {
                    stmts: vec![Stmt::Return {
                        value: Some(Expr::EnumAccess {
                            enum_name: "Ordering".into(),
                            variant: "Greater".into(),
                            span: s(),
                        }),
                        span: s(),
                    }],
                    span: s(),
                },
            )],
            else_branch: None,
            span: s(),
        },
        Stmt::Return {
            value: Some(ordering_equal()),
            span: s(),
        },
    ];
    let hook = HookDef {
        annotations: vec![],
        name: HookName::Op("<=>".into()),
        params: vec![other_param_named(&en.name)],
        return_type: Some(ordering()),
        body: Block { stmts, span: s() },
        span: s(),
    };
    vec![
        plain_impl("Ord", &en.name, vec![hook]),
        partial_ord_impl_from_ord(&en.name, other_param_named(&en.name)),
        marker_impl("Comparable", &en.name),
    ]
}

// ---- Eq -----------------------------------------------------------------------

/// Generates `impl PartialEq for T` and `impl Eq for T` (marker).
fn derive_eq(st: &StructDef) -> Vec<Item> {
    let hook = HookDef {
        annotations: vec![],
        name: HookName::Op("==".into()),
        params: vec![other_param(st)],
        return_type: Some(named("bool")),
        body: eq_body(st),
        span: s(),
    };
    vec![
        plain_impl("PartialEq", &st.name, vec![hook]),
        marker_impl("Eq", &st.name),
    ]
}

fn eq_body(st: &StructDef) -> Block {
    let expr = if st.fields.is_empty() {
        Expr::Bool(true, s())
    } else {
        let mut iter = st.fields.iter();
        let first = iter.next().unwrap();
        let init = field_eq(first);
        iter.fold(init, |acc, f| Expr::BinOp {
            op: BinOp::And,
            left: Box::new(acc),
            right: Box::new(field_eq(f)),
            span: s(),
        })
    };
    Block {
        stmts: vec![Stmt::Return {
            value: Some(expr),
            span: s(),
        }],
        span: s(),
    }
}

fn field_eq(f: &Field) -> Expr {
    Expr::BinOp {
        op: BinOp::Eq,
        left: Box::new(field_access("self", &f.name)),
        right: Box::new(field_access("other", &f.name)),
        span: s(),
    }
}

// ---- Display ------------------------------------------------------------------

/// Generates `impl Display for T` with a `to_str()` hook (no explicit params).
/// The hook body returns an interpolated string like `"T { f1: {self.f1}, ... }"`.
fn derive_display(st: &StructDef) -> Item {
    let body = display_body(st);
    let hook = HookDef {
        annotations: vec![],
        name: HookName::Named("to_str".into()),
        params: vec![], // self is implicit for hooks
        return_type: Some(named("str")),
        body,
        span: s(),
    };
    plain_impl("Display", &st.name, vec![hook])
}

fn display_body(st: &StructDef) -> Block {
    let mut segs = vec![StringSegment::Text(format!("{} {{ ", st.name))];
    for (i, f) in st.fields.iter().enumerate() {
        if i > 0 {
            segs.push(StringSegment::Text(", ".into()));
        }
        segs.push(StringSegment::Text(format!("{}: ", f.name)));
        segs.push(StringSegment::Interp(field_access("self", &f.name)));
    }
    segs.push(StringSegment::Text(" }".into()));
    let expr = Expr::Str(segs, s());
    Block {
        stmts: vec![Stmt::Return {
            value: Some(expr),
            span: s(),
        }],
        span: s(),
    }
}

// ---- Comparable ---------------------------------------------------------------

/// `impl PartialOrd for T` derived from `Ord`: `hook <` returns `(self <=> other) == Ordering:Less`.
fn partial_ord_impl_from_ord(type_name: &str, other_param: Param) -> Item {
    let lt_body = Block {
        stmts: vec![Stmt::Return {
            value: Some(Expr::BinOp {
                op: BinOp::Eq,
                left: Box::new(Expr::BinOp {
                    op: BinOp::Spaceship,
                    left: Box::new(Expr::Ident("self".into(), s())),
                    right: Box::new(Expr::Ident("other".into(), s())),
                    span: s(),
                }),
                right: Box::new(Expr::EnumAccess {
                    enum_name: "Ordering".into(),
                    variant: "Less".into(),
                    span: s(),
                }),
                span: s(),
            }),
            span: s(),
        }],
        span: s(),
    };
    let hook = HookDef {
        annotations: vec![],
        name: HookName::Op("<".into()),
        params: vec![other_param],
        return_type: Some(named("bool")),
        body: lt_body,
        span: s(),
    };
    plain_impl("PartialOrd", type_name, vec![hook])
}

/// Generates `impl Ord`, `impl PartialOrd`, and `impl Comparable` for T.
/// Lexicographic comparison: compare field-by-field, returning the first
/// non-Equal result; return `Ordering:Equal` if all fields match.
fn derive_comparable(st: &StructDef) -> Vec<Item> {
    let hook = HookDef {
        annotations: vec![],
        name: HookName::Op("<=>".into()),
        params: vec![other_param(st)],
        return_type: Some(ordering()),
        body: comparable_body(st),
        span: s(),
    };
    vec![
        plain_impl("Ord", &st.name, vec![hook]),
        partial_ord_impl_from_ord(&st.name, other_param(st)),
        marker_impl("Comparable", &st.name),
    ]
}

fn comparable_body(st: &StructDef) -> Block {
    if st.fields.is_empty() {
        return Block {
            stmts: vec![Stmt::Return {
                value: Some(ordering_equal()),
                span: s(),
            }],
            span: s(),
        };
    }
    // For each field, in order:
    //   if self.f < other.f { return Ordering:Less }
    //   if other.f < self.f { return Ordering:Greater }
    // return Ordering:Equal
    //
    // Uses only < (PartialOrd) rather than <=> (Ordering type mismatch with -1/0/1).
    let mut stmts: Vec<Stmt> = vec![];
    for f in &st.fields {
        let sf = field_access("self", &f.name);
        let of = field_access("other", &f.name);
        stmts.push(Stmt::If {
            branches: vec![(
                Expr::BinOp {
                    op: BinOp::Lt,
                    left: Box::new(sf.clone()),
                    right: Box::new(of.clone()),
                    span: s(),
                },
                Block {
                    stmts: vec![Stmt::Return {
                        value: Some(Expr::EnumAccess {
                            enum_name: "Ordering".into(),
                            variant: "Less".into(),
                            span: s(),
                        }),
                        span: s(),
                    }],
                    span: s(),
                },
            )],
            else_branch: None,
            span: s(),
        });
        stmts.push(Stmt::If {
            branches: vec![(
                Expr::BinOp {
                    op: BinOp::Lt,
                    left: Box::new(of),
                    right: Box::new(sf),
                    span: s(),
                },
                Block {
                    stmts: vec![Stmt::Return {
                        value: Some(Expr::EnumAccess {
                            enum_name: "Ordering".into(),
                            variant: "Greater".into(),
                            span: s(),
                        }),
                        span: s(),
                    }],
                    span: s(),
                },
            )],
            else_branch: None,
            span: s(),
        });
    }
    stmts.push(Stmt::Return {
        value: Some(ordering_equal()),
        span: s(),
    });
    Block { stmts, span: s() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::api::AnnotationTarget;
    use crate::diagnostics::Span;

    fn s() -> Span {
        Span::new(0, 0)
    }

    fn named(n: &str) -> TypeExpr {
        TypeExpr::Named {
            name: n.into(),
            generics: vec![],
            bindings: vec![],
            span: s(),
        }
    }

    fn point_struct() -> StructDef {
        StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "Point".into(),
            generic_params: vec![],
            interfaces: vec![],
            fields: vec![
                Field {
                    annotations: vec![],
                    is_priv: false,
                    name: "x".into(),
                    ty: named("int"),
                    default: None,
                    span: s(),
                },
                Field {
                    annotations: vec![],
                    is_priv: false,
                    name: "y".into(),
                    ty: named("int"),
                    default: None,
                    span: s(),
                },
            ],
            methods: vec![],
            decls: vec![],
            inline_hooks: vec![],
            span: s(),
        }
    }

    fn derive_args(traits: &[&str]) -> Vec<(String, Expr)> {
        traits
            .iter()
            .map(|t| (t.to_string(), Expr::Ident(t.to_string(), s())))
            .collect()
    }

    fn first_impl(result: &[Item]) -> &ImplBlock {
        match &result[0] {
            Item::ImplBlock(ib) => ib,
            _ => panic!("expected ImplBlock"),
        }
    }

    // ---- Eq -------------------------------------------------------------------

    #[test]
    fn derive_eq_generates_two_impl_blocks() {
        let st = point_struct();
        let args = derive_args(&["Eq"]);
        let result = process_derive(AnnotationTarget::Struct(&st), &args);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|i| matches!(i, Item::ImplBlock(_))));
    }

    #[test]
    fn derive_eq_first_impl_is_partial_eq() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Eq"]));
        let ib = first_impl(&result);
        assert!(matches!(&ib.interface, TypeExpr::Named { name, .. } if name == "PartialEq"));
    }

    #[test]
    fn derive_eq_second_impl_is_eq_marker() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Eq"]));
        let Item::ImplBlock(ib) = &result[1] else {
            panic!()
        };
        assert!(matches!(&ib.interface, TypeExpr::Named { name, .. } if name == "Eq"));
        assert!(ib.hooks.is_empty());
    }

    #[test]
    fn derive_eq_impl_targets_correct_type() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Eq"]));
        let ib = first_impl(&result);
        assert!(matches!(&ib.for_type, TypeExpr::Named { name, .. } if name == "Point"));
    }

    #[test]
    fn derive_eq_hook_is_eq_op() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Eq"]));
        let ib = first_impl(&result);
        assert_eq!(ib.hooks.len(), 1);
        assert!(matches!(&ib.hooks[0].name, HookName::Op(op) if op == "=="));
    }

    #[test]
    fn derive_eq_hook_has_one_param_other() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Eq"]));
        let ib = first_impl(&result);
        assert_eq!(ib.hooks[0].params.len(), 1);
        assert_eq!(ib.hooks[0].params[0].name, "other");
    }

    #[test]
    fn derive_eq_on_struct_with_no_fields_generates_two_impls() {
        let st = StructDef {
            annotations: vec![],
            is_builtin: false,
            name: "Unit".into(),
            generic_params: vec![],
            interfaces: vec![],
            fields: vec![],
            methods: vec![],
            decls: vec![],
            inline_hooks: vec![],
            span: s(),
        };
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Eq"]));
        assert_eq!(result.len(), 2);
    }

    // ---- Display --------------------------------------------------------------

    #[test]
    fn derive_display_generates_one_impl_block() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Display"]));
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], Item::ImplBlock(_)));
    }

    #[test]
    fn derive_display_hook_is_to_str() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Display"]));
        let ib = first_impl(&result);
        assert_eq!(ib.hooks.len(), 1);
        assert!(matches!(&ib.hooks[0].name, HookName::Named(n) if n == "to_str"));
    }

    #[test]
    fn derive_display_hook_has_no_params() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Display"]));
        let ib = first_impl(&result);
        assert!(ib.hooks[0].params.is_empty());
    }

    #[test]
    fn derive_display_impl_is_display() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Display"]));
        let ib = first_impl(&result);
        assert!(matches!(&ib.interface, TypeExpr::Named { name, .. } if name == "Display"));
    }

    // ---- Comparable -----------------------------------------------------------

    #[test]
    fn derive_comparable_generates_three_impl_blocks() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Comparable"]));
        assert_eq!(result.len(), 3);
        assert!(result.iter().all(|i| matches!(i, Item::ImplBlock(_))));
    }

    #[test]
    fn derive_comparable_first_impl_is_ord() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Comparable"]));
        let ib = first_impl(&result);
        assert!(matches!(&ib.interface, TypeExpr::Named { name, .. } if name == "Ord"));
    }

    #[test]
    fn derive_comparable_second_impl_is_partial_ord() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Comparable"]));
        let Item::ImplBlock(ib) = &result[1] else {
            panic!()
        };
        assert!(matches!(&ib.interface, TypeExpr::Named { name, .. } if name == "PartialOrd"));
        assert_eq!(ib.hooks.len(), 1);
        assert!(matches!(&ib.hooks[0].name, HookName::Op(op) if op == "<"));
    }

    #[test]
    fn derive_comparable_third_impl_is_comparable_marker() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Comparable"]));
        let Item::ImplBlock(ib) = &result[2] else {
            panic!()
        };
        assert!(matches!(&ib.interface, TypeExpr::Named { name, .. } if name == "Comparable"));
        assert!(ib.hooks.is_empty());
    }

    #[test]
    fn derive_comparable_hook_is_spaceship() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Comparable"]));
        let ib = first_impl(&result);
        assert_eq!(ib.hooks.len(), 1);
        assert!(matches!(&ib.hooks[0].name, HookName::Op(op) if op == "<=>"));
    }

    #[test]
    fn derive_comparable_hook_returns_ordering() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Comparable"]));
        let ib = first_impl(&result);
        assert!(matches!(
            &ib.hooks[0].return_type,
            Some(TypeExpr::Named { name, .. }) if name == "Ordering"
        ));
    }

    #[test]
    fn derive_comparable_hook_has_one_param_other() {
        let st = point_struct();
        let result = process_derive(AnnotationTarget::Struct(&st), &derive_args(&["Comparable"]));
        let ib = first_impl(&result);
        assert_eq!(ib.hooks[0].params.len(), 1);
        assert_eq!(ib.hooks[0].params[0].name, "other");
    }

    // ---- Combined -------------------------------------------------------------

    #[test]
    fn derive_all_three_generates_six_impl_blocks() {
        let st = point_struct();
        let args = derive_args(&["Eq", "Display", "Comparable"]);
        let result = process_derive(AnnotationTarget::Struct(&st), &args);
        // Eq->2, Display->1, Comparable->3
        assert_eq!(result.len(), 6);
        assert!(result.iter().all(|i| matches!(i, Item::ImplBlock(_))));
    }

    #[test]
    fn derive_on_function_produces_nothing() {
        let fn_def = FnDef {
            annotations: vec![],
            name: "foo".into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: named("void"),
            body: Block {
                stmts: vec![],
                span: s(),
            },
            is_declaration: false,
            span: s(),
        };
        let args = derive_args(&["Eq"]);
        let result = process_derive(AnnotationTarget::Function(&fn_def), &args);
        assert!(result.is_empty());
    }

    #[test]
    fn derive_unknown_trait_produces_nothing() {
        let st = point_struct();
        let args = derive_args(&["UnknownTrait"]);
        let result = process_derive(AnnotationTarget::Struct(&st), &args);
        assert!(result.is_empty());
    }

    // ---- Enum derive -----------------------------------------------------------

    fn priority_enum() -> EnumDef {
        let variants = ["Low", "Medium", "High", "Critical"]
            .iter()
            .map(|name| EnumVariant {
                name: name.to_string(),
                fields: vec![],
                discriminant: None,
                span: s(),
            })
            .collect();
        EnumDef {
            annotations: vec![],
            name: "Priority".into(),
            generic_params: vec![],
            interfaces: vec![],
            variants,
            methods: vec![],
            span: s(),
        }
    }

    #[test]
    fn derive_eq_on_enum_generates_two_impl_blocks() {
        let en = priority_enum();
        let args = derive_args(&["Eq"]);
        let result = process_derive(AnnotationTarget::Enum(&en), &args);
        assert_eq!(
            result.len(),
            2,
            "expected PartialEq + Eq marker: {result:?}"
        );
        assert!(result.iter().all(|i| matches!(i, Item::ImplBlock(_))));
    }

    #[test]
    fn derive_eq_on_enum_first_impl_is_partial_eq_for_enum() {
        let en = priority_enum();
        let result = process_derive(AnnotationTarget::Enum(&en), &derive_args(&["Eq"]));
        let ib = first_impl(&result);
        assert!(matches!(&ib.interface, TypeExpr::Named { name, .. } if name == "PartialEq"));
        assert!(matches!(&ib.for_type, TypeExpr::Named { name, .. } if name == "Priority"));
    }

    #[test]
    fn derive_comparable_on_enum_generates_three_impl_blocks() {
        let en = priority_enum();
        let args = derive_args(&["Comparable"]);
        let result = process_derive(AnnotationTarget::Enum(&en), &args);
        assert_eq!(
            result.len(),
            3,
            "expected Ord + PartialOrd + Comparable marker: {result:?}"
        );
        assert!(result.iter().all(|i| matches!(i, Item::ImplBlock(_))));
    }

    #[test]
    fn derive_comparable_on_enum_first_impl_is_ord() {
        let en = priority_enum();
        let result = process_derive(AnnotationTarget::Enum(&en), &derive_args(&["Comparable"]));
        let ib = first_impl(&result);
        assert!(matches!(&ib.interface, TypeExpr::Named { name, .. } if name == "Ord"));
        assert!(matches!(&ib.for_type, TypeExpr::Named { name, .. } if name == "Priority"));
    }

    #[test]
    fn derive_comparable_on_enum_hook_is_spaceship() {
        let en = priority_enum();
        let result = process_derive(AnnotationTarget::Enum(&en), &derive_args(&["Comparable"]));
        let ib = first_impl(&result);
        assert_eq!(ib.hooks.len(), 1);
        assert!(matches!(&ib.hooks[0].name, HookName::Op(op) if op == "<=>"));
    }

    #[test]
    fn derive_comparable_on_enum_hook_returns_ordering() {
        let en = priority_enum();
        let result = process_derive(AnnotationTarget::Enum(&en), &derive_args(&["Comparable"]));
        let ib = first_impl(&result);
        assert!(matches!(
            &ib.hooks[0].return_type,
            Some(TypeExpr::Named { name, .. }) if name == "Ordering"
        ));
    }
}
