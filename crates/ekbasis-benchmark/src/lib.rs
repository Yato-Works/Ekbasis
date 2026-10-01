//! Benchmark statistics and comparison (plan §5).
//!
//! AION compares *measurements*: durations, resource usage and correctness outcomes that were
//! actually observed. The vocabulary of this crate is deliberately factual — "regressed" means
//! "the measured number grew beyond the configured threshold", nothing more.

use ekbasis_core::fmt;
use ekbasis_core::model::{RunReport, Stats};
use serde::{Deserialize, Serialize};

pub mod evaluation;
pub mod markdown;
pub mod statistics;

pub use evaluation::{Dimension, Evaluation, Grade, evaluate};
pub use markdown::{comparison_markdown, graph_block, headline};
pub use statistics::{ALPHA, Distribution, Outlier, Significance, WELCH_TEST, welch};

/// Which direction counts as better for a metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    LowerIsBetter,
    HigherIsBetter,
}

/// Unit of a metric, used for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Milliseconds,
    Bytes,
    Percent,
    Count,
    /// Degrees Celsius (temperatures observed while the command ran).
    Celsius,
}

impl Unit {
    /// Formats a raw value with this unit.
    pub fn render(self, value: f64) -> String {
        match self {
            Unit::Milliseconds => fmt::duration_ms(value),
            Unit::Bytes => fmt::bytes(value.max(0.0) as u64),
            Unit::Percent => fmt::percent(value),
            Unit::Count => format!("{}", value.round() as i64),
            Unit::Celsius => fmt::temperature_c(value as f32),
        }
    }
}

/// Outcome of comparing one metric.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Improved,
    Regressed,
    Neutral,
    /// One or both sides have no value for this metric.
    Unavailable,
}

impl Verdict {
    /// ASCII label used in tables (keeps column alignment predictable).
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Improved => "improved",
            Verdict::Regressed => "regressed",
            Verdict::Neutral => "neutral",
            Verdict::Unavailable => "n/a",
        }
    }

    /// Short glyph for line-oriented output.
    pub fn symbol(self) -> &'static str {
        match self {
            Verdict::Improved => "[ok]",
            Verdict::Regressed => "[!!]",
            Verdict::Neutral => "[--]",
            Verdict::Unavailable => "[  ]",
        }
    }
}

/// One compared metric.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricDelta {
    pub name: String,
    pub unit: Unit,
    pub baseline: Option<f64>,
    pub candidate: Option<f64>,
    pub delta: Option<f64>,
    pub delta_percent: Option<f64>,
    pub direction: Direction,
    pub verdict: Verdict,
    /// Explanation shown when the metric is unavailable or needs context.
    pub note: Option<String>,
    /// Context metrics (memory, CPU, GPU, temperature) are reported but do not drive the verdict:
    /// they are *peaks* of a resource the benchmark did not intend to measure.
    #[serde(default)]
    pub secondary: bool,
}

impl MetricDelta {
    /// Builds a metric from two optional values, applying the regression threshold.
    pub fn new(
        name: impl Into<String>,
        unit: Unit,
        direction: Direction,
        baseline: Option<f64>,
        candidate: Option<f64>,
        threshold_percent: f64,
    ) -> MetricDelta {
        let name = name.into();
        match (baseline, candidate) {
            (Some(baseline), Some(candidate)) => {
                let delta = candidate - baseline;
                let delta_percent = if baseline.abs() > f64::EPSILON {
                    delta / baseline * 100.0
                } else {
                    0.0
                };
                let verdict = if delta_percent.abs() < threshold_percent {
                    Verdict::Neutral
                } else {
                    let better = match direction {
                        Direction::LowerIsBetter => delta < 0.0,
                        Direction::HigherIsBetter => delta > 0.0,
                    };
                    if better {
                        Verdict::Improved
                    } else {
                        Verdict::Regressed
                    }
                };
                MetricDelta {
                    name,
                    unit,
                    baseline: Some(baseline),
                    candidate: Some(candidate),
                    delta: Some(delta),
                    delta_percent: Some(delta_percent),
                    direction,
                    verdict,
                    note: None,
                    secondary: false,
                }
            }
            (baseline, candidate) => {
                let missing = match (baseline.is_none(), candidate.is_none()) {
                    (true, true) => "not measured on either side",
                    (true, false) => "not measured for the baseline",
                    _ => "not measured for the experiment",
                };
                MetricDelta {
                    name,
                    unit,
                    baseline,
                    candidate,
                    delta: None,
                    delta_percent: None,
                    direction,
                    verdict: Verdict::Unavailable,
                    note: Some(missing.to_string()),
                    secondary: false,
                }
            }
        }
    }

    /// Adds an explanatory note.
    pub fn with_note(mut self, note: impl Into<String>) -> MetricDelta {
        self.note = Some(note.into());
        self
    }

    /// Marks the metric as context (shown, but not counted in the verdict).
    pub fn with_secondary(mut self) -> MetricDelta {
        self.secondary = true;
        self
    }

    /// Rendered value of the baseline side.
    pub fn baseline_text(&self) -> String {
        self.baseline
            .map(|value| self.unit.render(value))
            .unwrap_or_else(|| "-".to_string())
    }

    /// Rendered value of the experiment side.
    pub fn candidate_text(&self) -> String {
        self.candidate
            .map(|value| self.unit.render(value))
            .unwrap_or_else(|| "-".to_string())
    }

    /// Rendered difference, e.g. `-18.7% (533.0 ms)`.
    pub fn delta_text(&self) -> String {
        match (self.delta, self.delta_percent) {
            (Some(delta), Some(percent)) => format!(
                "{} ({})",
                fmt::delta_percent(percent),
                match self.unit {
                    Unit::Milliseconds => fmt::duration_ms(delta.abs()),
                    Unit::Bytes => fmt::bytes(delta.abs() as u64),
                    Unit::Percent => fmt::percent(delta.abs()),
                    Unit::Count => format!("{}", delta.abs().round() as i64),
                    Unit::Celsius => fmt::temperature_c(delta.abs() as f32),
                }
            ),
            _ => "-".to_string(),
        }
    }
}

/// Coefficient of variation (in percent) above which AION calls a distribution noisy.
pub const NOISE_LIMIT_PERCENT: f64 = 5.0;

/// State of one correctness check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckState {
    Pass,
    Fail,
    Warn,
    Info,
}

impl CheckState {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckState::Pass => "ok",
            CheckState::Fail => "FAILED",
            CheckState::Warn => "warn",
            CheckState::Info => "info",
        }
    }
}

/// One correctness or reliability line of a comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckLine {
    pub name: String,
    pub state: CheckState,
    pub detail: String,
}

/// Everything AION can say about “what if this change had been made” (plan §5, §17).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparison {
    pub experiment: String,
    pub baseline_label: String,
    pub candidate_label: String,
    pub baseline_commit: String,
    pub candidate_commit: String,
    pub threshold_percent: f64,
    pub metrics: Vec<MetricDelta>,
    pub checks: Vec<CheckLine>,
    pub warnings: Vec<String>,
    /// Result of Welch's t-test on the primary measurement (startup mean).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub significance: Option<Significance>,
    /// Descriptive statistics of the baseline samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_distribution: Option<Distribution>,
    /// Descriptive statistics of the experiment samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_distribution: Option<Distribution>,
    /// Baseline durations in measurement order, for the graph in reports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub baseline_samples: Vec<f64>,
    /// Experiment durations in measurement order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_samples: Vec<f64>,
}

impl Comparison {
    /// Number of metrics per verdict, used for the conclusion line.
    ///
    /// Context metrics (memory, CPU, GPU, temperature) are excluded: they are peaks of resources
    /// the benchmark did not set out to measure.
    pub fn tally(&self) -> (usize, usize, usize) {
        let mut improved = 0;
        let mut regressed = 0;
        let mut neutral = 0;
        for metric in self.metrics.iter().filter(|metric| !metric.secondary) {
            match metric.verdict {
                Verdict::Improved => improved += 1,
                Verdict::Regressed => regressed += 1,
                Verdict::Neutral => neutral += 1,
                Verdict::Unavailable => {}
            }
        }
        (improved, regressed, neutral)
    }

    /// Names of the context metrics, for the footnote under the metric table.
    pub fn context_metric_names(&self) -> Vec<String> {
        self.metrics
            .iter()
            .filter(|metric| metric.secondary)
            .map(|metric| metric.name.clone())
            .collect()
    }

    /// The metric that carries the verdict: the mean of the measured command's duration.
    pub fn primary_metric(&self) -> Option<&MetricDelta> {
        self.metrics.iter().find(|metric| metric.name == "startup (mean)")
    }

    /// True when any correctness check failed.
    pub fn has_failures(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.state == CheckState::Fail)
    }
}

/// Warns when a distribution is too noisy for small deltas to mean anything.
pub fn reliability_warnings(label: &str, stats: &Stats, noise_limit_percent: f64) -> Vec<String> {
    let mut warnings = Vec::new();
    if stats.is_noisy(noise_limit_percent) {
        warnings.push(format!(
            "{label} variance is high (sigma = {} of the mean, limit {}): raise benchmark.iterations or quiet the machine",
            fmt::percent(stats.cv_percent),
            fmt::percent(noise_limit_percent)
        ));
    }
    if stats.count < 3 {
        warnings.push(format!(
            "{label} has only {} measured iteration(s): treat the deltas as indicative, not conclusive",
            stats.count
        ));
    }
    warnings
}

/// A metric derived from a report's statistics.
type StatMetric = (&'static str, fn(Stats) -> f64);

/// Compares a baseline run with an experiment run (plan §5).
///
/// The function never invents values: every metric either has two real measurements or is
/// reported as unavailable.
pub fn compare_reports(
    baseline: &RunReport,
    candidate: &RunReport,
    threshold_percent: f64,
) -> Comparison {
    let mut metrics: Vec<MetricDelta> = Vec::new();
    let mut checks: Vec<CheckLine> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    let baseline_stats = baseline.stats;
    let candidate_stats = candidate.stats;
    let baseline_samples = baseline.measured_samples();
    let candidate_samples = candidate.measured_samples();
    let baseline_distribution = Distribution::from_samples(&baseline_samples);
    let candidate_distribution = Distribution::from_samples(&candidate_samples);
    let significance = welch(
        &baseline.successful_durations(),
        &candidate.successful_durations(),
    );

    let stat_metrics: [StatMetric; 4] = [
        ("startup (mean)", |stats: Stats| stats.mean),
        ("startup (median)", |stats: Stats| stats.median),
        ("startup (best)", |stats: Stats| stats.min),
        ("startup (p95)", |stats: Stats| stats.p95),
    ];
    for (name, pick) in stat_metrics {
        metrics.push(MetricDelta::new(
            name,
            Unit::Milliseconds,
            Direction::LowerIsBetter,
            baseline_stats.map(pick),
            candidate_stats.map(pick),
            threshold_percent,
        ));
    }

    metrics.push(
        MetricDelta::new(
            "startup (trimmed mean)",
            Unit::Milliseconds,
            Direction::LowerIsBetter,
            baseline_distribution
                .as_ref()
                .map(|distribution| distribution.trimmed_mean),
            candidate_distribution
                .as_ref()
                .map(|distribution| distribution.trimmed_mean),
            threshold_percent,
        )
        .with_note("20% trimmed mean: the single fastest and the single slowest run are ignored"),
    );

    metrics.push(
        MetricDelta::new(
            "build",
            Unit::Milliseconds,
            Direction::LowerIsBetter,
            baseline.build_ms,
            candidate.build_ms,
            threshold_percent,
        )
        .with_note("compile time of this state"),
    );

    metrics.push(
        MetricDelta::new(
            "peak memory",
            Unit::Bytes,
            Direction::LowerIsBetter,
            baseline.peak_memory_bytes().map(|value| value as f64),
            candidate.peak_memory_bytes().map(|value| value as f64),
            threshold_percent,
        )
        .with_note("highest resident memory seen in any iteration: a single spike flips this number"),
    );

    metrics.push(
        MetricDelta::new(
            "avg cpu",
            Unit::Percent,
            Direction::LowerIsBetter,
            baseline.avg_cpu_percent(),
            candidate.avg_cpu_percent(),
            threshold_percent,
        )
        .with_secondary()
        .with_note("CPU time of the process tree as a share of the measured wall time"),
    );

    metrics.push(
        MetricDelta::new(
            "gpu util",
            Unit::Percent,
            Direction::LowerIsBetter,
            baseline.gpu_utilization_percent(),
            candidate.gpu_utilization_percent(),
            threshold_percent,
        )
        .with_secondary()
        .with_note("highest GPU utilisation seen while the command ran (needs nvidia-smi or rocm-smi)"),
    );

    metrics.push(
        MetricDelta::new(
            "vram",
            Unit::Bytes,
            Direction::LowerIsBetter,
            baseline.gpu_memory_bytes().map(|value| value as f64),
            candidate.gpu_memory_bytes().map(|value| value as f64),
            threshold_percent,
        )
        .with_secondary()
        .with_note("highest GPU memory usage seen while the command ran"),
    );

    metrics.push(
        MetricDelta::new(
            "max temp",
            Unit::Celsius,
            Direction::LowerIsBetter,
            baseline.max_temperature_c(),
            candidate.max_temperature_c(),
            threshold_percent,
        )
        .with_secondary()
        .with_note("hottest temperature sensor seen while the command ran"),
    );

    // ---- correctness -------------------------------------------------------
    let build_ok = |report: &RunReport| {
        report
            .build_phase()
            .map(|phase| phase.success)
            .unwrap_or(true)
    };
    let baseline_build = build_ok(baseline);
    let candidate_build = build_ok(candidate);
    checks.push(CheckLine {
        name: "build".to_string(),
        state: if baseline_build && candidate_build {
            CheckState::Pass
        } else {
            CheckState::Fail
        },
        detail: match (baseline_build, candidate_build) {
            (true, true) => "both states compiled".to_string(),
            (true, false) => "the experiment failed to build".to_string(),
            (false, true) => "the baseline failed to build".to_string(),
            (false, false) => "neither state built successfully".to_string(),
        },
    });

    match (
        baseline.test.as_ref().map(|test| test.success),
        candidate.test.as_ref().map(|test| test.success),
    ) {
        (None, None) => checks.push(CheckLine {
            name: "tests".to_string(),
            state: CheckState::Info,
            detail: "no test command configured".to_string(),
        }),
        (baseline_ok, candidate_ok) => {
            let detail = match (baseline_ok, candidate_ok) {
                (Some(true), Some(true)) => {
                    "baseline and experiment test suites passed".to_string()
                }
                (_, Some(false)) => "the experiment test suite FAILED".to_string(),
                (Some(false), _) => "the baseline test suite failed".to_string(),
                (None, Some(true)) => "experiment tests passed (baseline not tested)".to_string(),
                (Some(true), None) => "baseline tests passed (experiment not tested)".to_string(),
                (None, None) => "no test command configured".to_string(),
            };
            let state = if candidate_ok == Some(false) || baseline_ok == Some(false) {
                CheckState::Fail
            } else {
                CheckState::Pass
            };
            checks.push(CheckLine {
                name: "tests".to_string(),
                state,
                detail,
            });
        }
    }

    let candidate_failures = candidate.failed_iterations();
    if !candidate_failures.is_empty() {
        checks.push(CheckLine {
            name: "iterations".to_string(),
            state: CheckState::Fail,
            detail: format!(
                "{} of {} measured iterations failed: {}",
                candidate_failures.len(),
                candidate.iterations,
                candidate_failures
                    .iter()
                    .map(|(iteration, reason)| format!("#{iteration} {reason}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
    } else if candidate_stats.is_some() {
        checks.push(CheckLine {
            name: "iterations".to_string(),
            state: CheckState::Pass,
            detail: format!("all {} measured iterations completed", candidate.iterations),
        });
    }

    // ---- statistical significance (plan §17) -------------------------------
    let primary_percent = metrics
        .iter()
        .find(|metric| metric.name == "startup (mean)")
        .and_then(|metric| metric.delta_percent);
    let difference_is_visible = primary_percent
        .map(|delta| delta.abs() >= threshold_percent)
        .unwrap_or(false);

    match &significance {
        Some(result) if result.significant() => checks.push(CheckLine {
            name: "significance".to_string(),
            state: CheckState::Pass,
            detail: result.describe(),
        }),
        Some(result) => {
            checks.push(CheckLine {
                name: "significance".to_string(),
                state: if difference_is_visible {
                    CheckState::Warn
                } else {
                    CheckState::Info
                },
                detail: result.describe(),
            });
            if difference_is_visible {
                warnings.push(format!(
                    "the measured difference ({}) is not statistically significant (p = {}, n = {}/{}): increase benchmark.iterations before acting on it",
                    primary_percent
                        .map(fmt::delta_percent)
                        .unwrap_or_else(|| "-".to_string()),
                    fmt::probability(result.p_value),
                    result.baseline_n,
                    result.candidate_n,
                ));
            }
        }
        None => checks.push(CheckLine {
            name: "significance".to_string(),
            state: CheckState::Info,
            detail: "at least two successful iterations per side are needed for a t-test"
                .to_string(),
        }),
    }

    for (label, distribution) in [
        ("baseline", baseline_distribution.as_ref()),
        ("experiment", candidate_distribution.as_ref()),
    ] {
        let Some(distribution) = distribution else {
            continue;
        };
        if distribution.outliers.is_empty() {
            continue;
        }
        warnings.push(format!(
            "{label} contains {} outlier(s) outside Tukey's fences ({}): the trimmed mean is the more robust number here",
            distribution.outliers.len(),
            distribution
                .outliers
                .iter()
                .map(|outlier| format!("#{} {}", outlier.iteration, fmt::duration_ms(outlier.value)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let baseline_failures = baseline.failed_iterations();
    if !baseline_failures.is_empty() {
        warnings.push(format!(
            "{} baseline iteration(s) did not complete cleanly ({})",
            baseline_failures.len(),
            baseline_failures
                .iter()
                .map(|(iteration, reason)| format!("#{iteration} {reason}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // ---- reliability -------------------------------------------------------
    if let (Some(baseline_stats), Some(candidate_stats)) = (baseline_stats, candidate_stats) {
        warnings.extend(reliability_warnings(
            "baseline",
            &baseline_stats,
            NOISE_LIMIT_PERCENT,
        ));
        warnings.extend(reliability_warnings(
            "experiment",
            &candidate_stats,
            NOISE_LIMIT_PERCENT,
        ));
        if baseline_stats.count != candidate_stats.count {
            warnings.push(format!(
                "sample counts differ (baseline {} vs experiment {}): the comparison is less symmetrical than it should be",
                baseline_stats.count, candidate_stats.count
            ));
        }
    }
    if !baseline.machine.compatible_with(&candidate.machine) {
        warnings.push(format!(
            "the two runs describe different machines ({} vs {}): results may not be comparable",
            baseline.machine.summary(),
            candidate.machine.summary()
        ));
    }
    if baseline.commit == candidate.commit {
        warnings.push(
            "baseline and experiment ran the same commit: no change was measured".to_string(),
        );
    }
    if baseline.repro.clean_env != candidate.repro.clean_env {
        warnings.push(
            "environment isolation differs between the two runs (limits.clean_env)".to_string(),
        );
    }

    Comparison {
        experiment: candidate.experiment.clone(),
        baseline_label: baseline.label.clone(),
        candidate_label: candidate.label.clone(),
        baseline_commit: baseline.commit.clone(),
        candidate_commit: candidate.commit.clone(),
        threshold_percent,
        metrics,
        checks,
        warnings,
        significance,
        baseline_distribution,
        candidate_distribution,
        baseline_samples: baseline_samples.into_iter().map(|(_, value)| value).collect(),
        candidate_samples: candidate_samples.into_iter().map(|(_, value)| value).collect(),
    }
}

/// One-line description of a sample distribution.
pub fn variance_text(stats: &Stats) -> String {
    format!(
        "{} samples · sigma {} ({})",
        stats.count,
        fmt::duration_ms(stats.stddev),
        fmt::percent(stats.cv_percent)
    )
}

/// Pads a table cell; numeric columns are right aligned, text columns left aligned.
fn pad(text: &str, width: usize, right_align: bool) -> String {
    let length = text.chars().count();
    if length >= width {
        return text.to_string();
    }
    let filler = " ".repeat(width - length);
    if right_align {
        format!("{filler}{text}")
    } else {
        format!("{text}{filler}")
    }
}

/// Renders the metric table (plan §5 style: baseline / experiment / difference / verdict).
pub fn render_metric_table(metrics: &[MetricDelta]) -> String {
    let headers = ["metric", "baseline", "experiment", "difference", "verdict"];
    let rows: Vec<[String; 5]> = metrics
        .iter()
        .map(|metric| {
            [
                metric.name.clone(),
                metric.baseline_text(),
                metric.candidate_text(),
                metric.delta_text(),
                metric.verdict.as_str().to_string(),
            ]
        })
        .collect();

    let mut widths = [0usize; 5];
    for (index, header) in headers.iter().enumerate() {
        widths[index] = header.chars().count();
    }
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    let mut out = String::new();
    let header_cells: Vec<String> = headers
        .iter()
        .enumerate()
        .map(|(index, header)| pad(header, widths[index], index > 0))
        .collect();
    out.push_str(&format!("  {}\n", header_cells.join("  ")));
    out.push_str(&format!(
        "  {}\n",
        "-".repeat(widths.iter().sum::<usize>() + 2 * (widths.len() - 1))
    ));
    for row in &rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(index, cell)| pad(cell, widths[index], index > 0))
            .collect();
        out.push_str(&format!("  {}\n", cells.join("  ")));
    }
    out
}

impl Comparison {
    /// Full plain-text rendering: the numbers first, then what to trust about them.
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "baseline    {}  ({})\n",
            self.baseline_label,
            fmt::short_sha(&self.baseline_commit)
        ));
        out.push_str(&format!(
            "experiment  {}  ({})\n\n",
            self.candidate_label,
            fmt::short_sha(&self.candidate_commit)
        ));
        out.push_str(&render_metric_table(&self.metrics));

        let context = self.context_metric_names();
        if !context.is_empty() {
            out.push_str(&format!(
                "\n  context metrics (reported, not counted in the verdict): {}\n",
                context.join(", ")
            ));
        }

        let has_statistics = self.significance.is_some()
            || self.baseline_distribution.is_some()
            || self.candidate_distribution.is_some();
        if has_statistics {
            out.push_str("\nstatistics\n");
        }
        if let Some(significance) = &self.significance {
            out.push_str(&format!("  {}\n", significance.describe()));
        }
        for (label, distribution, samples) in [
            (
                "baseline  ",
                self.baseline_distribution.as_ref(),
                &self.baseline_samples,
            ),
            (
                "experiment",
                self.candidate_distribution.as_ref(),
                &self.candidate_samples,
            ),
        ] {
            let Some(distribution) = distribution else {
                continue;
            };
            out.push_str(&format!("  {label}  {}\n", distribution.describe()));
            if samples.len() > 1 {
                out.push_str(&format!(
                    "  {label}  {} (measurement order)\n",
                    crate::statistics::sparkline(samples)
                ));
            }
        }

        let graphs = self.render_graphs();
        if !graphs.is_empty() {
            out.push_str("\ndistribution\n");
            out.push_str(&graphs);
        }

        out.push('\n');
        out.push_str("correctness\n");
        for check in &self.checks {
            out.push_str(&format!(
                "  {:<8} {} - {}\n",
                check.state.as_str(),
                check.name,
                check.detail
            ));
        }

        if !self.warnings.is_empty() {
            out.push('\n');
            out.push_str("reliability\n");
            for warning in &self.warnings {
                out.push_str(&format!("  {:<8} {warning}\n", "warn"));
            }
        }

        let (improved, regressed, neutral) = self.tally();
        out.push('\n');
        out.push_str(&format!(
            "verdict  {improved} improved, {regressed} regressed, {neutral} neutral (threshold +/-{:.1}%)\n",
            self.threshold_percent
        ));
        if self.has_failures() {
            out.push_str(
                "         a correctness check failed: these numbers describe a state that does not work\n",
            );
        }
        out
    }

    /// Compact one-line summary, used in `aion experiment create` hints and notifications.
    pub fn headline(&self) -> String {
        let primary = self
            .metrics
            .iter()
            .find(|metric| metric.name == "startup (mean)");
        match primary.and_then(|metric| metric.delta_percent) {
            Some(delta) => format!(
                "startup mean {} ({})",
                fmt::delta_percent(delta),
                primary.map(|metric| metric.verdict.as_str()).unwrap_or("n/a")
            ),
            None => "no comparable measurement".to_string(),
        }
    }

    /// Sparklines and histograms of both sides, or an empty string when there is nothing to plot.
    ///
    /// Histograms are only drawn from four samples upwards: two bars of one count each would be
    /// decoration, not information.
    pub fn render_graphs(&self) -> String {
        let mut out = String::new();
        for (label, samples) in [
            ("baseline", &self.baseline_samples),
            ("experiment", &self.candidate_samples),
        ] {
            if samples.len() > 1 {
                out.push_str(&format!(
                    "  {label:<11} {}\n",
                    crate::statistics::sparkline(samples)
                ));
            }
            if samples.len() >= 4 {
                for line in crate::statistics::histogram(samples, 4, 20) {
                    out.push_str(&format!("  {label:<11} {line}\n"));
                }
            }
        }
        out
    }
}
