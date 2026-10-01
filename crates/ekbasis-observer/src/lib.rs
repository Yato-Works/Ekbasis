//! Hardware and runtime observation (plan §6, §17).
//!
//! Three jobs:
//!
//! * [`collect_machine_info_for`] — a static description of the machine a run happened on
//!   (OS, CPU, memory, GPUs, disk space).
//! * [`ObservationHandle`] — samples CPU, memory, temperature and GPU usage of a process tree
//!   while it runs.
//! * [`gpu`] — best-effort GPU support: `nvidia-smi` / `rocm-smi` for live numbers, an operating
//!   system query for the adapter list.
//!
//! The observer is deliberately conservative: when information cannot be obtained (very short
//! lived processes, no drivers, unsupported platform) it reports *nothing* instead of guessing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ekbasis_core::model::{MachineInfo, RuntimeObservation};
use sysinfo::{Components, Disks, Pid, ProcessesToUpdate, System};

pub mod gpu;

pub use gpu::{GpuSample, GpuSampler, detect_gpu_info};

/// Lower bound for sampling: shorter intervals produce meaningless CPU percentages.
const MIN_SAMPLE_MS: u64 = 100;

/// GPU and temperature readings are expensive (they start helper processes or query the firmware),
/// so they run on their own cadence instead of on every CPU/memory tick.
const SLOW_SAMPLE_INTERVAL: Duration = Duration::from_millis(250);

/// Sampling configuration.
#[derive(Debug, Clone, Copy)]
pub struct ObserveOptions {
    /// Requested interval between samples, in milliseconds.
    pub interval_ms: u64,
}

impl Default for ObserveOptions {
    fn default() -> Self {
        ObserveOptions { interval_ms: 250 }
    }
}

impl ObserveOptions {
    fn interval(&self) -> Duration {
        Duration::from_millis(self.interval_ms.max(MIN_SAMPLE_MS))
    }
}

/// Collects static machine metadata for reproducibility records.
///
/// `checkout` is the directory whose volume is reported: a benchmark that runs out of disk space
/// is measuring something other than the software under test.
pub fn collect_machine_info_for(checkout: &Path) -> MachineInfo {
    let mut system = System::new();
    system.refresh_memory();
    system.refresh_cpu_all();

    let cpu_brand = system
        .cpus()
        .first()
        .map(|cpu| cpu.brand().trim().to_string())
        .filter(|brand| !brand.is_empty());
    let (disk_free_bytes, disk_total_bytes) = disk_usage(checkout);

    MachineInfo {
        os: System::name().unwrap_or_else(|| std::env::consts::OS.to_string()),
        // `os_version()` yields e.g. `11 Home`, `long_os_version()` e.g. `Windows 11 Home`;
        // the shorter form composes better with `os` in `MachineInfo::summary`.
        os_version: System::os_version().or_else(System::long_os_version),
        kernel_version: System::kernel_version(),
        hostname: System::host_name(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_brand,
        cpu_physical_cores: System::physical_core_count(),
        cpu_logical_cores: system.cpus().len(),
        total_memory_bytes: system.total_memory(),
        gpus: detect_gpu_info(),
        disk_free_bytes,
        disk_total_bytes,
    }
}

/// Machine metadata for the current working directory.
pub fn collect_machine_info() -> MachineInfo {
    let checkout = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    collect_machine_info_for(&checkout)
}

/// Free and total space of the volume that holds `path`, when the platform reports it.
fn disk_usage(path: &Path) -> (Option<u64>, Option<u64>) {
    let disks = Disks::new_with_refreshed_list();
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut best: Option<(usize, u64, u64)> = None;
    for disk in disks.list() {
        let mount = disk.mount_point();
        if !canonical.starts_with(mount) {
            continue;
        }
        // Prefer the most specific mount point: nested volumes win over the root volume.
        let depth = mount.components().count();
        let better = best
            .map(|(best_depth, _, _)| depth > best_depth)
            .unwrap_or(true);
        if better {
            best = Some((depth, disk.available_space(), disk.total_space()));
        }
    }
    match best {
        Some((_, free, total)) => (Some(free), Some(total)),
        None => (None, None),
    }
}

/// A sampling thread attached to a running process tree.
pub struct ObservationHandle {
    stop: Arc<AtomicBool>,
    samples: Option<JoinHandle<RuntimeObservation>>,
    environment: Option<JoinHandle<RuntimeObservation>>,
}

impl ObservationHandle {
    /// Starts sampling `pid` and every process below it.
    ///
    /// Two threads are used: one for the cheap process counters (CPU, memory) and one for the
    /// expensive GPU/temperature readings. A vendor tool that takes a second to answer must never
    /// stall the counters, otherwise a short command would end up with no CPU/memory data at all.
    pub fn start(pid: u32, options: ObserveOptions) -> ObservationHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let counter_stop = Arc::clone(&stop);
        let environment_stop = Arc::clone(&stop);
        let interval = options.interval();
        let samples = thread::spawn(move || sample_tree(pid, interval, counter_stop));
        let environment =
            thread::spawn(move || sample_environment(environment_stop, SLOW_SAMPLE_INTERVAL));
        ObservationHandle {
            stop,
            samples: Some(samples),
            environment: Some(environment),
        }
    }

    /// Stops sampling and returns everything observed so far.
    pub fn stop(mut self) -> RuntimeObservation {
        self.finish()
    }

    fn finish(&mut self) -> RuntimeObservation {
        self.stop.store(true, Ordering::Relaxed);
        let mut observation = match self.samples.take() {
            Some(worker) => worker.join().unwrap_or_else(|_| RuntimeObservation {
                samples: 0,
                interval_ms: 0,
                note: Some("observer thread panicked".to_string()),
                ..Default::default()
            }),
            None => RuntimeObservation::default(),
        };
        if let Some(worker) = self.environment.take() {
            if let Ok(environment) = worker.join() {
                observation.gpu_utilization_percent = environment.gpu_utilization_percent;
                observation.gpu_memory_bytes = environment.gpu_memory_bytes;
                observation.max_temperature_c = environment.max_temperature_c;
            }
        }
        observation
    }
}

impl Drop for ObservationHandle {
    fn drop(&mut self) {
        // Makes sure the sampling thread never outlives the runner.
        self.finish();
    }
}

fn sample_tree(pid: u32, interval: Duration, stop: Arc<AtomicBool>) -> RuntimeObservation {
    let mut observation = RuntimeObservation {
        samples: 0,
        interval_ms: interval.as_millis() as u64,
        ..Default::default()
    };
    let root = Pid::from_u32(pid);
    let mut system = System::new();
    let started = Instant::now();

    // Per-process maxima: a child that burns CPU and exits before the last sample keeps its
    // contribution, and a process that forks does not lose the work of its parent.
    let mut cpu_time_by_pid: HashMap<Pid, u64> = HashMap::new();
    let mut memory_by_pid: HashMap<Pid, u64> = HashMap::new();
    let mut max_instant_cpu = 0.0f32;
    let mut peak_processes = 0u32;
    let mut known: Vec<Pid> = vec![root];
    let mut tick: u32 = 0;

    while !stop.load(Ordering::Relaxed) {
        // Walking the whole process table is only needed to discover children (a shell or a build
        // tool spawning sub-processes). In between, refreshing the pids we already know keeps the
        // observer's own cost out of the measurement it is taking.
        let full_refresh = tick == 0 || tick == 2 || tick % 8 == 0;
        if full_refresh {
            system.refresh_processes(ProcessesToUpdate::All, true);
            known = process_tree(&mut system, root);
        } else {
            system.refresh_processes(ProcessesToUpdate::Some(&known), true);
        }

        if !known.is_empty() {
            let mut instant_cpu = 0.0f32;
            for process_pid in &known {
                if let Some(process) = system.process(*process_pid) {
                    instant_cpu += process.cpu_usage();
                    let cpu_slot = cpu_time_by_pid.entry(*process_pid).or_insert(0);
                    *cpu_slot = (*cpu_slot).max(process.accumulated_cpu_time());
                    let memory_slot = memory_by_pid.entry(*process_pid).or_insert(0);
                    *memory_slot = (*memory_slot).max(process.memory());
                }
            }
            max_instant_cpu = max_instant_cpu.max(instant_cpu);
            peak_processes = peak_processes.max(known.len() as u32);
            observation.samples += 1;
        }

        tick = tick.saturating_add(1);
        thread::sleep(next_delay(tick, interval));
    }

    if observation.samples == 0 {
        observation.note =
            Some("the target process was never observed (too short lived for sampling)".to_string());
        return observation;
    }

    // CPU time consumed by the tree, expressed as a percentage of the observation window.
    // `accumulated_cpu_time()` is reported in milliseconds; one core fully busy yields ~100%.
    let window_ms = started.elapsed().as_secs_f64() * 1000.0;
    let cpu_time_ms: u64 = cpu_time_by_pid.values().copied().sum();
    if window_ms > 0.0 {
        observation.avg_cpu_percent = Some((cpu_time_ms as f64 / window_ms * 100.0) as f32);
    }
    observation.max_cpu_percent = (max_instant_cpu > 0.0).then_some(max_instant_cpu);
    let peak_memory: u64 = memory_by_pid.values().copied().sum();
    observation.peak_memory_bytes = (peak_memory > 0).then_some(peak_memory);
    observation.peak_process_count = peak_processes;
    observation
}

/// Samples GPU counters and temperatures on a thread of their own.
///
/// Vendor tools are slow (starting `nvidia-smi` costs 100 ms up to a few seconds) and temperatures
/// may need WMI round-trips, so this loop never shares a thread with the process counters. The
/// returned observation only carries the environment fields; the caller merges it into the process
/// observation.
fn sample_environment(stop: Arc<AtomicBool>, interval: Duration) -> RuntimeObservation {
    let gpu_sampler = GpuSampler::detect();
    let mut observation = RuntimeObservation::default();

    // The first GPU reading is taken before anything else: for a 100 ms command it may be the only
    // one we ever get, and building the temperature sensor list can take a second on its own.
    if let Some(sampler) = &gpu_sampler {
        merge_gpu_sample(&mut observation, sampler.sample());
    }

    let mut components = Components::new_with_refreshed_list();
    let has_temperature_sensors = !components.list().is_empty();
    if gpu_sampler.is_none() && !has_temperature_sensors {
        // Nothing on this machine can be sampled: no reason to keep a thread alive.
        return observation;
    }

    while !stop.load(Ordering::Relaxed) {
        // Sleep in short steps so `stop` is honoured promptly even after a slow vendor call.
        let deadline = Instant::now() + interval;
        while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }

        if let Some(sampler) = &gpu_sampler {
            merge_gpu_sample(&mut observation, sampler.sample());
        }

        if has_temperature_sensors {
            components.refresh(true);
            let hottest = components
                .iter()
                .filter_map(|component| component.temperature())
                .filter(|temperature| temperature.is_finite() && *temperature > 0.0)
                .fold(f32::MIN, f32::max);
            if hottest.is_finite() && hottest > 0.0 {
                observation.max_temperature_c =
                    max_f32(observation.max_temperature_c, Some(hottest));
            }
        }
    }
    observation
}

/// Keeps the highest value of every GPU metric seen so far.
fn merge_gpu_sample(observation: &mut RuntimeObservation, sample: Option<GpuSample>) {
    let Some(sample) = sample else {
        return;
    };
    observation.gpu_utilization_percent = max_f32(
        observation.gpu_utilization_percent,
        sample.utilization_percent,
    );
    observation.gpu_memory_bytes = match (observation.gpu_memory_bytes, sample.memory_bytes) {
        (Some(current), Some(new)) => Some(current.max(new)),
        (current, new) => current.or(new),
    };
    observation.max_temperature_c = max_f32(observation.max_temperature_c, sample.temperature_c);
}

/// `max` for optional floats.
fn max_f32(current: Option<f32>, candidate: Option<f32>) -> Option<f32> {
    match (current, candidate) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Sampling schedule: dense right after spawn (most measured commands are short lived and CPU time
/// is cumulative, so early samples are what make them readable), then the configured interval.
fn next_delay(tick: u32, interval: Duration) -> Duration {
    match tick {
        1 | 2 => Duration::from_millis(10),
        3 | 4 => Duration::from_millis(25),
        5..=8 => Duration::from_millis(50),
        9..=16 => Duration::from_millis(100),
        _ => interval.max(Duration::from_millis(100)),
    }
}

/// Collects `root` plus every descendant, following the parent links.
fn process_tree(system: &mut System, root: Pid) -> Vec<Pid> {
    if system.process(root).is_none() {
        return Vec::new();
    }
    let mut tree = vec![root];
    let mut frontier = vec![root];
    while let Some(current) = frontier.pop() {
        for (pid, process) in system.processes() {
            if process.parent() == Some(current) && !tree.contains(pid) {
                tree.push(*pid);
                frontier.push(*pid);
            }
        }
    }
    tree
}

/// Counts the processes of one command tree (plan §15: `limits.max_processes`).
///
/// The runner owns its own snapshot: [`ProcessTreeCounter::count`] refreshes the whole process
/// table, discovers the descendants of `root` and reports how many processes are alive. The
/// counter only exists while a process limit is configured, so unlimited commands pay nothing.
#[derive(Default)]
pub struct ProcessTreeCounter {
    system: System,
}

impl ProcessTreeCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of processes in the tree rooted at `root`, the root itself included.
    ///
    /// Returns `None` when `root` has already exited: there is no tree left to police.
    pub fn count(&mut self, root: u32) -> Option<usize> {
        self.system.refresh_processes(ProcessesToUpdate::All, true);
        let root = Pid::from_u32(root);
        self.system.process(root)?;
        Some(process_tree(&mut self.system, root).len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};

    /// Spawns a process that keeps one core busy for roughly 1.5 seconds.
    fn busy_child() -> Child {
        #[cfg(windows)]
        let child = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "$end = (Get-Date).AddMilliseconds(1500); while ((Get-Date) -lt $end) { $null = 1 + 1 }",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("powershell should be available on Windows");
        #[cfg(not(windows))]
        let child = Command::new("sh")
            .args([
                "-c",
                "end=$(( $(date +%s) + 2 )); while [ $(date +%s) -lt $end ]; do :; done",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("sh should be available");
        child
    }

    /// The counter that enforces `limits.max_processes` must see live roots (and their
    /// descendants) and report `None` once the root is gone, so finished commands are
    /// never policed.
    #[test]
    fn process_tree_counter_counts_live_roots_and_forgets_dead_ones() {
        let mut counter = ProcessTreeCounter::new();
        let mut child = busy_child();
        let size = counter
            .count(child.id())
            .expect("a live root must be counted");
        assert!(size >= 1, "the root itself is part of the tree: {size}");

        let _ = child.kill();
        let _ = child.wait();
        let mut forgotten = None;
        for _ in 0..20 {
            forgotten = counter.count(child.id());
            if forgotten.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(forgotten, None, "a reaped root leaves no tree to count");
    }

    #[test]
    fn observes_cpu_time_and_memory_of_a_busy_process() {
        let mut child = busy_child();
        let handle = ObservationHandle::start(child.id(), ObserveOptions { interval_ms: 150 });
        let _ = child.wait();
        let observation = handle.stop();
        eprintln!("observation: {observation:?}");

        assert!(
            observation.samples > 0,
            "expected at least one sample: {observation:?}"
        );
        assert!(
            observation.peak_memory_bytes.unwrap_or(0) > 0,
            "expected a memory reading: {observation:?}"
        );
        let cpu = observation
            .avg_cpu_percent
            .expect("CPU usage should be measured");
        // One core stays busy for the whole window, so the value should be around 100%.
        // The wide band still catches unit mistakes (milliseconds vs. ticks would be 10x off).
        assert!(
            (25.0..=250.0).contains(&cpu),
            "unexpected CPU percentage {cpu} ({observation:?})"
        );
    }

    #[test]
    fn environment_sampling_reports_gpu_metrics_without_starving_the_counters() {
        let Some(sampler) = GpuSampler::detect() else {
            eprintln!("skipping: no GPU vendor tool on this machine");
            return;
        };
        eprintln!("using {}", sampler.name());

        let mut child = busy_child();
        let handle = ObservationHandle::start(child.id(), ObserveOptions { interval_ms: 250 });
        let _ = child.wait();
        let observation = handle.stop();
        eprintln!("environment observation: {observation:?}");

        assert!(
            observation.gpu_utilization_percent.is_some()
                || observation.gpu_memory_bytes.is_some()
                || observation.max_temperature_c.is_some(),
            "a detected GPU tool must produce at least one reading: {observation:?}"
        );
        // The process counters run on their own thread and must not be starved by the GPU sampling.
        assert!(
            observation.samples > 0,
            "the counter thread must still have sampled: {observation:?}"
        );
    }
}
