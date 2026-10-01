//! `aion report` — Markdown reports from stored results (plan §11, §18).
//!
//! `--format markdown` produces the full document (CI artifact, step summary), `--format comment`
//! the compact body the GitHub Action posts on a pull request.

use ekbasis_benchmark::{Comparison, compare_reports};
use ekbasis_core::model::{RunKind, RunReport};
use anyhow::{Context as _, Result, bail};

use crate::ReportArgs;
use crate::commands::{Context, resolve_threshold};
use crate::output;
use crate::report::{self, ReportContext};

/// One experiment's stored results, ready to be rendered.
struct Loaded {
    experiment: String,
    baseline: Option<RunReport>,
    candidate: RunReport,
    baseline_report_path: Option<String>,
    candidate_report_path: Option<String>,
    comparison: Option<Comparison>,
}

/// `aion report [names...] [--all] [--format markdown|comment] [--out FILE]`
pub fn run(args: &ReportArgs, verbose: bool) -> Result<()> {
    let context = Context::load(verbose)?;
    let names = if args.all {
        context
            .store
            .list_experiments()?
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>()
    } else {
        args.names.clone()
    };
    if names.is_empty() {
        bail!(
            "no experiments to report on — pass experiment names or use `--all` (see `ekbasis experiment list`)"
        );
    }

    let mut documents: Vec<String> = Vec::new();
    let mut comments: Vec<String> = Vec::new();
    for name in &names {
        let loaded = load(&context, name)?;
        let changed_files = match (&loaded.baseline, &loaded.candidate.commit) {
            (Some(baseline), candidate) => context
                .git
                .changed_files(&baseline.commit, candidate)
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let reports: Vec<String> = [
            loaded.baseline_report_path.clone(),
            loaded.candidate_report_path.clone(),
        ]
        .into_iter()
        .flatten()
        .collect();
        let report_context = ReportContext {
            experiment: &loaded.experiment,
            baseline: loaded.baseline.as_ref(),
            candidate: &loaded.candidate,
            changed_files: &changed_files,
            reports: &reports,
        };
        documents.push(report::document(&report_context, loaded.comparison.as_ref()));
        comments.push(report::comment(&report_context, loaded.comparison.as_ref()));
    }

    let text = if args.format == crate::ReportFormat::Comment {
        comments.join("\n\n")
    } else {
        documents.join("\n\n")
    };

    match &args.out {
        Some(path) => {
            report::write(path, &text)?;
            println!("{}", output::kv("written", path.display()));
            println!(
                "{}",
                output::kv("experiments", names.join(", "))
            );
        }
        None => print!("{text}"),
    }
    Ok(())
}

/// Reads the newest baseline/experiment reports of one experiment plus its stored comparison.
fn load(context: &Context, name: &str) -> Result<Loaded> {
    let experiment = context
        .store
        .experiment_by_name(name)?
        .with_context(|| format!("no experiment named `{name}` in this repository"))?;

    let baseline_row = context
        .store
        .latest_run(experiment.id, RunKind::Baseline)?;
    let candidate_row = context
        .store
        .latest_run(experiment.id, RunKind::Experiment)?
        .with_context(|| {
            format!("experiment `{name}` has no experiment run yet — run `ekbasis experiment run {name}`")
        })?;

    let baseline = baseline_row
        .as_ref()
        .map(|row| read_report(context, row.id, row.report_path.as_deref()))
        .transpose()?;
    let candidate = read_report(
        context,
        candidate_row.id,
        candidate_row.report_path.as_deref(),
    )?;

    // The stored comparison is used when present; otherwise it is recomputed from the reports so a
    // deleted `<name>.comparison.json` never blocks a report.
    let comparison_path = context.paths.comparison_path(name);
    let comparison = if comparison_path.exists() {
        let text = std::fs::read_to_string(&comparison_path)
            .with_context(|| format!("cannot read `{}`", comparison_path.display()))?;
        Some(
            serde_json::from_str::<Comparison>(&text)
                .with_context(|| format!("cannot parse `{}`", comparison_path.display()))?,
        )
    } else {
        baseline.as_ref().map(|baseline_report| {
            compare_reports(
                baseline_report,
                &candidate,
                resolve_threshold(&context.config, None, None),
            )
        })
    };

    Ok(Loaded {
        experiment: experiment.name,
        baseline,
        candidate,
        baseline_report_path: baseline_row.and_then(|row| row.report_path),
        candidate_report_path: candidate_row.report_path,
        comparison,
    })
}

/// Loads a stored JSON report of one run.
fn read_report(context: &Context, run_id: i64, report_path: Option<&str>) -> Result<RunReport> {
    let path = report_path
        .map(|relative| context.paths.repo_root().join(relative))
        .with_context(|| format!("run #{run_id} has no stored report file"))?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read `{}`", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("cannot parse `{}`", path.display()))
}

