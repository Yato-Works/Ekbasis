mod ci;
mod compare;
mod dashboard;
mod diff;
mod doctor;
mod experiment;
mod init;
mod report;
mod timeline;

use std::path::PathBuf;

use ekbasis_core::{AionConfig, AionPaths};
use ekbasis_git::Git;
use ekbasis_storage::Store;
use anyhow::{Context as _, Result};

use crate::{CiCommand, Cli, Command, ExperimentCommand};

/// Everything a command needs: paths, configuration, git and the database.
pub struct Context {
    pub paths: AionPaths,
    pub config: AionConfig,
    pub git: Git,
    pub store: Store,
    pub verbose: bool,
}

impl Context {
    /// Builds the context for the current working directory (requires an AION repository).
    pub fn load(verbose: bool) -> Result<Context> {
        let cwd = std::env::current_dir().context("cannot read the current directory")?;
        let mut git = Git::discover(&cwd)?;
        git.set_verbose(verbose);
        let paths = AionPaths::new(git.root().to_path_buf());
        paths.require_initialized()?;
        let config = AionConfig::load_or_default(&paths.config_path())?;
        let store = Store::open(&paths.db_path())?;
        Ok(Context {
            paths,
            config,
            git,
            store,
            verbose,
        })
    }

    /// Path of an experiment spec, verified to exist.
    pub fn spec_path(&self, name: &str) -> Result<PathBuf> {
        let path = self.paths.spec_path(name);
        if path.exists() {
            return Ok(path);
        }
        let known: Vec<String> = self
            .store
            .list_experiments()?
            .into_iter()
            .map(|row| row.name)
            .collect();
        if known.is_empty() {
            anyhow::bail!(
                "no experiment named `{name}`; create one with `ekbasis experiment create {name}`"
            );
        }
        anyhow::bail!(
            "no experiment named `{name}`; known experiments: {}",
            known.join(", ")
        );
    }

    /// Branch AION uses for an experiment's alternative timeline.
    pub fn experiment_branch(&self, name: &str) -> String {
        format!("{}{}", self.config.git.branch_prefix, name)
    }

    /// Worktree the experiment is measured in.
    pub fn experiment_worktree(&self, name: &str) -> PathBuf {
        self.paths
            .worktrees_dir(&self.config.git.worktrees_dir)
            .join(name)
    }

    /// Worktree the unmodified baseline is measured in.
    pub fn baseline_worktree(&self, name: &str) -> PathBuf {
        self.paths
            .worktrees_dir(&self.config.git.worktrees_dir)
            .join(format!("{name}.baseline"))
    }
}

/// Executes the parsed command line.
pub fn dispatch(cli: Cli) -> Result<()> {
    let verbose = cli.verbose;
    match cli.command {
        Command::Init(args) => init::run(&args, verbose),
        Command::Experiment { command } => match command {
            ExperimentCommand::Create(args) => experiment::create(&args, verbose),
            ExperimentCommand::List(args) => experiment::list(&args, verbose),
            ExperimentCommand::Show(args) => experiment::show(&args, verbose),
            ExperimentCommand::Run(args) => experiment::run(&args, verbose),
            ExperimentCommand::Diff(args) => diff::run(&args, verbose),
            ExperimentCommand::Sweep(args) => experiment::sweep(&args, verbose),
        },
        Command::Compare(args) => compare::run(&args, verbose),
        Command::Report(args) => report::run(&args, verbose),
        Command::Timeline(args) => timeline::run(&args, verbose),
        Command::Dashboard(args) => dashboard::run(&args, verbose),
        Command::Sweep(args) => experiment::sweep(&args, verbose),
        Command::Ci { command } => match command {
            CiCommand::Init(args) => ci::init(&args, verbose),
        },
        Command::Doctor => doctor::run(verbose),
    }
}

/// Resolves the regression threshold: CLI flag, then spec value, then config default.
pub fn resolve_threshold(config: &AionConfig, requested: Option<f64>, from_spec: Option<f64>) -> f64 {
    requested
        .or(from_spec)
        .unwrap_or(config.defaults.regression_threshold_percent)
}
