use kiln_compiler::analyzer::analyze;
use kiln_compiler::lexer::Lexer;
use kiln_compiler::parser::Parser;

fn run(src: &str) -> Vec<String> {
    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    let ast = Parser::new(tokens).parse_file().expect("parse failed");
    match analyze(&ast) {
        Ok(_) => vec![],
        Err(errs) => errs
            .iter()
            .map(|e: &kiln_compiler::analyzer::AnalysisError| e.to_string())
            .collect(),
    }
}

fn has_bound_violation(errs: &[String]) -> bool {
    errs.iter().any(|e| e.contains("does not implement"))
}

#[test]
fn valid_function_passes() {
    let errs = run(r#"
def add(a: int, b: int) -> int {
    return a + b
}
"#);
    assert!(errs.is_empty(), "{errs:?}");
}

// Shadowing is legal in Kiln: redeclaring a name creates a new binding.
#[test]
fn shadowing_is_allowed() {
    let errs = run(r#"
def foo() -> void {
    x: int = 1
    x: int = 2
}
"#);
    assert!(errs.is_empty(), "shadowing should be allowed: {errs:?}");
}

// Assigning to an immutable binding is an error.
#[test]
fn assign_to_immutable_is_rejected() {
    let errs = run(r#"
def foo() -> void {
    x: int = 1
    x = 2
}
"#);
    assert!(
        !errs.is_empty(),
        "expected an error for assigning to immutable binding"
    );
}

// Assigning to a mut binding is fine.
#[test]
fn assign_to_mut_is_ok() {
    let errs = run(r#"
def foo() -> void {
    mut x: int = 1
    x = 2
}
"#);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn missing_return_is_rejected() {
    let errs = run(r#"
def foo() -> int {
    x: int = 1
}
"#);
    assert!(!errs.is_empty());
}

#[test]
fn void_function_without_return_passes() {
    let errs = run(r#"
def log(msg: str) -> void {
    print(msg)
}
"#);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn type_mismatch_is_rejected() {
    let errs = run(r#"
def foo() -> void {
    x: bool = 42
}
"#);
    assert!(!errs.is_empty());
}

#[test]
fn overloaded_functions_pass() {
    let errs = run(r#"
def add(a: int, b: int) -> int {
    return a + b
}

def add(a: float, b: float) -> float {
    return a + b
}

def main() -> void {
    mut x: int = add(1, 2)
    mut y: float = add(1.0, 2.0)
    println(x)
    println(y)
}
"#);
    assert!(errs.is_empty(), "overloads should be valid: {errs:?}");
}

#[test]
fn no_matching_overload_is_error() {
    let errs = run(r#"
def greet(name: str) -> void {
    println(name)
}

def greet(times: int) -> void {
    println(times)
}

def main() -> void {
    greet(true)
}
"#);
    assert!(!errs.is_empty(), "expected error for unmatched overload");
}

#[test]
fn duplicate_non_function_is_error() {
    let errs = run(r#"
struct Foo { x: int }
struct Foo { y: int }
"#);
    assert!(!errs.is_empty(), "duplicate struct name must be an error");
}

// ---------------------------------------------------------------------------
// Constraint solver: operator interface bounds
// ---------------------------------------------------------------------------

#[test]
fn int_arithmetic_passes() {
    let errs = run(r#"
def main() -> void {
    x: int = 1 + 2
    y: int = x * 3
    z: int = y - 1
}
"#);
    assert!(errs.is_empty(), "int arithmetic should not produce bound violations: {errs:?}");
}

#[test]
fn float_arithmetic_passes() {
    let errs = run(r#"
def main() -> void {
    x: float = 1.0 + 2.0
    y: float = x * 3.0
}
"#);
    assert!(errs.is_empty(), "float arithmetic should pass: {errs:?}");
}

#[test]
fn float_comparison_passes() {
    let errs = run(r#"
def main() -> void {
    x: float = 1.5
    y: bool = x < 2.0
}
"#);
    assert!(errs.is_empty(), "float < should be allowed via PartialOrd: {errs:?}");
}

#[test]
fn int_equality_passes() {
    let errs = run(r#"
def main() -> void {
    x: bool = 1 == 2
}
"#);
    assert!(errs.is_empty(), "int == should pass: {errs:?}");
}

#[test]
fn string_interpolation_int_passes() {
    let errs = run(r#"
def main() -> void {
    x: int = 42
    s: str = "{x}"
}
"#);
    assert!(errs.is_empty(), "int in string interpolation should pass (int: Display): {errs:?}");
}

#[test]
fn print_int_passes() {
    let errs = run(r#"
def main() -> void {
    print(42)
}
"#);
    assert!(errs.is_empty(), "print(int) should pass: {errs:?}");
}

#[test]
fn print_str_passes() {
    let errs = run(r#"
def main() -> void {
    print("hello")
}
"#);
    assert!(errs.is_empty(), "print(str) should pass: {errs:?}");
}

#[test]
fn print_struct_without_display_fails() {
    let errs = run(r#"
struct Point { x: int, y: int }

def main() -> void {
    p: Point = Point { x: 1, y: 2 }
    print(p)
}
"#);
    assert!(has_bound_violation(&errs), "print(Point) without Display impl should fail: {errs:?}");
}

#[test]
fn print_struct_with_display_impl_passes() {
    let errs = run(r#"
struct Point { x: int, y: int }

impl Display for Point {
    hook to_str() -> str { return "point" }
}

def main() -> void {
    p: Point = Point { x: 1, y: 2 }
    print(p)
}
"#);
    assert!(errs.is_empty(), "print(Point) with Display impl should pass: {errs:?}");
}

#[test]
fn string_interp_struct_without_display_fails() {
    let errs = run(r#"
struct Foo { val: int }

def main() -> void {
    f: Foo = Foo { val: 1 }
    s: str = "{f}"
}
"#);
    assert!(has_bound_violation(&errs), "struct without Display in interpolation should fail: {errs:?}");
}

#[test]
fn vec_int_display_passes() {
    let errs = run(r#"
def main() -> void {
    v: Vec[int] = Vec.new()
    print(v)
}
"#);
    assert!(errs.is_empty(), "print(Vec[int]) should pass (Vec[int]: Display): {errs:?}");
}

#[test]
fn generic_fn_with_bound_passes_with_int() {
    let errs = run(r#"
def double_print[T: Display](val: T) -> void {
    println(val)
}

def main() -> void {
    double_print(42)
}
"#);
    assert!(errs.is_empty(), "generic fn called with int should pass: {errs:?}");
}

#[test]
fn impl_with_generic_bound_registers_conformance() {
    let errs = run(r#"
struct Wrapper { inner: int }

impl Display for Wrapper {
    hook to_str() -> str { return "wrapper" }
}

def main() -> void {
    w: Wrapper = Wrapper { inner: 1 }
    s: str = "{w}"
}
"#);
    assert!(errs.is_empty(), "Wrapper with Display impl should allow interpolation: {errs:?}");
}
