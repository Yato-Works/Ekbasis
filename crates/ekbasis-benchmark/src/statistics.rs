//! Statistical treatment of measured samples (plan §17).
//!
//! AION keeps two kinds of numbers strictly apart:
//!
//! * *descriptive* numbers ([`Distribution`]) that describe what was observed, and
//! * *inferential* numbers ([`Significance`]) that say how much of an observed difference a
//!   two-sample Welch t-test can attribute to the change rather than to run-to-run noise.
//!
//! Nothing here changes a measurement — these functions only decide how much confidence a
//! comparison has earned.

use serde::{Deserialize, Serialize};

use ekbasis_core::fmt;
use ekbasis_core::model::Stats;

/// Alpha used for the significance decision (a 5% false-positive rate).
pub const ALPHA: f64 = 0.05;

/// Name of the test used for the significance decision.
pub const WELCH_TEST: &str = "Welch two-sample t-test";

/// Fraction trimmed from each end when computing the trimmed mean.
///
/// A 20% trimmed mean is the robust default for small samples: with five iterations it drops the
/// single fastest and the single slowest run.
const TRIM_FRACTION: f64 = 0.2;

/// One sample that sits far outside the rest of its group (Tukey's fences).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Outlier {
    /// 1-based iteration the value came from.
    pub iteration: u32,
    pub value: f64,
}

/// Descriptive statistics of one measured group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Distribution {
    /// The statistics AION has always reported (mean, median, min, max, sigma, CV, p95).
    pub stats: Stats,
    /// Mean with [`TRIM_FRACTION`] of each end removed: a robust read on the centre.
    pub trimmed_mean: f64,
    pub q1: f64,
    pub q3: f64,
    /// Interquartile range (`q3 - q1`).
    pub iqr: f64,
    /// Median absolute deviation: a noise estimate that ignores outliers.
    pub mad: f64,
    /// Samples outside Tukey's fences.
    pub outliers: Vec<Outlier>,
}

impl Distribution {
    /// Builds the distribution of `(iteration, duration_ms)` pairs.
    pub fn from_samples(samples: &[(u32, f64)]) -> Option<Distribution> {
        let values: Vec<f64> = samples
            .iter()
            .map(|(_, value)| *value)
            .filter(|value| value.is_finite())
            .collect();
        let stats = Stats::from_samples(&values)?;

        let mut sorted = values;
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let q1 = percentile(&sorted, 0.25);
        let q3 = percentile(&sorted, 0.75);
        let iqr = q3 - q1;
        let deviations: Vec<f64> = sorted
            .iter()
            .map(|value| (value - stats.median).abs())
            .collect();
        let mad = median(&deviations);
        let trimmed_mean = trimmed_mean(&sorted, TRIM_FRACTION);

        // Fences: Tukey's 1.5·IQR band. When the interquartile range is zero — constant or
        // near-constant runs — the band collapses to a point and every jitter sample would be
        // reported as an outlier, so a ±10% band around the median replaces it. A real spike
        // (e.g. 500 ms among 100 ms runs) still sits far outside that band.
        let (low_fence, high_fence) = if iqr > 0.0 {
            (q1 - 1.5 * iqr, q3 + 1.5 * iqr)
        } else if stats.median > 0.0 {
            (stats.median * 0.9, stats.median * 1.1)
        } else {
            (q1, q3)
        };
        let outliers = samples
            .iter()
            .filter(|(_, value)| value.is_finite() && (*value < low_fence || *value > high_fence))
            .map(|(iteration, value)| Outlier {
                iteration: *iteration,
                value: *value,
            })
            .collect();

        Some(Distribution {
            stats,
            trimmed_mean,
            q1,
            q3,
            iqr,
            mad,
            outliers,
        })
    }

    /// One-line summary used by the CLI and the Markdown report.
    pub fn describe(&self) -> String {
        let mut text = format!(
            "{} samples · mean {} · median {} · trimmed {} · sigma {} ({}) · MAD {}",
            self.stats.count,
            fmt::duration_ms(self.stats.mean),
            fmt::duration_ms(self.stats.median),
            fmt::duration_ms(self.trimmed_mean),
            fmt::duration_ms(self.stats.stddev),
            fmt::percent(self.stats.cv_percent),
            fmt::duration_ms(self.mad),
        );
        if !self.outliers.is_empty() {
            let listed = self
                .outliers
                .iter()
                .map(|outlier| format!("#{} {}", outlier.iteration, fmt::duration_ms(outlier.value)))
                .collect::<Vec<_>>()
                .join(", ");
            text.push_str(&format!(" · outliers: {listed}"));
        }
        text
    }
}

/// Result of Welch's two-sample t-test on baseline vs. experiment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Significance {
    /// Name of the test, so a stored report stays self-describing.
    pub test: String,
    /// Test statistic.
    pub t: f64,
    /// Welch–Satterthwaite degrees of freedom (fractional).
    pub df: f64,
    /// Two-sided p-value.
    pub p_value: f64,
    /// Lower bound of the 95% confidence interval of `candidate - baseline`.
    pub ci95_low: f64,
    /// Upper bound of that interval.
    pub ci95_high: f64,
    /// Cohen's d using the pooled standard deviation.
    pub cohens_d: f64,
    pub baseline_n: usize,
    pub candidate_n: usize,
}

impl Significance {
    /// True when the difference is unlikely to be pure noise (`p < ALPHA`).
    pub fn significant(&self) -> bool {
        self.p_value < ALPHA
    }

    /// True when the 95% confidence interval of the difference excludes zero.
    pub fn interval_excludes_zero(&self) -> bool {
        self.ci95_low > 0.0 || self.ci95_high < 0.0
    }

    /// Cohen's conventions for the magnitude of an effect.
    pub fn effect_label(&self) -> &'static str {
        let magnitude = self.cohens_d.abs();
        if magnitude < 0.2 {
            "negligible"
        } else if magnitude < 0.5 {
            "small"
        } else if magnitude < 0.8 {
            "medium"
        } else {
            "large"
        }
    }

    /// One-line summary, with durations rendered in the unit AION measured.
    pub fn describe(&self) -> String {
        let difference = self.ci95_high - (self.ci95_high - self.ci95_low) / 2.0;
        format!(
            "{}: t = {:.2}, df = {:.1}, p = {}, 95% CI of the difference [{}, {}] (centre {}), Cohen's d = {:.2} ({})",
            self.test,
            self.t,
            self.df,
            fmt::probability(self.p_value),
            fmt::duration_ms(self.ci95_low),
            fmt::duration_ms(self.ci95_high),
            fmt::duration_ms(difference),
            self.cohens_d,
            self.effect_label(),
        )
    }
}

/// Runs Welch's t-test on two independent samples.
///
/// Returns `None` when either group has fewer than two samples: a single measurement cannot say
/// anything about noise, and AION would rather stay silent than pretend otherwise.
pub fn welch(baseline: &[f64], candidate: &[f64]) -> Option<Significance> {
    let baseline: Vec<f64> = baseline
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    let candidate: Vec<f64> = candidate
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    if baseline.len() < 2 || candidate.len() < 2 {
        return None;
    }

    let n1 = baseline.len() as f64;
    let n2 = candidate.len() as f64;
    let mean1 = mean(&baseline);
    let mean2 = mean(&candidate);
    let var1 = sample_variance(&baseline, mean1);
    let var2 = sample_variance(&candidate, mean2);
    let difference = mean2 - mean1;
    let pooled_sd = (((n1 - 1.0) * var1 + (n2 - 1.0) * var2) / (n1 + n2 - 2.0)).sqrt();
    let cohens_d = if pooled_sd > 0.0 {
        difference / pooled_sd
    } else {
        0.0
    };

    let standard_error_squared = var1 / n1 + var2 / n2;
    // No standard error left (both groups perfectly constant) or numerically degenerate: the
    // difference is either exact or not there at all, so no t-test is needed.
    if standard_error_squared <= f64::MIN_POSITIVE || !standard_error_squared.is_finite() {
        return Some(Significance {
            test: WELCH_TEST.to_string(),
            t: if difference == 0.0 { 0.0 } else { f64::INFINITY.copysign(difference) },
            df: n1 + n2 - 2.0,
            p_value: if difference == 0.0 { 1.0 } else { 0.0 },
            ci95_low: difference,
            ci95_high: difference,
            cohens_d,
            baseline_n: baseline.len(),
            candidate_n: candidate.len(),
        });
    }

    let standard_error = standard_error_squared.sqrt();
    let t = difference / standard_error;
    let df = standard_error_squared.powi(2)
        / ((var1 / n1).powi(2) / (n1 - 1.0) + (var2 / n2).powi(2) / (n2 - 1.0));
    let p_value = student_t_two_sided_p(t, df);
    let t_critical_value = t_critical(ALPHA, df);

    Some(Significance {
        test: WELCH_TEST.to_string(),
        t,
        df,
        p_value,
        ci95_low: difference - t_critical_value * standard_error,
        ci95_high: difference + t_critical_value * standard_error,
        cohens_d,
        baseline_n: baseline.len(),
        candidate_n: candidate.len(),
    })
}

// ---------------------------------------------------------------------------
// Numeric helpers
// ---------------------------------------------------------------------------

/// Arithmetic mean of a non-empty slice.
fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

/// Unbiased (`n - 1`) sample variance around a known `mean`.
fn sample_variance(values: &[f64], mean: f64) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (values.len() - 1) as f64
}

/// Median of a slice (works on a sorted copy).
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    percentile(&sorted, 0.5)
}

/// Percentile of an already sorted slice, with linear interpolation (the R-7 rule).
fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    match sorted.len() {
        0 => 0.0,
        1 => sorted[0],
        len => {
            let position = fraction.clamp(0.0, 1.0) * (len - 1) as f64;
            let lower = position.floor() as usize;
            let upper = position.ceil() as usize;
            if lower == upper {
                sorted[lower]
            } else {
                let weight = position - lower as f64;
                sorted[lower] * (1.0 - weight) + sorted[upper] * weight
            }
        }
    }
}

/// Mean with `fraction` of each end of an already sorted slice removed.
///
/// A 20% trimmed mean is the robust default for small samples: with five iterations it drops the
/// single fastest and the single slowest run, which is exactly the kind of noise AION produces on
/// a developer machine.
fn trimmed_mean(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let floor_cut = (sorted.len() as f64 * fraction).floor() as usize;
    let cut = floor_cut.min((sorted.len().saturating_sub(1)) / 2);
    let window = &sorted[cut..sorted.len() - cut];
    mean(window)
}

/// Lanczos approximation of `ln Γ(x)`.
fn ln_gamma(x: f64) -> f64 {
    const COEFFICIENTS: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // Reflection formula: Γ(x)Γ(1-x) = π / sin(πx)
        let pi = std::f64::consts::PI;
        return (pi / (pi * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let shifted = x - 1.0;
    let mut sum = COEFFICIENTS[0];
    for (index, coefficient) in COEFFICIENTS.iter().enumerate().skip(1) {
        sum += coefficient / (shifted + index as f64);
    }
    let t = shifted + 7.5;
    0.5 * (2.0 * std::f64::consts::PI).ln() + (shifted + 0.5) * t.ln() - t + sum.ln()
}

/// Continued fraction for the incomplete beta function (modified Lentz's method).
fn betacf(a: f64, b: f64, x: f64) -> f64 {
    const MAX_ITERATIONS: u32 = 300;
    const EPSILON: f64 = 3.0e-12;
    const TINY: f64 = 1.0e-300;

    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;

    for step in 1..=MAX_ITERATIONS {
        let m = step as f64;
        let m2 = 2.0 * m;
        let numerator = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + numerator * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + numerator / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;

        let numerator = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + numerator * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + numerator / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;

        if (delta - 1.0).abs() < EPSILON {
            break;
        }
    }
    h
}

/// Regularised incomplete beta function `I_x(a, b)`.
fn inc_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front =
        (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln()).exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        front * betacf(a, b, x) / a
    } else {
        1.0 - front * betacf(b, a, 1.0 - x) / b
    }
}

/// Two-sided p-value of Student's t distribution: `P(|T| > t)`.
fn student_t_two_sided_p(t: f64, df: f64) -> f64 {
    if !t.is_finite() {
        return 0.0;
    }
    if df <= 0.0 {
        return 1.0;
    }
    let x = df / (df + t * t);
    inc_beta(df / 2.0, 0.5, x).clamp(0.0, 1.0)
}

/// Critical value for a two-sided interval: the `t` for which `P(|T| > t) = alpha`.
///
/// `alpha = 0.05` therefore yields the classic 5% critical value (2.306 for 8 degrees of freedom),
/// which is what a 95% confidence interval needs.
fn t_critical(alpha: f64, df: f64) -> f64 {
    let (mut low, mut high) = (0.0_f64, 1_000.0_f64);
    for _ in 0..200 {
        let middle = (low + high) / 2.0;
        if student_t_two_sided_p(middle, df) > alpha {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

// ---------------------------------------------------------------------------
// Graphs
// ---------------------------------------------------------------------------

/// Renders a compact sparkline of a series, oldest sample first.
pub fn sparkline(values: &[f64]) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let finite: Vec<f64> = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    if finite.is_empty() {
        return String::new();
    }
    let min = finite.iter().copied().fold(f64::INFINITY, f64::min);
    let max = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;
    finite
        .iter()
        .map(|value| {
            if span <= f64::EPSILON {
                BLOCKS[BLOCKS.len() / 2]
            } else {
                let position = ((value - min) / span * (BLOCKS.len() - 1) as f64).round() as usize;
                BLOCKS[position.min(BLOCKS.len() - 1)]
            }
        })
        .collect()
}

/// Renders an ASCII histogram: one line per bin, ready for a terminal or a Markdown code block.
pub fn histogram(values: &[f64], bins: usize, width: usize) -> Vec<String> {
    let finite: Vec<f64> = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    if finite.is_empty() || bins == 0 || width == 0 {
        return Vec::new();
    }
    let min = finite.iter().copied().fold(f64::INFINITY, f64::min);
    let max = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;

    let mut counts = vec![0usize; bins];
    for value in &finite {
        let index = if span <= f64::EPSILON {
            bins / 2
        } else {
            let scaled = (value - min) / span;
            ((scaled * bins as f64).floor() as usize).min(bins - 1)
        };
        counts[index] += 1;
    }

    let peak = counts.iter().copied().max().unwrap_or(1).max(1);
    counts
        .iter()
        .enumerate()
        .map(|(index, count)| {
            let low = min + span * index as f64 / bins as f64;
            let bar_width = ((*count as f64 / peak as f64) * width as f64).round() as usize;
            let bar = "#".repeat(bar_width.max(usize::from(*count > 0)));
            let padded = format!("{bar:<width$}", width = width);
            format!("{:<9} |{padded}| {}", fmt::duration_ms(low), count)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(values: &[(u32, f64)]) -> Distribution {
        Distribution::from_samples(values).expect("distribution")
    }

    #[test]
    fn ln_gamma_matches_known_values() {
        assert!((ln_gamma(1.0) - 0.0).abs() < 1e-9);
        assert!((ln_gamma(5.0) - 24.0_f64.ln()).abs() < 1e-9);
        assert!((ln_gamma(0.5) - std::f64::consts::PI.sqrt().ln()).abs() < 1e-9);
        // Γ(2.5) = 1.3293403881791370, so ln Γ(2.5) = 0.2846828704729192.
        assert!((ln_gamma(2.5) - 1.329_340_388_179_137_f64.ln()).abs() < 1e-9);
    }

    #[test]
    fn student_t_p_values_match_published_tables() {
        // t = 2.0 with 8 degrees of freedom is the classic p = 0.0805 case.
        assert!((student_t_two_sided_p(2.0, 8.0) - 0.0805).abs() < 5e-4);
        // t = 2.306 with 8 df is the 5% critical value.
        assert!((student_t_two_sided_p(2.306, 8.0) - 0.05).abs() < 5e-4);
        assert!((t_critical(0.05, 8.0) - 2.306).abs() < 1e-3);
        assert!((t_critical(0.05, 30.0) - 2.042).abs() < 2e-3);
        assert!((t_critical(0.05, 1000.0) - 1.962).abs() < 2e-3);
        assert!((t_critical(0.01, 8.0) - 3.355).abs() < 2e-3);
    }

    #[test]
    fn welch_reproduces_the_hand_computed_example() {
        let baseline = [1.0, 2.0, 3.0, 4.0, 5.0];
        let candidate = [3.0, 4.0, 5.0, 6.0, 7.0];
        let result = welch(&baseline, &candidate).expect("test result");
        assert!((result.t - 2.0).abs() < 1e-9, "{result:?}");
        assert!((result.df - 8.0).abs() < 1e-9, "{result:?}");
        assert!((result.p_value - 0.0805).abs() < 5e-4, "{result:?}");
        assert!(!result.significant());
        assert!(
            !result.interval_excludes_zero(),
            "the confidence interval must contain zero: {result:?}"
        );
        assert!((result.cohens_d - 1.2649).abs() < 1e-3, "{result:?}");
        assert_eq!(result.effect_label(), "large");
        assert!(result.describe().contains("Welch"));
    }

    #[test]
    fn a_clear_difference_is_significant() {
        let baseline = [100.0, 101.0, 99.0, 100.0, 100.0];
        let candidate = [50.0, 51.0, 49.0, 50.0, 50.0];
        let result = welch(&baseline, &candidate).expect("test result");
        assert!(result.significant(), "{result:?}");
        assert!(result.interval_excludes_zero(), "{result:?}");
        assert!(result.p_value < 1e-6, "{result:?}");
        assert!(
            result.ci95_high < 0.0,
            "the whole interval lies below zero: {result:?}"
        );
    }

    #[test]
    fn identical_and_degenerate_groups_are_handled() {
        let identical = welch(&[10.0, 11.0, 12.0], &[10.0, 11.0, 12.0]).expect("test result");
        assert!(!identical.significant());
        assert!((identical.p_value - 1.0).abs() < 1e-9, "{identical:?}");

        // Perfectly constant groups: no variance left, so the difference is either exact or absent.
        let exact = welch(&[10.0, 10.0, 10.0], &[20.0, 20.0, 20.0]).expect("test result");
        assert_eq!(exact.p_value, 0.0);
        assert!(exact.significant());

        assert!(
            welch(&[1.0], &[2.0, 3.0]).is_none(),
            "a single sample is not a group"
        );
        assert!(welch(&[], &[2.0, 3.0]).is_none());
    }

    #[test]
    fn distribution_reports_robust_centre_and_outliers() {
        let distribution = group(&[(1, 1.0), (2, 2.0), (3, 3.0), (4, 4.0), (5, 5.0)]);
        assert!((distribution.q1 - 2.0).abs() < 1e-9);
        assert!((distribution.q3 - 4.0).abs() < 1e-9);
        assert!((distribution.iqr - 2.0).abs() < 1e-9);
        assert!((distribution.mad - 1.0).abs() < 1e-9);
        assert!((distribution.trimmed_mean - 3.0).abs() < 1e-9);
        assert!(distribution.outliers.is_empty());

        // A single slow run must be reported, not silently averaged away.
        let spiky = group(&[(1, 100.0), (2, 100.0), (3, 100.0), (4, 100.0), (5, 500.0)]);
        assert_eq!(spiky.outliers.len(), 1);
        assert_eq!(spiky.outliers[0].iteration, 5);
        assert!((spiky.outliers[0].value - 500.0).abs() < 1e-9);
        assert!(spiky.describe().contains("outliers"));
        assert!(
            spiky.trimmed_mean < spiky.stats.mean,
            "trimming must resist the spike"
        );
    }

    #[test]
    fn distributions_without_finite_samples_are_rejected() {
        assert!(Distribution::from_samples(&[]).is_none());
        assert!(Distribution::from_samples(&[(1, f64::NAN)]).is_none());
    }
}
