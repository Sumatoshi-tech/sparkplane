use crate::core::stream::BlockHandler;
use crate::core::truncate::{CAP_ERRORS, CAP_WARNINGS};
use crate::core::utils::{join_with_overflow, truncate};
use serde::Deserialize;
use std::sync::LazyLock;
struct CargoBuildHandler {
    compiled: usize,
    warnings: usize,
    error_count: usize,
    finished_line: Option<String>,
    label: &'static str,
}

impl CargoBuildHandler {
    fn with_label(label: &'static str) -> Self {
        Self {
            compiled: 0,
            warnings: 0,
            error_count: 0,
            finished_line: None,
            label,
        }
    }
}

impl BlockHandler for CargoBuildHandler {
    fn should_skip(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start();
        if trimmed.starts_with("Compiling") || trimmed.starts_with("Checking") {
            self.compiled += 1;
            return true;
        }
        if trimmed.starts_with("Downloading") || trimmed.starts_with("Downloaded") {
            return true;
        }
        if trimmed.starts_with("Finished") {
            self.finished_line = Some(trimmed.to_string());
            return true;
        }
        if line.starts_with("warning:") && line.contains("generated") && line.contains("warning") {
            return true;
        }
        if (line.starts_with("error:") || line.starts_with("error["))
            && (line.contains("aborting due to") || line.contains("could not compile"))
        {
            return true;
        }
        false
    }

    fn is_block_start(&mut self, line: &str) -> bool {
        if line.starts_with("error[") || line.starts_with("error:") {
            self.error_count += 1;
            return true;
        }
        if line.starts_with("warning:") || line.starts_with("warning[") {
            self.warnings += 1;
            return true;
        }
        false
    }

    fn is_block_continuation(&mut self, line: &str, block: &[String]) -> bool {
        !(line.trim().is_empty() && block.len() > 3)
    }

    fn format_summary(&self, exit_code: i32, raw: &str) -> Option<String> {
        if self.error_count == 0 && self.warnings == 0 && exit_code == 0 {
            let summary =
                cargo_build_success_line(self.compiled, self.finished_line.as_deref(), self.label);
            return Some(crate::core::guard::never_worse(raw, &summary).to_string());
        }
        // The streamed path only runs for non-json build/check; error blocks are
        // emitted live, so the summary carries no rendered diagnostics.
        let empty = JsonDiagnostics {
            errors: Vec::new(),
            warnings: Vec::new(),
        };
        Some(cargo_build_failure_summary(
            self.compiled,
            self.error_count,
            self.warnings,
            &empty,
            self.label,
            exit_code,
        ))
    }
}

struct CargoTestHandler {
    in_failure_section: bool,
    in_failure_names: bool,
    summary_lines: Vec<String>,
    has_compile_errors: bool,
}

impl CargoTestHandler {
    fn new() -> Self {
        Self {
            in_failure_section: false,
            in_failure_names: false,
            summary_lines: Vec::new(),
            has_compile_errors: false,
        }
    }

    /// Compacted `cargo test` summary, before the never-worse guard.
    fn compute_test_summary(&self, raw: &str) -> Option<String> {
        if self.summary_lines.is_empty() {
            let json = extract_json_diagnostics(raw);
            if self.has_compile_errors || !json.errors.is_empty() {
                // Content-based (exit 0): a real compile error yields "cargo test: N
                // errors"; a bare "could not compile" leaves the raw tail fallback.
                let build_filtered = filter_cargo_build_labeled(raw, "test", 0);
                if build_filtered.contains("cargo test:") {
                    return Some(format!("{}\n", build_filtered));
                }
                // Fallback: last 5 meaningful lines
                let meaningful: Vec<&str> = raw
                    .lines()
                    .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with("Compiling"))
                    .collect();
                let last5: Vec<&str> = meaningful.iter().rev().take(5).rev().copied().collect();
                return Some(format!("{}\n", last5.join("\n")));
            }
        }

        // No failures emitted — aggregate pass results
        let mut aggregated: Option<AggregatedTestResult> = None;
        let mut all_parsed = true;

        for line in &self.summary_lines {
            if let Some(parsed) = AggregatedTestResult::parse_line(line) {
                if let Some(ref mut agg) = aggregated {
                    agg.merge(&parsed);
                } else {
                    aggregated = Some(parsed);
                }
            } else {
                all_parsed = false;
                break;
            }
        }

        if all_parsed
            && let Some(agg) = aggregated
            && agg.suites > 0
        {
            return Some(format!("{}\n", agg.format_compact()));
        }

        // Fallback: show raw summary lines
        if !self.summary_lines.is_empty() {
            let mut s = String::new();
            for line in &self.summary_lines {
                s.push_str(line);
                s.push('\n');
            }
            return Some(s);
        }

        None
    }
}

impl BlockHandler for CargoTestHandler {
    fn should_skip(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start();
        if trimmed.starts_with("Compiling")
            || trimmed.starts_with("Downloading")
            || trimmed.starts_with("Downloaded")
            || trimmed.starts_with("Finished")
        {
            return true;
        }
        if line.starts_with("running ") {
            return true;
        }
        if line.starts_with("test ") && line.ends_with("... ok") {
            return true;
        }
        // Track compile errors for fallback
        if trimmed.starts_with("error[") || trimmed.starts_with("error:") {
            self.has_compile_errors = true;
        }
        // "failures:" toggles section state
        if line == "failures:" {
            if self.in_failure_section {
                // Second "failures:" = list of failure names — skip them
                self.in_failure_names = true;
            }
            self.in_failure_section = true;
            return true;
        }
        // Skip the failure name listing section
        if self.in_failure_names {
            if line.starts_with("test result:") {
                self.in_failure_names = false;
                self.in_failure_section = false;
                self.summary_lines.push(line.to_string());
                return true;
            }
            return true;
        }
        if line.starts_with("test result:") {
            self.summary_lines.push(line.to_string());
            self.in_failure_section = false;
            return true;
        }
        false
    }

    fn is_block_start(&mut self, line: &str) -> bool {
        self.in_failure_section && line.starts_with("---- ")
    }

    fn is_block_continuation(&mut self, line: &str, _block: &[String]) -> bool {
        self.in_failure_section && !line.starts_with("---- ")
    }

    fn format_summary(&self, _exit_code: i32, raw: &str) -> Option<String> {
        // Same never-worse guard as CargoBuildHandler (#3430 review): if the
        // compacted summary ends up larger than the raw output (e.g. a tiny
        // `cargo test` run), keep the raw output instead of "compacting" it
        // into something bigger.
        let summary = self.compute_test_summary(raw)?;
        Some(crate::core::guard::never_worse(raw, &summary).to_string())
    }
}

/// Generic cargo command runner with filtering.
/// Builds the Command with restored `--` separator, then delegates to shared runner.

struct JsonDiagnostics {
    errors: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Deserialize)]
struct CargoJsonLine {
    reason: String,
    message: Option<CargoDiagnostic>,
}

#[derive(Deserialize)]
struct CargoDiagnostic {
    level: String,
    #[serde(default)]
    message: String,
    rendered: Option<String>,
}

fn extract_json_diagnostics(raw: &str) -> JsonDiagnostics {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    for line in raw.lines() {
        let line = line.trim_start();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<CargoJsonLine>(line) else {
            continue;
        };
        if entry.reason != "compiler-message" {
            continue;
        }
        let Some(msg) = entry.message else {
            continue;
        };
        let bucket = match msg.level.as_str() {
            "error" | "error: internal compiler error" => &mut errors,
            "warning" => &mut warnings,
            _ => continue,
        };
        if msg.message.starts_with("aborting due to")
            || msg.message.starts_with("could not compile")
            || (msg.message.contains("warning") && msg.message.contains("generated"))
        {
            continue;
        }
        if let Some(rendered) = msg.rendered {
            bucket.push(
                crate::core::utils::strip_ansi(&rendered)
                    .trim_end()
                    .to_string(),
            );
        } else if !msg.message.is_empty() {
            bucket.push(msg.message);
        }
    }
    JsonDiagnostics { errors, warnings }
}

fn merge_diag_counts(
    error_count: usize,
    warnings: usize,
    json: &JsonDiagnostics,
) -> (usize, usize) {
    (
        error_count.max(json.errors.len()),
        warnings.max(json.warnings.len()),
    )
}

fn cargo_build_success_line(compiled: usize, finished: Option<&str>, label: &str) -> String {
    match finished {
        Some(f) => format!("cargo {} ({} crates compiled)\n{}\n", label, compiled, f),
        None => format!("cargo {} ({} crates compiled)\n", label, compiled),
    }
}

fn cargo_build_failure_summary(
    compiled: usize,
    errors: usize,
    warnings: usize,
    json: &JsonDiagnostics,
    label: &str,
    exit_code: i32,
) -> String {
    let mut out = if errors == 0 && warnings == 0 {
        format!("cargo {}: failed (exit {})\n", label, exit_code)
    } else {
        format!(
            "cargo {}: {} errors, {} warnings ({} crates)\n",
            label, errors, warnings, compiled
        )
    };
    if !json.errors.is_empty() {
        let shown: Vec<String> = json.errors.iter().take(CAP_ERRORS).cloned().collect();
        out.push_str(&join_with_overflow(
            &shown,
            json.errors.len(),
            CAP_ERRORS,
            "errors",
        ));
        out.push('\n');
    }
    if !json.warnings.is_empty() {
        let shown: Vec<String> = json.warnings.iter().take(CAP_WARNINGS).cloned().collect();
        out.push_str(&join_with_overflow(
            &shown,
            json.warnings.len(),
            CAP_WARNINGS,
            "warnings",
        ));
        out.push('\n');
    }
    if json.errors.len() > CAP_ERRORS || json.warnings.len() > CAP_WARNINGS {
        let full = json
            .errors
            .iter()
            .chain(json.warnings.iter())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n\n");
        if let Some(hint) = crate::core::tee::force_tee_hint(&full, "cargo-json-issues") {
            out.push_str(&format!("  {}\n", hint));
        }
    }
    out
}

fn filter_cargo_build(output: &str) -> String {
    filter_cargo_build_labeled(output, "build", 0)
}

pub fn filter_cargo_build_labeled(output: &str, label: &'static str, exit_code: i32) -> String {
    let mut handler = CargoBuildHandler::with_label(label);
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let mut current_block: Vec<String> = Vec::new();
    let mut in_block = false;

    for line in output.lines() {
        if handler.should_skip(line) {
            continue;
        }
        if handler.is_block_start(line) {
            if in_block && !current_block.is_empty() {
                blocks.push(std::mem::take(&mut current_block));
            }
            in_block = true;
            current_block.push(line.to_string());
        } else if in_block {
            if handler.is_block_continuation(line, &current_block) {
                current_block.push(line.to_string());
            } else {
                blocks.push(std::mem::take(&mut current_block));
                in_block = false;
            }
        }
    }
    if !current_block.is_empty() {
        blocks.push(current_block);
    }

    let json = extract_json_diagnostics(output);
    let (errors, warnings) = merge_diag_counts(handler.error_count, handler.warnings, &json);

    if errors == 0 && warnings == 0 && exit_code == 0 {
        let summary =
            cargo_build_success_line(handler.compiled, handler.finished_line.as_deref(), label);
        return crate::core::guard::never_worse(output, &summary).to_string();
    }

    let mut result =
        cargo_build_failure_summary(handler.compiled, errors, warnings, &json, label, exit_code);
    const MAX_CHECK_BLOCKS: usize = CAP_ERRORS;
    for (i, blk) in blocks.iter().enumerate().take(MAX_CHECK_BLOCKS) {
        result.push_str(&blk.join("\n"));
        result.push('\n');
        if i < blocks.len() - 1 {
            result.push('\n');
        }
    }
    if blocks.len() > MAX_CHECK_BLOCKS {
        result.push_str(&format!(
            "\n… +{} more issues\n",
            blocks.len() - MAX_CHECK_BLOCKS
        ));
        let all_blocks: String = blocks
            .iter()
            .map(|b| b.join("\n"))
            .collect::<Vec<_>>()
            .join("\n\n");
        if let Some(hint) = crate::core::tee::force_tee_hint(&all_blocks, "cargo-check-issues") {
            result.push_str(&format!("  {}\n", hint));
        }
    }
    result.trim().to_string()
}

/// Aggregated test results for compact display
#[derive(Debug, Default, Clone)]
struct AggregatedTestResult {
    passed: usize,
    failed: usize,
    ignored: usize,
    measured: usize,
    filtered_out: usize,
    suites: usize,
    duration_secs: f64,
    has_duration: bool,
}

impl AggregatedTestResult {
    /// Parse a test result summary line
    /// Format: "test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s"
    fn parse_line(line: &str) -> Option<Self> {
        static RE: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(
                r"test result: (\w+)\.\s+(\d+) passed;\s+(\d+) failed;\s+(\d+) ignored;\s+(\d+) measured;\s+(\d+) filtered out(?:;\s+finished in ([\d.]+)s)?"
            ).unwrap()
        });

        let caps = RE.captures(line)?;
        let status = caps.get(1)?.as_str();

        // Only aggregate if status is "ok" (all tests passed)
        if status != "ok" {
            return None;
        }

        let passed = caps.get(2)?.as_str().parse().ok()?;
        let failed = caps.get(3)?.as_str().parse().ok()?;
        let ignored = caps.get(4)?.as_str().parse().ok()?;
        let measured = caps.get(5)?.as_str().parse().ok()?;
        let filtered_out = caps.get(6)?.as_str().parse().ok()?;

        let (duration_secs, has_duration) = if let Some(duration_match) = caps.get(7) {
            (duration_match.as_str().parse().unwrap_or(0.0), true)
        } else {
            (0.0, false)
        };

        Some(Self {
            passed,
            failed,
            ignored,
            measured,
            filtered_out,
            suites: 1,
            duration_secs,
            has_duration,
        })
    }

    /// Merge another test result into this one
    fn merge(&mut self, other: &Self) {
        self.passed += other.passed;
        self.failed += other.failed;
        self.ignored += other.ignored;
        self.measured += other.measured;
        self.filtered_out += other.filtered_out;
        self.suites += other.suites;
        self.duration_secs += other.duration_secs;
        self.has_duration = self.has_duration && other.has_duration;
    }

    /// Format as compact single line
    fn format_compact(&self) -> String {
        let mut parts = vec![format!("{} passed", self.passed)];

        if self.ignored > 0 {
            parts.push(format!("{} ignored", self.ignored));
        }
        if self.filtered_out > 0 {
            parts.push(format!("{} filtered out", self.filtered_out));
        }

        let counts = parts.join(", ");

        let suite_text = if self.suites == 1 {
            "1 suite".to_string()
        } else {
            format!("{} suites", self.suites)
        };

        if self.has_duration {
            format!(
                "cargo test: {} ({}, {:.2}s)",
                counts, suite_text, self.duration_secs
            )
        } else {
            format!("cargo test: {} ({})", counts, suite_text)
        }
    }
}

pub(crate) fn filter_cargo_test(output: &str) -> String {
    let mut failures: Vec<String> = Vec::new();
    let mut summary_lines: Vec<String> = Vec::new();
    let mut in_failure_section = false;
    let mut current_failure = Vec::new();

    for line in output.lines() {
        // Skip compilation lines
        if line.trim_start().starts_with("Compiling")
            || line.trim_start().starts_with("Downloading")
            || line.trim_start().starts_with("Downloaded")
            || line.trim_start().starts_with("Finished")
        {
            continue;
        }

        // Skip "running N tests" and individual "test ... ok" lines
        if line.starts_with("running ") || (line.starts_with("test ") && line.ends_with("... ok")) {
            continue;
        }

        // Detect failures section
        if line == "failures:" {
            in_failure_section = true;
            continue;
        }

        if in_failure_section {
            if line.starts_with("test result:") {
                in_failure_section = false;
                summary_lines.push(line.to_string());
            } else if line.starts_with("    ") || line.starts_with("---- ") {
                current_failure.push(line.to_string());
            } else if line.trim().is_empty() && !current_failure.is_empty() {
                failures.push(current_failure.join("\n"));
                current_failure.clear();
            } else if !line.trim().is_empty() {
                current_failure.push(line.to_string());
            }
        }

        // Capture test result summary
        if !in_failure_section && line.starts_with("test result:") {
            summary_lines.push(line.to_string());
        }
    }

    if !current_failure.is_empty() {
        failures.push(current_failure.join("\n"));
    }

    let mut result = String::new();

    if failures.is_empty() && !summary_lines.is_empty() {
        // All passed - try to aggregate
        let mut aggregated: Option<AggregatedTestResult> = None;
        let mut all_parsed = true;

        for line in &summary_lines {
            if let Some(parsed) = AggregatedTestResult::parse_line(line) {
                if let Some(ref mut agg) = aggregated {
                    agg.merge(&parsed);
                } else {
                    aggregated = Some(parsed);
                }
            } else {
                all_parsed = false;
                break;
            }
        }

        // If all lines parsed successfully and we have at least one suite, return compact format
        if all_parsed
            && let Some(agg) = aggregated
            && agg.suites > 0
        {
            return agg.format_compact();
        }

        // Fallback: use original behavior if regex failed
        for line in &summary_lines {
            result.push_str(&format!("{}\n", line));
        }
        return result.trim().to_string();
    }

    if !failures.is_empty() {
        result.push_str(&format!("FAILURES ({}):\n", failures.len()));
        const MAX_FAILURES: usize = CAP_WARNINGS;
        for (i, failure) in failures.iter().enumerate().take(MAX_FAILURES) {
            result.push_str(&format!("{}. {}\n", i + 1, truncate(failure, 200)));
        }
        if failures.len() > MAX_FAILURES {
            result.push_str(&format!(
                "\n… +{} more failures\n",
                failures.len() - MAX_FAILURES
            ));
            let all_failures = failures.join("\n\n");
            if let Some(hint) =
                crate::core::tee::force_tee_hint(&all_failures, "cargo-test-failures")
            {
                result.push_str(&format!("  {}\n", hint));
            }
        }
        result.push('\n');
    }

    for line in &summary_lines {
        result.push_str(&format!("{}\n", line));
    }

    if result.trim().is_empty() {
        let json = extract_json_diagnostics(output);
        let has_compile_errors = !json.errors.is_empty()
            || output.lines().any(|line| {
                let trimmed = line.trim_start();
                trimmed.starts_with("error[") || trimmed.starts_with("error:")
            });

        if has_compile_errors {
            let build_filtered = filter_cargo_build_labeled(output, "test", 0);
            if build_filtered.contains("cargo test:") {
                return build_filtered;
            }
        }

        // Fallback: show last meaningful lines
        let meaningful: Vec<&str> = output
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with("Compiling"))
            .collect();
        for line in meaningful.iter().rev().take(5).rev() {
            result.push_str(&format!("{}\n", line));
        }
    }

    result.trim().to_string()
}
