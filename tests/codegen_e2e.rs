use kiln_compiler::{
    analyzer::analyze,
    codegen::{compile::compile, context::CodegenContext, emit::emit_object},
    lexer::Lexer,
    parser::Parser,
};

/// Parse, analyze, and compile a Kiln source string, returning the object bytes.
fn compile_source(src: &str) -> Vec<u8> {
    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    let ast = Parser::new(tokens).parse_file().expect("parse failed");
    let typed = analyze(&ast).expect("analyze failed");
    let mut cgx = CodegenContext::new("e2e_test");
    compile(&typed, &mut cgx).expect("compile failed");
    emit_object(cgx).expect("emit failed")
}

#[test]
fn compile_void_main_produces_object() {
    let src = "def main() -> void {}";
    let bytes = compile_source(src);
    assert!(!bytes.is_empty(), "object file should not be empty");
}

#[test]
fn compile_returning_int_produces_object() {
    let src = "def answer() -> int { return 42 }";
    let bytes = compile_source(src);
    assert!(!bytes.is_empty());
}

#[test]
fn compile_two_functions_produces_object() {
    let src = r"
def add(x: int, y: int) -> int { return x + y }
def main() -> void {}
";
    let bytes = compile_source(src);
    assert!(!bytes.is_empty());
}

#[test]
fn compile_user_type_with_addable_and_normalizeable() {
    // Regression: hook +(rhs) and hook +() both used to encode to TypeName_add,
    // causing a duplicate-function panic. After the arity-aware fix they should
    // register as TypeName_add (binary) and TypeName_pos (unary) respectively.
    let src = r#"
struct Vec2 {
    x: float
    y: float
}
impl Addable for Vec2 {
    hook +(rhs: Vec2) -> Vec2 {
        return Vec2 { x: x + rhs.x, y: y + rhs.y }
    }
}
impl Normalizeable for Vec2 {
    hook +() -> Vec2 {
        len: float = x * x + y * y
        return Vec2 { x: x / len, y: y / len }
    }
}
def main() -> void {}
"#;
    let bytes = compile_source(src);
    assert!(!bytes.is_empty());
}

#[test]
fn compile_user_type_unary_neg_dispatches() {
    let src = r#"
struct Vec2 {
    x: float
    y: float
}
impl Negatable for Vec2 {
    hook -() -> Vec2 {
        return Vec2 { x: 0.0 - x, y: 0.0 - y }
    }
}
def negate(v: Vec2) -> Vec2 {
    return -v
}
def main() -> void {}
"#;
    let bytes = compile_source(src);
    assert!(!bytes.is_empty());
}

#[test]
fn compile_user_type_binary_add_dispatches() {
    let src = r#"
struct Vec2 {
    x: float
    y: float
}
impl Addable for Vec2 {
    hook +(rhs: Vec2) -> Vec2 {
        return Vec2 { x: x + rhs.x, y: y + rhs.y }
    }
}
def add_vecs(a: Vec2, b: Vec2) -> Vec2 {
    return a + b
}
def main() -> void {}
"#;
    let bytes = compile_source(src);
    assert!(!bytes.is_empty());
}

#[test]
fn kiln_build_subcommand_exists() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_kiln"))
        .args(["build", "--help"])
        .output()
        .expect("failed to run kiln");
    assert!(
        output.status.success(),
        "kiln build --help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn kiln_build_emits_object_file() {
    let tmp_dir = std::env::temp_dir();
    let src_path = tmp_dir.join("kiln_e2e_test.kn");
    let obj_path = tmp_dir.join("kiln_e2e_test.o");

    std::fs::write(&src_path, "def main() -> void {}").expect("write src");

    let status = std::process::Command::new(env!("CARGO_BIN_EXE_kiln"))
        .args([
            "build",
            "--no-link",
            src_path.to_str().unwrap(),
            "--output",
            obj_path.to_str().unwrap(),
        ])
        .status()
        .expect("run kiln build");

    assert!(status.success(), "kiln build failed");
    assert!(obj_path.exists(), "object file not created");
    assert!(obj_path.metadata().unwrap().len() > 0, "object file empty");
}
