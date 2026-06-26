use cranelift_codegen::ir::{types, AbiParam};
use cranelift_module::{FuncId, FuncOrDataId, Linkage, Module};
use cranelift_object::ObjectModule;
use std::collections::HashMap;

pub fn declare_math_runtime(module: &mut ObjectModule) -> HashMap<String, FuncId> {
    let mut ids: HashMap<String, FuncId> = HashMap::new();

    for name in &[
        "__kiln_math_floor",
        "__kiln_math_ceil",
        "__kiln_math_round",
        "__kiln_math_trunc",
        "__kiln_math_fract",
        "__kiln_math_sqrt",
        "__kiln_math_cbrt",
        "__kiln_math_exp",
        "__kiln_math_exp2",
        "__kiln_math_ln",
        "__kiln_math_log2",
        "__kiln_math_log10",
        "__kiln_math_sin",
        "__kiln_math_cos",
        "__kiln_math_tan",
        "__kiln_math_asin",
        "__kiln_math_acos",
        "__kiln_math_atan",
        "__kiln_math_sinh",
        "__kiln_math_cosh",
        "__kiln_math_tanh",
        "__kiln_math_asinh",
        "__kiln_math_acosh",
        "__kiln_math_atanh",
        "__kiln_math_to_degrees",
        "__kiln_math_to_radians",
        "__kiln_math_fabs",
    ] {
        let id = f64_to_f64(module, name);
        ids.insert((*name).into(), id);
    }

    for name in &[
        "__kiln_math_pow",
        "__kiln_math_hypot",
        "__kiln_math_fmin",
        "__kiln_math_fmax",
    ] {
        let id = ff64_to_f64(module, name);
        ids.insert((*name).into(), id);
    }

    for name in &["__kiln_math_log", "__kiln_math_atan2"] {
        let id = ff64_to_f64(module, name);
        ids.insert((*name).into(), id);
    }

    {
        let id = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::F64));
            sig.params.push(AbiParam::new(types::F64));
            sig.params.push(AbiParam::new(types::F64));
            sig.returns.push(AbiParam::new(types::F64));
            import_fn(module, "__kiln_math_fclamp", sig)
        };
        ids.insert("__kiln_math_fclamp".into(), id);
    }

    {
        let id = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::F64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::F64));
            import_fn(module, "__kiln_math_powi", sig)
        };
        ids.insert("__kiln_math_powi".into(), id);
    }

    for name in &[
        "__kiln_math_is_nan",
        "__kiln_math_is_infinite",
        "__kiln_math_is_finite",
    ] {
        let id = f64_to_i64(module, name);
        ids.insert((*name).into(), id);
    }

    {
        let id = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::F64));
            sig.params.push(AbiParam::new(types::F64));
            sig.params.push(AbiParam::new(types::F64));
            sig.params.push(AbiParam::new(types::F64));
            sig.returns.push(AbiParam::new(types::I64));
            import_fn(module, "__kiln_math_isclose", sig)
        };
        ids.insert("__kiln_math_isclose".into(), id);
    }

    for name in &[
        "__kiln_math_ipow",
        "__kiln_math_gcd",
        "__kiln_math_lcm",
        "__kiln_math_imin",
        "__kiln_math_imax",
    ] {
        let id = ii64_to_i64(module, name);
        ids.insert((*name).into(), id);
    }

    {
        let id = {
            let mut sig = module.make_signature();
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.params.push(AbiParam::new(types::I64));
            sig.returns.push(AbiParam::new(types::I64));
            import_fn(module, "__kiln_math_iclamp", sig)
        };
        ids.insert("__kiln_math_iclamp".into(), id);
    }

    {
        let id = i64_to_i64(module, "__kiln_math_factorial");
        ids.insert("__kiln_math_factorial".into(), id);
    }

    for name in &["__kiln_math_comb", "__kiln_math_perm"] {
        let id = ii64_to_i64(module, name);
        ids.insert((*name).into(), id);
    }

    ids
}

fn f64_to_f64(module: &mut ObjectModule, name: &str) -> FuncId {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::F64));
    sig.returns.push(AbiParam::new(types::F64));
    import_fn(module, name, sig)
}

fn ff64_to_f64(module: &mut ObjectModule, name: &str) -> FuncId {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::F64));
    sig.params.push(AbiParam::new(types::F64));
    sig.returns.push(AbiParam::new(types::F64));
    import_fn(module, name, sig)
}

fn f64_to_i64(module: &mut ObjectModule, name: &str) -> FuncId {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::F64));
    sig.returns.push(AbiParam::new(types::I64));
    import_fn(module, name, sig)
}

fn i64_to_i64(module: &mut ObjectModule, name: &str) -> FuncId {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I64));
    import_fn(module, name, sig)
}

fn ii64_to_i64(module: &mut ObjectModule, name: &str) -> FuncId {
    let mut sig = module.make_signature();
    sig.params.push(AbiParam::new(types::I64));
    sig.params.push(AbiParam::new(types::I64));
    sig.returns.push(AbiParam::new(types::I64));
    import_fn(module, name, sig)
}

fn import_fn(
    module: &mut ObjectModule,
    name: &str,
    sig: cranelift_codegen::ir::Signature,
) -> FuncId {
    module
        .declare_function(name, Linkage::Import, &sig)
        .unwrap_or_else(|_| match module.get_name(name) {
            Some(FuncOrDataId::Func(id)) => id,
            _ => panic!(
                "internal compiler error: failed to declare math runtime function '{}'",
                name
            ),
        })
}
