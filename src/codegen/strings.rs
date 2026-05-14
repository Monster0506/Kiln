use cranelift_codegen::ir::{types, AbiParam, InstBuilder};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{DataDescription, Linkage, Module};
use cranelift_object::ObjectModule;

/// Declare all string-related runtime functions as external imports.
///
///   __kiln_print(str_val: i64)
///   __kiln_str_concat(a: i64, b: i64) -> i64
///   __kiln_int_to_str(n: i64) -> i64
///   __kiln_float_to_str(bits: i64) -> i64
///   __kiln_bool_to_str(b: i64) -> i64
pub fn declare_str_runtime(module: &mut ObjectModule) {
    // __kiln_print(str_val: i64) -> void
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        module.declare_function("__kiln_print", Linkage::Import, &sig).ok();
    }

    // __kiln_str_concat(a: i64, b: i64) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        module.declare_function("__kiln_str_concat", Linkage::Import, &sig).ok();
    }

    // __kiln_int_to_str(n: i64) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        module.declare_function("__kiln_int_to_str", Linkage::Import, &sig).ok();
    }

    // __kiln_float_to_str(bits: i64) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        module.declare_function("__kiln_float_to_str", Linkage::Import, &sig).ok();
    }

    // __kiln_bool_to_str(b: i64) -> i64
    {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        module.declare_function("__kiln_bool_to_str", Linkage::Import, &sig).ok();
    }
}

/// Emit a string literal as a pair of global data objects and return a
/// single `i64` Value that is a pointer to a heap-layout `{ ptr: i64, len: i64 }`
/// (a KilnStr fat-pointer struct) stored in a global.
///
/// Layout of the fat-pointer global (16 bytes, little-endian):
///   bytes  0..8  : relocation pointing to the UTF-8 bytes global
///   bytes  8..16 : byte length as a little-endian i64
pub fn emit_str_literal(
    s: &str,
    module: &mut ObjectModule,
    builder: &mut FunctionBuilder,
) -> cranelift_codegen::ir::Value {
    let bytes = s.as_bytes();
    let byte_len = bytes.len() as i64;
    let mangled = mangle(s);

    // 1. Declare + define the raw UTF-8 bytes global (null-terminated).
    let bytes_name = format!(".str.bytes.{}", mangled);
    let bytes_id = module
        .declare_data(&bytes_name, Linkage::Local, false, false)
        .unwrap_or_else(|_| module.get_name(&bytes_name).and_then(|fod| {
            if let cranelift_module::FuncOrDataId::Data(id) = fod { Some(id) } else { None }
        }).expect("bytes data id"));

    {
        let mut desc = DataDescription::new();
        let mut data = bytes.to_vec();
        data.push(0); // null terminator
        desc.define(data.into_boxed_slice());
        module.define_data(bytes_id, &desc).ok();
    }

    // 2. Declare + define the KilnStr fat-pointer struct global (16 bytes).
    //    [ reloc_to_bytes (8 bytes) | byte_len as i64 LE (8 bytes) ]
    let fat_name = format!(".str.fat.{}", mangled);
    let fat_id = module
        .declare_data(&fat_name, Linkage::Local, false, false)
        .unwrap_or_else(|_| module.get_name(&fat_name).and_then(|fod| {
            if let cranelift_module::FuncOrDataId::Data(id) = fod { Some(id) } else { None }
        }).expect("fat data id"));

    {
        let mut desc = DataDescription::new();
        // 16-byte buffer: 8 for ptr reloc, 8 for length
        let mut buf = vec![0u8; 16];
        // Write the length into bytes 8..16 as little-endian i64
        let len_bytes = byte_len.to_le_bytes();
        buf[8..16].copy_from_slice(&len_bytes);
        desc.define(buf.into_boxed_slice());
        // Embed relocation at offset 0 pointing to the bytes global
        let gv = module.declare_data_in_data(bytes_id, &mut desc);
        desc.write_data_addr(0, gv, 0);
        module.define_data(fat_id, &desc).ok();
    }

    // 3. Return the address of the fat-pointer global as an i64.
    let fat_gv = module.declare_data_in_func(fat_id, builder.func);
    builder.ins().global_value(types::I64, fat_gv)
}

fn mangle(s: &str) -> String {
    s.bytes().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::context::CodegenContext;
    use cranelift_codegen::ir::{types, AbiParam};
    use cranelift_frontend::FunctionBuilderContext;

    #[test]
    fn str_runtime_declared() {
        let mut cgx = CodegenContext::new("test");
        declare_str_runtime(&mut cgx.module);
        assert!(cgx.module.get_name("__kiln_str_concat").is_some());
        assert!(cgx.module.get_name("__kiln_int_to_str").is_some());
        assert!(cgx.module.get_name("__kiln_print").is_some());
        assert!(cgx.module.get_name("__kiln_float_to_str").is_some());
        assert!(cgx.module.get_name("__kiln_bool_to_str").is_some());
    }

    #[test]
    fn str_literal_emits_ptr_and_len() {
        let mut cgx = CodegenContext::new("test");
        let mut fbc = FunctionBuilderContext::new();
        cgx.ctx.func.signature.returns.push(AbiParam::new(types::I64));

        let mut builder = FunctionBuilder::new(&mut cgx.ctx.func, &mut fbc);
        let block = builder.create_block();
        builder.switch_to_block(block);
        builder.seal_block(block);

        let fat_ptr = emit_str_literal("hello", &mut cgx.module, &mut builder);
        assert_eq!(builder.func.dfg.value_type(fat_ptr), types::I64);
        builder.ins().return_(&[fat_ptr]);
        builder.finalize();
    }
}
