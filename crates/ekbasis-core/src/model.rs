//! Serializable data models: the *observation language* of AION.
//!
//! These structs are the on-disk format of `.aion/results/*.json` and the in-memory
//! format passed between crates. They describe facts (what ran, for how long, with what
//! exit code, on which machine) — never predictions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::spec::CommandSpec;

/// Which side of an experiment a run belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunKind {
    /// The unmodified base commit.
    Baseline,
    /// The alternative timeline: base commit + experiment changes.
    Experiment,
}

impl RunKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RunKind::Baseline => "baseline",
            RunKind::Experiment => "experiment",
        }
    }

    /// Parses a user supplied kind (`baseline`, `base`, `experiment`, `exp`, ...).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "baseline" | "base" | "control" => Some(RunKind::Baseline),
            "experiment" | "exp" | "candidate" | "future" => Some(RunKind::Experiment),
            _ => None,
        }
    }
}

impl std::fmt::Display for RunKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single step of the experiment pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhaseKind {
    /// One-off preparation command (`command.setup`).
    Setup,
    /// Compilation / packaging.
    Build,
    /// Test suite execution.
    Test,
    /// A discarded warm-up iteration.
    Warmup,
    /// A measured benchmark iteration.
    Run,
}

impl PhaseKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PhaseKind::Setup => "setup",
            PhaseKind::Build => "build",
            PhaseKind::Test => "test",
            PhaseKind::Warmup => "warmup",
            PhaseKind::Run => "run",
        }
    }
}

impl std::fmt::Display for PhaseKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The recorded output of one executed command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseReport {
    pub kind: PhaseKind,
    /// 1-based iteration index for benchmark iterations; `None` for one-shot phases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    pub command: String,
    /// `direct` when the command was spawned without a shell, `shell` when it was
    /// interpreted by `cmd /C` (Windows) or `sh -c` (Unix).
    pub spawn_mode: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub timed_out: bool,
    /// `Some(limit)` when the command tree was killed for exceeding `limits.max_processes`
    /// (plan §15). Serialized only when set, so old reports stay byte-compatible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_hit: Option<u32>,
    pub duration_ms: f64,
    pub stdout: String,
    pub stderr: String,
    #[serde(default)]
    pub stdout_truncated: bool,
    #[serde(default)]
    pub stderr_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<RuntimeObservation>,
}

/// Resource usage observed while a command was running.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeObservation {
    /// Number of successfully attached samples.
    pub samples: u32,
    /// Requested sampling interval in milliseconds.
    pub interval_ms: u64,
    /// Highest resident memory of the observed process tree, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_memory_bytes: Option<u64>,
    /// Mean CPU usage of the process tree in percent of one core (may exceed 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avg_cpu_percent: Option<f32>,
    /// Highest CPU usage observed for the tree, in percent of one core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cpu_percent: Option<f32>,
    /// Highest GPU utilisation observed while the command ran, in percent (plan §6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_utilization_percent: Option<f32>,
    /// Highest GPU memory usage observed while the command ran, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_memory_bytes: Option<u64>,
    /// Highest temperature observed while the command ran, in degrees Celsius.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_temperature_c: Option<f32>,
    /// Highest number of processes seen in the tree.
    #[serde(default)]
    pub peak_process_count: u32,
    /// Set when observation was requested but could not be attached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl RuntimeObservation {
    /// True when at least one resource metric was actually measured.
    pub fn has_metrics(&self) -> bool {
        self.peak_memory_bytes.is_some()
            || self.avg_cpu_percent.is_some()
            || self.gpu_utilization_percent.is_some()
            || self.max_temperature_c.is_some()
    }
}

/// One measured benchmark iteration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleRecord {
    pub iteration: u32,
    pub duration_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub success: bool,
    pub timed_out: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<RuntimeObservation>,
}

/// One graphics adapter reported by the operating system (plan §6, §17).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    /// Video memory in bytes, when the driver reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_version: Option<String>,
}

impl GpuInfo {
    /// `NVIDIA GeForce RTX 4070 (12.0 GiB)`.
    pub fn summary(&self) -> String {
        match self.memory_bytes {
            Some(memory) if memory > 0 => format!("{} ({})", self.name, crate::fmt::bytes(memory)),
            _ => self.name.clone(),
        }
    }
}

/// Machine a run happened on (reproducibility metadata, plan §7).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MachineInfo {
    /// Operating system name, e.g. `Windows`.
    pub os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// Target architecture AION itself was built for.
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_brand: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_physical_cores: Option<usize>,
    #[serde(default)]
    pub cpu_logical_cores: usize,
    #[serde(default)]
    pub total_memory_bytes: u64,
    /// Graphics adapters, best effort: empty when the platform reports none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gpus: Vec<GpuInfo>,
    /// Free space on the volume that holds the measured checkout, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_free_bytes: Option<u64>,
    /// Total size of that volume, in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_total_bytes: Option<u64>,
}

impl MachineInfo {
    /// One-line summary, e.g. `Windows 11 Pro · AMD Ryzen 9 5900X · 12/24 cores · 32.0 GiB`.
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        parts.push(match &self.os_version {
            Some(version) if !version.is_empty() => format!("{} {}", self.os, version),
            _ => self.os.clone(),
        });
        if let Some(cpu) = &self.cpu_brand {
            if !cpu.is_empty() {
                parts.push(cpu.clone());
            }
        }
        let cores = match self.cpu_physical_cores {
            Some(physical) => format!("{physical}/{} cores", self.cpu_logical_cores),
            None => format!("{} cores", self.cpu_logical_cores),
        };
        parts.push(cores);
        if self.total_memory_bytes > 0 {
            parts.push(crate::fmt::bytes(self.total_memory_bytes));
        }
        parts.join(" · ")
    }

    /// Graphics adapters as one line, e.g. `NVIDIA RTX 4070 (12.0 GiB), Intel UHD 750`.
    pub fn gpu_summary(&self) -> Option<String> {
        if self.gpus.is_empty() {
            return None;
        }
        Some(
            self.gpus
                .iter()
                .map(|gpu| gpu.summary())
                .collect::<Vec<_>>()
                .join(", "),
        )
    }

    /// True when two runs can be considered comparable hardware-wise.
    ///
    /// The GPU list is part of the comparison: a run that used a different adapter (or lost one
    /// to a driver change) is not the same experiment.
    pub fn compatible_with(&self, other: &MachineInfo) -> bool {
        let gpu_names = |machine: &MachineInfo| -> Vec<String> {
            let mut names: Vec<String> = machine
                .gpus
                .iter()
                .map(|gpu| gpu.name.to_ascii_lowercase())
                .collect();
            names.sort();
            names
        };
        self.os == other.os
            && self.arch == other.arch
            && self.cpu_brand == other.cpu_brand
            && self.cpu_logical_cores == other.cpu_logical_cores
            && gpu_names(self) == gpu_names(other)
    }
}

/// Distribution of the measured benchmark samples.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Stats {
    pub count: usize,
    pub mean: f64,
    pub median: f64,
    pub min: f64,
    pub max: f64,
    /// Sample standard deviation (`n - 1`).
    pub stddev: f64,
    /// Coefficient of variation in percent (`stddev / mean * 100`).
    pub cv_percent: f64,
    /// 95th percentile (nearest rank).
    pub p95: f64,
}

impl Stats {
    /// Computes summary statistics for a set of samples (sorted internally).
    /// Returns `None` for an empty (or entirely non-finite) input.
    pub fn from_samples(samples: &[f64]) -> Option<Stats> {
        let mut sorted: Vec<f64> = samples.iter().copied().filter(|value| value.is_finite()).collect();
        if sorted.is_empty() {
            return None;
        }
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let count = sorted.len();
        let mean = sorted.iter().sum::<f64>() / count as f64;
        let median = if count % 2 == 1 {
            sorted[count / 2]
        } else {
            (sorted[count / 2 - 1] + sorted[count / 2]) / 2.0
        };
        let variance = if count > 1 {
            sorted.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / (count - 1) as f64
        } else {
            0.0
        };
        let stddev = variance.sqrt();
        let cv_percent = if mean.abs() > f64::EPSILON {
            stddev / mean * 100.0
        } else {
            0.0
        };
        let rank = ((count as f64 * 0.95).ceil() as usize).clamp(1, count);
        Some(Stats {
            count,
            mean,
            median,
            min: sorted[0],
            max: sorted[count - 1],
            stddev,
            cv_percent,
            p95: sorted[rank - 1],
        })
    }

    /// True when the sample spread is large enough to distrust small deltas.
    pub fn is_noisy(&self, threshold_percent: f64) -> bool {
        self.cv_percent > threshold_percent
    }
}

/// Outcome of running `command.test`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestSummary {
    pub command: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub duration_ms: f64,
    pub timed_out: bool,
}

/// Everything needed to re-derive a result later (plan §7: reproducibility).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReproInfo {
    /// Exact commit of the software state that was run.
    pub commit: String,
    /// Ref the timeline branched from, as written in the spec (`main`, `v0.2.0`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ref: Option<String>,
    /// Resolved commit of the base state.
    pub base_commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Worktree path the commands ran in, relative to the repository root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    /// Path of the experiment spec, relative to the repository root.
    pub spec_path: String,
    /// Verbatim spec text at run time (so later edits cannot rewrite history).
    pub spec_snapshot: String,
    /// Environment variables AION added on top of the inherited environment.
    pub environment: BTreeMap<String, String>,
    /// Whether commands ran with a whitelisted environment.
    pub clean_env: bool,
    /// Commands executed for this state.
    pub commands: CommandSpec,
    /// Order in which benchmark iterations were executed.
    pub run_order: Vec<String>,
}

/// Everything AION observed for one software state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    #[serde(alias = "ekbasis_version")]
    pub aion_version: String,
    pub experiment: String,
    pub kind: RunKind,
    /// Stable label used in the database and in `aion compare`, e.g. `bloom-test` or `bloom-test:baseline`.
    pub label: String,
    pub commit: String,
    pub base_commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub spec_path: String,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: f64,
    /// True when setup/build/test succeeded and every measured iteration succeeded.
    pub success: bool,
    pub iterations: u32,
    pub warmup: u32,
    pub environment: BTreeMap<String, String>,
    pub machine: MachineInfo,
    pub phases: Vec<PhaseReport>,
    pub samples: Vec<SampleRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<Stats>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<TestSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_ms: Option<f64>,
    pub repro: ReproInfo,
    /// Things the operator should know before trusting the numbers.
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl RunReport {
    /// The build phase, when a build command was configured.
    pub fn build_phase(&self) -> Option<&PhaseReport> {
        self.phases.iter().find(|phase| phase.kind == PhaseKind::Build)
    }

    /// Mean measured iteration duration in milliseconds.
    pub fn mean_duration_ms(&self) -> Option<f64> {
        self.stats.map(|stats| stats.mean)
    }

    /// Highest resident memory observed across all measured iterations.
    pub fn peak_memory_bytes(&self) -> Option<u64> {
        self.samples
            .iter()
            .filter_map(|sample| sample.observation.as_ref())
            .filter_map(|observation| observation.peak_memory_bytes)
            .max()
    }

    /// Mean CPU usage across measured iterations, in percent of one core.
    pub fn avg_cpu_percent(&self) -> Option<f64> {
        let values: Vec<f64> = self
            .samples
            .iter()
            .filter_map(|sample| sample.observation.as_ref())
            .filter_map(|observation| observation.avg_cpu_percent)
            .map(|value| value as f64)
            .collect();
        if values.is_empty() {
            return None;
        }
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }

    /// Highest GPU utilisation observed across the measured iterations, in percent.
    pub fn gpu_utilization_percent(&self) -> Option<f64> {
        self.samples
            .iter()
            .filter_map(|sample| sample.observation.as_ref())
            .filter_map(|observation| observation.gpu_utilization_percent)
            .map(|value| value as f64)
            .reduce(f64::max)
    }

    /// Highest GPU memory usage observed across the measured iterations, in bytes.
    pub fn gpu_memory_bytes(&self) -> Option<u64> {
        self.samples
            .iter()
            .filter_map(|sample| sample.observation.as_ref())
            .filter_map(|observation| observation.gpu_memory_bytes)
            .max()
    }

    /// Highest temperature observed across the measured iterations, in degrees Celsius.
    pub fn max_temperature_c(&self) -> Option<f64> {
        self.samples
            .iter()
            .filter_map(|sample| sample.observation.as_ref())
            .filter_map(|observation| observation.max_temperature_c)
            .map(|value| value as f64)
            .reduce(f64::max)
    }

    /// Durations of the successful measured iterations, in measurement order.
    pub fn successful_durations(&self) -> Vec<f64> {
        self.samples
            .iter()
            .filter(|sample| sample.success)
            .map(|sample| sample.duration_ms)
            .collect()
    }

    /// `(iteration, duration_ms)` pairs of the successful measured iterations.
    pub fn measured_samples(&self) -> Vec<(u32, f64)> {
        self.samples
            .iter()
            .filter(|sample| sample.success)
            .map(|sample| (sample.iteration, sample.duration_ms))
            .collect()
    }

    /// Iterations that failed or timed out, as `(iteration, reason)` pairs.
    pub fn failed_iterations(&self) -> Vec<(u32, String)> {
        self.samples
            .iter()
            .filter(|sample| !sample.success)
            .map(|sample| {
                let reason = if sample.timed_out {
                    "timed out".to_string()
                } else {
                    match sample.exit_code {
                        Some(code) => format!("exit code {code}"),
                        None => "terminated by signal".to_string(),
                    }
                };
                (sample.iteration, reason)
            })
            .collect()
    }

    /// One-line summary used by `aion timeline` and `aion experiment list`.
    pub fn summary_line(&self) -> String {
        match (self.stats, self.success) {
            (Some(stats), true) => format!("mean {}", crate::fmt::duration_ms(stats.mean)),
            (Some(stats), false) => {
                format!("mean {} (incomplete run)", crate::fmt::duration_ms(stats.mean))
            }
            (None, _) => "no samples".to_string(),
        }
    }
}
