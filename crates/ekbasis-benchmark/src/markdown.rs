//! Markdown rendering of comparison results (plan §11, §18).
//!
//! The GitHub Action posts [`comparison_markdown`] as a pull request comment and stores the same
//! document as a build artifact, so a review and the CI log show exactly the same numbers.

use ekbasis_core::fmt;

use crate::statistics::{histogram, sparkline};
use crate::{Comparison, MetricDelta, Verdict};

/// Markdown table of the metrics; `secondary = false` selects the metrics that carry the verdict.
pub fn metric_table(metrics: &[MetricDelta], secondary: bool) -> String {
    let mut out = String::new();
    out.push_str("| metric | baseline | experiment | difference | verdict |\n");
    out.push_str("| --- | ---: | ---: | ---: | :--- |\n");
    for metric in metrics.iter().filter(|metric| metric.secondary == secondary) {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            escape(&metric.name),
            metric.baseline_text(),
            metric.candidate_text(),
            metric.delta_text(),
            verdict_cell(metric.verdict),
        ));
    }
    out
}

/// Verdict as Markdown: regressions stand out even in a rendered table.
fn verdict_cell(verdict: Verdict) -> String {
    match verdict {
        Verdict::Improved => "improved".to_string(),
        Verdict::Regressed => "**regressed**".to_string(),
        Verdict::Neutral => "neutral".to_string(),
        Verdict::Unavailable => "n/a".to_string(),
    }
}

/// Escapes the characters that would break a Markdown table cell.
fn escape(text: &str) -> String {
    text.replace('|', "\\|")
}

/// Headline line, e.g. `startup (mean): -52.6% (improved)`.
pub fn headline(comparison: &Comparison) -> String {
    let Some(metric) = comparison.primary_metric() else {
        return "no comparable measurement".to_string();
    };
    match metric.delta_percent {
        Some(delta) => format!(
            "**{}: {} ({})**",
            metric.name,
            fmt::delta_percent(delta),
            metric.verdict.as_str()
        ),
        None => format!("**{}: not measured on both sides**", metric.name),
    }
}

/// Fatest distribution block: description, measurement-order sparkline and histogram.
pub fn graph_block(comparison: &Comparison) -> String {
    let mut out = String::new();
    for (label, samples, distribution) in [
        (
            "baseline",
            &comparison.baseline_samples,
            comparison.baseline_distribution.as_ref(),
        ),
        (
            "experiment",
            &comparison.candidate_samples,
            comparison.candidate_distribution.as_ref(),
        ),
    ] {
        if samples.is_empty() {
            continue;
        }
        if let Some(distribution) = distribution {
            out.push_str(&format!("{label}: {}\n", distribution.describe()));
        }
        if samples.len() > 1 {
            out.push_str(&format!("{label} (order): {}\n", sparkline(samples)));
        }
        if samples.len() >= 4 {
            for line in histogram(samples, 4, 20) {
                out.push_str(&format!("{label} {line}\n"));
            }
        }
    }
    out
}

/// Lists the files that changed between the two measured commits (CI comparisons).
pub fn changed_files_section(
    base_commit: &str,
    candidate_commit: &str,
    files: &[String],
    limit: usize,
) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "\n**Changed files** (`{}..{}`, {} total)\n\n",
        fmt::short_sha(base_commit),
        fmt::short_sha(candidate_commit),
        files.len()
    );
    for file in files.iter().take(limit) {
        out.push_str(&format!("- `{file}`\n"));
    }
    if files.len() > limit {
        out.push_str(&format!("- ... and {} more\n", files.len() - limit));
    }
    out
}

/// The comparison as Markdown: headline, tables, statistics, correctness, warnings and graphs.
pub fn comparison_markdown(comparison: &Comparison) -> String {
    let mut out = String::new();
    out.push_str(&format!("{}\n\n", headline(comparison)));
    out.push_str(&metric_table(&comparison.metrics, false));

    let context_names = comparison.context_metric_names();
    if !context_names.is_empty() {
        out.push_str(&format!(
            "\n<details><summary>context metrics ({})</summary>\n\n",
            context_names.join(", ")
        ));
        out.push_str(&metric_table(&comparison.metrics, true));
        out.push_str("\n</details>\n");
    }

    if let Some(significance) = &comparison.significance {
        out.push_str(&format!("\n{}\n", significance.describe()));
    }

    out.push_str("\n**correctness**\n\n");
    for check in &comparison.checks {
        out.push_str(&format!(
            "- `{}` {} - {}\n",
            check.state.as_str(),
            check.name,
            check.detail
        ));
    }

    let (improved, regressed, neutral) = comparison.tally();
    out.push_str(&format!(
        "\n{improved} improved, {regressed} regressed, {neutral} neutral (threshold +/-{:.1}%)\n",
        comparison.threshold_percent
    ));

    if !comparison.warnings.is_empty() {
        out.push_str("\n<details><summary>reliability warnings</summary>\n\n");
        for warning in &comparison.warnings {
            out.push_str(&format!("- {warning}\n"));
        }
        out.push_str("\n</details>\n");
    }

    let graphs = graph_block(comparison);
    if !graphs.is_empty() {
        out.push_str(&format!(
            "\n<details><summary>distribution</summary>\n\n```text\n{graphs}```\n\n</details>\n"
        ));
    }
    out
}
