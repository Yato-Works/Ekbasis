//! Markdown report assembly (plan §11, §18).
//!
//! The same document is used for three things:
//!
//! * the artifact written next to the JSON results after every run,
//! * the body posted as a pull request comment by the GitHub Action, and
//! * the output of `aion report`.

use std::path::Path;

use ekbasis_benchmark::{Comparison, markdown};
use ekbasis_core::fmt;
use ekbasis_core::model::RunReport;
use anyhow::{Context as _, Result};

/// Marker that lets the GitHub Action find and update its own comment.
pub const COMMENT_MARKER: &str = "<!-- aion-report -->";

/// Everything a report needs, including the context only the CLI knows (git history, file paths).
///
/// Borrowed on purpose: a report is rendered from the in-memory results right after a run and from
/// the stored artifacts later, without cloning multi-megabyte stdout captures.
pub struct ReportContext<'a> {
    pub experiment: &'a str,
    pub baseline: Option<&'a RunReport>,
    pub candidate: &'a RunReport,
    /// Files that changed between the two measured commits (empty outside CI mode).
    pub changed_files: &'a [String],
    /// Stored JSON artifacts, relative to the repository root.
    pub reports: &'a [String],
}

/// Full Markdown document: headline, numbers, reproducibility, how to reproduce.
pub fn document(context: &ReportContext<'_>, comparison: Option<&Comparison>) -> String {
    let candidate = context.candidate;
    let mut out = String::new();

    out.push_str(&format!("# AION experiment `{}`\n\n", context.experiment));
    out.push_str(&format!("{COMMENT_MARKER}\n\n"));
    out.push_str(&format!(
        "Measured {} against {} on {}.\n\n",
        short(&candidate.commit),
        context
            .baseline
            .map(|baseline| short(&baseline.commit))
            .unwrap_or_else(|| "-".to_string()),
        candidate.machine.summary(),
    ));

    match comparison {
        Some(comparison) => out.push_str(&markdown::comparison_markdown(comparison)),
        None => out.push_str(
            "*No baseline was measured for this run (`--no-baseline`), so there is nothing to compare.*\n",
        ),
    }

    if !context.changed_files.is_empty() {
        out.push_str(&markdown::changed_files_section(
            context
                .baseline
                .map(|baseline| baseline.commit.as_str())
                .unwrap_or(&candidate.base_commit),
            &candidate.commit,
            context.changed_files,
            12,
        ));
    }

    out.push_str("\n## Reproducibility\n\n");
    out.push_str("| field | value |\n| --- | --- |\n");
    out.push_str(&format!(
        "| baseline | {} |\n",
        context
            .baseline
            .map(|baseline| short(&baseline.commit))
            .unwrap_or_else(|| "not measured".to_string())
    ));
    out.push_str(&format!("| experiment | {} |\n", short(&candidate.commit)));
    out.push_str(&format!(
        "| iterations | {} measured, {} warmup |\n",
        candidate.iterations, candidate.warmup
    ));
    if let Some(baseline) = context.baseline {
        out.push_str(&format!("| baseline build | {} |\n", build_text(baseline)));
    }
    out.push_str(&format!(
        "| experiment build | {} |\n",
        build_text(candidate)
    ));
    out.push_str(&format!(
        "| commands | {} |\n",
        candidate
            .repro
            .commands
            .phases()
            .iter()
            .map(|(phase, command)| format!("`{phase}` {command}"))
            .collect::<Vec<_>>()
            .join("<br>")
    ));
    if !candidate.environment.is_empty() {
        out.push_str(&format!(
            "| environment | {} |\n",
            candidate
                .environment
                .iter()
                .map(|(name, value)| format!("`{name}={value}`"))
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    if let Some(gpus) = candidate.machine.gpu_summary() {
        out.push_str(&format!("| gpus | {gpus} |\n"));
    }
    if let Some(free) = candidate.machine.disk_free_bytes {
        out.push_str(&format!("| free disk | {} |\n", fmt::bytes(free)));
    }
    if !context.reports.is_empty() {
        out.push_str(&format!(
            "| artifacts | {} |\n",
            context
                .reports
                .iter()
                .map(|path| format!("`{path}`"))
                .collect::<Vec<_>>()
                .join("<br>")
        ));
    }
    out.push_str(&format!("| aion | v{} |\n", candidate.aion_version));

    out.push_str("\n## Reproduce\n\n```bash\n");
    out.push_str(&format!(
        "# from a checkout of the measured commit\naion experiment run {} --force\n",
        context.experiment
    ));
    out.push_str("```\n");

    if !candidate.warnings.is_empty() {
        out.push_str("\n## Run warnings\n\n");
        for warning in &candidate.warnings {
            out.push_str(&format!("- {warning}\n"));
        }
    }
    out
}

/// Compact body for a pull request comment.
pub fn comment(context: &ReportContext<'_>, comparison: Option<&Comparison>) -> String {
    let candidate = context.candidate;
    let mut out = String::new();
    out.push_str(&format!("{COMMENT_MARKER}\n"));
    out.push_str(&format!("### AION experiment `{}`\n\n", context.experiment));

    match comparison {
        Some(comparison) => out.push_str(&markdown::comparison_markdown(comparison)),
        None => out.push_str(
            "*No baseline was measured for this run, so there is nothing to compare.*\n",
        ),
    }

    if !context.changed_files.is_empty() {
        out.push_str(&markdown::changed_files_section(
            context
                .baseline
                .map(|baseline| baseline.commit.as_str())
                .unwrap_or(&candidate.base_commit),
            &candidate.commit,
            context.changed_files,
            8,
        ));
    }

    out.push_str(&format!(
        "\n---\n{} -> {} · {} iterations · {} · AION v{}\n",
        context
            .baseline
            .map(|baseline| short(&baseline.commit))
            .unwrap_or_else(|| "no baseline".to_string()),
        short(&candidate.commit),
        candidate.iterations,
        candidate.machine.summary(),
        candidate.aion_version,
    ));
    out.push_str(
        "<sub>posted by AION · every number above comes from a process AION actually ran</sub>\n",
    );
    out
}

/// Writes a report, creating parent directories as needed.
pub fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create `{}`", parent.display()))?;
    }
    std::fs::write(path, text).with_context(|| format!("cannot write `{}`", path.display()))
}

/// Short commit hash in backticks, for prose.
fn short(commit: &str) -> String {
    format!("`{}`", fmt::short_sha(commit))
}

/// Build time of a report, or a dash when no build command was configured.
fn build_text(report: &RunReport) -> String {
    match report.build_ms {
        Some(milliseconds) => fmt::duration_ms(milliseconds),
        None => "-".to_string(),
    }
}
