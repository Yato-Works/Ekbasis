# GitHub Actions integration

Ekbasis ships as a composite action that turns a pull request into a measured experiment:

```text
Pull Request ──► Ekbasis ──► Baseline (base commit)   ─┐
                         └──► Experiment (PR head)      ├──► Compare ──► PR comment + artifacts
                                                       ─┘
```

The baseline is the **base commit of the pull request**, the experiment side is the **head commit**;
Ekbasis measures both in separate git worktrees with the same commands, iteration count and environment
and reports what it measured. Nothing is predicted — a "regression" is a measured delta, not an
opinion.

## Quick start

1. Add `.github/workflows/ekbasis.yml` (see [`../.github/workflows/ekbasis.yml`](../.github/workflows/ekbasis.yml)).
2. Check out your Ekbasis build next to the project (`path: .ekbasis-toolchain`) and point the action at it.
3. Commit at least one experiment definition to `.ekbasis/experiments/<name>.yaml`.
4. Open a pull request: Ekbasis comments with the comparison table and uploads the artifacts.

The same action works without a pull request (nightly runs, `workflow_dispatch`): leave
`comment: 'false'` and Ekbasis measures the repository as it is.

## Inputs

| input | default | meaning |
| --- | --- | --- |
| `ekbasis-path` | `''` | path of an Ekbasis checkout (the tool). Empty means "use `ekbasis-binary`" |
| `ekbasis-binary` | `''` | prebuilt `ekbasis` binary; when set, Rust is not installed and nothing is built |
| `experiments` | `all` | space separated experiment names, or `all` for every spec in `.ekbasis/experiments` |
| `base-ref` | `''` | ref measured as the baseline; empty falls back to the spec's `experiment.base` |
| `candidate-ref` | `''` | ref measured as the experiment side (`ekbasis experiment run --candidate <ref>`) |
| `iterations` | `5` | measured iterations per side |
| `warmup` | `1` | discarded warm-up iterations per side |
| `fail-on-regression` | `false` | fail the job when the primary metric regressed (`--fail-on-regression`) |
| `comment` | `true` | post or update the pull request comment |
| `artifact-name` | `ekbasis-results` | artifact holding the JSON/Markdown results |
| `retention-days` | `14` | artifact retention |

Outputs: `result` (directory with the results) and `summary` (Markdown written to the step summary).

## What the comment contains

* the headline delta of the primary metric (mean duration) with its verdict,
* the metric table (mean/median/best/p95/trimmed mean, build time) and a collapsible context table
  (peak memory, CPU, GPU, VRAM, temperature),
* the statistical treatment: Welch two-sample t-test, 95% confidence interval, Cohen's d,
* correctness checks (build, tests, iterations) and reliability warnings (variance, outliers,
  "the difference is not statistically significant"),
* the files that changed between the two measured commits,
* reproducibility metadata (both commits, iterations, machine, commands, environment, artifacts).

The comment body carries `<!-- ekbasis-report -->` (or legacy `<!-- aion-report -->`) and is posted with `gh pr comment --edit-last`, so a
new push updates the existing comment instead of spamming the pull request.

## Security

* Ekbasis executes the code of the pull request: isolate it. Run the job only for branches you trust
  (`pull_request` from the same repository), require approval for fork PRs, and keep the job's
  permissions minimal (`contents: read`, `pull-requests: write`).
* The action runs `pull_request` (not `pull_request_target`) and never checks out the PR code into
  a job that holds secrets.
* Ekbasis's own guard rails: every measured state runs in a temporary git worktree,
  commands have a per-command timeout that kills the process tree, output is capped,
  `limits.clean_env` can restrict the environment, `limits.max_processes` caps the process tree,
  `limits.container` isolates execution in Docker / Podman, and `limits.network: block` removes network access.

## Performance and noise

* Runner machines are shared and noisy: use `iterations: 5` or more, keep `warmup` at 1, and read the
  reliability warnings the comparison prints. Ekbasis tells you when a delta is inside the noise.
* Both sides are measured on the same machine in the same job, in alternating single runs, so the
  comparison is a paired one by construction (same commit-independent setup, same commands).
* Cache the Ekbasis toolchain build (`actions/cache` on `<ekbasis-path>/target`, already wired in the
  action) so a job spends its time measuring instead of compiling.

## Troubleshooting

| symptom | cause |
| --- | --- |
| `ekbasis: command not found` | `ekbasis-path` does not point at an Ekbasis checkout and `ekbasis-binary` is empty |
| `cannot resolve base ref` | the checkout used `fetch-depth: 1`; use `fetch-depth: 0` |
| the comment is missing | the workflow lacks `pull-requests: write`, or the event is not a `pull_request` |
| `.ekbasis/` shows up as untracked | `ekbasis init` adds it to `.gitignore` (commit that) and to `.git/info/exclude` (local, immediate) |
| `branch ekbasis/<name> already exists` | a previous run left the branch; Ekbasis refuses to overwrite a recorded future — use `--force` or another experiment name |
| every delta is `n/a` | the spec's `command.run` did not produce samples (check the stored JSON report) |
