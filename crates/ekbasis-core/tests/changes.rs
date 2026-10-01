//! Behaviour of the change engine: every supported container, guards and failure modes.

use std::fs;
use std::path::{Path, PathBuf};

use ekbasis_core::{Change, ExperimentSpec, apply_changes};
use serde_yaml_ng::Value;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aion-core-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch directory");
    dir
}

fn write(path: &Path, text: &str) {
    fs::write(path, text).expect("write fixture");
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).expect("read fixture")
}

fn config(file: &str, key: &str, from: Option<Value>, to: Value) -> Change {
    Change::Config {
        file: file.to_string(),
        key: key.to_string(),
        from,
        to,
    }
}

#[test]
fn toml_change_keeps_comments_and_reports_the_previous_value() {
    let root = scratch("toml");
    write(
        &root.join("config.toml"),
        "# bloom configuration\n[startup]\npreload = true   # keep the cache warm\ninit_ms = 60\n",
    );

    let outcomes = apply_changes(
        &root,
        &[config(
            "config.toml",
            "startup.preload",
            Some(Value::Bool(true)),
            Value::Bool(false),
        )],
    )
    .expect("change applies");

    assert_eq!(outcomes.len(), 1);
    assert!(
        outcomes[0].detail.contains("true -> false"),
        "{}",
        outcomes[0].detail
    );
    let text = read(&root.join("config.toml"));
    assert!(
        text.contains("# bloom configuration"),
        "comments survive: {text}"
    );
    assert!(
        text.contains("# keep the cache warm"),
        "inline comment survives: {text}"
    );
    assert!(text.contains("preload = false"), "{text}");
    assert!(text.contains("init_ms = 60"), "other keys survive: {text}");
}

#[test]
fn toml_guard_refuses_a_change_that_would_measure_the_wrong_state() {
    let root = scratch("toml-guard");
    write(&root.join("config.toml"), "[startup]\npreload = false\n");

    let error = apply_changes(
        &root,
        &[config(
            "config.toml",
            "startup.preload",
            Some(Value::Bool(true)),
            Value::Bool(false),
        )],
    )
    .expect_err("the guard must refuse");

    let message = format!("{error:#}");
    assert!(message.contains("expected `startup.preload`"), "{message}");
    assert_eq!(read(&root.join("config.toml")), "[startup]\npreload = false\n");
}

#[test]
fn json_change_descends_into_nested_keys_and_arrays() {
    let root = scratch("json");
    write(
        &root.join("config.json"),
        "{\n  \"cache\": { \"levels\": [1, 2, 3] },\n  \"startup\": { \"preload\": true }\n}\n",
    );

    apply_changes(
        &root,
        &[
            config("config.json", "startup.preload", None, Value::Bool(false)),
            config("config.json", "cache.levels.1", None, Value::Number(9.into())),
        ],
    )
    .expect("json changes apply");

    let text = read(&root.join("config.json"));
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("still valid JSON");
    assert_eq!(parsed["startup"]["preload"], serde_json::Value::Bool(false));
    assert_eq!(parsed["cache"]["levels"][1], serde_json::Value::from(9));
}

#[test]
fn yaml_and_env_changes_are_applied() {
    let root = scratch("yaml-env");
    write(&root.join("settings.yaml"), "worker:\n  threads: 4\n");
    write(&root.join(".env"), "# runtime\nTHREADS=4\nMODE=dev\n");

    apply_changes(
        &root,
        &[
            config(
                "settings.yaml",
                "worker.threads",
                None,
                Value::Number(8.into()),
            ),
            config(
                ".env",
                "THREADS",
                Some(Value::String("4".into())),
                Value::String("8".into()),
            ),
        ],
    )
    .expect("yaml and env changes apply");

    let yaml: Value = serde_yaml_ng::from_str(&read(&root.join("settings.yaml"))).unwrap();
    assert_eq!(yaml["worker"]["threads"], Value::Number(8.into()));
    let env = read(&root.join(".env"));
    assert!(env.contains("THREADS=8"), "{env}");
    assert!(env.contains("MODE=dev"), "other entries survive: {env}");
    assert!(env.contains("# runtime"), "comments survive: {env}");
}

#[test]
fn replace_and_file_changes_work_and_validate_paths() {
    let root = scratch("replace-file");
    write(&root.join("main.rs"), "const CACHE: usize = 1024;\n");

    let outcomes = apply_changes(
        &root,
        &[
            Change::Replace {
                file: "main.rs".to_string(),
                from: "1024".to_string(),
                to: "4096".to_string(),
            },
            Change::File {
                path: "config/alt.yaml".to_string(),
                content: "mode: aggressive\n".to_string(),
            },
        ],
    )
    .expect("replace and file changes apply");

    assert!(
        outcomes[0].detail.contains("1 occurrence"),
        "{}",
        outcomes[0].detail
    );
    assert!(read(&root.join("main.rs")).contains("4096"));
    assert_eq!(read(&root.join("config/alt.yaml")), "mode: aggressive\n");

    let escaped = apply_changes(
        &root,
        &[Change::File {
            path: "../outside.txt".to_string(),
            content: "nope".to_string(),
        }],
    )
    .expect_err("paths outside the worktree are refused");
    assert!(
        format!("{escaped:#}").contains("must not contain"),
        "{escaped:#}"
    );

    let missing = apply_changes(
        &root,
        &[Change::Replace {
            file: "main.rs".to_string(),
            from: "does-not-exist".to_string(),
            to: "x".to_string(),
        }],
    )
    .expect_err("a replace without a match is refused");
    assert!(format!("{missing:#}").contains("no occurrence"), "{missing:#}");
}

#[test]
fn the_bundled_template_is_a_valid_spec() {
    let template = ExperimentSpec::template("demo", Some("main"), 5, 1, 120);
    let spec = ExperimentSpec::from_yaml(&template).expect("template parses");
    spec.validate().expect("template validates");
    assert_eq!(spec.experiment.name, "demo");
    assert_eq!(spec.experiment.base.as_deref(), Some("main"));
    assert_eq!(spec.benchmark.iterations, 5);
    assert_eq!(spec.limits.timeout_secs, 120);
    assert!(!spec.changes.is_empty());
}

#[test]
fn specs_reject_typos_and_unsafe_names() {
    let unknown = "experiment:\n  name: demo\n  basse: main\ncommand:\n  run: echo hi\n";
    let error = ExperimentSpec::from_yaml(unknown).expect_err("unknown keys are rejected");
    assert!(format!("{error:#}").contains("basse"), "{error:#}");

    let bad_name =
        ExperimentSpec::from_yaml("experiment:\n  name: ../escape\ncommand:\n  run: echo hi\n")
            .expect("parses");
    assert!(bad_name.validate().is_err());

    let no_run =
        ExperimentSpec::from_yaml("experiment:\n  name: demo\ncommand: {}\n").expect("parses");
    assert!(no_run.validate().is_err());
}

/// Plan §15 limits: the process cap and the network policy round-trip, and a cap of zero
/// (which would kill every command, including trivial ones) is refused.
#[test]
fn process_and_network_limits_parse_and_validate() {
    use ekbasis_core::NetworkPolicy;

    let spec = ExperimentSpec::from_yaml(
        "experiment:\n  name: demo\ncommand:\n  run: echo hi\nlimits:\n  max_processes: 8\n  network: block\n",
    )
    .expect("parses");
    spec.validate().expect("validates");
    assert_eq!(spec.limits.max_processes, Some(8));
    assert_eq!(spec.limits.network, NetworkPolicy::Block);

    // Defaults: no process cap, network untouched.
    let spec = ExperimentSpec::from_yaml("experiment:\n  name: demo\ncommand:\n  run: echo hi\n")
        .expect("parses");
    spec.validate().expect("validates");
    assert_eq!(spec.limits.max_processes, None);
    assert_eq!(spec.limits.network, NetworkPolicy::Allow);

    let zero = ExperimentSpec::from_yaml(
        "experiment:\n  name: demo\ncommand:\n  run: echo hi\nlimits:\n  max_processes: 0\n",
    )
    .expect("parses");
    let error = zero.validate().expect_err("a cap of zero must be refused");
    assert!(format!("{error:#}").contains("max_processes"), "{error:#}");

    let unknown_network = ExperimentSpec::from_yaml(
        "experiment:\n  name: demo\ncommand:\n  run: echo hi\nlimits:\n  network: firewalled\n",
    )
    .expect_err("only allow/block exist");
    assert!(
        format!("{unknown_network:#}").contains("network"),
        "{unknown_network:#}"
    );
}
