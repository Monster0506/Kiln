fn main() {
    println!("cargo:rerun-if-changed=src/runtime/kiln_rt.c");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let obj_path = format!("{}/kiln_rt.o", out_dir);

    let compiled = try_compile("src/runtime/kiln_rt.c", &obj_path);

    if compiled {
        println!("cargo:rustc-env=KILN_RT_OBJ_PATH={}", obj_path);
    } else {
        // Write an empty placeholder so include_bytes! does not fail to compile.
        // link_executable checks the length and warns at runtime instead.
        std::fs::write(&obj_path, []).unwrap();
        println!("cargo:rustc-env=KILN_RT_OBJ_PATH={}", obj_path);
        println!(
            "cargo:warning=No C compiler found; Kiln runtime (kiln_rt.c) not pre-compiled. \
             Linking user programs will fail. Install gcc, clang, or cc."
        );
    }
}

fn try_compile(src: &str, obj: &str) -> bool {
    let candidates: &[(&str, &[&str])] = if cfg!(windows) {
        &[
            ("gcc", &["-O2", "-c", "-o"]),
            ("cc", &["-O2", "-c", "-o"]),
            ("clang", &["-O2", "-c", "-o"]),
        ]
    } else {
        &[
            ("cc", &["-O2", "-c", "-o"]),
            ("gcc", &["-O2", "-c", "-o"]),
            ("clang", &["-O2", "-c", "-o"]),
        ]
    };

    for (compiler, flags) in candidates {
        let mut cmd = std::process::Command::new(compiler);
        for f in *flags {
            cmd.arg(f);
        }
        cmd.arg(obj).arg(src);
        if cmd.status().map(|s| s.success()).unwrap_or(false) {
            return true;
        }
    }
    false
}
