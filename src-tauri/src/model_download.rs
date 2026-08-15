//! Explicit, durable Hugging Face GGUF transfers.
//!
//! This module is intentionally separate from model discovery and inference. A transfer only runs
//! after a producer starts or resumes it, never resumes itself after restart, and never runs while
//! Kestrel's offline workspace owns the work gate. Partial bytes and a recoverable ledger make an
//! overnight transfer safe to stop without turning public-network access into background behavior.

use chrono::Utc;
use futures_util::StreamExt;
use reqwest::{
    header::{self, HeaderMap},
    redirect, Client, StatusCode,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;
use url::Url;

const LEDGER_SCHEMA_VERSION: u32 = 1;
const MAX_LEDGER_BYTES: u64 = 8 * 1024 * 1024;
const MAX_RETRIES: u32 = 12;
const PERSIST_INTERVAL: Duration = Duration::from_secs(5);
const EMIT_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadRequest {
    pub url: String,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadCandidate {
    pub file_path: String,
    pub file_name: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: Option<String>,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadInspection {
    pub repository: String,
    pub revision: String,
    pub candidates: Vec<ModelDownloadCandidate>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadRecord {
    pub id: String,
    pub status: String,
    pub source_url: String,
    pub repository: String,
    pub revision: String,
    pub file_name: String,
    pub destination_path: String,
    pub partial_path: String,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub bytes_per_second: u64,
    pub eta_seconds: Option<u64>,
    pub expected_sha256: Option<String>,
    pub actual_sha256: Option<String>,
    pub source_etag: Option<String>,
    pub checksum_source: String,
    pub retry_count: u32,
    pub created_at: String,
    pub updated_at: String,
    pub detail: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadLedger {
    schema_version: u32,
    transfers: Vec<ModelDownloadRecord>,
}

#[derive(Debug, Clone)]
pub struct ModelDownloadManager {
    models_root: PathBuf,
    ledger_path: PathBuf,
    records: Arc<Mutex<Vec<ModelDownloadRecord>>>,
}

#[derive(Debug)]
struct DownloadSource {
    url: Url,
    owner: String,
    repository_name: String,
    repository: String,
    revision: String,
    file_path: String,
    file_name: String,
}

#[derive(Debug)]
struct RepositorySource {
    owner: String,
    repository_name: String,
    revision: String,
    exact_file: Option<String>,
}

#[derive(Debug)]
struct RemoteProbe {
    total_bytes: u64,
    etag: Option<String>,
    sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HuggingFaceRepository {
    #[serde(default)]
    siblings: Vec<HuggingFaceFile>,
}

#[derive(Debug, Deserialize)]
struct HuggingFaceFile {
    rfilename: String,
    #[serde(default)]
    size: u64,
    lfs: Option<HuggingFaceLfs>,
}

#[derive(Debug, Deserialize)]
struct HuggingFaceLfs {
    sha256: String,
    size: u64,
}

#[derive(Debug)]
enum AttemptError {
    Cancelled,
    Transient(String),
    Fatal(String),
}

/// Keeps Windows awake only for the lifetime of one producer-approved transfer. The dedicated
/// thread matters because SetThreadExecutionState is thread-scoped while Tokio tasks may migrate.
#[cfg(windows)]
pub struct SystemAwakeGuard {
    stop: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

#[cfg(windows)]
impl SystemAwakeGuard {
    pub fn acquire() -> Result<Self, String> {
        use windows_sys::Win32::System::Power::{
            SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
        };

        let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
        let (stop_sender, stop_receiver) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("kestrel-model-download-awake".into())
            .spawn(move || {
                // SAFETY: This process-wide Windows API accepts only the documented flag bitmask.
                // The same dedicated thread clears its own continuous execution state before exit.
                let enabled =
                    unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) != 0 };
                let _ = ready_sender.send(enabled);
                if enabled {
                    let _ = stop_receiver.recv();
                    // SAFETY: ES_CONTINUOUS alone restores the calling thread's default state.
                    unsafe {
                        SetThreadExecutionState(ES_CONTINUOUS);
                    }
                }
            })
            .map_err(|error| format!("could not start Kestrel's system-awake guard: {error}"))?;
        match ready_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(true) => Ok(Self {
                stop: Some(stop_sender),
                worker: Some(worker),
            }),
            Ok(false) => {
                let _ = worker.join();
                Err("Windows refused Kestrel's request to keep the system awake for this transfer. Check the PC power policy before trying again.".into())
            }
            Err(_) => {
                let _ = stop_sender.send(());
                let _ = worker.join();
                Err("Windows did not acknowledge Kestrel's system-awake request. The transfer was not started.".into())
            }
        }
    }
}

#[cfg(windows)]
impl Drop for SystemAwakeGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(not(windows))]
pub struct SystemAwakeGuard;

#[cfg(not(windows))]
impl SystemAwakeGuard {
    pub fn acquire() -> Result<Self, String> {
        Ok(Self)
    }
}

impl ModelDownloadManager {
    pub fn new(library_root: &Path) -> Result<Self, String> {
        let root = library_root.join("model-downloads");
        let models_root = root.join("models");
        fs::create_dir_all(&models_root).map_err(|error| {
            format!(
                "could not create Kestrel's managed model folder {}: {error}",
                models_root.display()
            )
        })?;
        let ledger_path = root.join("downloads.json");
        let mut records = read_ledger(&ledger_path)?;
        let now = Utc::now().to_rfc3339();
        let mut recovered = false;
        for record in &mut records {
            record.downloaded_bytes = fs::metadata(&record.partial_path)
                .map(|metadata| metadata.len())
                .unwrap_or_else(|_| {
                    fs::metadata(&record.destination_path)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0)
                });
            if matches!(
                record.status.as_str(),
                "inspecting" | "downloading" | "retrying" | "verifying"
            ) {
                record.status = "interrupted".into();
                record.bytes_per_second = 0;
                record.eta_seconds = None;
                record.updated_at.clone_from(&now);
                record.detail = "Kestrel closed during this transfer. Partial bytes are preserved; Resume remains a producer decision.".into();
                record.error = None;
                recovered = true;
            }
        }
        if recovered {
            write_ledger(&ledger_path, &records)?;
        }
        Ok(Self {
            models_root,
            ledger_path,
            records: Arc::new(Mutex::new(records)),
        })
    }

    pub fn models_root(&self) -> &Path {
        &self.models_root
    }

    pub fn list(&self) -> Result<Vec<ModelDownloadRecord>, String> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| "model download ledger is unavailable".to_string())?
            .clone();
        records.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(records)
    }

    pub fn get(&self, id: &str) -> Result<ModelDownloadRecord, String> {
        self.records
            .lock()
            .map_err(|_| "model download ledger is unavailable".to_string())?
            .iter()
            .find(|record| record.id == id)
            .cloned()
            .ok_or_else(|| "unknown model download".to_string())
    }

    pub async fn inspect(&self, value: &str) -> Result<ModelDownloadInspection, String> {
        let source = normalize_repository_source(value)?;
        let client = internet_client()?;
        let metadata = repository_metadata(
            &client,
            &source.owner,
            &source.repository_name,
            &source.revision,
        )
        .await?;
        let mut candidates = metadata
            .siblings
            .into_iter()
            .filter(|file| {
                file.rfilename.to_ascii_lowercase().ends_with(".gguf")
                    && source
                        .exact_file
                        .as_ref()
                        .is_none_or(|exact| exact == &file.rfilename)
            })
            .map(|file| {
                let lfs = file.lfs.as_ref();
                let bytes = lfs.map(|value| value.size).unwrap_or(file.size);
                let sha256 = lfs.and_then(|value| extract_sha256(&value.sha256));
                let url = hugging_face_file_url(
                    &source.owner,
                    &source.repository_name,
                    &source.revision,
                    &file.rfilename,
                )?;
                let lower = file.rfilename.to_ascii_lowercase();
                let kind = if lower.contains("mmproj") {
                    "projector"
                } else if is_numbered_model_shard(&file.rfilename) {
                    "model-shard"
                } else {
                    "model"
                };
                Ok(ModelDownloadCandidate {
                    file_name: file
                        .rfilename
                        .rsplit('/')
                        .next()
                        .unwrap_or(&file.rfilename)
                        .to_string(),
                    file_path: file.rfilename,
                    url: url.to_string(),
                    bytes,
                    sha256,
                    kind: kind.into(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if candidates.len() > 512 {
            return Err("This repository exposes more than 512 GGUF files. Choose an exact file page so Kestrel can inspect one bounded transfer.".into());
        }
        candidates.sort_by(|left, right| {
            left.bytes
                .cmp(&right.bytes)
                .then(left.file_path.cmp(&right.file_path))
        });
        let repository = format!("{}/{}", source.owner, source.repository_name);
        let detail = if candidates.is_empty() {
            "This repository does not expose a GGUF file at the selected revision. Kestrel runs llama.cpp GGUF models; choose a publisher or quantizer repository that provides GGUF artifacts.".into()
        } else if candidates
            .iter()
            .any(|candidate| candidate.kind == "model-shard")
        {
            "GGUF candidates found. This downloader activates one verified file at a time, so numbered model shards are shown for clarity but cannot be selected. Choose a complete single-file quantization.".into()
        } else {
            format!(
                "{} bounded GGUF candidate(s) found with publisher sizes and checksums.",
                candidates.len()
            )
        };
        Ok(ModelDownloadInspection {
            repository,
            revision: source.revision,
            candidates,
            detail,
        })
    }

    pub async fn start(
        &self,
        request: ModelDownloadRequest,
        app: &AppHandle,
        cancel: &CancellationToken,
    ) -> Result<ModelDownloadRecord, String> {
        let source = normalize_source(&request.url)?;
        let supplied_sha = normalize_sha256(request.expected_sha256.as_deref())?;
        if let Some(existing) = self.list()?.into_iter().find(|record| {
            record.source_url == source.url.as_str()
                && record.status == "complete"
                && Path::new(&record.destination_path).is_file()
        }) {
            emit(app, &existing);
            return Ok(existing);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let repository_dir = self.models_root.join(safe_component(&source.repository));
        fs::create_dir_all(&repository_dir).map_err(|error| error.to_string())?;
        let destination = unique_destination(&repository_dir, &source.file_name, &id);
        let partial = destination.with_extension("gguf.part");
        let now = Utc::now().to_rfc3339();
        let mut record = ModelDownloadRecord {
            id,
            status: "inspecting".into(),
            source_url: source.url.to_string(),
            repository: source.repository,
            revision: source.revision,
            file_name: source.file_name,
            destination_path: destination.to_string_lossy().into_owned(),
            partial_path: partial.to_string_lossy().into_owned(),
            total_bytes: 0,
            downloaded_bytes: 0,
            bytes_per_second: 0,
            eta_seconds: None,
            expected_sha256: supplied_sha,
            actual_sha256: None,
            source_etag: None,
            checksum_source: "pending".into(),
            retry_count: 0,
            created_at: now.clone(),
            updated_at: now,
            detail: "Inspecting the public Hugging Face file before any model bytes are written."
                .into(),
            error: None,
        };
        self.update(&record)?;
        emit(app, &record);
        self.run(&mut record, app, cancel).await
    }

    pub async fn resume(
        &self,
        id: &str,
        app: &AppHandle,
        cancel: &CancellationToken,
    ) -> Result<ModelDownloadRecord, String> {
        let mut record = self.get(id)?;
        if record.status == "complete" {
            return Ok(record);
        }
        if !Path::new(&record.partial_path).is_file() && record.downloaded_bytes > 0 {
            return Err("The recorded partial model file is missing. Start a new transfer from the original Hugging Face URL.".into());
        }
        record.status = "inspecting".into();
        record.detail = "Rechecking the source before resuming preserved bytes.".into();
        record.error = None;
        record.bytes_per_second = 0;
        record.eta_seconds = None;
        record.updated_at = Utc::now().to_rfc3339();
        self.update(&record)?;
        emit(app, &record);
        self.run(&mut record, app, cancel).await
    }

    async fn run(
        &self,
        record: &mut ModelDownloadRecord,
        app: &AppHandle,
        cancel: &CancellationToken,
    ) -> Result<ModelDownloadRecord, String> {
        let source = normalize_source(&record.source_url)?;
        let client = internet_client()?;
        let probe = match tokio::select! {
            _ = cancel.cancelled() => None,
            result = probe_remote(&client, &source) => Some(result),
        } {
            None => {
                record.status = "paused".into();
                record.detail = "Stopped safely while inspecting the source. Any existing partial bytes remain resumable.".into();
                record.error = None;
                record.bytes_per_second = 0;
                record.eta_seconds = None;
                record.updated_at = Utc::now().to_rfc3339();
                self.update(record)?;
                emit(app, record);
                return Ok(record.clone());
            }
            Some(Ok(probe)) => probe,
            Some(Err(error)) => return self.fail(record, app, "failed", error),
        };
        if record.total_bytes > 0
            && record.total_bytes != probe.total_bytes
            && record.downloaded_bytes > 0
        {
            return self.fail(
                record,
                app,
                "source-changed",
                format!(
                    "The remote file size changed from {} to {} bytes. Kestrel did not append new bytes to the old partial file.",
                    record.total_bytes, probe.total_bytes
                ),
            );
        }
        if let (Some(previous), Some(current)) = (&record.source_etag, &probe.etag) {
            if previous != current && record.downloaded_bytes > 0 {
                return self.fail(
                    record,
                    app,
                    "source-changed",
                    "The Hugging Face object identity changed. Kestrel preserved the old partial file and refused to combine two revisions.".into(),
                );
            }
        }
        if let (Some(supplied), Some(upstream)) = (&record.expected_sha256, &probe.sha256) {
            if supplied != upstream {
                return self.fail(
                    record,
                    app,
                    "failed",
                    format!(
                        "The supplied SHA-256 does not match Hugging Face metadata ({upstream})."
                    ),
                );
            }
        }
        record.total_bytes = probe.total_bytes;
        record.source_etag = probe.etag;
        if record.expected_sha256.is_none() {
            record.expected_sha256 = probe.sha256;
            record.checksum_source = if record.expected_sha256.is_some() {
                "hugging-face-lfs".into()
            } else {
                "recorded-after-download".into()
            };
        } else {
            record.checksum_source = "producer-supplied".into();
        }
        let partial = PathBuf::from(&record.partial_path);
        record.downloaded_bytes = fs::metadata(&partial).map(|value| value.len()).unwrap_or(0);
        if record.downloaded_bytes > record.total_bytes {
            return self.fail(
                record,
                app,
                "source-changed",
                "The partial file is larger than the current remote object. It was preserved for inspection and will not be resumed.".into(),
            );
        }
        let parent = partial
            .parent()
            .ok_or_else(|| "model destination has no parent folder".to_string())?;
        let remaining = record.total_bytes.saturating_sub(record.downloaded_bytes);
        let available = fs2::available_space(parent).map_err(|error| error.to_string())?;
        if remaining > available {
            return self.fail(
                record,
                app,
                "failed",
                format!(
                    "The model needs {} more bytes, but the destination has only {} bytes free. Existing partial bytes remain resumable.",
                    remaining, available
                ),
            );
        }
        record.status = "downloading".into();
        record.detail = if record.downloaded_bytes > 0 {
            format!("Resuming {} from its durable checkpoint.", record.file_name)
        } else {
            format!(
                "Downloading {}. Kestrel may remain observed overnight.",
                record.file_name
            )
        };
        record.updated_at = Utc::now().to_rfc3339();
        self.update(record)?;
        emit(app, record);

        let mut retry = 0;
        if record.downloaded_bytes < record.total_bytes {
            loop {
                match self.download_once(&client, record, app, cancel).await {
                    Ok(()) => break,
                    Err(AttemptError::Cancelled) => {
                        record.status = "paused".into();
                        record.detail = "Stopped safely. Partial bytes are durable and will not resume without producer approval.".into();
                        record.error = None;
                        record.bytes_per_second = 0;
                        record.eta_seconds = None;
                        record.downloaded_bytes = fs::metadata(&partial).map(|value| value.len()).unwrap_or(0);
                        record.updated_at = Utc::now().to_rfc3339();
                        self.update(record)?;
                        emit(app, record);
                        return Ok(record.clone());
                    }
                    Err(AttemptError::Transient(error)) if retry < MAX_RETRIES => {
                        retry += 1;
                        record.retry_count = record.retry_count.saturating_add(1);
                        let delay = u64::from(5_u32.saturating_mul(2_u32.saturating_pow((retry - 1).min(4)))).min(60);
                        record.status = "retrying".into();
                        record.detail = format!("The connection paused ({error}). Retrying the preserved byte range in {delay}s ({retry}/{MAX_RETRIES}).");
                        record.error = Some(error);
                        record.bytes_per_second = 0;
                        record.eta_seconds = None;
                        record.downloaded_bytes = fs::metadata(&partial).map(|value| value.len()).unwrap_or(0);
                        record.updated_at = Utc::now().to_rfc3339();
                        self.update(record)?;
                        emit(app, record);
                        tokio::select! {
                            _ = tokio::time::sleep(Duration::from_secs(delay)) => {},
                            _ = cancel.cancelled() => {
                                record.status = "paused".into();
                                record.detail = "Stopped safely while waiting to retry. Partial bytes remain resumable.".into();
                                record.error = None;
                                record.updated_at = Utc::now().to_rfc3339();
                                self.update(record)?;
                                emit(app, record);
                                return Ok(record.clone());
                            }
                        }
                        record.status = "downloading".into();
                    }
                    Err(AttemptError::Transient(error)) => return self.fail(record, app, "interrupted", format!("The network remained unavailable after {MAX_RETRIES} retries: {error}. Resume later to continue from the preserved byte range.")),
                    Err(AttemptError::Fatal(error)) => {
                        return self.fail(record, app, "failed", error)
                    }
                }
            }
        } else if cancel.is_cancelled() {
            record.status = "paused".into();
            record.detail = "Stopped safely before verification. The complete partial file remains ready to verify on Resume.".into();
            record.error = None;
            record.bytes_per_second = 0;
            record.eta_seconds = None;
            record.downloaded_bytes = fs::metadata(&partial).map(|value| value.len()).unwrap_or(0);
            record.updated_at = Utc::now().to_rfc3339();
            self.update(record)?;
            emit(app, record);
            return Ok(record.clone());
        }

        record.status = "verifying".into();
        record.detail = "All bytes arrived. Computing SHA-256 and validating the GGUF container before cataloging it.".into();
        record.downloaded_bytes = record.total_bytes;
        record.bytes_per_second = 0;
        record.eta_seconds = Some(0);
        record.updated_at = Utc::now().to_rfc3339();
        self.update(record)?;
        emit(app, record);
        let hash_path = partial.clone();
        let hash_cancel = cancel.clone();
        let actual = match tokio::task::spawn_blocking(move || {
            sha256_file(&hash_path, Some(&hash_cancel))
        })
        .await
        {
            Ok(Ok(Some(actual))) => actual,
            Ok(Ok(None)) => {
                record.status = "paused".into();
                record.detail = "Stopped safely during checksum verification. The complete partial file remains ready to verify on Resume.".into();
                record.error = None;
                record.bytes_per_second = 0;
                record.eta_seconds = None;
                record.updated_at = Utc::now().to_rfc3339();
                self.update(record)?;
                emit(app, record);
                return Ok(record.clone());
            }
            Ok(Err(error)) => return self.fail(record, app, "failed", error),
            Err(error) => return self.fail(record, app, "failed", error.to_string()),
        };
        record.actual_sha256 = Some(actual.clone());
        if let Some(expected) = &record.expected_sha256 {
            if expected != &actual {
                let corrupt = partial
                    .with_extension(format!("corrupt-{}", Utc::now().format("%Y%m%dT%H%M%S")));
                let _ = fs::rename(&partial, &corrupt);
                return self.fail(
                    record,
                    app,
                    "failed",
                    format!(
                        "SHA-256 verification failed. The received bytes were quarantined at {}.",
                        corrupt.display()
                    ),
                );
            }
        }
        if let Err(error) = validate_gguf(&partial) {
            return self.fail(record, app, "failed", error);
        }
        if !record.file_name.to_ascii_lowercase().contains("mmproj") {
            if let Err(error) = crate::model::inspect_file(&partial) {
                return self.fail(record, app, "failed", format!(
                    "The GGUF container is not a catalogable llama.cpp model: {error}. The complete .part file was preserved."
                ));
            }
        }
        let destination = PathBuf::from(&record.destination_path);
        if destination.exists() {
            return self.fail(
                record,
                app,
                "failed",
                "A file appeared at the final model destination during the transfer. Kestrel preserved both files and refused to overwrite either one.".into(),
            );
        }
        if let Err(error) = fs::rename(&partial, &destination) {
            return self.fail(record, app, "failed", format!(
                "the verified model could not be activated at {}: {error}; the .part file remains complete",
                destination.display()
            ));
        }
        record.status = "complete".into();
        record.detail = if record.expected_sha256.is_some() {
            "Verified model installed in Kestrel's managed model library and ready for catalog inspection.".into()
        } else {
            "Model installed and its SHA-256 recorded. Hugging Face did not expose an upstream SHA-256, so Kestrel can prove future stability but not independent publisher identity.".into()
        };
        record.error = None;
        record.updated_at = Utc::now().to_rfc3339();
        self.update(record)?;
        emit(app, record);
        Ok(record.clone())
    }

    async fn download_once(
        &self,
        client: &Client,
        record: &mut ModelDownloadRecord,
        app: &AppHandle,
        cancel: &CancellationToken,
    ) -> Result<(), AttemptError> {
        let partial = PathBuf::from(&record.partial_path);
        let existing = fs::metadata(&partial).map(|value| value.len()).unwrap_or(0);
        let mut request = client.get(&record.source_url);
        if existing > 0 {
            request = request.header(header::RANGE, format!("bytes={existing}-"));
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(AttemptError::Cancelled),
            result = request.send() => result
                .map_err(|error| AttemptError::Transient(error.to_string()))?,
        };
        let resumed = existing > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
        if !response.status().is_success() {
            let status = response.status();
            let message = format!("Hugging Face returned {status}");
            return Err(
                if status == StatusCode::REQUEST_TIMEOUT
                    || status == StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error()
                {
                    AttemptError::Transient(message)
                } else {
                    AttemptError::Fatal(message)
                },
            );
        }
        if resumed {
            validate_content_range(response.headers(), existing, record.total_bytes)
                .map_err(AttemptError::Fatal)?;
        } else if existing > 0 {
            let parent = partial.parent().ok_or_else(|| {
                AttemptError::Fatal("model destination has no parent folder".into())
            })?;
            let available = fs2::available_space(parent)
                .map_err(|error| AttemptError::Fatal(error.to_string()))?;
            if record.total_bytes > available {
                return Err(AttemptError::Fatal(format!(
                    "Hugging Face did not honor byte-range resume, and restarting the full file needs {} bytes while only {available} bytes are free",
                    record.total_bytes
                )));
            }
        }
        let mut file = if resumed {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(&partial)
                .await
                .map_err(|error| AttemptError::Fatal(error.to_string()))?
        } else {
            tokio::fs::File::create(&partial)
                .await
                .map_err(|error| AttemptError::Fatal(error.to_string()))?
        };
        let started_at = Instant::now();
        let started_bytes = if resumed { existing } else { 0 };
        let mut downloaded = started_bytes;
        let mut last_emit = Instant::now() - EMIT_INTERVAL;
        let mut last_persist = Instant::now();
        let mut stream = response.bytes_stream();
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => {
                    file.flush().await.map_err(|error| AttemptError::Fatal(error.to_string()))?;
                    file.sync_data().await.map_err(|error| AttemptError::Fatal(error.to_string()))?;
                    return Err(AttemptError::Cancelled);
                },
                chunk = stream.next() => chunk,
            };
            let Some(chunk) = chunk else { break };
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    file.flush()
                        .await
                        .map_err(|error| AttemptError::Fatal(error.to_string()))?;
                    file.sync_data()
                        .await
                        .map_err(|error| AttemptError::Fatal(error.to_string()))?;
                    return Err(AttemptError::Transient(error.to_string()));
                }
            };
            file.write_all(&chunk)
                .await
                .map_err(|error| AttemptError::Fatal(error.to_string()))?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            if downloaded > record.total_bytes {
                return Err(AttemptError::Fatal(
                    "the server sent more bytes than its declared object size".into(),
                ));
            }
            let elapsed = started_at.elapsed().as_secs().max(1);
            let rate = downloaded.saturating_sub(started_bytes) / elapsed;
            record.downloaded_bytes = downloaded;
            record.bytes_per_second = rate;
            record.eta_seconds =
                (rate > 0).then(|| record.total_bytes.saturating_sub(downloaded).div_ceil(rate));
            record.detail = format!(
                "Downloading {} with a durable byte-range checkpoint.",
                record.file_name
            );
            record.updated_at = Utc::now().to_rfc3339();
            if last_emit.elapsed() >= EMIT_INTERVAL {
                emit(app, record);
                last_emit = Instant::now();
            }
            if last_persist.elapsed() >= PERSIST_INTERVAL {
                file.flush()
                    .await
                    .map_err(|error| AttemptError::Fatal(error.to_string()))?;
                file.sync_data()
                    .await
                    .map_err(|error| AttemptError::Fatal(error.to_string()))?;
                self.update(record).map_err(AttemptError::Fatal)?;
                last_persist = Instant::now();
            }
        }
        file.flush()
            .await
            .map_err(|error| AttemptError::Fatal(error.to_string()))?;
        file.sync_data()
            .await
            .map_err(|error| AttemptError::Fatal(error.to_string()))?;
        drop(file);
        let received = fs::metadata(&partial)
            .map_err(|error| AttemptError::Fatal(error.to_string()))?
            .len();
        if received != record.total_bytes {
            return Err(AttemptError::Transient(format!(
                "the connection ended at {received} of {} bytes",
                record.total_bytes
            )));
        }
        record.downloaded_bytes = received;
        self.update(record).map_err(AttemptError::Fatal)?;
        emit(app, record);
        Ok(())
    }

    fn fail(
        &self,
        record: &mut ModelDownloadRecord,
        app: &AppHandle,
        status: &str,
        error: String,
    ) -> Result<ModelDownloadRecord, String> {
        record.status = status.into();
        record.detail = error.clone();
        record.error = Some(error.clone());
        record.bytes_per_second = 0;
        record.eta_seconds = None;
        record.downloaded_bytes = fs::metadata(&record.partial_path)
            .map(|value| value.len())
            .unwrap_or(record.downloaded_bytes);
        record.updated_at = Utc::now().to_rfc3339();
        self.update(record)?;
        emit(app, record);
        Err(error)
    }

    fn update(&self, record: &ModelDownloadRecord) -> Result<(), String> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| "model download ledger is unavailable".to_string())?;
        if let Some(existing) = records.iter_mut().find(|item| item.id == record.id) {
            existing.clone_from(record);
        } else {
            records.push(record.clone());
        }
        write_ledger(&self.ledger_path, &records)
    }
}

fn normalize_repository_source(value: &str) -> Result<RepositorySource, String> {
    let value = value.trim();
    if value.len() > 4_096 {
        return Err("The Hugging Face URL exceeds Kestrel's 4,096-character safety limit.".into());
    }
    let url = Url::parse(value).map_err(|error| format!("invalid Hugging Face URL: {error}"))?;
    validate_hugging_face_url(&url)?;
    let segments = decoded_path_segments(&url)?;
    if segments.len() < 2 {
        return Err("Paste a Hugging Face model repository or GGUF file URL.".into());
    }
    let (revision, exact_file) = if segments.len() == 2 {
        ("main".into(), None)
    } else if segments.len() >= 4 && segments[2] == "tree" {
        (segments[3].clone(), None)
    } else if segments.len() >= 5 && matches!(segments[2].as_str(), "blob" | "resolve") {
        (segments[3].clone(), Some(segments[4..].join("/")))
    } else {
        return Err(
            "Use a Hugging Face repository page, revision page, or exact GGUF file page.".into(),
        );
    };
    Ok(RepositorySource {
        owner: segments[0].clone(),
        repository_name: segments[1].clone(),
        revision,
        exact_file,
    })
}

fn normalize_source(value: &str) -> Result<DownloadSource, String> {
    let value = value.trim();
    if value.len() > 4_096 {
        return Err("The Hugging Face URL exceeds Kestrel's 4,096-character safety limit.".into());
    }
    let url = Url::parse(value).map_err(|error| format!("invalid Hugging Face URL: {error}"))?;
    validate_hugging_face_url(&url)?;
    let segments = decoded_path_segments(&url)?;
    if segments.len() < 5 || !matches!(segments[2].as_str(), "blob" | "resolve") {
        return Err("Paste a Hugging Face GGUF file URL such as https://huggingface.co/owner/repository/blob/main/model.gguf.".into());
    }
    let file_name = segments.last().cloned().unwrap_or_default();
    if !file_name.to_ascii_lowercase().ends_with(".gguf") || file_name.len() > 240 {
        return Err("The selected Hugging Face file must end in .gguf.".into());
    }
    if file_name.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
    }) {
        return Err("The GGUF file name contains characters that are unsafe on Windows.".into());
    }
    if is_numbered_model_shard(&file_name) {
        return Err("Numbered GGUF model shards are not supported by the observed downloader because a partial shard set must never appear as an installed model. Choose a complete single-file quantization.".into());
    }
    let repository = format!(
        "{}--{}",
        safe_component(&segments[0]),
        safe_component(&segments[1])
    );
    let revision = segments[3].clone();
    let file_path = segments[4..].join("/");
    let url = hugging_face_file_url(&segments[0], &segments[1], &revision, &file_path)?;
    Ok(DownloadSource {
        url,
        owner: segments[0].clone(),
        repository_name: segments[1].clone(),
        repository,
        revision,
        file_path,
        file_name,
    })
}

fn validate_hugging_face_url(url: &Url) -> Result<(), String> {
    if url.scheme() != "https" || url.host_str() != Some("huggingface.co") {
        return Err("Model downloads accept only public https://huggingface.co URLs. Kestrel does not send credentials or follow arbitrary download hosts.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Do not place credentials in a model URL.".into());
    }
    if url.query_pairs().any(|(key, _)| {
        key.eq_ignore_ascii_case("token") || key.eq_ignore_ascii_case("authorization")
    }) {
        return Err("Kestrel will not persist a Hugging Face access token in a download record. Use a public model URL.".into());
    }
    Ok(())
}

fn decoded_path_segments(url: &Url) -> Result<Vec<String>, String> {
    url.path_segments()
        .ok_or_else(|| "Hugging Face URL has no path".to_string())?
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            percent_encoding::percent_decode_str(segment)
                .decode_utf8()
                .map(|value| value.into_owned())
                .map_err(|_| "Hugging Face URL contains invalid UTF-8 path text".to_string())
        })
        .collect()
}

fn hugging_face_file_url(
    owner: &str,
    repository: &str,
    revision: &str,
    file_path: &str,
) -> Result<Url, String> {
    let mut url = Url::parse("https://huggingface.co/").map_err(|error| error.to_string())?;
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| "could not construct a Hugging Face file URL".to_string())?;
    segments.pop_if_empty();
    segments.extend([owner, repository, "resolve", revision]);
    segments.extend(file_path.split('/').filter(|segment| !segment.is_empty()));
    drop(segments);
    Ok(url)
}

fn internet_client() -> Result<Client, String> {
    Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(120))
        .redirect(redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many Hugging Face redirects");
            }
            let url = attempt.url();
            if url.scheme() == "https" && url.host_str().is_some_and(allowed_redirect_host) {
                attempt.follow()
            } else {
                attempt.error("Hugging Face redirected the model to an unapproved host")
            }
        }))
        .user_agent("Kestrel-Local/model-downloader")
        .build()
        .map_err(|error| error.to_string())
}

fn allowed_redirect_host(host: &str) -> bool {
    host == "huggingface.co"
        || host.ends_with(".huggingface.co")
        || host.ends_with(".hf.co")
        || host.ends_with(".xethub.hf.co")
}

async fn probe_remote(client: &Client, source: &DownloadSource) -> Result<RemoteProbe, String> {
    let response = client
        .head(source.url.clone())
        .send()
        .await
        .map_err(|error| format!("could not inspect the Hugging Face file: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Hugging Face returned {} while inspecting the public model file",
            response.status()
        ));
    }
    let headers = response.headers();
    let total_bytes = header_u64(headers, "x-linked-size")
        .or_else(|| header_u64(headers, header::CONTENT_LENGTH.as_str()))
        .filter(|value| *value > 0)
        .ok_or_else(|| "Hugging Face did not provide a stable file size; Kestrel will not begin an unbounded overnight transfer.".to_string())?;
    let linked_etag = header_text(headers, "x-linked-etag");
    let etag = linked_etag
        .clone()
        .or_else(|| header_text(headers, header::ETAG.as_str()));
    let metadata = repository_metadata(
        client,
        &source.owner,
        &source.repository_name,
        &source.revision,
    )
    .await?;
    let sibling = metadata
        .siblings
        .into_iter()
        .find(|file| file.rfilename == source.file_path)
        .ok_or_else(|| {
            "The requested GGUF is not present in the Hugging Face repository metadata for this revision.".to_string()
        })?;
    let metadata_size = sibling
        .lfs
        .as_ref()
        .map(|lfs| lfs.size)
        .unwrap_or(sibling.size);
    if metadata_size > 0 && metadata_size != total_bytes {
        return Err(format!(
            "Hugging Face file metadata disagrees about size ({metadata_size} versus {total_bytes} bytes). Kestrel refused to start an ambiguous transfer."
        ));
    }
    let sha256 = sibling
        .lfs
        .as_ref()
        .and_then(|lfs| extract_sha256(&lfs.sha256))
        .or_else(|| linked_etag.as_deref().and_then(extract_sha256));
    Ok(RemoteProbe {
        total_bytes,
        etag,
        sha256,
    })
}

async fn repository_metadata(
    client: &Client,
    owner: &str,
    repository: &str,
    revision: &str,
) -> Result<HuggingFaceRepository, String> {
    let mut api =
        Url::parse("https://huggingface.co/api/models/").map_err(|error| error.to_string())?;
    let mut segments = api
        .path_segments_mut()
        .map_err(|_| "could not construct the Hugging Face metadata URL".to_string())?;
    segments.pop_if_empty();
    segments.extend([owner, repository]);
    drop(segments);
    api.query_pairs_mut()
        .append_pair("blobs", "true")
        .append_pair("revision", revision);
    let response = client
        .get(api)
        .send()
        .await
        .map_err(|error| format!("could not read Hugging Face file metadata: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Hugging Face returned {} while reading checksum metadata",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_LEDGER_BYTES)
    {
        return Err(
            "Hugging Face repository metadata exceeds Kestrel's 8 MiB safety limit.".into(),
        );
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|error| format!("could not read Hugging Face file metadata: {error}"))?;
        if bytes.len().saturating_add(chunk.len()) as u64 > MAX_LEDGER_BYTES {
            return Err(
                "Hugging Face repository metadata exceeds Kestrel's 8 MiB safety limit.".into(),
            );
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Hugging Face returned invalid repository metadata: {error}"))
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .trim()
                .trim_start_matches("W/")
                .trim_matches('"')
                .to_string()
        })
        .filter(|value| !value.is_empty())
}

fn header_u64(headers: &HeaderMap, name: &str) -> Option<u64> {
    header_text(headers, name)?.parse().ok()
}

fn extract_sha256(value: &str) -> Option<String> {
    let candidate = value
        .rsplit(':')
        .next()
        .unwrap_or(value)
        .trim()
        .trim_matches('"')
        .to_ascii_lowercase();
    (candidate.len() == 64
        && candidate
            .chars()
            .all(|character| character.is_ascii_hexdigit()))
    .then_some(candidate)
}

fn normalize_sha256(value: Option<&str>) -> Result<Option<String>, String> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => extract_sha256(value).map(Some).ok_or_else(|| {
            "Expected SHA-256 must contain exactly 64 hexadecimal characters.".into()
        }),
        None => Ok(None),
    }
}

fn validate_content_range(
    headers: &HeaderMap,
    expected_start: u64,
    expected_total: u64,
) -> Result<(), String> {
    let value = header_text(headers, header::CONTENT_RANGE.as_str()).ok_or_else(|| {
        "the server accepted a resumed request without a Content-Range identity".to_string()
    })?;
    let (range, total) = value
        .strip_prefix("bytes ")
        .and_then(|value| value.split_once('/'))
        .ok_or_else(|| format!("invalid Content-Range from Hugging Face: {value}"))?;
    let start = range
        .split_once('-')
        .and_then(|(start, _)| start.parse::<u64>().ok())
        .ok_or_else(|| format!("invalid Content-Range start from Hugging Face: {value}"))?;
    let total = total
        .parse::<u64>()
        .map_err(|_| format!("invalid Content-Range size from Hugging Face: {value}"))?;
    if start != expected_start || total != expected_total {
        return Err(format!(
            "the resumed byte range did not match the durable checkpoint (expected {expected_start}/{expected_total}, received {start}/{total})"
        ));
    }
    Ok(())
}

fn safe_component(value: &str) -> String {
    let safe = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .take(120)
        .collect::<String>();
    if safe.is_empty() {
        "model".into()
    } else {
        safe
    }
}

fn is_numbered_model_shard(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".gguf").unwrap_or(&lower);
    let Some((before_total, total)) = stem.rsplit_once("-of-") else {
        return false;
    };
    let Some((_, shard)) = before_total.rsplit_once('-') else {
        return false;
    };
    shard.len() == 5
        && total.len() == 5
        && shard.bytes().all(|value| value.is_ascii_digit())
        && total.bytes().all(|value| value.is_ascii_digit())
}

fn unique_destination(parent: &Path, file_name: &str, id: &str) -> PathBuf {
    let preferred = parent.join(file_name);
    if !preferred.exists() && !preferred.with_extension("gguf.part").exists() {
        return preferred;
    }
    let stem = file_name.strip_suffix(".gguf").unwrap_or(file_name);
    parent.join(format!("{}-{}.gguf", safe_component(stem), &id[..8]))
}

fn validate_gguf(path: &Path) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut magic = [0_u8; 4];
    file.read_exact(&mut magic)
        .map_err(|error| error.to_string())?;
    if &magic != b"GGUF" {
        return Err("The downloaded file passed byte hashing but is not a GGUF container. It remains as a complete .part file and was not cataloged.".into());
    }
    Ok(())
}

fn sha256_file(path: &Path, cancel: Option<&CancellationToken>) -> Result<Option<String>, String> {
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 4 * 1024 * 1024];
    loop {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Ok(None);
        }
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(Some(hex::encode(hasher.finalize())))
}

fn read_ledger(path: &Path) -> Result<Vec<ModelDownloadRecord>, String> {
    let backup = path.with_extension("json.backup");
    let read = |candidate: &Path| -> Result<DownloadLedger, String> {
        let metadata = fs::metadata(candidate).map_err(|error| error.to_string())?;
        if metadata.len() > MAX_LEDGER_BYTES {
            return Err("model download ledger exceeds 8 MiB".into());
        }
        serde_json::from_slice(&fs::read(candidate).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())
    };
    let ledger = if path.is_file() {
        match read(path) {
            Ok(value) => Some(value),
            Err(primary) if backup.is_file() => match read(&backup) {
                Ok(value) => {
                    fs::copy(&backup, path).map_err(|error| error.to_string())?;
                    Some(value)
                }
                Err(_) => return Err(primary),
            },
            Err(error) => return Err(error),
        }
    } else if backup.is_file() {
        let value = read(&backup)?;
        fs::copy(&backup, path).map_err(|error| error.to_string())?;
        Some(value)
    } else {
        None
    };
    match ledger {
        Some(ledger) if ledger.schema_version == LEDGER_SCHEMA_VERSION => Ok(ledger.transfers),
        Some(_) => Err("model download ledger uses an unsupported schema version".into()),
        None => Ok(Vec::new()),
    }
}

fn write_ledger(path: &Path, records: &[ModelDownloadRecord]) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.backup");
    let ledger = DownloadLedger {
        schema_version: LEDGER_SCHEMA_VERSION,
        transfers: records.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&ledger).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_LEDGER_BYTES {
        return Err("model download ledger exceeds 8 MiB".into());
    }
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    if path.is_file() {
        fs::copy(path, &backup).map_err(|error| error.to_string())?;
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.is_file() {
            let _ = fs::copy(&backup, path);
        }
        return Err(error.to_string());
    }
    if !backup.is_file() {
        fs::copy(path, &backup).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn emit(app: &AppHandle, record: &ModelDownloadRecord) {
    let _ = app.emit("model-download", record);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(root: &Path, status: &str) -> ModelDownloadRecord {
        ModelDownloadRecord {
            id: "download-1".into(),
            status: status.into(),
            source_url: "https://huggingface.co/Qwen/Test/resolve/main/model.gguf".into(),
            repository: "Qwen--Test".into(),
            revision: "main".into(),
            file_name: "model.gguf".into(),
            destination_path: root.join("model.gguf").to_string_lossy().into_owned(),
            partial_path: root.join("model.gguf.part").to_string_lossy().into_owned(),
            total_bytes: 12,
            downloaded_bytes: 4,
            bytes_per_second: 2,
            eta_seconds: Some(4),
            expected_sha256: None,
            actual_sha256: None,
            source_etag: Some("object".into()),
            checksum_source: "pending".into(),
            retry_count: 0,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            detail: "Downloading".into(),
            error: None,
        }
    }

    #[test]
    fn accepts_only_public_hugging_face_gguf_files() {
        let source = normalize_source(
            "https://huggingface.co/Qwen/Qwen3.8-27B/blob/main/Qwen3.8-27B.gguf?download=true",
        )
        .unwrap();
        assert_eq!(
            source.url.as_str(),
            "https://huggingface.co/Qwen/Qwen3.8-27B/resolve/main/Qwen3.8-27B.gguf"
        );
        let revision = normalize_source(
            "https://huggingface.co/Qwen/Test/blob/refs%2Fpr%2F7/folder/model%20one.gguf",
        )
        .unwrap();
        assert_eq!(revision.revision, "refs/pr/7");
        assert_eq!(
            revision.url.as_str(),
            "https://huggingface.co/Qwen/Test/resolve/refs%2Fpr%2F7/folder/model%20one.gguf"
        );
        assert!(normalize_source("https://example.com/model.gguf").is_err());
        assert!(normalize_source("https://huggingface.co/Qwen/Test/blob/main/model.bin").is_err());
        assert!(normalize_source(
            "https://huggingface.co/Qwen/Test/blob/main/model.gguf?token=secret"
        )
        .is_err());
        assert!(normalize_source(
            "https://huggingface.co/Qwen/Test/blob/main/model-00001-of-00004.gguf"
        )
        .is_err());
        assert!(normalize_repository_source(&format!(
            "https://huggingface.co/{}",
            "a".repeat(4_100)
        ))
        .is_err());
    }

    #[test]
    fn recognizes_every_numbered_model_shard_without_rejecting_ordinary_names() {
        assert!(is_numbered_model_shard("model-00001-of-00004.gguf"));
        assert!(is_numbered_model_shard("MODEL-00004-of-00004.GGUF"));
        assert!(!is_numbered_model_shard("model-of-the-year.gguf"));
        assert!(!is_numbered_model_shard("model-1-of-4.gguf"));
        assert!(!is_numbered_model_shard("mmproj-model.gguf"));
    }

    #[test]
    fn checksum_can_stop_at_a_durable_verification_checkpoint() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.gguf.part");
        fs::write(&path, b"GGUF payload").unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(sha256_file(&path, Some(&cancel)).unwrap(), None);
        assert_eq!(
            sha256_file(&path, None).unwrap().as_deref().map(str::len),
            Some(64)
        );
    }

    #[test]
    fn restart_marks_active_transfer_interrupted_without_deleting_partial() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("model-downloads");
        fs::create_dir_all(&root).unwrap();
        let ledger = root.join("downloads.json");
        let transfer = record(&root, "downloading");
        fs::write(&transfer.partial_path, b"half").unwrap();
        write_ledger(&ledger, &[transfer]).unwrap();
        let manager = ModelDownloadManager::new(temp.path()).unwrap();
        let recovered = manager.list().unwrap().remove(0);
        assert_eq!(recovered.status, "interrupted");
        assert_eq!(recovered.downloaded_bytes, 4);
        assert!(Path::new(&recovered.partial_path).is_file());
    }

    #[test]
    fn ledger_restores_valid_backup_after_primary_corruption() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("downloads.json");
        let transfer = record(temp.path(), "paused");
        write_ledger(&path, std::slice::from_ref(&transfer)).unwrap();
        fs::copy(&path, path.with_extension("json.backup")).unwrap();
        fs::write(&path, b"not json").unwrap();
        assert_eq!(read_ledger(&path).unwrap(), vec![transfer]);
    }

    #[test]
    fn content_range_must_match_checkpoint_and_object() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_RANGE, "bytes 100-199/1000".parse().unwrap());
        assert!(validate_content_range(&headers, 100, 1000).is_ok());
        assert!(validate_content_range(&headers, 99, 1000).is_err());
        assert!(validate_content_range(&headers, 100, 999).is_err());
    }

    #[tokio::test]
    #[ignore = "requires explicit public Hugging Face network access"]
    async fn live_inspects_public_gguf_repository_without_downloading_weights() {
        let temp = tempfile::tempdir().unwrap();
        let manager = ModelDownloadManager::new(temp.path()).unwrap();
        let inspection = manager
            .inspect("https://huggingface.co/unsloth/Qwen3.8-27B-GGUF")
            .await
            .unwrap();
        let smallest = inspection
            .candidates
            .iter()
            .find(|candidate| candidate.file_name == "Qwen3.8-27B-UD-IQ2_XXS.gguf")
            .unwrap();
        assert_eq!(smallest.bytes, 9_010_048_064);
        assert_eq!(smallest.sha256.as_deref().map(str::len), Some(64));
        let source = normalize_source(&smallest.url).unwrap();
        let probe = probe_remote(&internet_client().unwrap(), &source)
            .await
            .unwrap();
        assert_eq!(probe.total_bytes, smallest.bytes);
        assert_eq!(probe.sha256, smallest.sha256);
    }
}
