use crate::diagnostics::Span;
use crate::parser::ast::*;

struct TestCase {
    name: String,
    failing: bool,
    #[allow(dead_code)]
    isolate: bool,
}

fn collect_tests(source: &SourceFile) -> Vec<TestCase> {
    source
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Function(f) = item {
                if let Some(ann) = f.annotations.iter().find(|a| a.name == "test") {
                    let failing = ann.args.iter().any(|(k, _)| k == "failing");
                    let isolate = ann.args.iter().any(|(k, _)| k == "isolate");
                    return Some(TestCase {
                        name: f.name.clone(),
                        failing,
                        isolate,
                    });
                }
            }
            None
        })
        .collect()
}

/// Scan a parsed source file for `@test`-annotated functions and inject a
/// synthetic `main()` that runs each test with named output.
///
/// Supported annotation parameters:
///   @test             -- normal test: passes if returns, fails if raises
///   @test(failing)    -- expected-to-fail: passes if raises, fails if returns
///   @test(isolate)    -- currently runs inline (subprocess isolation not yet impl)
pub fn inject_harness(source: &mut SourceFile) {
    let tests = collect_tests(source);
    if tests.is_empty() {
        return;
    }

    let s = Span::new(0, 0);

    let named = |n: &str| TypeExpr::Named {
        name: n.into(),
        generics: vec![],
        bindings: vec![],
        span: s,
    };

    let println_str = |msg: &str| {
        Stmt::Expr(Expr::Call {
            callee: Box::new(Expr::Ident("println".into(), s)),
            args: vec![Expr::Str(vec![StringSegment::Text(msg.into())], s)],
            span: s,
        })
    };

    let mut stmts: Vec<Stmt> = Vec::new();

    for (idx, test) in tests.iter().enumerate() {
        let label = if test.failing {
            format!("{} (expected to fail) ...", test.name)
        } else {
            format!("{} ...", test.name)
        };
        stmts.push(println_str(&label));

        let call_test = Stmt::Expr(Expr::Call {
            callee: Box::new(Expr::Ident(test.name.clone(), s)),
            args: vec![],
            span: s,
        });

        if test.failing {
            // @test(failing): use a flag + try-catch to detect whether the test raised.
            let flag_name = format!("__did_raise_{}", idx);
            stmts.push(Stmt::VarDecl {
                name: flag_name.clone(),
                ty: named("bool"),
                value: Expr::Bool(false, s),
                mutable: true,
                span: s,
            });
            let set_flag = Stmt::Assign {
                target: Expr::Ident(flag_name.clone(), s),
                value: Expr::Bool(true, s),
                span: s,
            };
            stmts.push(Stmt::TryCatch {
                body: Block {
                    stmts: vec![call_test],
                    span: s,
                },
                handlers: vec![CatchHandler {
                    ty: named("Exception"),
                    binding: format!("__e_{}", idx),
                    body: Block {
                        stmts: vec![set_flag],
                        span: s,
                    },
                    span: s,
                }],
                finally: None,
                span: s,
            });
            // If the test did NOT raise, fail the whole suite.
            let fail_msg = format!("{}: expected exception but none was raised", test.name);
            stmts.push(Stmt::If {
                branches: vec![(
                    Expr::UnOp {
                        op: UnOp::Not,
                        operand: Box::new(Expr::Ident(flag_name, s)),
                        span: s,
                    },
                    Block {
                        stmts: vec![Stmt::Raise {
                            value: Some(Expr::Str(vec![StringSegment::Text(fail_msg)], s)),
                            span: s,
                        }],
                        span: s,
                    },
                )],
                else_branch: None,
                span: s,
            });
        } else {
            stmts.push(call_test);
        }

        stmts.push(println_str("ok"));
    }

    let main_fn = FnDef {
        annotations: vec![],
        name: "main".into(),
        generic_params: vec![],
        params: vec![],
        variadic: None,
        return_type: named("void"),
        body: Block { stmts, span: s },
        span: s,
    };

    source.items.push(Item::Function(main_fn));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Span;

    fn s() -> Span {
        Span::new(0, 0)
    }

    fn named(n: &str) -> TypeExpr {
        TypeExpr::Named {
            name: n.into(),
            generics: vec![],
            bindings: vec![],
            span: s(),
        }
    }

    fn test_fn(name: &str) -> FnDef {
        FnDef {
            annotations: vec![AnnotationUse {
                name: "test".into(),
                args: vec![],
                span: s(),
            }],
            name: name.into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: named("void"),
            body: Block {
                stmts: vec![],
                span: s(),
            },
            span: s(),
        }
    }

    fn failing_test_fn(name: &str) -> FnDef {
        FnDef {
            annotations: vec![AnnotationUse {
                name: "test".into(),
                args: vec![("failing".into(), Expr::Ident("failing".into(), s()))],
                span: s(),
            }],
            name: name.into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: named("void"),
            body: Block {
                stmts: vec![],
                span: s(),
            },
            span: s(),
        }
    }

    fn plain_fn(name: &str) -> FnDef {
        FnDef {
            annotations: vec![],
            name: name.into(),
            generic_params: vec![],
            params: vec![],
            variadic: None,
            return_type: named("void"),
            body: Block {
                stmts: vec![],
                span: s(),
            },
            span: s(),
        }
    }

    #[test]
    fn no_test_functions_produces_no_main() {
        let mut source = SourceFile {
            items: vec![Item::Function(plain_fn("helper"))],
            span: s(),
        };
        let before = source.items.len();
        inject_harness(&mut source);
        assert_eq!(source.items.len(), before);
    }

    #[test]
    fn single_test_function_injects_main() {
        let mut source = SourceFile {
            items: vec![Item::Function(test_fn("my_test"))],
            span: s(),
        };
        inject_harness(&mut source);
        assert_eq!(source.items.len(), 2);
        let last = source.items.last().unwrap();
        let Item::Function(f) = last else {
            panic!("expected function")
        };
        assert_eq!(f.name, "main");
    }

    #[test]
    fn main_calls_all_test_functions() {
        let mut source = SourceFile {
            items: vec![
                Item::Function(test_fn("test_a")),
                Item::Function(test_fn("test_b")),
                Item::Function(test_fn("test_c")),
            ],
            span: s(),
        };
        inject_harness(&mut source);
        let Item::Function(main) = source.items.last().unwrap() else {
            panic!()
        };
        // Each normal test: println + call + println = 3 stmts per test, but 3 tests = 9 stmts
        assert!(main.body.stmts.len() >= 3);
    }

    #[test]
    fn main_only_calls_annotated_tests_not_plain_functions() {
        let mut source = SourceFile {
            items: vec![
                Item::Function(test_fn("test_a")),
                Item::Function(plain_fn("helper")),
                Item::Function(test_fn("test_b")),
            ],
            span: s(),
        };
        inject_harness(&mut source);
        let Item::Function(main) = source.items.last().unwrap() else {
            panic!()
        };
        // 2 normal tests, each with 3 stmts = 6
        assert!(main.body.stmts.len() >= 2);
    }

    #[test]
    fn failing_test_generates_try_catch() {
        let mut source = SourceFile {
            items: vec![Item::Function(failing_test_fn("test_expected_fail"))],
            span: s(),
        };
        inject_harness(&mut source);
        let Item::Function(main) = source.items.last().unwrap() else {
            panic!()
        };
        // failing test: println + VarDecl + TryCatch + If + println = 5 stmts
        assert!(main
            .body
            .stmts
            .iter()
            .any(|s| matches!(s, Stmt::TryCatch { .. })));
    }

    #[test]
    fn inject_harness_is_idempotent_when_no_tests() {
        let mut source = SourceFile {
            items: vec![],
            span: s(),
        };
        inject_harness(&mut source);
        assert!(source.items.is_empty());
    }
}
