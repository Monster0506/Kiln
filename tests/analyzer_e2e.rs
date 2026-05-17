use kiln_compiler::analyzer::analyze;
use kiln_compiler::annotations::{default_registry, run_processors};
use kiln_compiler::lexer::Lexer;
use kiln_compiler::parser::Parser;
use std::fs;

fn run(src: &str) -> Vec<String> {
    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    let mut ast = Parser::new(tokens).parse_file().expect("parse failed");
    let registry = default_registry();
    run_processors(&mut ast, &registry);
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

fn analyze_file(path: &str) -> Vec<String> {
    let src = fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    run(&src)
}

// ---------------------------------------------------------------------------
// Fancy-interface examples: full analyzer pass
// ---------------------------------------------------------------------------

#[test]
fn analyze_fancy_layer1_arithmetic() {
    let errs = analyze_file("examples/fancy-interfaces/layer1_arithmetic.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_layer1_comparison() {
    let errs = analyze_file("examples/fancy-interfaces/layer1_comparison.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_layer1_assign() {
    let errs = analyze_file("examples/fancy-interfaces/layer1_assign.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_layer1_unary() {
    let errs = analyze_file("examples/fancy-interfaces/layer1_unary.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_layer1_indexing() {
    let errs = analyze_file("examples/fancy-interfaces/layer1_indexing.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_layer1_callable_iter() {
    let errs = analyze_file("examples/fancy-interfaces/layer1_callable_iter.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_layer1_identity() {
    let errs = analyze_file("examples/fancy-interfaces/layer1_identity.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_layer2_shorthands() {
    let errs = analyze_file("examples/fancy-interfaces/layer2_shorthands.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_layer3_semantic() {
    let errs = analyze_file("examples/fancy-interfaces/layer3_semantic.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_layer3_collection() {
    let errs = analyze_file("examples/fancy-interfaces/layer3_collection.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_blanket_impls() {
    let errs = analyze_file("examples/fancy-interfaces/blanket_impls.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_specialized_impls() {
    let errs = analyze_file("examples/fancy-interfaces/specialized_impls.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_extension_impls() {
    let errs = analyze_file("examples/fancy-interfaces/extension_impls.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_assoc_types() {
    let errs = analyze_file("examples/fancy-interfaces/assoc_types.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_hkt() {
    let errs = analyze_file("examples/fancy-interfaces/hkt.kn");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn analyze_fancy_dispatch() {
    let errs = analyze_file("examples/fancy-interfaces/dispatch.kn");
    assert!(errs.is_empty(), "{errs:?}");
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
    assert!(
        errs.is_empty(),
        "int arithmetic should not produce bound violations: {errs:?}"
    );
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
    assert!(
        errs.is_empty(),
        "float < should be allowed via PartialOrd: {errs:?}"
    );
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
    assert!(
        errs.is_empty(),
        "int in string interpolation should pass (int: Display): {errs:?}"
    );
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
    assert!(
        has_bound_violation(&errs),
        "print(Point) without Display impl should fail: {errs:?}"
    );
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
    assert!(
        errs.is_empty(),
        "print(Point) with Display impl should pass: {errs:?}"
    );
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
    assert!(
        has_bound_violation(&errs),
        "struct without Display in interpolation should fail: {errs:?}"
    );
}

#[test]
fn vec_int_display_passes() {
    let errs = run(r#"
def main() -> void {
    v: Vec[int] = Vec.new()
    print(v)
}
"#);
    assert!(
        errs.is_empty(),
        "print(Vec[int]) should pass (Vec[int]: Display): {errs:?}"
    );
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
    assert!(
        errs.is_empty(),
        "generic fn called with int should pass: {errs:?}"
    );
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
    assert!(
        errs.is_empty(),
        "Wrapper with Display impl should allow interpolation: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// Bug 5: ImplKind propagation and enforcement
// ---------------------------------------------------------------------------

#[test]
fn duplicate_plain_impl_is_error() {
    let errs = run(r#"
interface Greet {
    hook greet() -> str
}

struct Dog {}

impl Greet for Dog {
    hook greet() -> str { return "woof" }
}

impl Greet for Dog {
    hook greet() -> str { return "bark" }
}
"#);
    assert!(
        !errs.is_empty(),
        "duplicate plain impl for same type+interface should be an error"
    );
}

#[test]
fn specialized_impl_alongside_plain_impl_is_ok() {
    let errs = run(r#"
interface Greet {
    hook greet() -> str
}

struct Dog {}

impl Greet for Dog {
    hook greet() -> str { return "woof" }
}

specialized impl Greet for Dog {
    hook greet() -> str { return "bark" }
}
"#);
    assert!(
        errs.is_empty(),
        "specialized impl alongside plain impl should not be an error: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// Bug 6: Associated type names in scope during impl resolution
// ---------------------------------------------------------------------------

#[test]
fn assoc_type_name_in_impl_hook_signature_is_not_undefined() {
    let errs = run(r#"
interface Add {
    type Output
    hook +(rhs: Self) -> Output
}

struct Vec2 { x: float, y: float }

impl Add for Vec2 {
    hook +(rhs: Vec2) -> Vec2 { return Vec2 { x: 0.0, y: 0.0 } }
}
"#);
    let undefined_output = errs
        .iter()
        .any(|e| e.contains("undefined") && e.contains("Output"));
    assert!(
        !undefined_output,
        "Output assoc type should be in scope inside impl block: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// 8: Ordering builtin type
// ---------------------------------------------------------------------------

#[test]
fn ordering_type_is_defined() {
    let errs = run(r#"
def compare(a: int, b: int) -> Ordering {
    return a <=> b
}
"#);
    let has_undefined_ordering = errs
        .iter()
        .any(|e| e.contains("undefined") && e.contains("Ordering"));
    assert!(
        !has_undefined_ordering,
        "Ordering should be a defined builtin type: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// 10: Object safety checking
// ---------------------------------------------------------------------------

#[test]
fn non_object_safe_interface_as_dynamic_type_is_error() {
    let errs = run(r#"
interface Cloneable {
    def clone() -> Self {}
}

def process(x: Cloneable) -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "using non-object-safe interface (method returns Self) as dynamic type should be an error: {errs:?}"
    );
}

#[test]
fn object_safe_interface_as_dynamic_type_is_ok() {
    let errs = run(r#"
interface Printable {
    def print() -> void {}
}

def process(x: Printable) -> void {}
"#);
    assert!(
        errs.is_empty(),
        "using object-safe interface as dynamic type should be fine: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// Enum iteration: for x <- EnumType
// ---------------------------------------------------------------------------

#[test]
fn for_over_enum_with_wrong_type_annotation_is_error() {
    let errs = run(r#"
enum Color { Red, Green, Blue }
def test() -> void {
    for c: int <- Color { }
}
"#);
    assert!(
        !errs.is_empty(),
        "binding annotated as int but iterating Color should be a type error"
    );
}

#[test]
fn for_over_enum_no_errors() {
    let errs = run(r#"
enum Direction { North, South, East, West }
def describe(d: Direction) -> str {
    return match d {
        North => "N",
        South => "S",
        East => "E",
        West => "W"
    }
}
def all_directions() -> void {
    for d <- Direction {
        describe(d)
    }
}
"#);
    assert!(
        errs.is_empty(),
        "for over enum should have no errors: {errs:?}"
    );
}

#[test]
fn for_over_enum_with_correct_annotation_no_errors() {
    let errs = run(r#"
enum Season { Spring, Summer, Autumn, Winter }
def test() -> void {
    for s: Season <- Season { }
}
"#);
    assert!(
        errs.is_empty(),
        "for with correct enum annotation should have no errors: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// @derive(Eq, Comparable) on enums
// ---------------------------------------------------------------------------

#[test]
fn derive_eq_on_enum_no_errors() {
    let errs = run(r#"
@derive(Eq)
enum Priority { Low, Medium, High, Critical }
"#);
    assert!(errs.is_empty(), "@derive(Eq) on enum should have no errors: {errs:?}");
}

#[test]
fn derive_comparable_on_enum_no_errors() {
    let errs = run(r#"
@derive(Comparable)
enum Priority { Low, Medium, High, Critical }
"#);
    assert!(errs.is_empty(), "@derive(Comparable) on enum should have no errors: {errs:?}");
}

#[test]
fn derive_eq_comparable_on_enum_allows_comparison() {
    let errs = run(r#"
@derive(Eq, Comparable)
enum Priority { Low, Medium, High, Critical }
def is_urgent(p: Priority) -> bool {
    return (p <=> Priority:High) >= 0
}
"#);
    assert!(errs.is_empty(), "@derive(Eq, Comparable) on enum with <=> should pass: {errs:?}");
}

#[test]
fn analyze_enums_derive_example() {
    let errs = analyze_file("examples/enums_derive.kn");
    assert!(errs.is_empty(), "enums_derive.kn: {errs:?}");
}
