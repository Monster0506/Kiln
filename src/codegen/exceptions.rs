use cranelift_codegen::ir::{types, AbiParam, InstBuilder};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{Linkage, Module};
use cranelift_object::ObjectModule;

/// Declare the Kiln exception runtime as external imports.
///
/// - `__kiln_raise(exc_ptr: i64)` — stores exc_ptr and longjmps
/// - `__kiln_try_enter() -> i32`  — calls setjmp; returns 0 on entry, non-zero on resume
/// - `__kiln_try_exit()`          — pops the exception frame
/// - `__kiln_current_exc() -> i64` — returns the current exception pointer
pub fn declare_exception_runtime(module: &mut ObjectModule) {
    let mut raise_sig = module.make_signature();
    raise_sig.params.push(AbiParam::new(types::I64));
    module
        .declare_function("__kiln_raise", Linkage::Import, &raise_sig)
        .ok();

    let mut enter_sig = module.make_signature();
    enter_sig.returns.push(AbiParam::new(types::I32));
    module
        .declare_function("__kiln_try_enter", Linkage::Import, &enter_sig)
        .ok();

    let exit_sig = module.make_signature();
    module
        .declare_function("__kiln_try_exit", Linkage::Import, &exit_sig)
        .ok();

    let mut cur_exc_sig = module.make_signature();
    cur_exc_sig.returns.push(AbiParam::new(types::I64));
    module
        .declare_function("__kiln_current_exc", Linkage::Import, &cur_exc_sig)
        .ok();
}

/// Emit IR for a `raise expr` statement.
///
/// Calls `__kiln_raise(exc_ptr)`. After the call the block is considered
/// diverging (no further instructions should be emitted in the same block).
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

/// Emit IR for a `try` block with a single catch-all handler.
///
/// Structure emitted (pseudo-IR):
///
///   frame = call __kiln_try_enter()
///   brif frame != 0, exc_bb, try_body_bb
/// try_body:
///   <try stmts>  call __kiln_try_exit()  jump merge_bb
/// exc_bb:
///   <handler stmts>  jump merge_bb
/// merge_bb:
pub fn emit_try_catch(
    try_body: impl FnOnce(&mut FunctionBuilder),
    handler: impl FnOnce(&mut FunctionBuilder),
    module: &mut ObjectModule,
    builder: &mut FunctionBuilder,
) {
    let enter_sig = {
        let mut s = module.make_signature();
        s.returns.push(AbiParam::new(types::I32));
        s
    };
    let exit_sig = module.make_signature();

    let enter_id = module
        .declare_function("__kiln_try_enter", Linkage::Import, &enter_sig)
        .unwrap();
    let exit_id = module
        .declare_function("__kiln_try_exit", Linkage::Import, &exit_sig)
        .unwrap();

    let enter_ref = module.declare_func_in_func(enter_id, builder.func);
    let exit_ref = module.declare_func_in_func(exit_id, builder.func);

    let try_body_bb = builder.create_block();
    let exc_bb = builder.create_block();
    let merge_bb = builder.create_block();

    let call = builder.ins().call(enter_ref, &[]);
    let frame = builder.inst_results(call)[0];
    let zero = builder.ins().iconst(types::I32, 0);
    let is_exc = builder.ins().icmp(
        cranelift_codegen::ir::condcodes::IntCC::NotEqual,
        frame,
        zero,
    );
    builder.ins().brif(is_exc, exc_bb, &[], try_body_bb, &[]);

    builder.switch_to_block(try_body_bb);
    builder.seal_block(try_body_bb);
    try_body(builder);
    if !builder.is_unreachable() {
        builder.ins().call(exit_ref, &[]);
        builder.ins().jump(merge_bb, &[]);
    }

    builder.switch_to_block(exc_bb);
    builder.seal_block(exc_bb);
    handler(builder);
    if !builder.is_unreachable() {
        builder.ins().jump(merge_bb, &[]);
    }

    builder.switch_to_block(merge_bb);
    builder.seal_block(merge_bb);
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
        assert!(cgx.module.get_name("__kiln_raise").is_some());
        assert!(cgx.module.get_name("__kiln_try_enter").is_some());
        assert!(cgx.module.get_name("__kiln_try_exit").is_some());
        assert!(cgx.module.get_name("__kiln_current_exc").is_some());
    }

    #[test]
    fn try_catch_compiles() {
        use cranelift_codegen::ir::{types, AbiParam, InstBuilder};
        use cranelift_frontend::FunctionBuilderContext;

        let mut cgx = CodegenContext::new("test");
        declare_exception_runtime(&mut cgx.module);
        let mut fbc = FunctionBuilderContext::new();
        cgx.ctx
            .func
            .signature
            .returns
            .push(AbiParam::new(types::I64));

        let mut builder = FunctionBuilder::new(&mut cgx.ctx.func, &mut fbc);
        let entry = builder.create_block();
        builder.switch_to_block(entry);
        builder.seal_block(entry);

        let result_var = builder.declare_var(types::I64);
        let zero = builder.ins().iconst(types::I64, 0);
        builder.def_var(result_var, zero);

        emit_try_catch(
            |b| {
                let v = b.ins().iconst(types::I64, 1);
                b.def_var(result_var, v);
            },
            |b| {
                let v = b.ins().iconst(types::I64, -1i64);
                b.def_var(result_var, v);
            },
            &mut cgx.module,
            &mut builder,
        );

        let ret = builder.use_var(result_var);
        builder.ins().return_(&[ret]);
        builder.finalize();

        let flags = cranelift_codegen::settings::Flags::new(cranelift_codegen::settings::builder());
        assert!(
            cranelift_codegen::verify_function(&cgx.ctx.func, &flags).is_ok(),
            "{}",
            cranelift_codegen::verify_function(&cgx.ctx.func, &flags).unwrap_err()
        );
    }
}
