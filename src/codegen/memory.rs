use cranelift_codegen::ir::{types, AbiParam, InstBuilder, MemFlags, Value};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{Linkage, Module};
use cranelift_object::ObjectModule;

/// Declare `malloc(size: i64) -> i64` and `free(ptr: i64)` as external symbols.
pub fn declare_alloc_fns(module: &mut ObjectModule) {
    let mut malloc_sig = module.make_signature();
    malloc_sig.params.push(AbiParam::new(types::I64));
    malloc_sig.returns.push(AbiParam::new(types::I64));
    module.declare_function("malloc", Linkage::Import, &malloc_sig).ok();

    let mut free_sig = module.make_signature();
    free_sig.params.push(AbiParam::new(types::I64));
    module.declare_function("free", Linkage::Import, &free_sig).ok();

    let mut rc_inc_sig = module.make_signature();
    rc_inc_sig.params.push(AbiParam::new(types::I64));
    module.declare_function("__kiln_rc_inc", Linkage::Import, &rc_inc_sig).ok();

    let mut rc_dec_sig = module.make_signature();
    rc_dec_sig.params.push(AbiParam::new(types::I64));
    module.declare_function("__kiln_rc_dec", Linkage::Import, &rc_dec_sig).ok();
}

/// Emit a `malloc(byte_size)` call and return the heap pointer as I64.
pub fn emit_malloc(
    byte_size: u32,
    module: &mut ObjectModule,
    builder: &mut FunctionBuilder,
) -> Value {
    let callee = module.declare_function(
        "malloc",
        Linkage::Import,
        &{
            let mut s = module.make_signature();
            s.params.push(AbiParam::new(types::I64));
            s.returns.push(AbiParam::new(types::I64));
            s
        },
    ).unwrap();
    let func_ref = module.declare_func_in_func(callee, builder.func);
    let size_val = builder.ins().iconst(types::I64, byte_size as i64);
    let call = builder.ins().call(func_ref, &[size_val]);
    builder.inst_results(call)[0]
}

/// Emit a `free(ptr)` call.
pub fn emit_free(ptr: Value, module: &mut ObjectModule, builder: &mut FunctionBuilder) {
    let callee = module.declare_function(
        "free",
        Linkage::Import,
        &{
            let mut s = module.make_signature();
            s.params.push(AbiParam::new(types::I64));
            s
        },
    ).unwrap();
    let func_ref = module.declare_func_in_func(callee, builder.func);
    builder.ins().call(func_ref, &[ptr]);
}

/// Load an I64 value from `base_ptr + byte_offset`.
pub fn load_field(base_ptr: Value, byte_offset: u32, builder: &mut FunctionBuilder) -> Value {
    let offset = byte_offset as i32;
    builder.ins().load(types::I64, MemFlags::new(), base_ptr, offset)
}

/// Store an I64 value to `base_ptr + byte_offset`.
pub fn store_field(val: Value, base_ptr: Value, byte_offset: u32, builder: &mut FunctionBuilder) {
    let offset = byte_offset as i32;
    builder.ins().store(MemFlags::new(), val, base_ptr, offset);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::context::CodegenContext;
    use cranelift_codegen::ir::{types, AbiParam};
    use cranelift_frontend::FunctionBuilderContext;
    use cranelift_module::Module;

    #[test]
    fn alloc_fns_declared() {
        let mut cgx = CodegenContext::new("test");
        declare_alloc_fns(&mut cgx.module);
        assert!(cgx.module.get_name("malloc").is_some());
        assert!(cgx.module.get_name("free").is_some());
        assert!(cgx.module.get_name("__kiln_rc_inc").is_some());
        assert!(cgx.module.get_name("__kiln_rc_dec").is_some());
    }

    #[test]
    fn malloc_emit_compiles() {
        let mut cgx = CodegenContext::new("test");
        declare_alloc_fns(&mut cgx.module);
        let mut fbc = FunctionBuilderContext::new();
        cgx.ctx.func.signature.returns.push(AbiParam::new(types::I64));

        let mut builder = FunctionBuilder::new(&mut cgx.ctx.func, &mut fbc);
        let block = builder.create_block();
        builder.switch_to_block(block);
        builder.seal_block(block);

        let ptr = emit_malloc(64, &mut cgx.module, &mut builder);
        assert_eq!(builder.func.dfg.value_type(ptr), types::I64);
        builder.ins().return_(&[ptr]);
        builder.finalize();

        let flags = cranelift_codegen::settings::Flags::new(cranelift_codegen::settings::builder());
        assert!(
            cranelift_codegen::verify_function(&cgx.ctx.func, &flags).is_ok(),
            "{}",
            cranelift_codegen::verify_function(&cgx.ctx.func, &flags).unwrap_err()
        );
    }

    #[test]
    fn load_store_field_compiles() {
        let mut cgx = CodegenContext::new("test");
        let mut fbc = FunctionBuilderContext::new();
        cgx.ctx.func.signature.params.push(AbiParam::new(types::I64));
        cgx.ctx.func.signature.returns.push(AbiParam::new(types::I64));

        let mut builder = FunctionBuilder::new(&mut cgx.ctx.func, &mut fbc);
        let block = builder.create_block();
        builder.append_block_params_for_function_params(block);
        builder.switch_to_block(block);
        builder.seal_block(block);

        let base = builder.block_params(block)[0];
        let val = builder.ins().iconst(types::I64, 42);
        store_field(val, base, 0, &mut builder);
        let loaded = load_field(base, 0, &mut builder);
        builder.ins().return_(&[loaded]);
        builder.finalize();

        let flags = cranelift_codegen::settings::Flags::new(cranelift_codegen::settings::builder());
        assert!(cranelift_codegen::verify_function(&cgx.ctx.func, &flags).is_ok());
    }
}
