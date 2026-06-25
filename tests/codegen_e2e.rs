use kiln_compiler::{
    analyzer::{analyze, TypeRegistry},
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
    let registry = TypeRegistry::new();
    compile(&typed, &mut cgx, false, 3, &registry).expect("compile failed");
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

// Droppable / RAII drop tests

const DROP_PRELUDE: &str = r#"
struct Tracked { id: int }
impl Droppable for Tracked {
    hook drop() -> void { println("drop:{self.id}") }
}
"#;

#[test]
fn drop_fires_at_scope_exit() {
    let src = format!(
        r#"{DROP_PRELUDE}
@entry
def main() -> int {{
    t: Tracked = Tracked {{ id: 1 }}
    println("before")
    return 0
}}"#
    );
    let out = kiln_run(&src);
    assert_eq!(out.lines().collect::<Vec<_>>(), vec!["before", "drop:1"]);
}

#[test]
fn drop_fires_on_early_return() {
    let src = format!(
        r#"{DROP_PRELUDE}
def make_and_drop() -> void {{
    t: Tracked = Tracked {{ id: 2 }}
    return
}}
@entry
def main() -> int {{
    make_and_drop()
    return 0
}}"#
    );
    let out = kiln_run(&src);
    assert_eq!(out.trim(), "drop:2");
}

#[test]
fn drop_fires_on_loop_break() {
    let src = format!(
        r#"{DROP_PRELUDE}
@entry
def main() -> int {{
    mut i: int = 0
    while i < 3 {{
        t: Tracked = Tracked {{ id: i }}
        if i == 1 {{
            break
        }}
        i = i + 1
    }}
    return 0
}}"#
    );
    let out = kiln_run(&src);
    // iteration 0: normal exit drops id=0; iteration 1: break drops id=1
    assert_eq!(out.lines().collect::<Vec<_>>(), vec!["drop:0", "drop:1"]);
}

#[test]
fn drop_order_is_reverse_declaration() {
    let src = format!(
        r#"{DROP_PRELUDE}
def multi_drop() -> void {{
    a: Tracked = Tracked {{ id: 10 }}
    b: Tracked = Tracked {{ id: 20 }}
    c: Tracked = Tracked {{ id: 30 }}
}}
@entry
def main() -> int {{
    multi_drop()
    return 0
}}"#
    );
    let out = kiln_run(&src);
    // c declared last, so dropped first
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        vec!["drop:30", "drop:20", "drop:10"]
    );
}

#[test]
fn compile_droppable_interface_is_builtin() {
    // Droppable must resolve from the prelude without a local definition.
    let src = r#"
struct Handle { fd: int }
impl Droppable for Handle {
    hook drop() -> void { println("closed") }
}
@entry
def main() -> int {
    h: Handle = Handle { fd: 3 }
    return 0
}
"#;
    let out = kiln_run(src);
    assert_eq!(out.trim(), "closed");
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
fn match_block_arms_execute_statements() {
    let out = kiln_run(
        r#"
def main() -> void {
    x: int = 2
    mut result: int = 0
    match x {
        1 => { result = 1 }
        2 => {
            result = 20
            result = result + 5
        }
        3 => { result = 3 }
    }
    println(result)
}
"#,
    );
    assert_eq!(out.trim(), "25", "got: {out}");
}

#[test]
fn negative_index_accesses_from_end() {
    let out = kiln_run(
        r#"
def main() -> void {
    v: Vec[int] = [10, 20, 30]
    println(v[-1])
    println(v[-2])
    println(v[-3])
}
"#,
    );
    let lines: Vec<_> = out.lines().map(str::trim).collect();
    assert_eq!(lines, ["30", "20", "10"], "got: {out}");
}

#[test]
fn negative_index_assignment_writes_from_end() {
    let out = kiln_run(
        r#"
def main() -> void {
    mut v: Vec[int] = [1, 2, 3]
    v[-1] = 99
    println(v[2])
}
"#,
    );
    assert_eq!(out.trim(), "99", "got: {out}");
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
fn entry_annotation_compiles_and_runs() {
    let out = kiln_run(
        r#"
@entry
def run() -> void {
    println(42)
}
"#,
    );
    assert_eq!(out.trim(), "42", "got: {out}");
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

// ---- Mut-to-immut promotion and global inlining --------------------------------

#[test]
fn immutable_int_global_is_readable() {
    let out = kiln_run(
        r#"
LIMIT: int = 42
def main() -> void {
    println(LIMIT)
}
"#,
    );
    assert_eq!(out.trim(), "42", "got: {out}");
}

#[test]
fn immutable_float_global_is_readable() {
    let out = kiln_run(
        r#"
PI: float = 3.14
def main() -> void {
    println(PI)
}
"#,
    );
    assert!(out.trim().starts_with("3.14"), "got: {out}");
}

#[test]
fn immutable_bool_global_is_readable() {
    let out = kiln_run(
        r#"
FLAG: bool = true
def main() -> void {
    if FLAG {
        println(1)
    } else {
        println(0)
    }
}
"#,
    );
    assert_eq!(out.trim(), "1", "got: {out}");
}

#[test]
fn mutable_global_can_be_reassigned() {
    let out = kiln_run(
        r#"
mut counter: int = 0
def main() -> void {
    counter = 99
    println(counter)
}
"#,
    );
    assert_eq!(out.trim(), "99", "got: {out}");
}

#[test]
fn immutable_global_used_multiple_times() {
    let out = kiln_run(
        r#"
BASE: int = 10
def main() -> void {
    println(BASE + BASE)
}
"#,
    );
    assert_eq!(out.trim(), "20", "got: {out}");
}

#[test]
fn mut_global_never_written_is_inlined() {
    // `mut LIMIT` declared but never assigned -> should be promoted and inlined
    let out = kiln_run(
        r#"
mut LIMIT: int = 100
def main() -> void {
    println(LIMIT)
}
"#,
    );
    assert_eq!(out.trim(), "100", "got: {out}");
}

// ---- Forward function references -----------------------------------------------

#[test]
fn forward_reference_simple_call() {
    let out = kiln_run(
        r#"
def main() -> void {
    println(add(3, 4))
}

def add(x: int, y: int) -> int {
    return x + y
}
"#,
    );
    assert_eq!(out.trim(), "7", "got: {out}");
}

#[test]
fn forward_reference_multiple_functions() {
    let out = kiln_run(
        r#"
def main() -> void {
    println(add(10, 3))
    println(sub(10, 3))
}

def add(x: int, y: int) -> int {
    return x + y
}

def sub(x: int, y: int) -> int {
    return x - y
}
"#,
    );
    let lines: Vec<_> = out.lines().map(str::trim).collect();
    assert_eq!(lines, ["13", "7"], "got: {out}");
}

#[test]
fn forward_reference_mutual_recursion() {
    let out = kiln_run(
        r#"
def main() -> void {
    println(is_even(4))
}

def is_even(n: int) -> bool {
    if n == 0 { return true }
    return is_odd(n - 1)
}

def is_odd(n: int) -> bool {
    if n == 0 { return false }
    return is_even(n - 1)
}
"#,
    );
    assert_eq!(out.trim(), "true", "got: {out}");
}

// ---- Inline hook syntax --------------------------------------------------------

#[test]
fn inline_hook_addable_dispatches() {
    let out = kiln_run(
        r#"
struct Vec2 {
    x: float
    y: float

    @implements[Addable]
    hook +(rhs: Vec2) -> Vec2 {
        return Vec2 { x: self.x + rhs.x, y: self.y + rhs.y }
    }
}

def main() -> void {
    a: Vec2 = Vec2 { x: 1.0, y: 2.0 }
    b: Vec2 = Vec2 { x: 3.0, y: 4.0 }
    c: Vec2 = a + b
    println(c.x)
    println(c.y)
}
"#,
    );
    let lines: Vec<_> = out.lines().map(str::trim).collect();
    assert_eq!(lines, ["4", "6"], "got: {out}");
}

#[test]
fn inline_hook_multiple_interfaces() {
    let out = kiln_run(
        r#"
struct Counter {
    current: int
    stop: int

    @implements[Iterator]
    hook next() -> Option[int] {
        if self.current >= self.stop {
            return Option:None
        }
        val: int = self.current
        self.current = self.current + 1
        return Option:Some { value: val }
    }

    @implements[Iterable]
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
fn inline_hook_missing_implements_is_compile_error() {
    let src = r#"
struct Bad {
    x: int
    hook +(rhs: Bad) -> Bad {
        return Bad { x: self.x + rhs.x }
    }
}
def main() -> void {}
"#;
    let tokens = kiln_compiler::lexer::Lexer::new(src)
        .tokenize()
        .expect("lex failed");
    let ast = kiln_compiler::parser::Parser::new(tokens)
        .parse_file()
        .expect("parse failed");
    let result = kiln_compiler::analyzer::analyze(&ast);
    assert!(
        result.is_err(),
        "expected compile error for bare hook in struct"
    );
    let errs = result.unwrap_err();
    assert!(
        errs.iter().any(|e| e.code() == "E031"),
        "expected E031 MissingImplementsAnnotation, got: {errs:?}"
    );
}
