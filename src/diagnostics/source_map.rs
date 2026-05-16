use super::Span;

/// Maps byte offsets back to 1-indexed (line, column) pairs for human-readable diagnostics.
pub struct SourceMap {
    line_starts: Vec<usize>,
}

impl SourceMap {
    pub fn new(src: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, c) in src.char_indices() {
            if c == '\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts }
    }

    /// Returns the 1-indexed (line, column) for a byte offset.
    pub fn location_of(&self, offset: usize) -> (usize, usize) {
        let line = self
            .line_starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1);
        let col = offset - self.line_starts[line];
        (line + 1, col + 1)
    }

    /// Returns `"line:col"` for the start of a span.
    pub fn display_span(&self, span: Span) -> String {
        let (line, col) = self.location_of(span.start);
        format!("{line}:{col}")
    }

    /// Extracts the text of the line containing `offset` and the caret width
    /// for `span` on that line (clamped, minimum 1).
    fn line_info<'s>(&self, src: &'s str, span: Span) -> (&'s str, usize, usize) {
        let (line_num, col) = self.location_of(span.start);
        let line_idx = line_num - 1;
        let line_start = self.line_starts[line_idx];
        let line_end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1].saturating_sub(1)
        } else {
            src.len()
        };
        let text = &src[line_start..line_end];
        let caret_w = if span.end > span.start {
            (span.end.min(line_end).saturating_sub(span.start)).max(1)
        } else {
            1
        };
        (text, col, caret_w)
    }

    /// Renders a `note:` block with a secondary source location.
    ///
    /// ```text
    /// note: required because `sum` has use of `+=` on `T`
    ///   --> file.kn:14:5
    ///    |
    /// 14 |     total += item
    ///    |     ^^^^^^^^^^^^^
    /// ```
    pub fn render_note(&self, src: &str, span: Span, file: &str, note: &str) -> String {
        let (line_num, col) = self.location_of(span.start);
        let (text, _, caret_w) = self.line_info(src, span);
        let lnum = line_num.to_string();
        let gw = lnum.len();
        let pad = " ".repeat(gw);
        let indent = " ".repeat(col - 1);
        let caret = "^".repeat(caret_w);
        format!(
            "note: {note}\n\
             {pad}--> {file}:{line_num}:{col}\n\
             {pad} |\n\
             {lnum} | {text}\n\
             {pad} | {indent}{caret}"
        )
    }

    /// Returns a two-line snippet (source line + caret row).
    pub fn render_snippet(&self, src: &str, span: Span) -> String {
        let (line_num, _col) = self.location_of(span.start);
        let (text, col, caret_w) = self.line_info(src, span);
        let lnum = line_num.to_string();
        let pad = " ".repeat(lnum.len());
        let indent = " ".repeat(col - 1);
        let caret = "^".repeat(caret_w);
        format!("{lnum} | {text}\n{pad} | {indent}{caret}")
    }

    /// Renders a full Rust-style diagnostic block:
    ///
    /// ```text
    /// error[E002]: type mismatch: expected `bool`, found `int`
    ///   --> examples/file.kn:5:14
    ///    |
    /// 5  |     flag: bool = 42
    ///    |                  ^^
    /// ```
    ///
    /// The caller is responsible for printing the first `error[...]: ...` line.
    /// This method returns only the location + source excerpt portion.
    pub fn render_diagnostic(&self, src: &str, span: Span, file: &str) -> String {
        let (line_num, col) = self.location_of(span.start);
        let (text, _, caret_w) = self.line_info(src, span);
        let lnum = line_num.to_string();
        let gw = lnum.len();
        let pad = " ".repeat(gw);
        let indent = " ".repeat(col - 1);
        let caret = "^".repeat(caret_w);
        format!(
            "{pad}--> {file}:{line_num}:{col}\n\
             {pad} |\n\
             {lnum} | {text}\n\
             {pad} | {indent}{caret}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line_offsets() {
        let map = SourceMap::new("hello world");
        assert_eq!(map.location_of(0), (1, 1));
        assert_eq!(map.location_of(5), (1, 6));
        assert_eq!(map.location_of(10), (1, 11));
    }

    #[test]
    fn multi_line_offsets() {
        let src = "abc\ndef\nghi";
        let map = SourceMap::new(src);
        assert_eq!(map.location_of(0), (1, 1)); // 'a'
        assert_eq!(map.location_of(3), (1, 4)); // '\n'
        assert_eq!(map.location_of(4), (2, 1)); // 'd'
        assert_eq!(map.location_of(7), (2, 4)); // '\n'
        assert_eq!(map.location_of(8), (3, 1)); // 'g'
    }

    #[test]
    fn display_span_shows_line_col() {
        let src = "abc\ndef";
        let map = SourceMap::new(src);
        assert_eq!(map.display_span(Span::new(4, 7)), "2:1");
    }

    #[test]
    fn render_snippet_single_char_caret() {
        let src = "let x = 1\nlet y = 2\n";
        let map = SourceMap::new(src);
        let snippet = map.render_snippet(src, Span::new(4, 5)); // 'x' on line 1
        assert!(snippet.contains("let x = 1"), "should include line text");
        assert!(snippet.contains("^"), "should have caret");
        // caret should be at column 5 (0-indexed 4 => col 5 => 4 spaces + ^)
        let caret_line = snippet.lines().nth(1).unwrap();
        assert!(caret_line.contains("    ^"), "caret at right column");
    }

    #[test]
    fn render_snippet_multi_char_span() {
        let src = "foo bar baz";
        let map = SourceMap::new(src);
        let snippet = map.render_snippet(src, Span::new(4, 7)); // "bar"
        let caret_line = snippet.lines().nth(1).unwrap();
        assert!(caret_line.contains("^^^"), "caret spans the word");
    }

    #[test]
    fn render_snippet_caret_clamped_to_line() {
        let src = "hello\nworld";
        let map = SourceMap::new(src);
        // span crosses the newline; caret should not extend past "hello"
        let snippet = map.render_snippet(src, Span::new(0, 10));
        let caret_line = snippet.lines().nth(1).unwrap();
        // "hello" is 5 chars, caret should be exactly 5 wide
        let caret_count = caret_line.chars().filter(|&c| c == '^').count();
        assert_eq!(caret_count, 5, "caret clamped to line length");
    }
}
