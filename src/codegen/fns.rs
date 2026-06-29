use crate::codegen::types::type_expr_to_clif;
use crate::parser::ast::FnDef;
use cranelift_codegen::ir::AbiParam;
use cranelift_module::{FuncId, FuncOrDataId, Linkage, Module};
use cranelift_object::ObjectModule;

/// Register a function prototype in the module. Does not emit a body.
/// All top-level functions are exported so the linker can find `main`.
pub fn register_prototype(fn_def: &FnDef, module: &mut ObjectModule) -> FuncId {
    let mut sig = module.make_signature();

    for param in &fn_def.params {
        if let Some(ty) = type_expr_to_clif(&param.ty) {
            sig.params.push(AbiParam::new(ty));
        }
    }

    if let Some(ret_ty) = type_expr_to_clif(&fn_def.return_type) {
        sig.returns.push(AbiParam::new(ret_ty));
    }

    module
        .declare_function(&fn_def.name, Linkage::Export, &sig)
        .unwrap_or_else(|_| {
            // Duplicate or incompatible declaration: reuse the existing ID if available.
            match module.get_name(&fn_def.name) {
                Some(FuncOrDataId::Func(id)) => id,
                _ => panic!(
                    "internal compiler error: failed to declare function '{}' with incompatible signature",
                    fn_def.name
                ),
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::context::CodegenContext;
    use crate::diagnostics::Span;
    use crate::parser::ast::{Block, FnDef, Param, TypeExpr};

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

    #[test]
    fn function_prototype_is_registered() {
        let mut cgx = CodegenContext::new("test");
        let fn_def = FnDef {
            annotations: vec![],
            name: "add".into(),
            generic_params: vec![],
            params: vec![
                Param {
                    name: "a".into(),
                    ty: named_ty("int"),
                    mutable: false,
                    span: s(),
                },
                Param {
                    name: "b".into(),
                    ty: named_ty("int"),
                    mutable: false,
                    span: s(),
                },
            ],
            variadic: None,
            return_type: named_ty("int"),
            throws: false,
            body: Block {
                stmts: vec![],
                span: s(),
            },
            is_declaration: false,
            span: s(),
        };
        register_prototype(&fn_def, &mut cgx.module);
        assert!(cgx.module.get_name("add").is_some());
    }

    #[test]
    fn void_function_is_registered() {
        let mut cgx = CodegenContext::new("test");
        let fn_def = FnDef {
            annotations: vec![],
            name: "do_thing".into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: named_ty("void"),
            throws: false,
            body: Block {
                stmts: vec![],
                span: s(),
            },
            is_declaration: false,
            span: s(),
        };
        register_prototype(&fn_def, &mut cgx.module);
        assert!(cgx.module.get_name("do_thing").is_some());
    }

    #[test]
    fn duplicate_registration_is_idempotent() {
        let mut cgx = CodegenContext::new("test");
        let fn_def = FnDef {
            annotations: vec![],
            name: "foo".into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: named_ty("void"),
            throws: false,
            body: Block {
                stmts: vec![],
                span: s(),
            },
            is_declaration: false,
            span: s(),
        };
        let id1 = register_prototype(&fn_def, &mut cgx.module);
        let id2 = register_prototype(&fn_def, &mut cgx.module);
        assert_eq!(id1, id2);
    }
}
