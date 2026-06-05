use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub struct Colors {
    pub error: &'static str,
    pub warning: &'static str,
    pub note: &'static str,
    pub gutter: &'static str,
    pub dim: &'static str,
    pub bold: &'static str,
    pub caret_error: &'static str,
    pub caret_warning: &'static str,
    pub caret_note: &'static str,
    pub reset: &'static str,
    pub hyperlinks: bool,
}

static GLOBAL: OnceLock<Colors> = OnceLock::new();

pub fn get_colors() -> &'static Colors {
    GLOBAL.get_or_init(Colors::detect)
}

pub fn init_colors(enabled: bool) {
    GLOBAL.get_or_init(|| if enabled { Colors::on() } else { Colors::off() });
}

impl Colors {
    fn detect() -> Self {
        use std::io::IsTerminal;
        if std::env::var_os("NO_COLOR").is_some() {
            return Self::off();
        }
        if std::env::var_os("FORCE_COLOR").is_some() {
            return Self::on();
        }
        if std::io::stderr().is_terminal() {
            Self::on()
        } else {
            Self::off()
        }
    }

    pub fn on() -> Self {
        Self {
            error: "\x1b[1;31m",
            warning: "\x1b[1;38;2;255;146;47m",
            note: "\x1b[1;36m",
            gutter: "\x1b[94m",
            dim: "\x1b[2m",
            bold: "\x1b[1m",
            caret_error: "\x1b[31m",
            caret_warning: "\x1b[38;2;255;146;47m",
            caret_note: "\x1b[36m",
            reset: "\x1b[0m",
            hyperlinks: true,
        }
    }

    pub fn off() -> Self {
        Self {
            error: "",
            warning: "",
            note: "",
            gutter: "",
            dim: "",
            bold: "",
            caret_error: "",
            caret_warning: "",
            caret_note: "",
            reset: "",
            hyperlinks: false,
        }
    }

    pub fn code_color(&self, code: &str) -> &'static str {
        if code == "note" {
            self.note
        } else if code.starts_with('W') {
            self.warning
        } else {
            self.error
        }
    }

    pub fn code_caret(&self, code: &str) -> &'static str {
        if code == "note" {
            self.caret_note
        } else if code.starts_with('W') {
            self.caret_warning
        } else {
            self.caret_error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_color_error_returns_error_color() {
        let c = Colors::on();
        assert_eq!(c.code_color("E001"), c.error);
        assert_eq!(c.code_color("E002"), c.error);
    }

    #[test]
    fn code_color_warning_returns_warning_color() {
        let c = Colors::on();
        assert_eq!(c.code_color("W005"), c.warning);
        assert_eq!(c.code_color("W007"), c.warning);
    }

    #[test]
    fn code_color_note_returns_note_color() {
        let c = Colors::on();
        assert_eq!(c.code_color("note"), c.note);
    }

    #[test]
    fn off_colors_are_empty_strings() {
        let c = Colors::off();
        assert_eq!(c.error, "");
        assert_eq!(c.warning, "");
        assert_eq!(c.reset, "");
        assert!(!c.hyperlinks);
    }
}
