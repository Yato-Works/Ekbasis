//! Human readable formatting helpers shared by the CLI and the report renderers.

use crate::model::Stats;

/// Bullet/glyph helpers. Used only in line-oriented output, never inside tables,
/// because emoji glyph width breaks column alignment.
pub const OK: &str = "[ok]";
pub const FAIL: &str = "[!!]";
pub const WARN: &str = "[??]";
pub const INFO: &str = "[--]";
pub const DOT: &str = "·";

/// Formats a wall-clock duration in milliseconds using a readable scale.
pub fn duration_ms(ms: f64) -> String {
    let ms = ms.max(0.0);
    if ms < 1.0 {
        format!("{ms:.2} ms")
    } else if ms < 1000.0 {
        format!("{ms:.1} ms")
    } else if ms < 60_000.0 {
        format!("{:.3} s", ms / 1000.0)
    } else {
        let total = ms / 1000.0;
        let minutes = (total / 60.0).floor();
        let seconds = total - minutes * 60.0;
        format!("{minutes:.0}m {seconds:.1}s")
    }
}

/// Formats a byte count with binary units.
pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

/// Formats a percentage change with an explicit sign, e.g. `-18.7%`.
pub fn delta_percent(value: f64) -> String {
    format!("{value:+.1}%")
}

/// Formats a plain percentage, e.g. `41.0%`.
pub fn percent(value: f64) -> String {
    format!("{value:.1}%")
}

/// Formats a temperature, e.g. `72.4 °C`.
pub fn temperature_c(value: f32) -> String {
    format!("{value:.1} °C")
}

/// Formats a probability, using scientific notation for very small values.
pub fn probability(value: f64) -> String {
    if value <= 0.0 {
        "< 1e-12".to_string()
    } else if value < 0.001 {
        format!("{value:.2e}")
    } else {
        format!("{value:.4}")
    }
}

/// First seven characters of a commit hash.
pub fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// Truncates a single line to `max` characters, appending `…` when shortened.
pub fn truncate_line(text: &str, max: usize) -> String {
    let single = text.lines().next().unwrap_or("").trim();
    if single.chars().count() <= max {
        single.to_string()
    } else {
        let mut out: String = single.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// One-line description of a sample distribution.
pub fn stats_line(stats: &Stats) -> String {
    format!(
        "{} samples {DOT} mean {} {DOT} sigma {} ({}) {DOT} min {} {DOT} max {}",
        stats.count,
        duration_ms(stats.mean),
        duration_ms(stats.stddev),
        percent(stats.cv_percent),
        duration_ms(stats.min),
        duration_ms(stats.max),
    )
}
