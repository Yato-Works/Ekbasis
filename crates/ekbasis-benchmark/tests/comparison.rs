//! Statistics, verdicts and the rendered comparison.

use ekbasis_benchmark::{
    CheckState, Direction, MetricDelta, Unit, Verdict, compare_reports,
};
use ekbasis_core::model::{
    MachineInfo, PhaseKind, PhaseReport, ReproInfo, RunKind, RunReport, SampleRecord, Stats,
    TestSummary,
};
use ekbasis_core::CommandSpec;

/// Builds a synthetic report so the comparison logic can be exercised without running anything.
fn report(kind: RunKind, label: &str, commit: &str, samples: &[f64], test_passes: bool) -> RunReport {
    let build = PhaseReport {
        kind: PhaseKind::Build,
        iteration: None,
        command: "cargo build --release".to_string(),
        spawn_mode: "direct".to_string(),
        exit_code: Some(0),
        success: true,
        timed_out: false,
        limit_hit: None,
        duration_ms: 1000.0,
        stdout: String::new(),
        stderr: String::new(),
        stdout_truncated: false,
        stderr_truncated: false,
        observation: None,
    };
    RunReport {
        aion_version: "test".to_string(),
        experiment: "demo".to_string(),
        kind,
        label: label.to_string(),
        commit: commit.to_string(),
        base_commit: "base".to_string(),
        branch: None,
        spec_path: ".aion/experiments/demo.yaml".to_string(),
        started_at: "2026-01-01T00:00:00Z".to_string(),
        finished_at: "2026-01-01T00:01:00Z".to_string(),
        duration_ms: 60_000.0,
        success: true,
        iterations: samples.len() as u32,
        warmup: 1,
        environment: Default::default(),
        machine: MachineInfo {
            os: "TestOS".to_string(),
            arch: "test".to_string(),
            cpu_logical_cores: 4,
            ..Default::default()
        },
        phases: vec![build],
        samples: samples
            .iter()
            .enumerate()
            .map(|(index, duration)| SampleRecord {
                iteration: index as u32 + 1,
                duration_ms: *duration,
                exit_code: Some(0),
                success: true,
                timed_out: false,
                observation: None,
            })
            .collect(),
        stats: Stats::from_samples(samples),
        test: Some(TestSummary {
            command: "cargo test".to_string(),
            success: test_passes,
            exit_code: Some(if test_passes { 0 } else { 1 }),
            duration_ms: 500.0,
            timed_out: false,
        }),
        setup_ms: None,
        build_ms: Some(1000.0),
        repro: ReproInfo {
            commit: commit.to_string(),
            base_ref: Some("main".to_string()),
            base_commit: "base".to_string(),
            branch: None,
            worktree: None,
            spec_path: ".aion/experiments/demo.yaml".to_string(),
            spec_snapshot: String::new(),
            environment: Default::default(),
            clean_env: false,
            commands: CommandSpec::default(),
            run_order: Vec::new(),
        },
        warnings: Vec::new(),
    }
}

#[test]
fn stats_are_computed_from_the_samples() {
    let stats = Stats::from_samples(&[300.0, 100.0, 200.0]).expect("stats");
    assert_eq!(stats.count, 3);
    assert!((stats.mean - 200.0).abs() < 1e-9);
    assert!((stats.median - 200.0).abs() < 1e-9);
    assert!((stats.min - 100.0).abs() < 1e-9);
    assert!((stats.max - 300.0).abs() < 1e-9);
    assert!((stats.stddev - 100.0).abs() < 1e-6);
    assert!((stats.cv_percent - 50.0).abs() < 1e-6);
    assert!((stats.p95 - 300.0).abs() < 1e-9);
    assert!(stats.is_noisy(5.0));
    assert!(Stats::from_samples(&[]).is_none());
}

#[test]
fn verdicts_follow_the_direction_and_threshold() {
    let better = MetricDelta::new(
        "startup",
        Unit::Milliseconds,
        Direction::LowerIsBetter,
        Some(100.0),
        Some(90.0),
        3.0,
    );
    assert_eq!(better.verdict, Verdict::Improved);
    assert!((better.delta_percent.unwrap() + 10.0).abs() < 1e-9);

    let worse = MetricDelta::new(
        "startup",
        Unit::Milliseconds,
        Direction::LowerIsBetter,
        Some(100.0),
        Some(120.0),
        3.0,
    );
    assert_eq!(worse.verdict, Verdict::Regressed);

    let noise = MetricDelta::new(
        "startup",
        Unit::Milliseconds,
        Direction::LowerIsBetter,
        Some(100.0),
        Some(101.0),
        3.0,
    );
    assert_eq!(noise.verdict, Verdict::Neutral);

    let missing = MetricDelta::new(
        "memory",
        Unit::Bytes,
        Direction::LowerIsBetter,
        None,
        Some(1024.0),
        3.0,
    );
    assert_eq!(missing.verdict, Verdict::Unavailable);
    assert!(missing.note.is_some());
}

#[test]
fn comparison_reports_metrics_correctness_and_reliability() {
    let baseline = report(
        RunKind::Baseline,
        "demo:baseline",
        "aaaaaaa",
        &[200.0, 202.0, 198.0],
        true,
    );
    let candidate = report(
        RunKind::Experiment,
        "demo",
        "bbbbbbb",
        &[100.0, 102.0, 98.0],
        true,
    );
    let comparison = compare_reports(&baseline, &candidate, 3.0);

    let startup = comparison
        .metrics
        .iter()
        .find(|metric| metric.name == "startup (mean)")
        .expect("startup metric");
    assert_eq!(startup.verdict, Verdict::Improved);
    assert!(
        (startup.delta_percent.expect("delta") + 50.0).abs() < 1.0,
        "{startup:?}"
    );

    let (improved, regressed, neutral) = comparison.tally();
    assert!(improved >= 1);
    assert_eq!(regressed, 0);
    assert!(neutral >= 1, "equal build times should stay neutral");
    assert!(!comparison.has_failures());

    let rendered = comparison.render();
    assert!(rendered.contains("startup (mean)"));
    assert!(rendered.contains("improved"));
    assert!(rendered.contains("verdict"));
    assert!(rendered.contains("correctness"));
    assert!(comparison.headline().contains("improved"));
}

#[test]
fn a_failed_test_suite_is_a_correctness_failure() {
    let baseline = report(RunKind::Baseline, "demo:baseline", "aaaaaaa", &[200.0], true);
    let candidate = report(RunKind::Experiment, "demo", "bbbbbbb", &[100.0], false);
    let comparison = compare_reports(&baseline, &candidate, 3.0);

    assert!(comparison.has_failures());
    assert!(comparison
        .checks
        .iter()
        .any(|check| check.state == CheckState::Fail && check.name == "tests"));
    assert!(comparison.render().contains("FAILED"));
    assert!(
        comparison
            .warnings
            .iter()
            .any(|warning| warning.contains("indicative")),
        "few samples should be called out: {:?}",
        comparison.warnings
    );
}

#[test]
fn comparing_a_commit_with_itself_is_flagged() {
    let baseline = report(RunKind::Baseline, "demo:baseline", "same", &[100.0, 101.0, 99.0], true);
    let candidate = report(RunKind::Experiment, "demo", "same", &[100.0, 101.0, 99.0], true);
    let comparison = compare_reports(&baseline, &candidate, 3.0);
    assert!(
        comparison
            .warnings
            .iter()
            .any(|warning| warning.contains("same commit")),
        "{:?}",
        comparison.warnings
    );
}

#[test]
fn statistics_and_graphs_describe_the_samples() {
    let baseline = report(
        RunKind::Baseline,
        "demo:baseline",
        "aaaaaaa",
        &[200.0, 201.0, 199.0, 200.0, 200.0],
        true,
    );
    let candidate = report(
        RunKind::Experiment,
        "demo",
        "bbbbbbb",
        &[100.0, 101.0, 99.0, 100.0, 100.0],
        true,
    );
    let comparison = compare_reports(&baseline, &candidate, 3.0);

    let significance = comparison.significance.as_ref().expect("t-test result");
    assert!(significance.significant(), "{significance:?}");
    assert!(significance.interval_excludes_zero(), "{significance:?}");
    assert!(significance.describe().contains("Welch"));

    let distribution = comparison
        .candidate_distribution
        .as_ref()
        .expect("distribution");
    assert!((distribution.trimmed_mean - 100.0).abs() < 1e-9);

    // Context metrics are reported but must not drive the verdict.
    let context = comparison.context_metric_names();
    assert!(context.contains(&"avg cpu".to_string()), "{context:?}");
    assert!(
        !context.contains(&"startup (mean)".to_string()),
        "the verdict metric is not a context metric: {context:?}"
    );
    let (improved, regressed, neutral) = comparison.tally();
    assert_eq!(regressed, 0);
    assert!(improved + neutral >= 5, "the primary metrics carry the verdict");

    let rendered = comparison.render();
    assert!(rendered.contains("statistics"));
    assert!(rendered.contains("context metrics"));
    assert!(rendered.contains("Welch"));
    assert!(comparison.render_graphs().contains('▁') || comparison.render_graphs().contains('█'));
}

#[test]
fn markdown_report_contains_table_statistics_and_context_details() {
    let baseline = report(
        RunKind::Baseline,
        "demo:baseline",
        "aaaaaaa",
        &[200.0, 201.0, 199.0, 200.0, 200.0],
        true,
    );
    let candidate = report(
        RunKind::Experiment,
        "demo",
        "bbbbbbb",
        &[100.0, 101.0, 99.0, 100.0, 100.0],
        true,
    );
    let comparison = compare_reports(&baseline, &candidate, 3.0);
    let markdown = ekbasis_benchmark::comparison_markdown(&comparison);

    assert!(markdown.contains("| metric | baseline | experiment | difference | verdict |"));
    assert!(markdown.contains("| startup (mean) |"));
    assert!(markdown.contains("startup (mean): -50"));
    assert!(markdown.contains("Welch"));
    assert!(markdown.contains("**correctness**"));
    assert!(markdown.contains("<details><summary>context metrics ("));
    let fences = markdown.matches("```").count();
    assert_eq!(fences % 2, 0, "code fences must be balanced:\n{markdown}");
    assert!(ekbasis_benchmark::markdown::headline(&comparison).contains("improved"));
    assert!(ekbasis_benchmark::graph_block(&comparison).contains("baseline"));
}

#[test]
fn markdown_regression_is_bold_and_changed_files_are_listed() {
    let baseline = report(RunKind::Baseline, "demo:baseline", "aaaaaaa", &[100.0, 101.0, 99.0], true);
    let candidate = report(RunKind::Experiment, "demo", "bbbbbbb", &[150.0, 151.0, 149.0], true);
    let comparison = compare_reports(&baseline, &candidate, 3.0);
    let markdown = ekbasis_benchmark::comparison_markdown(&comparison);
    assert!(markdown.contains("**regressed**"), "{markdown}");

    let files = vec![
        "src/main.rs".to_string(),
        "config.toml".to_string(),
    ];
    let section = ekbasis_benchmark::markdown::changed_files_section("aaaaaaa", "bbbbbbb", &files, 8);
    assert!(section.contains("Changed files"));
    assert!(section.contains("`src/main.rs`"));
    assert!(section.contains("2 total"));
    assert!(
        ekbasis_benchmark::markdown::changed_files_section("a", "b", &[], 8).is_empty(),
        "an empty change list must not produce a section"
    );
}
