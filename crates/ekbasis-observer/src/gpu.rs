//! GPU observation (plan §6, §17).
//!
//! Strategy, in order of preference:
//!
//! 1. `nvidia-smi` — queryable, gives utilisation, VRAM usage and temperature per adapter.
//! 2. `rocm-smi` — the AMD equivalent, parsed from its plain-text output.
//! 3. A static operating-system query (`Get-CimInstance Win32_VideoController` on Windows,
//!    `lspci` on Linux, `system_profiler` on macOS) so at least the adapter names end up in the
//!    reproducibility record.
//!
//! Everything here is best effort: a machine without a GPU, without vendor tools or without a
//! driver simply produces no GPU metrics instead of an error.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

use ekbasis_core::model::GpuInfo;

/// Upper bound for a single vendor tool invocation.
const TOOL_TIMEOUT: Duration = Duration::from_secs(5);

/// One GPU reading.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GpuSample {
    /// Busiest adapter's utilisation, in percent.
    pub utilization_percent: Option<f32>,
    /// Highest VRAM usage across adapters, in bytes.
    pub memory_bytes: Option<u64>,
    /// Hottest adapter, in degrees Celsius.
    pub temperature_c: Option<f32>,
}

impl GpuSample {
    /// Merges another reading, keeping the higher value of every metric.
    pub fn merge(&mut self, other: GpuSample) {
        self.utilization_percent = max_f32(self.utilization_percent, other.utilization_percent);
        self.temperature_c = max_f32(self.temperature_c, other.temperature_c);
        if let Some(bytes) = other.memory_bytes {
            self.memory_bytes = Some(self.memory_bytes.unwrap_or(0).max(bytes));
        }
    }

    /// True when the reading carries at least one value.
    fn is_meaningful(&self) -> bool {
        self.utilization_percent.is_some()
            || self.memory_bytes.is_some()
            || self.temperature_c.is_some()
    }
}

/// Which vendor tool was found for live sampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Vendor {
    Nvidia,
    Amd,
}

/// A live GPU sampler (cheap to copy; it only wraps the vendor choice).
#[derive(Debug, Clone, Copy)]
pub struct GpuSampler {
    vendor: Vendor,
}

impl GpuSampler {
    /// Name of the vendor tool behind this sampler (`nvidia-smi`, `rocm-smi`).
    pub fn name(&self) -> &'static str {
        match self.vendor {
            Vendor::Nvidia => "nvidia-smi",
            Vendor::Amd => "rocm-smi",
        }
    }

    /// Detects a usable vendor tool, or `None` when none is installed.
    pub fn detect() -> Option<GpuSampler> {
        if program_available("nvidia-smi") {
            return Some(GpuSampler {
                vendor: Vendor::Nvidia,
            });
        }
        if program_available("rocm-smi") {
            return Some(GpuSampler { vendor: Vendor::Amd });
        }
        None
    }

    /// Reads the current GPU state, or `None` when the tool did not answer usefully.
    pub fn sample(&self) -> Option<GpuSample> {
        match self.vendor {
            Vendor::Nvidia => {
                let text = run_tool(
                    "nvidia-smi",
                    &[
                        "--query-gpu=utilization.gpu,memory.used,temperature.gpu",
                        "--format=csv,noheader,nounits",
                    ],
                )?;
                parse_nvidia_samples(&text)
            }
            Vendor::Amd => {
                let text = run_tool(
                    "rocm-smi",
                    &["--showuse", "--showmeminfo", "vram", "--showtemp"],
                )?;
                parse_rocm_samples(&text)
            }
        }
    }
}

/// Adapter list for the reproducibility record (cached for the process lifetime).
pub fn detect_gpu_info() -> Vec<GpuInfo> {
    static CACHE: OnceLock<Vec<GpuInfo>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let mut adapters = nvidia_adapters().unwrap_or_default();
            if adapters.is_empty() {
                adapters = os_adapters();
            }
            dedupe(adapters)
        })
        .clone()
}

/// Adapters reported by `nvidia-smi` (also fills in VRAM and driver version).
fn nvidia_adapters() -> Option<Vec<GpuInfo>> {
    if !program_available("nvidia-smi") {
        return None;
    }
    let text = run_tool(
        "nvidia-smi",
        &[
            "--query-gpu=name,memory.total,driver_version",
            "--format=csv,noheader,nounits",
        ],
    )?;
    let mut adapters = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split(',').map(|field| field.trim()).collect();
        if fields.is_empty() || fields[0].is_empty() {
            continue;
        }
        adapters.push(GpuInfo {
            name: fields[0].to_string(),
            memory_bytes: fields
                .get(1)
                .and_then(|value| value.parse::<f64>().ok())
                .map(mib_to_bytes)
                .filter(|bytes| *bytes > 0),
            driver_version: fields
                .get(2)
                .map(|value| value.to_string())
                .filter(|value| !value.is_empty()),
        });
    }
    Some(adapters)
}

/// Adapters reported by the operating system itself.
fn os_adapters() -> Vec<GpuInfo> {
    #[cfg(windows)]
    {
        windows_adapters().unwrap_or_default()
    }
    #[cfg(target_os = "macos")]
    {
        macos_adapters().unwrap_or_default()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux_adapters().unwrap_or_default()
    }
    #[cfg(not(any(windows, unix)))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
fn windows_adapters() -> Option<Vec<GpuInfo>> {
    let script = "Get-CimInstance Win32_VideoController | \
                  Select-Object Name,AdapterRAM,DriverVersion | \
                  ConvertTo-Csv -NoTypeInformation";
    let text = run_tool(
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command", script],
    )?;
    Some(parse_windows_csv(&text))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_adapters() -> Option<Vec<GpuInfo>> {
    let text = run_tool("lspci", &[])?;
    let mut adapters = Vec::new();
    for line in text.lines() {
        if line.contains("VGA compatible controller")
            || line.contains("3D controller")
            || line.contains("Display controller")
        {
            if let Some((_, name)) = line.rsplit_once(": ") {
                adapters.push(GpuInfo {
                    name: name.trim().to_string(),
                    memory_bytes: None,
                    driver_version: None,
                });
            }
        }
    }
    Some(adapters)
}

#[cfg(target_os = "macos")]
fn macos_adapters() -> Option<Vec<GpuInfo>> {
    let text = run_tool("system_profiler", &["SPDisplaysDataType"])?;
    let mut adapters = Vec::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once(':') {
            if key.trim() == "Chipset Model" {
                adapters.push(GpuInfo {
                    name: value.trim().to_string(),
                    memory_bytes: None,
                    driver_version: None,
                });
            }
        }
    }
    Some(adapters)
}

/// Parses the `ConvertTo-Csv -NoTypeInformation` output of `Win32_VideoController`.
fn parse_windows_csv(text: &str) -> Vec<GpuInfo> {
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default().to_ascii_lowercase();
    let columns: Vec<String> = split_csv_line(&header)
        .into_iter()
        .map(|column| column.trim().trim_matches('"').to_string())
        .collect();
    let index_of = |name: &str| columns.iter().position(|column| column == name);
    let name_at = index_of("name").unwrap_or(0);
    let memory_at = index_of("adapterram");
    let driver_at = index_of("driverversion");

    let mut adapters = Vec::new();
    for line in lines {
        let fields = split_csv_line(line);
        let Some(name) = fields.get(name_at).map(|value| value.trim().to_string()) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        adapters.push(GpuInfo {
            name,
            memory_bytes: memory_at
                .and_then(|index| fields.get(index))
                .and_then(|value| value.trim().parse::<u64>().ok())
                .filter(|bytes| *bytes > 0),
            driver_version: driver_at
                .and_then(|index| fields.get(index))
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
        });
    }
    adapters
}

/// Splits one CSV line, honouring double quotes (PowerShell escapes `"` as `""`).
fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' if in_quotes => {
                if characters.peek() == Some(&'"') {
                    current.push('"');
                    characters.next();
                } else {
                    in_quotes = false;
                }
            }
            '"' => in_quotes = true,
            ',' if !in_quotes => fields.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    fields.push(current);
    fields
}

/// Parses `nvidia-smi --format=csv,noheader,nounits` output (`util, mem_mib, temp` per adapter).
fn parse_nvidia_samples(text: &str) -> Option<GpuSample> {
    let mut sample = GpuSample::default();
    for line in text.lines() {
        let fields: Vec<&str> = line.split(',').map(|field| field.trim()).collect();
        if fields.len() < 3 {
            continue;
        }
        sample.merge(GpuSample {
            utilization_percent: fields[0].parse::<f32>().ok(),
            memory_bytes: fields[1].parse::<f64>().ok().map(mib_to_bytes),
            temperature_c: fields[2].parse::<f32>().ok(),
        });
    }
    sample.is_meaningful().then_some(sample)
}

/// Parses the plain-text output of `rocm-smi --showuse --showmeminfo vram --showtemp`, e.g.
///
/// ```text
/// GPU[0]          : GPU use (%): 42
/// GPU[0]          : Temperature (Sensor edge) (C): 61.0
/// GPU[0]          : VRAM Total Used Memory (B): 1073741824
/// ```
fn parse_rocm_samples(text: &str) -> Option<GpuSample> {
    let mut sample = GpuSample::default();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        let Some((_, value)) = line.rsplit_once(':') else {
            continue;
        };
        let value = value.trim();
        if lower.contains("gpu use") {
            if let Ok(percent) = value.trim_end_matches('%').trim().parse::<f32>() {
                sample.merge(GpuSample {
                    utilization_percent: Some(percent),
                    ..Default::default()
                });
            }
        } else if lower.contains("used memory") && lower.contains("(b)") {
            if let Ok(bytes) = value.parse::<u64>() {
                sample.merge(GpuSample {
                    memory_bytes: Some(bytes),
                    ..Default::default()
                });
            }
        } else if lower.contains("temperature") {
            let cleaned: String = value
                .chars()
                .filter(|character| character.is_ascii_digit() || *character == '.')
                .collect();
            if let Ok(temperature) = cleaned.parse::<f32>() {
                if temperature > 1.0 {
                    sample.merge(GpuSample {
                        temperature_c: Some(temperature),
                        ..Default::default()
                    });
                }
            }
        }
    }
    sample.is_meaningful().then_some(sample)
}

/// True when a program can be started (used to pick the vendor tool).
fn program_available(program: &str) -> bool {
    let probe = if cfg!(windows) { "where" } else { "which" };
    Command::new(probe)
        .arg(program)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Runs a vendor tool and returns its stdout, giving up after [`TOOL_TIMEOUT`].
///
/// A missing or misbehaving GPU tool must never make a measurement fail, so every error path
/// returns `None`.
fn run_tool(program: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    // Drain on a thread so a chatty tool cannot block on a full pipe while we wait for it.
    let mut stdout = child.stdout.take()?;
    let reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => {
                let _ = reader.join();
                return None;
            }
            Ok(None) if started.elapsed() > TOOL_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return None;
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                let _ = reader.join();
                return None;
            }
        }
    }
    reader.join().ok()
}

/// Removes adapters with duplicate or empty names (case-insensitive), keeping the first.
fn dedupe(adapters: Vec<GpuInfo>) -> Vec<GpuInfo> {
    let mut seen: Vec<String> = Vec::new();
    let mut unique = Vec::new();
    for adapter in adapters {
        let key = adapter.name.trim().to_ascii_lowercase();
        if key.is_empty() || seen.contains(&key) {
            continue;
        }
        seen.push(key);
        unique.push(adapter);
    }
    unique
}

/// Converts mebibytes (what the vendor tools report) to bytes.
fn mib_to_bytes(mebibytes: f64) -> u64 {
    (mebibytes.max(0.0) * 1024.0 * 1024.0) as u64
}

/// `max` for optional floats, treating `None` as "no reading".
fn max_f32(current: Option<f32>, candidate: Option<f32>) -> Option<f32> {
    match (current, candidate) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvidia_samples_keep_the_busiest_adapter() {
        let sample = parse_nvidia_samples("37, 4096, 61\n88, 8192, 74\n").expect("sample");
        assert_eq!(sample.utilization_percent, Some(88.0));
        assert_eq!(sample.temperature_c, Some(74.0));
        assert_eq!(sample.memory_bytes, Some(mib_to_bytes(8192.0)));
    }

    #[test]
    fn unsupported_nvidia_answers_are_ignored() {
        assert!(parse_nvidia_samples("[Not Supported]").is_none());
        assert!(parse_nvidia_samples("").is_none());
    }

    #[test]
    fn amd_output_is_parsed_from_plain_text() {
        let text = "\
======================= ROCm System Management Interface =======================
GPU[0]          : GPU use (%): 42
GPU[0]          : VRAM Total Memory (B): 17163091968
GPU[0]          : VRAM Total Used Memory (B): 1073741824
GPU[0]          : Temperature (Sensor edge) (C): 61.0
GPU[0]          : Temperature (Sensor junction) (C): 68.5
================================================================================
";
        let sample = parse_rocm_samples(text).expect("sample");
        assert_eq!(sample.utilization_percent, Some(42.0));
        assert_eq!(sample.memory_bytes, Some(1_073_741_824));
        assert_eq!(sample.temperature_c, Some(68.5));
    }

    #[test]
    fn windows_csv_is_parsed_in_header_order() {
        let csv = "\"Name\",\"AdapterRAM\",\"DriverVersion\"\n\
                   \"Intel(R) UHD Graphics 750\",\"2147483648\",\"31.0.101.4577\"\n\
                   \"NVIDIA GeForce RTX 4070, Laptop GPU\",\"8589934592\",\"536.99\"\n";
        let adapters = parse_windows_csv(csv);
        assert_eq!(adapters.len(), 2);
        assert_eq!(adapters[0].name, "Intel(R) UHD Graphics 750");
        assert_eq!(adapters[0].memory_bytes, Some(2_147_483_648));
        assert_eq!(adapters[0].driver_version.as_deref(), Some("31.0.101.4577"));
        assert_eq!(adapters[1].name, "NVIDIA GeForce RTX 4070, Laptop GPU");
        assert_eq!(adapters[1].memory_bytes, Some(8_589_934_592));
    }

    #[test]
    fn csv_splitting_handles_quotes_commas_and_missing_columns() {
        assert_eq!(
            split_csv_line("\"a,b\",\"c\""),
            vec!["a,b".to_string(), "c".to_string()]
        );
        assert_eq!(
            split_csv_line("\"he said \"\"hi\"\"\",\"x\""),
            vec!["he said \"hi\"".to_string(), "x".to_string()]
        );
        let adapters = parse_windows_csv("\"Name\",\"AdapterRAM\"\n\"Virtual Adapter\",\"\"\n");
        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].memory_bytes, None);
    }

    #[test]
    fn duplicate_adapters_are_merged() {
        let adapters = dedupe(vec![
            GpuInfo {
                name: "Intel UHD 750".to_string(),
                memory_bytes: None,
                driver_version: None,
            },
            GpuInfo {
                name: "intel uhd 750".to_string(),
                memory_bytes: Some(1024),
                driver_version: None,
            },
        ]);
        assert_eq!(adapters.len(), 1);
        assert_eq!(adapters[0].summary(), "Intel UHD 750");
    }

    #[test]
    fn gpu_detection_is_fail_soft() {
        // Detection must work (or return nothing) on any machine: no panics, no empty names.
        let adapters = detect_gpu_info();
        eprintln!("detected adapters: {adapters:?}");
        for adapter in &adapters {
            assert!(
                !adapter.name.trim().is_empty(),
                "adapters must have a name: {adapter:?}"
            );
        }
        // The cache must return the same list on the second call.
        assert_eq!(adapters.len(), detect_gpu_info().len());
    }
}
