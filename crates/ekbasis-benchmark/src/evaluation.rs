//! Evaluation of one alternative future (plan §19).
//!
//! AION does not score software with a magic number: each dimension is derived from a specific,
//! already-computed part of a [`Comparison`] and the derivation is written down next to the code.
//! The four dimensions are the ones plan §19 attaches to every future:
//!
//! * **performance** — the primary metric (mean duration) with Welch's t-test behind it,
//! * **memory** — peak resident memory of the measured command,
//! * **correctness** — build, tests and iteration checks of the comparison,
//! * **stability** — how much the experiment's own numbers jitter (CV, outliers, sample count).
//!
//! An overall grade is the *weakest* known dimension: a future that is fast but broken is a
//! broken future.

use serde::{Deserialize, Serialize};

use crate::{CheckState, Comparison, NOISE_LIMIT_PERCENT, Verdict};

/// Grade of a single dimension. Ordered best (`A`) to worst (`F`), `Unknown` last so that
/// `min()` over known grades ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Grade {
    A,
    B,
    C,
    D,
    F,
    /// Not enough evidence to grade this dimension.
    Unknown,
}

impl Grade {
    /// Single character used in tables and terminal output (`-` for unknown).
    pub fn as_char(self) -> char {
        match self {
            Grade::A => 'A',
            Grade::B => 'B',
            Grade::C => 'C',
            Grade::D => 'D',
            Grade::F => 'F',
            Grade::Unknown => '-',
        }
    }

    /// Short human label.
    pub fn as_str(self) -> &'static str {
        match self {
            Grade::A => "excellent",
            Grade::B => "good",
            Grade::C => "acceptable",
            Grade::D => "weak",
            Grade::F => "failing",
            Grade::Unknown => "unknown",
        }
    }

    /// One step worse (used when outliers or too few samples weaken a stability read).
    pub fn stepped_down(self) -> Grade {
        match self {
            Grade::A => Grade::B,
            Grade::B => Grade::C,
            Grade::C => Grade::D,
            Grade::D | Grade::F => Grade::F,
            Grade::Unknown => Grade::Unknown,
        }
    }
}

impl std::fmt::Display for Grade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_char())
    }
}

/// One evaluated dimension of a future.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dimension {
    /// Stable key (`performance`, `memory`, `correctness`, `stability`).
    pub key: String,
    /// Display title (`Performance`, ...).
    pub title: String,
    pub grade: Grade,
    /// One-line summary, e.g. `-53.0% (p = 3.4e-10, significant)`.
    pub summary: String,
    /// The evidence behind the grade, in full.
    pub detail: String,
}

/// The four evaluations of one future plus the overall grade (plan §19).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evaluation {
    pub performance: Dimension,
    pub memory: Dimension,
    pub correctness: Dimension,
    pub stability: Dimension,
    /// Weakest known grade across the four dimensions (unknown dimensions are ignored unless
    /// every dimension is unknown).
    pub overall: Grade,
}

impl Evaluation {
    /// The four dimensions in display order.
    pub fn dimensions(&self) -> [&Dimension; 4] {
        [
            &self.performance,
            &self.memory,
            &self.correctness,
            &self.stability,
        ]
    }

    /// Compact `P M C S` letters, e.g. `A C A B` (`-` for unknown dimensions).
    pub fn letters(&self) -> String {
        self.dimensions()
            .iter()
            .map(|dimension| dimension.grade.as_char().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Titles of the dimensions that hold the overall grade (the weakest ones).
    pub fn limiting_dimensions(&self) -> Vec<&str> {
        if self.overall == Grade::Unknown {
            return Vec::new();
        }
        self.dimensions()
            .iter()
            .filter(|dimension| dimension.grade == self.overall)
            .map(|dimension| dimension.title.as_str())
            .collect()
    }

    /// Block rendering for the terminal.
    pub fn render(&self) -> String {
        let mut out = String::from("evaluation\n");
        for dimension in self.dimensions() {
            out.push_str(&format!(
                "  {:<12} {}  {:<34} {}\n",
                dimension.key, dimension.grade, dimension.summary, dimension.detail,
            ));
        }
        let limiting = self.limiting_dimensions();
        out.push_str(&format!(
            "  {:<12} {}\n",
            "overall",
            if limiting.is_empty() {
                format!("{}", self.overall)
            } else {
                format!("{} ({})", self.overall, limiting.join(", ").to_lowercase())
            }
        ));
        out
    }
}


/// Evaluates one future against its baseline (plan §19).
///
/// Never invents a number: a dimension without evidence gets [`Grade::Unknown`] and says why.
pub fn evaluate(comparison: &Comparison) -> Evaluation {
    let performance = evaluate_performance(comparison);
    let memory = evaluate_memory(comparison);
    let correctness = evaluate_correctness(comparison);
    let stability = evaluate_stability(comparison);

    // Worst *known* grade: `Grade` orders A..F then Unknown, so `max` after filtering out
    // unknowns yields the weakest dimension (an all-unknown evaluation stays unknown).
    let overall = [
        performance.grade,
        memory.grade,
        correctness.grade,
        stability.grade,
    ]
    .into_iter()
    .filter(|grade| *grade != Grade::Unknown)
    .max()
    .unwrap_or(Grade::Unknown);

    Evaluation {
        performance,
        memory,
        correctness,
        stability,
        overall,
    }
}

/// Performance: the primary metric plus the significance of its difference.
fn evaluate_performance(comparison: &Comparison) -> Dimension {
    let Some(metric) = comparison.primary_metric() else {
        return dimension(
            "performance",
            "Performance",
            Grade::Unknown,
            "no primary metric",
            "the comparison carries no `startup (mean)` metric",
        );
    };
    let Some(delta) = metric.delta_percent else {
        let note = metric
            .note
            .clone()
            .unwrap_or_else(|| "one side has no measured duration".to_string());
        return dimension(
            "performance",
            "Performance",
            Grade::Unknown,
            "not comparable",
            &note,
        );
    };

    let significance = comparison.significance.as_ref();
    let significance_text = significance.map(|result| {
        format!(
            "p = {}, {}",
            ekbasis_core::fmt::probability(result.p_value),
            if result.significant() {
                "significant"
            } else {
                "not significant"
            }
        )
    });
    let delta_text = ekbasis_core::fmt::delta_percent(delta);

    let (grade, summary) = match metric.verdict {
        Verdict::Improved => match significance {
            None => (
                Grade::B,
                format!("{delta_text} (no t-test: fewer than two samples per side)"),
            ),
            Some(result) if result.significant() => (
                Grade::A,
                format!("{delta_text} ({})", significance_text.unwrap_or_default()),
            ),
            Some(_) => (
                Grade::B,
                format!(
                    "{delta_text} but not statistically significant ({})",
                    significance_text.unwrap_or_default()
                ),
            ),
        },
        Verdict::Neutral => (
            Grade::C,
            format!(
                "{delta_text} (within the ±{:.1}% threshold)",
                comparison.threshold_percent
            ),
        ),
        Verdict::Regressed => match significance {
            Some(result) if result.significant() => (
                Grade::F,
                format!("{delta_text} ({})", significance_text.unwrap_or_default()),
            ),
            Some(_) => (
                Grade::D,
                format!("{delta_text} ({})", significance_text.unwrap_or_default()),
            ),
            None => (Grade::D, format!("{delta_text} (no t-test available)")),
        },
        Verdict::Unavailable => {
            let note = metric
                .note
                .clone()
                .unwrap_or_else(|| "not measured".to_string());
            (Grade::Unknown, note)
        }
    };

    let detail = format!(
        "{} {} -> {}",
        metric.name,
        metric.baseline_text(),
        metric.candidate_text()
    );
    dimension(
        "performance",
        "Performance",
        grade,
        &summary,
        &detail,
    )
}


/// Memory: peak resident memory. A single spike flips this number, which the detail repeats.
fn evaluate_memory(comparison: &Comparison) -> Dimension {
    const NOTE: &str = "peak memory: one spike can flip this number";
    let Some(metric) = comparison
        .metrics
        .iter()
        .find(|metric| metric.name == "peak memory")
    else {
        return dimension(
            "memory",
            "Memory",
            Grade::Unknown,
            "no peak memory metric",
            "the comparison carries no memory measurement",
        );
    };

    let grade = match metric.verdict {
        Verdict::Improved => Grade::A,
        // The peak has no significance test (it is one number per run), so a regression beyond
        // the threshold is reported one step below a proven performance regression.
        Verdict::Regressed => Grade::D,
        Verdict::Neutral => Grade::C,
        Verdict::Unavailable => Grade::Unknown,
    };
    let summary = match metric.delta_percent {
        Some(delta) => format!(
            "{} ({})",
            ekbasis_core::fmt::delta_percent(delta),
            metric.verdict.as_str()
        ),
        None => metric
            .note
            .clone()
            .unwrap_or_else(|| "not measured".to_string()),
    };
    let detail = format!(
        "{} -> {}, {NOTE}",
        metric.baseline_text(),
        metric.candidate_text()
    );
    dimension("memory", "Memory", grade, &summary, &detail)
}

/// Correctness: build, tests and iteration checks of the comparison (the `significance` check
/// is a statistical statement, not a correctness statement, and is ignored here).
fn evaluate_correctness(comparison: &Comparison) -> Dimension {
    let relevant: Vec<_> = comparison
        .checks
        .iter()
        .filter(|check| !matches!(check.name.as_str(), "significance"))
        .collect();
    if relevant.is_empty() {
        return dimension(
            "correctness",
            "Correctness",
            Grade::Unknown,
            "no checks recorded",
            "the comparison has no build/test/iteration checks",
        );
    }

    let failed: Vec<&str> = relevant
        .iter()
        .filter(|check| check.state == CheckState::Fail)
        .map(|check| check.name.as_str())
        .collect();
    if !failed.is_empty() {
        let detail = relevant
            .iter()
            .filter(|check| check.state == CheckState::Fail)
            .map(|check| format!("{}: {}", check.name, check.detail))
            .collect::<Vec<_>>()
            .join("; ");
        let summary = format!("failed: {}", failed.join(", "));
        return dimension("correctness", "Correctness", Grade::F, &summary, &detail);
    }

    let tests_ran = relevant.iter().any(|check| {
        check.name == "tests"
            && check.state == CheckState::Pass
            && !check.detail.contains("no test command")
    });
    let grade = if tests_ran {
        Grade::A
    } else {
        // Everything that ran passed, but no test command was configured: correctness of the
        // state beyond "it builds and runs to completion" was not demonstrated.
        Grade::B
    };
    let summary = if tests_ran {
        "build, tests and iterations passed".to_string()
    } else {
        "build and iterations passed (no test command configured)".to_string()
    };
    let detail = relevant
        .iter()
        .map(|check| format!("{}: {}", check.name, check.detail))
        .collect::<Vec<_>>()
        .join("; ");
    dimension("correctness", "Correctness", grade, &summary, &detail)
}


/// Stability: how noisy the experiment's own numbers are. CV above the noise limit, outliers or
/// too few samples each step the grade down once — jumpy numbers cannot support small deltas.
fn evaluate_stability(comparison: &Comparison) -> Dimension {
    let Some(distribution) = comparison.candidate_distribution.as_ref() else {
        return dimension(
            "stability",
            "Stability",
            Grade::Unknown,
            "no successful samples",
            "the experiment side has no distribution to judge",
        );
    };
    let stats = distribution.stats;
    if stats.count < 2 {
        return dimension(
            "stability",
            "Stability",
            Grade::Unknown,
            &format!("only {} sample(s)", stats.count),
            "at least two successful samples are needed to speak about stability",
        );
    }

    let mut grade = if stats.cv_percent <= NOISE_LIMIT_PERCENT {
        Grade::A
    } else if stats.cv_percent <= 2.0 * NOISE_LIMIT_PERCENT {
        Grade::B
    } else if stats.cv_percent <= 4.0 * NOISE_LIMIT_PERCENT {
        Grade::C
    } else {
        Grade::D
    };
    let mut reasons: Vec<String> = Vec::new();
    if !distribution.outliers.is_empty() {
        grade = grade.stepped_down();
        reasons.push(format!("{} outlier(s)", distribution.outliers.len()));
    }
    if stats.count < 3 {
        grade = grade.stepped_down();
        reasons.push(format!("only {} sample(s)", stats.count));
    }

    let summary = format!(
        "cv {} over {} samples{}",
        ekbasis_core::fmt::percent(stats.cv_percent),
        stats.count,
        if reasons.is_empty() {
            String::new()
        } else {
            format!(", {}", reasons.join(", "))
        }
    );
    let detail = match comparison.baseline_distribution.as_ref() {
        Some(baseline) => format!(
            "experiment cv {}, baseline cv {} (noise limit {})",
            ekbasis_core::fmt::percent(stats.cv_percent),
            ekbasis_core::fmt::percent(baseline.stats.cv_percent),
            ekbasis_core::fmt::percent(NOISE_LIMIT_PERCENT)
        ),
        None => format!(
            "experiment cv {} (noise limit {})",
            ekbasis_core::fmt::percent(stats.cv_percent),
            ekbasis_core::fmt::percent(NOISE_LIMIT_PERCENT)
        ),
    };
    dimension("stability", "Stability", grade, &summary, &detail)
}

/// Assembles one dimension.
fn dimension(
    key: &'static str,
    title: &'static str,
    grade: Grade,
    summary: &str,
    detail: &str,
) -> Dimension {
    Dimension {
        key: key.to_string(),
        title: title.to_string(),
        grade,
        summary: summary.to_string(),
        detail: detail.to_string(),
    }
}

