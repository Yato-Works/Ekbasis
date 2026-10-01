//! `aion doctor` — what environment did this measurement happen in?

use ekbasis_core::{AionPaths, fmt};
use ekbasis_git::Git;
use ekbasis_observer::collect_machine_info;
use anyhow::{Context as _, Result};

use crate::output;

/// Prints versions, machine metadata and repository state.
pub fn run(verbose: bool) -> Result<()> {
    println!("{}", output::banner("doctor"));

    println!("\nekbasis");
    println!("{}", output::kv("version", ekbasis_core::EKBASIS_VERSION));
    println!(
        "{}",
        output::kv("database lib", format!("sqlite {}", ekbasis_storage::sqlite_version()))
    );
    println!("{}", output::kv("platform", format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)));

    println!("\nmachine");
    let machine = collect_machine_info();
    println!("{}", output::kv("summary", machine.summary()));
    println!(
        "{}",
        output::kv(
            "cpu",
            format!(
                "{} ({} physical / {} logical)",
                machine.cpu_brand.clone().unwrap_or_else(|| "unknown".to_string()),
                machine
                    .cpu_physical_cores
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                machine.cpu_logical_cores
            )
        )
    );
    println!("{}", output::kv("memory", fmt::bytes(machine.total_memory_bytes)));
    println!(
        "{}",
        output::kv(
            "gpus",
            machine
                .gpu_summary()
                .unwrap_or_else(|| "none reported by the operating system".to_string())
        )
    );
    println!(
        "{}",
        output::kv(
            "gpu sampler",
            match ekbasis_observer::GpuSampler::detect() {
                Some(sampler) => format!("{} (live GPU metrics enabled)", sampler.name()),
                None => "none (install nvidia-smi or rocm-smi for live GPU metrics)".to_string(),
            }
        )
    );
    println!(
        "{}",
        output::kv(
            "disk",
            match (machine.disk_free_bytes, machine.disk_total_bytes) {
                (Some(free), Some(total)) => format!(
                    "{} free of {}",
                    fmt::bytes(free),
                    fmt::bytes(total)
                ),
                _ => "not reported".to_string(),
            }
        )
    );

    println!("\nguard rails");
    let limits = ekbasis_core::LimitsSpec::default();
    println!(
        "{}",
        output::kv(
            "timeout",
            format!(
                "{}s per command (limits.timeout_secs; kill after grace)",
                limits.timeout_secs
            )
        )
    );
    println!(
        "{}",
        output::kv(
            "output cap",
            format!("{} KiB per stream (limits.max_output_kb)", limits.max_output_kb)
        )
    );
    println!(
        "{}",
        output::kv(
            "process limit",
            "off unless limits.max_processes is set; enforced while running (tree killed on breach)"
        )
    );
    println!(
        "{}",
        output::kv(
            "network block",
            match ekbasis_runner::network_block_support() {
                Ok(()) => "supported here (limits.network: block uses `unshare -Urn`)".to_string(),
                Err(reason) => format!("NOT supported: {reason}"),
            }
        )
    );
    println!(
        "{}",
        output::kv(
            "env whitelist",
            format!(
                "supported (limits.clean_env: {})",
                if limits.clean_env { "on" } else { "off" }
            )
        )
    );
    println!(
        "{}",
        output::kv(
            "container",
            match ekbasis_runner::detect_container_engine() {
                Some(engine) => {
                    format!("available ({engine} detected; enables portable network & filesystem sandboxing)")
                }
                None => "not detected (install docker or podman for container sandboxing)".to_string(),
            }
        )
    );

    println!("\ngit");
    let cwd = std::env::current_dir().context("cannot read the current directory")?;
    println!("{}", output::kv("working dir", cwd.display()));
    match Git::discover(&cwd) {
        Ok(git) => {
            println!("{}", output::kv("repository", git.root().display()));
            match git.head_branch() {
                Ok(Some(branch)) => println!("{}", output::kv("branch", branch)),
                Ok(None) => println!("{}", output::kv("branch", "detached HEAD")),
                Err(error) => println!("{}", output::kv("branch", format!("unknown ({error})"))),
            }
            match git.commit_info("HEAD") {
                Ok(info) => println!(
                    "{}",
                    output::kv(
                        "head",
                        format!(
                            "{} {} ({}, {})",
                            fmt::short_sha(&info.sha),
                            fmt::truncate_line(&info.subject, 40),
                            info.author,
                            info.date
                        )
                    )
                ),
                Err(error) => println!("{}", output::kv("head", format!("unknown ({error})"))),
            }
            println!(
                "{}",
                output::kv(
                    "worktree",
                    if git.is_dirty().unwrap_or(false) {
                        "dirty (uncommitted changes)"
                    } else {
                        "clean"
                    }
                )
            );
            match git.local_branches() {
                Ok(branches) => {
                    let exp_branches: Vec<String> = branches
                        .into_iter()
                        .filter(|branch| branch.starts_with("ekbasis/") || branch.starts_with("aion/"))
                        .collect();
                    println!(
                        "{}",
                        output::kv(
                            "experiment branches",
                            if exp_branches.is_empty() {
                                "none yet".to_string()
                            } else {
                                exp_branches.join(", ")
                            }
                        )
                    );
                }
                Err(error) => println!("{}", output::kv("branches", format!("unknown ({error})"))),
            }
        }
        Err(error) => println!("{}", output::kv("repository", format!("none ({error})"))),
    }

    println!("\nekbasis state");
    if let Ok(paths) = AionPaths::discover(&cwd) {
        println!(
            "{}",
            output::kv("state dir", paths.state_dir().display().to_string())
        );
        println!("{}", output::kv("config", paths.rel(&paths.config_path())));
        match ekbasis_storage::Store::open(&paths.db_path()) {
            Ok(store) => println!(
                "{}",
                output::kv(
                    "database",
                    format!(
                        "schema v{} · {} experiments · {} runs · {} samples",
                        store.schema_version()?.unwrap_or(0),
                        store.experiment_count()?,
                        store.run_count()?,
                        store.sample_count()?
                    )
                )
            ),
            Err(error) => println!("{}", output::kv("database", format!("unavailable ({error})"))),
        }
    } else {
        println!("{}", output::kv("state dir", "not initialised (run `ekbasis init`)"));
    }

    if verbose {
        println!("\nverbose");
        println!("  AION_GIT_EXE={}", std::env::var("AION_GIT_EXE").unwrap_or_else(|_| "git".to_string()));
        println!("  COMSPEC={}", std::env::var("COMSPEC").unwrap_or_else(|_| "<unset>".to_string()));
        println!("  PATH entries: {}", std::env::var("PATH").map(|path| path.split(';').count()).unwrap_or(0));
    }

    Ok(())
}
