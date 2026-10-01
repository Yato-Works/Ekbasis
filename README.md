# Ekbasis — Software Timeline & Counterfactual Experiment Engine

> Git tells you how your software changed. **Ekbasis lets you test what it could become.**
>
> Gitはソフトウェアがどう変わったかを記録する。Ekbasis（エグバシス / ἔκβασις: 出口・帰結・結果）は、そこから「どうなり得たか」を実際に試す。

Ekbasis（旧プロジェクト名: AION）は「もしこの変更を入れていたら？」を**推測せず、実行して観測する**ローカル実験エンジンです。
Gitリポジトリを時間軸として扱い、あるコミットから実験用ブランチを切り、宣言した変更を適用し、
ビルド・実行・計測して、変更前（baseline）と同じ条件で取った計測結果と比較します。

> [!NOTE]
> **名称変更と互換性について**:
> プロジェクト名を `AION` から **`Ekbasis`** へ正式刷新しました。
> 既存のスクリプトや環境との後方互換性を保つため、バイナリ名 `aion`、旧設定ディレクトリ `.aion/`、旧ブランチプレフィックス `aion/` も完全なエイリアスとして引き続きサポートされます。

```text
main
 │
 │  current implementation
 │
 ├──── Ekbasis Experiment #001     baseline: 2.840 s
 │       ├─ FFT_SIZE=2048          experiment: 2.310 s   -18.7%
 │       ├─ Threads=8
 │       └─ Compiler=O3
 │
 └──── Ekbasis Experiment #002     baseline: 2.840 s
         ├─ FFT_SIZE=1024          experiment: 3.180 s   +12.0%
         ├─ Threads=4
         └─ Compiler=O2
```

**Prediction ❌ 「たぶん速くなる」 → Execution ✅ 「実際に実行した結果、12.4%速くなった」**

---

## Ekbasis がやらないこと

* ❌ 未来を予測するAIではない（推測は一切しません。行うのは branch / apply / build / run / measure / compare）
* ❌ Gitの代替ではない（Git = Source History、Ekbasis = Experimental History の二層構造）
* ❌ 自動でコードを最適化するツールではない（変更はあなたがYAMLで宣言します）
* ❌ 結果の正しさを保証しない（計測値・分散・失敗をそのまま記録します）

代わりに Ekbasis は、**変更によって生じる可能性のある状態を、実際に分岐・実行・計測する**システムです。

---

## クイックスタート

```bash
# 0. ビルド（Rust 1.85+ / git / rusqlite の bundled SQLite 用に C コンパイラ）
cargo build --release
# できあがるバイナリ: target/release/ekbasis(.exe) および互換用 target/release/aion(.exe)

# 1. 計測したいプロジェクト（git リポジトリ）で Ekbasis を導入
cd ~/projects/my-service
ekbasis init
# → .ekbasis/ を作成し、.gitignore（コミット推奨）と .git/info/exclude（ローカル即時）に .ekbasis/ を登録

# 2. 実験を定義（コメント付きテンプレートが生成される）
ekbasis experiment create startup-preload-off

# 3. 変更・コマンド・計測条件を編集
$EDITOR .ekbasis/experiments/startup-preload-off.yaml

# 4. 実行: branch → change → build → test → benchmark → compare → store
ekbasis experiment run startup-preload-off

# 5. 結果を見る
ekbasis experiment list
ekbasis timeline
ekbasis compare startup-preload-off:baseline startup-preload-off
```

`ekbasis experiment run` の出力例:

```text
Ekbasis · experiment run startup-preload-off
-------------------------------------------
base          main (8f4c21a)
branch        ekbasis/startup-preload-off
iterations    5 measured, 1 warmup, timeout 180s
machine       Windows 11 (26200) · 11th Gen Intel(R) Core(TM) i7-11700 · 8/16 cores · 15.9 GiB

pipeline
  [1/8] base commit       8f4c21a feat: hot reload
  [2/8] create branch     ekbasis/startup-preload-off
  [3/8] worktree          .ekbasis/worktrees/startup-preload-off
  [4/8] apply changes     1
          config config.toml - startup.preload: true -> false
  [5/8] commit            5d2b901
  [6/8] baseline          measuring 8f4c21a in .ekbasis/worktrees/startup-preload-off.baseline
          baseline  : 5 samples · mean 182.4 ms · sigma 3.1 ms (1.7%) [ok]
  [7/8] experiment        measuring 5d2b901 in .ekbasis/worktrees/startup-preload-off
          experiment: 5 samples · mean 62.1 ms · sigma 1.2 ms (1.9%) [ok]
  [8/8] compare & store

comparison
  metric            baseline    experiment  difference          verdict
  startup (mean)    182.4 ms    62.1 ms     -65.9% (120.3 ms)   improved
  startup (median)  182.1 ms    61.9 ms     -66.0% (120.2 ms)   improved
  peak memory       3.1 MiB     3.0 MiB     -3.2% (102.4 KiB)   improved
  correctness
    ok       build - both states compiled
    ok       tests - baseline and experiment test suites passed
  verdict  3 improved, 0 regressed, 2 neutral (threshold +/-3.0%)
```

---

## CLI

| command | 説明 |
| --- | --- |
| `ekbasis init [--ci]` | リポジトリに `.ekbasis/` を作成（`--ci` で GitHub Actions ワークフローも自動セットアップ） |
| `ekbasis experiment create <name> [--matrix]` | 実験定義を作成（`--matrix` でパラメータスイープ用テンプレートを生成） |
| `ekbasis experiment show <name>` | 定義内容・コマンド・環境変数・記録済み run 一覧を表示（`--reports` でJSON全文） |
| `ekbasis experiment run <name>` | パイプライン実行（`--iterations` / `--warmup` / `--container <IMAGE>` / `--env K=V` / `--no-baseline` 等） |
| `ekbasis experiment run <name> --candidate <REF>` | **CI モード**: 指定した ref（PR の head など）をそのまま実験側として計測（`--base` と併用） |
| `ekbasis experiment sweep <name>` / `ekbasis sweep` | **マトリクススイープ**: パラメータ直積を展開し全バリアントを自動計測・ランキング比較 |
| `ekbasis experiment run --all` | `.ekbasis/experiments/` の全実験をまとめて実行（CI 向け、`--fail-on-regression` でゲート可能） |
| `ekbasis experiment list [--all]` | 実験と最新結果の一覧 |
| `ekbasis experiment diff <left> <right>` | 2つの実験の**定義差分**（change/command/env/benchmark/limits）と**計測差分**＋changed files |
| `ekbasis compare <left> <right>` | 2つの run を比較（run id / 実験名 / `name:baseline` / JSONファイル、`--threshold`） |
| `ekbasis report <name...> [--all]` | Markdown レポート生成（`--format markdown\|comment`、`--out FILE`）。run 直後にも `.ekbasis/results/<name>.report.md` を自動生成 |
| `ekbasis ci init` | **CI 導入**: GitHub Actions PR 自動ベンチマークワークフロー（`.github/workflows/ekbasis.yml`）を生成 |
| `ekbasis timeline [--all] [--limit N]` | 探索した未来（実験ツリー）を表示 |
| `ekbasis dashboard` | 自己完結型 HTML ダッシュボードを生成（ゼロ依存・ブラウザで直接閲覧可能） |
| `ekbasis doctor` | Ekbasis・git・SQLite・マシン・GPU・ディスク・コンテナランナー・`.ekbasis/` の状態を表示 |

*※ `aion` コマンドも上記すべてのサブコマンドのエイリアスとして完全動作します。*

グローバルオプション: `-C/--dir <DIR>`（別ディレクトリで実行）、`-v/--verbose`（実行した git / シェルコマンドを全て表示）。

---

## 実験定義（`.ekbasis/experiments/<name>.yaml`）

```yaml
experiment:
  name: bloom-startup-test
  base: main                         # 分岐元の ref / commit
  description: "skip the cache preload"
  hypothesis: "startup gets faster"  # 記録のみ。結果の予測には使われません
  tags: [startup]

changes:
  - type: config                     # TOML / JSON / YAML / .env のキーを書き換え
    file: config.toml
    key: startup.preload
    from: true                       # ガード: 現在値が違えば実験を中止
    to: false
  - type: replace                    # ソースコードの文字列置換
    file: src/main.rs
    from: "CACHE_SIZE = 1024"
    to: "CACHE_SIZE = 4096"
  - type: file                       # ファイル全体を書き出す
    path: config/alt.yaml
    content: "mode: aggressive"

environment:
  THREADS: "8"

command:
  build: cargo build --release
  test: cargo test --release
  run: target/release/app.exe        # ここが計測対象

benchmark:
  iterations: 5
  warmup: 1
  regression_threshold_percent: 3.0

limits:
  timeout_secs: 180
  max_output_kb: 64
  clean_env: false
```

詳細は [`docs/experiment-spec.md`](docs/experiment-spec.md) を参照してください。

---

## パラメータスイープ・マトリクス実験（Matrix Sweeps）

1つの実験定義から複数のパラメータ組み合わせを展開し、一括でベンチマーク＆ランキング比較できます。

```yaml
experiment:
  name: cache-threads-tuning
  base: main

matrix:
  params:
    threads: [2, 4, 8]
    mode: ["normal", "aggressive"]

changes:
  - type: replace
    file: src/main.rs
    from: "THREADS = 1"
    to: "THREADS = ${{ matrix.threads }}"

environment:
  RUN_MODE: "${{ matrix.mode }}"
  RAYON_NUM_THREADS: "${{ matrix.threads }}"
```

* **共通ベースラインの再利用**: ベースコミットの計測は最初の1回だけ実行され、全バリアントで共有されるため高速です。
* **ランキング出力**: 全組み合わせの実行完了後、改善率順にソートされたサマリー表と最良設定（★ Best configuration）を表示します。

```bash
ekbasis experiment sweep cache-threads-tuning
# または
ekbasis sweep cache-threads-tuning
```

---

## コンテナサンドボックス分離（Container Runner）

Docker / Podman を利用し、ホスト環境を汚さずに隔離されたコンテナ内でビルド・テスト・計測を実行します。
Windows や macOS でも完全なネットワーク遮断（`--network none`）と環境再現性が保証されます。

```yaml
limits:
  timeout_secs: 300
  network: block
  container:
    image: "rust:1.85-slim"     # python:3.12-slim や ubuntu:24.04 など
    # engine: "docker"          # docker / podman を明示（省略時は自動検出）
```

CLI から即座に指定することも可能です：
```bash
ekbasis experiment run startup-test --container rust:1.85-slim
```

---

## GitHub Actions 連携

PR の base commit と head commit を**同じランナー上で両方計測**し、結果を PR コメントとして自動投稿します。

```yaml
      - uses: ./.ekbasis-toolchain/.github/actions/ekbasis
        with:
          ekbasis-path: .ekbasis-toolchain
          base-ref: ${{ github.event.pull_request.base.sha }}
          candidate-ref: ${{ github.event.pull_request.head.sha }}
          iterations: '5'
          fail-on-regression: 'false'
```

* コメントは sticky マーカー付きで `gh pr comment --edit-last` により**更新**されます（push ごとに増えません）。
* 結果 JSON / Markdown は artifact として保存され、ジョブの step summary にも出力されます。
* `fail-on-regression: 'true'` で、主指標が閾値を超えて悪化した PR を**失敗（Gate CI）**にできます。
* `ekbasis ci init` または `ekbasis init --ci` でワークフローファイルをリポジトリへ即座にセットアップできます。
* 詳細と注意点は [`docs/github-actions.md`](docs/github-actions.md) を参照してください。

---

## パイプライン（`ekbasis experiment run`）

```text
Base Commit → Create Branch(ekbasis/<name>) → Worktree → Apply Changes → Commit
            → Baseline 計測（base commit を detached worktree に checkout）
            → Experiment 計測（変更済みブランチ）
            → Compare → JSON + SQLite に保存 → worktree を削除（ブランチは残す）
```

* **Baseline と Experiment は同じコマンド・同じ環境変数・同じ iterations で計測**します。
* 各フェーズの stdout / stderr / exit code / 実行時間 / 出力の切り捨て有無を保存します。
* 実行中のピークメモリ・平均CPU・プロセス数も記録します（`--no-observe` で無効化）。
* ビルド失敗・テスト失敗・タイムアウトは隠さず記録し、終了コードも非ゼロにします（CI向け）。
* 計測対象のコマンドは、シェルメタ文字が無ければ**シェルを介さず直接起動**し、観測対象のPIDを正確に取り、`cmd /C` の起動コストを計測に混ぜません。

## 保存されるもの

```text
.ekbasis/                              # （既存リポジトリでは .aion/ も自動認識）
├── config.yaml                        # Ekbasis の設定（branch prefix など）
├── experiments/<name>.yaml            # 実験定義
├── results/<name>.baseline.json       # baseline の全記録（stdout/stderr 含む）
├── results/<name>.experiment.json     # 実験側の全記録
├── results/<name>.comparison.json     # 比較結果（metric / verdict / warnings）
├── worktrees/                         # 実行中の一時 worktree（既定では終了時に削除）
└── timeline.db                        # SQLite: experiments / runs / samples
```

## リポジトリ構成

```text
crates/
├── ekbasis-core/      共有モデル・実験仕様・設定・変更適用（TOML/JSON/YAML/.env 編集）
├── ekbasis-git/       git CLI ラッパ（branch / worktree / commit / diff）
├── ekbasis-runner/    プロセス実行（タイムアウト・出力キャプチャ・環境ホワイトリスト・観測）
├── ekbasis-observer/  マシン情報（CPU/RAM/GPU/ディスク）と CPU/メモリ/GPU/温度のサンプリング
├── ekbasis-benchmark/ 統計（Welch t検定・CI・外れ値・グラフ）と比較・Markdown レンダラ
├── ekbasis-storage/   SQLite（experiments / runs / samples）
└── ekbasis-cli/       `ekbasis` / `aion` コマンド（init / experiment / compare / report / timeline / doctor）
examples/
├── rust-project/      Rust 製デモ（config.toml を書き換える実験付き）
└── python-project/    Python 製デモ（config.json を書き換える実験付き）
.github/
├── actions/ekbasis/   composite action（PR を base vs head で計測してコメント）
├── actions/aion/      legacy composite action エイリアス
└── workflows/         ekbasis.yml / aion.yml
docs/
├── architecture.md    構成・パイプライン・統計/GPU/CI の設計
├── experiment-spec.md 実験定義のリファレンス
└── github-actions.md  GitHub 連携の使い方・注意点
tests/e2e.ps1          E2E 検証スクリプト（一時リポジトリで全機能を実行）
```

## 検証

```bash
cargo test                   # ユニット/統合テスト（統計の数値検証、GPUパーサ、変更エンジン、E2E）
pwsh -File tests/e2e.ps1     # E2E: init→create→run→compare→report→diff→CIモード→回帰ゲート
```

## ライセンス

MIT — [`LICENSE`](LICENSE) を参照してください。
