//! `aion experiment ...` — define, inspect and run experiments.

use std::collections::BTreeMap;
use std::path::Path;

use ekbasis_core::model::RunKind;
use ekbasis_core::{ExperimentSpec, fmt};
use anyhow::{Context as _, Result};

use crate::commands::Context;
use crate::output;
use crate::pipeline::{self, PipelineOptions};
use crate::{CreateArgs, ListArgs, RunArgs, ShowArgs};

/// `aion experiment create` — writes a commented spec template.
pub fn create(args: &CreateArgs, verbose: bool) -> Result<()> {
    let context = Context::load(verbose)?;
    let name = args.name.trim().to_string();
    let spec_path = context.paths.spec_path(&name);

    println!("{}", output::banner(&format!("experiment create {name}")));

    if spec_path.exists() && !args.force {
        anyhow::bail!(
            "`{}` already exists — edit it, or pass --force to overwrite",
            context.paths.rel(&spec_path)
        );
    }

    let base = args
        .base
        .clone()
        .or_else(|| context.config.defaults.baseline_ref.clone());

    let mut spec = match &args.from {
        Some(source) => {
            let text = std::fs::read_to_string(source)
                .with_context(|| format!("cannot read `{}`", source.display()))?;
            let mut spec = ExperimentSpec::from_yaml(&text)
                .with_context(|| format!("cannot parse `{}`", source.display()))?;
            spec.experiment.name = name.clone();
            spec.experiment.base = base.clone();
            println!("{}", output::kv("source", context.paths.rel(source)));
            spec
        }
        None => {
            let template = if args.matrix {
                ExperimentSpec::template_matrix(&name, base.as_deref())
            } else {
                ExperimentSpec::template(
                    &name,
                    base.as_deref(),
                    context.config.defaults.iterations,
                    context.config.defaults.warmup,
                    context.config.defaults.timeout_secs,
                )
            };
            ExperimentSpec::from_yaml(&template)
                .context("the bundled template is invalid — please report this as a bug")?
        }
    };

    if let Some(description) = &args.description {
        spec.experiment.description = Some(description.clone());
    }
    if let Some(hypothesis) = &args.hypothesis {
        spec.experiment.hypothesis = Some(hypothesis.clone());
    }
    spec.validate()?;
    spec.save(&spec_path)?;

    let base_ref = spec.experiment.base.clone();
    let base_commit = match &base_ref {
        Some(reference) => context.git.resolve(reference).ok(),
        None => context.git.head_sha().ok(),
    };
    let id = context.store.upsert_experiment(
        &name,
        &context.paths.rel(&spec_path),
        base_ref.as_deref(),
        spec.experiment.description.as_deref(),
        base_commit.as_deref(),
    )?;

    println!("{}", output::kv("spec", context.paths.rel(&spec_path)));
    println!("{}", output::kv("experiment id", id.to_string()));
    println!(
        "{}",
        output::kv(
            "base",
            base_ref.unwrap_or_else(|| {
                context
                    .git
                    .head_branch()
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "HEAD".to_string())
            })
        )
    );
    println!(
        "{}",
        output::kv("changes", format!("{} declared", spec.changes.len()))
    );
    println!(
        "{}",
        output::kv(
            "benchmark",
            format!(
                "{} measured, {} warmup",
                spec.benchmark.iterations, spec.benchmark.warmup
            )
        )
    );
    println!();
    println!(
        "the template is a starting point: edit `{}`, then run",
        context.paths.rel(&spec_path)
    );
    println!("  ekbasis experiment run {name}");
    Ok(())
}

/// `aion experiment list` — overview of all experiments, optionally every run.
pub fn list(args: &ListArgs, verbose: bool) -> Result<()> {
    let context = Context::load(verbose)?;
    println!("{}", output::banner("experiment list"));

    let summaries = context.store.experiment_summaries()?;
    if summaries.is_empty() {
        println!("no experiments yet - create one with `ekbasis experiment create <name>`");
        return Ok(());
    }

    let mut rows: Vec<Vec<String>> = Vec::new();
    for summary in &summaries {
        let last = summary.last_run();
        rows.push(vec![
            summary.experiment.name.clone(),
            summary
                .experiment
                .base_ref
                .clone()
                .unwrap_or_else(|| "(current branch)".to_string()),
            summary.runs.len().to_string(),
            last.and_then(|run| run.mean_ms)
                .map(fmt::duration_ms)
                .unwrap_or_else(|| "-".to_string()),
            summary
                .delta_percent
                .map(fmt::delta_percent)
                .unwrap_or_else(|| "-".to_string()),
            last.map(|run| if run.success { "ok" } else { "failed" })
                .unwrap_or("-")
                .to_string(),
            last.map(|run| run.started_at.clone()).unwrap_or_default(),
        ]);
    }
    println!(
        "{}",
        output::table(
            &[
                "experiment",
                "base",
                "runs",
                "last mean",
                "delta",
                "state",
                "last run"
            ],
            &rows
        )
    );

    if args.all {
        for summary in &summaries {
            println!("\n{}", summary.experiment.name);
            let run_rows: Vec<Vec<String>> = summary
                .runs
                .iter()
                .map(|run| {
                    vec![
                        format!("#{}", run.id),
                        run.kind.to_string(),
                        fmt::short_sha(&run.commit_sha),
                        run.summary(),
                        run.cv_percent
                            .map(fmt::percent)
                            .unwrap_or_else(|| "-".to_string()),
                        if run.success { "ok" } else { "failed" }.to_string(),
                        run.started_at.clone(),
                    ]
                })
                .collect();
            println!(
                "{}",
                output::table(
                    &["run", "kind", "commit", "result", "variance", "state", "started"],
                    &run_rows
                )
            );
        }
    }
    Ok(())
}

/// `aion experiment show` — everything AION knows about one experiment.
pub fn show(args: &ShowArgs, verbose: bool) -> Result<()> {
    let context = Context::load(verbose)?;
    let spec_path = context.spec_path(&args.name)?;
    let spec = ExperimentSpec::load(&spec_path)?;
    let raw = ExperimentSpec::read_raw(&spec_path)?;

    println!(
        "{}",
        output::banner(&format!("experiment {}", spec.experiment.name))
    );
    if let Some(description) = &spec.experiment.description {
        println!("{}", output::kv("description", description.clone()));
    }
    if let Some(hypothesis) = &spec.experiment.hypothesis {
        println!("{}", output::kv("hypothesis", hypothesis.clone()));
    }
    println!("{}", output::kv("spec", context.paths.rel(&spec_path)));
    println!(
        "{}",
        output::kv(
            "base",
            spec.experiment
                .base
                .clone()
                .unwrap_or_else(|| "(current branch)".to_string())
        )
    );
    println!(
        "{}",
        output::kv("branch", context.experiment_branch(&spec.experiment.name))
    );
    println!(
        "{}",
        output::kv(
            "benchmark",
            format!(
                "{} iterations, {} warmup, threshold {}%",
                spec.benchmark.iterations,
                spec.benchmark.warmup,
                spec.benchmark.regression_threshold_percent
            )
        )
    );
    println!(
        "{}",
        output::kv(
            "limits",
            format!(
                "timeout {}s, output cap {} KiB, clean env: {}, processes: {}, network: {}",
                spec.limits.timeout_secs,
                spec.limits.max_output_kb,
                spec.limits.clean_env,
                match spec.limits.max_processes {
                    Some(limit) => format!("max {limit}"),
                    None => "unlimited".to_string(),
                },
                spec.limits.network.as_str()
            )
        )
    );

    println!("\nchanges ({})", spec.changes.len());
    if spec.changes.is_empty() {
        println!("  (none — this experiment would not change anything)");
    }
    for change in &spec.changes {
        println!("  - {}", change.describe());
    }

    println!("\ncommands");
    for (phase, command) in spec.command.phases() {
        println!("  {phase:<8}{command}");
    }

    if !spec.environment.is_empty() {
        println!("\nenvironment");
        for (key, value) in &spec.environment {
            println!("  {key}={value}");
        }
    }

    if let Some(experiment) = context.store.experiment_by_name(&args.name)? {
        let runs = context.store.runs_for_experiment(experiment.id)?;
        let baseline_mean = runs
            .iter()
            .filter(|run| run.kind == RunKind::Baseline)
            .find_map(|run| run.mean_ms);
        println!("\nrecorded runs ({})", runs.len());
        if runs.is_empty() {
            println!(
                "  (none yet — run `ekbasis experiment run {}`)",
                spec.experiment.name
            );
        } else {
            let rows: Vec<Vec<String>> = runs
                .iter()
                .map(|run| {
                    let delta = if run.kind == RunKind::Experiment {
                        run.delta_percent(baseline_mean)
                            .map(fmt::delta_percent)
                            .unwrap_or_else(|| "-".to_string())
                    } else {
                        "-".to_string()
                    };
                    vec![
                        format!("#{}", run.id),
                        run.kind.to_string(),
                        fmt::short_sha(&run.commit_sha),
                        run.summary(),
                        run.cv_percent
                            .map(fmt::percent)
                            .unwrap_or_else(|| "-".to_string()),
                        delta,
                        if run.success { "ok" } else { "failed" }.to_string(),
                        run.started_at.clone(),
                    ]
                })
                .collect();
            println!(
                "{}",
                output::table(
                    &["run", "kind", "commit", "result", "sigma", "delta", "state", "started"],
                    &rows
                )
            );
            if args.reports {
                for run in &runs {
                    if let Some(relative) = &run.report_path {
                        let path = context.paths.repo_root().join(relative);
                        println!("\n--- run #{} ({}) ---", run.id, relative);
                        match std::fs::read_to_string(&path) {
                            Ok(text) => println!("{text}"),
                            Err(error) => println!("cannot read `{}`: {error}", path.display()),
                        }
                    }
                }
            }
        }
    }

    println!("\nspec file\n{}", output::indent(&raw, 2));
    Ok(())
}

/// `aion experiment sweep` / `aion sweep` — runs a matrix sweep experiment.
pub fn sweep(args: &RunArgs, verbose: bool) -> Result<()> {
    let context = Context::load(verbose)?;
    let name = args
        .name
        .clone()
        .context("an experiment name is required for sweep")?;
    run_matrix_or_single(&context, &name, args)
}

/// `aion experiment run` — executes the whole pipeline for one or every experiment.
pub fn run(args: &RunArgs, verbose: bool) -> Result<()> {
    let context = Context::load(verbose)?;
    let names: Vec<String> = if args.all {
        let mut names = experiment_names(&context)?;
        names.sort();
        if names.is_empty() {
            anyhow::bail!(
                "no experiment specs found in `{}`",
                context.paths.rel(&context.paths.experiments_dir())
            );
        }
        names
    } else {
        vec![args
            .name
            .clone()
            .context("an experiment name is required (or pass --all)")?]
    };

    let mut failed: Vec<String> = Vec::new();
    for name in &names {
        match run_matrix_or_single(&context, name, args) {
            Ok(()) => {}
            Err(error) => {
                // With `--all` a CI run keeps going: every experiment should report its result.
                if args.all {
                    eprintln!("\nerror: experiment `{name}` failed: {error:#}");
                    failed.push(name.clone());
                } else {
                    return Err(error);
                }
            }
        }
    }
    if !failed.is_empty() {
        anyhow::bail!(
            "{} of {} experiments failed: {}",
            failed.len(),
            names.len(),
            failed.join(", ")
        );
    }
    Ok(())
}

/// Determines whether to run as a matrix sweep or single experiment.
fn run_matrix_or_single(context: &Context, name: &str, args: &RunArgs) -> Result<()> {
    let spec_path = context.spec_path(name)?;
    let spec = ExperimentSpec::load(&spec_path)?;

    if let Some(matrix) = &spec.matrix {
        if !matrix.is_empty() {
            return run_matrix(context, name, &spec, &spec_path, args);
        }
    }

    run_one(context, name, &spec, &spec_path, args)
}

/// Represents one evaluated variant in a matrix sweep.
struct VariantResult {
    name: String,
    params: BTreeMap<String, String>,
    mean_ms: Option<f64>,
    delta_percent: Option<f64>,
    verdict: ekbasis_benchmark::Verdict,
    memory_peak: Option<u64>,
    success: bool,
}

/// Runs a matrix parameter sweep across all generated variants.
fn run_matrix(
    context: &Context,
    base_name: &str,
    base_spec: &ExperimentSpec,
    _spec_path: &Path,
    args: &RunArgs,
) -> Result<()> {
    let variants = base_spec.expand_matrix()?;
    println!(
        "{}",
        output::banner(&format!(
            "matrix sweep {base_name} ({} combinations)",
            variants.len()
        ))
    );

    let mut extra_env: BTreeMap<String, String> = BTreeMap::new();
    for entry in &args.env {
        let (variable, value) = entry
            .split_once('=')
            .with_context(|| format!("`--env {entry}` must be KEY=VALUE"))?;
        let variable = variable.trim();
        if variable.is_empty() {
            anyhow::bail!("`--env {entry}` has an empty variable name");
        }
        extra_env.insert(variable.to_string(), value.to_string());
    }

    let mut shared_baseline: Option<ekbasis_core::RunReport> = None;
    let mut results: Vec<VariantResult> = Vec::with_capacity(variants.len());

    for (index, (params, variant_spec)) in variants.iter().enumerate() {
        let param_str = params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "\n>>> [{}/{}] variant: {} ({param_str})",
            index + 1,
            variants.len(),
            variant_spec.experiment.name
        );

        let variant_spec_path = context.paths.spec_path(&variant_spec.experiment.name);
        variant_spec.save(&variant_spec_path)?;

        let container_override = args.container.as_ref().map(|image| ekbasis_core::ContainerSpec {
            image: image.clone(),
            engine: None,
            mounts: Vec::new(),
        });

        let options = PipelineOptions {
            base_ref: args.base.clone(),
            candidate_ref: args.candidate.clone(),
            iterations: args.iterations,
            warmup: args.warmup,
            timeout_secs: args.timeout,
            extra_env: extra_env.clone(),
            skip_baseline: args.no_baseline,
            shared_baseline: shared_baseline.clone(),
            skip_test: args.skip_test,
            observe: !args.no_observe,
            force: args.force,
            keep_worktrees: args.keep_worktrees,
            container: container_override,
        };

        match pipeline::execute(context, variant_spec, &variant_spec_path, &options) {
            Ok(outcome) => {
                if shared_baseline.is_none() && outcome.baseline.is_some() {
                    shared_baseline = outcome.baseline.clone();
                }

                let mean_ms = outcome.experiment.stats.as_ref().map(|s| s.mean);
                let delta_percent = outcome
                    .comparison
                    .as_ref()
                    .and_then(|c| c.primary_metric())
                    .and_then(|m| m.delta_percent);
                let verdict = outcome
                    .comparison
                    .as_ref()
                    .and_then(|c| c.primary_metric())
                    .map(|m| m.verdict)
                    .unwrap_or(ekbasis_benchmark::Verdict::Neutral);
                let memory_peak = outcome
                    .experiment
                    .samples
                    .iter()
                    .filter_map(|s| s.observation.as_ref())
                    .filter_map(|o| o.peak_memory_bytes)
                    .max();

                results.push(VariantResult {
                    name: variant_spec.experiment.name.clone(),
                    params: params.clone(),
                    mean_ms,
                    delta_percent,
                    verdict,
                    memory_peak,
                    success: outcome.experiment.success,
                });
            }
            Err(err) => {
                eprintln!("variant `{}` failed: {err:#}", variant_spec.experiment.name);
                results.push(VariantResult {
                    name: variant_spec.experiment.name.clone(),
                    params: params.clone(),
                    mean_ms: None,
                    delta_percent: None,
                    verdict: ekbasis_benchmark::Verdict::Neutral,
                    memory_peak: None,
                    success: false,
                });
            }
        }
    }

    // Sort by best delta (lowest delta_percent first: e.g. -20% < -10% < +5%)
    results.sort_by(|a, b| {
        let da = a.delta_percent.unwrap_or(f64::INFINITY);
        let db = b.delta_percent.unwrap_or(f64::INFINITY);
        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
    });

    println!(
        "\n{}",
        output::banner(&format!("matrix sweep summary: {base_name}"))
    );
    let mut rows: Vec<Vec<String>> = Vec::new();
    for res in &results {
        let param_desc = res
            .params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ");
        let delta_str = res
            .delta_percent
            .map(fmt::delta_percent)
            .unwrap_or_else(|| "-".to_string());
        let mean_str = res.mean_ms.map(fmt::duration_ms).unwrap_or_else(|| "-".to_string());
        let mem_str = res.memory_peak.map(fmt::bytes).unwrap_or_else(|| "-".to_string());
        let status = if !res.success {
            "failed".to_string()
        } else {
            res.verdict.as_str().to_string()
        };

        rows.push(vec![
            res.name.clone(),
            param_desc,
            mean_str,
            delta_str,
            status,
            mem_str,
        ]);
    }

    println!(
        "{}",
        output::table(
            &["variant", "parameters", "mean", "delta", "verdict", "peak ram"],
            &rows
        )
    );

    if let Some(best) = results.iter().find(|r| r.success && r.delta_percent.is_some()) {
        if let Some(delta) = best.delta_percent {
            if delta < 0.0 {
                let params_desc = best
                    .params
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!(
                    "\n  ★ Best configuration: {} ({params_desc}) -> {}",
                    best.name,
                    fmt::delta_percent(delta)
                );
            }
        }
    }

    Ok(())
}

/// Experiment names that have a spec on disk.
fn experiment_names(context: &Context) -> Result<Vec<String>> {
    let directory = context.paths.experiments_dir();
    let mut names = Vec::new();
    for entry in std::fs::read_dir(&directory)
        .with_context(|| format!("cannot read `{}`", directory.display()))?
        .flatten()
    {
        let path = entry.path();
        let is_yaml = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml"))
            .unwrap_or(false);
        if !is_yaml {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
            names.push(stem.to_string());
        }
    }
    Ok(names)
}

/// Runs one experiment end to end.
fn run_one(
    context: &Context,
    _name: &str,
    spec: &ExperimentSpec,
    spec_path: &Path,
    args: &RunArgs,
) -> Result<()> {
    let mut extra_env: BTreeMap<String, String> = BTreeMap::new();
    for entry in &args.env {
        let (variable, value) = entry
            .split_once('=')
            .with_context(|| format!("`--env {entry}` must be KEY=VALUE"))?;
        let variable = variable.trim();
        if variable.is_empty() {
            anyhow::bail!("`--env {entry}` has an empty variable name");
        }
        extra_env.insert(variable.to_string(), value.to_string());
    }

    let container_override = args.container.as_ref().map(|image| ekbasis_core::ContainerSpec {
        image: image.clone(),
        engine: None,
        mounts: Vec::new(),
    });

    let options = PipelineOptions {
        base_ref: args.base.clone(),
        candidate_ref: args.candidate.clone(),
        iterations: args.iterations,
        warmup: args.warmup,
        timeout_secs: args.timeout,
        extra_env,
        skip_baseline: args.no_baseline,
        shared_baseline: None,
        skip_test: args.skip_test,
        observe: !args.no_observe,
        force: args.force,
        keep_worktrees: args.keep_worktrees,
        container: container_override,
    };

    let outcome = pipeline::execute(context, spec, spec_path, &options)?;

    println!("\nresult");
    if let Some(baseline) = &outcome.baseline {
        println!("  baseline    {}", baseline.summary_line());
    }
    println!("  experiment  {}", outcome.experiment.summary_line());
    println!(
        "  recorded    {} · {} artifact(s)",
        outcome.run_ids.len(),
        outcome.written.len()
    );
    if let Some(comparison) = &outcome.comparison {
        println!("  {}", comparison.headline());
    }

    // CI gate: the artifacts are written before this check, so the evidence survives the failure.
    if args.fail_on_regression {
        if let Some(comparison) = &outcome.comparison {
            if let Some(metric) = comparison.primary_metric() {
                if metric.verdict == ekbasis_benchmark::Verdict::Regressed {
                    anyhow::bail!(
                        "`{}` regressed by {} (baseline {} -> experiment {}): the measured result is stored in {}/",
                        metric.name,
                        metric
                            .delta_percent
                            .map(ekbasis_core::fmt::delta_percent)
                            .unwrap_or_else(|| "-".to_string()),
                        metric.baseline_text(),
                        metric.candidate_text(),
                        context.paths.rel(&context.paths.results_dir()),
                    );
                }
            }
        }
    }
    Ok(())
}
