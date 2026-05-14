#[derive(Debug, Clone, PartialEq)]
pub(super) enum Mode {
    Normal,
    /// Inside a string literal; `close` is the delimiter that ends it (`"` or `'`).
    String {
        close: char,
    },
    /// Inside a `{...}` interpolation within a string; tracks brace depth and the string's closing delimiter.
    Interp {
        depth: usize,
        close: char,
    },
}
