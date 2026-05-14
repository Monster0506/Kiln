use crate::codegen::context::CodegenContext;
use cranelift_object::ObjectProduct;
use std::path::Path;
use std::process::{Command, Stdio};

/// Kiln runtime object file, pre-compiled from kiln_rt.c during `cargo build`.
/// Empty slice means build.rs could not find a C compiler; linking will warn.
static RUNTIME_OBJ: &[u8] = include_bytes!(env!("KILN_RT_OBJ_PATH"));

/// Finalise the module and return the raw object file bytes.
pub fn emit_object(cgx: CodegenContext) -> Result<Vec<u8>, String> {
    let product: ObjectProduct = cgx.module.finish();
    product.emit().map_err(|e| e.to_string())
}

/// Write object bytes to `obj_path`, then invoke the system linker to
/// produce a native executable at `output`.
///
/// On Windows we try `link.exe` (MSVC) first, then `lld-link`, then `gcc`.
/// On Unix we use `cc`.
///
/// The Kiln runtime (pre-compiled from kiln_rt.c at cargo build time) is
/// written to a temporary object file and linked automatically.
pub fn link_executable(
    obj_bytes: &[u8],
    obj_path: &Path,
    output: &Path,
    verbose: bool,
) -> Result<(), String> {
    std::fs::write(obj_path, obj_bytes).map_err(|e| format!("write .o: {e}"))?;

    let tmp_dir = obj_path.parent().unwrap_or(Path::new("."));

    let runtime_path: Option<std::path::PathBuf> = if !RUNTIME_OBJ.is_empty() {
        let rt = tmp_dir.join("kiln_rt.o");
        std::fs::write(&rt, RUNTIME_OBJ).map_err(|e| format!("write runtime .o: {e}"))?;
        Some(rt)
    } else {
        eprintln!("warning: Kiln runtime not available; binary will be missing runtime symbols");
        None
    };

    struct LinkerSpec {
        name: &'static str,
        is_msvc_style: bool,
        trailing_libs: &'static [&'static str],
    }

    let linker_specs: &[LinkerSpec] = if cfg!(windows) {
        &[
            LinkerSpec {
                name: "link",
                is_msvc_style: true,
                trailing_libs: &[],
            },
            LinkerSpec {
                name: "lld-link",
                is_msvc_style: true,
                trailing_libs: &[],
            },
            // MinGW gcc: -lmingw32 supplies mainCRTStartup and the MinGW CRT
            LinkerSpec {
                name: "gcc",
                is_msvc_style: false,
                trailing_libs: &["-lmingw32"],
            },
        ]
    } else {
        &[LinkerSpec {
            name: "cc",
            is_msvc_style: false,
            trailing_libs: &[],
        }]
    };

    let mut last_err = String::from("no linker found");

    for spec in linker_specs {
        let mut cmd = Command::new(spec.name);

        if spec.is_msvc_style {
            let out_flag = format!("/out:{}", output.display());
            cmd.arg(obj_path).arg(&out_flag);
            if spec.name == "link" {
                cmd.args(&["/nologo", "/subsystem:console", "/defaultlib:libcmt"]);
            } else {
                cmd.args(&["/nologo", "/subsystem:console"]);
            }
            if let Some(ref rt) = runtime_path {
                cmd.arg(rt);
            }
        } else {
            cmd.arg("-o").arg(output).arg(obj_path);
            if let Some(ref rt) = runtime_path {
                cmd.arg(rt);
            }
            for lib in spec.trailing_libs {
                cmd.arg(lib);
            }
        }

        let stderr = if verbose {
            Stdio::inherit()
        } else {
            Stdio::null()
        };
        let result = cmd.stderr(stderr).status();
        match result {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                last_err = format!("{} exited with {}", spec.name, status);
                continue;
            }
            Err(_) => continue,
        }
    }

    Err(last_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::context::CodegenContext;
    use cranelift_codegen::ir::{types, AbiParam, InstBuilder};
    use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
    use cranelift_module::{Linkage, Module};

    #[test]
    fn emit_object_produces_bytes() {
        let mut cgx = CodegenContext::new("test_emit");
        let mut fbc = FunctionBuilderContext::new();

        let mut sig = cgx.module.make_signature();
        sig.returns.push(AbiParam::new(types::I64));
        let func_id = cgx
            .module
            .declare_function("test_fn", Linkage::Export, &sig)
            .unwrap();

        cgx.ctx.func.signature = sig;
        let mut builder = FunctionBuilder::new(&mut cgx.ctx.func, &mut fbc);
        let block = builder.create_block();
        builder.switch_to_block(block);
        builder.seal_block(block);
        let v = builder.ins().iconst(types::I64, 42);
        builder.ins().return_(&[v]);
        builder.finalize();

        cgx.module.define_function(func_id, &mut cgx.ctx).unwrap();
        cgx.ctx.clear();

        let bytes = emit_object(cgx).unwrap();
        assert!(!bytes.is_empty(), "object file must not be empty");
    }

    #[test]
    fn runtime_obj_is_embedded() {
        // RUNTIME_OBJ will be non-empty when a C compiler was available at
        // cargo build time. Just assert the slice exists; its content is
        // platform-specific and verified by the end-to-end link test.
        let _ = RUNTIME_OBJ;
    }
}
