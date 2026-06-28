use cranelift_codegen::ir::{types, AbiParam};
use cranelift_module::{FuncId, Linkage, Module};
use cranelift_object::ObjectModule;
use std::collections::HashMap;

/// Declare Path/HTTP runtime functions as external imports, returning FuncIds to
/// pre-seed func_ids and prevent re-declaration when processing builtins.kn.
pub fn declare_io_runtime(module: &mut ObjectModule) -> HashMap<String, FuncId> {
    let mut ids: HashMap<String, FuncId> = HashMap::new();

    // path_cwd() -> str
    let id = {
        let mut sig = module.make_signature();
        sig.returns.push(AbiParam::new(types::I64));
        import_fn(module, "path_cwd", sig)
    };
    ids.insert("path_cwd".into(), id);

    // path_temp_dir() -> str
    let id = {
        let mut sig = module.make_signature();
        sig.returns.push(AbiParam::new(types::I64));
        import_fn(module, "path_temp_dir", sig)
    };
    ids.insert("path_temp_dir".into(), id);

    // http_get(url: str) -> Result[HttpResponse, str]
    let id = {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        import_fn(module, "http_get", sig)
    };
    ids.insert("http_get".into(), id);

    // http_post(url: str, body: str, content_type: str) -> Result[HttpResponse, str]
    let id = {
        let mut sig = module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        import_fn(module, "http_post", sig)
    };
    ids.insert("http_post".into(), id);

    ids
}

fn import_fn(
    module: &mut ObjectModule,
    name: &str,
    sig: cranelift_codegen::ir::Signature,
) -> FuncId {
    module
        .declare_function(name, Linkage::Import, &sig)
        .unwrap_or_else(|e| panic!("failed to declare runtime import '{}': {e}", name))
}
