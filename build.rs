fn main() {
    println!("cargo:rerun-if-changed=src/runtime/kiln_rt.cpp");

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let obj_path = format!("{}/kiln_rt.o", out_dir);

    let compiled = try_compile("src/runtime/kiln_rt.cpp", &obj_path);

    if compiled {
        println!("cargo:rustc-env=KILN_RT_OBJ_PATH={}", obj_path);
    } else {
        std::fs::write(&obj_path, []).unwrap();
        println!("cargo:rustc-env=KILN_RT_OBJ_PATH={}", obj_path);
        println!(
            "cargo:warning=No C++ compiler found; Kiln runtime (kiln_rt.cpp) not pre-compiled. \
             Linking user programs will fail. Install g++, clang++, or c++."
        );
    }
}

fn try_compile(src: &str, obj: &str) -> bool {
    let candidates: &[(&str, &[&str])] = if cfg!(windows) {
        &[
            ("g++", &["-O2", "-c", "-o"]),
            ("c++", &["-O2", "-c", "-o"]),
            ("clang++", &["-O2", "-c", "-o"]),
        ]
    } else {
        &[
            ("c++", &["-O2", "-c", "-o"]),
            ("g++", &["-O2", "-c", "-o"]),
            ("clang++", &["-O2", "-c", "-o"]),
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
