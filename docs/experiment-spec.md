# Experiment specification reference

An experiment is a YAML file in `.ekbasis/experiments/<name>.yaml` (or `.aion/experiments/<name>.yaml`). It is the *only* place where the
meaning of an alternative timeline is written down — what changes, how to build it, how to run it,
how often to measure it, and what the measurements should be trusted against.

```yaml
experiment:
  name: bloom-startup-test
  base: main
  description: "skip the cache preload"
  hypothesis: "startup gets faster"
  tags: [startup, cache]

changes:
  - type: config
    file: config.toml
    key: startup.preload
    from: true
    to: false

environment:
  THREADS: "8"

command:
  setup: cargo fetch
  build: cargo build --release
  test: cargo test --release
  run: target/release/bloom.exe

benchmark:
  iterations: 5
  warmup: 1
  regression_threshold_percent: 3.0

limits:
  timeout_secs: 180
  max_output_kb: 64
  clean_env: false
```

Unknown keys are rejected (`deny_unknown_fields`), so a typo fails immediately instead of silently
measuring the wrong thing.

## `experiment`

| key | required | meaning |
| --- | --- | --- |
| `name` | yes | experiment name; letters, digits, `.`, `-`, `_` (used for the branch, spec file and result files) |
| `base` | no | ref or commit the timeline branches from; falls back to `defaults.baseline_ref` in `.ekbasis/config.yaml`, then to the current branch |
| `description` | no | one line shown by `ekbasis experiment list` / `show` |
| `hypothesis` | no | what you *expect*; recorded for the log, **never** used to predict or justify a result |
| `tags` | no | free-form labels |

## `changes`

Changes are applied to the experiment worktree in order. Each one is strict — Ekbasis fails instead
of measuring a state it did not intend to create.

### `type: config` — edit one key in a structured file

| key | meaning |
| --- | --- |
| `file` | path relative to the repository root |
| `key` | dotted key path, e.g. `startup.preload`, `cache.levels.0.name` |
| `from` | optional guard: the change is refused if the current value differs |
| `to` | new value (any YAML scalar, list or mapping) |

Supported containers:

| file | handling | notes |
| --- | --- | --- |
| `*.toml` | `toml_edit` | comments and formatting are preserved; keys address tables by name, `[[array of tables]]` cannot be indexed |
| `*.json` | `serde_json` | key order is preserved; numeric path parts index into arrays |
| `*.yaml`, `*.yml` | `serde_yaml_ng` | comments are **not** preserved |
| `.env`, `*.env` | line editor | flat `KEY=VALUE` only; `KEY` must be a single path part |

```yaml
- type: config
  file: config.json
  key: startup.preload
  from: true
  to: false
```

Nested values can be replaced wholesale:

```yaml
- type: config
  file: config.yaml
  key: cache
  to: { size_mb: 512, mode: aggressive }
```

### `type: replace` — literal text replacement

| key | meaning |
| --- | --- |
| `file` | path relative to the repository root |
| `from` | text that must occur at least once |
| `to` | replacement |

All occurrences are replaced; the report records how many were found.

### `type: file` — write a whole file

| key | meaning |
| --- | --- |
| `path` | path relative to the repository root (parent directories are created) |
| `content` | full file content |

## `environment`

Extra environment variables for every command of this experiment. They are added on top of the
inherited environment (or on top of the whitelist when `limits.clean_env` is enabled).
`aion experiment run --env KEY=VALUE` adds or overrides entries for a single run.

## `command`

| key | meaning |
| --- | --- |
| `setup` | executed once per worktree before the build (`npm ci`, `cargo fetch`, …) |
| `build` | executed once per worktree; a failure aborts that side's measurements |
| `test` | executed once per worktree; a failure is recorded and marked in the comparison, the benchmark still runs |
| `run` | **the measured command**; executed `warmup + iterations` times |

Commands are strings executed inside the worktree. When a command contains no shell
metacharacters (`| & ; < > ( ) $ \` " ' ^ % * ? ! { } [ ] ~ #`), AION spawns it directly instead of
going through `cmd /C` / `sh -c`. That removes shell startup cost from the measurement and lets the
observer attach to the real process. Relative program paths are resolved against the worktree, so
`target/release/app.exe` works on Windows too.

At least one of `run` / `test` must be present; `run` is required for a benchmark.

## `benchmark`

| key | default | meaning |
| --- | --- | --- |
| `iterations` | 3 | measured iterations |
| `warmup` | 0 (template writes 1) | discarded iterations executed first |
| `regression_threshold_percent` | 3.0 | deltas below this are reported as `neutral` |

Statistics are computed from the successful measured iterations:
`count`, `mean`, `median`, `min`, `max`, `stddev`, `cv_percent` (coefficient of variation), `p95`.
`cv_percent > 5` raises a reliability warning, as does fewer than three measured iterations.
`--iterations` / `--warmup` on the command line override these values for a single run.

## `limits`

| key | default | meaning |
| --- | --- | --- |
| `timeout_secs` | 600 | wall-clock limit per command; on timeout the whole process tree is killed |
| `max_output_kb` | 256 | per-stream stdout/stderr kept in the JSON report (the rest is discarded and marked as truncated) |
| `clean_env` | false | run commands with only whitelisted environment variables (PATH, TEMP, HOME, CARGO_HOME, …) |
| `max_processes` | unset (unlimited) | cap on the size of one command's process tree; the tree is polled while the command runs and killed (run marked failed) when it exceeds the cap |
| `network` | `allow` | `block` runs every command with no network access via Linux `unshare -Urn`; where the platform cannot enforce it the run refuses to start — AION never reports an isolation it did not apply |

`network: block` is supported on Linux only. `aion doctor` prints whether this machine can enforce
it, and a spec requesting `block` fails fast (before any branch or worktree is created) on a
platform that cannot.

## Report layout (`.aion/results/`)

* `<name>.baseline.json` and `<name>.experiment.json` — full `RunReport`s: commit, branch,
  worktree, machine (CPU, RAM, GPUs, disk space), every phase (command, exit code, duration,
  stdout/stderr), every measured sample with its observation (memory, CPU, GPU, temperature),
  statistics, correctness summary and warnings.
* `<name>.comparison.json` — the metric table, the statistical treatment (t-test, confidence
  interval, Cohen's d), the per-side distributions with outliers, the correctness checks and the
  reliability warnings.
* `<name>.report.md` — the human readable Markdown document (same content as `aion report`): metric
  tables, statistics, correctness, changed files, reproducibility and how to reproduce. The GitHub
  Action uploads it as an artifact and posts/updates it as a pull request comment.

Each report contains `repro.spec_snapshot`: the verbatim spec text used for the run, so later edits
to the spec cannot rewrite the meaning of an old result.

## What a comparison reports

| group | entries |
| --- | --- |
| primary metrics (drive the verdict) | `startup (mean)`, `startup (median)`, `startup (best)`, `startup (p95)`, `startup (trimmed mean)`, `build` |
| context metrics (reported, not counted) | `peak memory`, `avg cpu`, `gpu util`, `vram`, `max temp` |
| statistics | Welch two-sample t-test (`t`, `df`, two-sided `p`), 95% confidence interval of the difference, Cohen's d with an effect label |
| distributions | per side: mean, median, trimmed mean, sigma with CV%, MAD, IQR and Tukey outliers plus a sparkline/histogram in the report |
| correctness | build, tests, iteration completion |
| warnings | high variance (CV > 5%), fewer than three iterations, sample-count mismatch, machine mismatch, same commit on both sides, a threshold-clearing delta that is not statistically significant, and outliers |

A delta whose absolute value stays under `benchmark.regression_threshold_percent` is `neutral`;
anything above it is `improved` or `regressed` according to the metric's direction
(lower is better for every metric Ekbasis currently reports). Context metrics are excluded from that
tally on purpose, and `--fail-on-regression` only looks at the primary metric (`startup (mean)`).

## CI mode (`ekbasis experiment run --candidate <REF>`)

Used by the GitHub Action: instead of creating `ekbasis/<name>` and applying `changes`, Ekbasis checks out
the given ref into a detached worktree and measures it against `--base`.

* the spec's `changes` are ignored (a note is printed and the run warns about it),
* no branch is created and nothing is committed, so a pull request stays the only source of truth,
* everything else (baseline measurement, comparison, artifacts, database rows) behaves exactly like a
  normal run, which means `ekbasis compare`, `ekbasis experiment diff` and `ekbasis report` work on CI results
  too.

## Registered runs (`.ekbasis/timeline.db`)

| table | contents |
| --- | --- |
| `experiments` | name, spec path, base ref, resolved base commit, description, creation time |
| `runs` | one row per measured state: kind (`baseline`/`experiment`), label, branch, commit, timings, statistics, peak memory, average CPU, report path, machine summary |
| `samples` | per-iteration durations of each run |
| `meta` | schema version |

`ekbasis compare`, `ekbasis experiment list` and `ekbasis timeline` read this index; the JSON reports remain
the source of truth for a comparison itself, so `ekbasis compare` refuses to compare runs whose report
file was deleted rather than guessing.
