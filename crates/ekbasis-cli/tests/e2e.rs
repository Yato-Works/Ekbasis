//! End-to-end test of the `aion` binary against a throw-away repository.
//!
//! The test copies `examples/python-project` into a temporary directory, initialises a git
//! repository there, runs a complete experiment (branch, change, benchmark, compare, store) and
//! verifies the recorded artifacts. It is skipped when `git` or `python` is unavailable, so the
//! suite still passes on machines without a Python interpreter.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Path of the binary under test (provided by cargo).
fn aion() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_aion"))
}

/// Repository root of the AION checkout (`crates/aion-cli` -> root).
fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/aion-cli lives inside the repository")
        .to_path_buf()
}

fn tool_available(program: &str, argument: &str) -> bool {
    Command::new(program)
        .arg(argument)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run(program: &str, args: &[&str], cwd: &Path) -> Output {
    Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("cannot run `{program}`: {error}"))
}

fn run_aion(cwd: &Path, args: &[&str]) -> Output {
    Command::new(aion())
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|error| panic!("cannot run the aion binary: {error}"))
}

fn state_dir(sandbox: &Path) -> PathBuf {
    if sandbox.join(".ekbasis").is_dir() {
        sandbox.join(".ekbasis")
    } else {
        sandbox.join(".aion")
    }
}

fn copy_directory(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create directory");
    for entry in std::fs::read_dir(from).expect("read the example directory") {
        let entry = entry.expect("directory entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_directory(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

fn describe(output: &Output) -> String {
    format!(
        "status: {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The fixture itself must be usable, otherwise the test would pass for the wrong reason.
#[test]
fn example_projects_are_present() {
    let examples = repository_root().join("examples");
    for name in ["python-project", "rust-project"] {
        let directory = examples.join(name);
        assert!(
            directory.is_dir(),
            "example project missing: {}",
            directory.display()
        );
    }
    assert!(examples.join("rust-project/config.toml").is_file());
    assert!(examples
        .join("rust-project/bloom-startup-test.yaml")
        .is_file());
    assert!(examples.join("python-project/config.json").is_file());
    assert!(examples
        .join("python-project/service-startup-test.yaml")
        .is_file());
}

#[test]
fn python_experiment_runs_end_to_end() {
    if !tool_available("git", "--version") || !tool_available("python", "--version") {
        eprintln!("skipping: this test needs `git` and `python` on PATH");
        return;
    }

    let example = repository_root().join("examples").join("python-project");
    let sandbox = std::env::temp_dir().join(format!("aion-it-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sandbox);
    copy_directory(&example, &sandbox);

    // A real repository with a real commit: AION measures commits, not working trees.
    let init_git = run("git", &["init", "-q", "-b", "main"], &sandbox);
    assert!(init_git.status.success(), "{}", describe(&init_git));
    let add = run("git", &["add", "-A"], &sandbox);
    assert!(add.status.success(), "{}", describe(&add));
    let commit = run(
        "git",
        &[
            "-c",
            "user.name=AION test",
            "-c",
            "user.email=test@localhost",
            "commit",
            "-qm",
            "initial service",
        ],
        &sandbox,
    );
    assert!(commit.status.success(), "{}", describe(&commit));

    // Install AION and define the experiment from the bundled spec.
    let init = run_aion(&sandbox, &["init"]);
    assert!(init.status.success(), "{}", describe(&init));
    let create = run_aion(
        &sandbox,
        &[
            "experiment",
            "create",
            "service-startup-test",
            "--from",
            "service-startup-test.yaml",
        ],
    );
    assert!(create.status.success(), "{}", describe(&create));

    // Run the whole pipeline.
    let executed = run_aion(
        &sandbox,
        &[
            "experiment",
            "run",
            "service-startup-test",
            "--iterations",
            "3",
            "--warmup",
            "1",
        ],
    );
    assert!(executed.status.success(), "{}", describe(&executed));

    // The alternative timeline exists as a branch, with the change committed on it.
    let branch = run(
        "git",
        &["branch", "--list", "*service-startup-test"],
        &sandbox,
    );
    let branch_text = String::from_utf8_lossy(&branch.stdout);
    assert!(
        branch_text.contains("service-startup-test"),
        "{}",
        describe(&branch)
    );
    let branch_name = branch_text
        .lines()
        .find(|line| line.contains("service-startup-test"))
        .map(|line| line.trim().trim_start_matches('*').trim())
        .expect("branch name");
    let diff = run("git", &["diff", "main", branch_name], &sandbox);
    let diff_text = String::from_utf8_lossy(&diff.stdout);
    assert!(
        diff_text.contains("preload"),
        "expected the config change on the branch, got:\n{diff_text}"
    );

    // The comparison is stored, and the measured delta matches the change that was made.
    let comparison_path = state_dir(&sandbox).join("results/service-startup-test.comparison.json");
    let comparison_text = std::fs::read_to_string(&comparison_path).expect("comparison report");
    let comparison: serde_json::Value =
        serde_json::from_str(&comparison_text).expect("comparison report is JSON");
    let metric = comparison["metrics"]
        .as_array()
        .expect("metrics array")
        .iter()
        .find(|metric| metric["name"] == "startup (mean)")
        .expect("startup metric");
    let delta = metric["delta_percent"].as_f64().expect("delta percentage");
    assert!(
        delta < -20.0,
        "skipping the preload should clearly speed startup up, measured {delta}%"
    );
    assert_eq!(metric["verdict"].as_str(), Some("improved"));
    assert!(
        state_dir(&sandbox).join("timeline.db").is_file(),
        "the experiment database should exist"
    );

    // The recorded runs are visible to the read-only commands.
    let list = run_aion(&sandbox, &["experiment", "list"]);
    assert!(list.status.success(), "{}", describe(&list));
    assert!(String::from_utf8_lossy(&list.stdout).contains("service-startup-test"));

    let compare = run_aion(
        &sandbox,
        &[
            "compare",
            "service-startup-test:baseline",
            "service-startup-test",
        ],
    );
    assert!(compare.status.success(), "{}", describe(&compare));
    assert!(String::from_utf8_lossy(&compare.stdout).contains("improved"));

    let timeline = run_aion(&sandbox, &["timeline"]);
    assert!(timeline.status.success(), "{}", describe(&timeline));

    // Re-running without --force is refused: a recorded future is never overwritten silently.
    let second = run_aion(&sandbox, &["experiment", "run", "service-startup-test"]);
    assert!(
        !second.status.success(),
        "re-running without --force should fail, got: {}",
        describe(&second)
    );

    let _ = std::fs::remove_dir_all(&sandbox);
}

/// `limits.network: block` must isolate the run or refuse it before any branch exists —
/// AION never reports an isolation it did not apply (plan §15).
#[test]
fn network_block_guard_rails_refuse_or_isolate() {
    if !tool_available("git", "--version") || !tool_available("python", "--version") {
        eprintln!("skipping: this test needs `git` and `python` on PATH");
        return;
    }

    let example = repository_root().join("examples").join("python-project");
    let sandbox = std::env::temp_dir().join(format!("aion-net-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sandbox);
    copy_directory(&example, &sandbox);

    let git = |args: &[&str]| {
        let output = run("git", args, &sandbox);
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            describe(&output)
        );
        output
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.name=AION test",
        "-c",
        "user.email=test@localhost",
        "commit",
        "-qm",
        "initial service",
    ]);

    let init = run_aion(&sandbox, &["init"]);
    assert!(init.status.success(), "{}", describe(&init));
    let create = run_aion(
        &sandbox,
        &[
            "experiment",
            "create",
            "service-startup-test",
            "--from",
            "service-startup-test.yaml",
        ],
    );
    assert!(create.status.success(), "{}", describe(&create));

    // Doctor reports the guard rails, including whether this machine can enforce a block.
    let doctor = run_aion(&sandbox, &["doctor"]);
    assert!(doctor.status.success(), "{}", describe(&doctor));
    let doctor_text = String::from_utf8_lossy(&doctor.stdout);
    assert!(
        doctor_text.contains("guard rails"),
        "doctor must report the guard rails: {doctor_text}"
    );
    assert!(
        doctor_text.contains("network block"),
        "doctor must report the network block capability: {doctor_text}"
    );

    // The stored spec carries the policy explicitly; flip it without duplicating the key.
    let spec_path = state_dir(&sandbox).join("experiments/service-startup-test.yaml");
    let spec = std::fs::read_to_string(&spec_path).expect("stored spec is readable");
    assert!(
        spec.contains("network: allow"),
        "the stored spec carries the default network policy: {spec}"
    );
    std::fs::write(&spec_path, spec.replace("network: allow", "network: block"))
        .expect("store the blocked spec");

    let blocked = run_aion(
        &sandbox,
        &[
            "experiment",
            "run",
            "service-startup-test",
            "--force",
            "--iterations",
            "1",
            "--warmup",
            "0",
        ],
    );
    let stdout = String::from_utf8_lossy(&blocked.stdout);
    let stderr = String::from_utf8_lossy(&blocked.stderr);
    let combined = format!("{stdout}{stderr}");

    if cfg!(target_os = "linux") {
        // Linux may enforce the block (unshare) or refuse honestly; both are correct —
        // a silent connected run is not.
        assert!(
            blocked.status.success() || combined.contains("cannot be enforced"),
            "network block must isolate the run or refuse it: {}",
            describe(&blocked)
        );
    } else {
        assert!(
            !blocked.status.success(),
            "network: block must refuse where it cannot be enforced: {}",
            describe(&blocked)
        );
        assert!(
            combined.contains("cannot be enforced"),
            "the refusal names the reason: {}",
            describe(&blocked)
        );
        assert!(
            !combined.contains("create branch"),
            "the refusal must happen before any branch or worktree is created: {}",
            describe(&blocked)
        );
        let branch = git(&["branch", "--list", "aion/service-startup-test"]);
        assert!(
            String::from_utf8_lossy(&branch.stdout).trim().is_empty(),
            "no experiment branch may exist after the refusal: {}",
            describe(&branch)
        );
    }

    let _ = std::fs::remove_dir_all(&sandbox);
}

/// CI-shaped flow: measure a candidate ref against a base ref, render the artifacts, gate on it.
#[test]
fn ci_mode_reports_and_diffs_end_to_end() {
    if !tool_available("git", "--version") || !tool_available("python", "--version") {
        eprintln!("skipping: this test needs `git` and `python` on PATH");
        return;
    }

    let example = repository_root().join("examples").join("python-project");
    let sandbox = std::env::temp_dir().join(format!("aion-ci-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sandbox);
    copy_directory(&example, &sandbox);

    let git = |args: &[&str]| {
        let output = run("git", args, &sandbox);
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            describe(&output)
        );
        output
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.name=AION test",
        "-c",
        "user.email=test@localhost",
        "commit",
        "-qm",
        "initial service",
    ]);

    let init = run_aion(&sandbox, &["init"]);
    assert!(init.status.success(), "{}", describe(&init));
    let create = run_aion(
        &sandbox,
        &[
            "experiment",
            "create",
            "service-startup-test",
            "--from",
            "service-startup-test.yaml",
        ],
    );
    assert!(create.status.success(), "{}", describe(&create));

    // A pull-request-shaped branch: it makes startup faster by disabling the preload.
    git(&["checkout", "-q", "-b", "feature/faster"]);
    let config_path = sandbox.join("config.json");
    let config = std::fs::read_to_string(&config_path).expect("config.json");
    std::fs::write(
        &config_path,
        config.replace("\"preload\": true", "\"preload\": false"),
    )
    .expect("write config.json");
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.name=AION test",
        "-c",
        "user.email=test@localhost",
        "commit",
        "-qm",
        "speed up startup",
    ]);

    let executed = run_aion(
        &sandbox,
        &[
            "experiment",
            "run",
            "service-startup-test",
            "--base",
            "main",
            "--candidate",
            "feature/faster",
            "--iterations",
            "3",
            "--warmup",
            "0",
        ],
    );
    assert!(executed.status.success(), "{}", describe(&executed));
    let stdout = String::from_utf8_lossy(&executed.stdout).to_string();
    assert!(stdout.contains("candidate ref"), "{stdout}");
    assert!(stdout.contains("improved"), "{stdout}");

    // The Markdown artifact contains the table, the statistics and the changed files.
    let markdown_path = state_dir(&sandbox).join("results/service-startup-test.report.md");
    let markdown = std::fs::read_to_string(&markdown_path).expect("Markdown report");
    assert!(markdown.contains("| metric | baseline |"), "{markdown}");
    assert!(markdown.contains("Welch"), "{markdown}");
    assert!(markdown.contains("Changed files"), "{markdown}");

    // `aion report` reproduces it; the comment flavour carries the sticky marker.
    let report = run_aion(
        &sandbox,
        &["report", "service-startup-test", "--format", "comment"],
    );
    assert!(report.status.success(), "{}", describe(&report));
    assert!(String::from_utf8_lossy(&report.stdout).contains("<!-- aion-report -->"));

    let diff = run_aion(
        &sandbox,
        &[
            "experiment",
            "diff",
            "service-startup-test:baseline",
            "service-startup-test",
        ],
    );
    assert!(diff.status.success(), "{}", describe(&diff));
    let diff_text = String::from_utf8_lossy(&diff.stdout).to_string();
    assert!(diff_text.contains("specification"), "{diff_text}");
    assert!(diff_text.contains("changed files"), "{diff_text}");

    // The CI gate: a deliberately slower branch fails the run, but the evidence is stored first.
    git(&["checkout", "-q", "-b", "feature/slower", "main"]);
    let config = std::fs::read_to_string(&config_path).expect("config.json");
    std::fs::write(
        &config_path,
        config.replace("\"init_ms\": 60", "\"init_ms\": 400"),
    )
    .expect("write config.json");
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.name=AION test",
        "-c",
        "user.email=test@localhost",
        "commit",
        "-qm",
        "make startup slower",
    ]);

    let gated = run_aion(
        &sandbox,
        &[
            "experiment",
            "run",
            "service-startup-test",
            "--base",
            "main",
            "--candidate",
            "feature/slower",
            "--iterations",
            "3",
            "--warmup",
            "0",
            "--fail-on-regression",
        ],
    );
    assert!(
        !gated.status.success(),
        "--fail-on-regression must fail the run: {}",
        describe(&gated)
    );
    assert!(
        String::from_utf8_lossy(&gated.stderr).contains("regressed"),
        "{}",
        describe(&gated)
    );
    assert!(
        state_dir(&sandbox)
            .join("results/service-startup-test.experiment.json")
            .is_file(),
        "the measured evidence must survive the failed gate"
    );

    let _ = std::fs::remove_dir_all(&sandbox);
}

/// The GitHub Action and the example workflow must stay valid YAML with the documented shape.
#[test]
fn github_action_definition_is_valid() {
    let root = repository_root();

    let action_path = root.join(".github/actions/aion/action.yml");
    let action_text = std::fs::read_to_string(&action_path).expect("action.yml is readable");
    let action: serde_json::Value = serde_json::to_value(
        serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&action_text).expect("action.yml is YAML"),
    )
    .expect("action.yml converts to JSON");

    assert_eq!(action["name"].as_str(), Some("AION experiment"));
    assert_eq!(action["runs"]["using"].as_str(), Some("composite"));
    let inputs = action["inputs"].as_object().expect("inputs");
    for required in [
        "aion-path",
        "aion-binary",
        "experiments",
        "base-ref",
        "candidate-ref",
        "iterations",
        "warmup",
        "fail-on-regression",
        "comment",
    ] {
        assert!(
            inputs.contains_key(required),
            "input `{required}` is missing"
        );
    }

    let steps = action["runs"]["steps"].as_array().expect("steps");
    assert!(steps.len() >= 5, "install, build, run, report and comment");
    for step in steps {
        assert!(
            step["uses"].is_string() || step["shell"].as_str() == Some("bash"),
            "every step needs either an action or a bash shell: {step}"
        );
    }

    let workflow_path = root.join(".github/workflows/aion.yml");
    let workflow_text = std::fs::read_to_string(&workflow_path).expect("workflow is readable");
    let workflow: serde_json::Value = serde_json::to_value(
        serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&workflow_text).expect("workflow is YAML"),
    )
    .expect("workflow converts to JSON");
    assert!(
        workflow["jobs"]["aion"].is_object(),
        "an `aion` job exists"
    );
    assert_eq!(
        workflow["permissions"]["pull-requests"],
        serde_json::json!("write"),
        "posting the comment needs pull-requests: write"
    );
    assert!(
        workflow["jobs"]["aion"]["steps"]
            .as_array()
            .expect("steps")
            .iter()
            .any(|step| step["uses"]
                .as_str()
                .map(|uses| uses.contains(".github/actions/aion"))
                .unwrap_or(false)),
        "the workflow uses the bundled action"
    );
}
