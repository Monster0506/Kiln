use crate::parser::ast::{Block, Stmt};

/// Returns `true` if every code path through `block` ends with `return` or `raise`.
pub fn always_returns(block: &Block) -> bool {
    for stmt in &block.stmts {
        if stmt_always_returns(stmt) {
            return true; // rest of block is unreachable
        }
    }
    false
}

fn stmt_always_returns(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return { .. } => true,
        Stmt::Raise { .. } => true,

        Stmt::If {
            branches,
            else_branch,
            ..
        } => {
            if else_branch.is_none() {
                return false;
            }
            branches.iter().all(|(_, block)| always_returns(block))
                && always_returns(else_branch.as_ref().unwrap())
        }

        Stmt::TryCatch { body, handlers, .. } => {
            always_returns(body) && handlers.iter().all(|h| always_returns(&h.body))
        }

        // while/do-while/for: body may not execute
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Span;
    use crate::parser::ast::*;
    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    fn ret_stmt() -> Stmt {
        Stmt::Return {
            value: Some(Expr::Int(1, s())),
            span: s(),
        }
    }
    fn raise_stmt() -> Stmt {
        Stmt::Raise {
            value: Some(Expr::Int(1, s())),
            span: s(),
        }
    }

    #[test]
    fn empty_block_does_not_return() {
        assert!(!always_returns(&Block {
            stmts: vec![],
            span: s()
        }));
    }

    #[test]
    fn block_ending_in_return_always_returns() {
        assert!(always_returns(&Block {
            stmts: vec![ret_stmt()],
            span: s()
        }));
    }

    #[test]
    fn block_ending_in_raise_always_returns() {
        assert!(always_returns(&Block {
            stmts: vec![raise_stmt()],
            span: s()
        }));
    }

    #[test]
    fn if_else_both_return_is_ok() {
        let block = Block {
            stmts: vec![Stmt::If {
                branches: vec![(
                    Expr::Bool(true, s()),
                    Block {
                        stmts: vec![ret_stmt()],
                        span: s(),
                    },
                )],
                else_branch: Some(Block {
                    stmts: vec![ret_stmt()],
                    span: s(),
                }),
                span: s(),
            }],
            span: s(),
        };
        assert!(always_returns(&block));
    }

    #[test]
    fn if_without_else_does_not_always_return() {
        let block = Block {
            stmts: vec![Stmt::If {
                branches: vec![(
                    Expr::Bool(true, s()),
                    Block {
                        stmts: vec![ret_stmt()],
                        span: s(),
                    },
                )],
                else_branch: None,
                span: s(),
            }],
            span: s(),
        };
        assert!(!always_returns(&block));
    }
}
