use crate::analyzer::typed_ast::{TypedBlock, TypedStmt};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: usize,
    /// Indices into the flat stmt list (conceptual; here we store stmt refs by position).
    pub stmts: Vec<usize>,
    pub term: Terminator,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Terminator {
    Return,
    Raise,
    Jump(usize),          // target block id
    Branch(usize, usize), // then_id, else_id
    Unreachable,
}

#[derive(Debug, Clone)]
pub struct Cfg {
    pub blocks: Vec<BasicBlock>,
}

impl Cfg {
    /// Return the successors of block `id`.
    pub fn successors(&self, id: usize) -> Vec<usize> {
        match &self.blocks[id].term {
            Terminator::Jump(t) => vec![*t],
            Terminator::Branch(t, e) => vec![*t, *e],
            Terminator::Return | Terminator::Raise | Terminator::Unreachable => vec![],
        }
    }

    /// Return the predecessors of every block.
    pub fn predecessors(&self) -> Vec<HashSet<usize>> {
        let n = self.blocks.len();
        let mut preds: Vec<HashSet<usize>> = vec![HashSet::new(); n];
        for id in 0..n {
            for s in self.successors(id) {
                preds[s].insert(id);
            }
        }
        preds
    }
}

/// Find blocks that have no predecessors and are not the entry block (block 0).
pub fn find_unreachable_blocks(cfg: &Cfg) -> Vec<usize> {
    let preds = cfg.predecessors();
    preds
        .iter()
        .enumerate()
        .filter(|(id, p)| *id != 0 && p.is_empty())
        .map(|(id, _)| id)
        .collect()
}

pub struct CfgBuilder {
    blocks: Vec<BasicBlock>,
    current: usize,
    stmt_offset: usize,
}

impl CfgBuilder {
    pub fn build(block: &TypedBlock) -> Cfg {
        let mut builder = CfgBuilder {
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![],
                term: Terminator::Jump(0), // placeholder
            }],
            current: 0,
            stmt_offset: 0,
        };
        builder.process_block(block);
        // Terminate the last block as Return if it has no explicit terminator set.
        let n = builder.blocks.len();
        if builder.blocks[builder.current].term == Terminator::Jump(0) && builder.current == n - 1 {
            builder.blocks[builder.current].term = Terminator::Return;
        }
        // Fix any remaining placeholder jumps (Jump(0) on non-entry blocks).
        // These arise for empty functions -- treat as Return.
        for b in &mut builder.blocks {
            if b.term == Terminator::Jump(0) && b.id != 0 {
                b.term = Terminator::Return;
            }
        }
        Cfg {
            blocks: builder.blocks,
        }
    }

    fn new_block(&mut self) -> usize {
        let id = self.blocks.len();
        self.blocks.push(BasicBlock {
            id,
            stmts: vec![],
            term: Terminator::Jump(0), // placeholder
        });
        id
    }

    fn set_term(&mut self, term: Terminator) {
        self.blocks[self.current].term = term;
    }

    fn switch_to(&mut self, id: usize) {
        self.current = id;
    }

    fn process_block(&mut self, block: &TypedBlock) {
        for (i, stmt) in block.stmts.iter().enumerate() {
            let global_idx = self.stmt_offset + i;
            self.blocks[self.current].stmts.push(global_idx);
            match stmt {
                TypedStmt::Return { .. } => {
                    self.set_term(Terminator::Return);
                    // Statements after return are unreachable; create a new dead block.
                    let dead = self.new_block();
                    self.blocks[dead].term = Terminator::Unreachable;
                    self.stmt_offset += i + 1;
                    return;
                }
                TypedStmt::Raise { .. } => {
                    self.set_term(Terminator::Raise);
                    let dead = self.new_block();
                    self.blocks[dead].term = Terminator::Unreachable;
                    self.stmt_offset += i + 1;
                    return;
                }
                TypedStmt::Break(_) | TypedStmt::Continue(_) => {
                    // Treat as a jump to a placeholder (the loop exit/header).
                    // Use a sentinel block id (usize::MAX) for now.
                    self.set_term(Terminator::Jump(usize::MAX));
                    let dead = self.new_block();
                    self.blocks[dead].term = Terminator::Unreachable;
                    self.stmt_offset += i + 1;
                    return;
                }
                TypedStmt::If {
                    branches,
                    else_branch,
                    ..
                } => {
                    // Build: current --(branch)--> then_block / else_block --> merge_block
                    let merge = self.new_block();
                    let _last_else = merge;

                    // Build branch chain in reverse.
                    // For simplicity, model first branch as the primary branch.
                    let then_block = self.new_block();
                    let else_block = if else_branch.is_some() || branches.len() > 1 {
                        self.new_block()
                    } else {
                        merge
                    };

                    self.set_term(Terminator::Branch(then_block, else_block));
                    self.stmt_offset += i + 1;

                    // Process then body
                    let saved_then = self.stmt_offset;
                    self.switch_to(then_block);
                    if let Some((_, body)) = branches.first() {
                        let sub_stmt_count = count_stmts(body);
                        self.process_block(body);
                        self.stmt_offset = saved_then + sub_stmt_count;
                    }
                    // Jump to merge if not already terminated
                    if !is_terminal_term(&self.blocks[self.current].term) {
                        self.set_term(Terminator::Jump(merge));
                    }

                    // Process else/elif
                    if else_block != merge {
                        self.switch_to(else_block);
                        if let Some(eb) = else_branch {
                            self.process_block(eb);
                        }
                        if !is_terminal_term(&self.blocks[self.current].term) {
                            self.set_term(Terminator::Jump(merge));
                        }
                    }

                    self.switch_to(merge);
                    self.blocks[merge].term = Terminator::Return; // placeholder -> will be updated
                    return;
                }
                TypedStmt::While { body, .. } => {
                    // header -> body -> header (loop back edge), header -> exit
                    let header = self.new_block();
                    let body_block = self.new_block();
                    let exit_block = self.new_block();

                    self.set_term(Terminator::Jump(header));
                    self.stmt_offset += i + 1;

                    self.switch_to(header);
                    self.blocks[header].term = Terminator::Branch(body_block, exit_block);

                    let _saved = self.stmt_offset;
                    self.switch_to(body_block);
                    self.process_block(body);
                    if !is_terminal_term(&self.blocks[self.current].term) {
                        self.set_term(Terminator::Jump(header));
                    }

                    self.switch_to(exit_block);
                    self.blocks[exit_block].term = Terminator::Return; // placeholder
                    return;
                }
                TypedStmt::For { body, .. } => {
                    // Similar to while: header -> body -> header, header -> exit
                    let header = self.new_block();
                    let body_block = self.new_block();
                    let exit_block = self.new_block();

                    self.set_term(Terminator::Jump(header));
                    self.stmt_offset += i + 1;

                    self.switch_to(header);
                    self.blocks[header].term = Terminator::Branch(body_block, exit_block);

                    self.switch_to(body_block);
                    self.process_block(body);
                    if !is_terminal_term(&self.blocks[self.current].term) {
                        self.set_term(Terminator::Jump(header));
                    }

                    self.switch_to(exit_block);
                    self.blocks[exit_block].term = Terminator::Return; // placeholder
                    return;
                }
                TypedStmt::TryCatch { .. } => {
                    // Model try-catch similarly: entry -> merge
                    let merge = self.new_block();
                    self.set_term(Terminator::Jump(merge));
                    self.switch_to(merge);
                    self.blocks[merge].term = Terminator::Return;
                    self.stmt_offset += i + 1;
                    return;
                }
                _ => {} // Non-control-flow statements: continue adding to current block
            }
        }
        self.stmt_offset += block.stmts.len();
    }
}

fn count_stmts(block: &TypedBlock) -> usize {
    block.stmts.len()
}

fn is_terminal_term(term: &Terminator) -> bool {
    matches!(
        term,
        Terminator::Return | Terminator::Raise | Terminator::Unreachable | Terminator::Branch(_, _)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::ty::Ty;
    use crate::analyzer::typed_ast::{TypedBlock, TypedExpr, TypedExprKind, TypedStmt};
    use crate::diagnostics::Span;

    fn s() -> Span {
        Span { start: 0, end: 0 }
    }

    fn empty_block() -> TypedBlock {
        TypedBlock {
            stmts: vec![],
            span: s(),
        }
    }

    fn bool_expr() -> TypedExpr {
        TypedExpr {
            kind: TypedExprKind::Bool(true),
            ty: Ty::Bool,
            span: s(),
        }
    }

    #[test]
    fn cfg_for_linear_block_has_one_block() {
        let block = TypedBlock {
            stmts: vec![
                TypedStmt::Expr(TypedExpr {
                    kind: TypedExprKind::Int(1),
                    ty: Ty::Int,
                    span: s(),
                }),
                TypedStmt::Expr(TypedExpr {
                    kind: TypedExprKind::Int(2),
                    ty: Ty::Int,
                    span: s(),
                }),
            ],
            span: s(),
        };
        let cfg = CfgBuilder::build(&block);
        // Should have exactly 1 basic block with both statements.
        assert_eq!(cfg.blocks[0].stmts.len(), 2);
        assert_eq!(cfg.blocks[0].term, Terminator::Return);
    }

    #[test]
    fn cfg_for_if_else_has_three_blocks() {
        // if true { } else { }
        let block = TypedBlock {
            stmts: vec![TypedStmt::If {
                branches: vec![(bool_expr(), empty_block())],
                else_branch: Some(empty_block()),
                span: s(),
            }],
            span: s(),
        };
        let cfg = CfgBuilder::build(&block);
        // Entry block + then + else/merge = at least 3 blocks
        assert!(
            cfg.blocks.len() >= 3,
            "expected at least 3 blocks, got {}",
            cfg.blocks.len()
        );
        // Entry block should be a Branch
        assert!(matches!(cfg.blocks[0].term, Terminator::Branch(_, _)));
    }

    #[test]
    fn cfg_for_while_loop_has_back_edge() {
        let block = TypedBlock {
            stmts: vec![TypedStmt::While {
                cond: bool_expr(),
                body: TypedBlock {
                    stmts: vec![TypedStmt::Expr(TypedExpr {
                        kind: TypedExprKind::Int(1),
                        ty: Ty::Int,
                        span: s(),
                    })],
                    span: s(),
                },
                span: s(),
            }],
            span: s(),
        };
        let cfg = CfgBuilder::build(&block);

        // The body block should jump back to the header (back edge).
        // Find a block with a Jump that points to a lower-id block.
        let has_back_edge = cfg.blocks.iter().any(|b| {
            if let Terminator::Jump(target) = b.term {
                target < b.id
            } else {
                false
            }
        });
        assert!(has_back_edge, "expected a back edge in while loop CFG");
    }

    #[test]
    fn cfg_unreachable_block_after_return_is_detected() {
        let block = TypedBlock {
            stmts: vec![
                TypedStmt::Return {
                    value: None,
                    span: s(),
                },
                // This statement is after the return -- unreachable
                TypedStmt::Expr(TypedExpr {
                    kind: TypedExprKind::Int(42),
                    ty: Ty::Int,
                    span: s(),
                }),
            ],
            span: s(),
        };
        let cfg = CfgBuilder::build(&block);
        let unreachable = find_unreachable_blocks(&cfg);
        // There should be at least one unreachable block
        assert!(
            !unreachable.is_empty(),
            "expected at least one unreachable block after return"
        );
    }

    #[test]
    fn block_with_no_predecessors_is_flagged() {
        // Manually construct a CFG with a disconnected block
        let cfg = Cfg {
            blocks: vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![],
                    term: Terminator::Return,
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![],
                    term: Terminator::Return,
                },
            ],
        };
        let unreachable = find_unreachable_blocks(&cfg);
        assert!(
            unreachable.contains(&1),
            "block 1 has no predecessors so should be flagged"
        );
    }

    #[test]
    fn entry_block_is_never_flagged() {
        let cfg = Cfg {
            blocks: vec![BasicBlock {
                id: 0,
                stmts: vec![],
                term: Terminator::Return,
            }],
        };
        let unreachable = find_unreachable_blocks(&cfg);
        assert!(
            !unreachable.contains(&0),
            "entry block (id=0) should never be flagged unreachable"
        );
    }

    #[test]
    fn reachable_blocks_produce_no_warning() {
        // 0 -> 1 -> 2 (all reachable from 0)
        let cfg = Cfg {
            blocks: vec![
                BasicBlock {
                    id: 0,
                    stmts: vec![],
                    term: Terminator::Jump(1),
                },
                BasicBlock {
                    id: 1,
                    stmts: vec![],
                    term: Terminator::Jump(2),
                },
                BasicBlock {
                    id: 2,
                    stmts: vec![],
                    term: Terminator::Return,
                },
            ],
        };
        let unreachable = find_unreachable_blocks(&cfg);
        assert!(
            unreachable.is_empty(),
            "all blocks reachable, expected no warnings"
        );
    }
}
