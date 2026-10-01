# Ekbasis architecture

This document describes how the Ekbasis (formerly AION) implementation is put together. It follows the plan in
[`Plan.md`](../Plan.md) and records where the implementation deliberately differs from it.

## Crate map

| crate | responsibility | depends on |
| --- | --- | --- |
| `ekbasis-core` | shared vocabulary: `ExperimentSpec`, `RunReport`, `EkbasisConfig`/`AionConfig`, `EkbasisPaths`/`AionPaths`, change application (TOML/JSON/YAML/.env editing), formatting, timestamps | serde, serde_yaml_ng, serde_json, toml_edit, chrono |
| `ekbasis-git` | thin wrapper around the `git` CLI: discovery, refs, branch creation, worktrees, commits, diffs, `.gitignore` | — |
| `ekbasis-runner` | the only place that spawns processes: shell/direct split, timeout + process-tree kill, output capture with a size cap, environment whitelist, container isolation (Docker/Podman) | `ekbasis-observer` |
| `ekbasis-observer` | machine metadata (OS/CPU/RAM/GPU/disk) and sampling of CPU, memory, temperature and GPU usage for a process tree; `gpu` module holds the vendor-tool detection and parsers | sysinfo |
| `ekbasis-benchmark` | statistics (`statistics` module: distributions, Welch t-test, graphs), metric deltas with verdicts, comparison rendering and the Markdown renderer (`markdown` module) | `ekbasis-core` |
| `ekbasis-storage` | SQLite schema and queries (experiments, runs, samples) | rusqlite |
| `ekbasis-cli` | `ekbasis` / `aion` binary: argument parsing, command implementations, the pipeline and the report assembly (`report` module) | all of the above |

Dependencies only point downwards, so any crate can be reused by a future GUI, GitHub Action or
embedded use case without dragging the CLI along.

## Pipeline

```text
resolve base ref ──► create branch ekbasis/<name> ──► add worktree ──► apply changes
        │                                                                   │
        │                                                             commit on branch
        ▼                                                                   ▼
baseline worktree (detached at base commit)                        experiment worktree
        │                                                                   │
        └────────────► setup → build → test → warmup → iterations ◄─────────┘
                                        │
                               Stats + Comparison
                                        │
                    .ekbasis/results/*.json  +  .ekbasis/timeline.db
                                        │
                   remove worktrees (the branch stays as the future)
```

### Why worktrees instead of checking out branches

`git worktree` gives every measured state its own directory while sharing one object database.
That means:

* the user's checkout is never touched (no stashing, no dirty-state surprises),
* the baseline and the experiment can be prepared side by side,
* the experiment branch is a *real* branch, so `git log`, `git diff`, `git switch` and code review
  work exactly as they always do (Git = source history, Ekbasis = experimental history).

### What is measured

Ekbasis measures **wall-clock time of a command**, not a library-level microbenchmark. That choice
keeps Ekbasis language- and framework-agnostic: anything that can be started from a shell can be an
experiment target.

`command.run` is executed `benchmark.warmup` times (discarded) and then `benchmark.iterations`
times (measured). Statistics are computed from the measured iterations only:
`count, mean, median, min, max, stddev, cv_percent, p95`.

Every phase is recorded with its command, exit code, duration, stdout/stderr (capped) and spawn
mode, so a report is auditable without re-running anything.

### Observation

While a measured command runs, `ekbasis-observer` runs **two** sampling threads so that a slow
peripheral never hides a fast measurement:

* **process counters** — walks the process tree from the spawned pid and accumulates per-process
  maxima, so a child that exits before the last sample still counts: peak resident memory (sum over
  the tree), CPU time consumed by the tree as a percentage of the observation window
  (`accumulated_cpu_time()` divided by the wall-clock time between spawn and exit), the peak process
  count, and the highest instantaneous CPU reading when the platform reports one. Sampling starts
  dense (10 / 25 / 50 / 100 ms) and then settles on `observer.sample_interval_ms`, because CPU time
  is cumulative and most measured commands are short lived. The whole process table is walked only
  when children have to be discovered; in between, only the pids already known are refreshed.
* **environment** — GPU utilisation/VRAM/temperature via `nvidia-smi` or `rocm-smi` and temperatures
  from sysinfo components, on a >= 250 ms cadence. Vendor tools can take a second to answer; keeping
  them off the counter thread is what guarantees that a 100 ms command still has CPU and memory data.

The two observations are merged when the command exits. The observer reports `null`/nothing when it
cannot attach or when the platform offers no such sensor, instead of inventing values.

## Reproducibility record

`RunReport.repro` stores: commit, base ref, resolved base commit, branch, worktree path, spec path,
**the verbatim spec text at run time**, the environment variables Ekbasis added, whether a
whitelisted environment was used, the commands, the iteration order, and — inside `machine` — OS,
CPU, core counts and RAM. The database row points at the JSON report, and the JSON report contains
everything the CLI printed.

## Statistics

A comparison keeps descriptive and inferential numbers apart:

* **descriptive** — `Distribution` per side: mean, median, min, max, sigma, CV%, p95, 20% trimmed
  mean, MAD, IQR and Tukey outliers (with the iteration numbers they came from);
* **inferential** — Welch's two-sample t-test on the primary measurement: `t`, Welch–Satterthwaite
  `df`, two-sided p-value, the 95% confidence interval of `candidate - baseline`, and Cohen's d
  with the usual negligible/small/medium/large label.

The p-value comes from the regularised incomplete beta function (Lentz's continued fraction) and
the critical value from a bisection on that CDF; both are validated against published t-tables in
`crates/ekbasis-benchmark/src/statistics.rs`.

Verdicts stay threshold-based (`regression_threshold_percent`), but a delta that clears the
threshold and is *not* statistically significant produces a warning instead of quiet confidence, and
outliers are reported rather than averaged away. `variance_text`-style information also feeds the
Markdown report, so a reviewer sees the same caveats as the CLI operator.

Context metrics (peak memory, average CPU, GPU utilisation, VRAM, maximum temperature) are reported
in a separate table and never counted in the verdict: they are peaks of resources the benchmark did
not set out to measure.

## GPU and temperature observation

* **live metrics** — `nvidia-smi` (utilisation, VRAM, temperature) or `rocm-smi` is detected once per
  run and sampled on a slower cadence than CPU/memory (>= 250 ms), because starting a vendor tool is
  expensive;
* **static metadata** — `nvidia-smi -q`-style queries, `Get-CimInstance Win32_VideoController`
  (Windows), `lspci` (Linux) or `system_profiler` (macOS) fill `MachineInfo.gpus`, which is part of
  the hardware compatibility check between two runs;
* **temperatures** — sysinfo components are sampled best effort; a machine without sensors simply
  reports nothing.

Everything is fail-soft: a missing GPU, driver or vendor tool produces no GPU metrics, never an
error. The parsers are covered by unit tests with captured tool output.

## GitHub Actions

`.github/actions/ekbasis` (and alias `.github/actions/aion`) is a composite action:

```text
install Rust → cache + build the Ekbasis CLI → ekbasis init → ekbasis experiment run --all
             → ekbasis report (markdown + comment) → upload artifact → gh pr comment --edit-last
```

The workflow measures the pull request base commit and head commit on the same runner
(`--base <base.sha> --candidate <head.sha>`), so the two sides share hardware, toolchain and
iteration count. `--candidate` puts the pipeline in CI mode: no branch is created and the spec's
declared changes are ignored (with a printed note), because a pull request already carries its own
commits. `ekbasis experiment run --fail-on-regression` turns a measured regression into a non-zero exit
code *after* the JSON/Markdown evidence has been written.

## Failure behaviour

* A failed `setup` or `build` aborts that side: the report records `success = false`, the phases up
  to the failure, and the warnings; benchmark iterations are skipped ("this future did not build"
  is a result too).
* A failed `test` does **not** abort: the numbers are still measured, but the comparison marks the
  correctness check as failed and the CLI exits non-zero.
* A timed-out command is killed including its process tree; its sample is recorded as failed and
  excluded from the statistics, and the run is flagged.
* A command whose process tree exceeds `limits.max_processes` is killed the same way; the phase
  records `limit_hit` (the cap that was exceeded) and the failure reason says so explicitly.
* `limits.network: block` on a platform that cannot enforce it aborts `ekbasis experiment run` before
  any branch or worktree is created — an unenforceable isolation is an error, never a silent
  connected run.
* `--force` is required to rebuild a timeline that already exists, so a recorded future is never
  silently overwritten.
