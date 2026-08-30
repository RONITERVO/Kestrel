//! Explicit, bounded cleanup for competing NVIDIA GPU processes.
//!
//! This is a producer-triggered maintenance path. It never runs in the background, never resets
//! the driver, and never executes model output. Kestrel-owned, Windows-critical, undisclosed, and
//! graphics-driver processes can never be cleaned here. Common apps are excluded by default but
//! may be explicitly included through Advanced. Native code closes only the exact approved PIDs
//! after revalidation and uses fixed command arguments. A process that survives the ordinary close
//! request may be force-closed only through a second explicit action. That action matches
//! GpuClean's `taskkill /PID <pid> /F` path; if it fails, Kestrel exposes that exact command for an
//! administrator PowerShell instead of attempting a broader or less verifiable fallback.

use crate::{models::GpuSnapshot, services};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;
use tokio::process::Command;

const MAX_GPU_PROCESSES: usize = 256;
const CRITICAL_PROCESS_NAMES: &[&str] = &[
    // Windows shell, security, and GPU driver processes.
    "applicationframehost.exe",
    "appcontrol.exe",
    "amdrssrcext.exe",
    "cnext.exe",
    "crossdeviceresume.exe",
    "csrss.exe",
    "ctfmon.exe",
    "dwm.exe",
    "explorer.exe",
    "fontdrvhost.exe",
    "lockapp.exe",
    "lsass.exe",
    "nvcontainer.exe",
    "nvdisplay.container.exe",
    "nvidia overlay.exe",
    "nvidia share.exe",
    "phoneexperiencehost.exe",
    "radeonsoftware.exe",
    "searchhost.exe",
    "securityhealthsystray.exe",
    "services.exe",
    "shellexperiencehost.exe",
    "shellhost.exe",
    "sihost.exe",
    "smss.exe",
    "startmenuexperiencehost.exe",
    "systemsettings.exe",
    "taskhostw.exe",
    "textinputhost.exe",
    "winlogon.exe",
];
const DEFAULT_EXCLUDED_PROCESS_NAMES: &[&str] = &[
    // Producer workspaces and everyday apps stay out of the default cleanup, but Advanced may
    // explicitly include them after showing the exact process.
    "brave.exe",
    "chatgpt.exe",
    "chrome.exe",
    "cmd.exe",
    "code.exe",
    "codex.exe",
    "conhost.exe",
    "cursor.exe",
    "devenv.exe",
    "discord.exe",
    "firefox.exe",
    "idea64.exe",
    "kitty.exe",
    "ms-teams.exe",
    "msedge.exe",
    "msedgewebview2.exe",
    "nordvpn.exe",
    "notepad++.exe",
    "opera.exe",
    "powershell.exe",
    "pwsh.exe",
    "pycharm64.exe",
    "rider64.exe",
    "slack.exe",
    "spotify.exe",
    "sublime_text.exe",
    "telegram.exe",
    "vivaldi.exe",
    "webstorm64.exe",
    "whatsapp.exe",
    "windowsterminal.exe",
    "zed.exe",
    "zoom.exe",
];
const AI_PROCESS_PATTERNS: &[&str] = &[
    "accelerate",
    "blender",
    "comfy",
    "cuda",
    "deepspeed",
    "ffmpeg",
    "ipython",
    "jupyter",
    "llama",
    "ollama",
    "python",
    "stable-diffusion",
    "torch",
    "triton",
    "unity",
    "unreal",
    "vllm",
    "webui",
];

#[derive(Debug, Error)]
pub enum GpuMemoryError {
    #[error("NVIDIA GPU cleanup is unavailable because nvidia-smi could not be started: {0}")]
    NvidiaSmiUnavailable(String),
    #[error("NVIDIA could not list GPU processes: {0}")]
    QueryFailed(String),
    #[error("NVIDIA reported more than {MAX_GPU_PROCESSES} GPU processes. Close GPU applications manually, then try again")]
    TooManyProcesses,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GpuMemoryProcess {
    pub pid: u32,
    pub name: String,
    pub executable_path: String,
    pub memory_mib: u64,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VramCleanupPreview {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<GpuSnapshot>,
    pub candidates: Vec<GpuMemoryProcess>,
    pub exclusions: Vec<GpuMemoryExclusion>,
    pub candidate_memory_mib: u64,
    pub protected_process_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GpuMemoryExclusion {
    pub process: GpuMemoryProcess,
    pub reason: String,
    pub can_include: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuCleanupFailure {
    pub process: GpuMemoryProcess,
    pub detail: String,
    pub can_force_close: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub powershell_command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VramCleanupResult {
    pub attempted: Vec<GpuMemoryProcess>,
    pub terminated: Vec<GpuMemoryProcess>,
    pub failed: Vec<GpuCleanupFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_gpu: Option<GpuSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after_gpu: Option<GpuSnapshot>,
    pub freed_mib: u64,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct GpuProcessProtection {
    pids: HashSet<u32>,
    roots: Vec<String>,
    paths: HashSet<String>,
    system_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CleanupDisposition {
    Candidate,
    Excluded(String),
    Critical(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminationMode {
    Graceful,
    Force,
}

impl GpuProcessProtection {
    pub fn new(
        pids: impl IntoIterator<Item = u32>,
        roots: impl IntoIterator<Item = PathBuf>,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        let mut protection = Self {
            pids: pids.into_iter().collect(),
            roots: roots
                .into_iter()
                .filter_map(|path| normalise_path(&path))
                .collect(),
            paths: paths
                .into_iter()
                .filter_map(|path| normalise_path(&path))
                .collect(),
            system_root: std::env::var_os("SystemRoot")
                .and_then(|root| normalise_path(Path::new(&root))),
        };
        protection.roots.sort();
        protection.roots.dedup();
        protection
    }

    fn disposition(&self, process: &GpuMemoryProcess) -> CleanupDisposition {
        if self.pids.contains(&process.pid) {
            return CleanupDisposition::Critical(
                "Kestrel owns this process. Use Release Kestrel AI memory instead.".into(),
            );
        }
        let name = process.name.to_ascii_lowercase();
        if CRITICAL_PROCESS_NAMES.contains(&name.as_str()) {
            return CleanupDisposition::Critical(
                "Windows or the graphics driver requires this process.".into(),
            );
        }
        if process
            .executable_path
            .eq_ignore_ascii_case("[Insufficient Permissions]")
        {
            return CleanupDisposition::Critical(
                "Windows did not disclose this process path, so Kestrel cannot verify it safely."
                    .into(),
            );
        }
        let Some(path) = normalise_path(Path::new(&process.executable_path)) else {
            return CleanupDisposition::Critical(
                "Kestrel cannot verify this process path safely.".into(),
            );
        };
        if self.paths.contains(&path) {
            return CleanupDisposition::Critical(
                "Kestrel owns this executable. Use Release Kestrel AI memory instead.".into(),
            );
        }
        if self.roots.iter().any(|root| path_is_within(&path, root)) {
            return CleanupDisposition::Critical(
                "This process runs from Kestrel's managed installation.".into(),
            );
        }
        if self
            .system_root
            .as_ref()
            .is_some_and(|root| path_is_within(&path, root))
        {
            return CleanupDisposition::Critical("This is a Windows system process.".into());
        }
        if DEFAULT_EXCLUDED_PROCESS_NAMES.contains(&name.as_str()) {
            return CleanupDisposition::Excluded(
                "Excluded by default to protect everyday apps and producer workspaces.".into(),
            );
        }
        CleanupDisposition::Candidate
    }
}

pub async fn preview(
    protection: &GpuProcessProtection,
    gpu: Option<GpuSnapshot>,
) -> Result<VramCleanupPreview, GpuMemoryError> {
    let processes = scan_gpu_processes().await?;
    Ok(build_preview(processes, protection, gpu))
}

pub async fn clean(
    protection: &GpuProcessProtection,
    approved_pids: &HashSet<u32>,
) -> Result<VramCleanupResult, GpuMemoryError> {
    let attempted = select_approved(scan_gpu_processes().await?, protection, approved_pids);
    run_cleanup(attempted, TerminationMode::Graceful).await
}

pub async fn force_clean(
    protection: &GpuProcessProtection,
    expected_processes: &[GpuMemoryProcess],
) -> Result<VramCleanupResult, GpuMemoryError> {
    let attempted = select_expected(scan_gpu_processes().await?, protection, expected_processes);
    run_cleanup(attempted, TerminationMode::Force).await
}

async fn run_cleanup(
    attempted: Vec<GpuMemoryProcess>,
    mode: TerminationMode,
) -> Result<VramCleanupResult, GpuMemoryError> {
    let before_gpu = services::gpu_snapshot().await;
    let mut actions = Vec::with_capacity(attempted.len());
    for process in &attempted {
        actions.push((process.clone(), terminate_process(process.pid, mode).await));
    }

    if !actions.is_empty() {
        tokio::time::sleep(Duration::from_millis(750)).await;
    }
    // A process may exit between inspection and termination. Re-querying makes that a successful
    // cleanup, while a process that still owns a GPU context is reported honestly as a failure.
    let remaining = scan_gpu_processes().await.ok();
    let mut terminated = Vec::new();
    let mut failed = Vec::new();
    for (process, action) in actions {
        let still_present = remaining.as_ref().is_some_and(|processes| {
            processes.iter().any(|candidate| {
                candidate.pid == process.pid
                    && candidate
                        .executable_path
                        .eq_ignore_ascii_case(&process.executable_path)
            })
        });
        match (still_present, action) {
            (false, _) if remaining.is_some() => terminated.push(process),
            (false, Ok(())) => terminated.push(process),
            (true, Ok(())) => failed.push(cleanup_failure(
                process,
                if mode == TerminationMode::Force {
                    "Windows accepted the force-close request, but the process still owns GPU memory. It may have restarted or may require administrator rights."
                } else {
                    "Windows accepted the close request, but the process still owns GPU memory. Force close it to match GpuClean's cleanup level."
                },
                mode,
            )),
            (_, Err(detail)) => failed.push(cleanup_failure(process, &detail, mode)),
        }
    }
    let after_gpu = services::gpu_snapshot().await;
    let freed_mib = match (&before_gpu, &after_gpu) {
        (Some(before), Some(after)) => before.used_mib.saturating_sub(after.used_mib),
        _ => 0,
    };
    let message = cleanup_message(terminated.len(), failed.len(), after_gpu.as_ref());
    Ok(VramCleanupResult {
        attempted,
        terminated,
        failed,
        before_gpu,
        after_gpu,
        freed_mib,
        message,
    })
}

fn cleanup_failure(
    process: GpuMemoryProcess,
    detail: &str,
    mode: TerminationMode,
) -> GpuCleanupFailure {
    let forced = mode == TerminationMode::Force;
    GpuCleanupFailure {
        powershell_command: forced.then(|| powershell_force_command(process.pid)),
        can_force_close: !forced,
        process,
        detail: detail.into(),
    }
}

fn build_preview(
    processes: Vec<GpuMemoryProcess>,
    protection: &GpuProcessProtection,
    gpu: Option<GpuSnapshot>,
) -> VramCleanupPreview {
    let total_count = processes.len();
    let mut candidates = Vec::new();
    let mut exclusions = Vec::new();
    for process in processes {
        match protection.disposition(&process) {
            CleanupDisposition::Candidate => candidates.push(process),
            CleanupDisposition::Excluded(reason) => exclusions.push(GpuMemoryExclusion {
                process,
                reason,
                can_include: true,
            }),
            CleanupDisposition::Critical(reason) => exclusions.push(GpuMemoryExclusion {
                process,
                reason,
                can_include: false,
            }),
        }
    }
    let candidate_memory_mib = candidates.iter().map(|process| process.memory_mib).sum();
    VramCleanupPreview {
        gpu,
        protected_process_count: total_count.saturating_sub(candidates.len()),
        candidates,
        exclusions,
        candidate_memory_mib,
    }
}

fn select_approved(
    processes: Vec<GpuMemoryProcess>,
    protection: &GpuProcessProtection,
    approved_pids: &HashSet<u32>,
) -> Vec<GpuMemoryProcess> {
    processes
        .into_iter()
        .filter(|process| approved_pids.contains(&process.pid))
        .filter(|process| {
            !matches!(
                protection.disposition(process),
                CleanupDisposition::Critical(_)
            )
        })
        .collect()
}

fn select_expected(
    processes: Vec<GpuMemoryProcess>,
    protection: &GpuProcessProtection,
    expected_processes: &[GpuMemoryProcess],
) -> Vec<GpuMemoryProcess> {
    let expected = expected_processes
        .iter()
        .map(|process| (process.pid, process))
        .collect::<HashMap<_, _>>();
    processes
        .into_iter()
        .filter(|process| {
            expected.get(&process.pid).is_some_and(|expected| {
                expected.name.eq_ignore_ascii_case(&process.name)
                    && expected
                        .executable_path
                        .eq_ignore_ascii_case(&process.executable_path)
            })
        })
        .filter(|process| {
            !matches!(
                protection.disposition(process),
                CleanupDisposition::Critical(_)
            )
        })
        .collect()
}

async fn scan_gpu_processes() -> Result<Vec<GpuMemoryProcess>, GpuMemoryError> {
    #[cfg(windows)]
    let executable = "nvidia-smi.exe";
    #[cfg(not(windows))]
    let executable = "nvidia-smi";
    let mut command = Command::new(executable);
    command.args([
        "--query-compute-apps=pid,used_gpu_memory,process_name",
        "--format=csv,noheader,nounits",
    ]);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = command
        .output()
        .await
        .map_err(|error| GpuMemoryError::NvidiaSmiUnavailable(error.to_string()))?;
    if !output.status.success() {
        return Err(GpuMemoryError::QueryFailed(command_detail(&output)));
    }
    parse_processes(&String::from_utf8_lossy(&output.stdout))
}

fn parse_processes(output: &str) -> Result<Vec<GpuMemoryProcess>, GpuMemoryError> {
    let mut by_pid = HashMap::<u32, GpuMemoryProcess>::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut values = line.splitn(3, ',').map(|value| unquote(value.trim()));
        let Some(pid) = values.next().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        let memory_mib = values
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        let executable_path = values.next().unwrap_or_default().trim().to_string();
        if executable_path.is_empty() {
            continue;
        }
        let name = if executable_path.eq_ignore_ascii_case("[Insufficient Permissions]") {
            format!("Process {pid}")
        } else {
            file_name(&executable_path).to_string()
        };
        let kind = if AI_PROCESS_PATTERNS
            .iter()
            .any(|pattern| name.to_ascii_lowercase().contains(pattern))
        {
            "AI / compute"
        } else {
            "GPU application"
        }
        .to_string();
        by_pid
            .entry(pid)
            .and_modify(|process| {
                process.memory_mib = process.memory_mib.saturating_add(memory_mib)
            })
            .or_insert(GpuMemoryProcess {
                pid,
                name,
                executable_path,
                memory_mib,
                kind,
            });
        if by_pid.len() > MAX_GPU_PROCESSES {
            return Err(GpuMemoryError::TooManyProcesses);
        }
    }
    let mut processes = by_pid.into_values().collect::<Vec<_>>();
    processes.sort_by(|left, right| {
        right
            .memory_mib
            .cmp(&left.memory_mib)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.pid.cmp(&right.pid))
    });
    Ok(processes)
}

async fn terminate_process(pid: u32, mode: TerminationMode) -> Result<(), String> {
    #[cfg(windows)]
    let mut command = {
        let mut command = Command::new("taskkill.exe");
        command.args(["/PID", &pid.to_string()]);
        if mode == TerminationMode::Force {
            command.arg("/F");
        }
        command.creation_flags(0x08000000);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = Command::new("kill");
        command.args([
            if mode == TerminationMode::Force {
                "-KILL"
            } else {
                "-TERM"
            },
            &pid.to_string(),
        ]);
        command
    };
    let output = command
        .output()
        .await
        .map_err(|error| format!("Could not ask process {pid} to close: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Could not close process {pid}: {}",
            command_detail(&output)
        ))
    }
}

fn powershell_force_command(pid: u32) -> String {
    // This is deliberately the same fixed force operation used by GpuClean. `pid` is a native
    // integer, so no process-supplied text can enter the copyable command.
    format!("taskkill.exe /PID {pid} /F")
}

fn cleanup_message(terminated: usize, failed: usize, gpu: Option<&GpuSnapshot>) -> String {
    if terminated == 0 && failed == 0 {
        return "VRAM is ready. No competing GPU applications needed to be closed.".into();
    }
    let gpu_detail = gpu
        .map(|gpu| format!(" {} MiB is now free.", gpu.free_mib))
        .unwrap_or_default();
    if failed == 0 {
        format!(
            "Closed {terminated} competing GPU process{}.{}",
            if terminated == 1 { "" } else { "es" },
            gpu_detail
        )
    } else {
        format!(
            "Closed {terminated} competing GPU process{}; {failed} could not be closed.{}",
            if terminated == 1 { "" } else { "es" },
            gpu_detail
        )
    }
}

fn command_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    if detail.is_empty() {
        format!("command exited with {}", output.status)
    } else {
        detail.chars().take(500).collect()
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn normalise_path(path: &Path) -> Option<String> {
    if path.as_os_str().is_empty() {
        return None;
    }
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut value = resolved.to_string_lossy().replace('/', "\\");
    if let Some(stripped) = value.strip_prefix(r"\\?\") {
        value = stripped.to_string();
    }
    while value.ends_with('\\') && value.len() > 3 {
        value.pop();
    }
    Some(value.to_ascii_lowercase())
}

fn path_is_within(path: &str, root: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|remainder| remainder.starts_with('\\'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, path: &str, memory_mib: u64) -> GpuMemoryProcess {
        GpuMemoryProcess {
            pid,
            name: file_name(path).into(),
            executable_path: path.into(),
            memory_mib,
            kind: "GPU application".into(),
        }
    }

    #[test]
    fn parses_and_aggregates_nvidia_compute_rows() {
        let rows = concat!(
            "4100, 3072, C:\\AI\\python.exe\n",
            "4100, 1024, C:\\AI\\python.exe\n",
            "5200, N/A, C:\\Apps\\render,worker.exe\n",
            "5300, N/A, [Insufficient Permissions]\n",
        );
        let parsed = parse_processes(rows).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].pid, 4100);
        assert_eq!(parsed[0].memory_mib, 4096);
        assert_eq!(parsed[0].kind, "AI / compute");
        assert!(parsed
            .iter()
            .any(|process| process.name == "render,worker.exe"));
        assert!(parsed.iter().any(|process| process.name == "Process 5300"));
    }

    #[test]
    fn separates_critical_processes_from_advanced_exclusions() {
        let protection = GpuProcessProtection::new(
            [7000],
            [PathBuf::from(r"C:\Kestrel AI")],
            [PathBuf::from(r"D:\Tools\llama-server.exe")],
        );
        let preview = build_preview(
            vec![
                process(7000, r"D:\Other\python.exe", 900),
                process(7001, r"C:\Kestrel AI\ComfyUI\python.exe", 800),
                process(7002, r"D:\Tools\llama-server.exe", 700),
                process(7003, r"C:\Program Files\Google\Chrome\chrome.exe", 600),
                process(7004, r"D:\Other\ollama.exe", 500),
                process(7005, r"D:\Driver\nvcontainer.exe", 400),
            ],
            &protection,
            None,
        );
        assert_eq!(preview.protected_process_count, 5);
        assert_eq!(preview.candidates.len(), 1);
        assert_eq!(preview.candidates[0].pid, 7004);
        assert_eq!(preview.candidate_memory_mib, 500);
        assert_eq!(preview.exclusions.len(), 5);
        assert!(preview
            .exclusions
            .iter()
            .find(|item| item.process.pid == 7003)
            .is_some_and(|item| item.can_include));
        assert!(preview
            .exclusions
            .iter()
            .filter(|item| item.process.pid != 7003)
            .all(|item| !item.can_include));
    }

    #[test]
    fn root_comparison_does_not_protect_a_similarly_named_directory() {
        let protection = GpuProcessProtection::new([], [PathBuf::from(r"C:\Kestrel AI")], []);
        assert_eq!(
            protection.disposition(&process(42, r"C:\Kestrel AI Other\python.exe", 100)),
            CleanupDisposition::Candidate
        );
    }

    #[test]
    fn explicit_approval_can_include_default_exclusions_but_never_critical_processes() {
        let protection = GpuProcessProtection::new([7000], [], []);
        let selected = select_approved(
            vec![
                process(7000, r"D:\Other\python.exe", 900),
                process(7001, r"C:\Program Files\Google\Chrome\chrome.exe", 600),
                process(7002, r"D:\Other\ollama.exe", 500),
                process(7003, r"D:\Driver\nvcontainer.exe", 400),
            ],
            &protection,
            &[7000, 7001, 7002, 7003].into_iter().collect(),
        );
        assert_eq!(
            selected
                .iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![7001, 7002]
        );
    }

    #[test]
    fn force_selection_requires_the_same_process_identity_and_still_rejects_critical_processes() {
        let protection = GpuProcessProtection::new([7000], [], []);
        let expected = vec![
            process(7000, r"D:\Other\python.exe", 900),
            process(7001, r"D:\Other\python.exe", 800),
            process(7002, r"D:\Driver\nvcontainer.exe", 400),
            process(7004, r"D:\Other\ollama.exe", 500),
        ];
        let selected = select_expected(
            vec![
                process(7000, r"D:\Other\python.exe", 900),
                process(7001, r"D:\Different\python.exe", 800),
                process(7002, r"D:\Driver\nvcontainer.exe", 400),
                process(7003, r"D:\Other\ollama.exe", 500),
                process(7004, r"D:\Other\ollama.exe", 500),
            ],
            &protection,
            &expected,
        );
        assert_eq!(
            selected
                .iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![7004]
        );
    }

    #[test]
    fn manual_force_command_appears_only_after_the_in_app_force_attempt() {
        let process = process(21_352, r"D:\AI\python.exe", 4096);
        let graceful = cleanup_failure(
            process.clone(),
            "This process can only be terminated forcefully.",
            TerminationMode::Graceful,
        );
        assert!(graceful.can_force_close);
        assert_eq!(graceful.powershell_command, None);

        let forced = cleanup_failure(process, "Access is denied.", TerminationMode::Force);
        assert!(!forced.can_force_close);
        assert_eq!(
            forced.powershell_command.as_deref(),
            Some("taskkill.exe /PID 21352 /F")
        );
    }
}
