//! Filesystem layout of the `.ekbasis/` (or legacy `.aion/`) state directory.
//!
//! ```text
//! .ekbasis/
//! ├── config.yaml
//! ├── experiments/exp-001.yaml
//! ├── results/exp-001.experiment.json
//! ├── worktrees/exp-001/            (temporary checkouts)
//! └── timeline.db
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::{AION_DIR_NAME, EKBASIS_DIR_NAME, RunKind};

/// Sub-directory holding experiment definitions.
pub const EXPERIMENTS_DIR: &str = "experiments";
/// Sub-directory holding JSON reports.
pub const RESULTS_DIR: &str = "results";
/// Sub-directory holding temporary git worktrees.
pub const WORKTREES_DIR: &str = "worktrees";
/// Name of the configuration file inside state directory.
pub const CONFIG_FILE: &str = "config.yaml";
/// Name of the SQLite experiment database.
pub const DB_FILE: &str = "timeline.db";

/// All paths Ekbasis uses inside one repository.
#[derive(Debug, Clone)]
pub struct EkbasisPaths {
    repo_root: PathBuf,
    dir_name: String,
}

/// Backwards compatibility alias for EkbasisPaths.
pub type AionPaths = EkbasisPaths;

impl EkbasisPaths {
    /// Builds the layout for a repository root.
    /// Prefers `.ekbasis/` if it exists, otherwise `.aion/` if it exists, otherwise defaults to `.ekbasis/`.
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        let repo_root = repo_root.into();
        let dir_name = if repo_root.join(EKBASIS_DIR_NAME).is_dir() {
            EKBASIS_DIR_NAME.to_string()
        } else if repo_root.join(AION_DIR_NAME).is_dir() {
            AION_DIR_NAME.to_string()
        } else {
            EKBASIS_DIR_NAME.to_string()
        };
        EkbasisPaths {
            repo_root,
            dir_name,
        }
    }

    /// Explicitly creates paths with a chosen state dir name (e.g. `.ekbasis` or `.aion`).
    pub fn with_dir(repo_root: impl Into<PathBuf>, dir_name: impl Into<String>) -> Self {
        EkbasisPaths {
            repo_root: repo_root.into(),
            dir_name: dir_name.into(),
        }
    }

    /// Walks upwards from `start` looking for an existing `.ekbasis/` or `.aion/` directory.
    pub fn discover(start: &Path) -> Result<EkbasisPaths> {
        let start = if start.as_os_str().is_empty() {
            Path::new(".")
        } else {
            start
        };
        let absolute = std::fs::canonicalize(start)
            .with_context(|| format!("cannot resolve `{}`", start.display()))?;
        let mut current: Option<&Path> = Some(absolute.as_path());
        while let Some(dir) = current {
            if dir.join(EKBASIS_DIR_NAME).is_dir() {
                return Ok(EkbasisPaths::with_dir(dir.to_path_buf(), EKBASIS_DIR_NAME));
            }
            if dir.join(AION_DIR_NAME).is_dir() {
                return Ok(EkbasisPaths::with_dir(dir.to_path_buf(), AION_DIR_NAME));
            }
            current = dir.parent();
        }
        bail!(
            "no Ekbasis repository found in `{}` or any parent directory (run `ekbasis init` first)",
            absolute.display()
        )
    }

    /// Repository root that owns the state directory.
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// State directory (`.ekbasis/` or `.aion/`).
    pub fn state_dir(&self) -> PathBuf {
        self.repo_root.join(&self.dir_name)
    }

    /// `.ekbasis/` (or current state directory)
    pub fn ekbasis_dir(&self) -> PathBuf {
        self.state_dir()
    }

    /// Legacy alias for `.ekbasis/` / `.aion/`
    pub fn aion_dir(&self) -> PathBuf {
        self.state_dir()
    }

    /// Name of state dir (`.ekbasis` or `.aion`).
    pub fn dir_name(&self) -> &str {
        &self.dir_name
    }

    /// Config path: `<state_dir>/config.yaml`
    pub fn config_path(&self) -> PathBuf {
        self.state_dir().join(CONFIG_FILE)
    }

    /// Experiments dir: `<state_dir>/experiments/`
    pub fn experiments_dir(&self) -> PathBuf {
        self.state_dir().join(EXPERIMENTS_DIR)
    }

    /// Results dir: `<state_dir>/results/`
    pub fn results_dir(&self) -> PathBuf {
        self.state_dir().join(RESULTS_DIR)
    }

    /// Default worktrees dir: `<state_dir>/worktrees/`
    pub fn default_worktrees_dir(&self) -> PathBuf {
        self.state_dir().join(WORKTREES_DIR)
    }

    /// Worktrees directory, honouring the (possibly relative) configured path.
    pub fn worktrees_dir(&self, configured: &str) -> PathBuf {
        let configured = Path::new(configured);
        if configured.is_absolute() {
            configured.to_path_buf()
        } else {
            self.repo_root.join(configured)
        }
    }

    /// Database path: `<state_dir>/timeline.db`
    pub fn db_path(&self) -> PathBuf {
        self.state_dir().join(DB_FILE)
    }

    /// Experiment YAML path
    pub fn spec_path(&self, name: &str) -> PathBuf {
        self.experiments_dir().join(format!("{name}.yaml"))
    }

    /// Result report path
    pub fn report_path(&self, name: &str, kind: RunKind) -> PathBuf {
        self.results_dir().join(format!("{name}.{kind}.json"))
    }

    /// Comparison result path
    pub fn comparison_path(&self, name: &str) -> PathBuf {
        self.results_dir().join(format!("{name}.comparison.json"))
    }

    /// Markdown report path
    pub fn report_markdown_path(&self, name: &str) -> PathBuf {
        self.results_dir().join(format!("{name}.report.md"))
    }

    /// True when state directory exists.
    pub fn is_initialized(&self) -> bool {
        self.state_dir().is_dir()
    }

    /// Fails with a friendly message when not initialized.
    pub fn require_initialized(&self) -> Result<()> {
        if self.is_initialized() {
            Ok(())
        } else {
            bail!(
                "`{}` is not an Ekbasis repository yet - run `ekbasis init` to create .ekbasis/",
                self.repo_root.display()
            )
        }
    }

    /// Ensures directory layout exists.
    pub fn ensure_layout(&self) -> Result<()> {
        for dir in [
            self.state_dir(),
            self.experiments_dir(),
            self.results_dir(),
            self.default_worktrees_dir(),
        ] {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("cannot create `{}`", dir.display()))?;
        }
        Ok(())
    }

    /// Path relative to the repository root, using `/` separators (for reports and CLI output).
    pub fn rel(&self, path: &Path) -> String {
        let relative = path.strip_prefix(&self.repo_root).unwrap_or(path);
        relative.to_string_lossy().replace('\\', "/")
    }
}
