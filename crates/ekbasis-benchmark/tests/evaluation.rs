//! The four-dimension evaluation of a future (plan §19).

use ekbasis_benchmark::{Grade, compare_reports, evaluate};
use ekbasis_core::model::{
    MachineInfo, PhaseKind, PhaseReport, ReproInfo, RunKind, RunReport, SampleRecord, Stats,
    TestSummary,
};
use ekbasis_core::CommandSpec;

/// Synthetic report fixture (same shape as `comparison.rs`).
fn report(
    kind: RunKind,
    label: &str,
    commit: &str,
    samples: &[f64],
    test_passes: bool,
    test_configured: bool,
) -> RunReport {
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
        test: test_configured.then(|| TestSummary {
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

/// Both sides fast, stable, tested and passing — the good future.
fn good_future() -> ekbasis_benchmark::Comparison {
    let baseline = report(
        RunKind::Baseline,
        "demo:baseline",
        "aaaaaaa",
        &[200.0, 201.0, 199.0, 200.0, 200.0],
        true,
        true,
    );
    let candidate = report(
        RunKind::Experiment,
        "demo",
        "bbbbbbb",
        &[100.0, 101.0, 99.0, 100.0, 100.0],
        true,
        true,
    );
    compare_reports(&baseline, &candidate, 3.0)
}

#[test]
fn a_fast_stable_tested_future_earns_an_a_overall() {
    let evaluation = evaluate(&good_future());
    assert_eq!(evaluation.performance.grade, Grade::A, "{evaluation:?}");
    assert_eq!(evaluation.correctness.grade, Grade::A, "{evaluation:?}");
    assert_eq!(evaluation.stability.grade, Grade::A, "{evaluation:?}");
    // Peak memory was not observed in the fixture, so it stays unknown rather than guessed.
    assert_eq!(evaluation.memory.grade, Grade::Unknown, "{evaluation:?}");
    assert_eq!(evaluation.overall, Grade::A, "{evaluation:?}");
    assert!(evaluation.performance.summary.contains('-'));
    assert!(evaluation.letters().starts_with('A'));
}

#[test]
fn overall_is_the_weakest_known_dimension() {
    // A future whose tests fail must not grade well even when it is twice as fast.
    let baseline = report(
        RunKind::Baseline,
        "demo:baseline",
        "aaaaaaa",
        &[200.0, 201.0, 199.0, 200.0, 200.0],
        true,
        true,
    );
    let candidate = report(
        RunKind::Experiment,
        "demo",
        "bbbbbbb",
        &[100.0, 101.0, 99.0, 100.0, 100.0],
        false,
        true,
    );
    let evaluation = evaluate(&compare_reports(&baseline, &candidate, 3.0));
    assert_eq!(evaluation.performance.grade, Grade::A);
    assert_eq!(evaluation.correctness.grade, Grade::F, "{evaluation:?}");
    assert_eq!(evaluation.overall, Grade::F, "{evaluation:?}");
    assert!(
        evaluation.limiting_dimensions().contains(&"Correctness"),
        "{:?}",
        evaluation.limiting_dimensions()
    );
}

#[test]
fn a_significant_regression_is_an_f_performance_grade() {
    let baseline = report(
        RunKind::Baseline,
        "demo:baseline",
        "aaaaaaa",
        &[100.0, 101.0, 99.0, 100.0, 100.0],
        true,
        true,
    );
    let candidate = report(
        RunKind::Experiment,
        "demo",
        "bbbbbbb",
        &[200.0, 201.0, 199.0, 200.0, 200.0],
        true,
        true,
    );
    let evaluation = evaluate(&compare_reports(&baseline, &candidate, 3.0));
    assert_eq!(evaluation.performance.grade, Grade::F, "{evaluation:?}");
    assert!(evaluation.performance.summary.contains("significant"));
    assert_eq!(evaluation.overall, Grade::F);
}

#[test]
fn one_sample_per_side_cannot_earn_an_a_on_performance_or_stability() {
    let baseline = report(RunKind::Baseline, "demo:baseline", "aaaaaaa", &[200.0], true, true);
    let candidate = report(RunKind::Experiment, "demo", "bbbbbbb", &[100.0], true, true);
    let evaluation = evaluate(&compare_reports(&baseline, &candidate, 3.0));
    assert_eq!(evaluation.performance.grade, Grade::B, "{evaluation:?}");
    assert!(
        evaluation.performance.summary.contains("no t-test"),
        "{}",
        evaluation.performance.summary
    );
    assert_eq!(evaluation.stability.grade, Grade::Unknown, "{evaluation:?}");
    assert_eq!(evaluation.overall, Grade::B, "{evaluation:?}");
}

#[test]
fn correctness_without_a_test_command_caps_at_b() {
    let baseline = report(
        RunKind::Baseline,
        "demo:baseline",
        "aaaaaaa",
        &[200.0, 201.0, 199.0],
        true,
        false,
    );
    let candidate = report(
        RunKind::Experiment,
        "demo",
        "bbbbbbb",
        &[100.0, 101.0, 99.0],
        true,
        false,
    );
    let evaluation = evaluate(&compare_reports(&baseline, &candidate, 3.0));
    assert_eq!(evaluation.correctness.grade, Grade::B, "{evaluation:?}");
    assert!(evaluation.correctness.summary.contains("no test command"));
}

#[test]
fn noisy_samples_step_the_stability_grade_down() {
    // Wild jitter: the future cannot support small deltas.
    let baseline = report(
        RunKind::Baseline,
        "demo:baseline",
        "aaaaaaa",
        &[100.0, 200.0, 100.0, 200.0, 100.0],
        true,
        true,
    );
    let candidate = report(
        RunKind::Experiment,
        "demo",
        "bbbbbbb",
        &[100.0, 300.0, 100.0, 300.0, 100.0],
        true,
        true,
    );
    let evaluation = evaluate(&compare_reports(&baseline, &candidate, 3.0));
    assert!(
        evaluation.stability.grade > Grade::B,
        "jitter must not read as stable: {:?}",
        evaluation.stability
    );
    assert!(evaluation.stability.summary.contains("cv"));
}

#[test]
fn evaluation_renders_all_four_dimensions() {
    let evaluation = evaluate(&good_future());
    let rendered = evaluation.render();
    for key in ["performance", "memory", "correctness", "stability", "overall"] {
        assert!(rendered.contains(key), "missing `{key}` in:\n{rendered}");
    }
    // The letters line stays fixed-width for the timeline graph.
    assert_eq!(evaluation.letters().split(' ').count(), 4);
}

#[test]
fn evaluation_round_trips_through_json() {
    let evaluation = evaluate(&good_future());
    let text = serde_json::to_string(&evaluation).expect("serialize");
    let back: ekbasis_benchmark::Evaluation = serde_json::from_str(&text).expect("deserialize");
    assert_eq!(back.overall, evaluation.overall);
    assert_eq!(back.letters(), evaluation.letters());
}

