//! SQLite storage for experiments, runs and samples (plan §8).
//!
//! Full reports live as JSON files under `.aion/results/`; the database indexes them so that
//! `aion experiment list`, `aion compare` and `aion timeline` stay instant and keep working
//! long after an experiment finished.

use std::path::Path;

use ekbasis_core::model::{RunKind, RunReport};
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

/// Columns selected by [`RUN_SELECT`], in the order [`map_run_row`] expects them.
const RUN_SELECT: &str = "SELECT r.id, r.experiment_id, e.name, r.kind, r.label, r.branch,
       r.commit_sha, r.base_commit, r.started_at, r.finished_at, r.success, r.iterations,
       r.warmup, r.build_ms, r.setup_ms, r.test_success, r.mean_ms, r.median_ms, r.min_ms,
       r.max_ms, r.stddev_ms, r.cv_percent, r.peak_memory_bytes, r.avg_cpu_percent,
       r.report_path, r.machine
FROM runs r JOIN experiments e ON e.id = r.experiment_id";

/// Maps one joined `runs` + `experiments` row.
fn map_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunRow> {
    let kind_text: String = row.get(3)?;
    Ok(RunRow {
        id: row.get(0)?,
        experiment_id: row.get(1)?,
        experiment_name: row.get(2)?,
        kind: RunKind::parse(&kind_text).unwrap_or(RunKind::Experiment),
        label: row.get(4)?,
        branch: row.get(5)?,
        commit_sha: row.get(6)?,
        base_commit: row.get(7)?,
        started_at: row.get(8)?,
        finished_at: row.get(9)?,
        success: row.get::<_, i64>(10)? != 0,
        iterations: row.get::<_, i64>(11)? as u32,
        warmup: row.get::<_, i64>(12)? as u32,
        build_ms: row.get(13)?,
        setup_ms: row.get(14)?,
        test_success: row.get::<_, Option<i64>>(15)?.map(|value| value != 0),
        mean_ms: row.get(16)?,
        median_ms: row.get(17)?,
        min_ms: row.get(18)?,
        max_ms: row.get(19)?,
        stddev_ms: row.get(20)?,
        cv_percent: row.get(21)?,
        peak_memory_bytes: row.get::<_, Option<i64>>(22)?.map(|value| value as u64),
        avg_cpu_percent: row.get(23)?,
        report_path: row.get(24)?,
        machine: row.get(25)?,
    })
}

/// Maps one `experiments` row.
fn map_experiment_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExperimentRow> {
    Ok(ExperimentRow {
        id: row.get(0)?,
        name: row.get(1)?,
        spec_path: row.get(2)?,
        base_ref: row.get(3)?,
        base_commit: row.get(4)?,
        description: row.get(5)?,
        created_at: row.get(6)?,
    })
}

/// One point of an experiment's delta history: how the future measured against the baseline
/// *recorded at that time*. Ordered oldest first by [`Store::delta_history`].
#[derive(Debug, Clone)]
pub struct HistoryPoint {
    pub run_id: i64,
    pub started_at: String,
    /// Mean duration of this experiment run (when it produced samples).
    pub mean_ms: Option<f64>,
    /// Difference against the newest baseline recorded before this run.
    pub delta_percent: Option<f64>,
    pub success: bool,
    pub cv_percent: Option<f64>,
}

impl HistoryPoint {
    /// Classifies the delta against a regression threshold: `improved` / `regressed` / `neutral`.
    pub fn trend(&self, threshold_percent: f64) -> Option<&'static str> {
        let delta = self.delta_percent?;
        if delta <= -threshold_percent {
            Some("improved")
        } else if delta >= threshold_percent {
            Some("regressed")
        } else {
            Some("neutral")
        }
    }
}

/// One experiment with all of its runs, used by `aion experiment list` and `aion timeline`.
#[derive(Debug, Clone)]
pub struct ExperimentSummary {
    pub experiment: ExperimentRow,
    /// Runs newest first.
    pub runs: Vec<RunRow>,
    pub latest_baseline: Option<RunRow>,
    pub latest_experiment: Option<RunRow>,
    /// Difference of the newest experiment run against the newest baseline run.
    pub delta_percent: Option<f64>,
}

impl ExperimentSummary {
    /// Most recent run of any kind.
    pub fn last_run(&self) -> Option<&RunRow> {
        self.runs.first()
    }

    /// Number of measured runs.
    pub fn measured_runs(&self) -> usize {
        self.runs.iter().filter(|run| run.is_measured()).count()
    }
}

/// Schema version written into the `meta` table.
const SCHEMA_VERSION: i64 = 1;

/// Version of the SQLite library AION is linked against.
pub fn sqlite_version() -> String {
    rusqlite::version().to_string()
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS experiments (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT NOT NULL UNIQUE,
    spec_path   TEXT NOT NULL,
    base_ref    TEXT,
    base_commit TEXT,
    description TEXT,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS runs (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    experiment_id     INTEGER NOT NULL REFERENCES experiments(id) ON DELETE CASCADE,
    kind              TEXT NOT NULL,
    label             TEXT NOT NULL,
    branch            TEXT,
    commit_sha        TEXT NOT NULL,
    base_commit       TEXT NOT NULL,
    started_at        TEXT NOT NULL,
    finished_at       TEXT NOT NULL,
    success           INTEGER NOT NULL,
    iterations        INTEGER NOT NULL,
    warmup            INTEGER NOT NULL DEFAULT 0,
    build_ms          REAL,
    setup_ms          REAL,
    test_success      INTEGER,
    mean_ms           REAL,
    median_ms         REAL,
    min_ms            REAL,
    max_ms            REAL,
    stddev_ms         REAL,
    cv_percent        REAL,
    peak_memory_bytes INTEGER,
    avg_cpu_percent   REAL,
    report_path       TEXT,
    machine           TEXT
);

CREATE TABLE IF NOT EXISTS samples (
    run_id      INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    iteration   INTEGER NOT NULL,
    duration_ms REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_runs_experiment ON runs(experiment_id, kind, id);
"#;

/// Row of the `experiments` table.
#[derive(Debug, Clone)]
pub struct ExperimentRow {
    pub id: i64,
    pub name: String,
    pub spec_path: String,
    pub base_ref: Option<String>,
    pub base_commit: Option<String>,
    pub description: Option<String>,
    pub created_at: String,
}

/// Row of the `runs` table.
#[derive(Debug, Clone)]
pub struct RunRow {
    pub id: i64,
    pub experiment_id: i64,
    pub experiment_name: String,
    pub kind: RunKind,
    pub label: String,
    pub branch: Option<String>,
    pub commit_sha: String,
    pub base_commit: String,
    pub started_at: String,
    pub finished_at: String,
    pub success: bool,
    pub iterations: u32,
    pub warmup: u32,
    pub build_ms: Option<f64>,
    pub setup_ms: Option<f64>,
    pub test_success: Option<bool>,
    pub mean_ms: Option<f64>,
    pub median_ms: Option<f64>,
    pub min_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub stddev_ms: Option<f64>,
    pub cv_percent: Option<f64>,
    pub peak_memory_bytes: Option<u64>,
    pub avg_cpu_percent: Option<f64>,
    pub report_path: Option<String>,
    pub machine: Option<String>,
}

impl RunRow {
    /// One-line summary of the measured duration.
    pub fn summary(&self) -> String {
        match self.mean_ms {
            Some(mean) => format!("mean {}", ekbasis_core::fmt::duration_ms(mean)),
            None => "no samples".to_string(),
        }
    }

    /// True when the run has a measured duration.
    pub fn is_measured(&self) -> bool {
        self.mean_ms.is_some()
    }

    /// Percentage difference against a baseline mean, when both exist.
    pub fn delta_percent(&self, baseline_mean_ms: Option<f64>) -> Option<f64> {
        match (self.mean_ms, baseline_mean_ms) {
            (Some(candidate), Some(baseline)) if baseline.abs() > f64::EPSILON => {
                Some((candidate - baseline) / baseline * 100.0)
            }
            _ => None,
        }
    }
}

/// Everything needed to persist one run.
#[derive(Debug, Clone)]
pub struct RunRecord {
    pub experiment_id: i64,
    pub kind: RunKind,
    pub label: String,
    pub branch: Option<String>,
    pub commit_sha: String,
    pub base_commit: String,
    pub started_at: String,
    pub finished_at: String,
    pub success: bool,
    pub iterations: u32,
    pub warmup: u32,
    pub build_ms: Option<f64>,
    pub setup_ms: Option<f64>,
    pub test_success: Option<bool>,
    pub mean_ms: Option<f64>,
    pub median_ms: Option<f64>,
    pub min_ms: Option<f64>,
    pub max_ms: Option<f64>,
    pub stddev_ms: Option<f64>,
    pub cv_percent: Option<f64>,
    pub peak_memory_bytes: Option<u64>,
    pub avg_cpu_percent: Option<f64>,
    pub report_path: Option<String>,
    pub machine: Option<String>,
    pub samples: Vec<f64>,
}

impl RunRecord {
    /// Derives a database record from a finished report.
    pub fn from_report(
        experiment_id: i64,
        report: &RunReport,
        report_path: Option<String>,
    ) -> RunRecord {
        let stats = report.stats;
        RunRecord {
            experiment_id,
            kind: report.kind,
            label: report.label.clone(),
            branch: report.branch.clone(),
            commit_sha: report.commit.clone(),
            base_commit: report.base_commit.clone(),
            started_at: report.started_at.clone(),
            finished_at: report.finished_at.clone(),
            success: report.success,
            iterations: report.iterations,
            warmup: report.warmup,
            build_ms: report.build_ms,
            setup_ms: report.setup_ms,
            test_success: report.test.as_ref().map(|test| test.success),
            mean_ms: stats.map(|stats| stats.mean),
            median_ms: stats.map(|stats| stats.median),
            min_ms: stats.map(|stats| stats.min),
            max_ms: stats.map(|stats| stats.max),
            stddev_ms: stats.map(|stats| stats.stddev),
            cv_percent: stats.map(|stats| stats.cv_percent),
            peak_memory_bytes: report.peak_memory_bytes(),
            avg_cpu_percent: report.avg_cpu_percent(),
            report_path,
            machine: Some(report.machine.summary()),
            samples: report
                .samples
                .iter()
                .map(|sample| sample.duration_ms)
                .collect(),
        }
    }
}

/// The AION experiment database.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Opens (and creates, when needed) the database at `path`.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create `{}`", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("cannot open the AION database `{}`", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .context("cannot enable WAL mode")?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .context("cannot enable foreign keys")?;
        let store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Creates the schema (idempotent) and records the schema version.
    fn migrate(&self) -> Result<()> {
        self.conn
            .execute_batch(SCHEMA)
            .context("cannot create the AION database schema")?;
        self.conn
            .execute(
                "INSERT INTO meta(key, value) VALUES('schema_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![SCHEMA_VERSION.to_string()],
            )
            .context("cannot record the schema version")?;
        Ok(())
    }

    /// Schema version stored in the database.
    pub fn schema_version(&self) -> Result<Option<i64>> {
        let value: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(value.and_then(|value| value.parse().ok()))
    }

    /// Number of stored experiments.
    pub fn experiment_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM experiments", [], |row| row.get(0))?)
    }

    /// Number of stored runs.
    pub fn run_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0))?)
    }

    /// Number of stored samples.
    pub fn sample_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM samples", [], |row| row.get(0))?)
    }

    /// Inserts or updates an experiment definition, returning its id.
    pub fn upsert_experiment(
        &self,
        name: &str,
        spec_path: &str,
        base_ref: Option<&str>,
        description: Option<&str>,
        base_commit: Option<&str>,
    ) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO experiments(name, spec_path, base_ref, description, created_at, base_commit)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(name) DO UPDATE SET
                     spec_path = excluded.spec_path,
                     base_ref = COALESCE(excluded.base_ref, experiments.base_ref),
                     description = COALESCE(excluded.description, experiments.description),
                     base_commit = COALESCE(excluded.base_commit, experiments.base_commit)",
                params![
                    name,
                    spec_path,
                    base_ref,
                    description,
                    ekbasis_core::now_rfc3339(),
                    base_commit
                ],
            )
            .with_context(|| format!("cannot store experiment `{name}`"))?;
        let id: i64 = self.conn.query_row(
            "SELECT id FROM experiments WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// Records the commit an experiment branched from (resolved on the first run).
    pub fn set_experiment_base_commit(&self, experiment_id: i64, base_commit: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE experiments SET base_commit = ?2 WHERE id = ?1",
            params![experiment_id, base_commit],
        )?;
        Ok(())
    }

    /// Records a description without touching other columns.
    pub fn set_experiment_description(
        &self,
        experiment_id: i64,
        description: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE experiments SET description = ?2 WHERE id = ?1",
            params![experiment_id, description],
        )?;
        Ok(())
    }

    /// Inserts a finished run plus its samples, returning the run id.
    pub fn insert_run(&self, record: &RunRecord) -> Result<i64> {
        self.conn
            .execute(
                "INSERT INTO runs(
                    experiment_id, kind, label, branch, commit_sha, base_commit, started_at,
                    finished_at, success, iterations, warmup, build_ms, setup_ms, test_success,
                    mean_ms, median_ms, min_ms, max_ms, stddev_ms, cv_percent, peak_memory_bytes,
                    avg_cpu_percent, report_path, machine
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                           ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
                params![
                    record.experiment_id,
                    record.kind.as_str(),
                    record.label,
                    record.branch,
                    record.commit_sha,
                    record.base_commit,
                    record.started_at,
                    record.finished_at,
                    record.success as i32,
                    record.iterations as i64,
                    record.warmup as i64,
                    record.build_ms,
                    record.setup_ms,
                    record.test_success.map(|flag| flag as i32),
                    record.mean_ms,
                    record.median_ms,
                    record.min_ms,
                    record.max_ms,
                    record.stddev_ms,
                    record.cv_percent,
                    record.peak_memory_bytes.map(|value| value as i64),
                    record.avg_cpu_percent,
                    record.report_path,
                    record.machine,
                ],
            )
            .with_context(|| format!("cannot store run `{}`", record.label))?;
        let run_id = self.conn.last_insert_rowid();
        for (index, duration) in record.samples.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO samples(run_id, iteration, duration_ms) VALUES(?1, ?2, ?3)",
                params![run_id, (index + 1) as i64, duration],
            )?;
        }
        Ok(run_id)
    }

    // ---- reading -----------------------------------------------------------

    /// All experiments, newest first.
    pub fn list_experiments(&self) -> Result<Vec<ExperimentRow>> {
        let mut statement = self.conn.prepare(
            "SELECT id, name, spec_path, base_ref, base_commit, description, created_at
             FROM experiments ORDER BY created_at DESC, id DESC",
        )?;
        let rows = statement.query_map([], map_experiment_row)?;
        let mut experiments = Vec::new();
        for row in rows {
            experiments.push(row?);
        }
        Ok(experiments)
    }

    /// One experiment by name.
    pub fn experiment_by_name(&self, name: &str) -> Result<Option<ExperimentRow>> {
        let mut statement = self.conn.prepare(
            "SELECT id, name, spec_path, base_ref, base_commit, description, created_at
             FROM experiments WHERE name = ?1",
        )?;
        Ok(statement.query_row(params![name], map_experiment_row).optional()?)
    }

    /// All runs of one experiment, newest first.
    pub fn runs_for_experiment(&self, experiment_id: i64) -> Result<Vec<RunRow>> {
        let sql = format!("{RUN_SELECT} WHERE r.experiment_id = ?1 ORDER BY r.id DESC");
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params![experiment_id], map_run_row)?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    }

    /// Every run, newest first. `limit = None` returns all of them.
    pub fn all_runs(&self, limit: Option<usize>) -> Result<Vec<RunRow>> {
        let sql = format!("{RUN_SELECT} ORDER BY r.id DESC LIMIT ?1");
        let mut statement = self.conn.prepare(&sql)?;
        let bound = limit.map(|value| value as i64).unwrap_or(-1);
        let rows = statement.query_map(params![bound], map_run_row)?;
        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    }

    /// One run by database id.
    pub fn run_by_id(&self, id: i64) -> Result<Option<RunRow>> {
        let sql = format!("{RUN_SELECT} WHERE r.id = ?1");
        let mut statement = self.conn.prepare(&sql)?;
        Ok(statement.query_row(params![id], map_run_row).optional()?)
    }

    /// Newest run of one experiment and kind.
    pub fn latest_run(&self, experiment_id: i64, kind: RunKind) -> Result<Option<RunRow>> {
        let sql = format!(
            "{RUN_SELECT} WHERE r.experiment_id = ?1 AND r.kind = ?2 ORDER BY r.id DESC LIMIT 1"
        );
        let mut statement = self.conn.prepare(&sql)?;
        Ok(statement
            .query_row(params![experiment_id, kind.as_str()], map_run_row)
            .optional()?)
    }

    /// Measured durations of one run, in iteration order.
    pub fn samples_for_run(&self, run_id: i64) -> Result<Vec<f64>> {
        let mut statement = self
            .conn
            .prepare("SELECT duration_ms FROM samples WHERE run_id = ?1 ORDER BY iteration")?;
        let rows = statement.query_map(params![run_id], |row| row.get::<_, f64>(0))?;
        let mut samples = Vec::new();
        for row in rows {
            samples.push(row?);
        }
        Ok(samples)
    }

    /// Experiments together with their runs, newest first (`list`, `show`, `timeline`).
    pub fn experiment_summaries(&self) -> Result<Vec<ExperimentSummary>> {
        let experiments = self.list_experiments()?;
        let runs = self.all_runs(None)?;

        let mut summaries: Vec<ExperimentSummary> = experiments
            .into_iter()
            .map(|experiment| ExperimentSummary {
                experiment,
                runs: Vec::new(),
                latest_baseline: None,
                latest_experiment: None,
                delta_percent: None,
            })
            .collect();

        let mut positions = std::collections::HashMap::new();
        for (position, summary) in summaries.iter().enumerate() {
            positions.insert(summary.experiment.id, position);
        }

        for run in runs {
            let Some(position) = positions.get(&run.experiment_id).copied() else {
                continue;
            };
            let summary = &mut summaries[position];
            match run.kind {
                RunKind::Baseline => {
                    if summary.latest_baseline.is_none() {
                        summary.latest_baseline = Some(run.clone());
                    }
                }
                RunKind::Experiment => {
                    if summary.latest_experiment.is_none() {
                        summary.latest_experiment = Some(run.clone());
                    }
                }
            }
            summary.runs.push(run);
        }

        for summary in summaries.iter_mut() {
            let baseline_mean = summary
                .latest_baseline
                .as_ref()
                .and_then(|run| run.mean_ms);
            summary.delta_percent = summary
                .latest_experiment
                .as_ref()
                .and_then(|run| run.delta_percent(baseline_mean));
        }

        Ok(summaries)
    }

    /// Delta history of one experiment, oldest first (plan §19: trend across runs).
    ///
    /// Every experiment run is compared against the newest *baseline recorded before it*, so a
    /// re-baselined experiment (moved base commit, changed threshold) still yields an honest
    /// per-run delta instead of one number measured against the final baseline only. Experiment
    /// runs with no baseline yet are skipped: there is nothing to compare them to.
    pub fn delta_history(&self, experiment_id: i64) -> Result<Vec<HistoryPoint>> {
        let mut runs = self.runs_for_experiment(experiment_id)?;
        runs.reverse(); // oldest first for a time series

        let mut history = Vec::new();
        let mut latest_baseline_mean: Option<f64> = None;
        for run in runs {
            match run.kind {
                RunKind::Baseline => {
                    if let Some(mean) = run.mean_ms {
                        latest_baseline_mean = Some(mean);
                    }
                }
                RunKind::Experiment => {
                    // No baseline recorded yet: there is nothing to compute a delta against, so
                    // the run is not a history point (its report still lives in `aion compare`).
                    let Some(delta) = run.delta_percent(latest_baseline_mean) else {
                        continue;
                    };
                    history.push(HistoryPoint {
                        run_id: run.id,
                        started_at: run.started_at,
                        mean_ms: run.mean_ms,
                        delta_percent: Some(delta),
                        success: run.success,
                        cv_percent: run.cv_percent,
                    });
                }
            }
        }
        Ok(history)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One stored run with the fields the history cares about.
    fn record(
        experiment_id: i64,
        kind: RunKind,
        label: &str,
        started_at: &str,
        mean_ms: Option<f64>,
    ) -> RunRecord {
        RunRecord {
            experiment_id,
            kind,
            label: label.to_string(),
            branch: None,
            commit_sha: "c".repeat(40),
            base_commit: "b".repeat(40),
            started_at: started_at.to_string(),
            finished_at: started_at.to_string(),
            success: mean_ms.is_some(),
            iterations: 3,
            warmup: 1,
            build_ms: None,
            setup_ms: None,
            test_success: None,
            mean_ms,
            median_ms: mean_ms,
            min_ms: mean_ms,
            max_ms: mean_ms,
            stddev_ms: mean_ms.map(|_| 1.0),
            cv_percent: mean_ms.map(|_| 2.0),
            peak_memory_bytes: None,
            avg_cpu_percent: None,
            report_path: None,
            machine: None,
            samples: mean_ms.into_iter().collect(),
        }
    }

    /// Opens a store at a path unique to the calling test (tests run in parallel threads but
    /// share one process id, so the test name has to be part of the file name).
    fn store(test: &str) -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "aion-storage-test-{}-{test}.db",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let store = Store::open(&path).expect("open store");
        (store, path)
    }

    #[test]
    fn delta_history_compares_each_run_with_the_baseline_of_its_time() {
        let (store, path) = store("history");
        let id = store
            .upsert_experiment("demo", ".aion/experiments/demo.yaml", Some("main"), None, None)
            .expect("experiment");

        // baseline 100ms -> experiment 80ms (-20%) -> new baseline 90ms -> experiment 120ms (+33%)
        store.insert_run(&record(id, RunKind::Baseline, "b1", "2026-01-01", Some(100.0))).unwrap();
        store.insert_run(&record(id, RunKind::Experiment, "e1", "2026-01-02", Some(80.0))).unwrap();
        store.insert_run(&record(id, RunKind::Baseline, "b2", "2026-01-03", Some(90.0))).unwrap();
        store.insert_run(&record(id, RunKind::Experiment, "e2", "2026-01-04", Some(120.0))).unwrap();

        let history = store.delta_history(id).expect("history");
        assert_eq!(history.len(), 2, "baselines are not history points");
        assert!((history[0].delta_percent.expect("first delta") - (-20.0)).abs() < 1e-9);
        // The second run must use *its* baseline (90ms): 120 vs 90 = +33.3%.
        assert!((history[1].delta_percent.expect("second delta") - 100.0 * 3.0 / 9.0).abs() < 1e-9);

        assert_eq!(history[0].trend(5.0), Some("improved"));
        assert_eq!(history[1].trend(5.0), Some("regressed"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn experiment_runs_without_any_baseline_have_no_history_point() {
        let (store, path) = store("no-baseline");
        let id = store
            .upsert_experiment("demo", ".aion/experiments/demo.yaml", Some("main"), None, None)
            .expect("experiment");
        store.insert_run(&record(id, RunKind::Experiment, "e1", "2026-01-01", Some(80.0))).unwrap();
        let history = store.delta_history(id).expect("history");
        assert!(history.is_empty(), "no baseline means no delta: {history:?}");
        let _ = std::fs::remove_file(&path);
    }
}
