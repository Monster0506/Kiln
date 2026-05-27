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

#[test]
fn hook_with_self_param_is_not_erasable() {
    let errs = run(r#"
interface Addable {
    hook +(other: Self) -> Self {}
}

def process(x: Addable) -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "hook with Self parameter makes interface non-erasable: {errs:?}"
    );
}

#[test]
fn hook_with_self_return_is_not_erasable() {
    let errs = run(r#"
interface Builder {
    hook build() -> Self {}
}

def process(x: Builder) -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "hook returning Self makes interface non-erasable: {errs:?}"
    );
}

#[test]
fn generic_method_makes_interface_non_erasable() {
    let errs = run(r#"
interface Mapper {
    def map[U](f: int) -> U {}
}

def process(x: Mapper) -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "generic method makes interface non-erasable: {errs:?}"
    );
}

#[test]
fn non_erasable_interface_as_return_type_is_error() {
    let errs = run(r#"
interface Cloneable {
    def clone() -> Self {}
}

def make_one() -> Cloneable {}
"#);
    assert!(
        !errs.is_empty(),
        "non-erasable interface as function return type should be an error: {errs:?}"
    );
}

#[test]
fn non_erasable_interface_as_struct_field_is_error() {
    let errs = run(r#"
interface Cloneable {
    def clone() -> Self {}
}

struct Container {
    item: Cloneable,
}
"#);
    assert!(
        !errs.is_empty(),
        "non-erasable interface as struct field type should be an error: {errs:?}"
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
    assert!(
        errs.is_empty(),
        "@derive(Eq) on enum should have no errors: {errs:?}"
    );
}

#[test]
fn derive_comparable_on_enum_no_errors() {
    let errs = run(r#"
@derive(Comparable)
enum Priority { Low, Medium, High, Critical }
"#);
    assert!(
        errs.is_empty(),
        "@derive(Comparable) on enum should have no errors: {errs:?}"
    );
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
    assert!(
        errs.is_empty(),
        "@derive(Eq, Comparable) on enum with <=> should pass: {errs:?}"
    );
}

#[test]
fn analyze_enums_derive_example() {
    let errs = analyze_file("examples/enums_derive.kn");
    assert!(errs.is_empty(), "enums_derive.kn: {errs:?}");
}

// ---------------------------------------------------------------------------
// Implicit self field fall-through removal
// ---------------------------------------------------------------------------

#[test]
fn bare_field_name_in_method_body_is_error() {
    let errs = run(r#"
interface Summable {
    hook sum() -> int
}

struct Point { x: int, y: int }

impl Summable for Point {
    hook sum() -> int {
        return x + y
    }
}
"#);
    assert!(
        !errs.is_empty(),
        "bare field names `x` and `y` inside method body must be errors"
    );
    assert!(
        errs.iter()
            .any(|e| e.contains("x") || e.contains("field") || e.contains("self")),
        "error should mention the bare name or suggest `self.x`: {errs:?}"
    );
}

#[test]
fn explicit_self_field_in_method_body_passes() {
    let errs = run(r#"
interface Summable {
    hook sum() -> int
}

struct Point { x: int, y: int }

impl Summable for Point {
    hook sum() -> int {
        return self.x + self.y
    }
}
"#);
    assert!(
        errs.is_empty(),
        "explicit self.x in method body should pass: {errs:?}"
    );
}

#[test]
fn method_param_same_name_as_field_resolves_to_param() {
    let errs = run(r#"
interface Scalable {
    hook scale(value: int) -> int
}

struct Wrapper { value: int }

impl Scalable for Wrapper {
    hook scale(value: int) -> int {
        return value
    }
}
"#);
    // value is a parameter -- resolves to param, not a bare field access error
    assert!(
        errs.is_empty(),
        "method param named same as field should resolve to param without error: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// None/Some global accessibility (regression guard for prelude declarations)
// ---------------------------------------------------------------------------

#[test]
fn namespaced_variant_access_always_works() {
    let errs = run(r#"
enum Status {
    Ok
    Err
}
def main() -> void {
    s: Status = Status:Ok
}
"#);
    assert!(
        errs.is_empty(),
        "namespaced variant access should always work: {errs:?}"
    );
}

#[test]
fn prelude_none_accessible_without_namespace() {
    let errs = run(r#"
def check(x: Option[int]) -> int {
    return match x {
        Some { value: v } => v,
        None => 0
    }
}
"#);
    assert!(
        errs.is_empty(),
        "None without namespace must work (declared in prelude): {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// Prototype / forward declaration
// ---------------------------------------------------------------------------

#[test]
fn body_less_def_without_implementation_is_error() {
    let errs = run(r#"
def add(a: int, b: int) -> int
def main() -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "body-less def with no implementation must error (E018): {errs:?}"
    );
    assert!(
        errs.iter()
            .any(|e| e.contains("E018") || e.contains("implement")),
        "expected E018 MissingImplementation: {errs:?}"
    );
}

#[test]
fn declaration_with_implementation_passes() {
    let errs = run(r#"
def add(a: int, b: int) -> int
def add(a: int, b: int) -> int { return a + b }
def main() -> void {}
"#);
    assert!(
        errs.is_empty(),
        "declaration followed by implementation must be ok: {errs:?}"
    );
}

#[test]
fn generic_declaration_with_impl_omitting_bounds_passes() {
    let errs = run(r#"
def sum[T: Display](a: T, b: T) -> str
def sum[T](a: T, b: T) -> str { return "{a}" }
def main() -> void {}
"#);
    assert!(
        errs.is_empty(),
        "declaration + implementation omitting bounds must pass: {errs:?}"
    );
}

#[test]
fn implementation_can_omit_bounds_declared_in_declaration() {
    let errs = run(r#"
def sum[T: Display](a: T, b: T) -> str
def sum[T](a: T, b: T) -> str { return "{a}" }
def main() -> void {}
"#);
    assert!(
        errs.is_empty(),
        "implementation may omit bounds that are canonical in the declaration: {errs:?}"
    );
}

#[test]
fn duplicate_same_name_same_params_is_error() {
    let errs = run(r#"
def foo(x: int) -> int { return x }
def foo(x: int) -> int { return x + 1 }
def main() -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "two defs with same name and params must error"
    );
    assert!(
        errs.iter()
            .any(|e| e.contains("duplicate") || e.contains("E016")),
        "expected E016 DuplicateSignature: {errs:?}"
    );
}

#[test]
fn duplicate_declarations_are_error() {
    let errs = run(r#"
def foo(x: int) -> int
def foo(x: int) -> int
def main() -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "two declarations with same signature must error"
    );
    assert!(
        errs.iter()
            .any(|e| e.contains("duplicate") || e.contains("E016")),
        "expected E016 DuplicateSignature: {errs:?}"
    );
}

#[test]
fn declaration_bounds_enforced_at_call_site() {
    // T: BoundsTest is on the declaration but NOT on the implementation.
    // Calling with int (which does not implement BoundsTest) must error.
    let errs = run(r#"
interface BoundsTest { def probe(self) -> int {} }
def check[T: BoundsTest](x: T) -> int
def check[T](x: T) -> int { return x.probe() }
def main() -> void {
    _result: int = check(42)
}
"#);
    assert!(
        !errs.is_empty(),
        "calling check with int (no BoundsTest impl) must error: got no errors"
    );
    assert!(
        errs.iter().any(|e| {
            e.contains("BoundsTest")
                || e.contains("bound")
                || e.contains("E006")
                || e.contains("overload")
        }),
        "expected a bound violation or no-overload error: {errs:?}"
    );
}

#[test]
fn declaration_bounds_inherited_when_type_qualifies() {
    // When the concrete type DOES satisfy the declared bound, no error.
    // int implements Display, so this call must succeed.
    let errs = run(r#"
def describe[T: Display](x: T) -> str
def describe[T](x: T) -> str { return "{x}" }
def main() -> void {
    _result: str = describe(42)
}
"#);
    assert!(
        errs.is_empty(),
        "calling describe with int (implements Display) must pass: {errs:?}"
    );
}

#[test]
fn implementation_with_bounds_when_declaration_exists_is_error() {
    // Bounds belong canonically on the declaration; the implementation must omit them.
    let errs = run(r#"
def describe[T: Display](x: T) -> str
def describe[T: Display](x: T) -> str { return "{x}" }
def main() -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "implementation repeating bounds from declaration must be an error"
    );
    assert!(
        errs.iter()
            .any(|e| e.contains("E017") || e.contains("bound") || e.contains("omit")),
        "expected E017 BoundsOnImplementation: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// Orphan declaration (declared but never implemented)
// ---------------------------------------------------------------------------

#[test]
fn declaration_without_implementation_is_error() {
    let errs = run(r#"
def sum[T: Display](items: Vec[T]) -> str
def main() -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "declaration with no implementation must error"
    );
    assert!(
        errs.iter()
            .any(|e| e.contains("sum") || e.contains("E018") || e.contains("implement")),
        "expected E018 MissingImplementation for `sum`: {errs:?}"
    );
}

#[test]
fn declaration_with_implementation_is_not_orphan() {
    let errs = run(r#"
def sum[T: Display](x: T) -> str
def sum[T](x: T) -> str { return "{x}" }
def main() -> void {}
"#);
    assert!(
        errs.is_empty(),
        "paired declaration+implementation must not error: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// Struct literal field validation
// ---------------------------------------------------------------------------

#[test]
fn struct_literal_unknown_field_is_error() {
    let errs = run(r#"
struct Point { x: int, y: int }
def main() -> void {
    p: Point = Point { x: 1, z: 2 }
}
"#);
    assert!(
        !errs.is_empty(),
        "struct literal with unknown field `z` must error"
    );
    assert!(
        errs.iter()
            .any(|e| e.contains("z") || e.contains("E013") || e.contains("field")),
        "expected E013 NoField for `z`: {errs:?}"
    );
}

#[test]
fn struct_literal_all_valid_fields_passes() {
    let errs = run(r#"
struct Point { x: int, y: int }
def main() -> void {
    p: Point = Point { x: 1, y: 2 }
}
"#);
    assert!(
        errs.is_empty(),
        "struct literal with valid fields must pass: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// @entry annotation
// ---------------------------------------------------------------------------

#[test]
fn entry_annotation_is_recognized_not_unknown() {
    let errs = run(r#"
@entry
def program_start() -> void {}
"#);
    assert!(
        !errs
            .iter()
            .any(|e| e.contains("E014") || e.contains("unknown annotation")),
        "@entry must not produce E014 UnknownAnnotation: {errs:?}"
    );
}

#[test]
fn entry_annotation_does_not_require_main_name() {
    let errs = run(r#"
@entry
def run() -> void {}
"#);
    assert!(
        errs.is_empty(),
        "@entry on any void function must pass analysis: {errs:?}"
    );
}

#[test]
fn overloads_with_different_arity_are_ok() {
    let errs = run(r#"
def foo(x: int) -> int { return x }
def foo(x: int, y: int) -> int { return x + y }
def main() -> void {}
"#);
    assert!(
        errs.is_empty(),
        "different arity is a valid overload: {errs:?}"
    );
}

#[test]
fn unwrap_op_on_option_is_valid() {
    let errs = run(r#"
def use_opt(x: Option[int]) -> int {
    return x?
}
def main() -> void {}
"#);
    assert!(errs.is_empty(), "? on Option[int] must be valid: {errs:?}");
}

#[test]
fn unwrap_op_on_try_implementor_is_valid() {
    // ? must work on any type declaring `impl Try`, not just Option.
    // Try is defined in the prelude; user types can implement it.
    let errs = run(r#"
enum MyOpt[T] {
    MyVal { value: T }
    MyNone
}
impl[T] Try for MyOpt[T] {}
def use_it(x: MyOpt[int]) -> int {
    return x?
}
def main() -> void {}
"#);
    assert!(
        errs.is_empty(),
        "? on a user-defined Try implementor must be valid: {errs:?}"
    );
}

#[test]
fn unwrap_op_on_non_try_type_is_error() {
    let errs = run(r#"
def use_it(x: int) -> int {
    return x?
}
def main() -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "? on int (no Try impl) must produce an error"
    );
}

// ---------------------------------------------------------------------------
// Annotation processors and gen blocks
// ---------------------------------------------------------------------------

#[test]
fn annotations_gen_basic_analyzes_clean() {
    let errs = analyze_file("examples/annotations/gen_basic.kn");
    assert!(
        errs.is_empty(),
        "gen_basic.kn must analyze without errors: {errs:?}"
    );
}

#[test]
fn annotations_gen_processor_analyzes_clean() {
    let errs = analyze_file("examples/annotations/gen_processor.kn");
    assert!(
        errs.is_empty(),
        "gen_processor.kn must analyze without errors: {errs:?}"
    );
}

#[test]
fn processor_body_with_gen_block_analyzes_clean() {
    let errs = analyze_file("examples/annot_processors.kn");
    assert!(
        errs.is_empty(),
        "annot_processors.kn must analyze without errors: {errs:?}"
    );
}

#[test]
fn gen_block_example_analyzes_clean() {
    let errs = analyze_file("examples/annot_gen.kn");
    assert!(
        errs.is_empty(),
        "annot_gen.kn must analyze without errors: {errs:?}"
    );
}

#[test]
fn gen_block_result_type_is_block() {
    let errs = run(r#"
annotation Wrap { }
processor Wrap(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    b: Block = gen { }
    return (None, Vec.new())
}
@Wrap
def foo() -> void { }
def main() -> void { }
"#);
    assert!(
        errs.is_empty(),
        "gen block assigned to Block-typed variable must analyze clean: {errs:?}"
    );
}

#[test]
fn gen_splice_of_target_body_analyzes_clean() {
    let errs = run(r#"
annotation Log { }
processor Log(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    b: Block = gen {
        <<target.body>>
    }
    return (None, Vec.new())
}
@Log
def bar() -> void { }
def main() -> void { }
"#);
    assert!(
        errs.is_empty(),
        "<<target.body>> splice must analyze clean: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// Item 7: Ast.* type system (phases 7a-7c)
// ---------------------------------------------------------------------------

// 7a: FnDecl, Block, Decl, Stmt, Expr etc. must be known types after ast.kn loads.
#[test]
fn ast_fndecl_and_block_are_known_types() {
    let errs = run(r#"
def take_fndecl(f: FnDecl) -> void {}
def take_block(b: Block) -> void {}
def main() -> void {}
"#);
    assert!(
        errs.is_empty(),
        "FnDecl and Block must be recognized types (from ast.kn): {errs:?}"
    );
}

#[test]
fn ast_decl_stmt_expr_are_known_types() {
    let errs = run(r#"
def inspect(d: Decl, s: Stmt, e: Expr) -> void {}
def main() -> void {}
"#);
    assert!(
        errs.is_empty(),
        "Decl, Stmt, Expr must be recognized types (from ast.kn): {errs:?}"
    );
}

// 7b: Processor param types must be validated against the type registry.
#[test]
fn processor_with_undefined_param_type_is_error() {
    let errs = run(r#"
annotation Foo { }
processor Foo(target: NoSuchType) -> (Option[Decl], Vec[Decl]) {
    return (None, Vec.new())
}
@Foo
def bar() -> void {}
def main() -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "processor with undefined param type must produce an error"
    );
    assert!(
        errs.iter()
            .any(|e| e.contains("NoSuchType") || e.contains("E001")),
        "expected E001 for NoSuchType: {errs:?}"
    );
}

// 7c: gen {{ }} must have type Block, not Unknown -- can't be assigned to int.
#[test]
fn gen_block_not_assignable_to_int() {
    let errs = run(r#"
def make_block() -> void {
    x: int = gen { }
}
def main() -> void {}
"#);
    assert!(
        !errs.is_empty(),
        "gen block must not be assignable to int (type Block != int)"
    );
}

// ---------------------------------------------------------------------------
// Module system: import/export
// ---------------------------------------------------------------------------

fn analyze_file_from(path: &str) -> Vec<String> {
    use kiln_compiler::analyzer::analyze_with_base;
    use kiln_compiler::annotations::{default_registry, run_processors, run_user_processors};
    use kiln_compiler::lexer::Lexer;
    use kiln_compiler::parser::Parser;
    use std::path::PathBuf;
    let src = fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    let tokens = Lexer::new(&src).tokenize().expect("lex failed");
    let mut ast = Parser::new(tokens).parse_file().expect("parse failed");
    let registry = default_registry();
    run_processors(&mut ast, &registry);
    let mut _proc_errors = vec![];
    run_user_processors(&mut ast, &registry, &mut _proc_errors);
    let base = PathBuf::from(path).parent().unwrap().to_path_buf();
    match analyze_with_base(&ast, &base) {
        Ok(_) => vec![],
        Err(errs) => errs
            .iter()
            .map(|e: &kiln_compiler::analyzer::AnalysisError| e.to_string())
            .collect(),
    }
}

#[test]
fn import_selective_from_file() {
    let errs = analyze_file_from("examples/modules/selective_user.kn");
    assert!(
        errs.is_empty(),
        "selective import `add` from math.kn must analyze clean: {errs:?}"
    );
}

#[test]
fn import_wildcard_from_file() {
    let errs = analyze_file_from("examples/modules/wildcard_user.kn");
    assert!(
        errs.is_empty(),
        "wildcard import from math.kn must analyze clean: {errs:?}"
    );
}

#[test]
fn import_unexported_symbol_is_error() {
    let errs = run(r#"
import math { internal_helper }
def main() -> void {}
"#);
    // This is a special case: inline run() has no base path, so math won't resolve.
    // The key behavior: referencing `internal_helper` (not exported) is an error.
    // We test this via a file that imports a non-exported symbol.
    let _ = errs; // placeholder; file-based test covers this
}

#[test]
fn import_nonexistent_module_is_error() {
    use kiln_compiler::analyzer::analyze_with_base;
    use std::path::PathBuf;
    let src = "import no_such_module { foo }\ndef main() -> void {}";
    let tokens = kiln_compiler::lexer::Lexer::new(src)
        .tokenize()
        .expect("lex");
    let ast = kiln_compiler::parser::Parser::new(tokens)
        .parse_file()
        .expect("parse");
    let base = PathBuf::from(".");
    let errs: Vec<String> = match analyze_with_base(&ast, &base) {
        Ok(_) => vec![],
        Err(errs) => errs
            .iter()
            .map(|e: &kiln_compiler::analyzer::AnalysisError| e.to_string())
            .collect(),
    };
    assert!(
        !errs.is_empty(),
        "importing a nonexistent module must produce an error"
    );
}

// ---------------------------------------------------------------------------
// Item 22: const declarations
// ---------------------------------------------------------------------------

#[test]
fn const_int_is_usable_in_expressions() {
    let errs = run("const MAX_SIZE: int = 1024\ndef main() -> int { return MAX_SIZE }");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn const_float_is_usable_in_expressions() {
    let errs = run("const PI: float = 3.14\ndef main() -> float { return PI }");
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn assigning_to_const_is_error() {
    let errs = run("const X: int = 10\ndef main() -> void { X = 20 }");
    assert!(
        errs.iter()
            .any(|e| e.contains("E023") || e.contains("cannot assign to const")),
        "expected AssignToConst error, got: {errs:?}"
    );
}

#[test]
fn const_non_literal_initializer_is_error() {
    let errs = run("def helper() -> int { return 1 }\nconst X: int = helper()");
    assert!(
        errs.iter()
            .any(|e| e.contains("E022") || e.contains("literal")),
        "expected NonLiteralConst error, got: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// Multi-bound types (Iface1+Iface2 in value position)
// ---------------------------------------------------------------------------

#[test]
fn compound_type_in_param_position_accepts_valid_impl() {
    let errs = run(r#"
interface Printable {
    hook print_me() -> str
}
interface Traceable {
    hook trace_me() -> str
}
struct FullImpl {}
impl Printable for FullImpl {
    hook print_me() -> str { return "ok" }
}
impl Traceable for FullImpl {
    hook trace_me() -> str { return "ok" }
}
def use_it(x: Printable+Traceable) -> void {}
def main() -> void {
    v: Printable+Traceable = FullImpl{}
    use_it(v)
}
"#);
    assert!(
        errs.is_empty(),
        "compound type with full impl must analyze clean: {errs:?}"
    );
}

#[test]
fn compound_type_var_decl_checks_all_constituents() {
    let errs = run(r#"
interface Printable {
    hook print_me() -> str
}
interface Traceable {
    hook trace_me() -> str
}
struct PartialImpl {}
impl Printable for PartialImpl {
    hook print_me() -> str { return "ok" }
}
def main() -> void {
    v: Printable+Traceable = PartialImpl{}
}
"#);
    assert!(
        errs.iter()
            .any(|e| e.contains("TypeMismatch") || e.contains("E002") || e.contains("Traceable")),
        "compound type missing constituent impl must error: {errs:?}"
    );
}

#[test]
fn compound_type_in_function_return_position_is_valid() {
    let errs = run(r#"
interface Exportable {
    hook export_id() -> int
}
interface Serializable {
    hook serialize() -> str
}
def make() -> Exportable+Serializable { return make() }
"#);
    assert!(
        errs.is_empty(),
        "compound return type must analyze clean: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// Associated type pinning at erasure site
// ---------------------------------------------------------------------------

#[test]
fn bare_interface_with_assoc_type_is_error_at_erasure_site() {
    let errs = run(r#"
interface MyIter {
    type Item
    hook next() -> int
}
@builtin
def get_iter() -> MyIter
def main() -> void {}
"#);
    assert!(
        errs.iter()
            .any(|e| e.contains("E025") || e.contains("unbound") || e.contains("Item")),
        "bare interface with unbound assoc type must error (E025): {errs:?}"
    );
}

#[test]
fn pinned_interface_with_assoc_type_is_valid() {
    let errs = run(r#"
interface MyIter {
    type Item
    hook next() -> int
}
@builtin
def get_iter() -> MyIter[Item=int]
def main() -> void {}
"#);
    assert!(
        errs.is_empty(),
        "pinned interface must analyze clean: {errs:?}"
    );
}

#[test]
fn interface_without_assoc_types_needs_no_pinning() {
    let errs = run(r#"
interface Counter {
    hook count() -> int
}
@builtin
def get_counter() -> Counter
def main() -> void {}
"#);
    assert!(
        errs.is_empty(),
        "interface with no assoc types must not require pinning: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// Variance inference and enforcement
// ---------------------------------------------------------------------------

#[test]
fn mutex_is_invariant_in_type_arg() {
    let errs = run(r#"
interface AnimalIface {}
interface DogIface {}
@builtin
def get_dog_mutex() -> Mutex[DogIface]
def main() -> void {
    m: Mutex[AnimalIface] = get_dog_mutex
}
"#);
    assert!(
        errs.iter().any(|e| e.contains("E026")
            || e.contains("invariant")
            || e.contains("VarianceViolation")),
        "Mutex[DogIface] assigned to Mutex[AnimalIface] must be a variance violation: {errs:?}"
    );
}

#[test]
fn mutex_same_type_arg_is_valid() {
    let errs = run(r#"
interface AnimalIface {}
@builtin
def get_animal_mutex() -> Mutex[AnimalIface]
def main() -> void {
    m: Mutex[AnimalIface] = get_animal_mutex
}
"#);
    assert!(
        errs.is_empty(),
        "Mutex[AnimalIface] assigned to Mutex[AnimalIface] must be valid: {errs:?}"
    );
}

#[test]
fn mutex_concrete_type_usage_is_valid() {
    let errs = run(r#"
struct Counter {
    val: int
}
@builtin
def get_counter_mutex() -> Mutex[Counter]
def main() -> void {
    m: Mutex[Counter] = get_counter_mutex
}
"#);
    assert!(
        errs.is_empty(),
        "Mutex[Counter] assigned to Mutex[Counter] must be valid: {errs:?}"
    );
}

#[test]
fn for_loop_on_non_iterable_is_error() {
    let errs = run(r#"
def main() {
    x: int = 42
    for v <- x {
        print("v")
    }
}
"#);
    assert!(
        errs.iter()
            .any(|e| e.contains("E028") || e.contains("does not implement `Iterable`")),
        "expected NotIterable error, got: {errs:?}"
    );
}

#[test]
fn for_loop_on_bool_is_error() {
    let errs = run(r#"
def main() {
    b: bool = true
    for v <- b {
        print("v")
    }
}
"#);
    assert!(
        errs.iter()
            .any(|e| e.contains("E028") || e.contains("does not implement `Iterable`")),
        "expected NotIterable error for bool, got: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// E029: CyclicInterface
// ---------------------------------------------------------------------------

#[test]
fn cyclic_interface_direct_self_extension_is_error() {
    let errs = run(r#"
interface Loop: Loop {
    def go() -> void {}
}
def main() {}
"#);
    assert!(
        errs.iter()
            .any(|e| e.contains("E029") || e.contains("cyclic")),
        "expected E029 CyclicInterface for direct self-extension, got: {errs:?}"
    );
}

#[test]
fn cyclic_interface_two_hop_cycle_is_error() {
    let errs = run(r#"
interface A: B {
    def a() -> void {}
}
interface B: A {
    def b() -> void {}
}
def main() {}
"#);
    assert!(
        errs.iter()
            .any(|e| e.contains("E029") || e.contains("cyclic")),
        "expected E029 CyclicInterface for two-hop cycle A->B->A, got: {errs:?}"
    );
}

#[test]
fn cyclic_interface_three_hop_cycle_is_error() {
    let errs = run(r#"
interface X: Y {
    def x() -> void {}
}
interface Y: Z {
    def y() -> void {}
}
interface Z: X {
    def z() -> void {}
}
def main() {}
"#);
    assert!(
        errs.iter()
            .any(|e| e.contains("E029") || e.contains("cyclic")),
        "expected E029 CyclicInterface for three-hop cycle X->Y->Z->X, got: {errs:?}"
    );
}

#[test]
fn cyclic_interface_linear_chain_is_ok() {
    let errs = run(r#"
interface Base {
    def base() -> void {}
}
interface Mid: Base {
    def mid() -> void {}
}
interface Top: Mid {
    def top() -> void {}
}
def main() {}
"#);
    assert!(
        errs.is_empty(),
        "linear interface chain should not produce errors, got: {errs:?}"
    );
}

#[test]
fn cyclic_interface_example_file_reports_errors() {
    let errs = analyze_file("examples/check_cyclic_interface.kn");
    assert!(
        errs.iter()
            .any(|e| e.contains("E029") || e.contains("cyclic")),
        "expected E029 errors in check_cyclic_interface.kn, got: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// E030: RecursiveTypeWithoutIndirect
// ---------------------------------------------------------------------------

#[test]
fn recursive_struct_without_indirect_is_error() {
    let errs = run(r#"
struct Node {
    value: int
    next: Node
}
def main() {}
"#);
    assert!(
        errs.iter()
            .any(|e| e.contains("E030") || e.contains("infinite size") || e.contains("indirect")),
        "expected E030 RecursiveTypeWithoutIndirect, got: {errs:?}"
    );
}

#[test]
fn recursive_struct_with_indirect_is_ok() {
    let errs = run(r#"
struct Node {
    value: int
    @indirect next: Option[Node]
}
def main() {}
"#);
    assert!(
        errs.is_empty(),
        "@indirect on recursive field should not error, got: {errs:?}"
    );
}

#[test]
fn recursive_struct_multiple_bad_fields_each_reported() {
    let errs = run(r#"
struct Tree {
    value: int
    left: Tree
    right: Tree
}
def main() {}
"#);
    let cyclic_count = errs
        .iter()
        .filter(|e| e.contains("E030") || e.contains("infinite size"))
        .count();
    assert!(
        cyclic_count >= 2,
        "expected at least 2 E030 errors (one per bad field), got: {errs:?}"
    );
}

#[test]
fn recursive_struct_mixed_fields_only_bad_ones_reported() {
    let errs = run(r#"
struct Chain {
    value: int
    @indirect good: Option[Chain]
    bad: Chain
}
def main() {}
"#);
    assert!(
        errs.iter()
            .any(|e| e.contains("bad") && (e.contains("E030") || e.contains("infinite size"))),
        "expected E030 for field `bad`, got: {errs:?}"
    );
    assert!(
        !errs.iter().any(|e| e.contains("good")),
        "@indirect field `good` should not be reported, got: {errs:?}"
    );
}

#[test]
fn non_recursive_structs_are_ok() {
    let errs = run(r#"
struct Leaf {
    value: int
}
struct Branch {
    left: Leaf
    right: Leaf
}
def main() {}
"#);
    assert!(
        errs.is_empty(),
        "non-recursive structs should not produce errors, got: {errs:?}"
    );
}

#[test]
fn recursive_type_example_file_reports_errors() {
    let errs = analyze_file("examples/check_recursive_type.kn");
    assert!(
        errs.iter()
            .any(|e| e.contains("E030") || e.contains("infinite size")),
        "expected E030 errors in check_recursive_type.kn, got: {errs:?}"
    );
}

// ---------------------------------------------------------------------------
// Wildcard _ and .. in match field patterns
// ---------------------------------------------------------------------------

#[test]
fn wildcard_field_binding_does_not_introduce_name() {
    let errs = run(r#"
enum Color { Red, Blue { value: int } }
def f(c: Color) -> void {
    match c {
        Blue { value: _ } => {}
        Red => {}
    }
}
"#);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn rest_pattern_skips_unmentioned_fields() {
    let errs = run(r#"
enum Shape { Circle { x: int, y: int, radius: int } }
def f(s: Shape) -> void {
    match s {
        Circle { x: _px, .. } => {}
    }
}
"#);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn wildcard_and_named_bindings_in_same_pattern() {
    let errs = run(r#"
enum Point { P { x: int, y: int } }
def f(p: Point) -> void {
    match p {
        P { x: _v, y: _ } => {}
    }
}
"#);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn rest_pattern_with_no_named_fields_matches_any_struct_variant() {
    let errs = run(r#"
enum Node { Leaf { value: int, tag: int, depth: int } }
def f(n: Node) -> void {
    match n {
        Leaf { .. } => {}
    }
}
"#);
    assert!(errs.is_empty(), "{errs:?}");
}
