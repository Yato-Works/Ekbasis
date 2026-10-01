//! Ekbasis command line interface.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

pub mod commands;
pub mod dashboard;
pub mod futures;
pub mod output;
pub mod pipeline;
pub mod report;

/// Ekbasis — Software Timeline & Counterfactual Experiment Engine.
#[derive(Debug, Parser)]
#[command(
    name = "ekbasis",
    version,
    about = "Give software alternate futures: branch, change, build, measure, compare.",
    long_about = "Ekbasis turns “what if this change had been made?” into an experiment. It creates a \
branch from a base commit, applies the changes you declare, builds and runs the result, then \
compares the measurements against the same measurements taken on the unmodified base commit.\n\n\
Ekbasis never predicts: it only reports what it executed and observed.",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Run as if Ekbasis had been started in <DIR>.
    #[arg(short = 'C', long = "dir", global = true, value_name = "DIR")]
    pub dir: Option<PathBuf>,

    /// Print every git and shell command Ekbasis executes.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// Top level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Install Ekbasis into the current git repository (creates `.ekbasis/`).
    Init(InitArgs),
    /// Define, inspect and run experiments.
    #[command(alias = "exp")]
    Experiment {
        #[command(subcommand)]
        command: ExperimentCommand,
    },
    /// Compare two measured runs (run ids, experiment names or report files).
    Compare(CompareArgs),
    /// Write a Markdown report (CI artifact, pull request comment, step summary).
    Report(ReportArgs),
    /// Show the Ekbasis timeline of this repository.
    Timeline(TimelineArgs),
    /// Write the interactive alternative-timeline dashboard as a self-contained HTML page.
    Dashboard(DashboardArgs),
    /// Run a parameter sweep matrix experiment (alias for `experiment sweep`).
    Sweep(RunArgs),
    /// CI/CD integration and GitHub Actions workflow management.
    Ci {
        #[command(subcommand)]
        command: CiCommand,
    },
    /// Report the environment Ekbasis is running in.
    Doctor,
}

/// `ekbasis init`
#[derive(Debug, Args)]
pub struct InitArgs {
    /// Rewrite an existing configuration.
    #[arg(long)]
    pub force: bool,
    /// Do not touch `.gitignore`.
    #[arg(long)]
    pub no_gitignore: bool,
    /// Baseline ref stored in the config (used when a spec has no `base`).
    #[arg(long, value_name = "REF")]
    pub baseline_ref: Option<String>,
    /// Also install GitHub Actions benchmark workflow (`.github/workflows/ekbasis.yml`).
    #[arg(long)]
    pub ci: bool,
}

/// `ekbasis ci ...`
#[derive(Debug, Subcommand)]
pub enum CiCommand {
    /// Install GitHub Actions benchmark workflow (`.github/workflows/ekbasis.yml`).
    Init(CiInitArgs),
}

/// `ekbasis ci init`
#[derive(Debug, Args)]
pub struct CiInitArgs {
    /// Fail the CI job if a performance regression is detected.
    #[arg(long)]
    pub fail_on_regression: bool,
    /// Space-separated experiment names to benchmark, or "all".
    #[arg(long, default_value = "all")]
    pub experiments: Option<String>,
    /// Repository to pull the Ekbasis toolchain from.
    #[arg(long, default_value = "ekbasis/ekbasis")]
    pub repo: Option<String>,
    /// Overwrite an existing `.github/workflows/ekbasis.yml`.
    #[arg(long)]
    pub force: bool,
}

/// `ekbasis experiment ...`
#[derive(Debug, Subcommand)]
pub enum ExperimentCommand {
    /// Create a new experiment definition from a commented template.
    Create(CreateArgs),
    /// List experiments and their latest results.
    List(ListArgs),
    /// Show the definition and recorded runs of one experiment.
    Show(ShowArgs),
    /// Run an experiment: branch, change, build, measure, compare.
    Run(RunArgs),
    /// Show what changed between two experiments (spec and measurements).
    Diff(DiffArgs),
    /// Run a parameter sweep matrix experiment.
    Sweep(RunArgs),
}

/// `ekbasis experiment create`
#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Experiment name (letters, digits, '.', '-' and '_').
    pub name: String,
    /// Base ref or commit the alternative timeline branches from.
    #[arg(long, value_name = "REF")]
    pub base: Option<String>,
    /// Copy an existing spec file instead of writing the template.
    #[arg(long = "from", value_name = "FILE")]
    pub from: Option<PathBuf>,
    /// Generate a matrix sweep template.
    #[arg(long)]
    pub matrix: bool,
    /// Overwrite an existing spec.
    #[arg(long)]
    pub force: bool,
    /// One-line description stored with the experiment.
    #[arg(long, value_name = "TEXT")]
    pub description: Option<String>,
    /// Expected direction of the change (recorded, never used to predict).
    #[arg(long, value_name = "TEXT")]
    pub hypothesis: Option<String>,
}

/// `ekbasis experiment list`
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Also list every recorded run of every experiment.
    #[arg(long)]
    pub all: bool,
}

/// `ekbasis experiment show`
#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Experiment name.
    pub name: String,
    /// Print the stored JSON reports instead of a summary.
    #[arg(long)]
    pub reports: bool,
}

/// `ekbasis experiment run`
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Experiment name (omit when using `--all`).
    pub name: Option<String>,
    /// Run every experiment in experiments directory (used by CI).
    #[arg(long)]
    pub all: bool,
    /// Base ref or commit (overrides `experiment.base`).
    #[arg(long, value_name = "REF")]
    pub base: Option<String>,
    /// Measure this ref as it is instead of applying the spec's changes (CI: a pull request head).
    #[arg(long, value_name = "REF")]
    pub candidate: Option<String>,
    /// Measured iterations (overrides `benchmark.iterations`).
    #[arg(long, value_name = "N")]
    pub iterations: Option<u32>,
    /// Discarded warm-up iterations (overrides `benchmark.warmup`).
    #[arg(long, value_name = "N")]
    pub warmup: Option<u32>,
    /// Timeout per command in seconds (overrides `limits.timeout_secs`).
    #[arg(long, value_name = "SECS")]
    pub timeout: Option<u64>,
    /// Extra environment variable, repeatable (`--env THREADS=8`).
    #[arg(long = "env", value_name = "KEY=VALUE")]
    pub env: Vec<String>,
    /// Do not run the unmodified baseline (measure the experiment only).
    #[arg(long)]
    pub no_baseline: bool,
    /// Skip `command.test` even when the spec defines it.
    #[arg(long)]
    pub skip_test: bool,
    /// Disable CPU/memory observation for this run.
    #[arg(long)]
    pub no_observe: bool,
    /// Recreate the experiment branch and worktrees instead of failing.
    #[arg(long)]
    pub force: bool,
    /// Keep the worktrees after the run.
    #[arg(long)]
    pub keep_worktrees: bool,
    /// Run commands inside a Docker or Podman container image (e.g. `rust:1.85-slim`).
    #[arg(long, value_name = "IMAGE")]
    pub container: Option<String>,
    /// Exit with a non-zero status when the primary metric regressed (for CI).
    #[arg(long)]
    pub fail_on_regression: bool,
}

/// `ekbasis compare`
#[derive(Debug, Args)]
pub struct CompareArgs {
    /// Baseline side: run id, `<experiment>` or `<experiment>:baseline`.
    pub left: String,
    /// Experiment side: run id, `<experiment>`, `<experiment>:experiment` or a report file.
    pub right: String,
    /// Regression threshold in percent.
    #[arg(long, value_name = "PERCENT")]
    pub threshold: Option<f64>,
}

/// `ekbasis experiment diff`
#[derive(Debug, Args)]
pub struct DiffArgs {
    /// Left side: experiment name, run id, report file or `<experiment>:<kind>`.
    pub left: String,
    /// Right side: experiment name, run id, report file or `<experiment>:<kind>`.
    pub right: String,
}

/// `ekbasis report`
#[derive(Debug, Args)]
pub struct ReportArgs {
    /// Experiment names (omit with `--all`).
    pub names: Vec<String>,
    /// Report on every experiment in the repository.
    #[arg(long)]
    pub all: bool,
    /// `markdown` writes the full document, `comment` the compact pull request body.
    #[arg(long, value_enum, default_value_t = ReportFormat::Markdown)]
    pub format: ReportFormat,
    /// Write to this file instead of stdout.
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

/// Flavours of the Markdown report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ReportFormat {
    /// Full document: headline, tables, statistics, reproducibility, how to reproduce.
    Markdown,
    /// Compact body for a pull request comment.
    Comment,
}

/// `ekbasis timeline`
#[derive(Debug, Args)]
pub struct TimelineArgs {
    /// How many experiments to show.
    #[arg(long, default_value_t = 10, value_name = "N")]
    pub limit: usize,
    /// Show every recorded run instead of the latest two per experiment.
    #[arg(long)]
    pub all: bool,
}

/// `ekbasis dashboard`
#[derive(Debug, Args)]
pub struct DashboardArgs {
    /// Where to write the page (default: `.ekbasis/dashboard.html` or `.aion/dashboard.html`).
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
    /// How many futures to include.
    #[arg(long, default_value_t = 50, value_name = "N")]
    pub limit: usize,
    /// Also print the page to stdout instead of only writing the file.
    #[arg(long)]
    pub stdout: bool,
}

/// Common CLI runner used by both `ekbasis` and `aion` binaries.
pub fn run_main() {
    let cli = Cli::parse();
    if let Some(dir) = &cli.dir {
        if let Err(error) = std::env::set_current_dir(dir) {
            eprintln!(
                "error: cannot use `{}` as the working directory: {error}",
                dir.display()
            );
            std::process::exit(2);
        }
    }
    if let Err(error) = commands::dispatch(cli) {
        eprintln!("\nerror: {error:#}");
        std::process::exit(1);
    }
}
