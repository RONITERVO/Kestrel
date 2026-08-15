#[cfg(test)]
use crate::config::without_utf8_bom;
use crate::kiwix::KiwixClient;
use crate::models::{
    ControlSettings, GpuSnapshot, ManagedRuntimeSnapshot, ResearchSettings, RuntimeSnapshot,
    ServiceStatus, SystemSnapshot,
};
use crate::studio::ComfyWorkload;
use reqwest::Client;
#[cfg(test)]
use serde_json::Value;
use std::path::Path;
use thiserror::Error;
use tokio::process::Command;

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
    let kiwix = KiwixClient::new(settings.wikipedia_book.clone());
    let wikipedia = kiwix.health().await;
    ServiceStatus {
        model_runtime: "stopped".into(),
        wikipedia: if wikipedia { "ready" } else { "stopped" }.into(),
        model: "No local model selected".into(),
        archive: format!("Wikipedia EN · {}", settings.wikipedia_snapshot),
        offline_only: true,
    }
}

pub async fn prepare(settings: &ResearchSettings) -> Result<(), ServiceError> {
    let current = tokio::time::timeout(std::time::Duration::from_secs(15), status(settings))
        .await
        .map_err(|_| ServiceError::StartFailed {
            name: "local service check".into(),
            details: "offline Wikipedia status did not finish within 15 seconds".into(),
        })?;
    if current.wikipedia != "ready" {
        start_kiwix(settings).await?;
    }
    Ok(())
}

/// Migration cleanup for releases that could launch the old standalone Bonsai service. Current
/// releases never start it; executable paths and its old private port remain strictly verified.
pub async fn stop_legacy_bonsai_service(bonsai_root: &str) -> Result<Vec<u32>, ServiceError> {
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
    let runtime = runtime_snapshot(&settings);
    SystemSnapshot {
        status,
        gpu,
        runtime,
        settings,
        control: ControlSettings::default(),
        models: Vec::new(),
        managed_runtime: ManagedRuntimeSnapshot::default(),
    }
}

pub async fn start_comfy(comfy_root: &str, workload: ComfyWorkload) -> Result<(), ServiceError> {
    let client = Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .expect("HTTP client");
    let port = workload.port().to_string();
    let endpoint = format!("{}/system_stats", workload.base_url());
    if client
        .get(&endpoint)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
    {
        return Ok(());
    }
    let root = Path::new(comfy_root);
    let script = workload
        .script_names()
        .iter()
        .map(|name| root.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| {
            ServiceError::MissingScript(format!(
                "Kestrel's ComfyUI launcher is missing from {}. Open Setup and resume Movie Studio or Music Production.",
                root.display()
            ))
        })?;
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .args(["-Port", &port, "-NoBrowser"])
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
            .get(&endpoint)
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
    RuntimeSnapshot {
        context_window: settings.context_window,
        max_output_tokens: settings.max_output_tokens,
        parallel_slots: 1,
        kv_cache: "managed per model".into(),
        model_vram_mib: 0,
        model_root: String::new(),
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

#[cfg(test)]
fn read_json(path: &Path) -> Value {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(without_utf8_bom(&bytes)).ok())
        .unwrap_or(Value::Null)
}

#[cfg(test)]
fn number(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
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
