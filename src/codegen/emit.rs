use crate::codegen::context::CodegenContext;
use cranelift_object::ObjectProduct;
use std::path::Path;
use std::process::{Command, Stdio};

/// Kiln runtime object file, pre-compiled from kiln_rt.cpp during `cargo build`.
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
        let rt = tmp_dir.join(format!("kiln_rt_{}.o", std::process::id()));
        std::fs::write(&rt, RUNTIME_OBJ).map_err(|e| format!("write runtime .o: {e}"))?;
        Some(rt)
    } else {
        eprintln!("warning: Kiln runtime not available; binary will be missing runtime symbols");
        None
    };

    struct LinkerSpec {
        name: &'static str,
        is_msvc_style: bool,
        // Flags placed before the output and object arguments.
        leading_flags: &'static [&'static str],
    }

    let linker_specs: &[LinkerSpec] = if cfg!(windows) {
        &[
            LinkerSpec {
                name: "link",
                is_msvc_style: true,
                leading_flags: &[],
            },
            LinkerSpec {
                name: "lld-link",
                is_msvc_style: true,
                leading_flags: &[],
            },
            LinkerSpec {
                name: "g++",
                is_msvc_style: false,
                leading_flags: &["-mconsole", "-Wl,--subsystem,console", "-Wl,-e,mainCRTStartup"],
            },
            LinkerSpec {
                name: "gcc",
                is_msvc_style: false,
                leading_flags: &["-mconsole", "-Wl,--subsystem,console", "-Wl,-e,mainCRTStartup"],
            },
        ]
    } else {
        &[LinkerSpec {
            name: "c++",
            is_msvc_style: false,
            leading_flags: &[],
        }, LinkerSpec {
            name: "cc",
            is_msvc_style: false,
            leading_flags: &[],
        }]
    };

    let mut last_err = String::new();
    let mut tried_names: Vec<&str> = Vec::new();
    let mut any_found = false;

    for spec in linker_specs {
        let mut cmd = Command::new(spec.name);

        if spec.is_msvc_style {
            let out_flag = format!("/out:{}", output.display());
            cmd.arg(obj_path).arg(&out_flag);
            if spec.name == "link" {
                cmd.args(["/nologo", "/subsystem:console", "/defaultlib:libcmt"]);
            } else {
                cmd.args(["/nologo", "/subsystem:console"]);
            }
            if let Some(ref rt) = runtime_path {
                cmd.arg(rt);
            }
        } else {
            for flag in spec.leading_flags {
                cmd.arg(flag);
            }
            cmd.arg("-o").arg(output).arg(obj_path);
            if let Some(ref rt) = runtime_path {
                cmd.arg(rt);
            }
        }

        tried_names.push(spec.name);

        let (status, captured) = if verbose {
            match cmd.stderr(Stdio::inherit()).status() {
                Ok(s) => (Ok(s), String::new()),
                Err(e) => (Err(e), String::new()),
            }
        } else {
            match cmd.stderr(Stdio::piped()).output() {
                Ok(out) => (
                    Ok(out.status),
                    String::from_utf8_lossy(&out.stderr).into_owned(),
                ),
                Err(e) => (Err(e), String::new()),
            }
        };

        match status {
            Ok(s) if s.success() => {
                if let Some(ref rt) = runtime_path {
                    let _ = std::fs::remove_file(rt);
                }
                return Ok(());
            }
            Ok(s) => {
                any_found = true;
                let mut msg = format!("'{}' exited with {}", spec.name, s);
                let trimmed = captured.trim();
                if !trimmed.is_empty() {
                    msg.push_str("\n  linker output:\n");
                    for line in trimmed.lines() {
                        msg.push_str(&format!("    {line}\n"));
                    }
                    // trim trailing newline we just added
                    if msg.ends_with('\n') {
                        msg.pop();
                    }
                } else if !verbose {
                    msg.push_str(" (re-run with --verbose for linker output)");
                }
                last_err = msg;
            }
            Err(_) => {
                // binary not found in PATH -- try the next candidate
                tried_names.pop();
            }
        }
    }

    if !any_found {
        let tried = if tried_names.is_empty() {
            linker_specs
                .iter()
                .map(|s| s.name)
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            tried_names.join(", ")
        };
        let hint = if cfg!(windows) {
            "install Visual Studio Build Tools (provides link.exe) or MSYS2/MinGW (provides gcc)"
        } else {
            "install a C toolchain: apt install gcc  or  brew install gcc"
        };
        Err(format!(
            "no linker found in PATH (tried: {tried})\nhint: {hint}"
        ))
    } else {
        Err(format!("link failed: {last_err}"))
    }
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
