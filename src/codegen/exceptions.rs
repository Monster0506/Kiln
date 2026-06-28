use cranelift_codegen::ir::{types, AbiParam, InstBuilder};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{Linkage, Module};
use cranelift_object::ObjectModule;

/// Declare the Kiln exception runtime as external imports: setjmp/longjmp wrappers,
/// exc_push/pop frame management, raise, and current_exc.
pub fn declare_exception_runtime(module: &mut ObjectModule) {
    let mut setjmp_sig = module.make_signature();
    setjmp_sig.params.push(AbiParam::new(types::I64));
    setjmp_sig.returns.push(AbiParam::new(types::I32));
    module
        .declare_function("__kiln_setjmp", Linkage::Import, &setjmp_sig)
        .ok();

    let mut longjmp_sig = module.make_signature();
    longjmp_sig.params.push(AbiParam::new(types::I64));
    longjmp_sig.params.push(AbiParam::new(types::I32));
    module
        .declare_function("__kiln_longjmp", Linkage::Import, &longjmp_sig)
        .ok();

    let mut push_sig = module.make_signature();
    push_sig.params.push(AbiParam::new(types::I64));
    module
        .declare_function("__kiln_exc_push", Linkage::Import, &push_sig)
        .ok();

    let pop_sig = module.make_signature();
    module
        .declare_function("__kiln_exc_pop", Linkage::Import, &pop_sig)
        .ok();

    let mut raise_sig = module.make_signature();
    raise_sig.params.push(AbiParam::new(types::I64));
    module
        .declare_function("__kiln_raise", Linkage::Import, &raise_sig)
        .ok();

    let mut cur_exc_sig = module.make_signature();
    cur_exc_sig.returns.push(AbiParam::new(types::I64));
    module
        .declare_function("__kiln_current_exc", Linkage::Import, &cur_exc_sig)
        .ok();
}

/// Emit IR for `raise expr`: calls `__kiln_raise(exc_ptr)`.
/// The block is diverging after this; emit no further instructions.
pub fn emit_raise(
    exc_ptr: cranelift_codegen::ir::Value,
    module: &mut ObjectModule,
    builder: &mut FunctionBuilder,
) {
    let sig = {
        let mut s = module.make_signature();
        s.params.push(AbiParam::new(types::I64));
        s
    };
    let callee = module
        .declare_function("__kiln_raise", Linkage::Import, &sig)
        .unwrap();
    let func_ref = module.declare_func_in_func(callee, builder.func);
    builder.ins().call(func_ref, &[exc_ptr]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::context::CodegenContext;
    use cranelift_module::Module;

    #[test]
    fn exception_runtime_fns_declared() {
        let mut cgx = CodegenContext::new("test");
        declare_exception_runtime(&mut cgx.module);
        assert!(cgx.module.get_name("__kiln_setjmp").is_some());
        assert!(cgx.module.get_name("__kiln_exc_push").is_some());
        assert!(cgx.module.get_name("__kiln_exc_pop").is_some());
        assert!(cgx.module.get_name("__kiln_raise").is_some());
        assert!(cgx.module.get_name("__kiln_current_exc").is_some());
    }
}
