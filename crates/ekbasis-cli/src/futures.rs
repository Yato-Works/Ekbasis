//! Loading everything that describes one *future* (plan §19): the latest comparison, its
//! four-dimension evaluation and the delta history of the experiment.
//!
//! Shared by `aion timeline` and `aion dashboard` so both always tell the same story. Missing
//! artifacts (deleted reports, never-measured experiments) degrade to `None` instead of failing:
//! a timeline with a hole in it is still a timeline.

use ekbasis_benchmark::{Comparison, Evaluation, compare_reports, evaluate};
use ekbasis_storage::{ExperimentSummary, HistoryPoint};
use anyhow::Result;

use crate::commands::{Context, resolve_threshold};

/// One future with everything the UI can say about it.
pub struct Future {
    pub summary: ExperimentSummary,
    /// Comparison of the newest baseline vs the newest experiment run (when both reports exist).
    pub comparison: Option<Comparison>,
    /// Four-dimension evaluation of `comparison`.
    pub evaluation: Option<Evaluation>,
    /// Delta of every experiment run against the baseline recorded at its time, oldest first.
    pub history: Vec<HistoryPoint>,
    /// Regression threshold of the experiment's spec, when the spec is still around.
    pub spec_threshold: Option<f64>,
}

/// Fallback threshold when no spec exists (matches `DefaultsConfig::default`).
const DEFAULT_REGRESSION_THRESHOLD_PERCENT: f64 = 3.0;

impl Future {
    pub fn name(&self) -> &str {
        &self.summary.experiment.name
    }

    /// Branch label the future was measured on (`aion/<name>` for local runs, none in CI mode).
    pub fn branch(&self) -> Option<&String> {
        self.summary
            .latest_experiment
            .as_ref()
            .and_then(|run| run.branch.as_ref())
    }

    /// Latest measured delta against the latest baseline, when both sides exist.
    pub fn delta_percent(&self) -> Option<f64> {
        self.summary.delta_percent
    }

    /// Regression threshold the history trends are classified against: the spec's value when
    /// the spec still exists, otherwise the repository default.
    pub fn threshold(&self) -> f64 {
        self.spec_threshold
            .unwrap_or(DEFAULT_REGRESSION_THRESHOLD_PERCENT)
    }

    /// Evaluation letters (`A B C A`), or `- - - -` when nothing could be evaluated.
    pub fn letters(&self) -> String {
        self.evaluation
            .as_ref()
            .map(|evaluation| evaluation.letters())
            .unwrap_or_else(|| "- - - -".to_string())
    }

    /// Trend of the deltas across runs (for the sparkline).
    pub fn deltas(&self) -> Vec<f64> {
        self.history
            .iter()
            .filter_map(|point| point.delta_percent)
            .collect()
    }
}

/// Loads every experiment as a future, newest experiment first, capped at `limit`.
pub fn load_futures(context: &Context, limit: usize) -> Result<Vec<Future>> {
    let summaries = context.store.experiment_summaries()?;
    let mut futures = Vec::new();
    for summary in summaries.into_iter().take(limit) {
        futures.push(load_future(context, summary)?);
    }
    Ok(futures)
}

/// Loads one experiment as a future.
pub fn load_future(context: &Context, summary: ExperimentSummary) -> Result<Future> {
    let name = summary.experiment.name.clone();
    let history = context.store.delta_history(summary.experiment.id)?;

    // Threshold: the spec's value when the spec is still around, else the config default —
    // the same resolution `aion compare` applies.
    let spec_threshold = context
        .paths
        .spec_path(&name)
        .exists()
        .then(|| {
            ekbasis_core::ExperimentSpec::load(&context.paths.spec_path(&name))
                .ok()
                .map(|spec| spec.benchmark.regression_threshold_percent)
        })
        .flatten();
    let threshold = resolve_threshold(&context.config, None, spec_threshold);

    let comparison = match (&summary.latest_baseline, &summary.latest_experiment) {
        (Some(baseline), Some(experiment)) => {
            match (
                read_report(context, baseline),
                read_report(context, experiment),
            ) {
                (Ok(baseline_report), Ok(experiment_report)) => Some(compare_reports(
                    &baseline_report,
                    &experiment_report,
                    threshold,
                )),
                // A missing report file must not kill the timeline; it only loses the details.
                _ => None,
            }
        }
        _ => None,
    };
    let evaluation = comparison.as_ref().map(evaluate);

    Ok(Future {
        summary,
        comparison,
        evaluation,
        history,
        spec_threshold,
    })
}

/// Reads the JSON report a run row points at.
fn read_report(context: &Context, run: &ekbasis_storage::RunRow) -> Result<ekbasis_core::model::RunReport> {
    let relative = run
        .report_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("run #{} has no report path", run.id))?;
    let path = context.paths.repo_root().join(relative);
    let text = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&text)?)
}
