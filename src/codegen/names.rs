use crate::parser::ast::{BinOp, UnOp};

/// Encode a binary operator string (from `HookName::Op` with params) to its
/// function-name suffix. Used when registering and dispatching hook functions.
pub fn encode_op(op: &str) -> String {
    match op {
        "+" => "op_add".into(),
        "-" => "op_sub".into(),
        "*" => "op_mul".into(),
        "/" => "op_div".into(),
        "%" => "op_mod".into(),
        "==" => "op_eq".into(),
        "!=" => "op_ne".into(),
        "<=>" => "op_cmp".into(),
        "[]" => "op_index".into(),
        "[]=" => "op_index_set".into(),
        "<" => "op_lt".into(),
        ">" => "op_gt".into(),
        "<=" => "op_lte".into(),
        ">=" => "op_gte".into(),
        "+=" => "op_add_assign".into(),
        "-=" => "op_sub_assign".into(),
        "*=" => "op_mul_assign".into(),
        "/=" => "op_div_assign".into(),
        "%=" => "op_mod_assign".into(),
        "()" => "op_call".into(),
        other => format!(
            "op_{}",
            other.chars().map(|c| c as u32).fold(0u32, |a, b| a ^ b)
        ),
    }
}

/// Encode a unary operator string to its function-name suffix. Distinct from binary
/// names so `hook +()` (normalize) and `hook +(rhs)` (add) don't collide.
pub fn encode_unary_op(op: &str) -> &'static str {
    match op {
        "+" => "pos",
        "-" => "neg",
        "!" => "not",
        _ => "unop",
    }
}

/// Map a `BinOp` to the hook function suffix for dispatch at call-sites.
/// Returns `None` for operators that have no corresponding hook (e.g. `&&`, `||`).
pub fn binop_fn_suffix(op: &BinOp) -> Option<&'static str> {
    match op {
        BinOp::Add => Some("op_add"),
        BinOp::Sub => Some("op_sub"),
        BinOp::Mul => Some("op_mul"),
        BinOp::Div => Some("op_div"),
        BinOp::Mod => Some("op_mod"),
        BinOp::Eq => Some("op_eq"),
        BinOp::Lt => Some("op_lt"),
        BinOp::Spaceship => Some("op_cmp"),
        _ => None,
    }
}

/// Map a `UnOp` to the hook function suffix for dispatch at call-sites.
pub fn unop_fn_suffix(op: &UnOp) -> &'static str {
    match op {
        UnOp::Neg => "neg",
        UnOp::Not => "not",
        UnOp::Pos => "pos",
    }
}
