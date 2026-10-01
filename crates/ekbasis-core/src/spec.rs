//! Experiment definitions: the declarative description of an alternative timeline.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

/// A complete AION experiment definition (`.aion/experiments/<name>.yaml`).
///
/// ```yaml
/// experiment:
///   name: bloom-startup-test
///   base: main
///   hypothesis: "preload off should cut startup time"
///
/// changes:
///   - type: config
///     file: config.toml
///     key: startup.preload
///     from: true
///     to: false
///
/// environment:
///   THREADS: "8"
///
/// command:
///   build: cargo build --release
///   run: ./target/release/bloom
///
/// benchmark:
///   iterations: 5
///   warmup: 1
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentSpec {
    pub experiment: ExperimentMeta,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matrix: Option<MatrixSpec>,
    #[serde(default)]
    pub changes: Vec<Change>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub command: CommandSpec,
    #[serde(default)]
    pub benchmark: BenchmarkSpec,
    #[serde(default)]
    pub limits: LimitsSpec,
}

/// Identity of an experiment: what it is called and where it branches from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperimentMeta {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Ref or commit the alternative timeline starts from (defaults to the current branch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Free-form labels shown by `aion experiment list`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// What the experiment is *expected* to show. Recorded for the log, never used to
    /// predict or justify a result — AION reports only what it measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hypothesis: Option<String>,
}

/// A single modification applied to the experiment worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Change {
    /// Modify one key inside a structured config file (TOML, JSON, YAML or `.env`).
    Config {
        /// Path of the file, relative to the repository root.
        file: String,
        /// Dotted key path, e.g. `startup.preload` or `cache.levels.0`.
        key: String,
        /// Optional guard: fail unless the value currently equals this.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<serde_yaml_ng::Value>,
        /// New value.
        to: serde_yaml_ng::Value,
    },
    /// Replace a literal text fragment inside a file (source-level experiments).
    Replace { file: String, from: String, to: String },
    /// Write (or overwrite) a whole file.
    File { path: String, content: String },
}

impl Change {
    /// The path this change touches.
    pub fn path(&self) -> &str {
        match self {
            Change::Config { file, .. } | Change::Replace { file, .. } => file,
            Change::File { path, .. } => path,
        }
    }

    /// Short type label used in CLI output (`config`, `replace`, `file`).
    pub fn kind(&self) -> &'static str {
        match self {
            Change::Config { .. } => "config",
            Change::Replace { .. } => "replace",
            Change::File { .. } => "file",
        }
    }

    /// Human readable one-liner, e.g. `config config.toml: startup.preload true -> false`.
    pub fn describe(&self) -> String {
        match self {
            Change::Config { file, key, to, .. } => {
                format!("config {file}: {key} -> {}", crate::changes::describe_yaml(to))
            }
            Change::Replace { file, from, to } => format!("replace {file}: {from:?} -> {to:?}"),
            Change::File { path, content } => {
                format!("file {path}: write {} bytes", content.len())
            }
        }
    }
}

/// Commands that make up the pipeline for one software state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    /// Executed once per worktree before building (e.g. `npm ci`, `cargo fetch`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test: Option<String>,
    /// The command whose wall-clock time is benchmarked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<String>,
}

impl CommandSpec {
    /// Non-empty commands as `(phase, command)` pairs, in pipeline order.
    pub fn phases(&self) -> Vec<(&'static str, &str)> {
        let mut phases = Vec::new();
        if let Some(setup) = &self.setup {
            phases.push(("setup", setup.as_str()));
        }
        if let Some(build) = &self.build {
            phases.push(("build", build.as_str()));
        }
        if let Some(test) = &self.test {
            phases.push(("test", test.as_str()));
        }
        if let Some(run) = &self.run {
            phases.push(("run", run.as_str()));
        }
        phases
    }

    /// True when no command is configured at all.
    pub fn is_empty(&self) -> bool {
        self.setup.is_none() && self.build.is_none() && self.test.is_none() && self.run.is_none()
    }
}

/// How often the measured command is executed and when a delta counts as real.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchmarkSpec {
    /// Measured iterations.
    #[serde(default = "default_iterations")]
    pub iterations: u32,
    /// Discarded iterations executed first (page cache / JIT warm-up).
    #[serde(default)]
    pub warmup: u32,
    /// Deltas below this percentage are reported as `neutral`.
    #[serde(default = "default_threshold_percent")]
    pub regression_threshold_percent: f64,
}

fn default_iterations() -> u32 {
    3
}

fn default_threshold_percent() -> f64 {
    3.0
}

impl Default for BenchmarkSpec {
    fn default() -> Self {
        BenchmarkSpec {
            iterations: default_iterations(),
            warmup: 0,
            regression_threshold_percent: default_threshold_percent(),
        }
    }
}

/// Safety limits applied to every executed command (plan §15).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsSpec {
    /// Wall-clock limit per command, in seconds. Timed-out commands are killed.
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// Per-stream output kept in the JSON report, in KiB.
    #[serde(default = "default_max_output_kb")]
    pub max_output_kb: usize,
    /// When true, commands run with a whitelisted environment instead of inheriting everything.
    #[serde(default)]
    pub clean_env: bool,
    /// Maximum number of processes the command tree may spawn. When the count is exceeded the
    /// tree is killed and the run is marked failed. Unset means unlimited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_processes: Option<u32>,
    /// Network policy for the measured commands. `block` refuses to run when the platform
    /// cannot enforce it — AION never reports an isolation it did not apply (plan §15).
    #[serde(default)]
    pub network: NetworkPolicy,
    /// Container isolation: runs commands inside a Docker or Podman container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<ContainerSpec>,
}

/// Container isolation configuration for sandboxed benchmark execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContainerSpec {
    /// Container image name, e.g. `rust:1.85-slim`, `python:3.12-slim`, `ubuntu:24.04`.
    pub image: String,
    /// Container engine (`docker`, `podman`, or auto-detected if omitted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// Additional volume mounts (`host_dir:container_dir[:ro]`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<String>,
}

/// Whether measured commands may reach the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPolicy {
    /// Normal connectivity (the default).
    #[default]
    Allow,
    /// No network access: enforced through a network namespace where the platform allows it,
    /// a hard error where it does not.
    Block,
}

impl NetworkPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            NetworkPolicy::Allow => "allow",
            NetworkPolicy::Block => "block",
        }
    }
}

fn default_timeout_secs() -> u64 {
    600
}

fn default_max_output_kb() -> usize {
    256
}

impl Default for LimitsSpec {
    fn default() -> Self {
        LimitsSpec {
            timeout_secs: default_timeout_secs(),
            max_output_kb: default_max_output_kb(),
            clean_env: false,
            max_processes: None,
            network: NetworkPolicy::Allow,
            container: None,
        }
    }
}

impl ExperimentSpec {
    /// Parses a spec from YAML text.
    pub fn from_yaml(text: &str) -> Result<ExperimentSpec> {
        serde_yaml_ng::from_str(text).context("cannot parse experiment spec")
    }

    /// Loads and validates a spec from disk.
    pub fn load(path: &Path) -> Result<ExperimentSpec> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read experiment spec `{}`", path.display()))?;
        let spec: ExperimentSpec = serde_yaml_ng::from_str(&text)
            .with_context(|| format!("cannot parse experiment spec `{}`", path.display()))?;
        spec.validate()
            .with_context(|| format!("invalid experiment spec `{}`", path.display()))?;
        Ok(spec)
    }

    /// Verbatim spec text (used for the reproducibility snapshot).
    pub fn read_raw(path: &Path) -> Result<String> {
        std::fs::read_to_string(path)
            .with_context(|| format!("cannot read experiment spec `{}`", path.display()))
    }

    /// Serializes the spec back to YAML.
    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml_ng::to_string(self).context("cannot serialize experiment spec")
    }

    /// Writes the spec to `path`, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create `{}`", parent.display()))?;
        }
        std::fs::write(path, self.to_yaml()?)
            .with_context(|| format!("cannot write experiment spec `{}`", path.display()))
    }

    /// Structural validation: catches typos before any worktree is created.
    pub fn validate(&self) -> Result<()> {
        let name = self.experiment.name.trim();
        if name.is_empty() {
            bail!("experiment.name must not be empty");
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            bail!("experiment.name `{name}` may only contain letters, digits, '.', '-' and '_'");
        }
        if name.starts_with('.') || name.ends_with('.') {
            bail!("experiment.name `{name}` must not start or end with '.'");
        }
        if self.command.is_empty() {
            bail!("at least one command (setup/build/test/run) must be configured");
        }
        if self.command.run.is_none() && self.command.test.is_none() {
            bail!(
                "`command.run` is required: it is the command whose wall-clock time AION benchmarks"
            );
        }
        if self.benchmark.iterations == 0 {
            bail!("benchmark.iterations must be at least 1");
        }
        if self.benchmark.iterations > 1000 {
            bail!("benchmark.iterations must be at most 1000");
        }
        if self.benchmark.warmup > 100 {
            bail!("benchmark.warmup must be at most 100");
        }
        if !(0.0..=100.0).contains(&self.benchmark.regression_threshold_percent) {
            bail!("benchmark.regression_threshold_percent must be between 0 and 100");
        }
        if self.limits.timeout_secs == 0 {
            bail!("limits.timeout_secs must be at least 1");
        }
        if let Some(0) = self.limits.max_processes {
            bail!("limits.max_processes must be at least 1 (or omitted for no limit)");
        }
        for change in &self.changes {
            let path = change.path();
            if path.trim().is_empty() {
                bail!("every change needs a non-empty file path");
            }
            let candidate = Path::new(path);
            if candidate.is_absolute() {
                bail!("change path `{path}` must be relative to the repository root");
            }
            if candidate
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                bail!("change path `{path}` must not contain `..`");
            }
            match change {
                Change::Config { key, .. } if key.trim().is_empty() => {
                    bail!("config change in `{path}` needs a `key`");
                }
                Change::Replace { from, .. } if from.is_empty() => {
                    bail!("replace change in `{path}` needs a non-empty `from`");
                }
                _ => {}
            }
        }
        if let Some(matrix) = &self.matrix {
            matrix.validate()?;
        }
        if let Some(container) = &self.limits.container {
            if container.image.trim().is_empty() {
                bail!("limits.container.image must not be empty");
            }
        }
        for key in self.environment.keys() {
            if key.trim().is_empty() {
                bail!("environment variable names must not be empty");
            }
            if key.contains('=') {
                bail!("environment variable name `{key}` must not contain '='");
            }
        }
        Ok(())
    }

    /// Commented YAML template written by `aion experiment create`.
    ///
    /// The template is deliberately chatty: an experiment definition is the *only* place where
    /// the meaning of an alternative timeline is written down.
    pub fn template(
        name: &str,
        base: Option<&str>,
        iterations: u32,
        warmup: u32,
        timeout_secs: u64,
    ) -> String {
        let base = base.unwrap_or("<branch-or-commit>");
        format!(
            r#"# AION experiment definition, created by `aion experiment create`.
#
# AION does not guess what could have happened: it creates a branch from `base`,
# applies the changes below, then builds, runs and measures the result and compares
# it against the same measurements taken on an unmodified checkout of `base`.
#
#   aion experiment show {name}     # what this timeline would change
#   aion experiment run  {name}     # actually run and compare it

experiment:
  name: {name}
  base: {base}
  description: "what this alternative timeline is about"
  # hypothesis: "expected direction of the change - recorded, never used to predict"
  tags: []

# ---------------------------------------------------------------------------
# changes: what makes this future different from the base commit
# ---------------------------------------------------------------------------
changes:
  # Modify one key inside a structured config file (TOML / JSON / YAML / .env):
  - type: config
    file: config.toml
    key: startup.preload
    from: true          # optional guard: fail if the current value differs
    to: false

  # Replace literal text (source-level experiments):
  # - type: replace
  #   file: src/main.rs
  #   from: "CACHE_SIZE = 1024"
  #   to: "CACHE_SIZE = 4096"

  # Write a whole file:
  # - type: file
  #   path: config/alt.yaml
  #   content: "mode: aggressive"

environment:
  # Variables added on top of the inherited environment:
  THREADS: "8"

command:
  # setup: optional, executed once per worktree before the build
  build: cargo build --release
  test: cargo test --release
  run: ./target/release/app

benchmark:
  iterations: {iterations}
  warmup: {warmup}
  regression_threshold_percent: 3.0

limits:
  timeout_secs: {timeout_secs}
  max_output_kb: 256
  clean_env: false
  # Plan §15 guard rails (both optional):
  # max_processes: 64      # kill the command tree when it spawns more processes than this
  # network: block          # no network for measured commands (hard error where unsupported)
"#
        )
    }

    /// Expands the experiment matrix into concrete variants.
    ///
    /// If no matrix is defined or it is empty, returns a single variant with an empty parameter map.
    pub fn expand_matrix(&self) -> Result<Vec<(BTreeMap<String, String>, ExperimentSpec)>> {
        let matrix = match &self.matrix {
            Some(m) if !m.is_empty() => m,
            _ => return Ok(vec![(BTreeMap::new(), self.clone())]),
        };

        let combinations = matrix.combinations();
        let mut variants = Vec::with_capacity(combinations.len());

        for combo in combinations {
            let mut variant = self.clone();
            variant.matrix = None;

            // Generate a safe unique name for the variant
            let mut suffix_parts = Vec::new();
            for (k, v) in &combo {
                let clean_k: String = k
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                let clean_v: String = v
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '-' })
                    .collect();
                suffix_parts.push(format!("{clean_k}_{clean_v}"));
            }
            let suffix = suffix_parts.join("-");
            variant.experiment.name = format!("{}-{suffix}", self.experiment.name);

            if !variant.experiment.tags.contains(&"matrix".to_string()) {
                variant.experiment.tags.push("matrix".to_string());
            }
            if !variant.experiment.tags.contains(&"sweep".to_string()) {
                variant.experiment.tags.push("sweep".to_string());
            }
            for (k, v) in &combo {
                variant.experiment.tags.push(format!("{k}={v}"));
            }

            if let Some(desc) = &self.experiment.description {
                variant.experiment.description = Some(substitute_str(desc, &combo));
            }
            if let Some(hypo) = &self.experiment.hypothesis {
                variant.experiment.hypothesis = Some(substitute_str(hypo, &combo));
            }

            variant.changes = self
                .changes
                .iter()
                .map(|change| match change {
                    Change::Config {
                        file,
                        key,
                        from,
                        to,
                    } => Change::Config {
                        file: substitute_str(file, &combo),
                        key: substitute_str(key, &combo),
                        from: from.as_ref().map(|v| substitute_yaml_value(v, &combo)),
                        to: substitute_yaml_value(to, &combo),
                    },
                    Change::Replace { file, from, to } => Change::Replace {
                        file: substitute_str(file, &combo),
                        from: substitute_str(from, &combo),
                        to: substitute_str(to, &combo),
                    },
                    Change::File { path, content } => Change::File {
                        path: substitute_str(path, &combo),
                        content: substitute_str(content, &combo),
                    },
                })
                .collect();

            variant.environment = self
                .environment
                .iter()
                .map(|(k, v)| (substitute_str(k, &combo), substitute_str(v, &combo)))
                .collect();

            variant.command = CommandSpec {
                setup: self.command.setup.as_ref().map(|s| substitute_str(s, &combo)),
                build: self.command.build.as_ref().map(|s| substitute_str(s, &combo)),
                test: self.command.test.as_ref().map(|s| substitute_str(s, &combo)),
                run: self.command.run.as_ref().map(|s| substitute_str(s, &combo)),
            };

            variant.validate().with_context(|| {
                format!("validating generated matrix variant `{}`", variant.experiment.name)
            })?;

            variants.push((combo, variant));
        }

        Ok(variants)
    }

    /// Commented YAML template for a matrix sweep experiment.
    pub fn template_matrix(name: &str, base: Option<&str>) -> String {
        let base = base.unwrap_or("<branch-or-commit>");
        format!(
            r#"# AION matrix sweep experiment definition.
#
# AION will expand the `matrix.params` into Cartesian product variants,
# measure the baseline commit once, then run and benchmark each variant.

experiment:
  name: {name}
  base: {base}
  description: "parameter sweep experiment"
  tags: [sweep]

matrix:
  params:
    threads: [2, 4, 8]
    mode: ["normal", "aggressive"]

changes:
  - type: replace
    file: src/main.rs
    from: "THREADS = 1"
    to: "THREADS = ${{{{ matrix.threads }}}}"

environment:
  RUN_MODE: "${{{{ matrix.mode }}}}"
  RAYON_NUM_THREADS: "${{{{ matrix.threads }}}}"

command:
  build: cargo build --release
  test: cargo test --release
  run: ./target/release/app

benchmark:
  iterations: 3
  warmup: 1
  regression_threshold_percent: 3.0

limits:
  timeout_secs: 300
"#
        )
    }
}

/// Parameter matrix definition for multi-variant sweep experiments.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MatrixSpec {
    /// Parameter names mapped to lists of values.
    /// Example: `threads: [1, 2, 4, 8]`
    #[serde(default)]
    pub params: BTreeMap<String, Vec<serde_yaml_ng::Value>>,
}

impl MatrixSpec {
    pub fn is_empty(&self) -> bool {
        self.params.is_empty() || self.params.values().all(|v| v.is_empty())
    }

    pub fn validate(&self) -> Result<()> {
        let mut total_combinations: usize = 1;
        for (key, values) in &self.params {
            let key = key.trim();
            if key.is_empty() {
                bail!("matrix parameter name must not be empty");
            }
            if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                bail!("matrix parameter `{key}` may only contain alphanumeric characters and '_'");
            }
            if values.is_empty() {
                bail!("matrix parameter `{key}` must have at least one value");
            }
            total_combinations = total_combinations.saturating_mul(values.len());
        }
        if total_combinations > 256 {
            bail!("matrix produces {total_combinations} combinations; maximum allowed is 256");
        }
        Ok(())
    }

    /// Computes the Cartesian product of all parameters.
    pub fn combinations(&self) -> Vec<BTreeMap<String, String>> {
        if self.is_empty() {
            return Vec::new();
        }
        let mut result = vec![BTreeMap::new()];
        for (key, values) in &self.params {
            let mut next = Vec::with_capacity(result.len() * values.len());
            for combo in &result {
                for val in values {
                    let mut new_combo = combo.clone();
                    let val_str = match val {
                        serde_yaml_ng::Value::String(s) => s.clone(),
                        serde_yaml_ng::Value::Number(n) => n.to_string(),
                        serde_yaml_ng::Value::Bool(b) => b.to_string(),
                        _ => serde_json::to_string(val).unwrap_or_else(|_| "".into()),
                    };
                    new_combo.insert(key.clone(), val_str);
                    next.push(new_combo);
                }
            }
            result = next;
        }
        result
    }
}

fn substitute_str(text: &str, params: &BTreeMap<String, String>) -> String {
    let mut out = text.to_string();
    for (k, v) in params {
        out = out.replace(&format!("${{{{ matrix.{k} }}}}"), v);
        out = out.replace(&format!("${{{{matrix.{k}}}}}"), v);
        out = out.replace(&format!("${{matrix.{k}}}"), v);
        out = out.replace(&format!("${{ matrix.{k} }}"), v);
        out = out.replace(&format!("{{{{ matrix.{k} }}}}"), v);
        out = out.replace(&format!("{{{{matrix.{k}}}}}"), v);
    }
    out
}

fn substitute_yaml_value(
    val: &serde_yaml_ng::Value,
    params: &BTreeMap<String, String>,
) -> serde_yaml_ng::Value {
    match val {
        serde_yaml_ng::Value::String(s) => {
            let substituted = substitute_str(s, params);
            if substituted != *s {
                if let Ok(parsed) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&substituted) {
                    parsed
                } else {
                    serde_yaml_ng::Value::String(substituted)
                }
            } else {
                serde_yaml_ng::Value::String(substituted)
            }
        }
        serde_yaml_ng::Value::Sequence(seq) => serde_yaml_ng::Value::Sequence(
            seq.iter()
                .map(|item| substitute_yaml_value(item, params))
                .collect(),
        ),
        serde_yaml_ng::Value::Mapping(map) => {
            let mut new_map = serde_yaml_ng::Mapping::new();
            for (k, v) in map {
                new_map.insert(k.clone(), substitute_yaml_value(v, params));
            }
            serde_yaml_ng::Value::Mapping(new_map)
        }
        other => other.clone(),
    }
}
