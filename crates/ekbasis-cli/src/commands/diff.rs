//! `aion experiment diff` — what changed between two experiments (or two measured runs).
//!
//! Two questions, answered side by side:
//!
//! * *what did the definitions change?* — spec level: changes, commands, environment, benchmark and
//!   limits (plan §17: "Experiment diff"), and
//! * *what did the measurements change?* — run level: the same comparison AION prints after a run,
//!   plus the files that changed between the two measured commits.

use std::path::PathBuf;

use ekbasis_core::model::{RunKind, RunReport};
use ekbasis_core::{ExperimentSpec, fmt};
use anyhow::{Context as _, Result};

use crate::commands::{Context, resolve_threshold};
use crate::output;
use crate::DiffArgs;

/// One side of a diff: an experiment (spec + newest run) or a stored run/report.
struct Side {
    label: String,
    spec: Option<ExperimentSpec>,
    report: Option<RunReport>,
}

impl Side {
    /// Resolves a token: run id, report file, `<experiment>:<kind>` or experiment name.
    fn load(context: &Context, token: &str) -> Result<Side> {
        if let Ok(id) = token.parse::<i64>() {
            let row = context
                .store
                .run_by_id(id)?
                .with_context(|| format!("there is no run #{id} in this repository"))?;
            return Ok(Side {
                label: format!("#{} {}", row.id, row.label),
                spec: None,
                report: Some(read_report(context, row.id, row.report_path.as_deref())?),
            });
        }

        let path = PathBuf::from(token);
        if path.is_file() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("cannot read `{}`", path.display()))?;
            let report = serde_json::from_str(&text)
                .with_context(|| format!("cannot parse `{}`", path.display()))?;
            return Ok(Side {
                label: path.display().to_string(),
                spec: None,
                report: Some(report),
            });
        }

        let (name, kind) = match token.split_once(':') {
            Some((name, kind_text)) => (
                name,
                Some(RunKind::parse(kind_text).with_context(|| {
                    format!("`{kind_text}` is not a run kind (use `baseline` or `experiment`)")
                })?),
            ),
            None => (token, None),
        };

        let experiment = context
            .store
            .experiment_by_name(name)?
            .with_context(|| format!("no experiment named `{name}` in this repository"))?;
        let runs = context.store.runs_for_experiment(experiment.id)?;
        let chosen = match kind {
            Some(kind) => runs.iter().find(|run| run.kind == kind),
            None => runs
                .iter()
                .find(|run| run.kind == RunKind::Experiment)
                .or_else(|| runs.first()),
        };
        let report = chosen
            .map(|row| read_report(context, row.id, row.report_path.as_deref()))
            .transpose()?;

        let spec_path = context.paths.spec_path(name);
        let spec = if spec_path.exists() {
            Some(ExperimentSpec::load(&spec_path)?)
        } else {
            None
        };

        Ok(Side {
            label: match kind {
                Some(kind) => format!("{name}:{kind}"),
                None => name.to_string(),
            },
            spec,
            report,
        })
    }

    /// Short description for the header.
    fn description(&self) -> String {
        match &self.report {
            Some(report) => format!(
                "{} ({} {}, {} samples)",
                self.label,
                fmt::short_sha(&report.commit),
                report.kind,
                report.stats.map(|stats| stats.count).unwrap_or(0)
            ),
            None => format!("{} (no measured run yet)", self.label),
        }
    }
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

/// `aion experiment diff <left> <right>`
pub fn run(args: &DiffArgs, verbose: bool) -> Result<()> {
    let context = Context::load(verbose)?;
    let left = Side::load(&context, &args.left)?;
    let right = Side::load(&context, &args.right)?;

    println!(
        "{}",
        output::banner(&format!("experiment diff {} -> {}", args.left, args.right))
    );
    println!("{}", output::kv("left", left.description()));
    println!("{}", output::kv("right", right.description()));

    println!("\nspecification");
    match (&left.spec, &right.spec) {
        (Some(left_spec), Some(right_spec)) => {
            let differences = spec_differences(left_spec, right_spec);
            if differences.is_empty() {
                println!("  identical");
            }
            for line in differences {
                println!("  {line}");
            }
        }
        _ => println!("  (not comparable: a run id or report file carries no spec)"),
    }

    println!("\nmeasured");
    match (&left.report, &right.report) {
        (Some(left_report), Some(right_report)) => {
            let comparison = ekbasis_benchmark::compare_reports(
                left_report,
                right_report,
                resolve_threshold(&context.config, None, None),
            );
            print!("{}", output::indent(&comparison.render(), 2));

            let files = context
                .git
                .changed_files(&left_report.commit, &right_report.commit)
                .unwrap_or_default();
            if files.is_empty() {
                println!("\nchanged files\n  (none between the two commits)");
            } else {
                println!(
                    "\nchanged files ({} -> {}):",
                    fmt::short_sha(&left_report.commit),
                    fmt::short_sha(&right_report.commit)
                );
                for file in files.iter().take(20) {
                    println!("  {file}");
                }
                if files.len() > 20 {
                    println!("  ... and {} more", files.len() - 20);
                }
            }
        }
        _ => println!("  (one side has no stored run yet, so nothing can be compared)"),
    }
    Ok(())
}

/// Human readable differences between two experiment definitions.
fn spec_differences(left: &ExperimentSpec, right: &ExperimentSpec) -> Vec<String> {
    let mut lines = Vec::new();

    if left.experiment.name != right.experiment.name {
        lines.push(format!(
            "name: {} -> {}",
            left.experiment.name, right.experiment.name
        ));
    }
    if left.experiment.base != right.experiment.base {
        let base = |spec: &ExperimentSpec| {
            spec.experiment
                .base
                .clone()
                .unwrap_or_else(|| "(current branch)".to_string())
        };
        lines.push(format!("base: {} -> {}", base(left), base(right)));
    }

    let changes = |spec: &ExperimentSpec| -> Vec<String> {
        spec.changes.iter().map(|change| change.describe()).collect()
    };
    let (left_changes, right_changes) = (changes(left), changes(right));
    for change in left_changes
        .iter()
        .filter(|change| !right_changes.contains(change))
    {
        lines.push(format!("- change removed: {change}"));
    }
    for change in right_changes
        .iter()
        .filter(|change| !left_changes.contains(change))
    {
        lines.push(format!("+ change added:   {change}"));
    }

    let left_phases = left.command.phases();
    let right_phases = right.command.phases();
    for (phase, left_command) in &left_phases {
        match right_phases.iter().find(|(other, _)| other == phase) {
            Some((_, right_command)) if right_command == left_command => {}
            Some((_, right_command)) => {
                lines.push(format!("{phase} command: `{left_command}` -> `{right_command}`"))
            }
            None => lines.push(format!("{phase} command: `{left_command}` -> (removed)")),
        }
    }
    for (phase, right_command) in &right_phases {
        if !left_phases.iter().any(|(other, _)| other == phase) {
            lines.push(format!("{phase} command: (none) -> `{right_command}`"));
        }
    }

    for (name, value) in &left.environment {
        match right.environment.get(name) {
            Some(other) if other == value => {}
            Some(other) => lines.push(format!("environment {name}: {value} -> {other}")),
            None => lines.push(format!("environment {name}: {value} -> (unset)")),
        }
    }
    for (name, value) in &right.environment {
        if !left.environment.contains_key(name) {
            lines.push(format!("environment {name}: (unset) -> {value}"));
        }
    }

    if left.benchmark.iterations != right.benchmark.iterations {
        lines.push(format!(
            "benchmark.iterations: {} -> {}",
            left.benchmark.iterations, right.benchmark.iterations
        ));
    }
    if left.benchmark.warmup != right.benchmark.warmup {
        lines.push(format!(
            "benchmark.warmup: {} -> {}",
            left.benchmark.warmup, right.benchmark.warmup
        ));
    }
    if (left.benchmark.regression_threshold_percent - right.benchmark.regression_threshold_percent)
        .abs()
        > f64::EPSILON
    {
        lines.push(format!(
            "benchmark.regression_threshold_percent: {} -> {}",
            left.benchmark.regression_threshold_percent, right.benchmark.regression_threshold_percent
        ));
    }
    if left.limits.timeout_secs != right.limits.timeout_secs {
        lines.push(format!(
            "limits.timeout_secs: {} -> {}",
            left.limits.timeout_secs, right.limits.timeout_secs
        ));
    }
    if left.limits.clean_env != right.limits.clean_env {
        lines.push(format!(
            "limits.clean_env: {} -> {}",
            left.limits.clean_env, right.limits.clean_env
        ));
    }
    if left.limits.max_output_kb != right.limits.max_output_kb {
        lines.push(format!(
            "limits.max_output_kb: {} -> {}",
            left.limits.max_output_kb, right.limits.max_output_kb
        ));
    }

    lines
}
