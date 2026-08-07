use crate::config::without_utf8_bom;
use crate::kiwix::KiwixClient;
use crate::models::{
    GpuSnapshot, ResearchSettings, RuntimeSnapshot, ServiceStatus, SystemSnapshot,
};
use reqwest::Client;
use serde_json::Value;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::process::Command;

const BONSAI_HEALTH: &str = "http://127.0.0.1:8080/health";

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("required local service script is missing: {0}")]
    MissingScript(String),
    #[error("could not start {name}: {details}")]
    StartFailed { name: String, details: String },
    #[error("could not stop {name}: {details}")]
    StopFailed { name: String, details: String },
    #[error("local service check failed: {0}")]
    Request(#[from] reqwest::Error),
}

pub async fn status(settings: &ResearchSettings) -> ServiceStatus {
    let client = Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .expect("HTTP client");
    let bonsai_check = client.get(BONSAI_HEALTH).send();
    let kiwix = KiwixClient::new(settings.wikipedia_book.clone());
    let (bonsai, wikipedia) = tokio::join!(bonsai_check, kiwix.health());
    ServiceStatus {
        bonsai: if bonsai
            .map(|response| response.status().is_success())
            .unwrap_or(false)
        {
            "ready"
        } else {
            "stopped"
        }
        .into(),
        wikipedia: if wikipedia { "ready" } else { "stopped" }.into(),
        model: "Ternary Bonsai 27B".into(),
        archive: format!("Wikipedia EN · {}", settings.wikipedia_snapshot),
        offline_only: true,
    }
}

pub async fn prepare(settings: &ResearchSettings) -> Result<(), ServiceError> {
    let current = tokio::time::timeout(std::time::Duration::from_secs(15), status(settings))
        .await
        .map_err(|_| ServiceError::StartFailed {
            name: "local service check".into(),
            details: "Bonsai/Wikipedia status did not finish within 15 seconds".into(),
        })?;
    if current.wikipedia != "ready" {
        start_kiwix(settings).await?;
    }
    if current.bonsai != "ready" {
        let script = Path::new(&settings.bonsai_root).join("Start-BonsaiServer.ps1");
        if script.is_file() {
            run_script("Bonsai", &script).await?;
        }
    }
    Ok(())
}

pub async fn restart_bonsai(bonsai_root: &str) -> Result<(), ServiceError> {
    run_script(
        "Bonsai",
        &Path::new(bonsai_root).join("Start-BonsaiServer.ps1"),
    )
    .await
}

/// Stop the configured Bonsai server and telemetry proxy, but no other model host. Executable
/// paths and the server's private backend port are verified inside the fixed PowerShell program.
pub async fn stop_bonsai(bonsai_root: &str) -> Result<Vec<u32>, ServiceError> {
    const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
$root=[IO.Path]::GetFullPath($env:KESTREL_BONSAI_ROOT)
$server=[IO.Path]::GetFullPath((Join-Path $root 'runtime\llama-server.exe'))
$proxy=[IO.Path]::GetFullPath((Join-Path $root 'BonsaiTelemetryProxy.exe'))
$targets=@(Get-CimInstance Win32_Process | Where-Object {
  ($_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath) -ieq $proxy) -or
  ($_.ExecutablePath -and [IO.Path]::GetFullPath($_.ExecutablePath) -ieq $server -and $_.CommandLine -match '(--port|-p)\s+8081(?:\s|$)')
})
foreach($item in $targets){
  try {
    Stop-Process -Id $item.ProcessId -Force -ErrorAction Stop
    Write-Output $item.ProcessId
  } catch {
    Write-Warning "Could not stop Bonsai process $($item.ProcessId): $($_.Exception.Message)"
  }
}"#;
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        SCRIPT,
    ]);
    command.env("KESTREL_BONSAI_ROOT", bonsai_root);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = command
        .output()
        .await
        .map_err(|error| ServiceError::StopFailed {
            name: "Bonsai shutdown".into(),
            details: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(ServiceError::StopFailed {
            name: "Bonsai shutdown".into(),
            details: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect())
}

pub async fn system_snapshot(settings: ResearchSettings) -> SystemSnapshot {
    let status = status(&settings).await;
    let gpu = gpu_snapshot().await;
    let mut runtime = runtime_snapshot(&settings);
    if status.bonsai != "ready" {
        runtime.model_vram_mib = 0;
    }
    SystemSnapshot {
        status,
        gpu,
        runtime,
        settings,
    }
}

pub async fn start_comfy(comfy_root: &str) -> Result<(), ServiceError> {
    let client = Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .expect("HTTP client");
    let endpoint = "http://127.0.0.1:8188/system_stats";
    if client
        .get(endpoint)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
    {
        return Ok(());
    }
    let script = Path::new(comfy_root).join("Start-ComfyUI-MiniMax-H3.ps1");
    if !script.is_file() {
        return Err(ServiceError::MissingScript(format!(
            "MiniMax H3 launcher is missing: {}. Open Setup and install or locate Movie Studio.",
            script.display()
        )));
    }
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .args(["-Port", "8188", "-NoBrowser"])
        .current_dir(comfy_root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(false);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command.spawn().map_err(|error| ServiceError::StartFailed {
        name: "ComfyUI".into(),
        details: error.to_string(),
    })?;
    for _ in 0..180 {
        if client
            .get(endpoint)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Err(ServiceError::StartFailed {
        name: "ComfyUI".into(),
        details: "it did not become ready within six minutes".into(),
    })
}

async fn start_kiwix(settings: &ResearchSettings) -> Result<(), ServiceError> {
    let server = Path::new(&settings.kiwix_server_path);
    let archive = Path::new(&settings.wikipedia_zim_path);
    if !server.is_file() {
        return Err(ServiceError::MissingScript(format!(
            "Kiwix server is missing: {}. Open Setup to install it or choose an existing copy.",
            server.display()
        )));
    }
    if !archive.is_file() {
        return Err(ServiceError::MissingScript(format!(
            "Wikipedia archive is missing: {}. Open Setup to install it or choose an existing .zim file.",
            archive.display()
        )));
    }
    stop_configured_kiwix(server).await?;
    let mut command = Command::new(server);
    command
        .arg("--address=127.0.0.1")
        .arg("--port=8085")
        .arg("--blockexternal")
        .arg(archive);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command.spawn().map_err(|error| ServiceError::StartFailed {
        name: "offline Wikipedia".into(),
        details: error.to_string(),
    })?;
    let kiwix = KiwixClient::new(settings.wikipedia_book.clone());
    for _ in 0..40 {
        if kiwix.health().await {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(ServiceError::StartFailed {
        name: "offline Wikipedia".into(),
        details: "Kiwix was started but did not answer on 127.0.0.1:8085 within 20 seconds".into(),
    })
}

async fn stop_configured_kiwix(server: &Path) -> Result<(), ServiceError> {
    const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
$server=[IO.Path]::GetFullPath($env:KESTREL_KIWIX_SERVER)
$targets=@(Get-CimInstance Win32_Process | Where-Object {
  $_.ExecutablePath -and
  [IO.Path]::GetFullPath($_.ExecutablePath) -ieq $server -and
  $_.CommandLine -match '(--port(?:=|\s+)8085|-p\s+8085)(?:\s|$)'
})
foreach($item in $targets){ Stop-Process -Id $item.ProcessId -Force -ErrorAction Stop }
"#;
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        SCRIPT,
    ]);
    command.env("KESTREL_KIWIX_SERVER", server);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = command
        .output()
        .await
        .map_err(|error| ServiceError::StopFailed {
            name: "offline Wikipedia".into(),
            details: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(ServiceError::StopFailed {
            name: "offline Wikipedia".into(),
            details: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(())
}

fn runtime_snapshot(settings: &ResearchSettings) -> RuntimeSnapshot {
    let root = PathBuf::from(&settings.bonsai_root);
    let runtime_settings = read_json(&root.join("settings.json"));
    let vram = read_json(&root.join("logs").join("vram-session.json"));
    RuntimeSnapshot {
        context_window: number(&runtime_settings, "ContextWindow")
            .unwrap_or(settings.context_window as u64) as u32,
        max_output_tokens: number(&runtime_settings, "MainMaxOutputTokens")
            .unwrap_or(settings.max_output_tokens as u64) as u32,
        parallel_slots: 1,
        kv_cache: format!(
            "{} / {}",
            text_at(&vram, "/KvKeyType").unwrap_or("q4_0"),
            text_at(&vram, "/KvValueType").unwrap_or("q4_0")
        ),
        model_vram_mib: number(&vram, "BonsaiLoadedDeltaMiB").unwrap_or(0),
        model_root: settings.bonsai_root.clone(),
    }
}

pub async fn gpu_snapshot() -> Option<GpuSnapshot> {
    let mut command = Command::new("nvidia-smi.exe");
    command.args([
        "--query-gpu=name,memory.total,memory.used,memory.free,utilization.gpu",
        "--format=csv,noheader,nounits",
    ]);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = command.output().await.ok()?;
    let line = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .to_owned();
    let values = line.split(',').map(str::trim).collect::<Vec<_>>();
    if values.len() < 5 {
        return None;
    }
    Some(GpuSnapshot {
        name: values[0].into(),
        total_mib: values[1].parse().ok()?,
        used_mib: values[2].parse().ok()?,
        free_mib: values[3].parse().ok()?,
        utilization_percent: values[4].parse().ok()?,
    })
}

fn read_json(path: &Path) -> Value {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(without_utf8_bom(&bytes)).ok())
        .unwrap_or(Value::Null)
}

fn number(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

fn text_at<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

async fn run_script(name: &str, script: &Path) -> Result<(), ServiceError> {
    if !script.is_file() {
        return Err(ServiceError::MissingScript(script.display().to_string()));
    }
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(script);
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000);
    }
    let output = command.output().await?;
    if !output.status.success() {
        let details = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(ServiceError::StartFailed {
            name: name.into(),
            details: if details.is_empty() {
                String::from_utf8_lossy(&output.stdout).trim().to_owned()
            } else {
                details
            },
        });
    }
    Ok(())
}

impl From<std::io::Error> for ServiceError {
    fn from(error: std::io::Error) -> Self {
        ServiceError::StartFailed {
            name: "PowerShell".into(),
            details: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_powershell_utf8_bom_json() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("telemetry.json");
        std::fs::write(&path, b"\xEF\xBB\xBF{\"ContextWindow\":98304}").unwrap();
        assert_eq!(number(&read_json(&path), "ContextWindow"), Some(98_304));
    }
}
