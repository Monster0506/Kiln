fn main() {
    println!("cargo:rerun-if-changed=src/runtime/kiln_rt.cpp");
    println!("cargo:rerun-if-changed=kiln_rt/src/lib.rs");
    println!("cargo:rerun-if-changed=kiln_rt/Cargo.toml");

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

    match build_http_rt(&out_dir) {
        Some((lib_path, fmt)) => {
            println!("cargo:rustc-env=KILN_HTTP_RT_PATH={}", lib_path);
            println!("cargo:rustc-env=KILN_HTTP_RT_FORMAT={}", fmt);
        }
        None => {
            let empty = format!("{}/kiln_http_rt_empty.a", out_dir);
            std::fs::write(&empty, b"").unwrap();
            println!("cargo:rustc-env=KILN_HTTP_RT_PATH={}", empty);
            println!("cargo:rustc-env=KILN_HTTP_RT_FORMAT=none");
            println!(
                "cargo:warning=Failed to build Rust HTTP runtime; \
                 HTTP builtins will produce linker errors."
            );
        }
    }
}

// Returns (lib_path, format) where format is "gnu", "msvc", or "none".
fn build_http_rt(out_dir: &str) -> Option<(String, &'static str)> {
    let cargo = std::env::var("CARGO").ok()?;
    let target_dir = format!("{}/kiln_http_rt_build", out_dir);
    let release = std::env::var("PROFILE").ok().as_deref() == Some("release");
    let profile_dir = if release { "release" } else { "debug" };

    // On Windows MSVC hosts, user programs are typically linked by MinGW gcc,
    // so cross-compile for x86_64-pc-windows-gnu (GNU ar format, compatible with gcc).
    // On other hosts, compile natively.
    let gnu_cross = cfg!(all(windows, target_env = "msvc"));

    let target_triple = if gnu_cross {
        // Install the GNU target if not present (one-time setup).
        let _ = std::process::Command::new("rustup")
            .args(["target", "add", "x86_64-pc-windows-gnu"])
            .status();
        Some("x86_64-pc-windows-gnu")
    } else {
        None
    };

    let mut cmd = std::process::Command::new(&cargo);
    cmd.args([
        "build",
        "--manifest-path",
        "kiln_rt/Cargo.toml",
        "--target-dir",
        &target_dir,
    ]);
    if let Some(t) = target_triple {
        cmd.args(["--target", t]);
    }
    if release {
        cmd.arg("--release");
    }
    // Isolate from parent build flags that could cause conflicts.
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");
    cmd.env_remove("RUSTFLAGS");

    let status = cmd.status().ok()?;
    if !status.success() {
        return None;
    }

    if let Some(triple) = target_triple {
        // Output is at target_dir/<triple>/<profile>/libkiln_rt.a
        let path = format!("{}/{}/{}/libkiln_rt.a", target_dir, triple, profile_dir);
        if std::path::Path::new(&path).exists() {
            return Some((path, "gnu"));
        }
        return None;
    }

    // Native: MSVC produces kiln_rt.lib, GNU produces libkiln_rt.a
    let native_target = std::env::var("TARGET").unwrap_or_default();
    if native_target.contains("msvc") {
        let path = format!("{}/{}/kiln_rt.lib", target_dir, profile_dir);
        if std::path::Path::new(&path).exists() {
            return Some((path, "msvc"));
        }
    } else {
        let path = format!("{}/{}/libkiln_rt.a", target_dir, profile_dir);
        if std::path::Path::new(&path).exists() {
            return Some((path, "gnu"));
        }
    }
    None
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
