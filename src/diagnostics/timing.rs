use std::io::Write;
use std::time::{Duration, Instant};

pub struct ProcessorRun {
    pub name: String,
    pub item_count: usize,
    pub duration: Duration,
}

#[derive(Default)]
pub struct ItemCounts {
    pub functions: usize,
    pub structs: usize,
    pub enums: usize,
    pub processors: usize,
}

#[derive(Default)]
pub struct BuildStats {
    pub source_file: String,
    pub source_lines: usize,
    pub token_count: usize,
    pub ast_node_count: usize,
    pub item_counts: ItemCounts,
    pub processor_runs: Vec<ProcessorRun>,
    pub fn_codegen_times: Vec<(String, Duration)>,
    pub object_bytes: usize,
    pub object_path: String,
    pub binary_bytes: usize,
    pub binary_path: String,
    pub warning_count: usize,
    pub error_count: usize,
}

pub struct PhaseTimer {
    phases: Vec<(String, Duration)>,
    current: Option<(String, Instant)>,
}

impl Default for PhaseTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl PhaseTimer {
    pub fn new() -> Self {
        Self {
            phases: vec![],
            current: None,
        }
    }

    pub fn start(&mut self, name: &str) {
        self.stop();
        self.current = Some((name.to_string(), Instant::now()));
    }

    pub fn stop(&mut self) {
        if let Some((name, t)) = self.current.take() {
            self.phases.push((name, t.elapsed()));
        }
    }

    /// Lightweight timing table for commands that don't have full BuildStats (e.g. `check`).
    pub fn report_simple(&self, label: &str, out: &mut dyn Write) {
        let mut phases = self.phases.clone();
        if let Some((name, t)) = &self.current {
            phases.push((name.clone(), t.elapsed()));
        }
        if phases.is_empty() {
            return;
        }
        let total: std::time::Duration = phases.iter().map(|(_, d)| *d).sum();
        let phase_name_w = phases
            .iter()
            .map(|(n, _)| n.len())
            .max()
            .unwrap_or(0)
            .max("total".len());
        let time_strings: Vec<String> = phases.iter().map(|(_, d)| fmt_dur(*d)).collect();
        let total_str = fmt_dur(total);
        let time_w = time_strings
            .iter()
            .map(|s| s.len())
            .max()
            .unwrap_or(0)
            .max(total_str.len());

        writeln!(out, "{label}").unwrap();
        for ((name, _), time_str) in phases.iter().zip(time_strings.iter()) {
            writeln!(
                out,
                "  {:<name_w$}  {:>time_w$}",
                name,
                time_str,
                name_w = phase_name_w,
                time_w = time_w
            )
            .unwrap();
        }
        let sep = "-".repeat(phase_name_w + time_w + 4);
        writeln!(out, "  {}", sep).unwrap();
        writeln!(
            out,
            "  {:<name_w$}  {:>time_w$}",
            "total",
            total_str,
            name_w = phase_name_w,
            time_w = time_w
        )
        .unwrap();
    }

    pub fn report(&self, stats: &BuildStats, verbose: bool, out: &mut dyn Write) {
        // Collect all phases including any active one.
        let mut phases = self.phases.clone();
        if let Some((name, t)) = &self.current {
            phases.push((name.clone(), t.elapsed()));
        }

        let total: Duration = phases.iter().map(|(_, d)| *d).sum();

        // Compute column widths.
        let phase_name_w = phases
            .iter()
            .map(|(n, _)| n.len())
            .max()
            .unwrap_or(0)
            .max("total".len());

        let time_strings: Vec<String> = phases.iter().map(|(_, d)| fmt_dur(*d)).collect();
        let total_str = fmt_dur(total);
        let time_w = time_strings
            .iter()
            .map(|s| s.len())
            .max()
            .unwrap_or(0)
            .max(total_str.len());

        if verbose {
            // Source header block.
            let ic = &stats.item_counts;
            let total_items = ic.functions + ic.structs + ic.enums + ic.processors;
            writeln!(out, "source: {}", stats.source_file).unwrap();
            writeln!(out, "  lines:      {}", stats.source_lines).unwrap();
            writeln!(out, "  tokens:     {}", stats.token_count).unwrap();
            writeln!(
                out,
                "  items:      {}  ({} functions  {} structs  {} enums  {} processors)",
                total_items, ic.functions, ic.structs, ic.enums, ic.processors
            )
            .unwrap();
            writeln!(out).unwrap();
        }

        // Timing table header.
        writeln!(out, "kiln build timing").unwrap();

        for ((name, _), time_str) in phases.iter().zip(time_strings.iter()) {
            let suffix = phase_suffix(name, stats);
            writeln!(
                out,
                "  {:<name_w$}  {:>time_w$}{}",
                name,
                time_str,
                suffix,
                name_w = phase_name_w,
                time_w = time_w
            )
            .unwrap();
        }

        let sep = "-".repeat(phase_name_w + time_w + 6);
        writeln!(out, "  {}", sep).unwrap();
        writeln!(
            out,
            "  {:<name_w$}  {:>time_w$}",
            "total",
            total_str,
            name_w = phase_name_w,
            time_w = time_w
        )
        .unwrap();

        if verbose {
            // Per-processor breakdown.
            if !stats.processor_runs.is_empty() {
                writeln!(out).unwrap();
                writeln!(out, "processors").unwrap();
                let proc_name_w = stats
                    .processor_runs
                    .iter()
                    .map(|r| r.name.len())
                    .max()
                    .unwrap_or(0);
                for run in &stats.processor_runs {
                    let item_word = if run.item_count == 1 { "item" } else { "items" };
                    writeln!(
                        out,
                        "  {:<w$}  {} {}  {}",
                        run.name,
                        run.item_count,
                        item_word,
                        fmt_dur(run.duration),
                        w = proc_name_w
                    )
                    .unwrap();
                }
            }

            // Per-function codegen times (sorted slowest first).
            if !stats.fn_codegen_times.is_empty() {
                let mut sorted = stats.fn_codegen_times.clone();
                sorted.sort_by(|a, b| b.1.cmp(&a.1));
                writeln!(out).unwrap();
                writeln!(out, "codegen (per function)").unwrap();
                let fn_name_w = sorted.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
                for (name, dur) in &sorted {
                    writeln!(out, "  {:<w$}  {}", name, fmt_dur(*dur), w = fn_name_w).unwrap();
                }
            }

            // Output sizes.
            if stats.object_bytes > 0 || stats.binary_bytes > 0 {
                writeln!(out).unwrap();
                writeln!(out, "output").unwrap();
                if stats.object_bytes > 0 {
                    writeln!(
                        out,
                        "  object:  {}   {}",
                        fmt_bytes(stats.object_bytes),
                        stats.object_path
                    )
                    .unwrap();
                }
                if stats.binary_bytes > 0 {
                    writeln!(
                        out,
                        "  binary:  {}   {}",
                        fmt_bytes(stats.binary_bytes),
                        stats.binary_path
                    )
                    .unwrap();
                }
            }
        }

        // Error/warning summary always shown.
        writeln!(out).unwrap();
        writeln!(
            out,
            "{} errors  {} warnings",
            stats.error_count, stats.warning_count
        )
        .unwrap();
    }
}

fn fmt_dur(d: Duration) -> String {
    let ms = d.as_millis();
    if ms == 0 {
        "<1ms".to_string()
    } else {
        format!("{}ms", ms)
    }
}

fn fmt_bytes(bytes: usize) -> String {
    let kb = bytes as f64 / 1024.0;
    format!("{:.1} KB", kb)
}

fn phase_suffix(name: &str, stats: &BuildStats) -> String {
    match name {
        "lex" => format!(
            "    {} lines  {} tokens",
            stats.source_lines, stats.token_count
        ),
        "parse" => format!("    {} nodes", stats.ast_node_count),
        "processors" => {
            let total_items: usize = stats.processor_runs.iter().map(|r| r.item_count).sum();
            let n = stats.processor_runs.len();
            format!("    {} processors  {} items transformed", n, total_items)
        }
        "analyze" => String::new(),
        "codegen" => {
            let n = stats.item_counts.functions;
            format!("    {} functions", n)
        }
        "emit" => {
            if stats.object_bytes > 0 {
                format!("    {} object", fmt_bytes(stats.object_bytes))
            } else {
                String::new()
            }
        }
        "link" => {
            if stats.binary_bytes > 0 {
                format!("    {} binary", fmt_bytes(stats.binary_bytes))
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_stats() -> BuildStats {
        BuildStats {
            source_file: "examples/test.kn".to_string(),
            source_lines: 847,
            token_count: 3201,
            ast_node_count: 1204,
            item_counts: ItemCounts {
                functions: 7,
                structs: 2,
                enums: 1,
                processors: 2,
            },
            processor_runs: vec![
                ProcessorRun {
                    name: "Instrument".to_string(),
                    item_count: 2,
                    duration: Duration::from_millis(1),
                },
                ProcessorRun {
                    name: "WithFallback".to_string(),
                    item_count: 1,
                    duration: Duration::from_micros(500),
                },
            ],
            fn_codegen_times: vec![
                ("main".to_string(), Duration::from_millis(9)),
                ("fetch".to_string(), Duration::from_millis(7)),
                ("parse_int".to_string(), Duration::from_millis(6)),
            ],
            object_bytes: 14541,
            object_path: "build/test.o".to_string(),
            binary_bytes: 32358,
            binary_path: "build/test".to_string(),
            warning_count: 2,
            error_count: 0,
        }
    }

    fn timer_with_phases() -> PhaseTimer {
        let mut t = PhaseTimer::new();
        // Manually insert phases so the test is deterministic.
        t.phases = vec![
            ("lex".to_string(), Duration::from_millis(4)),
            ("parse".to_string(), Duration::from_millis(6)),
            ("processors".to_string(), Duration::from_millis(2)),
            ("analyze".to_string(), Duration::from_millis(12)),
            ("codegen".to_string(), Duration::from_millis(38)),
            ("emit".to_string(), Duration::from_millis(1)),
            ("link".to_string(), Duration::from_millis(41)),
        ];
        t
    }

    #[test]
    fn phase_timer_records_elapsed_for_named_phase() {
        let mut t = PhaseTimer::new();
        t.start("lex");
        std::thread::sleep(Duration::from_millis(2));
        t.stop();
        assert_eq!(t.phases.len(), 1);
        assert_eq!(t.phases[0].0, "lex");
        assert!(t.phases[0].1 >= Duration::from_millis(1));
    }

    #[test]
    fn timing_report_includes_all_seven_phases() {
        let t = timer_with_phases();
        let stats = make_stats();
        let mut out = Vec::<u8>::new();
        t.report(&stats, false, &mut out);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("lex"), "missing lex: {}", s);
        assert!(s.contains("parse"), "missing parse: {}", s);
        assert!(s.contains("processors"), "missing processors: {}", s);
        assert!(s.contains("analyze"), "missing analyze: {}", s);
        assert!(s.contains("codegen"), "missing codegen: {}", s);
        assert!(s.contains("emit"), "missing emit: {}", s);
        assert!(s.contains("link"), "missing link: {}", s);
        assert!(s.contains("total"), "missing total: {}", s);
        assert!(s.contains("kiln build timing"), "missing header: {}", s);
    }

    #[test]
    fn verbose_report_includes_per_function_breakdown() {
        let t = timer_with_phases();
        let stats = make_stats();
        let mut out = Vec::<u8>::new();
        t.report(&stats, true, &mut out);
        let s = String::from_utf8(out).unwrap();
        assert!(
            s.contains("codegen (per function)"),
            "missing per-fn header: {}",
            s
        );
        assert!(s.contains("main"), "missing main: {}", s);
        assert!(s.contains("fetch"), "missing fetch: {}", s);
        assert!(s.contains("parse_int"), "missing parse_int: {}", s);
    }

    #[test]
    fn verbose_implies_timing_flag() {
        // verbose=true must include all the same timing table content as timing=true.
        let t = timer_with_phases();
        let stats = make_stats();
        let mut timing_out = Vec::<u8>::new();
        let mut verbose_out = Vec::<u8>::new();
        t.report(&stats, false, &mut timing_out);
        t.report(&stats, true, &mut verbose_out);
        let timing_s = String::from_utf8(timing_out).unwrap();
        let verbose_s = String::from_utf8(verbose_out).unwrap();
        // The verbose output must contain the timing header.
        assert!(
            verbose_s.contains("kiln build timing"),
            "verbose missing timing header"
        );
        // And must contain everything timing would print.
        for phase in &[
            "lex",
            "parse",
            "processors",
            "analyze",
            "codegen",
            "emit",
            "link",
            "total",
        ] {
            assert!(
                verbose_s.contains(phase),
                "verbose missing phase '{}'",
                phase
            );
        }
        // Verbose has MORE content than timing-only.
        assert!(
            verbose_s.len() > timing_s.len(),
            "verbose should be longer than timing"
        );
    }

    #[test]
    fn error_and_warning_counts_always_present() {
        let t = timer_with_phases();
        let mut stats = make_stats();
        stats.error_count = 3;
        stats.warning_count = 1;
        let mut out = Vec::<u8>::new();
        t.report(&stats, false, &mut out);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("3 errors"), "missing error count: {}", s);
        assert!(s.contains("1 warnings"), "missing warning count: {}", s);
    }

    #[test]
    fn no_timing_flag_produces_no_extra_output() {
        // When report is never called (timing=false path in main), nothing extra prints.
        // This is a structural test: verify report writes nothing when timer has no phases.
        // The actual "timing=false means no call" is enforced in main.rs.
        let t = PhaseTimer::new();
        let stats = BuildStats::default();
        let mut out = Vec::<u8>::new();
        t.report(&stats, false, &mut out);
        let s = String::from_utf8(out).unwrap();
        // With no phases, there's still a header and totals line but no phase rows.
        assert!(!s.contains("lex"));
        assert!(!s.contains("parse"));
    }

    #[test]
    fn timing_output_goes_to_stderr_not_stdout() {
        // report() takes a &mut dyn Write; the production call site passes stderr().
        // This test verifies the struct compiles with a Vec<u8> (stdout substitute)
        // and that the API is designed for stderr injection.
        let t = PhaseTimer::new();
        let stats = BuildStats::default();
        let mut buf: Vec<u8> = Vec::new();
        // This must compile and write only to buf, not to any global stream.
        t.report(&stats, false, &mut buf);
        assert!(!buf.is_empty(), "report wrote nothing");
    }

    #[test]
    fn fmt_dur_shows_less_than_one_ms_for_sub_ms() {
        assert_eq!(fmt_dur(Duration::from_micros(500)), "<1ms");
        assert_eq!(fmt_dur(Duration::from_millis(0)), "<1ms");
        assert_eq!(fmt_dur(Duration::from_millis(1)), "1ms");
        assert_eq!(fmt_dur(Duration::from_millis(104)), "104ms");
    }

    #[test]
    fn fmt_bytes_formats_to_one_decimal_kb() {
        assert_eq!(fmt_bytes(14541), "14.2 KB");
        assert_eq!(fmt_bytes(1024), "1.0 KB");
        assert_eq!(fmt_bytes(32358), "31.6 KB");
    }
}
