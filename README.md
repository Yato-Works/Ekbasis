# Ekbasis — Software Timeline & Counterfactual Experiment Engine

<p align="center">
  <strong><a href="README.md">English</a></strong> | <strong><a href="README.ja.md">日本語</a></strong>
</p>

<p align="center">
  <a href="https://github.com/Yato-Works/Ekbasis/actions/workflows/ekbasis.yml"><img src="https://github.com/Yato-Works/Ekbasis/actions/workflows/ekbasis.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"></a>
  <a href="https://www.rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.85%2B-orange.svg" alt="Rust 1.85+"></a>
  <a href="https://github.com/Yato-Works/Ekbasis/releases"><img src="https://img.shields.io/badge/Platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey.svg" alt="Platform: Linux | macOS | Windows"></a>
</p>

> **Git tells you how your software changed. Ekbasis lets you test what it could become.**
>
> Git records the historical facts of how your codebase evolved.  
> **Ekbasis** (Ancient Greek: *ἔκβασις* — outcome, eventuation, consequence) is an automated local experiment engine that tests **counterfactuals** ("what if we made this change?") without guessing: it branches, mutates, compiles, executes, measures, and statistically verifies performance deltas against baseline.

```text
main (Commit 8f4c21a)
 │
 ├── [Production Baseline] ────────── run: ./target/release/server --benchmark ──── 182.4 ms (cv 1.2%)
 │
 ├── [Branch: ekbasis/opt-threads] ─── worker_threads: 4 -> 8 ───────────────────── 112.1 ms (-38.5%, p < 0.001) ★ Best
 │
 └── [Branch: ekbasis/simd-patch] ──── patch: simd-vectorize.patch ──────────────── 141.7 ms (-22.3%, p = 0.004)
```

**Prediction ❌ "This change should probably make it faster."**  
**Execution ✅ "Measured under identical machine conditions; Welch's t-test confirms a statistically significant 38.5% improvement (p < 0.001, 95% CI [-72.9ms, -67.7ms])."**

---

## Why Ekbasis? (Beyond DIY Shell Scripts & Hyperfine)

Engineers often ask: *"Can't I just create a `git worktree`, write a bash script to build, and benchmark it with `hyperfine`?"*

While ad-hoc scripts work for toy projects, rigorous performance engineering hits fundamental barriers that manual tooling cannot solve:

| Feature / Requirement | DIY Shell Scripts | `hyperfine` alone | **Ekbasis** |
| :--- | :---: | :---: | :--- |
| **Counterfactual Git Branching** | ⚠️ Manual worktree management; leaves dirty working trees on failure | ❌ None (no Git awareness) | **✅ Fully automated: isolated worktree → mutation → commit → benchmark → clean teardown** |
| **Surgical Configuration Surgery** | ❌ `sed` / regex breaks formatting and destroys file comments | ❌ None | **✅ AST-level Round-Trip preservation (`toml_edit`, JSON, YAML, `.env`) with validation guards** |
| **Zero-Overhead Process Spawning** | ❌ `sh -c` / `cmd.exe` introduces shell startup and parsing jitter | ⚠️ Spawns commands through system shell by default | **✅ Direct OS process execution without shell wrappers; deep PID process-tree telemetry** |
| **Statistical Inferential Rigor** | ❌ Naive average comparison; cannot distinguish signal from noise | ⚠️ Descriptive stats (mean/stddev) only | **✅ Welch's t-test (unequal variances), p-value, 95% CI, Cohen's d effect size, Tukey's fences** |
| **Daemon & Workload Protocol** | ⚠️ Unhandled background services hang indefinitely | ⚠️ Measures until process exit | **✅ Explicit support for startup benchmarks, workload scenario drivers, and container sandboxes** |
| **Historical Experiment Ledger** | ❌ Ephemeral stdout logs lost in terminal history | ❌ Console output only | **✅ Embedded SQLite (`timeline.db`) indexing every commit, sample, telemetry point, and run diff** |
| **PR Regression Gate & CI** | ⚠️ Fragile homemade CI pipelines | ❌ None | **✅ Same-runner dual baseline/candidate measurement, sticky PR comment updates, and CI failure gates** |

### Where Does Ekbasis Stand? (vs. Criterion.rs & Bencher)

* **vs. In-Process Microbenchmarks (`Criterion.rs`, `Google Benchmark`, `Go test -bench`)**:  
  Criterion measures isolated, nanosecond-to-microsecond in-process pure functions via custom test harnesses.  
  → **Ekbasis operates at the binary and system E2E level**: It tests compiled release binaries, measuring end-to-end realities (startup latency, memory high-water marks, full pipeline throughput, daemon protocols) across counterfactual Git branches without requiring dedicated benchmark code harnesses.
* **vs. Continuous Benchmarking Platforms (`Bencher`)**:  
  Bencher is a cloud platform for tracking trends across CI pipelines over months.  
  → **Ekbasis is the local execution & counterfactual branch engine**: It automatically creates ephemeral Git worktrees, mutates configs/patches, runs identical-runner A/B statistical comparisons, and persists local timeline ledgers (`timeline.db`) *before* code is ever merged to main.

---

## Architectural Deep Dive

### 1. Zero-Overhead Direct Process Execution
Invoking benchmarks via shells (`sh -c` or `cmd.exe`) injects unpredictable initialization latency, environment variable parsing delays, and wrapper process noise into sub-millisecond benchmarks.  
Ekbasis executes target binaries **directly via OS system calls** (`CreateProcessW` / `execve`).

* **High-Watermark Accounting**: For short-lived processes, peak resident memory (RSS) is harvested directly from OS kernel process accounting (`getrusage` / `GetProcessMemoryInfo`), capturing true lifetime peaks without missing sub-tick spikes.
* **Continuous Telemetry**: For longer workloads, the child process tree is polled every 150ms to sample CPU utilization curves, GPU VRAM, and thermal headroom.

### 2. Surgical Round-Trip Configuration Surgery
Ad-hoc text replacement (`type: replace` or `sed`) is brittle—formatting changes or comments can easily cause silent failures or corrupted configs.  
Ekbasis provides first-class, **Round-Trip AST preservation**:
* **TOML**: Powered by `toml_edit`'s Concrete Syntax Tree (CST). It surgically updates the target value while preserving 100% of existing indentation, inline tables, and `# comments`.
* **JSON / YAML**: Structured AST tree navigation preserving object schemas.
* **Pre-condition Guards (`from`)**: Invariant verification—if the upstream default value in `config.toml` has shifted from expected `4` to `8`, the experiment halts safely before generating invalid measurements.
* **Unified Git Patches (`type: patch`)**: Apply standard unified diffs cleanly to source code.

### 3. Benchmarking Long-Running Daemons & Web Services
A common pitfall in benchmarking tools is trying to execute a background daemon (e.g. `run: ./server`), which blocks waiting for requests until it times out. Ekbasis enforces an explicit execution contract:
* **Startup & Init Latency**: Benchmark daemon startup time with self-terminating flags (e.g. `run: ./target/release/server --benchmark-startup` or `--check`).
* **End-to-End Workload Drivers**: Benchmark full client-server scenarios by executing a test driver script (e.g. `run: ./scripts/bench-workload.sh`) that launches the service, fires deterministic load (via `oha`, `wrk`, or integration tests), and terminates cleanly with exit code `0`.
* **Batch / CLI Pipelines**: Directly benchmark algorithmic workloads (e.g. `run: ./target/release/indexer --input bench.bin`).

### 4. Statistical Rigor & Sample Size Guidelines
Ekbasis treats statistical claims with utmost integrity:
* **Recommended Sample Size**: While $N=5$ iterations suffice for rapid CI smoke testing, Ekbasis recommends **$N \ge 15 \sim 30$** for definitive microbenchmarks to dampen OS scheduler context switches and CPU governor transitions.
* **Welch's Two-Sample t-Test**: Accommodates unequal variances ($\sigma_1^2 \ne \sigma_2^2$) between baseline and experiment, calculating fractional degrees of freedom via the Welch–Satterthwaite equation.
* **Tukey's Fences ($1.5 \times \text{IQR}$)**: Outliers are detected and reported for telemetry diagnostics (e.g., GC spikes, background cron jobs), without silently tampering with the raw distribution.
* **Automatic Reliability Warnings**: Ekbasis downgrades the Stability Grade ($A \to D$) and emits actionable warnings whenever variance is excessive ($\text{CV} > 5\%$) or sample counts are insufficient for statistical power.

### 5. CI Cost & Cache Strategy (Same-Runner Isolation)
Why does Ekbasis benchmark both the baseline and candidate on the *same runner*?  
Because comparing a baseline run on Runner A (e.g., an Intel Xeon in Azure) against a candidate on Runner B (an AMD EPYC in AWS) produces fake performance diffs due to hardware variance.

To avoid CI cost explosion:
* **Compiler Caching**: Combine with `actions/cache` and `sccache` (`Swatinem/rust-cache` or language equivalents) to reduce build times from minutes to seconds.
* **Baseline Re-use (`--no-baseline`)**: If your base commit was already benchmarked and preserved in SQLite (`.ekbasis/timeline.db`) or as a GitHub Artifact, pass `--no-baseline` to skip re-measuring the base branch and compare directly against the cached run.
* **Shared Matrix Baselines**: In parameter sweeps, the baseline commit is measured exactly once and shared across all $M$ permutations.

### 6. Crash & Signal Safety (Isolated Worktree Guarantees)
A critical concern in Git automation is: *"What happens if I hit Ctrl+C (SIGINT) mid-run, or if the benchmark process is killed by OOM?"*  
Ekbasis guarantees repository safety through multi-layered isolation:
* **Out-of-Tree Execution**: Benchmarks and mutations never touch your active working directory or `HEAD`. They run strictly in detached temporary worktrees under `.ekbasis/worktrees/<name>`.
* **Self-Healing State & Stale Lock Pruning**: Upon initiation, before any run, and during `ekbasis doctor`, Ekbasis inspects active Git worktree records and automatically prunes stale administrative directories (`git worktree prune`) and orphaned locks. Even if killed abruptly via `SIGKILL`, your main branch, staged files, and uncommitted edits remain 100% untouched.

---

## Installation

Ekbasis is distributed as a single static binary with zero external runtime dependencies (Git is required).

### Quick Install
* **Linux & macOS (Bash)**:
  ```bash
  curl -fsSL https://raw.githubusercontent.com/Yato-Works/Ekbasis/main/install.sh | sh
  ```
* **Windows (PowerShell)**:
  ```powershell
  irm https://raw.githubusercontent.com/Yato-Works/Ekbasis/main/install.ps1 | iex
  ```

### Pre-built Binaries (GitHub Releases)
Download standalone binaries for Linux (x86_64, aarch64), macOS (Apple Silicon, Intel), and Windows from [GitHub Releases](https://github.com/Yato-Works/Ekbasis/releases).

### Install via Cargo
```bash
# Direct install from GitHub
cargo install --git https://github.com/Yato-Works/Ekbasis.git ekbasis-cli --bin ekbasis

# Fast binary install via cargo-binstall
cargo binstall --git https://github.com/Yato-Works/Ekbasis.git ekbasis-cli
```

---

## Quickstart

Navigate to any existing Git repository and start testing counterfactuals in three steps:

```bash
# 1. Initialize Ekbasis in your repository
cd ~/projects/my-service
ekbasis init
# -> Creates .ekbasis/ and registers it in .gitignore and .git/info/exclude

# 2. Scaffold an experiment specification
ekbasis experiment create startup-opt

# 3. Configure mutations and commands
$EDITOR .ekbasis/experiments/startup-opt.yaml

# 4. Run the experiment (Branch -> Mutate -> Build -> Test -> Benchmark -> Compare -> Store)
ekbasis experiment run startup-opt

# 5. Inspect the results and counterfactual timeline
ekbasis experiment list
ekbasis timeline
ekbasis compare startup-opt:baseline startup-opt
```

### Sample Output (`ekbasis experiment run`)

```text
Ekbasis · experiment run startup-opt
---------------------------------------------
base          main (8f4c21a)
branch        ekbasis/startup-opt
iterations    15 measured, 2 warmup, timeout 180s
machine       Linux 6.8.0-generic (x86_64) · AMD EPYC 9R14 (16 vCPUs) · 62.8 GiB RAM

pipeline
  [1/8] base commit       8f4c21a feat: async engine initialization
  [2/8] create branch     ekbasis/startup-opt
  [3/8] worktree          .ekbasis/worktrees/startup-opt
  [4/8] apply changes     1 structured change(s) applied
          config config.toml - runtime.worker_threads: 4 -> 8 (round-trip preserved)
  [5/8] commit            5d2b901 perf(exp): scale worker threads to 8
  [6/8] baseline          measuring 8f4c21a in .ekbasis/worktrees/startup-opt.baseline
          baseline  : 15 samples · mean 182.4 ms · sigma 2.1 ms (1.2%) [ok]
          baseline  : peak memory 32.4 MiB · avg cpu 42.1%
  [7/8] experiment        measuring 5d2b901 in .ekbasis/worktrees/startup-opt
          experiment: 15 samples · mean 112.1 ms · sigma 1.8 ms (1.6%) [ok]
          experiment: peak memory 34.1 MiB · avg cpu 78.4%
  [8/8] compare & store   results indexed in SQLite (.ekbasis/timeline.db)

comparison
  metric                  baseline    experiment  difference          verdict
  ---------------------------------------------------------------------------
  startup (mean)          182.4 ms    112.1 ms    -38.5% (70.3 ms)    improved
  startup (median)        182.1 ms    111.9 ms    -38.5% (70.2 ms)    improved
  startup (p95)           185.0 ms    114.2 ms    -38.3% (70.8 ms)    improved
  build time              4.21 s      4.18 s      -0.7% (30.0 ms)     neutral
  peak memory             32.4 MiB    34.1 MiB    +5.2% (1.7 MiB)     regressed
  avg cpu                 42.1%       78.4%       +86.2% (36.3%)      improved

  statistics
    Welch two-sample t-test: t = -56.42, df = 26.4, p = 1.42e-10 (statistically significant)
    95% Confidence Interval: [-72.9 ms, -67.7 ms] · Cohen's d = 1.85 (large effect)

  correctness
    ok       build - both baseline and experiment states compiled cleanly
    ok       tests - test suites passed on both branches
    ok       iterations - all 15 measured iterations completed

  verdict  4 improved, 1 regressed, 1 neutral (threshold +/-3.0%)
```

---

## Declarative Experiment Specification

Save this under `.ekbasis/experiments/<name>.yaml`:

```yaml
experiment:
  name: startup-opt
  base: main                         # Base git ref to branch from
  description: "tune worker pool size and enable fast memory allocator"
  tags: [performance, runtime]

# First-class structured mutations (safe, comment-preserving, reversible)
changes:
  - type: config                     # AST-level update for TOML, JSON, YAML, or .env
    file: config.toml
    key: runtime.worker_threads
    from: 4                          # Guard: aborts safely if current value differs
    to: 8

  - type: patch                      # Standard Git unified diff
    path: patches/jemalloc-tuning.patch

  - type: file                       # Generate experimental override files
    path: config/experimental.env
    content: "MALLOC_CONF=dirty_decay_ms:0,muzzy_decay_ms:0"

  # Fallback for minor constants:
  # - type: replace
  #   file: src/constants.rs
  #   from: "BUFFER_CAPACITY: usize = 1024;"
  #   to: "BUFFER_CAPACITY: usize = 4096;"

environment:
  RUST_LOG: "warn"

command:
  build: cargo build --release
  test: cargo test --release
  run: ./target/release/server --benchmark-startup  # Target command (zero-overhead execution)

benchmark:
  iterations: 15                     # Measured iterations (15+ recommended for microbenchmarks)
  warmup: 2                          # Warm-up iterations discarded from statistics
  regression_threshold_percent: 3.0  # Threshold for regression verdict

limits:
  timeout_secs: 180                  # Runaway process timeout
  max_output_kb: 64                  # Output truncation threshold
  clean_env: false                   # Run in a sanitized minimal environment
```

Refer to [`docs/experiment-spec.md`](docs/experiment-spec.md) for the complete schema specification.

---

## Advanced Features

### 1. Parameter Matrix Sweeps
Explore parameter spaces in parallel without duplicating specs. The baseline is benchmarked once and shared across all permutations:

```yaml
matrix:
  params:
    threads: [2, 4, 8]
    allocator: ["system", "mimalloc"]

changes:
  - type: config
    file: config.toml
    key: runtime.threads
    to: ${{ matrix.threads }}

environment:
  APP_ALLOCATOR: "${{ matrix.allocator }}"
```

```bash
ekbasis sweep startup-opt
# -> Runs all combinations and renders a sorted leaderboard highlighting the ★ Best configuration
```

### 2. Container Sandboxing (Docker / Podman)
Execute benchmarks inside an isolated, containerized environment to guarantee hermetic repeatability and enforce network isolation (spec: `network: block`, translated to `--network none` by the container engine):

```yaml
limits:
  timeout_secs: 300
  network: block
  container:
    image: "rust:1.85-slim"          # ubuntu:24.04, python:3.12-slim, etc.
```

```bash
ekbasis experiment run startup-opt --container rust:1.85-slim
```

### 3. GitHub Actions PR Regression Gate
Benchmark pull requests by comparing the base commit and head commit on the same CI runner:

```yaml
      - uses: Yato-Works/Ekbasis/.github/actions/ekbasis@v1
        with:
          base-ref: ${{ github.event.pull_request.base.sha }}
          candidate-ref: ${{ github.event.pull_request.head.sha }}
          iterations: '10'
          fail-on-regression: 'true'   # Fails the PR build if performance regresses beyond threshold
```

* Posts and updates a single sticky Markdown comment on the pull request.
* Scaffold CI workflows automatically with `ekbasis ci init`.

### 4. Zero-Dependency HTML Dashboard
Export a standalone, interactive single-file HTML dashboard with historical performance charts:
```bash
ekbasis dashboard --out dashboard.html
```

---

## CLI Reference

| Command | Description |
| :--- | :--- |
| `ekbasis init [--ci]` | Initialize `.ekbasis/` in repository (`--ci` sets up GitHub Actions workflow) |
| `ekbasis experiment create <name> [--matrix]` | Scaffold a new experiment specification (`--matrix` for parameter sweep) |
| `ekbasis experiment show <name>` | Display experiment spec, variables, commands, and historical runs |
| `ekbasis experiment run <name>` | Run full experiment pipeline (`--iterations`, `--warmup`, `--container`, `--no-baseline`) |
| `ekbasis experiment run <name> --candidate <REF>` | **CI Mode**: Benchmark an explicit git ref (e.g. PR head) against base commit |
| `ekbasis experiment sweep <name>` / `ekbasis sweep` | **Matrix Sweep**: Benchmark all parameter combinations and print ranked summary |
| `ekbasis experiment run --all` | Run all specs in `.ekbasis/experiments/` (`--fail-on-regression` for CI gate) |
| `ekbasis experiment list [--all]` | List all experiment specs and latest benchmark results |
| `ekbasis experiment diff <left> <right>` | Diff experiment specifications and benchmark metrics side-by-side |
| `ekbasis compare <left> <right>` | Statistically compare two runs (e.g. `exp:baseline` vs `exp`) |
| `ekbasis report <name...> [--all]` | Generate Markdown or PR comment report (`--format markdown\|comment`) |
| `ekbasis ci init` | Generate GitHub Actions PR automated benchmark workflow |
| `ekbasis timeline [--all]` | Visualize the counterfactual experiment tree and evaluation grades |
| `ekbasis dashboard` | Export interactive zero-dependency HTML dashboard |
| `ekbasis doctor` | Diagnose environment (Git, SQLite, hardware sensors, containers, limits) |

Global options: `-C/--dir <DIR>` (run in target directory), `-v/--verbose` (print all spawned commands and system calls).

---

## Workspace Structure

```text
crates/
├── ekbasis-core/      Shared models, experiment spec parser, AST mutation engine (toml_edit/JSON/YAML)
├── ekbasis-git/       Git CLI driver (worktrees, branches, commits, diffs)
├── ekbasis-runner/    Direct process execution, resource limits, container sandboxing, stream capture
├── ekbasis-observer/  Hardware detection, high-frequency CPU/RAM/GPU/thermal telemetry
├── ekbasis-benchmark/ Inferential statistics (Welch's t-test, CI, IQR outliers), Markdown renderer
├── ekbasis-storage/   SQLite persistence engine (experiments, runs, samples)
└── ekbasis-cli/       `ekbasis` CLI implementation (subcommands, output formatters)

examples/
├── rust-project/      Sample Rust service with config mutation benchmark
└── python-project/    Sample Python application with JSON mutation benchmark

.github/
└── actions/ekbasis/   GitHub Composite Action for automated PR benchmarking
docs/
├── architecture.md    Detailed architectural design (statistical models, telemetry, process trees)
├── experiment-spec.md Complete YAML experiment specification reference
└── github-actions.md  CI integration best practices and caching recipes
tests/
├── e2e.sh             End-to-end integration test suite for Linux, macOS & POSIX environments
└── e2e.ps1            End-to-end integration test suite for Windows PowerShell
install.sh             Automated POSIX installation script
install.ps1            Automated Windows PowerShell installation script
README.md              Primary documentation (English)
README.ja.md           日本語ドキュメント
```

---

## Verification & Testing

Ekbasis is tested continuously across Linux, macOS, and Windows:

```bash
# 1. Run all unit and integration test suites
cargo test --workspace

# 2. Run end-to-end pipeline validation
./tests/e2e.sh          # Linux / macOS (Bash)
pwsh tests/e2e.ps1      # Windows (PowerShell)
```

---

## Migration from Legacy AION

This project was formerly known as `AION` and has been redesigned and rebranded as **`Ekbasis`**.  
For seamless backwards compatibility with existing pipelines:

* **CLI Alias**: The `aion` binary is maintained as an exact alias to `ekbasis`.
* **State Directory**: If an existing `.aion/` directory is detected, Ekbasis reads and writes to it transparently.
* **Git Branch Prefix**: Branches prefixed with `aion/<name>` remain fully tracked and comparable.

---

## License

Ekbasis is released under the [MIT License](LICENSE).
