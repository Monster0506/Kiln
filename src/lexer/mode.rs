#[derive(Debug, Clone, PartialEq)]
pub(super) enum Mode {
    Normal,
    /// Inside a string literal, accumulating text.
    String,
    /// Inside a `{...}` interpolation within a string; tracks brace depth.
    Interp {
        depth: usize,
    },
}
