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
const WIKIPEDIA_SCRIPT: &str = r"D:\LocalAI\OfflineWikipedia\Start-OfflineWikipedia.ps1";

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("required local service script is missing: {0}")]
    MissingScript(String),
    #[error("could not start {name}: {details}")]
    StartFailed { name: String, details: String },
    #[error("local service check failed: {0}")]
    Request(#[from] reqwest::Error),
}

pub async fn status() -> ServiceStatus {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .expect("HTTP client");
    let bonsai_check = client.get(BONSAI_HEALTH).send();
    let kiwix = KiwixClient::new();
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
        archive: "Wikipedia EN · Jan 2024".into(),
        offline_only: true,
    }
}

pub async fn prepare_with_root(bonsai_root: &str) -> Result<(), ServiceError> {
    let current = status().await;
    if current.wikipedia != "ready" {
        run_script("offline Wikipedia", Path::new(WIKIPEDIA_SCRIPT)).await?;
    }
    if current.bonsai != "ready" {
        run_script(
            "Bonsai",
            &Path::new(bonsai_root).join("Start-BonsaiServer.ps1"),
        )
        .await?;
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

pub async fn system_snapshot(settings: ResearchSettings) -> SystemSnapshot {
    let status = status().await;
    let gpu = gpu_snapshot().await;
    let runtime = runtime_snapshot(&settings);
    SystemSnapshot {
        status,
        gpu,
        runtime,
        settings,
    }
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

async fn gpu_snapshot() -> Option<GpuSnapshot> {
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
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
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
