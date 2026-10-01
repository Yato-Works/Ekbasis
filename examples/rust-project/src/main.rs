//! AION example project — a tiny service whose startup cost is driven by `config.toml`.
//!
//! `startup.preload` makes the service burn `preload_entries * preload_entry_ms` milliseconds on
//! a synthetic warm-up cache before it reports ready. The bundled AION experiment switches that
//! off and *measures* what happens instead of guessing.
//!
//! The program deliberately has no dependencies, so it can be dropped into any scratch
//! repository and used as an experiment target.

use std::process::ExitCode;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let started = Instant::now();
    let config = match Config::load("config.toml") {
        Ok(config) => config,
        Err(error) => {
            eprintln!("bloom: cannot read config.toml: {error}");
            return ExitCode::FAILURE;
        }
    };

    if config.startup.preload {
        burn(config.startup.preload_entries * config.startup.preload_entry_ms);
    }
    burn(config.startup.init_ms);

    println!(
        "bloom ready in {:.3} ms (preload: {}, threads: {}, cache: {} MiB)",
        started.elapsed().as_secs_f64() * 1000.0,
        config.startup.preload,
        config.worker.threads,
        config.cache.size_mb
    );
    ExitCode::SUCCESS
}

/// Busy work instead of sleeping, so an observer sees real CPU time and memory traffic.
fn burn(milliseconds: u64) {
    let budget = Duration::from_millis(milliseconds);
    let started = Instant::now();
    let mut acc: u64 = 0;
    while started.elapsed() < budget {
        for value in 0..20_000u64 {
            acc = acc.wrapping_add(value.wrapping_mul(2_654_435_761));
        }
        std::hint::black_box(acc);
    }
}

/// The slice of `config.toml` this example cares about.
struct Config {
    startup: Startup,
    worker: Worker,
    cache: Cache,
}

struct Startup {
    preload: bool,
    preload_entries: u64,
    preload_entry_ms: u64,
    init_ms: u64,
}

struct Worker {
    threads: u64,
}

struct Cache {
    size_mb: u64,
}

impl Config {
    /// Reads the handful of keys this example needs (deliberately dependency-free).
    fn load(path: &str) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let mut config = Config {
            startup: Startup {
                preload: true,
                preload_entries: 30,
                preload_entry_ms: 4,
                init_ms: 60,
            },
            worker: Worker { threads: 4 },
            cache: Cache { size_mb: 256 },
        };

        let mut section = String::from("startup");
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') {
                section = line.trim_matches(|c| c == '[' || c == ']').trim().to_string();
                continue;
            }
            let Some((key, raw)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let raw = raw.split('#').next().unwrap_or("").trim();
            let number = || raw.trim_matches('"').trim().parse::<u64>().unwrap_or(0);
            match (section.as_str(), key) {
                ("startup", "preload") => config.startup.preload = raw.eq_ignore_ascii_case("true"),
                ("startup", "preload_entries") => config.startup.preload_entries = number(),
                ("startup", "preload_entry_ms") => config.startup.preload_entry_ms = number(),
                ("startup", "init_ms") => config.startup.init_ms = number(),
                ("worker", "threads") => config.worker.threads = number(),
                ("cache", "size_mb") => config.cache.size_mb = number(),
                _ => {}
            }
        }
        Ok(config)
    }
}
