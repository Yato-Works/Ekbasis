//! `aion compare` — compare two measured runs side by side.

use std::path::{Path, PathBuf};

use ekbasis_benchmark::{Comparison, compare_reports};
use ekbasis_core::model::{RunKind, RunReport};
use ekbasis_storage::{RunRow, Store};
use anyhow::{Context as _, Result, bail};

use crate::commands::{Context as CommandContext, resolve_threshold};
use crate::output;
use crate::CompareArgs;

/// One resolved side of a comparison.
struct ResolvedRun {
    label: String,
    report: RunReport,
}

/// `aion compare <left> <right>`
pub fn run(args: &CompareArgs, verbose: bool) -> Result<()> {
    let context = CommandContext::load(verbose)?;
    let left = resolve_side(&context, &args.left)?;
    let right = resolve_side(&context, &args.right)?;
    let threshold = resolve_threshold(&context.config, args.threshold, None);

    println!("{}", output::banner("compare"));
    let comparison = compare_reports(&left.report, &right.report, threshold);
    println!("  {} (left, baseline)", left.label);
    println!("  {} (right, experiment)", right.label);
    println!();
    print!("{}", output::indent(&comparison.render(), 2));
    print_notes(&comparison);
    Ok(())
}

/// Prints the warnings and the machine context of a comparison.
fn print_notes(comparison: &Comparison) {
    if !comparison.warnings.is_empty() {
        println!("\nnotes");
        for warning in &comparison.warnings {
            println!("  warn  {warning}");
        }
    }
}

/// Resolves a user supplied token into a run report.
///
/// Accepted forms:
///
/// * `42` — a run id from `.aion/timeline.db`
/// * `path/to/report.json` — a stored report
/// * `<experiment>:baseline` / `<experiment>:experiment` — newest run of that kind
/// * `<experiment>` — newest run of that experiment (experiment run preferred)
fn resolve_side(context: &CommandContext, token: &str) -> Result<ResolvedRun> {
    // 1. run id
    if let Ok(id) = token.parse::<i64>() {
        let row = context
            .store
            .run_by_id(id)?
            .with_context(|| format!("there is no run #{id} in this repository"))?;
        return report_for_row(context, &row);
    }

    // 2. explicit report file
    let candidate = PathBuf::from(token);
    if candidate.is_file() {
        let report = read_report(&candidate)?;
        return Ok(ResolvedRun {
            label: candidate.display().to_string(),
            report,
        });
    }

    // 3. `<experiment>:<kind>`
    if let Some((name, kind_text)) = token.split_once(':') {
        let kind = RunKind::parse(kind_text)
            .with_context(|| format!("`{kind_text}` is not a run kind (use `baseline` or `experiment`)"))?;
        let row = require_run(&context.store, name, Some(kind))?;
        return report_for_row(context, &row);
    }

    // 4. bare experiment name
    let row = require_run(&context.store, token, None)?;
    report_for_row(context, &row)
}

/// Finds the newest run of an experiment; prefers experiment runs when no kind is given.
fn require_run(store: &Store, name: &str, kind: Option<RunKind>) -> Result<RunRow> {
    let experiment = store
        .experiment_by_name(name)?
        .with_context(|| format!("no experiment named `{name}` in this repository"))?;
    let runs = store.runs_for_experiment(experiment.id)?;
    if runs.is_empty() {
        bail!(
            "experiment `{name}` has no recorded runs yet - run `ekbasis experiment run {name}` first"
        );
    }
    let chosen = match kind {
        Some(kind) => runs
            .iter()
            .find(|run| run.kind == kind)
            .with_context(|| format!("experiment `{name}` has no `{kind}` run yet"))?,
        None => runs
            .iter()
            .find(|run| run.kind == RunKind::Experiment)
            .unwrap_or(&runs[0]),
    };
    Ok(chosen.clone())
}

/// Loads the JSON report a database row points at.
fn report_for_row(context: &CommandContext, row: &RunRow) -> Result<ResolvedRun> {
    let path = row
        .report_path
        .as_ref()
        .map(|path| context.paths.repo_root().join(path))
        .with_context(|| {
            format!(
                "run #{} has no stored report file (created by an older AION version?)",
                row.id
            )
        })?;
    let report = read_report(&path).with_context(|| {
        format!(
            "cannot read the report of run #{} — was the results directory deleted?",
            row.id
        )
    })?;
    Ok(ResolvedRun {
        label: format!("#{} {}", row.id, row.label),
        report,
    })
}

/// Reads a JSON report from disk.
fn read_report(path: &Path) -> Result<RunReport> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read `{}`", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("cannot parse `{}`", path.display()))
}
