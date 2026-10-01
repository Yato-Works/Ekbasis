//! Command execution: spawn, timeout, capture and observe (plan §4, §6, §15).
//!
//! The runner is the only place in AION that starts a process. Everything it does is explicit:
//! which command, in which directory, with which environment, for how long, and what came out.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use ekbasis_core::ContainerSpec;
use ekbasis_core::model::RuntimeObservation;
use ekbasis_observer::{ObservationHandle, ObserveOptions, ProcessTreeCounter};
use anyhow::{Context, Result, bail};

/// Environment variables kept when `clean_env` is enabled (plan §15: environment whitelist).
const ENV_WHITELIST: &[&str] = &[
    "PATH",
    "PATHEXT",
    "COMSPEC",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
    "CommonProgramFiles",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "PROCESSOR_IDENTIFIER",
    "OS",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "TERM",
    "TZ",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "JAVA_HOME",
    "DOTNET_ROOT",
    "PYTHONHOME",
    "PYTHONPATH",
    "NODE_PATH",
    "GOPATH",
    "GOROOT",
];

/// Extra grace period granted to a process that ignores the kill signal.
const KILL_GRACE: Duration = Duration::from_secs(15);

/// How often the process tree size is sampled while `limits.max_processes` is enforced.
/// Frequent enough to stop a fork bomb quickly, rare enough not to disturb the measurement.
const PROCESS_POLL: Duration = Duration::from_millis(150);

/// How a command string was turned into a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnMode {
    /// Executed directly: no shell overhead and the observed pid is the program itself.
    Direct,
    /// Interpreted by the platform shell (`cmd /C`, `sh -c`) because the command uses shell syntax.
    Shell,
}

impl SpawnMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SpawnMode::Direct => "direct",
            SpawnMode::Shell => "shell",
        }
    }
}

/// Raw result of one executed command.
#[derive(Debug, Clone)]
pub struct RawOutcome {
    pub command: String,
    pub spawn_mode: SpawnMode,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub timed_out: bool,
    /// `Some(limit)` when the command tree was killed for containing more than `limit`
    /// processes (plan §15: `limits.max_processes`).
    pub limit_hit: Option<u32>,
    pub duration: Duration,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub observation: Option<RuntimeObservation>,
}

impl RawOutcome {
    /// Measured wall-clock time in milliseconds.
    pub fn duration_ms(&self) -> f64 {
        self.duration.as_secs_f64() * 1000.0
    }

    /// Short, human readable reason for a failure (used in CLI output and warnings).
    pub fn failure_reason(&self) -> Option<String> {
        if self.timed_out {
            return Some(format!(
                "timed out after {}",
                ekbasis_core::fmt::duration_ms(self.duration_ms())
            ));
        }
        if let Some(limit) = self.limit_hit {
            return Some(format!(
                "process limit exceeded (more than {limit} processes); the command tree was killed"
            ));
        }
        if self.success {
            return None;
        }
        Some(match self.exit_code {
            Some(code) => format!("exit code {code}"),
            None => "terminated without an exit code".to_string(),
        })
    }
}

/// Whether `limits.network: block` can be enforced on this machine (plan §15).
///
/// AION never reports an isolation it did not apply: where no network namespace can be
/// created, [`Executor::with_network_block`] refuses to run the command instead of silently
/// leaving it connected. On Linux the answer comes from probing `unshare -Urn` once per
/// process (the result is cached); on every other platform it is always an error that names
/// what is missing.
#[cfg(target_os = "linux")]
pub fn network_block_support() -> Result<(), String> {
    static PROBE: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    PROBE
        .get_or_init(|| {
            let probe = std::process::Command::new("unshare")
                .args(["-Urn", "--", "true"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            match probe {
                Ok(status) if status.success() => Ok(()),
                Ok(status) => Err(format!(
                    "`unshare -Urn` failed ({status}); unprivileged user namespaces may be disabled"
                )),
                Err(error) => Err(format!("`unshare` is not available ({error})")),
            }
        })
        .clone()
}

/// See the Linux variant: no supported network-block mechanism exists here.
#[cfg(not(target_os = "linux"))]
pub fn network_block_support() -> Result<(), String> {
    Err(format!(
        "AION blocks the network with Linux `unshare -Urn` (user + network namespace), but this platform is {} — run the experiment on Linux or inside a container with network isolation",
        std::env::consts::OS
    ))
}

/// Detects an available container runtime (Docker or Podman).
pub fn detect_container_engine() -> Option<&'static str> {
    static ENGINE: std::sync::OnceLock<Option<&'static str>> = std::sync::OnceLock::new();
    *ENGINE.get_or_init(|| {
        let probe = |cmd: &str| {
            Command::new(cmd)
                .arg("--version")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };
        if probe("docker") {
            Some("docker")
        } else if probe("podman") {
            Some("podman")
        } else {
            None
        }
    })
}

/// Executes command strings with a timeout inside one working directory.
#[derive(Debug, Clone)]
pub struct Executor {
    cwd: PathBuf,
    timeout: Duration,
    env: BTreeMap<String, String>,
    clean_env: bool,
    max_output_bytes: usize,
    observe: Option<ObserveOptions>,
    verbose: bool,
    /// Kill the command tree once it contains more than this many processes (plan §15).
    max_processes: Option<u32>,
    /// Refuse to run unless the command can be wrapped in an empty network namespace.
    network_block: bool,
    /// Optional container isolation (Docker / Podman).
    container: Option<ContainerSpec>,
}

impl Executor {
    /// Creates an executor for `cwd` with AION's defaults (10 min timeout, 256 KiB output, observe).
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Executor {
            cwd: cwd.into(),
            timeout: Duration::from_secs(600),
            env: BTreeMap::new(),
            clean_env: false,
            max_output_bytes: 256 * 1024,
            observe: Some(ObserveOptions::default()),
            verbose: false,
            max_processes: None,
            network_block: false,
            container: None,
        }
    }

    pub fn with_timeout_secs(mut self, seconds: u64) -> Self {
        self.timeout = Duration::from_secs(seconds.max(1));
        self
    }

    /// Adds/overrides environment variables on top of the inherited environment.
    pub fn with_env(mut self, env: &BTreeMap<String, String>) -> Self {
        for (name, value) in env {
            self.env.insert(name.clone(), value.clone());
        }
        self
    }

    /// When enabled, only whitelisted variables are inherited (plan §15).
    pub fn with_clean_env(mut self, clean: bool) -> Self {
        self.clean_env = clean;
        self
    }

    /// Per-stream output kept in memory, in KiB.
    pub fn with_max_output_kb(mut self, kb: usize) -> Self {
        self.max_output_bytes = kb.saturating_mul(1024).max(1024);
        self
    }

    /// Enables or disables CPU/RAM sampling for the (sub)processes.
    pub fn with_observe(mut self, observe: Option<ObserveOptions>) -> Self {
        self.observe = observe;
        self
    }

    /// Prints every command before it runs.
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Kills the command tree once it exceeds `limit` processes (plan §15).
    ///
    /// The tree size is polled while the command runs; a breach kills the whole tree and the
    /// outcome is reported as failed. `None` disables the limit.
    pub fn with_max_processes(mut self, limit: Option<u32>) -> Self {
        self.max_processes = limit.filter(|limit| *limit > 0);
        self
    }

    /// Runs every command inside an empty network namespace (plan §15: `limits.network: block`).
    ///
    /// When the platform cannot enforce the block ([`network_block_support`]), execution
    /// fails with the reason instead of running connected — AION never reports an isolation
    /// it did not apply.
    pub fn with_network_block(mut self, block: bool) -> Self {
        self.network_block = block;
        self
    }

    /// Runs commands inside an isolated Docker or Podman container.
    pub fn with_container(mut self, container: Option<ContainerSpec>) -> Self {
        self.container = container;
        self
    }

    /// Working directory of this executor.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Runs a command without resource observation.
    pub fn run(&self, command: &str) -> Result<RawOutcome> {
        self.execute(command, false)
    }

    /// Runs a command and samples CPU/memory while it is alive.
    pub fn run_observed(&self, command: &str) -> Result<RawOutcome> {
        self.execute(command, true)
    }

    fn execute(&self, command: &str, observe: bool) -> Result<RawOutcome> {
        let (program, args, spawn_mode) = if let Some(container) = &self.container {
            let engine = match container.engine.as_deref() {
                Some(e) => e,
                None => detect_container_engine().ok_or_else(|| {
                    anyhow::anyhow!(
                        "container execution requested ({}), but neither 'docker' nor 'podman' was found in PATH",
                        container.image
                    )
                })?,
            };

            let mut run_args = vec!["run".to_string(), "--rm".to_string()];

            let cwd_str = self.cwd.to_string_lossy().to_string();
            run_args.push("-v".to_string());
            run_args.push(format!("{cwd_str}:/workspace"));
            run_args.push("-w".to_string());
            run_args.push("/workspace".to_string());

            if self.network_block {
                run_args.push("--network".to_string());
                run_args.push("none".to_string());
            }

            for mount in &container.mounts {
                run_args.push("-v".to_string());
                run_args.push(mount.clone());
            }

            for (name, value) in &self.env {
                run_args.push("-e".to_string());
                run_args.push(format!("{name}={value}"));
            }

            run_args.push(container.image.clone());
            run_args.push("sh".to_string());
            run_args.push("-c".to_string());
            run_args.push(command.to_string());

            (engine.to_string(), run_args, SpawnMode::Shell)
        } else {
            let (program, args, spawn_mode) = split_command(command);
            // Windows resolves a relative program name against the *calling* process's working
            // directory, not the child's, so a path like `target/release/app.exe` has to be made
            // absolute against our own working directory before spawning.
            let program = resolve_program(&program, &self.cwd);
            // Network isolation (plan §15): wrap in `unshare -Urn` where that works, refuse to run
            // anywhere else — a connected command must never be reported as blocked.
            let (program, args) = if self.network_block {
                match network_block_support() {
                    Ok(()) => {
                        let mut wrapped = vec!["-Urn".to_string(), "--".to_string(), program];
                        wrapped.extend(args);
                        ("unshare".to_string(), wrapped)
                    }
                    Err(reason) => {
                        bail!("`limits.network: block` was requested but cannot be enforced: {reason}")
                    }
                }
            } else {
                (program, args)
            };
            (program, args, spawn_mode)
        };
        if self.verbose {
            eprintln!(
                "  $ {}   [{}] (cwd: {})",
                command,
                spawn_mode.as_str(),
                self.cwd.display()
            );
        }
        let mut process = Command::new(&program);
        process
            .args(&args)
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if self.clean_env {
            process.env_clear();
            for name in ENV_WHITELIST {
                if let Ok(value) = std::env::var(name) {
                    process.env(name, value);
                }
            }
        }
        for (name, value) in &self.env {
            process.env(name, value);
        }

        let started = Instant::now();
        let mut child = process
            .spawn()
            .with_context(|| format!("cannot start `{command}` in `{}`", self.cwd.display()))?;
        let pid = child.id();
        let cap = self.max_output_bytes;
        let stdout_worker = child
            .stdout
            .take()
            .map(|stream| thread::spawn(move || drain(stream, cap)));
        let stderr_worker = child
            .stderr
            .take()
            .map(|stream| thread::spawn(move || drain(stream, cap)));
        let mut observation = match (observe, self.observe) {
            (true, Some(options)) => Some(ObservationHandle::start(pid, options)),
            _ => None,
        };

        let mut timed_out = false;
        let mut limit_hit: Option<u32> = None;
        // Process limit (plan §15): only commands that declared a limit pay for the polling.
        let mut counter = self.max_processes.map(|_| ProcessTreeCounter::new());
        let mut next_limit_poll = started + PROCESS_POLL;
        let mut kill_requested_at: Option<Instant> = None;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            let now = Instant::now();
            if kill_requested_at.is_none() {
                if let (Some(limit), Some(counter)) = (self.max_processes, counter.as_mut()) {
                    if now >= next_limit_poll {
                        next_limit_poll = now + PROCESS_POLL;
                        // `None` means the root already exited: nothing left to police.
                        if let Some(size) = counter.count(pid) {
                            if size > limit as usize {
                                limit_hit = Some(limit);
                                kill_requested_at = Some(now);
                                kill_process_tree(&mut child, pid);
                            }
                        }
                    }
                }
            }
            if kill_requested_at.is_none() && now >= started + self.timeout {
                timed_out = true;
                kill_requested_at = Some(now);
                kill_process_tree(&mut child, pid);
            }
            if let Some(requested) = kill_requested_at {
                if now >= requested + KILL_GRACE {
                    let _ = child.kill();
                    break child.wait()?;
                }
            }
            thread::sleep(Duration::from_millis(20));
        };
        let duration = started.elapsed();
        let observed = observation.take().map(|handle| handle.stop());
        let (stdout, stdout_truncated) = join_stream(stdout_worker);
        let (stderr, stderr_truncated) = join_stream(stderr_worker);

        Ok(RawOutcome {
            command: command.to_string(),
            spawn_mode,
            exit_code: status.code(),
            success: status.success() && !timed_out && limit_hit.is_none(),
            timed_out,
            limit_hit,
            duration,
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
            observation: observed,
        })
    }
}

/// Splits a command string into `(program, args, mode)`.
///
/// A command without shell metacharacters is executed directly: this removes the `cmd /C`
/// (or `sh -c`) wrapper, its startup cost, and gives the observer the real process id.
fn split_command(command: &str) -> (String, Vec<String>, SpawnMode) {
    let trimmed = command.trim();
    let via_shell = |text: &str| {
        (
            shell_program(),
            vec![shell_flag().to_string(), text.to_string()],
            SpawnMode::Shell,
        )
    };
    if trimmed.is_empty() {
        return via_shell(trimmed);
    }
    if has_shell_syntax(trimmed) {
        return via_shell(trimmed);
    }
    match trimmed.split_whitespace().next() {
        Some(program) => {
            let args: Vec<String> = trimmed
                .split_whitespace()
                .skip(1)
                .map(|part| part.to_string())
                .collect();
            (program.to_string(), args, SpawnMode::Direct)
        }
        None => via_shell(trimmed),
    }
}

/// Makes a relative program path absolute against the executor's working directory.
///
/// Bare names (`cargo`) are left untouched so the operating system can search `PATH`.
fn resolve_program(program: &str, cwd: &Path) -> String {
    let candidate = Path::new(program);
    if candidate.is_absolute() {
        return program.to_string();
    }
    let has_directory_component = candidate.components().count() > 1;
    if !has_directory_component {
        return program.to_string();
    }
    let resolved = cwd.join(candidate);
    if resolved.exists() {
        resolved.to_string_lossy().to_string()
    } else {
        program.to_string()
    }
}

/// True when the command must be interpreted by a shell instead of spawned directly.
fn has_shell_syntax(command: &str) -> bool {
    command.chars().any(|c| {
        matches!(
            c,
            '|' | '&'
                | ';'
                | '<'
                | '>'
                | '('
                | ')'
                | '$'
                | '`'
                | '"'
                | '\''
                | '^'
                | '%'
                | '*'
                | '?'
                | '!'
                | '{'
                | '}'
                | '['
                | ']'
                | '~'
                | '#'
                | '\n'
                | '\r'
                | '\t'
        )
    })
}

#[cfg(windows)]
fn shell_program() -> String {
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd".to_string())
}

#[cfg(windows)]
fn shell_flag() -> &'static str {
    "/C"
}

#[cfg(not(windows))]
fn shell_program() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string())
}

#[cfg(not(windows))]
fn shell_flag() -> &'static str {
    "-c"
}

/// Reads a stream to the end, keeping at most `cap` bytes and discarding the rest.
fn drain<R: Read>(mut stream: R, cap: usize) -> (String, bool) {
    let mut kept: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 16 * 1024];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                if kept.len() < cap {
                    let room = cap - kept.len();
                    let take = room.min(read);
                    kept.extend_from_slice(&chunk[..take]);
                    if take < read {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(_) => break,
        }
    }
    (String::from_utf8_lossy(&kept).to_string(), truncated)
}

fn join_stream(worker: Option<thread::JoinHandle<(String, bool)>>) -> (String, bool) {
    match worker {
        Some(worker) => worker.join().unwrap_or_else(|_| (String::new(), true)),
        None => (String::new(), false),
    }
}

/// Kills a child process and, on Windows, every process it spawned.
fn kill_process_tree(child: &mut Child, pid: u32) {
    #[cfg(windows)]
    {
        // Interpreters and build tools spawn grandchildren; /T removes the whole tree.
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
    }
    let _ = child.kill();
}

/// Last `max_chars` characters of a text block (used to explain command failures).
pub fn tail_output(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        text.trim().to_string()
    } else {
        chars[chars.len() - max_chars..]
            .iter()
            .collect::<String>()
            .trim()
            .to_string()
    }
}

/// How AION would execute a command string: `(program, args, mode)`.
pub fn describe_invocation(command: &str) -> (String, Vec<String>, &'static str) {
    let (program, args, mode) = split_command(command);
    (program, args, mode.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_commands_run_directly() {
        let (program, args, mode) = split_command("cargo build --release");
        assert_eq!(program, "cargo");
        assert_eq!(args, vec!["build".to_string(), "--release".to_string()]);
        assert_eq!(mode, SpawnMode::Direct);
    }

    #[test]
    fn pipelines_go_through_the_shell() {
        let (_, _, mode) = split_command("echo hi | findstr h");
        assert_eq!(mode, SpawnMode::Shell);
    }

    #[test]
    fn windows_style_paths_stay_direct() {
        let (program, args, mode) = split_command(".\\target\\release\\app.exe --fast");
        assert_eq!(program, ".\\target\\release\\app.exe");
        assert_eq!(args, vec!["--fast".to_string()]);
        assert_eq!(mode, SpawnMode::Direct);
    }

    #[test]
    fn captures_stdout_and_exit_code() {
        let executor = Executor::new(std::env::temp_dir())
            .with_timeout_secs(60)
            .with_observe(None);
        #[cfg(windows)]
        let command = "cmd /C echo aion-output";
        #[cfg(not(windows))]
        let command = "echo aion-output";
        let outcome = executor.run(command).expect("command should run");
        assert!(outcome.success, "stderr: {}", outcome.stderr);
        assert_eq!(outcome.exit_code, Some(0));
        assert!(outcome.stdout.contains("aion-output"));
        assert!(outcome.duration_ms() > 0.0);
    }

    #[test]
    fn kills_commands_that_exceed_the_timeout() {
        let executor = Executor::new(std::env::temp_dir())
            .with_timeout_secs(1)
            .with_observe(None);
        #[cfg(windows)]
        let command = "cmd /C ping 127.0.0.1 -n 30";
        #[cfg(not(windows))]
        let command = "sh -c \"sleep 30\"";
        let outcome = executor.run(command).expect("command should run");
        assert!(outcome.timed_out, "expected a timeout, got {outcome:?}");
        assert!(!outcome.success);
        assert_eq!(outcome.limit_hit, None);
    }

    /// A tree that spawns more processes than allowed is killed and reported as failed,
    /// well before the timeout would have fired (plan §15).
    #[test]
    fn kills_trees_that_exceed_the_process_limit() {
        let executor = Executor::new(std::env::temp_dir())
            .with_timeout_secs(60)
            .with_observe(None)
            .with_max_processes(Some(1));
        // cmd.exe (or sh) spawns the workload as a child, so the tree is at least two
        // processes wide and must breach a limit of one.
        #[cfg(windows)]
        let command = "cmd /C ping 127.0.0.1 -n 30";
        #[cfg(not(windows))]
        let command = "sh -c \"sleep 30 & wait\"";
        let outcome = executor.run(command).expect("command should run");
        assert_eq!(
            outcome.limit_hit,
            Some(1),
            "expected the process limit to fire, got {outcome:?}"
        );
        assert!(!outcome.success);
        assert!(!outcome.timed_out, "the limit, not the timeout, must kill");
        let reason = outcome.failure_reason().expect("failure reason");
        assert!(reason.contains("process limit"), "{reason}");
    }

    /// A tree inside the limit runs to completion: the guard must not slow down or kill
    /// well-behaved commands.
    #[test]
    fn lets_trees_within_the_process_limit_finish() {
        let executor = Executor::new(std::env::temp_dir())
            .with_timeout_secs(60)
            .with_observe(None)
            .with_max_processes(Some(16));
        // cmd.exe spawns PowerShell as a child (a tree of two, inside the limit) and the
        // command must succeed untouched. Loopback ping is deliberately avoided: it is
        // commonly blocked and would fail for reasons unrelated to the guard.
        #[cfg(windows)]
        let command = "cmd /C powershell -NoProfile -Command Start-Sleep -Seconds 1";
        #[cfg(not(windows))]
        let command = "sh -c \"sleep 1\"";
        let outcome = executor.run(command).expect("command should run");
        assert!(
            outcome.limit_hit.is_none(),
            "no breach expected, got {outcome:?}"
        );
        assert!(outcome.success, "stderr: {}", outcome.stderr);
    }

    /// `network: block` must never run a connected command: where the platform cannot
    /// enforce isolation, execution fails with the reason instead (plan §15).
    #[test]
    fn network_block_fails_loudly_where_it_cannot_be_enforced() {
        let executor = Executor::new(std::env::temp_dir())
            .with_timeout_secs(60)
            .with_observe(None)
            .with_network_block(true);
        #[cfg(not(target_os = "linux"))]
        {
            let error = executor.run("cmd /C echo hi").expect_err("must refuse");
            let text = format!("{error:#}");
            assert!(text.contains("cannot be enforced"), "{text}");
            assert!(text.contains("block"), "{text}");
        }
        #[cfg(target_os = "linux")]
        match network_block_support() {
            Err(reason) => {
                let error = executor.run("echo hi").expect_err("must refuse");
                assert!(
                    format!("{error:#}").contains(&reason),
                    "the error must carry the probe's reason"
                );
            }
            Ok(()) => {
                let outcome = executor.run("echo hi").expect("command should run");
                assert!(outcome.success, "stderr: {}", outcome.stderr);
            }
        }
    }
}
