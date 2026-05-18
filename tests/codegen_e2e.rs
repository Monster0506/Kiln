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
        return Vec2 { x: self.x + rhs.x, y: self.y + rhs.y }
    }
}
impl Normalizeable for Vec2 {
    hook +() -> Vec2 {
        len: float = self.x * self.x + self.y * self.y
        return Vec2 { x: self.x / len, y: self.y / len }
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
        return Vec2 { x: 0.0 - self.x, y: 0.0 - self.y }
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
        return Vec2 { x: self.x + rhs.x, y: self.y + rhs.y }
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

fn kiln_run(src: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let id = CTR.fetch_add(1, Ordering::Relaxed);
    let tmp_dir = std::env::temp_dir();
    let src_path = tmp_dir.join(format!("kiln_run_test_{id}.kn"));
    std::fs::write(&src_path, src).expect("write src");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_kiln"))
        .args(["run", src_path.to_str().unwrap()])
        .output()
        .expect("run kiln run");
    assert!(
        output.status.success(),
        "kiln run failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn array_literal_length_is_correct() {
    let out = kiln_run(
        r#"
def main() -> void {
    v: Vec[int] = [10, 20, 30]
    println(v.length())
}
"#,
    );
    assert_eq!(out.trim(), "3", "expected length 3, got: {out}");
}

#[test]
fn array_literal_elements_are_accessible() {
    let out = kiln_run(
        r#"
def main() -> void {
    v: Vec[int] = [7, 8, 9]
    println(v[0])
    println(v[1])
    println(v[2])
}
"#,
    );
    let lines: Vec<_> = out.lines().map(str::trim).collect();
    assert_eq!(lines, ["7", "8", "9"], "got: {out}");
}

#[test]
fn empty_array_literal_has_zero_length() {
    let out = kiln_run(
        r#"
def main() -> void {
    v: Vec[int] = []
    println(v.length())
}
"#,
    );
    assert_eq!(out.trim(), "0", "got: {out}");
}

// ---- Custom Iterator / Iterable tests ----------------------------------------

#[test]
fn custom_iterator_produces_correct_values() {
    let out = kiln_run(
        r#"
struct Counter {
    current: int
    stop: int
}

impl Iterator for Counter {
    hook next() -> Option[int] {
        if self.current >= self.stop {
            return Option:None
        }
        val: int = self.current
        self.current = self.current + 1
        return Option:Some { value: val }
    }
}

impl Iterable for Counter {
    hook iter() -> Counter {
        return self
    }
}

def main() -> void {
    c: Counter = Counter { current: 0, stop: 3 }
    for x <- c {
        println(x)
    }
}
"#,
    );
    let lines: Vec<_> = out.lines().map(str::trim).collect();
    assert_eq!(lines, ["0", "1", "2"], "got: {out}");
}

#[test]
fn custom_iterator_break_exits_early() {
    let out = kiln_run(
        r#"
struct Counter {
    current: int
    stop: int
}

impl Iterator for Counter {
    hook next() -> Option[int] {
        if self.current >= self.stop {
            return Option:None
        }
        val: int = self.current
        self.current = self.current + 1
        return Option:Some { value: val }
    }
}

impl Iterable for Counter {
    hook iter() -> Counter {
        return self
    }
}

def main() -> void {
    c: Counter = Counter { current: 0, stop: 10 }
    for x <- c {
        println(x)
        if x == 2 { break }
    }
}
"#,
    );
    let lines: Vec<_> = out.lines().map(str::trim).collect();
    assert_eq!(lines, ["0", "1", "2"], "got: {out}");
}

#[test]
fn custom_iterator_empty_runs_no_iterations() {
    let out = kiln_run(
        r#"
struct Counter {
    current: int
    stop: int
}

impl Iterator for Counter {
    hook next() -> Option[int] {
        if self.current >= self.stop {
            return Option:None
        }
        val: int = self.current
        self.current = self.current + 1
        return Option:Some { value: val }
    }
}

impl Iterable for Counter {
    hook iter() -> Counter {
        return self
    }
}

def main() -> void {
    c: Counter = Counter { current: 5, stop: 5 }
    mut ran: int = 0
    for x <- c {
        ran = ran + 1
    }
    println(ran)
}
"#,
    );
    assert_eq!(out.trim(), "0", "got: {out}");
}

#[test]
fn enum_variant_with_fields_constructs_correctly() {
    let out = kiln_run(
        r#"
def make_some(n: int) -> Option[int] {
    return Option:Some { value: n }
}

def main() -> void {
    opt: Option[int] = make_some(42)
    result: int = match opt {
        Some { value: v } => v,
        None => 0
    }
    println(result)
}
"#,
    );
    assert_eq!(out.trim(), "42", "got: {out}");
}

// ---- Vec Iterable / VecIter tests -------------------------------------------

#[test]
fn vec_for_loop_iterates_all_elements() {
    let out = kiln_run(
        r#"
def main() -> void {
    v: Vec[int] = [10, 20, 30]
    for x <- v {
        println(x)
    }
}
"#,
    );
    let lines: Vec<_> = out.lines().map(str::trim).collect();
    assert_eq!(lines, ["10", "20", "30"], "got: {out}");
}

#[test]
fn vec_for_loop_empty_vec_runs_zero_iterations() {
    let out = kiln_run(
        r#"
def main() -> void {
    v: Vec[int] = []
    mut ran: int = 0
    for x <- v {
        ran = ran + 1
    }
    println(ran)
}
"#,
    );
    assert_eq!(out.trim(), "0", "got: {out}");
}

#[test]
fn vec_for_loop_sum_elements() {
    let out = kiln_run(
        r#"
def main() -> void {
    v: Vec[int] = [1, 2, 3, 4, 5]
    mut total: int = 0
    for x <- v {
        total = total + x
    }
    println(total)
}
"#,
    );
    assert_eq!(out.trim(), "15", "got: {out}");
}

#[test]
fn vec_for_loop_break_exits_early() {
    let out = kiln_run(
        r#"
def main() -> void {
    v: Vec[int] = [1, 2, 3, 4, 5]
    for x <- v {
        println(x)
        break
    }
}
"#,
    );
    assert_eq!(out.trim(), "1", "got: {out}");
}
