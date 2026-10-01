//! Repository-local Ekbasis configuration (`.ekbasis/config.yaml` or legacy `.aion/config.yaml`).

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{EKBASIS_VERSION, now_rfc3339};

/// Content of `.ekbasis/config.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EkbasisConfig {
    /// Identity of this installation (supports both `ekbasis` and legacy `aion` keys).
    #[serde(alias = "aion")]
    pub ekbasis: EkbasisMeta,
    /// Defaults used when neither the CLI nor the experiment spec overrides them.
    pub defaults: DefaultsConfig,
    /// Git layout used for alternative timelines.
    pub git: GitConfig,
    /// Runtime observer settings.
    pub observer: ObserverConfig,
}

pub type AionConfig = EkbasisConfig;

/// Identity block of configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EkbasisMeta {
    /// Version that created (or last wrote) this directory.
    pub version: String,
    pub created_at: String,
}

pub type AionMeta = EkbasisMeta;

/// Defaults applied by the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DefaultsConfig {
    /// Ref used as the baseline when the CLI and the spec both stay silent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_ref: Option<String>,
    pub iterations: u32,
    pub warmup: u32,
    pub timeout_secs: u64,
    pub regression_threshold_percent: f64,
}

/// Git layout of alternative timelines.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GitConfig {
    /// Branch prefix, e.g. `ekbasis/` produces `ekbasis/bloom-startup-test`.
    pub branch_prefix: String,
    /// Worktree root, relative to the repository root.
    pub worktrees_dir: String,
    /// Keep worktrees after a run instead of deleting them.
    pub keep_worktrees: bool,
}

/// Runtime observer settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ObserverConfig {
    pub enabled: bool,
    pub sample_interval_ms: u64,
}

impl Default for EkbasisMeta {
    fn default() -> Self {
        EkbasisMeta {
            version: EKBASIS_VERSION.to_string(),
            created_at: now_rfc3339(),
        }
    }
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        DefaultsConfig {
            baseline_ref: None,
            iterations: 3,
            warmup: 1,
            timeout_secs: 600,
            regression_threshold_percent: 3.0,
        }
    }
}

impl Default for GitConfig {
    fn default() -> Self {
        GitConfig {
            branch_prefix: crate::DEFAULT_BRANCH_PREFIX.to_string(),
            worktrees_dir: format!("{}/worktrees", crate::EKBASIS_DIR_NAME),
            keep_worktrees: false,
        }
    }
}

impl Default for ObserverConfig {
    fn default() -> Self {
        ObserverConfig {
            enabled: true,
            sample_interval_ms: 250,
        }
    }
}

impl EkbasisConfig {
    /// Loads `.ekbasis/config.yaml` or `.aion/config.yaml`.
    pub fn load(path: &Path) -> Result<EkbasisConfig> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read Ekbasis config `{}`", path.display()))?;
        let config: EkbasisConfig = serde_yaml_ng::from_str(&text)
            .with_context(|| format!("cannot parse Ekbasis config `{}`", path.display()))?;
        Ok(config)
    }

    /// Loads the config when it exists, otherwise falls back to defaults.
    /// Used by read-only commands that should still work in a partially set up repository.
    pub fn load_or_default(path: &Path) -> Result<EkbasisConfig> {
        if path.exists() {
            EkbasisConfig::load(path)
        } else {
            Ok(EkbasisConfig::default())
        }
    }

    /// Writes the config, prefixed with a short explanation for human editors.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create `{}`", parent.display()))?;
        }
        let body = serde_yaml_ng::to_string(self)
            .context("cannot serialize Ekbasis config")?;
        let header = "# Ekbasis configuration, created by `ekbasis init`.\n\
                      # Per-experiment values go into .ekbasis/experiments/<name>.yaml.\n";
        std::fs::write(path, format!("{header}{body}"))
            .with_context(|| format!("cannot write `{}`", path.display()))?;
        Ok(())
    }
}
