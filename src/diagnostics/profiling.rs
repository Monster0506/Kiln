use std::io::Write;

pub struct RegistryStats {
    pub types_total: usize,
    pub types_struct: usize,
    pub types_enum: usize,
    pub types_alias: usize,
    pub methods_total: usize,
    pub conformances_total: usize,
    pub interfaces_total: usize,
    /// Top method names by count of types that define them (capped at 20).
    pub top_methods: Vec<(String, usize)>,
}

impl RegistryStats {
    pub fn report(&self, out: &mut dyn Write) {
        let label_w: usize = 20;
        writeln!(out, "kiln profile report").unwrap();
        writeln!(out).unwrap();
        writeln!(out, "  type registry").unwrap();
        writeln!(
            out,
            "    {:<w$}  {}",
            "types registered",
            self.types_total,
            w = label_w
        )
        .unwrap();
        writeln!(
            out,
            "      {:<w$}  {}",
            "struct",
            self.types_struct,
            w = label_w - 2
        )
        .unwrap();
        writeln!(
            out,
            "      {:<w$}  {}",
            "enum",
            self.types_enum,
            w = label_w - 2
        )
        .unwrap();
        writeln!(
            out,
            "      {:<w$}  {}",
            "alias",
            self.types_alias,
            w = label_w - 2
        )
        .unwrap();
        writeln!(
            out,
            "    {:<w$}  {}",
            "methods registered",
            self.methods_total,
            w = label_w
        )
        .unwrap();
        writeln!(
            out,
            "    {:<w$}  {}",
            "conformances",
            self.conformances_total,
            w = label_w
        )
        .unwrap();
        writeln!(
            out,
            "    {:<w$}  {}",
            "interfaces",
            self.interfaces_total,
            w = label_w
        )
        .unwrap();

        if !self.top_methods.is_empty() {
            writeln!(out).unwrap();
            writeln!(
                out,
                "  method frequency (top {}, by type count)",
                self.top_methods.len()
            )
            .unwrap();
            let name_w = self
                .top_methods
                .iter()
                .map(|(n, _)| n.len())
                .max()
                .unwrap_or(4);
            for (name, count) in &self.top_methods {
                writeln!(out, "    {:<w$}  {}", name, count, w = name_w).unwrap();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_stats() -> RegistryStats {
        RegistryStats {
            types_total: 10,
            types_struct: 5,
            types_enum: 3,
            types_alias: 2,
            methods_total: 40,
            conformances_total: 12,
            interfaces_total: 4,
            top_methods: vec![
                ("add".to_string(), 6),
                ("to_str".to_string(), 4),
                ("eq".to_string(), 3),
            ],
        }
    }

    #[test]
    fn report_includes_type_counts() {
        let stats = sample_stats();
        let mut buf = Vec::<u8>::new();
        stats.report(&mut buf);
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("10"), "missing total: {s}");
        assert!(s.contains("struct"), "missing struct: {s}");
        assert!(
            s.contains('5'.to_string().as_str()),
            "missing struct count: {s}"
        );
        assert!(s.contains("enum"), "missing enum: {s}");
        assert!(s.contains("alias"), "missing alias: {s}");
    }

    #[test]
    fn report_includes_method_frequency_table() {
        let stats = sample_stats();
        let mut buf = Vec::<u8>::new();
        stats.report(&mut buf);
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("method frequency"), "missing header: {s}");
        assert!(s.contains("add"), "missing add: {s}");
        assert!(s.contains("to_str"), "missing to_str: {s}");
    }

    #[test]
    fn report_omits_method_table_when_empty() {
        let mut stats = sample_stats();
        stats.top_methods = vec![];
        let mut buf = Vec::<u8>::new();
        stats.report(&mut buf);
        let s = String::from_utf8(buf).unwrap();
        assert!(!s.contains("method frequency"), "should omit table: {s}");
    }

    #[test]
    fn report_header_is_present() {
        let stats = sample_stats();
        let mut buf = Vec::<u8>::new();
        stats.report(&mut buf);
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("kiln profile report"), "missing header: {s}");
    }
}
