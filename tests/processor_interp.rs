use kiln_compiler::annotations::run_user_processors;
use kiln_compiler::lexer::Lexer;
use kiln_compiler::parser::ast::{Block, Expr, Item, Stmt, StringSegment};
use kiln_compiler::parser::Parser;

// AST inspection helpers

fn expr_contains_gen_splice(e: &Expr) -> bool {
    match e {
        Expr::GenSplice(_, _) => true,
        Expr::BinOp { left, right, .. } => {
            expr_contains_gen_splice(left) || expr_contains_gen_splice(right)
        }
        Expr::UnOp { operand, .. } => expr_contains_gen_splice(operand),
        Expr::Call { callee, args, .. } => {
            expr_contains_gen_splice(callee) || args.iter().any(expr_contains_gen_splice)
        }
        Expr::Field { object, .. } => expr_contains_gen_splice(object),
        Expr::Index { object, index, .. } => {
            expr_contains_gen_splice(object) || expr_contains_gen_splice(index)
        }
        Expr::Str(segs, _) => segs.iter().any(|s| {
            if let StringSegment::Interp(ie) = s {
                expr_contains_gen_splice(ie)
            } else {
                false
            }
        }),
        Expr::Tuple(es, _) | Expr::Array(es, _) => es.iter().any(expr_contains_gen_splice),
        Expr::StructLiteral { fields, .. } => {
            fields.iter().any(|(_, fe)| expr_contains_gen_splice(fe))
        }
        Expr::Unwrap(inner, _) => expr_contains_gen_splice(inner),
        Expr::As { expr, .. } => expr_contains_gen_splice(expr),
        Expr::Match {
            scrutinee, arms, ..
        } => {
            expr_contains_gen_splice(scrutinee)
                || arms.iter().any(|arm| {
                    arm.guard.as_ref().is_some_and(expr_contains_gen_splice)
                        || expr_contains_gen_splice(&arm.body)
                })
        }
        Expr::Spawn(inner, _) => expr_contains_gen_splice(inner),
        Expr::Ref { expr, .. } => expr_contains_gen_splice(expr),
        _ => false,
    }
}

fn block_contains_gen_splice(block: &Block) -> bool {
    block.stmts.iter().any(stmt_contains_gen_splice)
}

fn stmt_contains_gen_splice(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::VarDecl { value, .. } => expr_contains_gen_splice(value),
        Stmt::Assign { target, value, .. } => {
            expr_contains_gen_splice(target) || expr_contains_gen_splice(value)
        }
        Stmt::CompoundAssign { target, rhs, .. } => {
            expr_contains_gen_splice(target) || expr_contains_gen_splice(rhs)
        }
        Stmt::Return { value: Some(e), .. } => expr_contains_gen_splice(e),
        Stmt::Raise { value: Some(e), .. } => expr_contains_gen_splice(e),
        Stmt::Expr(e) => expr_contains_gen_splice(e),
        Stmt::If {
            branches,
            else_branch,
            ..
        } => {
            branches
                .iter()
                .any(|(c, b)| expr_contains_gen_splice(c) || block_contains_gen_splice(b))
                || else_branch.as_ref().is_some_and(block_contains_gen_splice)
        }
        Stmt::While { cond, body, .. } => {
            expr_contains_gen_splice(cond) || block_contains_gen_splice(body)
        }
        Stmt::DoWhile { body, cond, .. } => {
            block_contains_gen_splice(body) || expr_contains_gen_splice(cond)
        }
        Stmt::For { iterable, body, .. } => {
            expr_contains_gen_splice(iterable) || block_contains_gen_splice(body)
        }
        Stmt::TryCatch {
            body,
            handlers,
            finally,
            ..
        } => {
            block_contains_gen_splice(body)
                || handlers.iter().any(|h| block_contains_gen_splice(&h.body))
                || finally.as_ref().is_some_and(block_contains_gen_splice)
        }
        _ => false,
    }
}

fn fn_body_clean(ast: &kiln_compiler::parser::ast::SourceFile, name: &str) -> bool {
    ast.items
        .iter()
        .find_map(|i| {
            if let Item::Function(f) = i {
                if f.name == name {
                    return Some(!block_contains_gen_splice(&f.body));
                }
            }
            None
        })
        .unwrap_or(true)
}

fn parse_and_run(src: &str) -> kiln_compiler::parser::ast::SourceFile {
    let tokens = Lexer::new(src).tokenize().expect("lex failed");
    let mut ast = Parser::new(tokens).parse_file().expect("parse failed");
    let mut _proc_errors = vec![];
    run_user_processors(&mut ast, &mut _proc_errors);
    ast
}

fn fn_names(ast: &kiln_compiler::parser::ast::SourceFile) -> Vec<String> {
    ast.items
        .iter()
        .filter_map(|i| {
            if let Item::Function(f) = i {
                Some(f.name.clone())
            } else {
                None
            }
        })
        .collect()
}

fn fn_body_stmt_count(ast: &kiln_compiler::parser::ast::SourceFile, name: &str) -> usize {
    ast.items
        .iter()
        .find_map(|i| {
            if let Item::Function(f) = i {
                if f.name == name {
                    return Some(f.body.stmts.len());
                }
            }
            None
        })
        .unwrap_or(0)
}

// Noop processor: returns (None, []) -- keeps original, emits nothing

#[test]
fn noop_processor_leaves_source_unchanged() {
    let src = r#"
annotation Noop { }
processor Noop(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    return (None, Vec.new())
}
@Noop
def greet() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    let names = fn_names(&ast);
    assert!(
        names.contains(&"greet".to_string()),
        "greet must still exist: {names:?}"
    );
    assert!(
        names.contains(&"main".to_string()),
        "main must still exist: {names:?}"
    );
    assert_eq!(names.len(), 2, "no extra items emitted: {names:?}");
}

// Body-wrapping processor: replaces body with gen { <<target.body>> }
// (effectively a no-op transform, but exercises gen splice)

#[test]
fn wrap_processor_preserves_body_statement_count() {
    let src = r#"
annotation Wrap { }
processor Wrap(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@Wrap
def work() -> void {
    x: int = 1
    y: int = 2
}
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    // The "work" function had 2 statements; after wrapping with gen { <<target.body>> }
    // it should still have 2 statements (body is preserved via splice).
    let count = fn_body_stmt_count(&ast, "work");
    assert_eq!(
        count, 2,
        "wrapped body must still have 2 stmts, got {count}"
    );
}

// Prepend processor: adds a statement before the original body

#[test]
fn prepend_processor_adds_statement_to_body() {
    let src = r#"
annotation LogEnter { }
processor LogEnter(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        sentinel: int = 0
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@LogEnter
def action() -> void {
    x: int = 42
}
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    // "action" had 1 statement; after prepending sentinel it should have 2.
    let count = fn_body_stmt_count(&ast, "action");
    assert_eq!(count, 2, "prepend processor must add 1 stmt, got {count}");
}

// CompoundAssign inside gen block: <<splice>> in rhs must be substituted

#[test]
fn gen_block_compound_assign_splice_is_substituted() {
    let src = r#"
annotation Step { amount: int = 1 }
processor Step(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        x: int = 0
        x += <<target.annot.amount>>
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@Step { amount: 7 }
def go() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    assert!(
        fn_body_clean(&ast, "go"),
        "gen body with compound-assign splice must have no residual GenSplice nodes"
    );
    let count = fn_body_stmt_count(&ast, "go");
    assert_eq!(count, 2, "expected x:int=0 + x+=7, got {count} stmts");
}

// String interpolation with GenSplice inside gen block must be substituted

#[test]
fn gen_block_string_interp_splice_is_substituted() {
    let src = r#"
annotation Tag { }
processor Tag(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        label: str = "fn:{<<target.name>>}"
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@Tag
def work() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    assert!(
        fn_body_clean(&ast, "work"),
        "gen body with string-interp splice must have no residual GenSplice nodes"
    );
}

// For loop inside gen block: splice in iterable must be substituted

#[test]
fn gen_block_for_loop_splice_is_substituted() {
    let src = r#"
annotation RepeatN { count: int = 2 }
processor RepeatN(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        i: int = 0
        for _ <- range(<<target.annot.count>>) {
            <<target.body>>
        }
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@RepeatN { count: 3 }
def ping() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    assert!(
        fn_body_clean(&ast, "ping"),
        "gen body with for-loop splice must have no residual GenSplice nodes"
    );
}

// DoWhile inside gen block: splice in cond must be substituted

#[test]
fn gen_block_do_while_splice_is_substituted() {
    let src = r#"
annotation Thresh { limit: int = 5 }
processor Thresh(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        x: int = 0
        do {
            <<target.body>>
            x = x + 1
        } while x < <<target.annot.limit>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@Thresh { limit: 10 }
def tick() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    assert!(
        fn_body_clean(&ast, "tick"),
        "gen body with do-while splice must have no residual GenSplice nodes"
    );
}

// TryCatch handler body inside gen block: splice in handler must be substituted

#[test]
fn gen_block_try_catch_handler_splice_is_substituted() {
    let src = r#"
annotation Safe { }
processor Safe(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        try {
            <<target.body>>
        } except Exception as e {
            logged: str = "{<<target.name>>} failed"
        }
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@Safe
def risky() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    assert!(
        fn_body_clean(&ast, "risky"),
        "gen body with try-catch handler splice must have no residual GenSplice nodes"
    );
    let count = fn_body_stmt_count(&ast, "risky");
    assert_eq!(
        count, 1,
        "risky must be transformed to have 1 try-catch stmt, got {count}"
    );
}

// Struct literal with splice inside gen block must be substituted

#[test]
fn gen_block_struct_literal_splice_is_substituted() {
    let src = r#"
struct Named { label: str }
annotation AddLabel { }
processor AddLabel(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        n: Named = Named { label: "{<<target.name>>}" }
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@AddLabel
def demo() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    assert!(
        fn_body_clean(&ast, "demo"),
        "gen body with struct-literal splice must have no residual GenSplice nodes"
    );
}

// Array literal with splice inside gen block must be substituted

#[test]
fn gen_block_array_literal_splice_is_substituted() {
    let src = r#"
annotation WithLen { size: int = 1 }
processor WithLen(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        arr: Vec[int] = [<<target.annot.size>>, 0]
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@WithLen { size: 42 }
def sized() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    assert!(
        fn_body_clean(&ast, "sized"),
        "gen body with array-literal splice must have no residual GenSplice nodes"
    );
}

// target.params is accessible (populated in the FnDecl struct value)

#[test]
fn processor_can_access_target_params_len() {
    let src = r#"
annotation ParamCount { }
processor ParamCount(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    pc: int = target.params.len()
    new_body: Block = gen {
        param_count: int = <<pc>>
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@ParamCount
def add(a: int, b: int) -> int { return a + b }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    assert!(
        fn_body_clean(&ast, "add"),
        "processor body with target.params.len() must produce clean gen body"
    );
    let count = fn_body_stmt_count(&ast, "add");
    assert_eq!(
        count, 2,
        "expected param_count decl + original return, got {count}"
    );
}

// Stacked annotations must compose: each processor sees the previous output

#[test]
fn stacked_annotations_compose_in_order() {
    let src = r#"
annotation First { }
processor First(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        first: int = 1
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
annotation Second { }
processor Second(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    new_body: Block = gen {
        second: int = 2
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@First
@Second
def work() -> void {
    original: int = 0
}
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    // @First runs first (prepends first:int=1), then @Second sees the result
    // and prepends second:int=2 -> final: [second, first, original] = 3 stmts
    let count = fn_body_stmt_count(&ast, "work");
    assert_eq!(
        count, 3,
        "stacked annotations must compose: second wraps first wraps original = 3 stmts, got {count}"
    );
}

// BinOp arithmetic in processor body is evaluated

#[test]
fn processor_binop_adds_param_count_to_base() {
    let src = r#"
annotation Offset { base: int = 10 }
processor Offset(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    pc: int = target.params.len()
    total: int = pc + target.annot.base
    new_body: Block = gen {
        result: int = <<total>>
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@Offset { base: 5 }
def compute(x: int, y: int) -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    // compute has 2 params, base=5, total = 2+5 = 7 -> body has result:int=7
    assert!(
        fn_body_clean(&ast, "compute"),
        "binop arithmetic in processor body must produce clean gen block"
    );
    let count = fn_body_stmt_count(&ast, "compute");
    assert_eq!(count, 1, "expected result:int=7 as only stmt, got {count}");
}

// If-branch in processor body selects different generated code

#[test]
fn processor_if_branch_selects_different_body() {
    let src = r#"
annotation Mode { kind: int = 1 }
processor Mode(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    if target.annot.kind == 1 {
        new_body: Block = gen {
            a1: int = 1
            a2: int = 2
            <<target.body>>
        }
        return (Some { value: target.with_body(new_body) }, Vec.new())
    }
    new_body: Block = gen {
        b1: int = 1
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@Mode { kind: 1 }
def alpha() -> void { }
@Mode { kind: 2 }
def beta() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    let alpha_count = fn_body_stmt_count(&ast, "alpha");
    let beta_count = fn_body_stmt_count(&ast, "beta");
    assert_eq!(
        alpha_count, 2,
        "kind=1 -> 2 stmts prepended, got {alpha_count}"
    );
    assert_eq!(
        beta_count, 1,
        "kind=2 -> 1 stmt prepended, got {beta_count}"
    );
}

// Processor emits extra declaration via the Vec[Decl] return slot

#[test]
fn processor_emits_cloned_function_as_extra() {
    let src = r#"
annotation Clone { as_name: str = "cloned" }
processor Clone(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    clone: Decl = target.with_name(target.annot.as_name)
    return (None, [clone])
}
@Clone { as_name: "greet_copy" }
def greet() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    let names = fn_names(&ast);
    assert!(
        names.contains(&"greet".to_string()),
        "original must still exist: {names:?}"
    );
    assert!(
        names.contains(&"greet_copy".to_string()),
        "cloned function must be emitted: {names:?}"
    );
    assert_eq!(names.len(), 3, "exactly greet, greet_copy, main: {names:?}");
}

// with_name produces a renamed function keeping the original body

#[test]
fn processor_with_name_renames_function() {
    let src = r#"
annotation Rename { new_name: str = "renamed" }
processor Rename(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    renamed: Decl = target.with_name(target.annot.new_name)
    return (Some { value: renamed }, Vec.new())
}
@Rename { new_name: "hello" }
def original_name() -> void {
    x: int = 42
}
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    let names = fn_names(&ast);
    assert!(
        names.contains(&"hello".to_string()),
        "renamed function must exist: {names:?}"
    );
    assert!(
        !names.contains(&"original_name".to_string()),
        "original name must be gone: {names:?}"
    );
    let count = fn_body_stmt_count(&ast, "hello");
    assert_eq!(
        count, 1,
        "body of renamed function preserved (x:int=42), got {count}"
    );
}

// For loop in processor body iterates over params and counts them

#[test]
fn processor_for_loop_counts_params() {
    let src = r#"
annotation CountParams { }
processor CountParams(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    count: int = 0
    for p <- target.params {
        count = count + 1
    }
    new_body: Block = gen {
        counted: int = <<count>>
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@CountParams
def triple(a: int, b: int, c: int) -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    // triple has 3 params so count=3, body gets counted:int=3 prepended
    assert!(
        fn_body_clean(&ast, "triple"),
        "for-loop counting in processor body must produce clean gen block"
    );
    let count = fn_body_stmt_count(&ast, "triple");
    assert_eq!(count, 1, "expected counted:int=3 as only stmt, got {count}");
}

// CompoundAssign in processor body updates the binding

#[test]
fn processor_compound_assign_in_body() {
    let src = r#"
annotation Bump { amount: int = 1 }
processor Bump(target: FnDecl) -> (Option[Decl], Vec[Decl]) {
    n: int = target.annot.amount
    n += 10
    new_body: Block = gen {
        bumped: int = <<n>>
        <<target.body>>
    }
    return (Some { value: target.with_body(new_body) }, Vec.new())
}
@Bump { amount: 7 }
def go() -> void { }
def main() -> void { }
"#;
    let ast = parse_and_run(src);
    // n starts at 7, then n += 10 -> n = 17
    // body gets bumped:int=17
    assert!(
        fn_body_clean(&ast, "go"),
        "compound-assign in processor body must produce clean gen block"
    );
    let count = fn_body_stmt_count(&ast, "go");
    assert_eq!(count, 1, "expected bumped:int=17 as only stmt, got {count}");
}
