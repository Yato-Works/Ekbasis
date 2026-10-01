//! `aion timeline` — the software timeline AION has explored in this repository.
//!
//! Two views of the same data (plan §13, §19):
//!
//! * the **futures graph**: every experiment branching off its base commit, with the measured
//!   delta, the four evaluation letters and the trend sparkline, and
//! * the **run list**: every recorded run with its commit, variance and outcome.

use ekbasis_core::fmt;
use ekbasis_core::model::RunKind;
use ekbasis_storage::HistoryPoint;
use anyhow::Result;

use crate::commands::Context;
use crate::futures::{self, Future};
use crate::output;
use crate::TimelineArgs;

/// `aion timeline [--all] [--limit N]`
pub fn run(args: &TimelineArgs, verbose: bool) -> Result<()> {
    let context = Context::load(verbose)?;
    println!("{}", output::banner("timeline"));

    let futures = futures::load_futures(&context, args.limit)?;
    if futures.is_empty() {
        println!("nothing explored yet - `ekbasis experiment create <name>` starts a timeline");
        return Ok(());
    }

    let head = context.git.head_branch().ok().flatten();
    println!(
        "repository  {}  ({} experiments, {} runs)",
        context.paths.repo_root().display(),
        context.store.experiment_count()?,
        context.store.run_count()?
    );
    if let Some(branch) = &head {
        println!("current     {branch}");
    }

    render_graph(&futures);

    for future in &futures {
        render_runs(future, args.all)?;
    }

    println!("\nlegend");
    println!("  baseline  : unmodified checkout of the base commit");
    println!("  experiment: branch with the experiment changes applied");
    println!("  delta     : measured change against the newest baseline run of the same experiment");
    println!("  letters   : performance memory correctness stability (A best ... F worst, - unknown)");
    println!("  trend     : delta across runs, oldest first (lower is faster)");
    Ok(())
}

/// Renders the branching-futures graph (plan §19).
///
/// Futures are grouped by the commit they branch from, because a timeline only makes sense
/// relative to its base: `main 8f4c21a ──> future A / future B`.
fn render_graph(futures: &[Future]) {
    // Group by `base_ref base_commit`, preserving first-seen order.
    let mut groups: Vec<(String, Vec<&Future>)> = Vec::new();
    for future in futures {
        let base = future
            .summary
            .experiment
            .base_ref
            .clone()
            .unwrap_or_else(|| "(current branch)".to_string());
        let base_commit = future
            .summary
            .experiment
            .base_commit
            .as_deref()
            .map(fmt::short_sha)
            .unwrap_or_else(|| "???".to_string());
        let key = format!("{base} {base_commit}");
        match groups.iter_mut().find(|(candidate, _)| *candidate == key) {
            Some((_, members)) => members.push(future),
            None => groups.push((key, vec![future])),
        }
    }

    println!("\nfutures");
    for (base, members) in &groups {
        println!("  {base}");
        for (index, future) in members.iter().enumerate() {
            let last = index + 1 == members.len();
            let marker = if last { "└─" } else { "├─" };
            let delta = future
                .delta_percent()
                .map(fmt::delta_percent)
                .unwrap_or_else(|| "     -".to_string());
            let trend = trend_line(&future.history);
            let mean = future
                .summary
                .latest_experiment
                .as_ref()
                .and_then(|run| run.mean_ms)
                .map(fmt::duration_ms)
                .unwrap_or_else(|| "not measured".to_string());
            let state = future
                .summary
                .latest_experiment
                .as_ref()
                .map(|run| if run.success { "" } else { "  [failed]" })
                .unwrap_or("  [never run]");
            println!(
                "    {marker} {:<24} {delta:>8}  {}  {trend}  {mean}{state}",
                future.name(),
                future.letters(),
            );
        }
    }
}

/// Sparkline of the delta history, oldest first.
fn trend_line(history: &[HistoryPoint]) -> String {
    let deltas: Vec<f64> = history
        .iter()
        .filter_map(|point| point.delta_percent)
        .collect();
    if deltas.len() < 2 {
        return "     -".to_string();
    }
    ekbasis_benchmark::statistics::sparkline(&deltas)
}

/// The per-run listing under one future.
fn render_runs(future: &Future, all: bool) -> Result<()> {
    let summary = &future.summary;
    let experiment = &summary.experiment;
    let base = experiment
        .base_ref
        .clone()
        .unwrap_or_else(|| "(current branch)".to_string());
    let base_commit = experiment
        .base_commit
        .as_ref()
        .map(|sha| fmt::short_sha(sha))
        .unwrap_or_else(|| "???".to_string());

    println!("\n● {}  (from {} {})", experiment.name, base, base_commit);
    if let Some(description) = &experiment.description {
        println!("  {}", fmt::truncate_line(description, 90));
    }
    if let Some(evaluation) = &future.evaluation {
        print!("{}", output::indent(&evaluation.render(), 2));
    }

    let runs: Vec<&ekbasis_storage::RunRow> = if all {
        summary.runs.iter().collect()
    } else {
        summary.runs.iter().take(4).collect()
    };
    let total = summary.runs.len();
    for (index, run) in runs.iter().enumerate() {
        let last = index + 1 == runs.len();
        let marker = if last { "└─" } else { "├─" };
        let branch_label = run
            .branch
            .clone()
            .unwrap_or_else(|| format!("detached at {}", fmt::short_sha(&run.base_commit)));
        let state = if run.success { "ok" } else { "failed" };
        println!(
            "  {marker} #{} {:<10} {:<12} {:<8} [{}]",
            run.id,
            run.kind.to_string(),
            fmt::short_sha(&run.commit_sha),
            branch_label,
            state
        );
        let delta = if run.kind == RunKind::Experiment {
            summary
                .delta_percent
                .map(fmt::delta_percent)
                .unwrap_or_else(|| "-".to_string())
        } else {
            "-".to_string()
        };
        println!(
            "       {}  {}  {}",
            run.summary(),
            run.cv_percent
                .map(|cv| format!("sigma {}", fmt::percent(cv)))
                .unwrap_or_else(|| "sigma -".to_string()),
            delta
        );
        println!("       {}  {}", run.started_at, run.label);
    }
    if !all && total > runs.len() {
        println!("  ({} more runs — use --all)", total - runs.len());
    }
    Ok(())
}
