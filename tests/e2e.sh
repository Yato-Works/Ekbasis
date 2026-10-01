#!/usr/bin/env bash
# Ekbasis end-to-end verification script for Linux, macOS, and POSIX environments.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SANDBOX="${SANDBOX:-$(mktemp -d -t ekbasis-e2e-XXXXXX)}"
KEEP_SANDBOX="${KEEP_SANDBOX:-0}"
EXPERIMENT_NAME="bloom-startup-test"

echo "=========================================="
echo "Ekbasis End-to-End Verification (POSIX)"
echo "Repository : $REPO_ROOT"
echo "Sandbox    : $SANDBOX"
echo "=========================================="

cleanup() {
    if [ "$KEEP_SANDBOX" -eq 0 ]; then
        rm -rf "$SANDBOX"
    fi
}
trap cleanup EXIT

log_step() {
    echo ""
    echo "==> $1"
}

assert() {
    if ! "$@"; then
        echo "Assertion failed: $*" >&2
        exit 1
    fi
}
assert_msg() {
    local msg="$1"
    shift
    if ! "$@"; then
        echo "Assertion failed: $msg" >&2
        exit 1
    fi
    echo "  [ok] $msg"
}

log_step "Building Ekbasis CLI"
(cd "$REPO_ROOT" && cargo build --manifest-path Cargo.toml -p ekbasis-cli)

CLI_BIN="$REPO_ROOT/target/debug/ekbasis"
if [ ! -f "$CLI_BIN" ] && [ -f "$REPO_ROOT/target/debug/ekbasis.exe" ]; then
    CLI_BIN="$REPO_ROOT/target/debug/ekbasis.exe"
fi
assert_msg "Ekbasis CLI binary exists" [ -f "$CLI_BIN" ]

log_step "Preparing sandbox repository"
cp -r "$REPO_ROOT/examples/rust-project/." "$SANDBOX/"
cd "$SANDBOX"

git init -q
git add -A
git -c user.name=E2E -c user.email=e2e@localhost commit -qm "initial bloom demo"
git branch -M main
assert_msg "Repository is on main branch" test "$(git rev-parse --abbrev-ref HEAD)" = "main"

log_step "ekbasis init"
"$CLI_BIN" init
assert_msg ".ekbasis directory created" [ -d .ekbasis ]
assert_msg ".ekbasis/config.yaml written" [ -f .ekbasis/config.yaml ]
assert_msg ".ekbasis/timeline.db created" [ -f .ekbasis/timeline.db ]

log_step "ekbasis experiment create"
"$CLI_BIN" experiment create "$EXPERIMENT_NAME" --from bloom-startup-test.yaml --description "e2e check"
rm -f bloom-startup-test.yaml
assert_msg "Experiment specification stored" [ -f .ekbasis/experiments/$EXPERIMENT_NAME.yaml ]

log_step "ekbasis experiment show"
"$CLI_BIN" experiment show "$EXPERIMENT_NAME"

log_step "ekbasis experiment run"
RUN_OUTPUT="$("$CLI_BIN" experiment run "$EXPERIMENT_NAME" --iterations 3 --warmup 1)"
echo "$RUN_OUTPUT"
assert_msg "Pipeline created experiment branch" grep -q "create branch" <<< "$RUN_OUTPUT"
assert_msg "Pipeline compared and stored results" grep -q "compare & store" <<< "$RUN_OUTPUT"

log_step "Verifying recorded state and artifacts"
git rev-parse --verify "ekbasis/$EXPERIMENT_NAME" >/dev/null 2>&1
assert_msg "Branch ekbasis/$EXPERIMENT_NAME exists" test $? -eq 0
assert_msg "Experiment results saved" [ -f .ekbasis/results/$EXPERIMENT_NAME.experiment.json ]
assert_msg "Baseline results saved" [ -f .ekbasis/results/$EXPERIMENT_NAME.baseline.json ]
assert_msg "Comparison results saved" [ -f .ekbasis/results/$EXPERIMENT_NAME.comparison.json ]

log_step "ekbasis compare"
COMPARE_OUTPUT="$("$CLI_BIN" compare "$EXPERIMENT_NAME:baseline" "$EXPERIMENT_NAME")"
echo "$COMPARE_OUTPUT"
assert_msg "Comparison contains metric" grep -q "startup (mean)" <<< "$COMPARE_OUTPUT"
assert_msg "Comparison reports improvement" grep -q "improved" <<< "$COMPARE_OUTPUT"

log_step "ekbasis experiment list & timeline"
"$CLI_BIN" experiment list --all
"$CLI_BIN" timeline

log_step "ekbasis doctor"
DOCTOR_OUTPUT="$("$CLI_BIN" doctor)"
assert_msg "Doctor reports guard rails" grep -q "guard rails" <<< "$DOCTOR_OUTPUT"

log_step "ekbasis report"
"$CLI_BIN" report "$EXPERIMENT_NAME" --format markdown --out ekbasis-report.md
assert_msg "Markdown report generated" [ -f ekbasis-report.md ]
assert_msg "Report includes Welch t-test details" grep -q "Welch" ekbasis-report.md

log_step "CI mode (--candidate)"
git checkout -q -b feature/faster-startup main
sed -i.bak 's/preload = true/preload = false/' config.toml && rm -f config.toml.bak
git add -A
git -c user.name=E2E -c user.email=e2e@localhost commit -qm "speed up startup"
CI_OUTPUT="$("$CLI_BIN" experiment run "$EXPERIMENT_NAME" --base main --candidate feature/faster-startup --iterations 2 --warmup 0)"
echo "$CI_OUTPUT"
assert_msg "CI mode verified speedup" grep -q "improved" <<< "$CI_OUTPUT"

echo ""
echo "=========================================="
echo "ALL POSIX E2E CHECKS PASSED!"
echo "=========================================="
