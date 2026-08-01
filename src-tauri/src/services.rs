use crate::kiwix::KiwixClient;
use crate::models::ServiceStatus;
use reqwest::Client;
use std::path::Path;
use thiserror::Error;
use tokio::process::Command;

const BONSAI_HEALTH: &str = "http://127.0.0.1:8080/health";
const WIKIPEDIA_SCRIPT: &str = r"D:\LocalAI\OfflineWikipedia\Start-OfflineWikipedia.ps1";
const BONSAI_SCRIPT: &str = r"D:\LocalAI\Bonsai27B\Start-BonsaiServer.ps1";

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

pub async fn prepare() -> Result<(), ServiceError> {
    let current = status().await;
    if current.wikipedia != "ready" {
        run_script("offline Wikipedia", WIKIPEDIA_SCRIPT).await?;
    }
    if current.bonsai != "ready" {
        run_script("Bonsai", BONSAI_SCRIPT).await?;
    }
    Ok(())
}

async fn run_script(name: &str, script: &str) -> Result<(), ServiceError> {
    if !Path::new(script).is_file() {
        return Err(ServiceError::MissingScript(script.into()));
    }
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        script,
    ]);
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
