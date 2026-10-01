//! The experiment pipeline (plan §4, §11):
//!
//! ```text
//! Base Commit -> Create Branch -> Apply Changes -> Build -> Test -> Benchmark -> Collect -> Compare
//! ```
//!
//! With `--candidate <REF>` the pipeline switches to *CI mode*: instead of creating a branch and
//! applying the declared changes, it measures an existing ref (a pull request head, for example)
//! against the base commit, which is exactly what a review needs.
//!
//! The pipeline is deliberately linear and verbose: every step prints what it did, every phase is
//! recorded in the report, and nothing is measured twice by accident.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ekbasis_benchmark::{Comparison, compare_reports};
use ekbasis_core::changes::apply_changes;
use ekbasis_core::model::{
    MachineInfo, PhaseKind, PhaseReport, ReproInfo, RunKind, RunReport, SampleRecord, Stats,
    TestSummary,
};
use ekbasis_core::{CommandSpec, ExperimentSpec, NetworkPolicy, fmt};
use ekbasis_observer::{ObserveOptions, collect_machine_info_for};
use ekbasis_runner::{Executor, RawOutcome};
use ekbasis_storage::RunRecord;
use anyhow::{Context as _, Result, bail};

use crate::commands::{Context, resolve_threshold};
use crate::output;

/// Command line overrides for a single run.
#[derive(Debug, Clone)]
pub struct PipelineOptions {
    pub base_ref: Option<String>,
    /// Measure this ref as-is instead of creating a branch and applying the spec's changes.
    /// Used by `aion experiment run --candidate` to compare a pull request head against its base.
    pub candidate_ref: Option<String>,
    pub iterations: Option<u32>,
    pub warmup: Option<u32>,
    pub timeout_secs: Option<u64>,
    pub extra_env: BTreeMap<String, String>,
    pub skip_baseline: bool,
    /// When provided, this baseline report is reused instead of measuring the base commit again.
    pub shared_baseline: Option<RunReport>,
    pub skip_test: bool,
    pub observe: bool,
    pub force: bool,
    pub keep_worktrees: bool,
    /// Container sandbox to execute commands in (Docker / Podman).
    pub container: Option<ekbasis_core::ContainerSpec>,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        PipelineOptions {
            base_ref: None,
            candidate_ref: None,
            iterations: None,
            warmup: None,
            timeout_secs: None,
            extra_env: BTreeMap::new(),
            skip_baseline: false,
            shared_baseline: None,
            skip_test: false,
            observe: true,
            force: false,
            keep_worktrees: false,
            container: None,
        }
    }
}

/// Everything the pipeline produced.
pub struct PipelineOutcome {
    pub baseline: Option<RunReport>,
    pub experiment: RunReport,
    pub comparison: Option<Comparison>,
    /// `(label, run id)` pairs written to the database.
    pub run_ids: Vec<(String, i64)>,
    /// Files written under `.aion/results/`.
    pub written: Vec<PathBuf>,
}

/// Which software state is being measured.
struct Side {
    kind: RunKind,
    label: String,
    branch: Option<String>,
    worktree: PathBuf,
    commit: String,
    base_commit: String,
    base_ref: Option<String>,
}

/// Accumulates the observation of one side while the pipeline runs.
struct SideBuilder {
    experiment: String,
    kind: RunKind,
    label: String,
    branch: Option<String>,
    commit: String,
    base_commit: String,
    base_ref: Option<String>,
    worktree: Option<String>,
    spec_path: String,
    spec_snapshot: String,
    started_at: String,
    started: Instant,
    iterations: u32,
    warmup: u32,
    environment: BTreeMap<String, String>,
    clean_env: bool,
    commands: CommandSpec,
    machine: MachineInfo,
    phases: Vec<PhaseReport>,
    samples: Vec<SampleRecord>,
    setup_ms: Option<f64>,
    build_ms: Option<f64>,
    test: Option<TestSummary>,
    success: bool,
    warnings: Vec<String>,
    run_order: Vec<String>,
}

impl SideBuilder {
    /// Starts collecting observations for one side.
    #[allow(clippy::too_many_arguments)]
    fn new(
        context: &Context,
        spec: &ExperimentSpec,
        options: &PipelineOptions,
        machine: MachineInfo,
        spec_snapshot: &str,
        side: &Side,
        iterations: u32,
        warmup: u32,
    ) -> SideBuilder {
        let mut environment = spec.environment.clone();
        for (key, value) in &options.extra_env {
            environment.insert(key.clone(), value.clone());
        }
        SideBuilder {
            experiment: spec.experiment.name.clone(),
            kind: side.kind,
            label: side.label.clone(),
            branch: side.branch.clone(),
            commit: side.commit.clone(),
            base_commit: side.base_commit.clone(),
            base_ref: side.base_ref.clone(),
            worktree: Some(context.paths.rel(&side.worktree)),
            spec_path: context.paths.rel(&context.paths.spec_path(&spec.experiment.name)),
            spec_snapshot: spec_snapshot.to_string(),
            started_at: ekbasis_core::now_rfc3339(),
            started: Instant::now(),
            iterations,
            warmup,
            environment,
            clean_env: spec.limits.clean_env,
            commands: spec.command.clone(),
            machine,
            phases: Vec::new(),
            samples: Vec::new(),
            setup_ms: None,
            build_ms: None,
            test: None,
            success: true,
            warnings: Vec::new(),
            run_order: Vec::new(),
        }
    }

    /// Records one executed command as a phase of the report.
    fn record(&mut self, kind: PhaseKind, iteration: Option<u32>, outcome: &RawOutcome) {
        self.phases.push(phase_report(kind, iteration, outcome));
    }

    /// True when at least one measured iteration completed.
    fn has_samples(&self) -> bool {
        !self.samples.is_empty()
    }

    /// Freezes the accumulated observations into a report.
    fn finalize(self, expect_samples: bool) -> RunReport {
        let successful: Vec<f64> = self
            .samples
            .iter()
            .filter(|sample| sample.success)
            .map(|sample| sample.duration_ms)
            .collect();
        let stats = Stats::from_samples(&successful);
        let all_iterations_ok = self.samples.iter().all(|sample| sample.success);
        let measured_ok = !expect_samples || (self.has_samples() && all_iterations_ok);
        let success = self.success && measured_ok;

        RunReport {
            aion_version: ekbasis_core::AION_VERSION.to_string(),
            experiment: self.experiment,
            kind: self.kind,
            label: self.label,
            commit: self.commit.clone(),
            base_commit: self.base_commit.clone(),
            branch: self.branch.clone(),
            spec_path: self.spec_path.clone(),
            started_at: self.started_at,
            finished_at: ekbasis_core::now_rfc3339(),
            duration_ms: self.started.elapsed().as_secs_f64() * 1000.0,
            success,
            iterations: self.iterations,
            warmup: self.warmup,
            environment: self.environment.clone(),
            machine: self.machine,
            phases: self.phases,
            samples: self.samples,
            stats,
            test: self.test,
            setup_ms: self.setup_ms,
            build_ms: self.build_ms,
            repro: ReproInfo {
                commit: self.commit,
                base_ref: self.base_ref,
                base_commit: self.base_commit,
                branch: self.branch.clone(),
                worktree: self.worktree,
                spec_path: self.spec_path,
                spec_snapshot: self.spec_snapshot,
                environment: self.environment,
                clean_env: self.clean_env,
                commands: self.commands,
                run_order: self.run_order,
            },
            warnings: self.warnings,
        }
    }
}

/// Converts a raw command result into the serializable phase report.
fn phase_report(kind: PhaseKind, iteration: Option<u32>, outcome: &RawOutcome) -> PhaseReport {
    PhaseReport {
        kind,
        iteration,
        command: outcome.command.clone(),
        spawn_mode: outcome.spawn_mode.as_str().to_string(),
        exit_code: outcome.exit_code,
        success: outcome.success,
        timed_out: outcome.timed_out,
        limit_hit: outcome.limit_hit,
        duration_ms: outcome.duration_ms(),
        stdout: outcome.stdout.clone(),
        stderr: outcome.stderr.clone(),
        stdout_truncated: outcome.stdout_truncated,
        stderr_truncated: outcome.stderr_truncated,
        observation: outcome.observation.clone(),
    }
}

/// Runs one side of the pipeline: setup, build, test, warm-up and measured iterations.
#[allow(clippy::too_many_arguments)]
fn measure_side(
    context: &Context,
    spec: &ExperimentSpec,
    options: &PipelineOptions,
    machine: &MachineInfo,
    spec_snapshot: &str,
    side: &Side,
    iterations: u32,
    warmup: u32,
    timeout_secs: u64,
) -> Result<RunReport> {
    let mut builder = SideBuilder::new(
        context,
        spec,
        options,
        machine.clone(),
        spec_snapshot,
        side,
        iterations,
        warmup,
    );
    let expect_samples = spec.command.run.is_some();

    let observe = if options.observe && context.config.observer.enabled {
        Some(ObserveOptions {
            interval_ms: context.config.observer.sample_interval_ms,
        })
    } else {
        None
    };
    let executor = Executor::new(&side.worktree)
        .with_timeout_secs(timeout_secs)
        .with_max_output_kb(spec.limits.max_output_kb)
        .with_clean_env(spec.limits.clean_env)
        .with_max_processes(spec.limits.max_processes)
        .with_network_block(spec.limits.network == NetworkPolicy::Block)
        .with_container(options.container.clone().or_else(|| spec.limits.container.clone()))
        .with_env(&builder.environment)
        .with_observe(observe)
        .with_verbose(context.verbose);

    // ---- setup ------------------------------------------------------------
    if let Some(command) = &spec.command.setup {
        let outcome = executor.run(command)?;
        builder.setup_ms = Some(outcome.duration_ms());
        builder.record(PhaseKind::Setup, None, &outcome);
        if !outcome.success {
            builder.success = false;
            builder.warnings.push(format!(
                "setup command failed ({}); nothing was measured",
                failure_text(&outcome)
            ));
            return Ok(builder.finalize(expect_samples));
        }
    }

    // ---- build ------------------------------------------------------------
    if let Some(command) = &spec.command.build {
        let outcome = executor.run(command)?;
        builder.build_ms = Some(outcome.duration_ms());
        builder.record(PhaseKind::Build, None, &outcome);
        if !outcome.success {
            builder.success = false;
            builder.warnings.push(format!(
                "build failed ({}); benchmark iterations were skipped",
                failure_text(&outcome)
            ));
            return Ok(builder.finalize(expect_samples));
        }
    }

    // ---- test -------------------------------------------------------------
    if let Some(command) = &spec.command.test {
        if options.skip_test {
            builder
                .warnings
                .push("test phase skipped by --skip-test".to_string());
        } else {
            let outcome = executor.run(command)?;
            builder.test = Some(TestSummary {
                command: outcome.command.clone(),
                success: outcome.success,
                exit_code: outcome.exit_code,
                duration_ms: outcome.duration_ms(),
                timed_out: outcome.timed_out,
            });
            builder.record(PhaseKind::Test, None, &outcome);
            if !outcome.success {
                builder.success = false;
                builder.warnings.push(format!(
                    "test command failed ({}); the measurements below come from a state whose tests do not pass",
                    failure_text(&outcome)
                ));
            }
        }
    }

    // ---- benchmark --------------------------------------------------------
    if let Some(command) = &spec.command.run {
        for iteration in 1..=warmup {
            let outcome = executor.run(command)?;
            builder.record(PhaseKind::Warmup, Some(iteration), &outcome);
            builder.run_order.push(format!("warmup #{iteration}"));
            if !outcome.success {
                builder.warnings.push(format!(
                    "warm-up iteration #{iteration} failed ({})",
                    failure_text(&outcome)
                ));
            }
        }
        for iteration in 1..=iterations {
            let outcome = executor.run_observed(command)?;
            builder.record(PhaseKind::Run, Some(iteration), &outcome);
            builder.run_order.push(format!("run #{iteration}"));
            builder.samples.push(SampleRecord {
                iteration,
                duration_ms: outcome.duration_ms(),
                exit_code: outcome.exit_code,
                success: outcome.success,
                timed_out: outcome.timed_out,
                observation: outcome.observation.clone(),
            });
            if !outcome.success {
                builder.success = false;
                builder.warnings.push(format!(
                    "measured iteration #{iteration} failed ({})",
                    failure_text(&outcome)
                ));
            }
        }
    } else {
        builder
            .warnings
            .push("no `command.run` configured: nothing was benchmarked".to_string());
    }

    Ok(builder.finalize(expect_samples))
}

/// Short reason why a command failed, used in warnings and error messages.
fn failure_text(outcome: &RawOutcome) -> String {
    outcome
        .failure_reason()
        .unwrap_or_else(|| "unknown failure".to_string())
}

/// Runs the whole pipeline for one experiment (plan §4).
pub fn execute(
    context: &Context,
    spec: &ExperimentSpec,
    spec_path: &Path,
    options: &PipelineOptions,
) -> Result<PipelineOutcome> {
    let name = spec.experiment.name.clone();
    // Guard rails are checked before any branch or worktree exists: an isolation the
    // platform cannot provide must fail the run up front, never mid-measurement (plan §15).
    let has_container = options.container.is_some() || spec.limits.container.is_some();
    if spec.limits.network == NetworkPolicy::Block && !has_container {
        ekbasis_runner::network_block_support().map_err(|reason| {
            anyhow::anyhow!(
                "`limits.network: block` was requested for `{name}` but cannot be enforced: {reason}"
            )
        })?;
    }
    let branch = context.experiment_branch(&name);
    let experiment_worktree = context.experiment_worktree(&name);
    let baseline_worktree = context.baseline_worktree(&name);
    let keep_worktrees = options.keep_worktrees || context.config.git.keep_worktrees;

    // ---- base commit ------------------------------------------------------
    let base_request = options
        .base_ref
        .clone()
        .or_else(|| spec.experiment.base.clone())
        .or_else(|| context.config.defaults.baseline_ref.clone())
        .or_else(|| context.git.head_branch().ok().flatten());
    let Some(base_request) = base_request else {
        bail!(
            "cannot determine a base ref: pass `--base <ref>` or set `experiment.base` in the spec"
        );
    };
    let base_commit = context
        .git
        .resolve(&base_request)
        .with_context(|| format!("cannot resolve base ref `{base_request}`"))?;
    let base_info = context.git.commit_info(&base_commit)?;

    let iterations = options
        .iterations
        .unwrap_or(spec.benchmark.iterations)
        .max(1);
    let warmup = options.warmup.unwrap_or(spec.benchmark.warmup);
    let timeout_secs = options.timeout_secs.unwrap_or(spec.limits.timeout_secs).max(1);
    let threshold = resolve_threshold(
        &context.config,
        None,
        Some(spec.benchmark.regression_threshold_percent),
    );
    let machine = collect_machine_info_for(context.paths.repo_root());
    let spec_snapshot = ExperimentSpec::read_raw(spec_path)?;

    println!("{}", output::banner(&format!("experiment run {name}")));
    println!(
        "{}",
        output::kv(
            "base",
            format!("{base_request} ({})", fmt::short_sha(&base_commit))
        )
    );
    println!(
        "{}",
        output::kv("base commit", fmt::truncate_line(&base_info.subject, 56))
    );
    println!("{}", output::kv("branch", branch.clone()));
    if let Some(candidate) = &options.candidate_ref {
        println!(
            "{}",
            output::kv(
                "mode",
                format!("CI: `{candidate}` is measured as it is against the base commit")
            )
        );
    }
    println!(
        "{}",
        output::kv(
            "iterations",
            format!("{iterations} measured, {warmup} warmup, timeout {timeout_secs}s")
        )
    );
    println!("{}", output::kv("threshold", format!("{threshold}%")));
    println!("{}", output::kv("machine", machine.summary()));

    if !spec.changes.is_empty() {
        println!("\nchanges");
        for change in &spec.changes {
            println!("  - {}", change.describe());
        }
    }
    if context.git.is_dirty().unwrap_or(false) {
        println!(
            "\nnote: this checkout has uncommitted changes; AION measures commits, not the working tree"
        );
    }

    println!("\npipeline");
    println!(
        "  [1/8] base commit       {} {}",
        fmt::short_sha(&base_commit),
        fmt::truncate_line(&base_info.subject, 40)
    );

    // ---- prepare the experiment side --------------------------------------
    // In CI mode (`--candidate`) the candidate ref *is* the experiment: AION measures it as it is,
    // because a pull request already carries its own commits.
    let (experiment_side, applied_changes) = match options.candidate_ref.clone() {
        Some(candidate_ref) => {
            let candidate_commit = context
                .git
                .resolve(&candidate_ref)
                .with_context(|| format!("cannot resolve candidate ref `{candidate_ref}`"))?;
            println!(
                "  [2/8] candidate ref     {candidate_ref} ({})",
                fmt::short_sha(&candidate_commit)
            );
            if !spec.changes.is_empty() {
                println!(
                    "          note: the {} declared change(s) are ignored — with --candidate the ref itself is measured",
                    spec.changes.len()
                );
            }
            cleanup_worktree(context, &experiment_worktree, options.force)?;
            context
                .git
                .worktree_add(&experiment_worktree, None, Some(&candidate_commit), false)?;
            println!(
                "  [3/8] worktree          {} (detached at {})",
                context.paths.rel(&experiment_worktree),
                fmt::short_sha(&candidate_commit)
            );
            println!("  [4/8] apply changes     skipped (--candidate)");
            println!("  [5/8] commit            skipped (the candidate is already committed)");
            (
                Side {
                    kind: RunKind::Experiment,
                    label: name.clone(),
                    branch: None,
                    worktree: experiment_worktree.clone(),
                    commit: candidate_commit,
                    base_commit: base_commit.clone(),
                    base_ref: Some(base_request.clone()),
                },
                0usize,
            )
        }
        None => {
            prepare_experiment_state(context, &branch, &experiment_worktree, options.force)?;
            context.git.create_branch(&branch, &base_commit)?;
            println!("  [2/8] create branch     {branch}");
            context
                .git
                .worktree_add(&experiment_worktree, Some(&branch), None, false)?;
            println!(
                "  [3/8] worktree          {}",
                context.paths.rel(&experiment_worktree)
            );

            let outcomes = apply_changes(&experiment_worktree, &spec.changes)?;
            if outcomes.is_empty() {
                println!("  [4/8] apply changes     none declared");
            } else {
                println!("  [4/8] apply changes     {}", outcomes.len());
                for outcome in &outcomes {
                    println!(
                        "          {} {} - {}",
                        outcome.kind, outcome.file, outcome.detail
                    );
                }
            }

            let commit = {
                let worktree_git = context.git.in_dir(&experiment_worktree);
                if worktree_git.is_dirty()? {
                    let mut message = format!(
                        "aion({name}): apply experiment changes\n\nbase: {}\n",
                        fmt::short_sha(&base_commit)
                    );
                    if outcomes.is_empty() {
                        message
                            .push_str("\n(the diff came from files already staged in the worktree)");
                    } else {
                        message.push('\n');
                        for outcome in &outcomes {
                            message.push_str(&format!(
                                "- {} {}: {}\n",
                                outcome.kind, outcome.file, outcome.detail
                            ));
                        }
                    }
                    let sha = worktree_git.add_all_and_commit(&message)?;
                    println!("  [5/8] commit            {}", fmt::short_sha(&sha));
                    sha
                } else {
                    println!(
                        "  [5/8] commit            no file changed, the branch stays at the base commit"
                    );
                    base_commit.clone()
                }
            };

            (
                Side {
                    kind: RunKind::Experiment,
                    label: name.clone(),
                    branch: Some(branch.clone()),
                    worktree: experiment_worktree.clone(),
                    commit,
                    base_commit: base_commit.clone(),
                    base_ref: Some(base_request.clone()),
                },
                outcomes.len(),
            )
        }
    };
    let experiment_commit = experiment_side.commit.clone();

    // ---- baseline ---------------------------------------------------------
    let baseline_report = if let Some(shared) = &options.shared_baseline {
        println!("  [6/8] baseline          reusing shared baseline ({})", fmt::short_sha(&shared.commit));
        print_side_summary("baseline  ", shared);
        Some(shared.clone())
    } else if options.skip_baseline {
        println!("  [6/8] baseline          skipped (--no-baseline)");
        None
    } else {
        cleanup_worktree(context, &baseline_worktree, options.force)?;
        context
            .git
            .worktree_add(&baseline_worktree, None, Some(&base_commit), false)?;
        println!(
            "  [6/8] baseline          measuring {} in {}",
            fmt::short_sha(&base_commit),
            context.paths.rel(&baseline_worktree)
        );
        let report = measure_side(
            context,
            spec,
            options,
            &machine,
            &spec_snapshot,
            &Side {
                kind: RunKind::Baseline,
                label: format!("{name}:baseline"),
                branch: None,
                worktree: baseline_worktree.clone(),
                commit: base_commit.clone(),
                base_commit: base_commit.clone(),
                base_ref: Some(base_request.clone()),
            },
            iterations,
            warmup,
            timeout_secs,
        )?;
        print_side_summary("baseline  ", &report);
        Some(report)
    };

    // ---- experiment -------------------------------------------------------
    println!(
        "  [7/8] experiment        measuring {} in {}",
        fmt::short_sha(&experiment_commit),
        context.paths.rel(&experiment_worktree)
    );
    let experiment_report = measure_side(
        context,
        spec,
        options,
        &machine,
        &spec_snapshot,
        &experiment_side,
        iterations,
        warmup,
        timeout_secs,
    )?;
    print_side_summary("experiment", &experiment_report);

    // ---- compare and store ------------------------------------------------
    println!(
        "  [8/8] compare & store   {}",
        match &options.candidate_ref {
            Some(_) => "CI mode: candidate ref against the base commit".to_string(),
            None => format!("{applied_changes} applied change(s)"),
        }
    );
    let comparison = baseline_report
        .as_ref()
        .map(|baseline| compare_reports(baseline, &experiment_report, threshold));

    let baseline_path = context.paths.report_path(&name, RunKind::Baseline);
    let experiment_path = context.paths.report_path(&name, RunKind::Experiment);
    let mut written: Vec<PathBuf> = Vec::new();
    if let Some(baseline) = &baseline_report {
        write_json(&baseline_path, baseline)?;
        written.push(baseline_path.clone());
    }
    write_json(&experiment_path, &experiment_report)?;
    written.push(experiment_path.clone());
    if let Some(comparison) = &comparison {
        let path = context.paths.comparison_path(&name);
        write_json(&path, comparison)?;
        written.push(path);
    }

    // Markdown document for CI artifacts, pull request comments and step summaries.
    let changed_files = if experiment_commit == base_commit {
        Vec::new()
    } else {
        context
            .git
            .changed_files(&base_commit, &experiment_commit)
            .unwrap_or_default()
    };
    let report_rel_paths: Vec<String> = written
        .iter()
        .map(|path| context.paths.rel(path))
        .collect();
    let markdown_path = context.paths.report_markdown_path(&name);
    let markdown = {
        let report_context = crate::report::ReportContext {
            experiment: &name,
            baseline: baseline_report.as_ref(),
            candidate: &experiment_report,
            changed_files: &changed_files,
            reports: &report_rel_paths,
        };
        crate::report::document(&report_context, comparison.as_ref())
    };
    crate::report::write(&markdown_path, &markdown)?;
    written.push(markdown_path);

    let experiment_id = context.store.upsert_experiment(
        &name,
        &context.paths.rel(spec_path),
        Some(base_request.as_str()),
        spec.experiment.description.as_deref(),
        Some(&base_commit),
    )?;
    context
        .store
        .set_experiment_base_commit(experiment_id, &base_commit)?;

    let mut run_ids: Vec<(String, i64)> = Vec::new();
    if let Some(baseline) = &baseline_report {
        let record = RunRecord::from_report(
            experiment_id,
            baseline,
            Some(context.paths.rel(&baseline_path)),
        );
        run_ids.push((baseline.label.clone(), context.store.insert_run(&record)?));
    }
    let record = RunRecord::from_report(
        experiment_id,
        &experiment_report,
        Some(context.paths.rel(&experiment_path)),
    );
    run_ids.push((
        experiment_report.label.clone(),
        context.store.insert_run(&record)?,
    ));

    // ---- result -----------------------------------------------------------
    if let Some(comparison) = &comparison {
        println!("\ncomparison");
        print!("{}", output::indent(&comparison.render(), 2));
    }

    println!("\nsaved");
    for path in &written {
        println!("  {}", context.paths.rel(path));
    }
    let ids: Vec<String> = run_ids
        .iter()
        .map(|(label, id)| format!("{label} = run #{id}"))
        .collect();
    println!("  database  {}", ids.join(", "));

    if keep_worktrees {
        println!("\nworktrees kept");
        println!("  {}", context.paths.rel(&experiment_worktree));
        if baseline_report.is_some() {
            println!("  {}", context.paths.rel(&baseline_worktree));
        }
    }

    println!("\ntimeline");
    println!(
        "  {} is the recorded future of {}",
        fmt::short_sha(&experiment_commit),
        fmt::short_sha(&base_commit)
    );
    println!("  git log --oneline {branch}");
    if let Some(comparison) = &comparison {
        println!("  {}", comparison.headline());
    }

    if !experiment_report.warnings.is_empty() {
        println!("\nwarnings");
        for warning in &experiment_report.warnings {
            println!("  {warning}");
        }
    }

    // ---- cleanup ----------------------------------------------------------
    if !keep_worktrees {
        remove_worktree(context, &experiment_worktree);
        if baseline_report.is_some() {
            remove_worktree(context, &baseline_worktree);
        }
    }

    if !experiment_report.success {
        bail!(
            "experiment `{name}` did not complete cleanly — inspect {} and the warnings above",
            context.paths.rel(&experiment_path)
        );
    }

    Ok(PipelineOutcome {
        baseline: baseline_report,
        experiment: experiment_report,
        comparison,
        run_ids,
        written,
    })
}

/// Removes stale experiment state so a timeline can be rebuilt from scratch.
fn prepare_experiment_state(
    context: &Context,
    branch: &str,
    worktree: &Path,
    force: bool,
) -> Result<()> {
    let branch_exists = context.git.branch_exists(branch);
    let worktree_exists = worktree.exists();
    if !branch_exists && !worktree_exists {
        return Ok(());
    }
    if !force {
        if branch_exists {
            let sha = context.git.resolve(branch).unwrap_or_default();
            bail!(
                "branch `{branch}` already exists ({}) — pass --force to rebuild this timeline, or use another experiment name",
                fmt::short_sha(&sha)
            );
        }
        bail!(
            "worktree `{}` already exists — pass --force to clean it up",
            context.paths.rel(worktree)
        );
    }
    remove_worktree(context, worktree);
    if worktree.exists() {
        std::fs::remove_dir_all(worktree)
            .with_context(|| format!("cannot delete `{}`", worktree.display()))?;
        let _ = context.git.worktree_prune();
    }
    if context.git.branch_exists(branch) {
        context.git.delete_branch(branch)?;
    }
    Ok(())
}

/// Removes a worktree, tolerating "not a registered worktree" errors.
fn remove_worktree(context: &Context, worktree: &Path) {
    if !worktree.exists() {
        let _ = context.git.worktree_prune();
        return;
    }
    if context.git.worktree_remove(worktree).is_err() {
        let _ = std::fs::remove_dir_all(worktree);
    }
    let _ = context.git.worktree_prune();
}

/// Verifies a worktree path is free before creating a new one.
fn cleanup_worktree(context: &Context, worktree: &Path, force: bool) -> Result<()> {
    if !worktree.exists() {
        return Ok(());
    }
    if !force {
        bail!(
            "worktree `{}` already exists — pass --force to clean it up",
            context.paths.rel(worktree)
        );
    }
    remove_worktree(context, worktree);
    if worktree.exists() {
        bail!(
            "cannot remove `{}`: close any program using it and try again",
            context.paths.rel(worktree)
        );
    }
    Ok(())
}

/// Writes a JSON artifact under `.aion/results/`.
fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create `{}`", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value).context("cannot serialize the report")?;
    std::fs::write(path, format!("{text}\n"))
        .with_context(|| format!("cannot write `{}`", path.display()))
}

/// Prints the measured result of one measured side.
fn print_side_summary(label: &str, report: &RunReport) {
    let state = if report.success { "ok" } else { "failed" };
    let measured = match report.stats {
        Some(stats) => fmt::stats_line(&stats),
        None => report.summary_line(),
    };
    println!("          {label}: {measured} [{state}]");
    if let Some(build) = report.build_ms {
        println!("          {label}: build {}", fmt::duration_ms(build));
    }
    if let Some(memory) = report.peak_memory_bytes() {
        println!("          {label}: peak memory {}", fmt::bytes(memory));
    }
    if let Some(test) = &report.test {
        println!(
            "          {label}: tests {}",
            if test.success { "passed" } else { "FAILED" }
        );
    }
}
