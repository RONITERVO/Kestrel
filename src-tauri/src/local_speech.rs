//! Shared offline speech through the user's local ComfyUI installation.
//!
//! Kestrel never falls back to a browser, operating-system, or remote speech service. The first
//! adapters target ComfyUI-Chatterbox for narration and Kestrel's small ComfyUI Whisper boundary
//! for timestamped microphone transcription. Additional adapters belong here rather than in
//! individual product UIs.

use crate::store::default_research_root;
use base64::Engine as _;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use thiserror::Error;
use tokio::{process::Child, sync::Mutex};
use tokio_util::sync::CancellationToken;

const COMFY_BASE: &str = "http://127.0.0.1:8188";
const TTS_ADAPTER_REVISION: &str = "chatterbox-opus-v2";
const STT_ADAPTER_REVISION: &str = "kestrel-whisper-v1";
const CHATTERBOX_NODE: &str = "custom_nodes/ComfyUI-Chatterbox/nodes.py";
const CHATTERBOX_MODEL_ROOT: &str = "models/tts/chatterbox";
const WHISPER_NODE: &str = "custom_nodes/Kestrel-Whisper/nodes.py";
const WHISPER_MODEL_ROOT: &str = "models/stt/whisper";
const CHATTERBOX_FILES: [&str; 5] = [
    "conds.pt",
    "s3gen.safetensors",
    "t3_cfg.safetensors",
    "tokenizer.json",
    "ve.safetensors",
];
const MAX_TEXT_BYTES: usize = 8_192;
const MAX_TRANSCRIPTION_AUDIO_BYTES: usize = 32 * 1024 * 1024;
const MAX_TRANSCRIPTION_PROMPT_BYTES: usize = 4_096;
const MAX_GENERATED_AUDIO_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 2 * 1024 * 1024;
const MAX_TRANSCRIPT_TIMINGS: usize = 100_000;
const MAX_ID_BYTES: usize = 128;
const GENERATION_TIMEOUT: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Error)]
pub enum SpeechError {
    #[error("Local speech file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ComfyUI speech request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Local speech receipt is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Local speech configuration is invalid: {0}")]
    Invalid(String),
    #[error("Local speech is unavailable: {0}")]
    Unavailable(String),
    #[error("Local speech operation was stopped")]
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechModel {
    pub id: String,
    pub name: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSnapshot {
    pub narration_available: bool,
    pub transcription_available: bool,
    pub comfy_ready: bool,
    pub voices: Vec<SpeechModel>,
    pub transcribers: Vec<SpeechModel>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSynthesisRequest {
    pub job_id: String,
    pub source_kind: String,
    pub source_id: String,
    pub passage_id: String,
    pub text: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechAlignmentRequest {
    pub job_id: String,
    pub source_kind: String,
    pub source_id: String,
    pub passage_id: String,
    pub text: String,
    pub relative_path: String,
    pub voice_model_id: String,
    pub alignment_model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTiming {
    pub value: String,
    pub start: f64,
    pub end: f64,
}

fn default_transcription_language() -> String {
    "auto".into()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechClip {
    pub job_id: String,
    pub passage_id: String,
    pub relative_path: String,
    pub model_id: String,
    pub cache_hit: bool,
    pub segments: Vec<SpeechTiming>,
    pub words: Vec<SpeechTiming>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTranscriptionRequest {
    pub job_id: String,
    pub source_kind: String,
    pub source_id: String,
    pub recording_id: String,
    pub audio_base64: String,
    pub mime_type: String,
    pub model_id: String,
    #[serde(default = "default_transcription_language")]
    pub language: String,
    #[serde(default)]
    pub prompt: String,
    pub final_pass: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTranscription {
    pub job_id: String,
    pub recording_id: String,
    pub text: String,
    pub segments: Vec<SpeechTiming>,
    pub words: Vec<SpeechTiming>,
    pub audio_relative_path: Option<String>,
    pub final_pass: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechProgress {
    pub job_id: String,
    pub passage_id: String,
    pub stage: String,
    pub detail: String,
}

#[derive(Clone)]
pub struct LocalSpeech {
    cache_root: PathBuf,
    http: Client,
    generation: Arc<Mutex<()>>,
    startup: Arc<Mutex<()>>,
    comfy_child: Arc<Mutex<Option<Child>>>,
}

impl LocalSpeech {
    pub fn new(library_root: &Path) -> Result<Self, SpeechError> {
        let cache_root = library_root.join("speech-cache");
        fs::create_dir_all(cache_root.join("logs"))?;
        let http = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("fixed loopback HTTP client");
        Ok(Self {
            cache_root,
            http,
            generation: Arc::new(Mutex::new(())),
            startup: Arc::new(Mutex::new(())),
            comfy_child: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn snapshot(&self, comfy_root: &str) -> SpeechSnapshot {
        let root = Path::new(comfy_root);
        let voices = discover_chatterbox_models(root);
        let transcribers = discover_whisper_models(root);
        let narration_node_ready = root.join(CHATTERBOX_NODE).is_file();
        let transcription_node_ready = root.join(WHISPER_NODE).is_file();
        let comfy_ready = self.comfy_ready().await;
        let narration_available = narration_node_ready && !voices.is_empty();
        let transcription_available = transcription_node_ready && !transcribers.is_empty();
        let detail = if !root.join("main.py").is_file() {
            "Choose the user's ComfyUI folder in Setup before using local speech.".into()
        } else if !narration_available && !transcription_available {
            "No supported local ComfyUI speech models are ready. Open Setup and install Local voice and dictation; Kestrel never falls back to system or browser speech.".into()
        } else if comfy_ready {
            "Local ComfyUI speech is ready. Narration is cached and dictation receives a final timestamped Whisper pass.".into()
        } else {
            "The local ComfyUI speech engine is starting in the background. The first uncached model use can take longer while weights load.".into()
        };
        SpeechSnapshot {
            narration_available,
            transcription_available,
            comfy_ready,
            voices,
            transcribers,
            detail,
        }
    }

    pub fn cached_clip(
        &self,
        comfy_root: &str,
        request: &SpeechSynthesisRequest,
    ) -> Result<Option<SpeechClip>, SpeechError> {
        validate_synthesis_request(comfy_root, request)?;
        let target = self.cache_target(request)?;
        if valid_cached_audio(&target) {
            return Ok(Some(clip_receipt(
                &self.cache_root,
                request,
                &target,
                true,
            )?));
        }
        Ok(None)
    }

    pub fn cached_alignment(
        &self,
        comfy_root: &str,
        request: &SpeechAlignmentRequest,
    ) -> Result<Option<SpeechClip>, SpeechError> {
        let target = self.validate_alignment_request(comfy_root, request)?;
        Ok(read_alignment(&target, request)
            .map(|(segments, words)| alignment_clip(request, segments, words, true)))
    }

    pub async fn ensure_comfy(
        &self,
        comfy_root: &str,
        cancel: &CancellationToken,
    ) -> Result<(), SpeechError> {
        if self.comfy_ready().await {
            return Ok(());
        }
        let _startup = self.startup.lock().await;
        if self.comfy_ready().await {
            return Ok(());
        }
        let root = PathBuf::from(comfy_root);
        let main = root.join("main.py");
        if !main.is_file() {
            return Err(SpeechError::Unavailable(format!(
                "ComfyUI main.py is missing from {}. Choose the correct ComfyUI folder in Setup.",
                root.display()
            )));
        }
        let python = comfy_python(&root).ok_or_else(|| {
            SpeechError::Unavailable(format!(
                "ComfyUI's Python runtime was not found beside {}.",
                root.display()
            ))
        })?;
        let stdout = fs::File::create(self.cache_root.join("logs/comfy-speech.stdout.log"))?;
        let stderr = fs::File::create(self.cache_root.join("logs/comfy-speech.stderr.log"))?;
        let mut command = tokio::process::Command::new(python);
        command
            .arg(&main)
            .args([
                "--listen",
                "127.0.0.1",
                "--port",
                "8188",
                "--preview-method",
                "none",
                "--disable-auto-launch",
                "--lowvram",
                "--cache-none",
            ])
            .current_dir(&root)
            .env("PYTHONUTF8", "1")
            .env("HF_HOME", root.join(".cache/huggingface"))
            .env("HF_HUB_OFFLINE", "1")
            .env("TRANSFORMERS_OFFLINE", "1")
            .env("CUDA_VISIBLE_DEVICES", "0")
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .kill_on_drop(false);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let child = command.spawn()?;
        *self.comfy_child.lock().await = Some(child);
        for _ in 0..180 {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {},
                _ = cancel.cancelled() => return Err(SpeechError::Cancelled),
            }
            if self.comfy_ready().await {
                return Ok(());
            }
        }
        Err(SpeechError::Unavailable(format!(
            "ComfyUI did not become ready within six minutes. See {}.",
            self.cache_root
                .join("logs/comfy-speech.stderr.log")
                .display()
        )))
    }

    pub async fn stop_comfy(&self) {
        if let Some(mut child) = self.comfy_child.lock().await.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
    }

    pub async fn synthesize(
        &self,
        comfy_root: &str,
        request: &SpeechSynthesisRequest,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<SpeechClip, SpeechError> {
        validate_synthesis_request(comfy_root, request)?;
        self.verify_live_node("ChatterboxTTS", "ComfyUI-Chatterbox")
            .await?;
        if let Some(cached) = self.cached_clip(comfy_root, request)? {
            emit_progress(
                app,
                request,
                "cached",
                "Playing the locally cached passage.",
            );
            return Ok(cached);
        }
        let _generation = self.generation.lock().await;
        if let Some(cached) = self.cached_clip(comfy_root, request)? {
            return Ok(cached);
        }
        if cancel.is_cancelled() {
            return Err(SpeechError::Cancelled);
        }
        emit_progress(
            app,
            request,
            "generating",
            "ComfyUI is generating this passage with the selected local voice model.",
        );
        let target = self.cache_target(request)?;
        let prefix = format!("kestrel_speech/{}", cache_key(request));
        let graph = chatterbox_graph(request, &prefix);
        let client_id = format!("kestrel-local-tts-{}", uuid::Uuid::new_v4().simple());
        let response = self
            .http
            .post(format!("{COMFY_BASE}/prompt"))
            .json(&json!({"prompt": graph, "client_id": client_id}))
            .send()
            .await?;
        let status = response.status();
        let value: Value = response.json().await?;
        if !status.is_success() {
            return Err(SpeechError::Unavailable(format!(
                "ComfyUI rejected the Chatterbox workflow: {}",
                truncate(&value.to_string(), 900)
            )));
        }
        let prompt_id = value
            .get("prompt_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                SpeechError::Unavailable(format!(
                    "ComfyUI returned no prompt ID: {}",
                    truncate(&value.to_string(), 500)
                ))
            })?;
        let deadline = tokio::time::Instant::now() + GENERATION_TIMEOUT;
        loop {
            if cancel.is_cancelled() {
                let _ = self
                    .http
                    .post(format!("{COMFY_BASE}/interrupt"))
                    .send()
                    .await;
                return Err(SpeechError::Cancelled);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(SpeechError::Unavailable(format!(
                    "ComfyUI did not finish passage {} within fifteen minutes.",
                    request.passage_id
                )));
            }
            let history: Value = self
                .http
                .get(format!("{COMFY_BASE}/history/{prompt_id}"))
                .send()
                .await?
                .json()
                .await?;
            if let Some(entry) = history.get(prompt_id) {
                if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
                    return Err(SpeechError::Unavailable(
                        comfy_execution_error(entry).unwrap_or_else(|| {
                            format!(
                                "ComfyUI execution failed: {}",
                                truncate(&entry.to_string(), 900)
                            )
                        }),
                    ));
                }
                if entry.pointer("/status/completed").and_then(Value::as_bool) == Some(true) {
                    let (filename, subfolder) = find_audio_output(entry).ok_or_else(|| {
                        SpeechError::Unavailable(
                            "ComfyUI completed the TTS graph without exposing saved audio.".into(),
                        )
                    })?;
                    let source = safe_comfy_output(Path::new(comfy_root), &filename, &subfolder)?;
                    if source
                        .metadata()
                        .is_ok_and(|metadata| metadata.len() > MAX_GENERATED_AUDIO_BYTES)
                    {
                        return Err(SpeechError::Unavailable(
                            "ComfyUI produced speech larger than the 64 MiB local safety limit. Shorten the passage or inspect the voice workflow.".into(),
                        ));
                    }
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let temporary = target.with_extension("opus.tmp");
                    tokio::fs::copy(&source, &temporary).await?;
                    if !valid_cached_audio(&temporary) {
                        let _ = tokio::fs::remove_file(&temporary).await;
                        return Err(SpeechError::Unavailable(
                            "ComfyUI produced an unreadable Opus passage.".into(),
                        ));
                    }
                    if let Err(error) = tokio::fs::rename(&temporary, &target).await {
                        let _ = tokio::fs::remove_file(&temporary).await;
                        if !valid_cached_audio(&target) {
                            return Err(error.into());
                        }
                    }
                    emit_progress(
                        app,
                        request,
                        "complete",
                        "The next passage is ready locally.",
                    );
                    return clip_receipt(&self.cache_root, request, &target, false);
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(250)) => {},
                _ = cancel.cancelled() => {
                    let _ = self.http.post(format!("{COMFY_BASE}/interrupt")).send().await;
                    return Err(SpeechError::Cancelled);
                },
            }
        }
    }

    pub async fn transcribe(
        &self,
        comfy_root: &str,
        request: &SpeechTranscriptionRequest,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<SpeechTranscription, SpeechError> {
        validate_transcription_request(comfy_root, request)?;
        let audio = base64::engine::general_purpose::STANDARD
            .decode(&request.audio_base64)
            .map_err(|_| SpeechError::Invalid("recorded audio is not valid base64".into()))?;
        if audio.len() > MAX_TRANSCRIPTION_AUDIO_BYTES {
            return Err(SpeechError::Invalid(format!(
                "recorded audio exceeds the {} MiB local dictation limit",
                MAX_TRANSCRIPTION_AUDIO_BYTES / 1024 / 1024
            )));
        }
        let extension = recording_extension(&request.mime_type).ok_or_else(|| {
            SpeechError::Invalid(format!(
                "unsupported microphone recording type {}",
                request.mime_type
            ))
        })?;
        let _generation = self.generation.lock().await;
        if cancel.is_cancelled() {
            return Err(SpeechError::Cancelled);
        }
        self.verify_live_node("KestrelWhisper", "Kestrel's Whisper adapter")
            .await?;
        self.verify_live_node("PreviewAny", "the ComfyUI Preview as Text node")
            .await?;

        let mut fingerprint = Sha256::new();
        fingerprint.update(STT_ADAPTER_REVISION.as_bytes());
        fingerprint.update(&audio);
        let digest = hex::encode(fingerprint.finalize());
        let persisted = if request.final_pass {
            let target = self
                .cache_root
                .join("recordings")
                .join(&request.source_kind)
                .join(&request.source_id)
                .join(format!("{}-{}.{}", request.recording_id, digest, extension));
            Some(write_recording_atomic(&target, &audio)?)
        } else {
            None
        };

        emit_transcription_progress(
            app,
            request,
            "transcribing",
            if request.final_pass {
                "ComfyUI Whisper is making the final timestamped transcript."
            } else {
                "ComfyUI Whisper is updating the live provisional transcript."
            },
        );
        let root = Path::new(comfy_root);
        let input_directory = root.join("input/kestrel_speech");
        fs::create_dir_all(&input_directory)?;
        let input_name = format!("{}-{}.{}", request.job_id, &digest[..16], extension);
        let input_path = input_directory.join(&input_name);
        write_recording_atomic(&input_path, &audio)?;
        let input_relative = format!("kestrel_speech/{input_name}");
        let graph = whisper_graph(request, &input_relative);
        let result = self
            .execute_whisper_graph(&request.job_id, graph, cancel)
            .await;
        let _ = tokio::fs::remove_file(&input_path).await;
        let (text, segments, words) = result?;
        let audio_relative_path = persisted
            .as_ref()
            .map(|path| relative_cache_path(&self.cache_root, path))
            .transpose()?;
        emit_transcription_progress(
            app,
            request,
            "complete",
            if request.final_pass {
                "Final local transcript and word timings are saved."
            } else {
                "Live provisional dictation updated."
            },
        );
        let transcription = SpeechTranscription {
            job_id: request.job_id.clone(),
            recording_id: request.recording_id.clone(),
            text,
            segments,
            words,
            audio_relative_path,
            final_pass: request.final_pass,
        };
        if let Some(audio_path) = persisted.as_ref() {
            let receipt = json!({
                "revision": STT_ADAPTER_REVISION,
                "sourceKind": request.source_kind,
                "sourceId": request.source_id,
                "recordingId": request.recording_id,
                "modelId": request.model_id,
                "language": request.language,
                "mimeType": request.mime_type,
                "text": transcription.text,
                "segments": transcription.segments,
                "words": transcription.words,
                "audioRelativePath": transcription.audio_relative_path,
                "createdAt": chrono::Utc::now().to_rfc3339(),
            });
            write_json_atomic(&sidecar_path(audio_path), &receipt)?;
        }
        Ok(transcription)
    }

    pub async fn align(
        &self,
        comfy_root: &str,
        request: &SpeechAlignmentRequest,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<SpeechClip, SpeechError> {
        let target = self.validate_alignment_request(comfy_root, request)?;
        if let Some((segments, words)) = read_alignment(&target, request) {
            return Ok(alignment_clip(request, segments, words, true));
        }
        let _generation = self.generation.lock().await;
        if let Some((segments, words)) = read_alignment(&target, request) {
            return Ok(alignment_clip(request, segments, words, true));
        }
        if cancel.is_cancelled() {
            return Err(SpeechError::Cancelled);
        }
        self.verify_live_node("KestrelWhisper", "Kestrel's Whisper adapter")
            .await?;
        self.verify_live_node("PreviewAny", "the ComfyUI Preview as Text node")
            .await?;
        emit_alignment_progress(
            app,
            request,
            "aligning",
            "Whisper is aligning words to the unchanged local voice recording.",
        );
        let input_directory = Path::new(comfy_root).join("input/kestrel_speech");
        fs::create_dir_all(&input_directory)?;
        let input_name = format!("{}-alignment.opus", request.job_id);
        let input_path = input_directory.join(&input_name);
        tokio::fs::copy(&target, &input_path).await?;
        let transcription = SpeechTranscriptionRequest {
            job_id: request.job_id.clone(),
            source_kind: request.source_kind.clone(),
            source_id: request.source_id.clone(),
            recording_id: request.passage_id.clone(),
            audio_base64: String::new(),
            mime_type: "audio/ogg".into(),
            model_id: request.alignment_model_id.clone(),
            language: "auto".into(),
            prompt: bounded_prefix(&request.text, MAX_TRANSCRIPTION_PROMPT_BYTES),
            final_pass: false,
        };
        let graph = whisper_graph(&transcription, &format!("kestrel_speech/{input_name}"));
        let result = self
            .execute_whisper_graph(&request.job_id, graph, cancel)
            .await;
        let _ = tokio::fs::remove_file(&input_path).await;
        let (_text, segments, words) = result?;
        write_synthesis_receipt(
            &target,
            request,
            &segments,
            &words,
            Some(&request.alignment_model_id),
            true,
        )?;
        emit_alignment_progress(
            app,
            request,
            "complete",
            "Exact local speech word timings are ready for click-to-seek.",
        );
        Ok(alignment_clip(request, segments, words, false))
    }

    async fn execute_whisper_graph(
        &self,
        job_id: &str,
        graph: Value,
        cancel: &CancellationToken,
    ) -> Result<(String, Vec<SpeechTiming>, Vec<SpeechTiming>), SpeechError> {
        let client_id = format!("kestrel-whisper-{}", uuid::Uuid::new_v4().simple());
        let response = self
            .http
            .post(format!("{COMFY_BASE}/prompt"))
            .json(&json!({"prompt": graph, "client_id": client_id}))
            .send()
            .await?;
        let status = response.status();
        let value: Value = response.json().await?;
        if !status.is_success() {
            return Err(SpeechError::Unavailable(format!(
                "ComfyUI rejected the Whisper workflow: {}",
                truncate(&value.to_string(), 900)
            )));
        }
        let prompt_id = value
            .get("prompt_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                SpeechError::Unavailable(format!(
                    "ComfyUI returned no Whisper prompt ID for {job_id}: {}",
                    truncate(&value.to_string(), 500)
                ))
            })?;
        let deadline = tokio::time::Instant::now() + GENERATION_TIMEOUT;
        loop {
            if cancel.is_cancelled() {
                let _ = self
                    .http
                    .post(format!("{COMFY_BASE}/interrupt"))
                    .send()
                    .await;
                return Err(SpeechError::Cancelled);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(SpeechError::Unavailable(format!(
                    "ComfyUI Whisper did not finish {job_id} within fifteen minutes."
                )));
            }
            let history: Value = self
                .http
                .get(format!("{COMFY_BASE}/history/{prompt_id}"))
                .send()
                .await?
                .json()
                .await?;
            if let Some(entry) = history.get(prompt_id) {
                if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
                    return Err(SpeechError::Unavailable(
                        comfy_execution_error(entry).unwrap_or_else(|| {
                            format!(
                                "ComfyUI Whisper failed: {}",
                                truncate(&entry.to_string(), 900)
                            )
                        }),
                    ));
                }
                if entry.pointer("/status/completed").and_then(Value::as_bool) == Some(true) {
                    return parse_whisper_output(entry);
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(200)) => {},
                _ = cancel.cancelled() => {
                    let _ = self.http.post(format!("{COMFY_BASE}/interrupt")).send().await;
                    return Err(SpeechError::Cancelled);
                },
            }
        }
    }

    pub async fn release_model_memory(&self) {
        let _ = self
            .http
            .post(format!("{COMFY_BASE}/kestrel/speech/free"))
            .timeout(Duration::from_secs(30))
            .send()
            .await;
        let _ = self
            .http
            .post(format!("{COMFY_BASE}/free"))
            .json(&json!({"unload_models": true, "free_memory": true}))
            .timeout(Duration::from_secs(30))
            .send()
            .await;
    }

    async fn comfy_ready(&self) -> bool {
        self.http
            .get(format!("{COMFY_BASE}/system_stats"))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    async fn verify_live_node(&self, node: &str, adapter: &str) -> Result<(), SpeechError> {
        let value: Value = self
            .http
            .get(format!("{COMFY_BASE}/object_info/{node}"))
            .send()
            .await?
            .json()
            .await?;
        if value.get(node).is_none() {
            return Err(SpeechError::Unavailable(format!(
                "The running ComfyUI does not expose {node}. Restart it after installing {adapter}."
            )));
        }
        Ok(())
    }

    fn cache_target(&self, request: &SpeechSynthesisRequest) -> Result<PathBuf, SpeechError> {
        safe_source(&request.source_kind, &request.source_id)?;
        Ok(self
            .cache_root
            .join("generated")
            .join(&request.source_kind)
            .join(&request.source_id)
            .join(format!("{}.opus", cache_key(request))))
    }

    fn validate_alignment_request(
        &self,
        comfy_root: &str,
        request: &SpeechAlignmentRequest,
    ) -> Result<PathBuf, SpeechError> {
        let root = Path::new(comfy_root);
        if !root.is_absolute() || !root.join("main.py").is_file() {
            return Err(SpeechError::Invalid(
                "ComfyUI root must be an absolute local installation path".into(),
            ));
        }
        safe_identifier(&request.job_id, "job ID")?;
        safe_source(&request.source_kind, &request.source_id)?;
        safe_identifier(&request.passage_id, "passage ID")?;
        if request.text.trim().is_empty() || request.text.len() > MAX_TEXT_BYTES {
            return Err(SpeechError::Invalid(format!(
                "alignment text must contain 1 to {MAX_TEXT_BYTES} UTF-8 bytes"
            )));
        }
        if !safe_relative_path(&request.relative_path)
            || !request.relative_path.starts_with(&format!(
                "generated/{}/{}/",
                request.source_kind, request.source_id
            ))
            || !request.relative_path.ends_with(".opus")
        {
            return Err(SpeechError::Invalid(
                "speech alignment path does not belong to its durable source".into(),
            ));
        }
        if !discover_chatterbox_models(root)
            .iter()
            .any(|model| model.id == request.voice_model_id)
        {
            return Err(SpeechError::Invalid(format!(
                "{} is not a complete local ComfyUI voice pack",
                request.voice_model_id
            )));
        }
        if !discover_whisper_models(root)
            .iter()
            .any(|model| model.id == request.alignment_model_id)
        {
            return Err(SpeechError::Invalid(format!(
                "{} is not a complete local ComfyUI Whisper model",
                request.alignment_model_id
            )));
        }
        let canonical_root = fs::canonicalize(&self.cache_root)?;
        let target = fs::canonicalize(self.cache_root.join(&request.relative_path))?;
        if !target.starts_with(canonical_root) || !valid_cached_audio(&target) {
            return Err(SpeechError::Invalid(
                "speech alignment audio is outside the private cache or unreadable".into(),
            ));
        }
        Ok(target)
    }
}

pub fn media_response(request: tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    match read_media_response(&request) {
        Ok(response) => response,
        Err((status, message)) => tauri::http::Response::builder()
            .status(status)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Access-Control-Allow-Origin", "*")
            .body(message.into_bytes())
            .expect("fixed local speech error response"),
    }
}

fn read_media_response(
    request: &tauri::http::Request<Vec<u8>>,
) -> Result<tauri::http::Response<Vec<u8>>, (u16, String)> {
    let relative =
        percent_encoding::percent_decode_str(request.uri().path().trim_start_matches('/'))
            .decode_utf8()
            .map_err(|_| (400, "invalid local speech audio path".to_string()))?;
    if !safe_relative_path(relative.as_ref()) {
        return Err((403, "unsafe local speech audio path".into()));
    }
    let root = default_research_root().join("speech-cache");
    let canonical_root = fs::canonicalize(&root)
        .map_err(|_| (404, "local speech audio cache does not exist".to_string()))?;
    let target = fs::canonicalize(root.join(relative.as_ref()))
        .map_err(|_| (404, "Local speech audio was not found".to_string()))?;
    let allowed_extension = target
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| {
            matches!(
                value,
                "opus" | "ogg" | "webm" | "m4a" | "mp4" | "wav" | "flac"
            )
        });
    if !target.starts_with(&canonical_root) || !target.is_file() || !allowed_extension {
        return Err((403, "Speech audio is outside the private cache".into()));
    }
    ranged_audio_response(request, &target)
}

fn ranged_audio_response(
    request: &tauri::http::Request<Vec<u8>>,
    target: &Path,
) -> Result<tauri::http::Response<Vec<u8>>, (u16, String)> {
    let mut file = fs::File::open(target).map_err(|error| (500, error.to_string()))?;
    let length = file
        .metadata()
        .map_err(|error| (500, error.to_string()))?
        .len();
    let mime = match target.extension().and_then(|value| value.to_str()) {
        Some("opus" | "ogg") => "audio/ogg",
        Some("webm") => "audio/webm",
        Some("m4a" | "mp4") => "audio/mp4",
        Some("wav") => "audio/wav",
        _ => "audio/flac",
    };
    let mut builder = tauri::http::Response::builder()
        .header("Content-Type", mime)
        .header("Access-Control-Allow-Origin", "*")
        .header("Accept-Ranges", "bytes");
    if request.method() == tauri::http::Method::HEAD {
        return builder
            .header("Content-Length", length)
            .body(Vec::new())
            .map_err(|error| (500, error.to_string()));
    }
    if let Some(range) = request
        .headers()
        .get("range")
        .and_then(|value| value.to_str().ok())
    {
        let value = range
            .strip_prefix("bytes=")
            .and_then(|value| value.split(',').next())
            .ok_or_else(|| (416, "unsupported audio byte range".to_string()))?;
        let (start_text, end_text) = value
            .split_once('-')
            .ok_or_else(|| (416, "invalid audio byte range".to_string()))?;
        let start = start_text
            .parse::<u64>()
            .map_err(|_| (416, "suffix audio ranges are unsupported".to_string()))?;
        if start >= length {
            return Err((416, "audio byte range starts beyond the file".into()));
        }
        const MAX_CHUNK: u64 = 4 * 1024 * 1024;
        let requested_end = end_text.parse::<u64>().unwrap_or(length.saturating_sub(1));
        let end = requested_end.min(length - 1).min(start + MAX_CHUNK - 1);
        let count = end - start + 1;
        file.seek(SeekFrom::Start(start))
            .map_err(|error| (500, error.to_string()))?;
        let mut body = Vec::with_capacity(count as usize);
        file.take(count)
            .read_to_end(&mut body)
            .map_err(|error| (500, error.to_string()))?;
        builder = builder
            .status(tauri::http::StatusCode::PARTIAL_CONTENT)
            .header("Content-Range", format!("bytes {start}-{end}/{length}"))
            .header("Content-Length", count);
        return builder.body(body).map_err(|error| (500, error.to_string()));
    }
    let mut body = Vec::with_capacity(length.min(usize::MAX as u64) as usize);
    file.read_to_end(&mut body)
        .map_err(|error| (500, error.to_string()))?;
    builder
        .header("Content-Length", length)
        .body(body)
        .map_err(|error| (500, error.to_string()))
}

fn discover_chatterbox_models(comfy_root: &Path) -> Vec<SpeechModel> {
    let root = comfy_root.join(CHATTERBOX_MODEL_ROOT);
    let mut models = fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let pack = entry.file_name().to_string_lossy().into_owned();
            if !safe_pack_name(&pack)
                || !CHATTERBOX_FILES.iter().all(|file| {
                    entry
                        .path()
                        .join(file)
                        .metadata()
                        .is_ok_and(|metadata| metadata.len() > 0)
                })
            {
                return None;
            }
            Some(SpeechModel {
                id: format!("chatterbox:{pack}"),
                name: humanize_pack_name(&pack),
                provider: "ComfyUI Chatterbox".into(),
            })
        })
        .collect::<Vec<_>>();
    models.sort_by(|left, right| left.name.cmp(&right.name));
    models
}

fn discover_whisper_models(comfy_root: &Path) -> Vec<SpeechModel> {
    let root = comfy_root.join(WHISPER_MODEL_ROOT);
    let mut models = fs::read_dir(root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("pt")
                || !path.metadata().is_ok_and(|metadata| metadata.len() > 1_024)
            {
                return None;
            }
            let model = path.file_stem()?.to_str()?.to_string();
            if !safe_pack_name(&model) {
                return None;
            }
            Some(SpeechModel {
                id: format!("whisper:{model}"),
                name: format!("Whisper {}", humanize_pack_name(&model)),
                provider: "ComfyUI Whisper".into(),
            })
        })
        .collect::<Vec<_>>();
    models.sort_by(|left, right| left.name.cmp(&right.name));
    models
}

fn validate_synthesis_request(
    comfy_root: &str,
    request: &SpeechSynthesisRequest,
) -> Result<(), SpeechError> {
    let root = Path::new(comfy_root);
    if !root.is_absolute() || !root.join("main.py").is_file() {
        return Err(SpeechError::Invalid(
            "ComfyUI root must be an absolute local installation path".into(),
        ));
    }
    safe_identifier(&request.job_id, "job ID")?;
    safe_source(&request.source_kind, &request.source_id)?;
    safe_identifier(&request.passage_id, "passage ID")?;
    let text = request.text.trim();
    if text.is_empty() || text.len() > MAX_TEXT_BYTES {
        return Err(SpeechError::Invalid(format!(
            "passage text must contain 1-{MAX_TEXT_BYTES} UTF-8 bytes"
        )));
    }
    if !discover_chatterbox_models(root)
        .iter()
        .any(|model| model.id == request.model_id)
    {
        return Err(SpeechError::Invalid(format!(
            "{} is not a complete local ComfyUI voice pack",
            request.model_id
        )));
    }
    Ok(())
}

fn validate_transcription_request(
    comfy_root: &str,
    request: &SpeechTranscriptionRequest,
) -> Result<(), SpeechError> {
    let root = Path::new(comfy_root);
    if !root.is_absolute() || !root.join("main.py").is_file() {
        return Err(SpeechError::Invalid(
            "ComfyUI root must be an absolute local installation path".into(),
        ));
    }
    safe_identifier(&request.job_id, "job ID")?;
    safe_source(&request.source_kind, &request.source_id)?;
    safe_identifier(&request.recording_id, "recording ID")?;
    if request.audio_base64.is_empty()
        || request.audio_base64.len() > (MAX_TRANSCRIPTION_AUDIO_BYTES * 4 / 3 + 8)
    {
        return Err(SpeechError::Invalid(
            "recorded audio is empty or exceeds the local dictation limit".into(),
        ));
    }
    if request.prompt.len() > MAX_TRANSCRIPTION_PROMPT_BYTES {
        return Err(SpeechError::Invalid(format!(
            "dictation guidance exceeds {MAX_TRANSCRIPTION_PROMPT_BYTES} UTF-8 bytes"
        )));
    }
    if request.language.is_empty()
        || request.language.len() > 64
        || !request
            .language
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || matches!(byte, b' ' | b'-'))
    {
        return Err(SpeechError::Invalid("unsafe dictation language".into()));
    }
    if !discover_whisper_models(root)
        .iter()
        .any(|model| model.id == request.model_id)
    {
        return Err(SpeechError::Invalid(format!(
            "{} is not a complete local ComfyUI Whisper model",
            request.model_id
        )));
    }
    recording_extension(&request.mime_type).ok_or_else(|| {
        SpeechError::Invalid(format!(
            "unsupported microphone recording type {}",
            request.mime_type
        ))
    })?;
    Ok(())
}

fn safe_source(kind: &str, id: &str) -> Result<(), SpeechError> {
    if !matches!(kind, "research" | "chat" | "task" | "copilot") {
        return Err(SpeechError::Invalid("unsafe speech source kind".into()));
    }
    safe_identifier(id, "speech source ID")
}

fn safe_identifier(value: &str, label: &str) -> Result<(), SpeechError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(SpeechError::Invalid(format!("unsafe {label}")));
    }
    Ok(())
}

fn safe_pack_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn safe_relative_path(value: &str) -> bool {
    if value.is_empty() || value.contains(['\\', ':', '\0', '\r', '\n']) {
        return false;
    }
    Path::new(value)
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
}

fn comfy_python(root: &Path) -> Option<PathBuf> {
    [
        root.join(".venv/Scripts/python.exe"),
        root.parent()?.join("python_embeded/python.exe"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

fn recording_extension(mime_type: &str) -> Option<&'static str> {
    let normalized = mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match normalized.as_str() {
        "audio/webm" => Some("webm"),
        "audio/ogg" | "audio/opus" => Some("opus"),
        "audio/mp4" | "audio/m4a" => Some("m4a"),
        "audio/wav" | "audio/wave" | "audio/x-wav" => Some("wav"),
        _ => None,
    }
}

fn write_recording_atomic(target: &Path, bytes: &[u8]) -> Result<PathBuf, SpeechError> {
    if target.is_file() {
        return Ok(target.to_path_buf());
    }
    let parent = target
        .parent()
        .ok_or_else(|| SpeechError::Invalid("recording path has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        target
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("speech"),
        uuid::Uuid::new_v4()
    ));
    let mut file = fs::File::create(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    match fs::rename(&temporary, target) {
        Ok(()) => Ok(target.to_path_buf()),
        Err(_) if target.is_file() => {
            let _ = fs::remove_file(&temporary);
            Ok(target.to_path_buf())
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error.into())
        }
    }
}

fn sidecar_path(target: &Path) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|value| value.to_os_string())
        .unwrap_or_else(|| "speech".into());
    name.push(".json");
    target.with_file_name(name)
}

fn write_json_atomic(target: &Path, value: &Value) -> Result<(), SpeechError> {
    if target.is_file() {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    write_recording_atomic(target, &bytes)?;
    Ok(())
}

/// Replace a derived speech receipt without ever modifying its source audio. On platforms where
/// rename cannot replace an existing file, a recovery copy is kept until the new receipt is live.
fn replace_json_recoverable(target: &Path, value: &Value) -> Result<(), SpeechError> {
    let parent = target
        .parent()
        .ok_or_else(|| SpeechError::Invalid("speech receipt path has no parent".into()))?;
    fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        target
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("speech-receipt"),
        uuid::Uuid::new_v4()
    ));
    let backup = receipt_recovery_path(target);
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;

    if target.is_file() {
        fs::copy(target, &backup)?;
        fs::remove_file(target)?;
    }
    match fs::rename(&temporary, target) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            if backup.is_file() {
                let _ = fs::rename(&backup, target);
            }
            Err(error.into())
        }
    }
}

fn receipt_recovery_path(receipt_path: &Path) -> PathBuf {
    receipt_path.with_extension("json.recovery")
}

fn read_receipt_recoverable(receipt_path: &Path) -> Option<Value> {
    let read = |path: &Path| {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    };
    if let Some(value) = read(receipt_path) {
        return Some(value);
    }
    let backup = receipt_recovery_path(receipt_path);
    let value = read(&backup)?;
    if fs::copy(&backup, receipt_path).is_err() {
        return None;
    }
    Some(value)
}

fn validate_timings(timings: &[SpeechTiming]) -> bool {
    timings.len() <= MAX_TRANSCRIPT_TIMINGS
        && timings.iter().all(|timing| {
            timing.value.len() <= 4_096
                && timing.start.is_finite()
                && timing.end.is_finite()
                && timing.start >= 0.0
                && timing.end >= timing.start
                && timing.end <= 24.0 * 60.0 * 60.0
        })
}

fn read_alignment(
    audio_path: &Path,
    request: &SpeechAlignmentRequest,
) -> Option<(Vec<SpeechTiming>, Vec<SpeechTiming>)> {
    let receipt = read_receipt_recoverable(&sidecar_path(audio_path))?;
    if receipt.get("alignmentModelId").and_then(Value::as_str)
        != Some(request.alignment_model_id.as_str())
        || receipt.get("modelId").and_then(Value::as_str) != Some(request.voice_model_id.as_str())
        || receipt.get("text").and_then(Value::as_str) != Some(request.text.as_str())
    {
        return None;
    }
    receipt_timings(&receipt)
}

fn receipt_timings(receipt: &Value) -> Option<(Vec<SpeechTiming>, Vec<SpeechTiming>)> {
    let segments: Vec<SpeechTiming> =
        serde_json::from_value(receipt.get("segments")?.clone()).ok()?;
    let words: Vec<SpeechTiming> = serde_json::from_value(receipt.get("words")?.clone()).ok()?;
    if words.is_empty() || !validate_timings(&segments) || !validate_timings(&words) {
        return None;
    }
    Some((segments, words))
}

fn write_synthesis_receipt(
    audio_path: &Path,
    request: &SpeechAlignmentRequest,
    segments: &[SpeechTiming],
    words: &[SpeechTiming],
    alignment_model_id: Option<&str>,
    replace: bool,
) -> Result<(), SpeechError> {
    if !validate_timings(segments) || words.is_empty() || !validate_timings(words) {
        return Err(SpeechError::Unavailable(
            "ComfyUI Whisper returned no safe word alignment for this speech passage.".into(),
        ));
    }
    let receipt_path = sidecar_path(audio_path);
    let created_at = read_receipt_recoverable(&receipt_path)
        .and_then(|value| {
            value
                .get("createdAt")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let receipt = json!({
        "revision": TTS_ADAPTER_REVISION,
        "sourceKind": request.source_kind,
        "sourceId": request.source_id,
        "passageId": request.passage_id,
        "modelId": request.voice_model_id,
        "alignmentModelId": alignment_model_id,
        "text": request.text,
        "audioRelativePath": request.relative_path,
        "segments": segments,
        "words": words,
        "createdAt": created_at,
        "alignedAt": chrono::Utc::now().to_rfc3339(),
    });
    if replace {
        replace_json_recoverable(&receipt_path, &receipt)
    } else {
        write_json_atomic(&receipt_path, &receipt)
    }
}

fn relative_cache_path(cache_root: &Path, target: &Path) -> Result<String, SpeechError> {
    let relative = target
        .strip_prefix(cache_root)
        .map_err(|_| SpeechError::Invalid("speech artifact escaped its cache".into()))?;
    let value = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if !safe_relative_path(&value) {
        return Err(SpeechError::Invalid("unsafe speech artifact path".into()));
    }
    Ok(value)
}

pub fn clean_speech_text(raw: &str) -> String {
    if raw.trim().is_empty() {
        return String::new();
    }
    let mut text = raw.to_string();

    while let Some(start) = text.find("```") {
        if let Some(end) = text[start + 3..].find("```") {
            text.replace_range(start..start + 3 + end + 3, " Code block on screen. ");
        } else {
            text.replace_range(start..start + 3, " ");
        }
    }

    let mut cleaned = String::with_capacity(text.len());
    let mut prev_char = ' ';

    for ch in text.chars() {
        match ch {
            '#' | '¤' | '*' | '+' | '^' | '~' | '\\' | '|' | '<' | '>' | '§' | '°' | '±' | '²'
            | '³' | 'µ' | '¶' | '©' | '®' | '™' | '•' | '·' | '‣' | '⁃' | '✓' | '✔' | '✕'
            | '✖' | '✗' | '★' | '☆' | '▲' | '▼' | '◄' | '►' | '◆' | '◇' | '●' | '○' | '■'
            | '□' | '`' => {
                if prev_char != ' ' {
                    cleaned.push(' ');
                    prev_char = ' ';
                }
            }
            '_' => {
                if prev_char != ' ' {
                    cleaned.push(' ');
                    prev_char = ' ';
                }
            }
            c if c.is_whitespace() => {
                if prev_char != ' ' {
                    cleaned.push(' ');
                    prev_char = ' ';
                }
            }
            c => {
                let u = c as u32;
                if (0x2500..=0x257F).contains(&u)
                    || (0x2580..=0x259F).contains(&u)
                    || (0x25A0..=0x25FF).contains(&u)
                    || (0x1F300..=0x1FAFF).contains(&u)
                    || (0x1F600..=0x1F64F).contains(&u)
                    || (0x1F680..=0x1F6FF).contains(&u)
                {
                    if prev_char != ' ' {
                        cleaned.push(' ');
                        prev_char = ' ';
                    }
                } else {
                    cleaned.push(c);
                    prev_char = c;
                }
            }
        }
    }

    cleaned.trim().to_string()
}

fn chatterbox_graph(request: &SpeechSynthesisRequest, prefix: &str) -> Value {
    let pack = request
        .model_id
        .strip_prefix("chatterbox:")
        .unwrap_or_default();
    let clean_text = clean_speech_text(&request.text);
    let speech_text = if clean_text.is_empty() {
        request.text.trim().to_string()
    } else {
        clean_text
    };
    let word_count = speech_text.split_whitespace().count() as u32;
    let max_new_tokens = (word_count.saturating_mul(18).saturating_add(96)).clamp(128, 1_600);
    let seed = deterministic_seed(request);
    json!({
        "1": {
            "class_type": "ChatterboxTTS",
            "inputs": {
                "model_pack_name": pack,
                "text": speech_text,
                "max_new_tokens": max_new_tokens,
                "flow_cfg_scale": 0.7,
                "exaggeration": 0.5,
                "temperature": 0.8,
                "cfg_weight": 0.5,
                "repetition_penalty": 1.2,
                "min_p": 0.05,
                "top_p": 1.0,
                "seed": seed,
                "use_watermark": false
            }
        },
        "2": {
            "class_type": "SaveAudioOpus",
            "inputs": {
                "audio": ["1", 0],
                "filename_prefix": prefix,
                "quality": "64k"
            }
        }
    })
}

fn whisper_graph(request: &SpeechTranscriptionRequest, input_relative: &str) -> Value {
    let model = request
        .model_id
        .strip_prefix("whisper:")
        .unwrap_or_default();
    json!({
        "1": {
            "class_type": "LoadAudio",
            "inputs": {"audio": input_relative}
        },
        "2": {
            "class_type": "KestrelWhisper",
            "inputs": {
                "audio": ["1", 0],
                "model": model,
                "language": request.language,
                "prompt": request.prompt.trim()
            }
        },
        "3": {"class_type": "PreviewAny", "inputs": {"source": ["2", 0]}},
        "4": {"class_type": "PreviewAny", "inputs": {"source": ["2", 1]}},
        "5": {"class_type": "PreviewAny", "inputs": {"source": ["2", 2]}}
    })
}

fn parse_whisper_output(
    entry: &Value,
) -> Result<(String, Vec<SpeechTiming>, Vec<SpeechTiming>), SpeechError> {
    let output_text = |node: &str| {
        entry
            .pointer(&format!("/outputs/{node}/text/0"))
            .and_then(Value::as_str)
    };
    let text = output_text("3")
        .ok_or_else(|| {
            SpeechError::Unavailable(
                "ComfyUI Whisper completed without returning transcript text.".into(),
            )
        })?
        .trim()
        .to_string();
    if text.len() > MAX_TRANSCRIPT_BYTES {
        return Err(SpeechError::Unavailable(
            "ComfyUI Whisper returned a transcript larger than the 2 MiB local safety limit."
                .into(),
        ));
    }
    let parse_timing = |node: &str, label: &str| -> Result<Vec<SpeechTiming>, SpeechError> {
        let value = output_text(node).ok_or_else(|| {
            SpeechError::Unavailable(format!(
                "ComfyUI Whisper completed without returning {label} timestamps."
            ))
        })?;
        let timings: Vec<SpeechTiming> = serde_json::from_str(value).map_err(|error| {
            SpeechError::Unavailable(format!(
                "ComfyUI Whisper returned invalid {label} timestamps: {error}"
            ))
        })?;
        if !validate_timings(&timings) {
            return Err(SpeechError::Unavailable(format!(
                "ComfyUI Whisper returned unsafe {label} timestamps."
            )));
        }
        Ok(timings)
    };
    Ok((
        text,
        parse_timing("4", "segment")?,
        parse_timing("5", "word")?,
    ))
}

fn deterministic_seed(request: &SpeechSynthesisRequest) -> u64 {
    let digest = Sha256::digest(format!(
        "{TTS_ADAPTER_REVISION}\0{}\0{}",
        request.model_id,
        request.text.trim()
    ));
    u64::from_le_bytes(digest[..8].try_into().expect("eight-byte digest prefix")).max(1)
}

fn cache_key(request: &SpeechSynthesisRequest) -> String {
    let digest = Sha256::digest(format!(
        "{TTS_ADAPTER_REVISION}\0{}\0{}",
        request.model_id,
        request.text.trim()
    ));
    hex::encode(digest)
}

fn clip_receipt(
    cache_root: &Path,
    request: &SpeechSynthesisRequest,
    target: &Path,
    cache_hit: bool,
) -> Result<SpeechClip, SpeechError> {
    let relative = target
        .strip_prefix(cache_root)
        .map_err(|_| SpeechError::Invalid("invalid speech cache path".into()))?;
    let relative_path = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if !safe_relative_path(&relative_path) {
        return Err(SpeechError::Invalid("invalid speech cache path".into()));
    }
    let receipt_path = sidecar_path(target);
    let (segments, words) = read_receipt_recoverable(&receipt_path)
        .as_ref()
        .and_then(receipt_timings)
        .unwrap_or_default();
    let clip = SpeechClip {
        job_id: request.job_id.clone(),
        passage_id: request.passage_id.clone(),
        relative_path,
        model_id: request.model_id.clone(),
        cache_hit,
        segments,
        words,
    };
    write_json_atomic(
        &receipt_path,
        &json!({
            "revision": TTS_ADAPTER_REVISION,
            "sourceKind": request.source_kind,
            "sourceId": request.source_id,
            "passageId": request.passage_id,
            "modelId": request.model_id,
            "text": request.text,
            "audioRelativePath": clip.relative_path,
            "createdAt": chrono::Utc::now().to_rfc3339(),
        }),
    )?;
    Ok(clip)
}

fn alignment_clip(
    request: &SpeechAlignmentRequest,
    segments: Vec<SpeechTiming>,
    words: Vec<SpeechTiming>,
    cache_hit: bool,
) -> SpeechClip {
    SpeechClip {
        job_id: request.job_id.clone(),
        passage_id: request.passage_id.clone(),
        relative_path: request.relative_path.clone(),
        model_id: request.voice_model_id.clone(),
        cache_hit,
        segments,
        words,
    }
}

fn valid_cached_audio(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    let mut magic = [0u8; 4];
    metadata.len() > 64
        && file.read_exact(&mut magic).is_ok()
        && (&magic == b"OggS" || &magic == b"fLaC" || &magic == b"RIFF")
}

fn safe_comfy_output(
    comfy_root: &Path,
    filename: &str,
    subfolder: &str,
) -> Result<PathBuf, SpeechError> {
    if !safe_relative_path(filename) || (!subfolder.is_empty() && !safe_relative_path(subfolder)) {
        return Err(SpeechError::Unavailable(
            "ComfyUI returned an unsafe audio output path.".into(),
        ));
    }
    let output = comfy_root.join("output");
    let canonical_output = fs::canonicalize(&output)?;
    let source = fs::canonicalize(output.join(subfolder).join(filename))?;
    if !source.starts_with(canonical_output) || !source.is_file() {
        return Err(SpeechError::Unavailable(
            "ComfyUI audio output escaped its local output folder.".into(),
        ));
    }
    Ok(source)
}

fn find_audio_output(entry: &Value) -> Option<(String, String)> {
    let outputs = entry.get("outputs")?.as_object()?;
    for output in outputs.values() {
        for media in output
            .get("audio")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(filename) = media.get("filename").and_then(Value::as_str) {
                return Some((
                    filename.into(),
                    media
                        .get("subfolder")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                ));
            }
        }
    }
    None
}

fn comfy_execution_error(entry: &Value) -> Option<String> {
    let messages = entry.pointer("/status/messages")?.as_array()?;
    for message in messages.iter().rev() {
        let parts = message.as_array()?;
        if parts.first().and_then(Value::as_str) != Some("execution_error") {
            continue;
        }
        let payload = parts.get(1)?;
        let node_type = payload
            .get("node_type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let node_id = payload
            .get("node_id")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let exception = payload
            .get("exception_message")
            .and_then(Value::as_str)
            .unwrap_or("failed without an exception message");
        return Some(format!(
            "{node_type} node {node_id} failed: {}",
            truncate(exception, 800)
        ));
    }
    None
}

fn emit_progress(
    app: Option<&AppHandle>,
    request: &SpeechSynthesisRequest,
    stage: &str,
    detail: &str,
) {
    if let Some(app) = app {
        let _ = app.emit(
            "local-speech-progress",
            SpeechProgress {
                job_id: request.job_id.clone(),
                passage_id: request.passage_id.clone(),
                stage: stage.into(),
                detail: detail.into(),
            },
        );
    }
}

fn emit_transcription_progress(
    app: Option<&AppHandle>,
    request: &SpeechTranscriptionRequest,
    stage: &str,
    detail: &str,
) {
    if let Some(app) = app {
        let _ = app.emit(
            "local-speech-progress",
            SpeechProgress {
                job_id: request.job_id.clone(),
                passage_id: request.recording_id.clone(),
                stage: stage.into(),
                detail: detail.into(),
            },
        );
    }
}

fn emit_alignment_progress(
    app: Option<&AppHandle>,
    request: &SpeechAlignmentRequest,
    stage: &str,
    detail: &str,
) {
    if let Some(app) = app {
        let _ = app.emit(
            "local-speech-progress",
            SpeechProgress {
                job_id: request.job_id.clone(),
                passage_id: request.passage_id.clone(),
                stage: stage.into(),
                detail: detail.into(),
            },
        );
    }
}

fn humanize_pack_name(value: &str) -> String {
    value
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| format!("{}{}", first.to_uppercase(), chars.as_str()))
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.into();
    }
    let mut boundary = maximum;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}…", &value[..boundary])
}

fn bounded_prefix(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.into();
    }
    let mut boundary = maximum;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(text: &str) -> SpeechSynthesisRequest {
        SpeechSynthesisRequest {
            job_id: "job-1".into(),
            source_kind: "research".into(),
            source_id: "report-1".into(),
            passage_id: "overview".into(),
            text: text.into(),
            model_id: "chatterbox:resembleai_default_voice".into(),
        }
    }

    fn complete_pack(root: &Path, name: &str) {
        let pack = root.join(CHATTERBOX_MODEL_ROOT).join(name);
        fs::create_dir_all(&pack).unwrap();
        for file in CHATTERBOX_FILES {
            fs::write(pack.join(file), b"model").unwrap();
        }
    }

    fn complete_whisper(root: &Path, name: &str) {
        let directory = root.join(WHISPER_MODEL_ROOT);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(format!("{name}.pt")), vec![7; 1_025]).unwrap();
    }

    fn transcription_request() -> SpeechTranscriptionRequest {
        SpeechTranscriptionRequest {
            job_id: "stt-job-1".into(),
            source_kind: "chat".into(),
            source_id: "chat-1".into(),
            recording_id: "recording-1".into(),
            audio_base64: base64::engine::general_purpose::STANDARD.encode(b"recorded audio"),
            mime_type: "audio/webm;codecs=opus".into(),
            model_id: "whisper:large-v3-turbo".into(),
            language: "auto".into(),
            prompt: "Existing draft".into(),
            final_pass: true,
        }
    }

    fn alignment_request(clip: &SpeechClip) -> SpeechAlignmentRequest {
        SpeechAlignmentRequest {
            job_id: "alignment-job-1".into(),
            source_kind: "research".into(),
            source_id: "report-1".into(),
            passage_id: "overview".into(),
            text: "A concise locally generated research passage.".into(),
            relative_path: clip.relative_path.clone(),
            voice_model_id: "chatterbox:resembleai_default_voice".into(),
            alignment_model_id: "whisper:large-v3-turbo".into(),
        }
    }

    #[test]
    fn discovers_only_complete_safe_local_voice_packs() {
        let directory = tempfile::tempdir().unwrap();
        complete_pack(directory.path(), "narrator_voice");
        let incomplete = directory
            .path()
            .join(CHATTERBOX_MODEL_ROOT)
            .join("incomplete");
        fs::create_dir_all(&incomplete).unwrap();
        fs::write(incomplete.join(CHATTERBOX_FILES[0]), b"model").unwrap();
        complete_pack(directory.path(), "unsafe pack");

        let models = discover_chatterbox_models(directory.path());
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "chatterbox:narrator_voice");
        assert_eq!(models[0].name, "Narrator Voice");
    }

    #[test]
    fn graph_is_a_bounded_deterministic_chatterbox_workflow() {
        let request = request("A concise locally generated research passage.");
        let graph = chatterbox_graph(&request, "kestrel_research/test");
        assert_eq!(graph["1"]["class_type"], "ChatterboxTTS");
        assert_eq!(
            graph["1"]["inputs"]["model_pack_name"],
            "resembleai_default_voice"
        );
        assert_eq!(graph["1"]["inputs"]["text"], request.text);
        assert_eq!(graph["2"]["class_type"], "SaveAudioOpus");
        assert_eq!(graph["2"]["inputs"]["audio"], json!(["1", 0]));
        assert_eq!(graph["2"]["inputs"]["quality"], "64k");
        assert_eq!(cache_key(&request), cache_key(&request));
        assert_ne!(
            cache_key(&request),
            cache_key(&self::request("Different text."))
        );
    }

    #[test]
    fn discovers_local_whisper_models_and_builds_timestamped_workflow() {
        let directory = tempfile::tempdir().unwrap();
        complete_whisper(directory.path(), "large-v3-turbo");
        fs::write(
            directory.path().join(WHISPER_MODEL_ROOT).join("partial.pt"),
            b"partial",
        )
        .unwrap();
        let models = discover_whisper_models(directory.path());
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "whisper:large-v3-turbo");

        let request = transcription_request();
        let graph = whisper_graph(&request, "kestrel_speech/recording.webm");
        assert_eq!(graph["1"]["class_type"], "LoadAudio");
        assert_eq!(graph["2"]["class_type"], "KestrelWhisper");
        assert_eq!(graph["2"]["inputs"]["model"], "large-v3-turbo");
        assert_eq!(graph["2"]["inputs"]["language"], "auto");
        assert_eq!(graph["3"]["class_type"], "PreviewAny");
        assert_eq!(graph["4"]["inputs"]["source"], json!(["2", 1]));
        assert_eq!(graph["5"]["inputs"]["source"], json!(["2", 2]));
    }

    #[test]
    fn parses_and_validates_whisper_word_and_sentence_timings() {
        let entry = json!({"outputs": {
            "3": {"text": ["Good morning."]},
            "4": {"text": ["[{\"value\":\"Good morning.\",\"start\":0.0,\"end\":1.2}]"]},
            "5": {"text": ["[{\"value\":\"Good\",\"start\":0.0,\"end\":0.5},{\"value\":\"morning.\",\"start\":0.5,\"end\":1.2}]"]}
        }});
        let (text, segments, words) = parse_whisper_output(&entry).unwrap();
        assert_eq!(text, "Good morning.");
        assert_eq!(segments.len(), 1);
        assert_eq!(words.len(), 2);
        assert_eq!(words[1].value, "morning.");

        let unsafe_entry = json!({"outputs": {
            "3": {"text": ["Bad"]},
            "4": {"text": ["[{\"value\":\"Bad\",\"start\":2.0,\"end\":1.0}]"]},
            "5": {"text": ["[]"]}
        }});
        assert!(parse_whisper_output(&unsafe_entry).is_err());
    }

    #[test]
    fn alignment_updates_only_the_receipt_and_is_reused_by_cached_playback() {
        let library = tempfile::tempdir().unwrap();
        let comfy = tempfile::tempdir().unwrap();
        fs::write(comfy.path().join("main.py"), b"# local comfy").unwrap();
        complete_pack(comfy.path(), "resembleai_default_voice");
        complete_whisper(comfy.path(), "large-v3-turbo");
        let speech = LocalSpeech::new(library.path()).unwrap();
        let synthesis = request("A concise locally generated research passage.");
        let target = speech.cache_target(&synthesis).unwrap();
        let mut audio = vec![0_u8; 96];
        audio[..4].copy_from_slice(b"OggS");
        write_recording_atomic(&target, &audio).unwrap();
        let clip = clip_receipt(&speech.cache_root, &synthesis, &target, false).unwrap();
        let alignment = alignment_request(&clip);
        let segments = vec![SpeechTiming {
            value: synthesis.text.clone(),
            start: 0.0,
            end: 2.0,
        }];
        let words = vec![SpeechTiming {
            value: "A".into(),
            start: 0.0,
            end: 0.2,
        }];

        write_synthesis_receipt(
            &target,
            &alignment,
            &segments,
            &words,
            Some(&alignment.alignment_model_id),
            true,
        )
        .unwrap();

        assert_eq!(fs::read(&target).unwrap(), audio);
        let cached = speech
            .cached_alignment(&comfy.path().to_string_lossy(), &alignment)
            .unwrap()
            .unwrap();
        assert!(cached.cache_hit);
        assert_eq!(cached.words[0].start, 0.0);
        assert_eq!(
            speech
                .cached_clip(&comfy.path().to_string_lossy(), &synthesis)
                .unwrap()
                .unwrap()
                .words[0]
                .value,
            "A"
        );
        let receipt = sidecar_path(&target);
        let recovery = receipt_recovery_path(&receipt);
        assert!(!recovery.exists());
        fs::copy(&receipt, &recovery).unwrap();
        fs::remove_file(&receipt).unwrap();
        assert!(speech
            .cached_alignment(&comfy.path().to_string_lossy(), &alignment)
            .unwrap()
            .is_some());
        assert!(receipt.is_file());
    }

    #[test]
    fn audio_history_and_paths_reject_non_audio_or_traversal() {
        let entry = json!({"outputs":{"2":{"audio":[{
            "filename":"passage_00001.flac","subfolder":"kestrel_research","type":"output"
        }]}}});
        assert_eq!(
            find_audio_output(&entry),
            Some(("passage_00001.flac".into(), "kestrel_research".into()))
        );
        assert!(safe_relative_path("report/hash.flac"));
        assert!(!safe_relative_path("../report/hash.flac"));
        assert!(!safe_relative_path(r"report\hash.flac"));
        assert!(safe_identifier("chat:session", "speech source ID").is_err());
    }

    #[test]
    fn speech_text_sanitizer_removes_markdown_and_symbol_stutter() {
        let raw = r#"# 📊 Key Summary:
| Metric | Value |
|---|---|
| Speed | 99% |
```json
{"hidden": true}
```
* Important note (#¤-_''*+) on foo_bar_baz!"#;
        let cleaned = clean_speech_text(raw);
        assert!(!cleaned.contains("```"));
        assert!(!cleaned.contains("hidden"));
        assert!(!cleaned.contains('#'));
        assert!(!cleaned.contains('¤'));
        assert!(!cleaned.contains('*'));
        assert!(!cleaned.contains('|'));
        assert!(cleaned.contains("Code block on screen."));
        assert!(cleaned.contains("Speed"));
        assert!(cleaned.contains("Important note"));
        assert!(cleaned.contains("foo bar baz"));
    }

    #[tokio::test]
    #[ignore = "requires KESTREL_LIVE_COMFY_ROOT and an installed local ComfyUI-Chatterbox voice pack"]
    async fn live_chatterbox_generates_a_cached_research_passage() {
        let Some(comfy_root) = std::env::var_os("KESTREL_LIVE_COMFY_ROOT") else {
            panic!("KESTREL_LIVE_COMFY_ROOT is required");
        };
        let library = tempfile::tempdir().unwrap();
        let speech = LocalSpeech::new(library.path()).unwrap();
        let snapshot = speech.snapshot(&comfy_root.to_string_lossy()).await;
        assert!(snapshot.narration_available, "{}", snapshot.detail);
        let request = SpeechSynthesisRequest {
            model_id: snapshot.voices[0].id.clone(),
            ..request("Kestrel reads this short research passage entirely through local ComfyUI.")
        };
        let cancel = CancellationToken::new();
        speech
            .ensure_comfy(&comfy_root.to_string_lossy(), &cancel)
            .await
            .unwrap();
        let clip = speech
            .synthesize(&comfy_root.to_string_lossy(), &request, &cancel, None)
            .await
            .unwrap();
        assert!(!clip.cache_hit);
        assert!(valid_cached_audio(
            &library
                .path()
                .join("speech-cache")
                .join(&clip.relative_path)
        ));
        assert!(sidecar_path(
            &library
                .path()
                .join("speech-cache")
                .join(&clip.relative_path)
        )
        .is_file());
        assert!(
            speech
                .cached_clip(&comfy_root.to_string_lossy(), &request)
                .unwrap()
                .unwrap()
                .cache_hit
        );
    }

    #[tokio::test]
    #[ignore = "requires KESTREL_LIVE_COMFY_ROOT plus installed ComfyUI-Chatterbox and Kestrel Whisper models"]
    async fn live_local_voice_round_trip_returns_durable_word_timings() {
        let Some(comfy_root) = std::env::var_os("KESTREL_LIVE_COMFY_ROOT") else {
            panic!("KESTREL_LIVE_COMFY_ROOT is required");
        };
        let library = tempfile::tempdir().unwrap();
        let speech = LocalSpeech::new(library.path()).unwrap();
        let snapshot = speech.snapshot(&comfy_root.to_string_lossy()).await;
        assert!(snapshot.narration_available, "{}", snapshot.detail);
        assert!(snapshot.transcription_available, "{}", snapshot.detail);
        let cancel = CancellationToken::new();
        speech
            .ensure_comfy(&comfy_root.to_string_lossy(), &cancel)
            .await
            .unwrap();
        let synthesis = SpeechSynthesisRequest {
            model_id: snapshot.voices[0].id.clone(),
            ..request("Kestrel saves this private local voice recording with exact word timing.")
        };
        let clip = speech
            .synthesize(&comfy_root.to_string_lossy(), &synthesis, &cancel, None)
            .await
            .unwrap();
        let alignment = SpeechAlignmentRequest {
            job_id: "round-trip-alignment".into(),
            source_kind: synthesis.source_kind.clone(),
            source_id: synthesis.source_id.clone(),
            passage_id: synthesis.passage_id.clone(),
            text: synthesis.text.clone(),
            relative_path: clip.relative_path.clone(),
            voice_model_id: synthesis.model_id.clone(),
            alignment_model_id: snapshot.transcribers[0].id.clone(),
        };
        let aligned = speech
            .align(&comfy_root.to_string_lossy(), &alignment, &cancel, None)
            .await
            .unwrap();
        assert!(!aligned.segments.is_empty());
        assert!(!aligned.words.is_empty());
        let recording = library
            .path()
            .join("speech-cache")
            .join(&aligned.relative_path);
        assert!(valid_cached_audio(&recording));
        assert!(sidecar_path(&recording).is_file());
        assert!(
            speech
                .cached_alignment(&comfy_root.to_string_lossy(), &alignment)
                .unwrap()
                .unwrap()
                .cache_hit
        );
        speech.release_model_memory().await;
    }
}
