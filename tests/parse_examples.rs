use kiln_compiler::lexer::Lexer;
use kiln_compiler::parser::Parser;
use std::fs;

fn parse_file(path: &str) {
    let src = fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    let tokens = Lexer::new(&src)
        .tokenize()
        .unwrap_or_else(|errors| panic!("lex errors in {path}: {errors:?}"));
    Parser::new(tokens)
        .parse_file()
        .unwrap_or_else(|e| panic!("parse error in {path}: {e:?}"));
}

// ---- hello -------------------------------------------------------------------

#[test]
fn parse_hello() {
    parse_file("examples/hello.kn");
}

// ---- variables and primitives ------------------------------------------------

#[test]
fn parse_vars() {
    parse_file("examples/vars.kn");
}

#[test]
fn parse_strings() {
    parse_file("examples/strings.kn");
}

#[test]
fn parse_operators() {
    parse_file("examples/operators.kn");
}

#[test]
fn parse_control_flow() {
    parse_file("examples/control_flow.kn");
}

#[test]
fn parse_loops() {
    parse_file("examples/loops.kn");
}

// ---- functions ---------------------------------------------------------------

#[test]
fn parse_fn_basic() {
    parse_file("examples/fn_basic.kn");
}

#[test]
fn parse_fn_generic() {
    parse_file("examples/fn_generic.kn");
}

#[test]
fn parse_fn_variadic() {
    parse_file("examples/fn_variadic.kn");
}

#[test]
fn parse_fn_nested() {
    parse_file("examples/fn_nested.kn");
}

#[test]
fn parse_fn_recursive() {
    parse_file("examples/fn_recursive.kn");
}

#[test]
fn parse_fn_callable() {
    parse_file("examples/fn_callable.kn");
}

// ---- closures ----------------------------------------------------------------

#[test]
fn parse_closures_basic() {
    parse_file("examples/closures_basic.kn");
}

#[test]
fn parse_closures_capture() {
    parse_file("examples/closures_capture.kn");
}

#[test]
fn parse_closures_hof() {
    parse_file("examples/closures_hof.kn");
}

// ---- enums -------------------------------------------------------------------

#[test]
fn parse_enums_basic() {
    parse_file("examples/enums_basic.kn");
}

#[test]
fn parse_enums_fields() {
    parse_file("examples/enums_fields.kn");
}

#[test]
fn parse_enums_generic() {
    parse_file("examples/enums_generic.kn");
}

#[test]
fn parse_enums_interfaces() {
    parse_file("examples/enums_interfaces.kn");
}

// ---- structs -----------------------------------------------------------------

#[test]
fn parse_structs_basic() {
    parse_file("examples/structs_basic.kn");
}

#[test]
fn parse_structs_methods() {
    parse_file("examples/structs_methods.kn");
}

#[test]
fn parse_structs_interfaces() {
    parse_file("examples/structs_interfaces.kn");
}

#[test]
fn parse_structs_indirect() {
    parse_file("examples/structs_indirect.kn");
}

// ---- interfaces --------------------------------------------------------------

#[test]
fn parse_iface_basic() {
    parse_file("examples/iface_basic.kn");
}

#[test]
fn parse_iface_hooks() {
    parse_file("examples/iface_hooks.kn");
}

#[test]
fn parse_iface_impl() {
    parse_file("examples/iface_impl.kn");
}

#[test]
fn parse_iface_hierarchy() {
    parse_file("examples/iface_hierarchy.kn");
}

#[test]
fn parse_iface_runtime() {
    parse_file("examples/iface_runtime.kn");
}

#[test]
fn parse_iface_operator_bounds() {
    parse_file("examples/iface_operator_bounds.kn");
}

#[test]
fn parse_iface_generic_bounds() {
    parse_file("examples/iface_generic_bounds.kn");
}

#[test]
fn parse_iface_numeric_hierarchy() {
    parse_file("examples/iface_numeric_hierarchy.kn");
}

#[test]
fn parse_iface_conditional_impl() {
    parse_file("examples/iface_conditional_impl.kn");
}

// ---- patterns ----------------------------------------------------------------

#[test]
fn parse_patterns_literal() {
    parse_file("examples/patterns_literal.kn");
}

#[test]
fn parse_patterns_binding() {
    parse_file("examples/patterns_binding.kn");
}

#[test]
fn parse_patterns_enum() {
    parse_file("examples/patterns_enum.kn");
}

#[test]
fn parse_patterns_tuple() {
    parse_file("examples/patterns_tuple.kn");
}

#[test]
fn parse_patterns_guards() {
    parse_file("examples/patterns_guards.kn");
}

#[test]
fn parse_patterns_option() {
    parse_file("examples/patterns_option.kn");
}

// ---- concurrency -------------------------------------------------------------

#[test]
fn parse_concurrency_tasks() {
    parse_file("examples/concurrency_tasks.kn");
}

#[test]
fn parse_concurrency_atomic() {
    parse_file("examples/concurrency_atomic.kn");
}

#[test]
fn parse_concurrency_mutex() {
    parse_file("examples/concurrency_mutex.kn");
}

#[test]
fn parse_concurrency_rwlock() {
    parse_file("examples/concurrency_rwlock.kn");
}

// ---- error handling ----------------------------------------------------------

#[test]
fn parse_errors_raise() {
    parse_file("examples/errors_raise.kn");
}

#[test]
fn parse_errors_try_except() {
    parse_file("examples/errors_try_except.kn");
}

#[test]
fn parse_errors_custom() {
    parse_file("examples/errors_custom.kn");
}

#[test]
fn parse_errors_hierarchy() {
    parse_file("examples/errors_hierarchy.kn");
}

// ---- types -------------------------------------------------------------------

#[test]
fn parse_types_primitives() {
    parse_file("examples/types_primitives.kn");
}

#[test]
fn parse_types_generics() {
    parse_file("examples/types_generics.kn");
}

#[test]
fn parse_types_union() {
    parse_file("examples/types_union.kn");
}

#[test]
fn parse_types_callable() {
    parse_file("examples/types_callable.kn");
}

// ---- memory ------------------------------------------------------------------

#[test]
fn parse_mem_copy_move() {
    parse_file("examples/mem_copy_move.kn");
}

#[test]
fn parse_mem_references() {
    parse_file("examples/mem_references.kn");
}

#[test]
fn parse_mem_shared() {
    parse_file("examples/mem_shared.kn");
}

#[test]
fn parse_mem_drop() {
    parse_file("examples/mem_drop.kn");
}

#[test]
fn parse_mem_indirect() {
    parse_file("examples/mem_indirect.kn");
}

// ---- memory (new) ------------------------------------------------------------

#[test]
fn parse_mem_lifetimes() {
    parse_file("examples/mem_lifetimes.kn");
}

#[test]
fn parse_mem_weak() {
    parse_file("examples/mem_weak.kn");
}

// ---- annotations -------------------------------------------------------------

#[test]
fn parse_annot_basic() {
    parse_file("examples/annot_basic.kn");
}

#[test]
fn parse_annot_builtin() {
    parse_file("examples/annot_builtin.kn");
}

#[test]
fn parse_annot_processors() {
    parse_file("examples/annot_processors.kn");
}

#[test]
fn parse_annot_test() {
    parse_file("examples/annot_test.kn");
}

#[test]
fn parse_annot_gen() {
    parse_file("examples/annot_gen.kn");
}

// ---- modules -----------------------------------------------------------------

#[test]
fn parse_modules() {
    parse_file("examples/modules.kn");
}

// ---- error (single-file) -----------------------------------------------------

#[test]
fn parse_error() {
    parse_file("examples/error.kn");
}

// ---- check examples ----------------------------------------------------------

#[test]
fn parse_check_valid() {
    parse_file("examples/check_valid.kn");
}

#[test]
fn parse_check_valid_complex() {
    parse_file("examples/check_valid_complex.kn");
}

#[test]
fn parse_check_conformance() {
    parse_file("examples/check_conformance.kn");
}

#[test]
fn parse_check_duplicate_names() {
    parse_file("examples/check_duplicate_names.kn");
}

#[test]
fn parse_check_immutability() {
    parse_file("examples/check_immutability.kn");
}

#[test]
fn parse_check_missing_return() {
    parse_file("examples/check_missing_return.kn");
}

#[test]
fn parse_check_mixed() {
    parse_file("examples/check_mixed.kn");
}

#[test]
fn parse_check_return_paths() {
    parse_file("examples/check_return_paths.kn");
}

#[test]
fn parse_check_scope() {
    parse_file("examples/check_scope.kn");
}

#[test]
fn parse_check_type_mismatch() {
    parse_file("examples/check_type_mismatch.kn");
}

#[test]
fn parse_check_undefined() {
    parse_file("examples/check_undefined.kn");
}

// ---- interface dispatch ------------------------------------------------------

#[test]
fn parse_iface_dispatch() {
    parse_file("examples/iface_dispatch.kn");
}

// ---- mono --------------------------------------------------------------------

#[test]
fn parse_mono_identity() {
    parse_file("examples/mono_identity.kn");
}

// ---- tricky parse cases ------------------------------------------------------

#[test]
fn parse_tricky_precedence() {
    parse_file("examples/tricky_precedence.kn");
}

#[test]
fn parse_tricky_ambiguity() {
    parse_file("examples/tricky_ambiguity.kn");
}

#[test]
fn parse_tricky_postfix() {
    parse_file("examples/tricky_postfix.kn");
}

#[test]
fn parse_tricky_chaining() {
    parse_file("examples/tricky_chaining.kn");
}

#[test]
fn parse_tricky_interp() {
    parse_file("examples/tricky_interp.kn");
}

#[test]
fn parse_tricky_types() {
    parse_file("examples/tricky_types.kn");
}

#[test]
fn parse_tricky_calls() {
    parse_file("examples/tricky_calls.kn");
}

#[test]
fn parse_tricky_closures() {
    parse_file("examples/tricky_closures.kn");
}

#[test]
fn parse_tricky_match() {
    parse_file("examples/tricky_match.kn");
}

#[test]
fn parse_tricky_control() {
    parse_file("examples/tricky_control.kn");
}

// ---- fancy-interfaces --------------------------------------------------------

#[test]
fn parse_fancy_layer1_arithmetic() {
    parse_file("examples/fancy-interfaces/layer1_arithmetic.kn");
}

#[test]
fn parse_fancy_layer1_comparison() {
    parse_file("examples/fancy-interfaces/layer1_comparison.kn");
}

#[test]
fn parse_fancy_layer1_assign() {
    parse_file("examples/fancy-interfaces/layer1_assign.kn");
}

#[test]
fn parse_fancy_layer1_unary() {
    parse_file("examples/fancy-interfaces/layer1_unary.kn");
}

#[test]
fn parse_fancy_layer1_indexing() {
    parse_file("examples/fancy-interfaces/layer1_indexing.kn");
}

#[test]
fn parse_fancy_layer1_callable_iter() {
    parse_file("examples/fancy-interfaces/layer1_callable_iter.kn");
}

#[test]
fn parse_fancy_layer1_identity() {
    parse_file("examples/fancy-interfaces/layer1_identity.kn");
}

#[test]
fn parse_fancy_layer2_shorthands() {
    parse_file("examples/fancy-interfaces/layer2_shorthands.kn");
}

#[test]
fn parse_fancy_layer3_semantic() {
    parse_file("examples/fancy-interfaces/layer3_semantic.kn");
}

#[test]
fn parse_fancy_layer3_collection() {
    parse_file("examples/fancy-interfaces/layer3_collection.kn");
}

#[test]
fn parse_fancy_blanket_impls() {
    parse_file("examples/fancy-interfaces/blanket_impls.kn");
}

#[test]
fn parse_fancy_specialized_impls() {
    parse_file("examples/fancy-interfaces/specialized_impls.kn");
}

#[test]
fn parse_fancy_extension_impls() {
    parse_file("examples/fancy-interfaces/extension_impls.kn");
}

#[test]
fn parse_fancy_assoc_types() {
    parse_file("examples/fancy-interfaces/assoc_types.kn");
}

#[test]
fn parse_fancy_hkt() {
    parse_file("examples/fancy-interfaces/hkt.kn");
}

#[test]
fn parse_fancy_dispatch() {
    parse_file("examples/fancy-interfaces/dispatch.kn");
}

// ---- iteration ---------------------------------------------------------------

#[test]
fn parse_iteration_enum_iter_basic() {
    parse_file("examples/iteration/enum_iter_basic.kn");
}

#[test]
fn parse_iteration_enum_iter_use() {
    parse_file("examples/iteration/enum_iter_use.kn");
}
