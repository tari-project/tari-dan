//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Turns a dead child process into something readable on the swarm daemon's own stdout.
//!
//! A crashed node is otherwise silent here: the reason is buried in one of its log files, which the operator has to
//! know to go and find.

use std::{fmt::Write, path::Path};

use crate::{logfile, process_manager::Instance};

/// Lines of context shown when there is no panic to point at.
const TAIL_LINES: usize = 50;
/// Lines of a panic's backtrace worth showing before it turns into noise.
const PANIC_CONTEXT_LINES: usize = 30;

/// Renders the panic message, or failing that the tail of the process's output, as an indented block.
pub fn crash_report(instance: &Instance) -> String {
    let mut report = String::new();
    // Rust panics go to stderr, so it is both the likelier place to find one and the shorter file to scan.
    for path in [instance.stderr_log_path(), instance.stdout_log_path()] {
        let Some(section) = report_for(&path) else {
            continue;
        };
        let _ = write!(report, "\n  ── {} ──\n{}", path.display(), section);
    }

    if report.is_empty() {
        format!(
            "\n  ── no output captured for {} in {} ──\n",
            instance.name(),
            instance.stdout_log_path().display()
        )
    } else {
        report
    }
}

fn report_for(path: &Path) -> Option<String> {
    let lines = logfile::tail_lines(path, TAIL_LINES.max(PANIC_CONTEXT_LINES)).ok()?;
    if lines.is_empty() {
        return None;
    }

    let lines = match find_panic(&lines) {
        Some(at) => &lines[at..(at + PANIC_CONTEXT_LINES).min(lines.len())],
        None => &lines[lines.len().saturating_sub(TAIL_LINES)..],
    };

    Some(lines.iter().fold(String::new(), |mut out, line| {
        let _ = writeln!(out, "  {line}");
        out
    }))
}

/// Index of the last panic header in `lines`, so that a process that recovered from an earlier panic still reports the
/// one it died on.
fn find_panic(lines: &[String]) -> Option<usize> {
    lines
        .iter()
        .rposition(|line| line.contains("panicked at") || line.contains("fatal runtime error"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(ToString::to_string).collect()
    }

    #[test]
    fn the_last_panic_wins() {
        let found = find_panic(&lines(
            "thread 'main' panicked at src/a.rs:1\nrecovered\nthread 'x' panicked at src/b.rs:2\nnote: backtrace",
        ));
        assert_eq!(found, Some(2));
    }

    #[test]
    fn clean_output_has_no_panic() {
        assert_eq!(find_panic(&lines("starting\nready\n")), None);
    }

    fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("swarm-crash-{name}.log"));
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn a_panic_is_reported_with_its_backtrace_and_nothing_before_it() {
        let mut body = (0..200).map(|i| format!("noise {i}\n")).collect::<String>();
        body.push_str("thread 'tokio-runtime-worker' panicked at crates/consensus/src/lib.rs:12:9:\n");
        body.push_str("assertion failed: leaf.height() > 0\n");
        body.push_str("note: run with `RUST_BACKTRACE=1` for a backtrace\n");

        let report = report_for(&write_temp("panic", &body)).unwrap();
        assert!(
            report.contains("panicked at crates/consensus/src/lib.rs:12:9"),
            "{report}"
        );
        assert!(report.contains("assertion failed: leaf.height() > 0"), "{report}");
        assert!(!report.contains("noise 199"), "{report}");
    }

    #[test]
    fn output_without_a_panic_falls_back_to_the_tail() {
        let body = (0..200).map(|i| format!("noise {i}\n")).collect::<String>();

        let report = report_for(&write_temp("tail", &body)).unwrap();
        assert!(report.contains("noise 199"), "{report}");
        assert!(report.contains("noise 150"), "{report}");
        assert!(!report.contains("noise 149"), "{report}");
    }

    #[test]
    fn a_missing_or_empty_file_reports_nothing() {
        assert!(report_for(&write_temp("blank", "")).is_none());
        assert!(report_for(std::path::Path::new("/nonexistent/stderr.log")).is_none());
    }
}
