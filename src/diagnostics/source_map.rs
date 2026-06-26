use super::colors::get_colors;
use super::Span;

pub struct RenderOpts<'a> {
    pub code: &'a str,
    pub caret_label: Option<&'a str>,
    /// Pre-computed gutter digit width for cross-diagnostic alignment.
    /// Pass 0 to let the method compute from the local context range.
    pub gutter_width: usize,
    pub hyperlinks: bool,
    pub context_before: usize,
    pub context_after: usize,
}

fn make_file_uri(file: &str, line: usize, col: usize) -> String {
    let abs = std::fs::canonicalize(file)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| file.to_string());
    let normalized = abs.trim_start_matches(r"\\?\").replace('\\', "/");
    format!("file:///{normalized}:{line}:{col}")
}

fn osc8_open(uri: &str) -> String {
    format!("\x1b]8;;{uri}\x1b\\")
}

fn osc8_close() -> &'static str {
    "\x1b]8;;\x1b\\"
}

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

    fn get_line_opt<'s>(&self, src: &'s str, line_num: usize) -> Option<&'s str> {
        if line_num < 1 || line_num > self.line_starts.len() {
            return None;
        }
        let idx = line_num - 1;
        let start = self.line_starts[idx];
        let end = if idx + 1 < self.line_starts.len() {
            self.line_starts[idx + 1].saturating_sub(1)
        } else {
            src.len()
        };
        Some(&src[start..end])
    }

    /// Renders a rich diagnostic block with context lines, colors, hyperlinks, and caret label.
    ///
    /// The caller is responsible for printing the header line (`error[E002]: ...`).
    /// This method returns the location + source excerpt portion.
    ///
    /// ```text
    ///   --> file.kn:5:14
    ///    |
    /// 3  | (dim) prev prev line
    /// 4  | (dim) prev line
    /// 5  | let x: bool = 42
    ///    |                ^^ expected `bool`
    /// 6  | (dim) next line
    /// ```
    pub fn render_rich(&self, src: &str, span: Span, file: &str, opts: &RenderOpts<'_>) -> String {
        let c = get_colors();
        let (line_num, col) = self.location_of(span.start);
        let (_, _, caret_w) = self.line_info(src, span);

        let total_lines = self.line_starts.len();
        let ctx_start = line_num.saturating_sub(opts.context_before).max(1);
        let ctx_end = (line_num + opts.context_after).min(total_lines);

        let gw = if opts.gutter_width > 0 {
            opts.gutter_width
        } else {
            ctx_end.to_string().len()
        };
        let pad = " ".repeat(gw);

        let arrow = if opts.hyperlinks {
            let uri = make_file_uri(file, line_num, col);
            format!(
                "{pad}--> {}{}:{line_num}:{col}{}",
                osc8_open(&uri),
                file,
                osc8_close()
            )
        } else {
            format!("{pad}--> {file}:{line_num}:{col}")
        };

        let mut out = String::new();
        out.push_str(&format!("{}{arrow}{}\n", c.gutter, c.reset));
        out.push_str(&format!("{}{pad} |{}\n", c.gutter, c.reset));

        for ln in ctx_start..=ctx_end {
            if let Some(line_text) = self.get_line_opt(src, ln) {
                let lnum_str = format!("{:>gw$}", ln, gw = gw);
                if ln == line_num {
                    out.push_str(&format!(
                        "{}{lnum_str} |{} {line_text}\n",
                        c.gutter, c.reset
                    ));
                    let indent = " ".repeat(col - 1);
                    let caret = "^".repeat(caret_w);
                    let label = match opts.caret_label {
                        Some(l) if !l.is_empty() => format!(" {l}"),
                        _ => String::new(),
                    };
                    let cc = c.code_caret(opts.code);
                    out.push_str(&format!(
                        "{}{pad} |{} {indent}{}{caret}{label}{}\n",
                        c.gutter, c.reset, cc, c.reset
                    ));
                } else {
                    out.push_str(&format!(
                        "{}{lnum_str} |{}{} {line_text}{}\n",
                        c.gutter, c.reset, c.dim, c.reset
                    ));
                }
            }
        }

        if out.ends_with('\n') {
            out.pop();
        }
        out
    }

    /// Renders a rich note block with context lines, colors, hyperlinks, and caret.
    ///
    /// Includes the `note: <text>` header line.
    pub fn render_note_rich(
        &self,
        src: &str,
        span: Span,
        file: &str,
        note: &str,
        opts: &RenderOpts<'_>,
    ) -> String {
        let c = get_colors();
        let note_opts = RenderOpts {
            code: "note",
            caret_label: None,
            gutter_width: opts.gutter_width,
            hyperlinks: opts.hyperlinks,
            context_before: 0,
            context_after: 0,
        };
        let block = self.render_rich(src, span, file, &note_opts);
        format!("{}note:{} {note}\n{block}", c.note, c.reset)
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

    use super::RenderOpts;

    fn no_color_opts<'a>(code: &'a str, caret_label: Option<&'a str>) -> RenderOpts<'a> {
        RenderOpts {
            code,
            caret_label,
            gutter_width: 0,
            hyperlinks: false,
            context_before: 2,
            context_after: 1,
        }
    }

    #[test]
    fn render_rich_includes_context_lines() {
        let src = "line1\nline2\nline3\nline4\nline5\n";
        let map = SourceMap::new(src);
        let span = Span::new(12, 17);
        let result = map.render_rich(src, span, "test.kn", &no_color_opts("E001", None));
        assert!(result.contains("line2"), "context before");
        assert!(result.contains("line3"), "error line");
        assert!(result.contains("line4"), "context after");
        assert!(result.contains('^'), "caret present");
    }

    #[test]
    fn render_rich_caret_label_appended() {
        let src = "let x: bool = 42\n";
        let map = SourceMap::new(src);
        let span = Span::new(14, 16);
        let result = map.render_rich(
            src,
            span,
            "test.kn",
            &no_color_opts("E002", Some("expected `bool`")),
        );
        assert!(result.contains("expected `bool`"), "label in output");
    }

    #[test]
    fn render_rich_hyperlink_when_enabled() {
        let src = "let x = 42\n";
        let map = SourceMap::new(src);
        let span = Span::new(4, 5);
        let opts = RenderOpts {
            code: "E001",
            caret_label: None,
            gutter_width: 0,
            hyperlinks: true,
            context_before: 0,
            context_after: 0,
        };
        let result = map.render_rich(src, span, "test.kn", &opts);
        assert!(result.contains("\x1b]8;;"), "OSC 8 hyperlink present");
    }

    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn render_rich_gutter_width_aligns() {
        let src = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\n";
        let map = SourceMap::new(src);
        let span = Span::new(20, 21);
        let opts = RenderOpts {
            code: "E001",
            caret_label: None,
            gutter_width: 3,
            hyperlinks: false,
            context_before: 1,
            context_after: 1,
        };
        let result = map.render_rich(src, span, "test.kn", &opts);
        let first_line = strip_ansi(result.lines().next().unwrap_or(""));
        assert!(
            first_line.starts_with("   -->"),
            "gutter width=3 pads arrow: got {:?}",
            first_line
        );
    }

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
