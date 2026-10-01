use ekbasis_core::spec::ExperimentSpec;

#[test]
fn matrix_expansion_generates_cartesian_product() {
    let yaml = r#"
experiment:
  name: sweep-test
  base: main
  description: "test matrix with ${{ matrix.threads }} threads"

matrix:
  params:
    threads: [2, 4]
    mode: ["fast", "slow"]

changes:
  - type: replace
    file: src/main.rs
    from: "THREADS = 1"
    to: "THREADS = ${{ matrix.threads }}"

environment:
  MODE: "${{ matrix.mode }}"
  NUM_THREADS: "${{ matrix.threads }}"

command:
  run: "./run --threads ${{ matrix.threads }} --mode ${{ matrix.mode }}"
"#;

    let spec: ExperimentSpec = serde_yaml_ng::from_str(yaml).expect("deserialize");
    spec.validate().expect("validate base spec");

    let variants = spec.expand_matrix().expect("expand matrix");
    assert_eq!(variants.len(), 4);

    let (combo0, var0) = &variants[0];
    assert_eq!(var0.experiment.name, "sweep-test-mode_fast-threads_2");
    assert_eq!(var0.experiment.description.as_deref(), Some("test matrix with 2 threads"));
    assert_eq!(var0.environment.get("MODE").map(|s| s.as_str()), Some("fast"));
    assert_eq!(var0.environment.get("NUM_THREADS").map(|s| s.as_str()), Some("2"));
    assert_eq!(var0.command.run.as_deref(), Some("./run --threads 2 --mode fast"));
    assert!(var0.experiment.tags.contains(&"matrix".to_string()));
    assert!(var0.experiment.tags.contains(&"sweep".to_string()));
    assert!(var0.experiment.tags.contains(&"threads=2".to_string()));
    assert!(var0.experiment.tags.contains(&"mode=fast".to_string()));
    assert_eq!(combo0.get("threads").map(|s| s.as_str()), Some("2"));
    assert_eq!(combo0.get("mode").map(|s| s.as_str()), Some("fast"));

    // Check last variant
    let (_, var3) = &variants[3];
    assert_eq!(var3.experiment.name, "sweep-test-mode_slow-threads_4");
    assert_eq!(var3.command.run.as_deref(), Some("./run --threads 4 --mode slow"));
}

#[test]
fn matrix_with_config_yaml_substitutions() {
    let yaml = r#"
experiment:
  name: config-sweep
  base: main

matrix:
  params:
    batch_size: [32, 64]

changes:
  - type: config
    file: config.json
    key: batch.size
    to: "${{ matrix.batch_size }}"

command:
  run: ./test
"#;

    let spec: ExperimentSpec = serde_yaml_ng::from_str(yaml).expect("deserialize");
    let variants = spec.expand_matrix().expect("expand");
    assert_eq!(variants.len(), 2);

    match &variants[0].1.changes[0] {
        ekbasis_core::spec::Change::Config { to, .. } => {
            // Verify it promotes string "32" to number 32 if valid YAML number
            assert_eq!(*to, serde_yaml_ng::Value::Number(32.into()));
        }
        _ => panic!("expected Config change"),
    }
}

#[test]
fn matrix_empty_or_none_returns_single_variant() {
    let yaml = r#"
experiment:
  name: plain-test
  base: main

command:
  run: ./test
"#;

    let spec: ExperimentSpec = serde_yaml_ng::from_str(yaml).expect("deserialize");
    let variants = spec.expand_matrix().expect("expand");
    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].1.experiment.name, "plain-test");
    assert!(variants[0].0.is_empty());
}
