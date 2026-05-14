use cranelift_codegen::settings::Configurable;
use cranelift_codegen::Context;
use cranelift_module::Module;
use cranelift_object::{ObjectBuilder, ObjectModule};

pub struct CodegenContext {
    pub module: ObjectModule,
    pub ctx: Context,
}

impl CodegenContext {
    pub fn new(module_name: &str) -> Self {
        let mut flag_builder = cranelift_codegen::settings::builder();
        flag_builder.set("use_colocated_libcalls", "false").unwrap();
        flag_builder.set("is_pic", "false").unwrap();
        let flags = cranelift_codegen::settings::Flags::new(flag_builder);

        let isa = cranelift_native::builder()
            .expect("unsupported host platform")
            .finish(flags)
            .expect("failed to build ISA");

        let obj_builder =
            ObjectBuilder::new(isa, module_name, cranelift_module::default_libcall_names())
                .expect("failed to create ObjectBuilder");

        let module = ObjectModule::new(obj_builder);
        let ctx = module.make_context();

        Self { module, ctx }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_creates_without_panic() {
        let _ctx = CodegenContext::new("test_module");
    }
}
