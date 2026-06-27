use kiln_compiler::{
    analyzer::analyze_with_base_and_registry,
    annotations::{default_registry, run_processors, run_source_processors, run_user_processors},
    codegen::{compile::compile, context::CodegenContext, emit},
    lexer::Lexer,
    parser::Parser,
    test_harness::inject_harness,
};
use std::path::PathBuf;

fn run_kiln_tests(path: &str) {
    let file = PathBuf::from(path);
    let src =
        std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));

    let tokens = Lexer::new(&src)
        .tokenize()
        .unwrap_or_else(|e| panic!("lex error in {path}: {e:?}"));

    let mut ast = Parser::new(tokens)
        .parse_file()
        .unwrap_or_else(|e| panic!("parse error in {path}: {e:?}"));

    let registry = default_registry();
    run_source_processors(&mut ast, &registry);
    let mut proc_errors = vec![];
    run_user_processors(&mut ast, &mut proc_errors);
    assert!(
        proc_errors.is_empty(),
        "processor errors in {path}: {proc_errors:?}"
    );

    inject_harness(&mut ast);

    let base = file
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let (mut typed, type_registry, warns) = analyze_with_base_and_registry(&ast, &base)
        .unwrap_or_else(|errs| panic!("analyze errors in {path}: {errs:?}"));
    assert!(warns.is_empty(), "analysis warnings in {path}: {warns:?}");

    run_processors(&ast, &mut typed, &registry);

    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("test");
    let mut cgx = CodegenContext::new(stem);
    compile(&typed, &mut cgx, false, 3, &type_registry)
        .unwrap_or_else(|e| panic!("codegen error in {path}: {e}"));

    let obj_bytes = emit::emit_object(cgx).unwrap_or_else(|e| panic!("emit error in {path}: {e}"));

    let tmp = std::env::temp_dir();
    let obj_path = tmp.join(format!("kiln_test_{stem}.o"));
    let exe_path = tmp.join(format!("kiln_test_{stem}.exe"));

    emit::link_executable(&obj_bytes, &obj_path, &exe_path, false)
        .unwrap_or_else(|e| panic!("link error in {path}: {e}"));
    let _ = std::fs::remove_file(&obj_path);

    let status = std::process::Command::new(&exe_path)
        .status()
        .unwrap_or_else(|e| panic!("run error for {path}: {e}"));
    let _ = std::fs::remove_file(&exe_path);

    assert!(status.success(), "kiln tests failed in {path}");
}

#[test]
fn stdlib_string_methods() {
    run_kiln_tests("examples/stdlib/string_methods.kn");
}

#[test]
fn stdlib_math() {
    run_kiln_tests("examples/stdlib/math_test.kn");
}

#[test]
fn stdlib_collections() {
    run_kiln_tests("examples/stdlib/collections_test.kn");
}

#[test]
fn stdlib_result() {
    run_kiln_tests("examples/stdlib/result_test.kn");
}

#[test]
fn stdlib_fileio() {
    run_kiln_tests("examples/stdlib/fileio_test.kn");
}
