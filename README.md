# Ekbasis — Software Timeline & Counterfactual Experiment Engine

[![CI](https://github.com/Yato-Works/Ekbasis/actions/workflows/ekbasis.yml/badge.svg)](https://github.com/Yato-Works/Ekbasis/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.85%2B-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/Platform-Linux%20%7C%20macOS%20%7C%20Windows-lightgrey.svg)]()

> **Git tells you how your software changed. Ekbasis lets you test what it could become.**
>
> Gitはソフトウェアが「どう変わったか」の事実を記録する。  
> **Ekbasis**（エグバシス / 古代ギリシャ語: *ἔκβασις* — 出口・帰結・結果）は、そこから「もしこの変更を加えていたらどうなっていたか？」という**反事実（Counterfactual）**を推測せず、実際に分岐・実行・観測・統計検証する実験エンジンです。

```text
main (Commit 8f4c21a)
 │
 ├── [Production Baseline] ────────── run: ./target/release/server ──── 182.4 ms (cv 1.2%)
 │
 ├── [Branch: ekbasis/opt-threads] ─── worker_threads: 4 -> 8 ───────── 112.1 ms (-38.5%, p < 0.001) ★ Best
 │
 └── [Branch: ekbasis/simd-patch] ──── patch: simd-vectorize.patch ──── 141.7 ms (-22.3%, p = 0.004)
```

**Prediction ❌ 「たぶん速くなるはず」 → Execution ✅ 「同一マシン・同一条件下で実行し、Welchのt検定（p < 0.001）により有意に38.5%高速化を実証」**

---

## なぜ Ekbasis なのか？（Why Ekbasis?）

「手動で `git worktree` を切り、シェルスクリプトでビルドして `hyperfine` で測ればいいのでは？」と思うかもしれません。  
しかし、現実のソフトウェア最適化や性能評価には、**シェルスクリプトや単体ベンチマークツールでは決して解決できない壁**が存在します。

| 観点・機能 | 自作シェルスクリプト | `hyperfine` 単体 | **Ekbasis** |
| :--- | :---: | :---: | :--- |
| **反事実ブランチ（Git連動）** | ⚠️ 手動でworktree作成・復元が必要。失敗時に作業ツリーが汚染 | ❌ 非対応（Gitの概念なし） | **✅ 完全自動。独立worktree作成 → 変更適用 → コミット → 計測 → 自動掃除** |
| **構造化ミューテーション** | ❌ `sed` や文字列置換はフォーマット崩れで壊れる | ❌ 非対応 | **✅ TOML/JSON/YAML/.env の型安全書き換え・Patch適用・事前値ガード** |
| **シェルの測定ノイズ排除** | ❌ `sh -c` や `cmd.exe` の起動コスト・引数展開が測定値に混入 | ⚠️ 指定コマンドをシェル経由で起動するためオーバーヘッド混入 | **✅ OS APIによる直接プロセス起動（Direct Process Execution）＋子プロセスツリー追跡** |
| **統計的有意性の数学的証明** | ❌ 平均値だけの比較で、ノイズか真の改善か判断不能 | ⚠️ 基本統計量（平均・標準偏差）のみ | **✅ Welchのt検定・p値・95%信頼区間・Cohen's d効果量・外れ値（IQR）自動除外** |
| **システムテレメトリ観測** | ❌ 外部モニタを手動監視 | ❌ 実行時間のみ | **✅ CPU/メモリピーク/GPU VRAM/温度をミリ秒単位でサンプリング記録** |
| **実験台帳の永続化** | ❌ 端末ログが流れて過去の検証が消失 | ❌ その場のコンソール出力のみ | **✅ SQLite (`timeline.db`) に全試行・全サンプル・コミットハッシュを恒久記録** |
| **PR性能回帰テスト & CI** | ⚠️ 泥臭いCIスクリプトの自作とメンテが必要 | ❌ 外部連携機能なし | **✅ 同一ランナー比較・Sticky PRコメント自動更新・性能回帰ガード（Gate CI）** |

### 1. ゼロオーバーヘッドの直接プロセス起動
シェル（`bash` や `cmd.exe`）を介してプロセスを起動すると、シェルの初期化コストや環境変数のパース時間が計測値に混ざり、ミリ秒単位のベンチマークは容易に歪みます。  
Ekbasis は **OS のシステムコールで対象バイナリを直接起動**し、プロセスのPIDツリー全体を深層追跡。真の実行時間・ピークメモリ・CPU使用率・GPU負荷を高精度にサンプリングします。

### 2. 統計学に基づく有意差判定（Welch's t-test）
「3%速くなった」は、CPUのサーマルスロットリングやOSのバックグラウンド処理による偶然（ノイズ）かもしれません。  
Ekbasis は不等分散を前提とした **Welchのt検定（Welch's two-sample t-test）** を実行し、p値（有意水準）と95%信頼区間を算出。「測定回数が足りない」「分散が大きすぎて結論を出せない」といった警告を自動提示します。

### 3. 反事実的タイムライン（Counterfactual Timeline）
Gitが管理するのは「メインストリームとして採用された過去の歴史」だけです。Ekbasis は「試されたが無駄だったパラメータ」「採用されなかった変更案」も含め、**探索したパラレルワールドの全実験データを SQLite に台帳として記録**。いつでも過去の実験と現在の実装を diff / compare できます。

---

## インストール（Installation）

Ekbasis はシングルバイナリで動作し、外部ランタイム依存はありません（Git のみ必要です）。

### ワンライナー導入（Linux / macOS）
```bash
curl -fsSL https://raw.githubusercontent.com/Yato-Works/Ekbasis/main/install.sh | sh
```

### Pre-built Binaries (GitHub Releases)
Linux (x86_64, aarch64), macOS (Apple Silicon, Intel), Windows 向けのビルド済みバイナリを [GitHub Releases](https://github.com/Yato-Works/Ekbasis/releases) から直接ダウンロードできます。

### Cargo からのインストール
```bash
# GitHub リポジトリから直接インストール
cargo install --git https://github.com/Yato-Works/Ekbasis.git ekbasis-cli --bin ekbasis

# 高速バイナリ展開ツール cargo-binstall を利用する場合
cargo binstall --git https://github.com/Yato-Works/Ekbasis.git ekbasis-cli
```

---

## クイックスタート

お使いのプロジェクト（任意の Git リポジトリ）に移動して、わずか数ステップで実験を開始できます。

```bash
# 1. あなたのプロジェクトで Ekbasis を初期化
cd ~/projects/my-service
ekbasis init
# → .ekbasis/ を作成し、.gitignore と .git/info/exclude に自動登録

# 2. 実験スペックを作成（テンプレートが生成されます）
ekbasis experiment create startup-optimization

# 3. 変更内容・ビルド・計測コマンドを設定
$EDITOR .ekbasis/experiments/startup-optimization.yaml

# 4. 実験を実行（Branch → Mutate → Build → Test → Benchmark → Compare → Store）
ekbasis experiment run startup-optimization

# 5. 比較結果と探索タイムラインを確認
ekbasis experiment list
ekbasis timeline
ekbasis compare startup-optimization:baseline startup-optimization
```

### 実行ログの例 (`ekbasis experiment run`)

```text
Ekbasis · experiment run startup-optimization
---------------------------------------------
base          main (8f4c21a)
branch        ekbasis/startup-optimization
iterations    5 measured, 1 warmup, timeout 180s
machine       Linux 6.8.0-generic (x86_64) · AMD EPYC 9R14 (16 vCPUs) · 62.8 GiB RAM

pipeline
  [1/8] base commit       8f4c21a feat: async engine initialization
  [2/8] create branch     ekbasis/startup-optimization
  [3/8] worktree          .ekbasis/worktrees/startup-optimization
  [4/8] apply changes     1 structured change(s) applied
          config config.toml - runtime.worker_threads: 4 -> 8
  [5/8] commit            5d2b901 perf(exp): increase worker threads to 8
  [6/8] baseline          measuring 8f4c21a in .ekbasis/worktrees/startup-optimization.baseline
          baseline  : 5 samples · mean 182.4 ms · sigma 2.1 ms (1.2%) [ok]
          baseline  : peak memory 32.4 MiB · avg cpu 42.1%
  [7/8] experiment        measuring 5d2b901 in .ekbasis/worktrees/startup-optimization
          experiment: 5 samples · mean 112.1 ms · sigma 1.8 ms (1.6%) [ok]
          experiment: peak memory 34.1 MiB · avg cpu 78.4%
  [8/8] compare & store   results saved to SQLite (.ekbasis/timeline.db)

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
    Welch two-sample t-test: t = -56.42, df = 7.8, p = 1.42e-10 (statistically significant)
    95% Confidence Interval: [-72.9 ms, -67.7 ms] · Cohen's d = 35.7 (huge effect)

  correctness
    ok       build - both baseline and experiment states compiled cleanly
    ok       tests - test suites passed on both states
    ok       iterations - all 5 measured iterations completed

  verdict  4 improved, 1 regressed, 1 neutral (threshold +/-3.0%)
```

---

## 実験定義（`.ekbasis/experiments/<name>.yaml`）

Ekbasis は、壊れやすい生の文字列置換に頼りません。**AST・フォーマットを保持する構造化設定書き換え**や、**クリーンな Git パッチ適用**を第一級のミューテーション機構として提供します。

```yaml
experiment:
  name: startup-optimization
  base: main                         # 分岐元の ref / commit
  description: "tune worker pool size and enable fast memory allocator"
  tags: [performance, runtime]

# 堅牢な構造化変更（フォーマット崩れを起こさず、型安全に適用）
changes:
  - type: config                     # TOML / JSON / YAML / .env を解釈して書き換え
    file: config.toml
    key: runtime.worker_threads
    from: 4                          # ガード: 現在値が4でなければ実験を安全に中止
    to: 8

  - type: patch                      # Git unified diff を安全に適用
    path: patches/jemalloc-tuning.patch

  - type: file                       # 実験用オーバーライド設定の生成
    path: config/experimental.env
    content: "MALLOC_CONF=dirty_decay_ms:0,muzzy_decay_ms:0"

  # （補助フォールバック: 小さな定数変更用の文字列置換もサポート）
  # - type: replace
  #   file: src/constants.rs
  #   from: "BUFFER_CAPACITY: usize = 1024;"
  #   to: "BUFFER_CAPACITY: usize = 4096;"

environment:
  RUST_LOG: "warn"

command:
  build: cargo build --release
  test: cargo test --release
  run: ./target/release/server       # ここが計測対象（シェルを介さず直接起動）

benchmark:
  iterations: 5                      # 計測試行回数
  warmup: 1                          # ウォームアップ回数（統計から除外）
  regression_threshold_percent: 3.0  # 有意差とみなすパーセント閾値

limits:
  timeout_secs: 180                  # 暴走防止タイムアウト
  max_output_kb: 64                  # ログ肥大化防止
  clean_env: false                   # 汚染のないクリーン環境で実行するか
```

詳細な仕様は [`docs/experiment-spec.md`](docs/experiment-spec.md) を参照してください。

---

## 高度な機能

### 1. パラメータスイープ・マトリクス実験（Matrix Sweeps）
1つの実験スペックからパラメータの直積を展開し、全組み合わせを自動計測・ランキング比較します。
ベースラインの計測は1度だけ実行されて全バリアントで共有されるため、極めて効率的です。

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
ekbasis sweep startup-optimization
# → 全組み合わせの実行完了後、改善率順のランキング表（★ Best configuration）を表示
```

### 2. コンテナサンドボックス分離（Container Runner）
Docker / Podman を利用し、ホスト環境を一切汚さずに隔離されたコンテナ内でビルド・テスト・計測を実行します。ネットワーク遮断（`--network none`）と完全な環境再現性が保証されます。

```yaml
limits:
  timeout_secs: 300
  network: block
  container:
    image: "rust:1.85-slim"          # または ubuntu:24.04, python:3.12-slim など
```

```bash
ekbasis experiment run startup-optimization --container rust:1.85-slim
```

### 3. GitHub Actions 連携 & PR 回帰ゲート（Gate CI）
PR の base commit と candidate (head) commit を**同一ランナー上で両方計測**し、結果を PR コメントとして自動投稿します。

```yaml
      - uses: Yato-Works/Ekbasis/.github/actions/ekbasis@main
        with:
          base-ref: ${{ github.event.pull_request.base.sha }}
          candidate-ref: ${{ github.event.pull_request.head.sha }}
          iterations: '5'
          fail-on-regression: 'true'   # 性能回帰（閾値超過）を検知した場合にPRビルドを失敗させる
```

* コメントは sticky マーカー付きで自動更新されるため、コミットごとにPRコメントが乱立しません。
* `ekbasis ci init` を実行するだけで、ワークフローファイルが自動生成されます。

### 4. 自己完結型 HTML ダッシュボード
外部サーバーや依存関係なしに、ブラウザで直接閲覧できるリッチな HTML ダッシュボードを即座に生成します。
```bash
ekbasis dashboard --out dashboard.html
```

---

## CLI リファレンス

| コマンド | 説明 |
| :--- | :--- |
| `ekbasis init [--ci]` | リポジトリに `.ekbasis/` を作成（`--ci` で GitHub Actions ワークフローも自動セットアップ） |
| `ekbasis experiment create <name> [--matrix]` | 実験スペックを作成（`--matrix` でパラメータスイープ用テンプレートを生成） |
| `ekbasis experiment show <name>` | スペック定義・コマンド・環境変数・記録済み run 一覧を表示 |
| `ekbasis experiment run <name>` | パイプライン実行（`--iterations` / `--warmup` / `--container <IMG>` / `--env K=V`） |
| `ekbasis experiment run <name> --candidate <REF>` | **CI モード**: 指定した git ref（PR の head など）を変更前と同一条件で比較計測 |
| `ekbasis experiment sweep <name>` / `ekbasis sweep` | **マトリクススイープ**: パラメータ直積を展開し全バリアントを自動計測・ランキング比較 |
| `ekbasis experiment run --all` | `.ekbasis/experiments/` 内の全実験を一括実行（`--fail-on-regression` でゲート判定） |
| `ekbasis experiment list [--all]` | 実験スペックおよび最新の計測結果サマリー一覧 |
| `ekbasis experiment diff <left> <right>` | 2つの実験の**スペック差分**（設定・環境）と**計測結果差分**を表示 |
| `ekbasis compare <left> <right>` | 2つの run または `name:baseline` と `name` を統計比較（`--threshold` 指定可能） |
| `ekbasis report <name...> [--all]` | Markdown / PRコメント用レポート生成（`--format markdown\|comment`） |
| `ekbasis ci init` | **CI 導入**: GitHub Actions PR 自動ベンチマークワークフローを自動生成 |
| `ekbasis timeline [--all]` | これまで探索した反事実的タイムライン（実験ツリー・成績評価）を表示 |
| `ekbasis dashboard` | ゼロ依存・単一ファイルの HTML ダッシュボードを生成 |
| `ekbasis doctor` | システム診断（Git、SQLite、CPU/メモリ/GPU、コンテナランナー、ガードレールの状態） |

グローバルオプション: `-C/--dir <DIR>`（対象ディレクトリ指定）、`-v/--verbose`（実行したシステムコールやGitコマンドの詳細出力）。

---

## リポジトリ構成

```text
crates/
├── ekbasis-core/      共有モデル・実験仕様・設定・構造化変更エンジン（TOML/JSON/YAML/.env）
├── ekbasis-git/       Git CLI ラッパー（worktree / branch / commit / diff）
├── ekbasis-runner/    直接プロセス起動・リソース制限・コンテナ分離・出力キャプチャ
├── ekbasis-observer/  マシン診断・CPU/メモリ/GPU/温度の高精度サンプリング
├── ekbasis-benchmark/ 統計検定エンジン（Welch's t-test / CI / 外れ値除外）と Markdown レンダラ
├── ekbasis-storage/   SQLite 永続化エンジン（experiments / runs / samples）
└── ekbasis-cli/       `ekbasis` CLI コマンド実装（init / run / compare / report / timeline / doctor）

examples/
├── rust-project/      Rust 製デモプロジェクト（config.toml を書き換える実験同梱）
└── python-project/    Python 製デモプロジェクト（config.json を書き換える実験同梱）

.github/
└── actions/ekbasis/   PR 自動計測 & コメント投稿用 GitHub Composite Action
docs/
├── architecture.md    アーキテクチャ詳細・統計/テレメトリ/セキュリティ設計
├── experiment-spec.md 実験定義仕様リファレンス
└── github-actions.md  CI 連携と運用ベストプラクティス
tests/
├── e2e.sh             Linux / macOS / POSIX 向けエンドツーエンド検証スクリプト
└── e2e.ps1            Windows (PowerShell) 向けエンドツーエンド検証スクリプト
```

---

## 検証（Testing & Verification）

Ekbasis は Linux、macOS、Windows のクロスプラットフォームでテストされています。

```bash
# 1. ユニットテストおよび統合テスト（全クレート・統計検定・変更エンジン）
cargo test --workspace

# 2. クロスプラットフォーム E2E パイプライン検証
./tests/e2e.sh          # Linux / macOS (Bash)
pwsh tests/e2e.ps1      # Windows (PowerShell)
```

---

## Migration from Legacy AION (後方互換性)

本プロジェクトは旧名 `AION` から **`Ekbasis`** へ正式刷新されました。  
過去のスクリプトや CI 資産との互換性を保つため、以下のエイリアス機能がビルトインされています：

* **CLI エイリアス**: `aion` コマンドが `ekbasis` と全く同じ挙動で動作します。
* **設定ディレクトリ**: `.aion/` ディレクトリが存在する場合、自動的に後方互換モードで認識・読み書きされます。
* **Git プレフィックス**: 過去に作成された `aion/<name>` ブランチもそのまま追跡・比較可能です。

---

## ライセンス

本プロジェクトは [MIT License](LICENSE) の下で公開されています。
