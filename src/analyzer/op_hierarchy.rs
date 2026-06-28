use crate::parser::ast::BinOp;

/// Ordered resolution path for a compound-assign operator, most specific to most general.
/// Codegen walks for the best impl; constraint checking uses the last as the required bound.
pub fn compound_assign_hierarchy(op: &BinOp) -> &'static [&'static str] {
    match op {
        BinOp::Add => &["AddAssignable", "AddableWith", "Addable"],
        BinOp::Sub => &["SubtractAssignable", "SubtractableWith", "Subtractable"],
        BinOp::Mul => &["MultiplyAssignable", "MultipliableWith", "Multiplicable"],
        BinOp::Div => &["DivideAssignable", "DividableWith", "Divisible"],
        BinOp::Mod => &["RemainderAssignable", "RemainderableWith", "Remainder"],
        _ => &[],
    }
}

/// Returns the loosest required interface for a compound-assign operator,
/// or None for operators with no compound-assign interface.
pub fn compound_assign_iface(op: &BinOp) -> Option<&'static str> {
    compound_assign_hierarchy(op).last().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_hierarchy_starts_with_add_assignable() {
        let h = compound_assign_hierarchy(&BinOp::Add);
        assert_eq!(h[0], "AddAssignable");
    }

    #[test]
    fn add_hierarchy_ends_with_addable() {
        let h = compound_assign_hierarchy(&BinOp::Add);
        assert_eq!(*h.last().unwrap(), "Addable");
    }

    #[test]
    fn add_iface_is_addable() {
        assert_eq!(compound_assign_iface(&BinOp::Add), Some("Addable"));
    }

    #[test]
    fn non_compound_op_returns_empty() {
        assert!(compound_assign_hierarchy(&BinOp::Eq).is_empty());
    }
}
