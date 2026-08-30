//! Single-owner local inference runtime.
//!
//! Research, chat, and future local tools must obtain an `InferenceLease`. The semaphore is the
//! VRAM/KV safety boundary: the detected GPU never receives competing Kestrel generations or
//! duplicate Kestrel-managed model processes. Every catalog model uses the same managed path.

use crate::model::ModelInfo;
use crate::models::{
    ControlSettings, EngineCandidate, ManagedRuntimeSnapshot, ResearchSettings, RuntimeLog,
};
use reqwest::Client;
use serde_json::json;
use sha2::Digest;
use std::{
    collections::{HashSet, VecDeque},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use thiserror::Error;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, BufReader},
    process::{Child, Command},
    sync::{Mutex, OwnedSemaphorePermit, Semaphore},
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(300);
const ENGINE_SOURCE_CONFIGURED: &str = "Configured";
const ENGINE_SOURCE_BUNDLED: &str = "Kestrel bundled engine";
const ENGINE_SOURCE_JAN: &str = "Jan backend";
const ENGINE_SOURCE_PATH: &str = "Windows PATH";

/// Finds only well-known local engine locations. It never searches whole drives and never
/// downloads or executes a candidate during discovery.
pub fn detect_engines(configured: &str, bundled_model_root: &str) -> Vec<EngineCandidate> {
    let mut candidates = Vec::new();
    push_engine(
        &mut candidates,
        PathBuf::from(configured),
        ENGINE_SOURCE_CONFIGURED,
    );
    push_engine(
        &mut candidates,
        Path::new(bundled_model_root)
            .join("runtime")
            .join("llama-server.exe"),
        ENGINE_SOURCE_BUNDLED,
    );
    if let Some(base) = directories::BaseDirs::new() {
        let jan = base
            .data_dir()
            .join("Jan")
            .join("data")
            .join("llamacpp")
            .join("backends");
        if jan.is_dir() {
            for entry in walkdir::WalkDir::new(jan)
                .follow_links(false)
                .max_depth(5)
                .into_iter()
                .filter_map(Result::ok)
            {
                if entry.file_type().is_file()
                    && entry
                        .file_name()
                        .to_string_lossy()
                        .eq_ignore_ascii_case("llama-server.exe")
                {
                    push_engine(
                        &mut candidates,
                        entry.path().to_path_buf(),
                        ENGINE_SOURCE_JAN,
                    );
                }
            }
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            push_engine(
                &mut candidates,
                directory.join("llama-server.exe"),
                ENGINE_SOURCE_PATH,
            );
        }
    }
    let mut seen = HashSet::new();
    candidates.retain(|candidate| seen.insert(candidate.path.to_lowercase()));
    candidates.sort_by_cached_key(engine_rank);
    candidates
}

fn engine_rank(candidate: &EngineCandidate) -> (u8, u8, String) {
    let path = candidate.path.to_lowercase();
    let source = match candidate.source.as_str() {
        ENGINE_SOURCE_CONFIGURED => 0,
        ENGINE_SOURCE_BUNDLED => 1,
        ENGINE_SOURCE_JAN => 2,
        _ => 3,
    };
    let backend = if path.contains("cuda") {
        0
    } else if path.contains("vulkan") {
        1
    } else {
        2
    };
    (source, backend, path)
}

fn push_engine(candidates: &mut Vec<EngineCandidate>, path: PathBuf, source: &str) {
    if is_llama_server_file(&path) {
        candidates.push(EngineCandidate {
            path: path.to_string_lossy().into_owned(),
            source: source.into(),
        });
    }
}

pub fn is_llama_server_file(path: &Path) -> bool {
    path.is_file()
        && path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("llama-server.exe"))
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("model file is missing: {0}")]
    MissingModel(String),
    #[error("llama.cpp engine is missing: {0}")]
    MissingEngine(String),
    #[error("model engine must be an existing file named llama-server.exe: {0}")]
    InvalidEngine(String),
    #[error("no local port is available")]
    NoPort,
    #[error("could not start the local model: {0}")]
    Start(#[from] std::io::Error),
    #[error("local model startup failed: {0}")]
    Startup(String),
    #[error("local model request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("local model returned invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("local runtime maintenance failed: {0}")]
    Maintenance(String),
}

#[derive(Debug, Clone)]
pub struct ModelConnection {
    pub endpoint: String,
    pub api_key: Option<String>,
    pub model_id: String,
    pub model_label: String,
}

pub struct InferenceLease {
    pub connection: ModelConnection,
    _permit: OwnedSemaphorePermit,
}

struct RuntimeProcess {
    child: Option<Child>,
    api_key_file: Option<PathBuf>,
    connection: ModelConnection,
    snapshot: ManagedRuntimeSnapshot,
}

pub struct RuntimeManager {
    process: Mutex<Option<RuntimeProcess>>,
    gate: Arc<Semaphore>,
    http: Client,
    logs: Arc<Mutex<VecDeque<RuntimeLog>>>,
}

impl RuntimeManager {
    pub fn new() -> Self {
        Self {
            process: Mutex::new(None),
            gate: Arc::new(Semaphore::new(1)),
            http: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3_600))
                .build()
                .expect("local runtime HTTP client"),
            logs: Arc::new(Mutex::new(VecDeque::with_capacity(500))),
        }
    }

    pub async fn snapshot(&self) -> ManagedRuntimeSnapshot {
        let mut process = self.process.lock().await;
        if let Some(current) = process.as_mut() {
            let healthy = self.health(&current.connection).await;
            if !healthy {
                if current
                    .child
                    .as_mut()
                    .is_some_and(|child| child.try_wait().ok().flatten().is_some())
                {
                    current.snapshot.phase = "failed".into();
                    current.snapshot.detail = "The managed llama.cpp process exited. Its logs remain visible in the runtime feed.".into();
                } else {
                    current.snapshot.phase = "unavailable".into();
                    current.snapshot.detail =
                        "The runtime endpoint did not answer its local health check.".into();
                }
            } else {
                current.snapshot.phase = "ready".into();
            }
            current.snapshot.inference_busy = self.gate.available_permits() == 0;
            return current.snapshot.clone();
        }
        ManagedRuntimeSnapshot::default()
    }

    /// Full history is retained for native diagnostics and tests. UI snapshots use `recent_logs`.
    #[allow(dead_code)]
    pub async fn logs(&self) -> Vec<RuntimeLog> {
        self.logs.lock().await.iter().cloned().collect()
    }

    pub async fn recent_logs(&self, limit: usize) -> Vec<RuntimeLog> {
        let logs = self.logs.lock().await;
        logs.iter()
            .skip(logs.len().saturating_sub(limit))
            .cloned()
            .collect()
    }

    /// The active managed process remains protected from producer-triggered competing-app cleanup.
    /// This is intentionally a passive ownership lookup and does not perform a health probe.
    pub async fn owned_process_id(&self) -> Option<u32> {
        self.process
            .lock()
            .await
            .as_ref()
            .and_then(|process| process.snapshot.pid)
            .filter(|pid| *pid != 0)
    }

    pub async fn lease_research(
        self: &Arc<Self>,
        model_id: &str,
        models: &[ModelInfo],
        control: &ControlSettings,
        research: &ResearchSettings,
        app: Option<&AppHandle>,
    ) -> Result<InferenceLease, RuntimeError> {
        let mut effective = control.for_model(model_id);
        if research.advanced_mode {
            effective.context_window = research.context_window;
            effective.max_output_tokens = research.max_output_tokens;
        }
        effective.model_overrides.clear();
        self.lease_model(model_id, models, &effective, app).await
    }

    pub async fn start_model(
        &self,
        model: &ModelInfo,
        settings: &ControlSettings,
        app: Option<&AppHandle>,
    ) -> Result<ManagedRuntimeSnapshot, RuntimeError> {
        let effective = settings.for_model(&model.id);
        self.start_managed(model, &effective, app).await?;
        Ok(self.snapshot().await)
    }

    async fn start_managed(
        &self,
        model: &ModelInfo,
        settings: &ControlSettings,
        app: Option<&AppHandle>,
    ) -> Result<ModelConnection, RuntimeError> {
        if !Path::new(&model.path).is_file() {
            return Err(RuntimeError::MissingModel(model.path.clone()));
        }
        if !Path::new(&settings.engine_path).is_file() {
            return Err(RuntimeError::MissingEngine(settings.engine_path.clone()));
        }
        if !is_llama_server_file(Path::new(&settings.engine_path)) {
            return Err(RuntimeError::InvalidEngine(settings.engine_path.clone()));
        }
        self.stop_managed().await?;
        let port = portpicker::pick_unused_port().ok_or(RuntimeError::NoPort)?;
        let api_key = hex::encode(sha2::Sha256::digest(
            format!("{}:{port}:{}", model.id, chrono::Utc::now()).as_bytes(),
        ));
        let api_key_file = create_api_key_file(&api_key)?;
        let context = settings.context_window.max(1);
        let mut args = vec![
            "--model".into(),
            model.path.clone(),
            "--alias".into(),
            model.id.clone(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string(),
            "--api-key-file".into(),
            api_key_file.to_string_lossy().into_owned(),
            "--parallel".into(),
            "1".into(),
            "--ctx-size".into(),
            context.to_string(),
            "--threads".into(),
            settings.threads.max(1).to_string(),
            "--metrics".into(),
            "--props".into(),
            "--slots".into(),
            "--jinja".into(),
            "--cache-ram".into(),
            "0".into(),
            "--n-gpu-layers".into(),
            "all".into(),
            "--split-mode".into(),
            "none".into(),
            "--fit".into(),
            "off".into(),
            "--kv-offload".into(),
            "--op-offload".into(),
        ];
        if let Some(projector) = &model.mmproj_path {
            args.extend([
                "--mmproj".into(),
                projector.clone(),
                "--mmproj-offload".into(),
            ]);
        }
        let visible_args = args.clone();
        let mut command = Command::new(&settings.engine_path);
        command
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(parent) = Path::new(&settings.engine_path).parent() {
            command.current_dir(parent);
        }
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = fs::remove_file(&api_key_file);
                return Err(error.into());
            }
        };
        let pid = child.id();
        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, "stdout", self.logs.clone(), app.cloned());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, "stderr", self.logs.clone(), app.cloned());
        }
        let connection = ModelConnection {
            endpoint: format!("http://127.0.0.1:{port}/v1"),
            api_key: Some(api_key),
            model_id: model.id.clone(),
            model_label: model.name.clone(),
        };
        let snapshot = ManagedRuntimeSnapshot {
            phase: "starting".into(),
            mode: "managed".into(),
            model_id: Some(model.id.clone()),
            model_name: Some(model.name.clone()),
            endpoint: Some(connection.endpoint.clone()),
            pid,
            context_window: context,
            launch_args: visible_args,
            detail: "Loading one strict full-GPU llama.cpp runtime.".into(),
            inference_busy: false,
        };
        {
            let mut process = self.process.lock().await;
            *process = Some(RuntimeProcess {
                child: Some(child),
                api_key_file: Some(api_key_file),
                connection: connection.clone(),
                snapshot,
            });
        }
        emit_runtime(
            app,
            "starting",
            &format!("Loading {} into the GPU", model.name),
        );
        let started = std::time::Instant::now();
        while started.elapsed() < STARTUP_TIMEOUT {
            if self.health(&connection).await {
                let mut process = self.process.lock().await;
                if let Some(current) = process.as_mut() {
                    current.snapshot.phase = "ready".into();
                    current.snapshot.detail =
                        "Model ready. Chat and research share this single authenticated runtime."
                            .into();
                }
                emit_runtime(app, "ready", &format!("{} is ready", model.name));
                return Ok(connection);
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        self.stop_managed().await?;
        Err(RuntimeError::Startup("timed out after five minutes".into()))
    }

    pub async fn stop_managed(&self) -> Result<(), RuntimeError> {
        let mut process = self.process.lock().await;
        if let Some(current) = process.as_mut() {
            if let Some(child) = current.child.as_mut() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            if let Some(path) = current.api_key_file.take() {
                let _ = fs::remove_file(path);
            }
        }
        *process = None;
        Ok(())
    }

    /// Wait until every Kestrel inference lease has been returned. Memory release uses this after
    /// cancelling visible work so the server is never killed while a native tool may still be
    /// committing its durable result.
    pub async fn wait_until_idle(&self, maximum: Duration) -> Option<OwnedSemaphorePermit> {
        tokio::time::timeout(maximum, self.gate.clone().acquire_owned())
            .await
            .ok()
            .and_then(Result::ok)
    }

    /// Stop only abandoned llama.cpp processes carrying Kestrel's private API-key marker. A live
    /// parent means another Kestrel window still owns the process, so it is left untouched.
    #[cfg(windows)]
    pub async fn stop_orphaned_kestrel_processes(&self) -> Result<Vec<u32>, RuntimeError> {
        const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
$all=@(Get-CimInstance Win32_Process)
$live=@{}
foreach($item in $all){$live[[uint32]$item.ProcessId]=$true}
foreach($item in $all){
  if($item.Name -ieq 'llama-server.exe' -and
     $item.CommandLine -match 'kestrel-runtime-key-[0-9a-f-]+\.txt' -and
     -not $live.ContainsKey([uint32]$item.ParentProcessId)){
    $keyMatch=[regex]::Match($item.CommandLine,'--api-key-file\s+(?:"([^"]+)"|(\S+))')
    $keyFile=if($keyMatch.Groups[1].Success){$keyMatch.Groups[1].Value}else{$keyMatch.Groups[2].Value}
    try {
      Stop-Process -Id $item.ProcessId -Force -ErrorAction Stop
      if($keyFile -and [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($keyFile)) -ieq [IO.Path]::GetFullPath($env:TEMP)){
        Remove-Item -LiteralPath $keyFile -Force -ErrorAction SilentlyContinue
      }
      Write-Output $item.ProcessId
    } catch {
      Write-Warning "Could not stop orphaned Kestrel process $($item.ProcessId): $($_.Exception.Message)"
    }
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
        command.creation_flags(0x08000000);
        let output = command
            .output()
            .await
            .map_err(|error| RuntimeError::Maintenance(error.to_string()))?;
        if !output.status.success() {
            return Err(RuntimeError::Maintenance(
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .collect())
    }

    #[cfg(not(windows))]
    pub async fn stop_orphaned_kestrel_processes(&self) -> Result<Vec<u32>, RuntimeError> {
        Ok(Vec::new())
    }

    /// Obtain the single inference slot for an interactive feature. The lease keeps the gate
    /// occupied until streaming or an agent loop has completely stopped.
    pub async fn lease_model(
        self: &Arc<Self>,
        model_id: &str,
        models: &[ModelInfo],
        settings: &ControlSettings,
        app: Option<&AppHandle>,
    ) -> Result<InferenceLease, RuntimeError> {
        let model = models
            .iter()
            .find(|model| model.id == model_id)
            .ok_or_else(|| RuntimeError::MissingModel(model_id.to_string()))?;
        let effective = settings.for_model(&model.id);
        let connection = match self
            .current_for_model(&model.id, effective.context_window)
            .await
        {
            Some(current) => current,
            None => {
                self.start_model(model, settings, app).await?;
                self.current_healthy()
                    .await
                    .ok_or_else(|| RuntimeError::Startup("runtime disappeared".into()))?
            }
        };
        let permit = self
            .gate
            .clone()
            .acquire_owned()
            .await
            .expect("inference gate is never closed");
        Ok(InferenceLease {
            connection,
            _permit: permit,
        })
    }

    async fn current_healthy(&self) -> Option<ModelConnection> {
        let connection = self
            .process
            .lock()
            .await
            .as_ref()
            .map(|value| value.connection.clone())?;
        self.health(&connection).await.then_some(connection)
    }

    async fn current_for_model(
        &self,
        catalog_id: &str,
        context_window: u32,
    ) -> Option<ModelConnection> {
        let (connection, matches) = self.process.lock().await.as_ref().map(|value| {
            (
                value.connection.clone(),
                process_matches(&value.snapshot, catalog_id, context_window),
            )
        })?;
        (matches && self.health(&connection).await).then_some(connection)
    }

    async fn health(&self, connection: &ModelConnection) -> bool {
        let base = connection.endpoint.trim_end_matches("/v1");
        let request = authorized(self.http.get(format!("{base}/health")), connection);
        tokio::time::timeout(Duration::from_secs(3), request.send())
            .await
            .ok()
            .and_then(Result::ok)
            .is_some_and(|response| response.status().is_success())
    }
}

fn process_matches(
    snapshot: &ManagedRuntimeSnapshot,
    catalog_id: &str,
    context_window: u32,
) -> bool {
    snapshot.model_id.as_deref() == Some(catalog_id) && snapshot.context_window == context_window
}

pub fn authorized(
    builder: reqwest::RequestBuilder,
    connection: &ModelConnection,
) -> reqwest::RequestBuilder {
    match connection.api_key.as_ref() {
        Some(key) => builder.bearer_auth(key),
        None => builder,
    }
}

fn emit_runtime(app: Option<&AppHandle>, phase: &str, detail: &str) {
    if let Some(app) = app {
        let _ = app.emit("runtime-progress", json!({"phase":phase,"detail":detail}));
    }
}

fn spawn_log_reader<R>(
    reader: R,
    stream: &'static str,
    logs: Arc<Mutex<VecDeque<RuntimeLog>>>,
    app: Option<AppHandle>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => break,
                Err(error) => {
                    let record = RuntimeLog {
                        stream: "kestrel".into(),
                        line: format!("Failed to read managed runtime {stream}: {error}"),
                        at: chrono::Utc::now().to_rfc3339(),
                    };
                    {
                        let mut values = logs.lock().await;
                        if values.len() == 500 {
                            values.pop_front();
                        }
                        values.push_back(record.clone());
                    }
                    if let Some(app) = &app {
                        let _ = app.emit("runtime-log", &record);
                    }
                    break;
                }
            };
            let record = RuntimeLog {
                stream: stream.into(),
                line: truncate(&line, 4_000),
                at: chrono::Utc::now().to_rfc3339(),
            };
            {
                let mut values = logs.lock().await;
                if values.len() == 500 {
                    values.pop_front();
                }
                values.push_back(record.clone());
            }
            if let Some(app) = &app {
                let _ = app.emit("runtime-log", &record);
            }
        }
    });
}

fn create_api_key_file(api_key: &str) -> Result<PathBuf, std::io::Error> {
    let path =
        std::env::temp_dir().join(format!("kestrel-runtime-key-{}.txt", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    if let Err(error) = file.write_all(format!("{api_key}\n").as_bytes()) {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_is_present_for_managed_local_services() {
        let managed = ModelConnection {
            endpoint: "http://127.0.0.1:10000/v1".into(),
            api_key: Some("secret".into()),
            model_id: "x".into(),
            model_label: "x".into(),
        };
        assert_eq!(managed.api_key.as_deref(), Some("secret"));
    }

    #[test]
    fn api_key_file_does_not_expose_the_secret_in_its_path() {
        let path = create_api_key_file("session-secret").unwrap();
        assert!(!path.to_string_lossy().contains("session-secret"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "session-secret\n");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn configured_engine_is_discovered_first_without_execution() {
        let directory = tempfile::tempdir().unwrap();
        let engine = directory.path().join("llama-server.exe");
        fs::write(&engine, b"not executable during discovery").unwrap();

        let candidates = detect_engines(&engine.to_string_lossy(), "Z:\\missing-bonsai");

        assert_eq!(candidates.first().unwrap().path, engine.to_string_lossy());
        assert_eq!(candidates.first().unwrap().source, "Configured");
    }

    #[test]
    fn engine_discovery_rejects_other_executables() {
        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("program.exe");
        fs::write(&program, b"not a llama server").unwrap();

        let candidates = detect_engines(&program.to_string_lossy(), "Z:\\missing-bonsai");

        assert!(!is_llama_server_file(&program));
        assert!(!candidates
            .iter()
            .any(|candidate| candidate.path == program.to_string_lossy()));
    }

    #[test]
    fn runtime_reuse_requires_the_same_model_and_context_window() {
        let snapshot = ManagedRuntimeSnapshot {
            model_id: Some("model-a".into()),
            context_window: 32_768,
            ..ManagedRuntimeSnapshot::default()
        };

        assert!(process_matches(&snapshot, "model-a", 32_768));
        assert!(!process_matches(&snapshot, "model-a", 98_304));
        assert!(!process_matches(&snapshot, "model-b", 32_768));
    }

    #[tokio::test]
    async fn inference_gate_allows_exactly_one_owner() {
        let manager = RuntimeManager::new();
        let first = manager.gate.clone().acquire_owned().await.unwrap();
        assert!(tokio::time::timeout(
            Duration::from_millis(20),
            manager.gate.clone().acquire_owned()
        )
        .await
        .is_err());
        drop(first);
        assert!(tokio::time::timeout(
            Duration::from_millis(20),
            manager.gate.clone().acquire_owned()
        )
        .await
        .is_ok());
    }

    #[tokio::test]
    async fn recent_logs_returns_newest_records_in_order() {
        let manager = RuntimeManager::new();
        {
            let mut logs = manager.logs.lock().await;
            for index in 0..5 {
                logs.push_back(RuntimeLog {
                    stream: "test".into(),
                    line: index.to_string(),
                    at: index.to_string(),
                });
            }
        }
        assert_eq!(
            manager
                .recent_logs(3)
                .await
                .into_iter()
                .map(|record| record.line)
                .collect::<Vec<_>>(),
            ["2", "3", "4"]
        );
        assert_eq!(manager.logs().await.len(), 5);
    }
}
