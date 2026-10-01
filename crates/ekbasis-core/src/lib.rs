//! Ekbasis core: the shared vocabulary of the whole system.
//!
//! Everything in Ekbasis is expressed with the types of this crate:
//!
//! * [`ExperimentSpec`] — what an experiment *is* (“if this change had been made…”).
//! * [`RunReport`] — what actually happened when a software state was built, run and measured.
//! * [`EkbasisPaths`] / [`AionPaths`] — where Ekbasis keeps its experiments, results and database.
//! * [`EkbasisConfig`] / [`AionConfig`] — the repository-local configuration.
//!
//! Nothing in this crate predicts anything: it only describes states, changes and observations.

pub mod changes;
pub mod config;
pub mod fmt;
pub mod model;
pub mod paths;
pub mod spec;

pub use changes::{ChangeOutcome, apply_changes};
pub use config::{
    AionConfig, AionMeta, DefaultsConfig, EkbasisConfig, EkbasisMeta, GitConfig, ObserverConfig,
};
pub use model::{
    GpuInfo, MachineInfo, PhaseKind, PhaseReport, ReproInfo, RunKind, RunReport,
    RuntimeObservation, SampleRecord, Stats, TestSummary,
};
pub use paths::{AionPaths, EkbasisPaths};
pub use spec::{
    BenchmarkSpec, Change, CommandSpec, ContainerSpec, ExperimentMeta, ExperimentSpec, LimitsSpec,
    MatrixSpec, NetworkPolicy,
};

/// Version of the Ekbasis implementation that produced a given report.
pub const EKBASIS_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Legacy alias for version constant.
pub const AION_VERSION: &str = EKBASIS_VERSION;

/// Branch prefix reserved for alternative timelines, unless configured otherwise.
pub const DEFAULT_BRANCH_PREFIX: &str = "ekbasis/";

/// Primary name of the Ekbasis state directory inside a repository.
pub const EKBASIS_DIR_NAME: &str = ".ekbasis";

/// Legacy name of the AION state directory inside a repository for backwards compatibility.
pub const AION_DIR_NAME: &str = ".aion";

/// Current UTC time as an RFC 3339 string with second precision.
///
/// Every Ekbasis timestamp (config, reports, database rows) uses this format so that
/// results remain comparable and sortable as plain text.
pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
