//! Shared offline speech through the user's local ComfyUI installation.
//!
//! Kestrel never falls back to a browser, operating-system, or remote speech service. The first
//! adapters target ComfyUI-Chatterbox for narration and Kestrel's small ComfyUI Whisper boundary
//! for timestamped microphone transcription. Additional adapters belong here rather than in
//! individual product UIs.

use crate::{
    models::MAX_TRANSCRIPT_TIMINGS,
    store::default_research_root,
    voice_library::{VoiceConditioning, VoiceProfile},
};
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
const TTS_ADAPTER_REVISION: &str = "chatterbox-voice-profile-v1";
const STT_ADAPTER_REVISION: &str = "kestrel-whisper-v2";
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
const MAX_TRANSCRIPTION_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_FILE_TRANSCRIPTION_PROMPT_BYTES: usize = 64 * 1024;
const MAX_TRANSCRIPTION_PROMPT_BYTES: usize = 4_096;
const MAX_MUSIC_OPENING_PROMPT_BYTES: usize = 512;
const MAX_MUSIC_OPENING_PROMPT_LINES: usize = 4;
const MUSIC_REPEAT_CONTEXT_STRATEGY: &str = "whisper-repeat-context-v1";
const MUSIC_REPEAT_CONTEXT_SEAM_SECONDS: f64 = 1.0;
const MAX_MUSIC_CONTEXT_SOURCE_SECONDS: f64 = 330.0;
const MAX_MUSIC_SCORING_TOKENS: usize = 4_096;
const MAX_GENERATED_AUDIO_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 2 * 1024 * 1024;
const MAX_TRANSCRIPT_TIMING_BYTES: usize = 16 * 1024 * 1024;
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
    pub voice_profiles: Vec<VoiceProfile>,
    pub default_voice_profile_id: String,
    pub detail: String,
}

fn default_voice_profile_id() -> String {
    "voice-default".into()
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
    #[serde(default = "default_voice_profile_id")]
    pub voice_profile_id: String,
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
    #[serde(default = "default_voice_profile_id")]
    pub voice_profile_id: String,
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
    pub voice_profile_id: String,
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

#[derive(Debug, Clone)]
pub(crate) struct SpeechFileTranscriptionRequest {
    pub job_id: String,
    pub recording_id: String,
    pub audio_path: PathBuf,
    pub model_id: String,
    pub language: String,
    pub prompt: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SpeechFileRangeTranscriptionRequest {
    pub job_id: String,
    pub recording_id: String,
    pub audio_path: PathBuf,
    pub model_id: String,
    pub language: String,
    pub prompt: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct SpeechFileTranscription {
    pub text: String,
    pub segments: Vec<SpeechTiming>,
    pub words: Vec<SpeechTiming>,
    pub strategy: String,
    pub context_copies: u8,
    pub context_seam_seconds: f64,
    pub selected_context_copy: u8,
    pub first_context_score: f64,
    pub second_context_score: f64,
}

#[derive(Debug)]
struct WhisperTranscriptionResult {
    text: String,
    segments: Vec<SpeechTiming>,
    words: Vec<SpeechTiming>,
    selected_context_copy: u8,
    first_context_score: f64,
    second_context_score: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhisperContextMode {
    Single,
    RepeatedMusic,
}

impl WhisperContextMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::RepeatedMusic => "music-repeat",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WhisperContextMetadata {
    mode: String,
    source_duration: f64,
    second_start: f64,
    second_end: f64,
    seam_seconds: f64,
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
            voice_profiles: Vec::new(),
            default_voice_profile_id: default_voice_profile_id(),
            detail,
        }
    }

    pub fn cached_clip(
        &self,
        comfy_root: &str,
        request: &SpeechSynthesisRequest,
        voice: &VoiceConditioning,
    ) -> Result<Option<SpeechClip>, SpeechError> {
        validate_synthesis_request(comfy_root, request, voice)?;
        let target = self.cache_target(request, voice)?;
        if valid_cached_audio(&target) {
            return Ok(Some(clip_receipt(
                &self.cache_root,
                request,
                voice,
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
        voice: &VoiceConditioning,
    ) -> Result<Option<SpeechClip>, SpeechError> {
        let request = spoken_alignment_request(request);
        let target = self.validate_alignment_request(comfy_root, &request, voice)?;
        Ok(read_alignment(&target, &request, voice)
            .map(|(segments, words)| alignment_clip(&request, voice, segments, words, true)))
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
        voice: &VoiceConditioning,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<SpeechClip, SpeechError> {
        validate_synthesis_request(comfy_root, request, voice)?;
        self.verify_live_node("ChatterboxTTS", "ComfyUI-Chatterbox")
            .await?;
        if let Some(cached) = self.cached_clip(comfy_root, request, voice)? {
            emit_progress(
                app,
                request,
                "cached",
                "Playing the locally cached passage.",
            );
            return Ok(cached);
        }
        let _generation = self.generation.lock().await;
        if let Some(cached) = self.cached_clip(comfy_root, request, voice)? {
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
        let target = self.cache_target(request, voice)?;
        let prefix = format!("kestrel_speech/{}", cache_key(request, voice));
        let voice_input = prepare_voice_input(Path::new(comfy_root), voice)?;
        let graph = chatterbox_graph(request, &prefix, voice, voice_input.as_deref());
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
                    return clip_receipt(&self.cache_root, request, voice, &target, false);
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
        self.verify_live_whisper_node().await?;
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
        let context_mode = WhisperContextMode::Single;
        let graph = whisper_graph(request, &input_relative, context_mode);
        let result = self
            .execute_whisper_graph(&request.job_id, graph, context_mode, None, cancel)
            .await;
        let _ = tokio::fs::remove_file(&input_path).await;
        let WhisperTranscriptionResult {
            text,
            segments,
            words,
            ..
        } = result?;
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

    /// Transcribe a backend-validated durable audio artifact without routing it through the
    /// browser or duplicating it in the dictation cache. Callers remain responsible for proving
    /// that `audio_path` belongs to their durable data boundary before invoking this method.
    pub(crate) async fn transcribe_file(
        &self,
        comfy_root: &str,
        request: &SpeechFileTranscriptionRequest,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<SpeechFileTranscription, SpeechError> {
        validate_file_transcription_request(comfy_root, request)?;
        let _generation = self.generation.lock().await;
        if cancel.is_cancelled() {
            return Err(SpeechError::Cancelled);
        }
        self.verify_live_whisper_node().await?;
        self.verify_live_node("PreviewAny", "the ComfyUI Preview as Text node")
            .await?;

        emit_file_transcription_progress(
            app,
            request,
            "transcribing",
            "ComfyUI Whisper is syncing the preserved music take locally.",
        );
        let root = Path::new(comfy_root);
        let input_directory = root.join("input/kestrel_speech");
        fs::create_dir_all(&input_directory)?;
        let extension = request
            .audio_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("audio")
            .to_ascii_lowercase();
        let input_name = format!(
            "{}-music-{}.{}",
            request.job_id,
            uuid::Uuid::new_v4().simple(),
            extension
        );
        let input_path = input_directory.join(&input_name);
        tokio::fs::copy(&request.audio_path, &input_path).await?;
        let graph_request = SpeechTranscriptionRequest {
            job_id: request.job_id.clone(),
            source_kind: "copilot".into(),
            source_id: request.recording_id.clone(),
            recording_id: request.recording_id.clone(),
            audio_base64: String::new(),
            mime_type: String::new(),
            model_id: request.model_id.clone(),
            language: request.language.clone(),
            prompt: music_opening_prompt(&request.prompt),
            final_pass: true,
        };
        let context_mode = WhisperContextMode::RepeatedMusic;
        let graph = whisper_graph(
            &graph_request,
            &format!("kestrel_speech/{input_name}"),
            context_mode,
        );
        let result = self
            .execute_whisper_graph(
                &request.job_id,
                graph,
                context_mode,
                Some(&request.prompt),
                cancel,
            )
            .await;
        let _ = tokio::fs::remove_file(&input_path).await;
        let WhisperTranscriptionResult {
            text,
            segments,
            words,
            selected_context_copy,
            first_context_score,
            second_context_score,
        } = result?;
        emit_file_transcription_progress(
            app,
            request,
            "complete",
            "Local lyric segments and word timings are ready.",
        );
        Ok(SpeechFileTranscription {
            text,
            segments,
            words,
            strategy: MUSIC_REPEAT_CONTEXT_STRATEGY.into(),
            context_copies: 2,
            context_seam_seconds: MUSIC_REPEAT_CONTEXT_SEAM_SECONDS,
            selected_context_copy,
            first_context_score,
            second_context_score,
        })
    }

    pub(crate) async fn transcribe_file_range(
        &self,
        comfy_root: &str,
        request: &SpeechFileRangeTranscriptionRequest,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<SpeechFileTranscription, SpeechError> {
        let file_req = SpeechFileTranscriptionRequest {
            job_id: request.job_id.clone(),
            recording_id: request.recording_id.clone(),
            audio_path: request.audio_path.clone(),
            model_id: request.model_id.clone(),
            language: request.language.clone(),
            prompt: request.prompt.clone(),
        };
        validate_file_transcription_request(comfy_root, &file_req)?;
        let _generation = self.generation.lock().await;
        if cancel.is_cancelled() {
            return Err(SpeechError::Cancelled);
        }
        self.verify_live_whisper_node().await?;
        self.verify_live_node("PreviewAny", "the ComfyUI Preview as Text node")
            .await?;

        emit_file_transcription_progress(
            app,
            &file_req,
            "transcribing",
            "ComfyUI Whisper is repairing lyric range with start prompt.",
        );

        let root = Path::new(comfy_root);
        let input_directory = root.join("input/kestrel_speech");
        fs::create_dir_all(&input_directory)?;
        let input_name = format!(
            "{}-repair-{}.wav",
            request.job_id,
            uuid::Uuid::new_v4().simple(),
        );
        let input_path = input_directory.join(&input_name);

        let buffer_seconds = 1.5_f64;
        let slice_start = (request.start_seconds - buffer_seconds).max(0.0);
        let slice_end = request.end_seconds + buffer_seconds;

        if slice_wav_pcm(&request.audio_path, &input_path, slice_start, slice_end).is_err() {
            let ffmpeg_cmd = std::env::var_os("KESTREL_FFMPEG_PATH")
                .map(PathBuf::from)
                .filter(|p| p.is_file())
                .unwrap_or_else(|| PathBuf::from("ffmpeg"));
            let status = tokio::process::Command::new(ffmpeg_cmd)
                .args([
                    "-y",
                    "-ss",
                    &format!("{:.3}", slice_start),
                    "-to",
                    &format!("{:.3}", slice_end),
                    "-i",
                ])
                .arg(&request.audio_path)
                .arg(&input_path)
                .output()
                .await;
            if status.is_err() || !status.unwrap().status.success() {
                tokio::fs::copy(&request.audio_path, &input_path).await?;
            }
        }

        let prompt = bounded_prefix(&request.prompt, MAX_MUSIC_OPENING_PROMPT_BYTES);
        let graph_request = SpeechTranscriptionRequest {
            job_id: request.job_id.clone(),
            source_kind: "copilot".into(),
            source_id: request.recording_id.clone(),
            recording_id: request.recording_id.clone(),
            audio_base64: String::new(),
            mime_type: String::new(),
            model_id: request.model_id.clone(),
            language: request.language.clone(),
            prompt: prompt.clone(),
            final_pass: true,
        };
        let context_mode = WhisperContextMode::Single;
        let graph = whisper_graph(
            &graph_request,
            &format!("kestrel_speech/{input_name}"),
            context_mode,
        );
        let result = self
            .execute_whisper_graph(
                &request.job_id,
                graph,
                context_mode,
                Some(&prompt),
                cancel,
            )
            .await;
        let _ = tokio::fs::remove_file(&input_path).await;
        let WhisperTranscriptionResult {
            text,
            mut segments,
            mut words,
            selected_context_copy,
            first_context_score,
            second_context_score,
        } = result?;

        for seg in &mut segments {
            seg.start += slice_start;
            seg.end += slice_start;
        }
        for word in &mut words {
            word.start += slice_start;
            word.end += slice_start;
        }

        words.retain(|w| {
            let mid = (w.start + w.end) / 2.0;
            mid >= request.start_seconds - 0.25 && mid <= request.end_seconds + 0.25
        });

        emit_file_transcription_progress(
            app,
            &file_req,
            "complete",
            "Lyric range repair is ready.",
        );

        Ok(SpeechFileTranscription {
            text,
            segments,
            words,
            strategy: "range-repair-prompt-guided".into(),
            context_copies: 1,
            context_seam_seconds: 0.0,
            selected_context_copy,
            first_context_score,
            second_context_score,
        })
    }

    pub async fn align(
        &self,
        comfy_root: &str,
        request: &SpeechAlignmentRequest,
        voice: &VoiceConditioning,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<SpeechClip, SpeechError> {
        let request = spoken_alignment_request(request);
        let target = self.validate_alignment_request(comfy_root, &request, voice)?;
        if let Some((segments, words)) = read_alignment(&target, &request, voice) {
            return Ok(alignment_clip(&request, voice, segments, words, true));
        }
        let _generation = self.generation.lock().await;
        if let Some((segments, words)) = read_alignment(&target, &request, voice) {
            return Ok(alignment_clip(&request, voice, segments, words, true));
        }
        if cancel.is_cancelled() {
            return Err(SpeechError::Cancelled);
        }
        self.verify_live_whisper_node().await?;
        self.verify_live_node("PreviewAny", "the ComfyUI Preview as Text node")
            .await?;
        emit_alignment_progress(
            app,
            &request,
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
        let context_mode = WhisperContextMode::Single;
        let graph = whisper_graph(
            &transcription,
            &format!("kestrel_speech/{input_name}"),
            context_mode,
        );
        let result = self
            .execute_whisper_graph(&request.job_id, graph, context_mode, None, cancel)
            .await;
        let _ = tokio::fs::remove_file(&input_path).await;
        let WhisperTranscriptionResult {
            segments, words, ..
        } = result?;
        write_synthesis_receipt(
            &target,
            &request,
            voice,
            &segments,
            &words,
            Some(&request.alignment_model_id),
            true,
        )?;
        emit_alignment_progress(
            app,
            &request,
            "complete",
            "Exact local speech word timings are ready for click-to-seek.",
        );
        Ok(alignment_clip(&request, voice, segments, words, false))
    }

    async fn execute_whisper_graph(
        &self,
        job_id: &str,
        graph: Value,
        context_mode: WhisperContextMode,
        transcript_reference: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<WhisperTranscriptionResult, SpeechError> {
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
                    return parse_whisper_output(entry, context_mode, transcript_reference);
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

    async fn verify_live_whisper_node(&self) -> Result<(), SpeechError> {
        let value: Value = self
            .http
            .get(format!("{COMFY_BASE}/object_info/KestrelWhisper"))
            .send()
            .await?
            .json()
            .await?;
        if whisper_node_contract_is_current(&value) {
            return Ok(());
        }
        Err(SpeechError::Unavailable(
            "The running ComfyUI still has an older Kestrel Whisper adapter loaded. Open Setup and Resume Local voice and dictation, then stop and restart ComfyUI before retrying lyric sync.".into(),
        ))
    }

    fn cache_target(
        &self,
        request: &SpeechSynthesisRequest,
        voice: &VoiceConditioning,
    ) -> Result<PathBuf, SpeechError> {
        safe_source(&request.source_kind, &request.source_id)?;
        Ok(self
            .cache_root
            .join("generated")
            .join(&request.source_kind)
            .join(&request.source_id)
            .join(format!("{}.opus", cache_key(request, voice))))
    }

    fn validate_alignment_request(
        &self,
        comfy_root: &str,
        request: &SpeechAlignmentRequest,
        voice: &VoiceConditioning,
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
        validate_voice_identity(&request.voice_profile_id, voice)?;
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
    voice: &VoiceConditioning,
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
    validate_voice_identity(&request.voice_profile_id, voice)?;
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

fn validate_voice_identity(
    requested_profile_id: &str,
    voice: &VoiceConditioning,
) -> Result<(), SpeechError> {
    safe_identifier(requested_profile_id, "voice profile ID")?;
    if requested_profile_id != voice.profile_id {
        return Err(SpeechError::Invalid(
            "the resolved voice does not match the requested voice profile".into(),
        ));
    }
    if let Some(path) = voice.reference_path.as_ref() {
        if !path.is_absolute() || !path.is_file() || voice.reference_sha256.is_none() {
            return Err(SpeechError::Invalid(
                "the selected custom voice reference is incomplete".into(),
            ));
        }
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

fn validate_file_transcription_request(
    comfy_root: &str,
    request: &SpeechFileTranscriptionRequest,
) -> Result<(), SpeechError> {
    let root = Path::new(comfy_root);
    if !root.is_absolute() || !root.join("main.py").is_file() {
        return Err(SpeechError::Invalid(
            "ComfyUI root must be an absolute local installation path".into(),
        ));
    }
    safe_identifier(&request.job_id, "job ID")?;
    safe_identifier(&request.recording_id, "recording ID")?;
    let metadata = fs::metadata(&request.audio_path)?;
    if !request.audio_path.is_absolute()
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_TRANSCRIPTION_FILE_BYTES
    {
        return Err(SpeechError::Invalid(
            "music transcription audio is missing, empty, or exceeds 512 MiB".into(),
        ));
    }
    let extension = request
        .audio_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if !matches!(
        extension.to_ascii_lowercase().as_str(),
        "wav" | "flac" | "mp3" | "ogg" | "opus" | "webm" | "m4a" | "mp4"
    ) {
        return Err(SpeechError::Invalid(
            "music transcription requires WAV, FLAC, MP3, Ogg/Opus, WebM, M4A, or MP4 audio".into(),
        ));
    }
    if request.prompt.len() > MAX_FILE_TRANSCRIPTION_PROMPT_BYTES {
        return Err(SpeechError::Invalid(format!(
            "music lyric guidance exceeds {MAX_FILE_TRANSCRIPTION_PROMPT_BYTES} UTF-8 bytes"
        )));
    }
    if request.language.is_empty()
        || request.language.len() > 64
        || !request
            .language
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || matches!(byte, b' ' | b'-'))
    {
        return Err(SpeechError::Invalid("unsafe transcription language".into()));
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
    voice: &VoiceConditioning,
) -> Option<(Vec<SpeechTiming>, Vec<SpeechTiming>)> {
    let receipt = read_receipt_recoverable(&sidecar_path(audio_path))?;
    if receipt.get("alignmentModelId").and_then(Value::as_str)
        != Some(request.alignment_model_id.as_str())
        || receipt.get("modelId").and_then(Value::as_str) != Some(request.voice_model_id.as_str())
        || receipt.get("voiceProfileId").and_then(Value::as_str)
            != Some(request.voice_profile_id.as_str())
        || receipt.get("voiceReferenceSha256").and_then(Value::as_str)
            != voice.reference_sha256.as_deref()
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
    voice: &VoiceConditioning,
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
        "voiceProfileId": request.voice_profile_id,
        "voiceReferenceSha256": voice.reference_sha256,
        "performance": voice.performance,
        "alignmentModelId": alignment_model_id,
        "text": spoken_text(&request.text),
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

fn expand_decimal_points(raw: &str) -> String {
    let characters = raw.chars().collect::<Vec<_>>();
    let mut expanded = String::with_capacity(raw.len());
    for (index, character) in characters.iter().enumerate() {
        if *character == '.'
            && index > 0
            && index + 1 < characters.len()
            && characters[index - 1].is_ascii_digit()
            && characters[index + 1].is_ascii_digit()
        {
            expanded.push_str(" point ");
        } else {
            expanded.push(*character);
        }
    }
    expanded
}

pub fn clean_speech_text(raw: &str) -> String {
    if raw.trim().is_empty() {
        return String::new();
    }
    let mut text = expand_decimal_points(raw)
        .replace("e.g.", "for example")
        .replace("E.g.", "For example")
        .replace("i.e.", "that is")
        .replace("I.e.", "That is")
        .replace("CO₂", "carbon dioxide")
        .replace("co₂", "carbon dioxide")
        .replace("O₂", "oxygen")
        .replace("o₂", "oxygen")
        .replace('Δ', " change in ")
        .replace('≥', " at least ")
        .replace('≤', " at most ")
        .replace('↑', " rising ")
        .replace('↓', " falling ")
        .replace('→', " then ");

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
            | '³' | 'µ' | '¶' | '©' | '®' | '™' | '•' | '·' | '‣' | '⁃' | '✓' | '✔' | '✕' | '✖'
            | '✗' | '★' | '☆' | '▲' | '▼' | '◄' | '►' | '◆' | '◇' | '●' | '○' | '■' | '□' | '`'
            | '_' => {
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
                    || (0x2600..=0x27BF).contains(&u)
                    || (0x2B00..=0x2BFF).contains(&u)
                    || (0x1F300..=0x1FAFF).contains(&u)
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

fn spoken_text(raw: &str) -> String {
    let cleaned = clean_speech_text(raw);
    if cleaned.is_empty() {
        raw.trim().to_string()
    } else {
        cleaned
    }
}

fn spoken_alignment_request(request: &SpeechAlignmentRequest) -> SpeechAlignmentRequest {
    SpeechAlignmentRequest {
        text: spoken_text(&request.text),
        ..request.clone()
    }
}

fn chatterbox_graph(
    request: &SpeechSynthesisRequest,
    prefix: &str,
    voice: &VoiceConditioning,
    voice_input: Option<&str>,
) -> Value {
    let pack = request
        .model_id
        .strip_prefix("chatterbox:")
        .unwrap_or_default();
    let speech_text = spoken_text(&request.text);
    let word_count = speech_text.split_whitespace().count() as u32;
    let max_new_tokens = (word_count.saturating_mul(18).saturating_add(96)).clamp(128, 1_600);
    let seed = deterministic_seed(request, voice);
    let (flow_cfg_scale, exaggeration, temperature, cfg_weight) =
        performance_parameters(&voice.performance);
    let mut graph = json!({
        "1": {
            "class_type": "ChatterboxTTS",
            "inputs": {
                "model_pack_name": pack,
                "text": speech_text,
                "max_new_tokens": max_new_tokens,
                "flow_cfg_scale": flow_cfg_scale,
                "exaggeration": exaggeration,
                "temperature": temperature,
                "cfg_weight": cfg_weight,
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
    });
    if let Some(input) = voice_input {
        graph["0"] = json!({"class_type":"LoadAudio","inputs":{"audio":input}});
        graph["1"]["inputs"]["audio_prompt"] = json!(["0", 0]);
    }
    graph
}

fn performance_parameters(performance: &str) -> (f64, f64, f64, f64) {
    match performance {
        "restrained" => (0.75, 0.35, 0.65, 0.55),
        "expressive" => (0.7, 0.8, 0.9, 0.5),
        "dramatic" => (0.65, 1.15, 0.95, 0.45),
        _ => (0.7, 0.5, 0.8, 0.5),
    }
}

fn prepare_voice_input(
    comfy_root: &Path,
    voice: &VoiceConditioning,
) -> Result<Option<String>, SpeechError> {
    let Some(source) = voice.reference_path.as_ref() else {
        return Ok(None);
    };
    let hash = voice.reference_sha256.as_deref().ok_or_else(|| {
        SpeechError::Invalid("custom voice reference has no integrity hash".into())
    })?;
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| value.len() <= 5 && value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .ok_or_else(|| {
            SpeechError::Invalid("custom voice reference has no safe extension".into())
        })?;
    let input_directory = comfy_root.join("input/kestrel_speech/voices");
    fs::create_dir_all(&input_directory)?;
    let file_name = format!("{hash}.{extension}");
    let target = input_directory.join(&file_name);
    let reusable = target.is_file()
        && target.metadata()?.len() == source.metadata()?.len()
        && sha256_path(&target).is_ok_and(|actual| actual == hash);
    if !reusable {
        let temporary = target.with_extension(format!("{extension}.tmp"));
        fs::copy(source, &temporary)?;
        if sha256_path(&temporary)? != hash {
            let _ = fs::remove_file(&temporary);
            return Err(SpeechError::Invalid(
                "custom voice reference changed while preparing it for ComfyUI".into(),
            ));
        }
        if target.is_file() {
            fs::remove_file(&target)?;
        }
        if let Err(error) = fs::rename(&temporary, &target) {
            let _ = fs::remove_file(&temporary);
            if !target.is_file() {
                return Err(error.into());
            }
        }
    }
    Ok(Some(format!("kestrel_speech/voices/{file_name}")))
}

fn sha256_path(path: &Path) -> Result<String, SpeechError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn whisper_graph(
    request: &SpeechTranscriptionRequest,
    input_relative: &str,
    context_mode: WhisperContextMode,
) -> Value {
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
                "prompt": request.prompt.trim(),
                "context_mode": context_mode.as_str()
            }
        },
        "3": {"class_type": "PreviewAny", "inputs": {"source": ["2", 0]}},
        "4": {"class_type": "PreviewAny", "inputs": {"source": ["2", 1]}},
        "5": {"class_type": "PreviewAny", "inputs": {"source": ["2", 2]}},
        "6": {"class_type": "PreviewAny", "inputs": {"source": ["2", 3]}}
    })
}

fn whisper_node_contract_is_current(value: &Value) -> bool {
    let Some(node) = value.get("KestrelWhisper") else {
        return false;
    };
    node.pointer("/input/required/context_mode").is_some()
        && node
            .get("output")
            .and_then(Value::as_array)
            .is_some_and(|outputs| outputs.len() == 4)
        && node
            .get("output_name")
            .and_then(Value::as_array)
            .and_then(|names| names.get(3))
            .and_then(Value::as_str)
            == Some("context_json")
}

fn parse_whisper_output(
    entry: &Value,
    context_mode: WhisperContextMode,
    transcript_reference: Option<&str>,
) -> Result<WhisperTranscriptionResult, SpeechError> {
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
        if value.len() > MAX_TRANSCRIPT_TIMING_BYTES {
            return Err(SpeechError::Unavailable(format!(
                "ComfyUI Whisper returned {label} timestamps larger than the 16 MiB local safety limit."
            )));
        }
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
    let segments = parse_timing("4", "segment")?;
    let words = parse_timing("5", "word")?;
    let context_value = output_text("6").ok_or_else(|| {
        SpeechError::Unavailable(
            "ComfyUI Whisper completed without the current context boundary. Resume Local voice and dictation in Setup, stop and restart ComfyUI, then retry.".into(),
        )
    })?;
    if context_value.len() > 4_096 {
        return Err(SpeechError::Unavailable(
            "ComfyUI Whisper returned an oversized context boundary.".into(),
        ));
    }
    let context: WhisperContextMetadata = serde_json::from_str(context_value).map_err(|error| {
        SpeechError::Unavailable(format!(
            "ComfyUI Whisper returned an invalid context boundary: {error}"
        ))
    })?;
    validate_whisper_context(&context, context_mode)?;
    if context_mode == WhisperContextMode::Single {
        return Ok(WhisperTranscriptionResult {
            text,
            segments,
            words,
            selected_context_copy: 1,
            first_context_score: 0.0,
            second_context_score: 0.0,
        });
    }
    let transcript_reference = transcript_reference.ok_or_else(|| {
        SpeechError::Unavailable(
            "Kestrel cannot select a repeated music pass without the authoritative lyrics.".into(),
        )
    })?;
    let first = select_context_candidate(0.0, context.source_duration, &segments, &words);
    let second =
        select_context_candidate(context.second_start, context.second_end, &segments, &words);
    let (selected_context_copy, selected, first_context_score, second_context_score) =
        select_music_context_candidate(transcript_reference, first, second);
    Ok(WhisperTranscriptionResult {
        text: selected.text,
        segments: selected.segments,
        words: selected.words,
        selected_context_copy,
        first_context_score,
        second_context_score,
    })
}

fn validate_whisper_context(
    context: &WhisperContextMetadata,
    expected: WhisperContextMode,
) -> Result<(), SpeechError> {
    let values_are_safe = context.source_duration.is_finite()
        && context.second_start.is_finite()
        && context.second_end.is_finite()
        && context.seam_seconds.is_finite()
        && context.source_duration > 0.0
        && context.source_duration <= 24.0 * 60.0 * 60.0
        && context.second_start >= 0.0
        && context.second_end >= context.second_start
        && context.second_end <= 24.0 * 60.0 * 60.0
        && context.seam_seconds >= 0.0;
    let duration_matches =
        (context.second_end - context.second_start - context.source_duration).abs() <= 0.05;
    let mode_matches = context.mode == expected.as_str();
    let single_boundary_matches = expected != WhisperContextMode::Single
        || (context.second_start <= 0.05 && context.seam_seconds <= 0.05);
    let repeated_boundary_matches = expected != WhisperContextMode::RepeatedMusic
        || (context.source_duration <= MAX_MUSIC_CONTEXT_SOURCE_SECONDS
            && context.second_start >= context.source_duration
            && (context.seam_seconds - MUSIC_REPEAT_CONTEXT_SEAM_SECONDS).abs() <= 0.05
            && (context.second_start - context.source_duration - context.seam_seconds).abs()
                <= 0.05);
    if values_are_safe
        && duration_matches
        && mode_matches
        && single_boundary_matches
        && repeated_boundary_matches
    {
        Ok(())
    } else {
        Err(SpeechError::Unavailable(
            "ComfyUI Whisper returned an unsafe or mismatched context boundary.".into(),
        ))
    }
}

fn select_context_candidate(
    source_start: f64,
    source_end: f64,
    segments: &[SpeechTiming],
    words: &[SpeechTiming],
) -> WhisperTranscriptionResult {
    let selected_words = words
        .iter()
        .filter_map(|word| rebase_context_timing(word, source_start, source_end))
        .collect::<Vec<_>>();
    let selected_segments = segments
        .iter()
        .filter_map(|segment| {
            let crosses_boundary = segment.start < source_start || segment.end > source_end;
            let rebased = rebase_context_timing(segment, source_start, source_end)?;
            let value = if crosses_boundary {
                let boundary_words = words
                    .iter()
                    .filter(|word| {
                        let midpoint = (word.start + word.end) / 2.0;
                        midpoint >= source_start
                            && midpoint <= source_end
                            && midpoint >= segment.start
                            && midpoint <= segment.end
                    })
                    .map(|word| word.value.trim())
                    .filter(|value| !value.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                if !boundary_words.is_empty() {
                    boundary_words
                } else {
                    segment.value.clone()
                }
            } else {
                segment.value.clone()
            };
            (!value.trim().is_empty()).then_some(SpeechTiming { value, ..rebased })
        })
        .collect::<Vec<_>>();
    let text = if selected_segments.is_empty() {
        selected_words
            .iter()
            .map(|word| word.value.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        selected_segments
            .iter()
            .map(|segment| segment.value.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    WhisperTranscriptionResult {
        text,
        segments: selected_segments,
        words: selected_words,
        selected_context_copy: 0,
        first_context_score: 0.0,
        second_context_score: 0.0,
    }
}

fn select_music_context_candidate(
    lyrics: &str,
    first: WhisperTranscriptionResult,
    second: WhisperTranscriptionResult,
) -> (u8, WhisperTranscriptionResult, f64, f64) {
    let reference = lyric_reference_tokens(lyrics);
    let first_score = music_candidate_score(&reference, &first);
    let second_score = music_candidate_score(&reference, &second);
    if second_score > first_score + 0.01 {
        (2, second, first_score, second_score)
    } else {
        (1, first, first_score, second_score)
    }
}

fn lyric_reference_tokens(lyrics: &str) -> Vec<String> {
    lyrics
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !is_music_section_heading(line))
        .flat_map(transcript_tokens)
        .take(MAX_MUSIC_SCORING_TOKENS)
        .collect()
}

fn transcript_tokens(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

fn music_candidate_score(reference: &[String], candidate: &WhisperTranscriptionResult) -> f64 {
    let candidate_text = if candidate.words.is_empty() {
        candidate.text.clone()
    } else {
        candidate
            .words
            .iter()
            .map(|word| word.value.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut candidate_tokens = transcript_tokens(&candidate_text);
    candidate_tokens.truncate(MAX_MUSIC_SCORING_TOKENS);
    let full_score = ordered_token_f1(reference, &candidate_tokens);
    let reference_opening = &reference[..reference.len().min(48)];
    let candidate_opening = &candidate_tokens[..candidate_tokens.len().min(64)];
    let opening_score = ordered_token_f1(reference_opening, candidate_opening);
    full_score * 0.7 + opening_score * 0.3
}

fn ordered_token_f1(left: &[String], right: &[String]) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let mut previous = vec![0_usize; right.len() + 1];
    let mut current = vec![0_usize; right.len() + 1];
    for left_token in left {
        for (index, right_token) in right.iter().enumerate() {
            current[index + 1] = if left_token == right_token {
                previous[index] + 1
            } else {
                current[index].max(previous[index + 1])
            };
        }
        std::mem::swap(&mut previous, &mut current);
        current.fill(0);
    }
    let overlap = previous[right.len()] as f64;
    2.0 * overlap / (left.len() + right.len()) as f64
}

fn rebase_context_timing(
    timing: &SpeechTiming,
    source_start: f64,
    source_end: f64,
) -> Option<SpeechTiming> {
    if timing.end <= source_start || timing.start >= source_end {
        return None;
    }
    let start = timing.start.max(source_start).min(source_end) - source_start;
    let end = timing.end.max(source_start).min(source_end) - source_start;
    Some(SpeechTiming {
        value: timing.value.clone(),
        start,
        end: end.max(start),
    })
}

fn deterministic_seed(request: &SpeechSynthesisRequest, voice: &VoiceConditioning) -> u64 {
    let digest = Sha256::digest(format!(
        "{TTS_ADAPTER_REVISION}\0{}\0{}\0{}\0{}\0{}",
        request.model_id,
        voice.profile_id,
        voice.fingerprint(),
        voice.performance,
        request.text.trim()
    ));
    u64::from_le_bytes(digest[..8].try_into().expect("eight-byte digest prefix")).max(1)
}

fn cache_key(request: &SpeechSynthesisRequest, voice: &VoiceConditioning) -> String {
    let digest = Sha256::digest(format!(
        "{TTS_ADAPTER_REVISION}\0{}\0{}\0{}\0{}\0{}",
        request.model_id,
        voice.profile_id,
        voice.fingerprint(),
        voice.performance,
        request.text.trim()
    ));
    hex::encode(digest)
}

fn clip_receipt(
    cache_root: &Path,
    request: &SpeechSynthesisRequest,
    voice: &VoiceConditioning,
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
    let existing_receipt = read_receipt_recoverable(&receipt_path);
    let (segments, words) = existing_receipt
        .as_ref()
        .and_then(receipt_timings)
        .unwrap_or_default();
    let clip = SpeechClip {
        job_id: request.job_id.clone(),
        passage_id: request.passage_id.clone(),
        relative_path,
        model_id: request.model_id.clone(),
        voice_profile_id: request.voice_profile_id.clone(),
        cache_hit,
        segments,
        words,
    };
    let created_at = existing_receipt
        .as_ref()
        .and_then(|value| value.get("createdAt"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let alignment_model_id = existing_receipt
        .as_ref()
        .and_then(|value| value.get("alignmentModelId"))
        .cloned()
        .unwrap_or(Value::Null);
    write_json_atomic(
        &receipt_path,
        &json!({
            "revision": TTS_ADAPTER_REVISION,
            "sourceKind": request.source_kind,
            "sourceId": request.source_id,
            "passageId": request.passage_id,
            "modelId": request.model_id,
            "voiceProfileId": request.voice_profile_id,
            "voiceReferenceSha256": voice.reference_sha256,
            "performance": voice.performance,
            "text": spoken_text(&request.text),
            "audioRelativePath": clip.relative_path,
            "segments": clip.segments,
            "words": clip.words,
            "alignmentModelId": alignment_model_id,
            "createdAt": created_at,
        }),
    )?;
    Ok(clip)
}

fn alignment_clip(
    request: &SpeechAlignmentRequest,
    voice: &VoiceConditioning,
    segments: Vec<SpeechTiming>,
    words: Vec<SpeechTiming>,
    cache_hit: bool,
) -> SpeechClip {
    SpeechClip {
        job_id: request.job_id.clone(),
        passage_id: request.passage_id.clone(),
        relative_path: request.relative_path.clone(),
        model_id: request.voice_model_id.clone(),
        voice_profile_id: voice.profile_id.clone(),
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

pub(crate) fn slice_wav_pcm(
    input_path: &Path,
    output_path: &Path,
    start_seconds: f64,
    end_seconds: f64,
) -> Result<(), std::io::Error> {
    let bytes = std::fs::read(input_path)?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not a valid RIFF/WAVE file",
        ));
    }
    let mut pos = 12;
    let mut fmt_opt: Option<(u16, u16, u32, u32, u16, u16)> = None;
    let mut data_chunk: Option<(usize, usize)> = None;
    while pos + 8 <= bytes.len() {
        let chunk_id = &bytes[pos..pos + 4];
        let chunk_len = u32::from_le_bytes(
            bytes[pos + 4..pos + 8]
                .try_into()
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid chunk size"))?,
        ) as usize;
        let data_start = pos + 8;
        if chunk_id == b"fmt " && chunk_len >= 16 && data_start + 16 <= bytes.len() {
            let format_tag = u16::from_le_bytes(bytes[data_start..data_start + 2].try_into().unwrap());
            let channels = u16::from_le_bytes(bytes[data_start + 2..data_start + 4].try_into().unwrap());
            let sample_rate = u32::from_le_bytes(bytes[data_start + 4..data_start + 8].try_into().unwrap());
            let byte_rate = u32::from_le_bytes(bytes[data_start + 8..data_start + 12].try_into().unwrap());
            let block_align = u16::from_le_bytes(bytes[data_start + 12..data_start + 14].try_into().unwrap());
            let bits_per_sample = u16::from_le_bytes(bytes[data_start + 14..data_start + 16].try_into().unwrap());
            fmt_opt = Some((format_tag, channels, sample_rate, byte_rate, block_align, bits_per_sample));
        } else if chunk_id == b"data" {
            let actual_data_len = chunk_len.min(bytes.len().saturating_sub(data_start));
            data_chunk = Some((data_start, actual_data_len));
            break;
        }
        pos = data_start + chunk_len + (chunk_len % 2);
    }
    let (format_tag, channels, sample_rate, _byte_rate, block_align, bits_per_sample) = fmt_opt
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing fmt chunk"))?;
    let (data_start, data_len) = data_chunk
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing data chunk"))?;

    let bytes_per_sec = sample_rate as f64 * block_align as f64;
    let start_sample_byte = ((start_seconds * bytes_per_sec) as usize / block_align as usize) * block_align as usize;
    let end_sample_byte = ((end_seconds * bytes_per_sec) as usize / block_align as usize) * block_align as usize;
    let slice_start = start_sample_byte.min(data_len);
    let slice_end = end_sample_byte.min(data_len).max(slice_start);
    let sliced_audio_bytes = &bytes[data_start + slice_start..data_start + slice_end];
    let sliced_len = sliced_audio_bytes.len() as u32;

    let mut out = Vec::with_capacity(44 + sliced_audio_bytes.len());
    let total_file_size = 36 + sliced_len;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&total_file_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&format_tag.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * block_align as u32).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits_per_sample.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&sliced_len.to_le_bytes());
    out.extend_from_slice(sliced_audio_bytes);

    std::fs::write(output_path, out)?;
    Ok(())
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

fn emit_file_transcription_progress(
    app: Option<&AppHandle>,
    request: &SpeechFileTranscriptionRequest,
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

fn music_opening_prompt(lyrics: &str) -> String {
    let mut prompt = String::new();
    let mut lyric_lines = 0_usize;
    for line in lyrics.lines().map(str::trim) {
        if line.is_empty() || is_music_section_heading(line) {
            continue;
        }
        let separator_bytes = usize::from(!prompt.is_empty());
        if prompt.len() + separator_bytes >= MAX_MUSIC_OPENING_PROMPT_BYTES {
            break;
        }
        let remaining = MAX_MUSIC_OPENING_PROMPT_BYTES - prompt.len() - separator_bytes;
        let fragment = bounded_prefix(line, remaining);
        if fragment.is_empty() {
            break;
        }
        if !prompt.is_empty() {
            prompt.push('\n');
        }
        prompt.push_str(&fragment);
        lyric_lines += 1;
        if lyric_lines >= MAX_MUSIC_OPENING_PROMPT_LINES || fragment.len() < line.len() {
            break;
        }
    }
    prompt
}

fn is_music_section_heading(line: &str) -> bool {
    if line.len() > 96 {
        return false;
    }
    let delimited = [('[', ']'), ('{', '}'), ('(', ')')];
    delimited
        .iter()
        .any(|(start, end)| line.starts_with(*start) && line.ends_with(*end))
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
            voice_profile_id: "voice-default".into(),
        }
    }

    fn default_conditioning() -> VoiceConditioning {
        VoiceConditioning {
            profile_id: "voice-default".into(),
            reference_path: None,
            reference_sha256: None,
            performance: "natural".into(),
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

    #[test]
    fn durable_file_transcription_accepts_only_bounded_local_audio_and_installed_whisper() {
        let comfy = tempfile::tempdir().unwrap();
        fs::write(comfy.path().join("main.py"), b"# local comfy").unwrap();
        complete_whisper(comfy.path(), "large-v3-turbo");
        let audio_root = tempfile::tempdir().unwrap();
        let audio = audio_root.path().join("take.flac");
        fs::write(&audio, b"fLaC local music").unwrap();
        let request = SpeechFileTranscriptionRequest {
            job_id: "music-sync-1".into(),
            recording_id: "take-1".into(),
            audio_path: audio.clone(),
            model_id: "whisper:large-v3-turbo".into(),
            language: "auto".into(),
            prompt: "Known producer lyrics".into(),
        };
        assert!(
            validate_file_transcription_request(&comfy.path().to_string_lossy(), &request).is_ok()
        );

        let mut unsafe_request = request;
        unsafe_request.audio_path = PathBuf::from("relative.flac");
        assert!(validate_file_transcription_request(
            &comfy.path().to_string_lossy(),
            &unsafe_request
        )
        .is_err());
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
            voice_profile_id: "voice-default".into(),
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
        let voice = default_conditioning();
        let graph = chatterbox_graph(&request, "kestrel_research/test", &voice, None);
        assert_eq!(graph["1"]["class_type"], "ChatterboxTTS");
        assert_eq!(
            graph["1"]["inputs"]["model_pack_name"],
            "resembleai_default_voice"
        );
        assert_eq!(graph["1"]["inputs"]["text"], request.text);
        assert_eq!(graph["2"]["class_type"], "SaveAudioOpus");
        assert_eq!(graph["2"]["inputs"]["audio"], json!(["1", 0]));
        assert_eq!(graph["2"]["inputs"]["quality"], "64k");
        assert_eq!(cache_key(&request, &voice), cache_key(&request, &voice));
        assert_ne!(
            cache_key(&request, &voice),
            cache_key(&self::request("Different text."), &voice)
        );
    }

    #[test]
    fn custom_voice_graph_is_conditioned_and_has_an_independent_cache_identity() {
        let request = request("A concise locally generated research passage.");
        let built_in = default_conditioning();
        let custom = VoiceConditioning {
            profile_id: "voice-evening-narrator".into(),
            reference_path: Some(PathBuf::from("C:/private/voice-reference.wav")),
            reference_sha256: Some("a".repeat(64)),
            performance: "expressive".into(),
        };
        let graph = chatterbox_graph(
            &request,
            "kestrel_research/custom",
            &custom,
            Some("kestrel_speech/voices/reference.wav"),
        );

        assert_eq!(graph["0"]["class_type"], "LoadAudio");
        assert_eq!(
            graph["0"]["inputs"]["audio"],
            "kestrel_speech/voices/reference.wav"
        );
        assert_eq!(graph["1"]["inputs"]["audio_prompt"], json!(["0", 0]));
        assert_eq!(graph["1"]["inputs"]["exaggeration"], 0.8);
        assert_ne!(cache_key(&request, &built_in), cache_key(&request, &custom));

        let mut changed_reference = custom.clone();
        changed_reference.reference_sha256 = Some("b".repeat(64));
        assert_ne!(
            cache_key(&request, &custom),
            cache_key(&request, &changed_reference)
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
        let graph = whisper_graph(
            &request,
            "kestrel_speech/recording.webm",
            WhisperContextMode::Single,
        );
        assert_eq!(graph["1"]["class_type"], "LoadAudio");
        assert_eq!(graph["2"]["class_type"], "KestrelWhisper");
        assert_eq!(graph["2"]["inputs"]["model"], "large-v3-turbo");
        assert_eq!(graph["2"]["inputs"]["language"], "auto");
        assert_eq!(graph["2"]["inputs"]["context_mode"], "single");
        assert_eq!(graph["3"]["class_type"], "PreviewAny");
        assert_eq!(graph["4"]["inputs"]["source"], json!(["2", 1]));
        assert_eq!(graph["5"]["inputs"]["source"], json!(["2", 2]));
        assert_eq!(graph["6"]["inputs"]["source"], json!(["2", 3]));

        let repeated = whisper_graph(
            &request,
            "kestrel_speech/music.flac",
            WhisperContextMode::RepeatedMusic,
        );
        assert_eq!(repeated["2"]["inputs"]["context_mode"], "music-repeat");
    }

    #[test]
    fn live_whisper_contract_rejects_a_three_output_adapter_until_restart() {
        let old = json!({"KestrelWhisper": {
            "input": {"required": {"audio": ["AUDIO"]}},
            "output": ["STRING", "STRING", "STRING"],
            "output_name": ["transcript", "segments_json", "words_json"]
        }});
        assert!(!whisper_node_contract_is_current(&old));

        let current = json!({"KestrelWhisper": {
            "input": {"required": {
                "audio": ["AUDIO"],
                "context_mode": [["single", "music-repeat"], {"default": "single"}]
            }},
            "output": ["STRING", "STRING", "STRING", "STRING"],
            "output_name": ["transcript", "segments_json", "words_json", "context_json"]
        }});
        assert!(whisper_node_contract_is_current(&current));
    }

    #[test]
    fn parses_and_validates_whisper_word_and_sentence_timings() {
        let entry = json!({"outputs": {
            "3": {"text": ["Good morning."]},
            "4": {"text": ["[{\"value\":\"Good morning.\",\"start\":0.0,\"end\":1.2}]"]},
            "5": {"text": ["[{\"value\":\"Good\",\"start\":0.0,\"end\":0.5},{\"value\":\"morning.\",\"start\":0.5,\"end\":1.2}]"]},
            "6": {"text": ["{\"mode\":\"single\",\"sourceDuration\":1.2,\"secondStart\":0.0,\"secondEnd\":1.2,\"seamSeconds\":0.0}"]}
        }});
        let result = parse_whisper_output(&entry, WhisperContextMode::Single, None).unwrap();
        assert_eq!(result.text, "Good morning.");
        assert_eq!(result.segments.len(), 1);
        assert_eq!(result.words.len(), 2);
        assert_eq!(result.words[1].value, "morning.");

        let unsafe_entry = json!({"outputs": {
            "3": {"text": ["Bad"]},
            "4": {"text": ["[{\"value\":\"Bad\",\"start\":2.0,\"end\":1.0}]"]},
            "5": {"text": ["[]"]},
            "6": {"text": ["{\"mode\":\"single\",\"sourceDuration\":2.0,\"secondStart\":0.0,\"secondEnd\":2.0,\"seamSeconds\":0.0}"]}
        }});
        assert!(parse_whisper_output(&unsafe_entry, WhisperContextMode::Single, None).is_err());
    }

    #[test]
    fn repeated_music_context_keeps_only_copy_two_and_rebases_its_opening() {
        let entry = json!({"outputs": {
            "3": {"text": ["discarded first copy tail Opening now second discarded suffix"]},
            "4": {"text": ["[{\"value\":\"discarded first copy\",\"start\":0.0,\"end\":3.0},{\"value\":\"tail Opening now\",\"start\":9.8,\"end\":11.0},{\"value\":\"second\",\"start\":12.0,\"end\":13.0},{\"value\":\"discarded suffix\",\"start\":19.0,\"end\":20.0}]"]},
            "5": {"text": ["[{\"value\":\"tail\",\"start\":9.8,\"end\":9.95},{\"value\":\"Opening\",\"start\":10.0,\"end\":10.5},{\"value\":\"now\",\"start\":10.5,\"end\":11.0},{\"value\":\"second\",\"start\":12.0,\"end\":13.0},{\"value\":\"suffix\",\"start\":19.0,\"end\":19.5}]"]},
            "6": {"text": ["{\"mode\":\"music-repeat\",\"sourceDuration\":9.0,\"secondStart\":10.0,\"secondEnd\":19.0,\"seamSeconds\":1.0}"]}
        }});
        let result = parse_whisper_output(
            &entry,
            WhisperContextMode::RepeatedMusic,
            Some("Opening now second"),
        )
        .unwrap();
        assert_eq!(result.selected_context_copy, 2);
        assert_eq!(result.text, "Opening now second");
        assert_eq!(result.segments.len(), 2);
        assert_eq!(result.segments[0].value, "Opening now");
        assert_eq!(result.segments[0].start, 0.0);
        assert_eq!(result.words.len(), 3);
        assert_eq!(result.words[0].value, "Opening");
        assert_eq!(result.words[0].start, 0.0);
        assert!(result.segments.iter().all(|timing| timing.end <= 9.0));
        assert!(result.words.iter().all(|timing| timing.end <= 9.0));
    }

    #[test]
    fn repeated_music_selector_rejects_tail_context_that_skips_the_opening_verse() {
        let candidate = |text: &str| WhisperTranscriptionResult {
            text: text.into(),
            segments: vec![SpeechTiming {
                value: text.into(),
                start: 15.0,
                end: 29.0,
            }],
            words: text
                .split_whitespace()
                .enumerate()
                .map(|(index, value)| SpeechTiming {
                    value: value.into(),
                    start: 15.0 + index as f64 * 0.2,
                    end: 15.2 + index as f64 * 0.2,
                })
                .collect(),
            selected_context_copy: 0,
            first_context_score: 0.0,
            second_context_score: 0.0,
        };
        let lyrics = "[Verse]\nI am the stone that holds the night\nA glowing square in the endless void\n[Chorus]\nHold the weight of the galaxies";
        let first = candidate(
            "I am the stone that holds the night A glowing square in the endless void Hold the weight of the galaxies",
        );
        let second = candidate("Hold the weight of the galaxies");
        let (selected_copy, selected, first_score, second_score) =
            select_music_context_candidate(lyrics, first, second);
        assert_eq!(selected_copy, 1);
        assert!(selected.text.starts_with("I am the stone"));
        assert!(first_score > second_score);

        let first = candidate("unrelated opening noise");
        let second = candidate(
            "I am the stone that holds the night A glowing square in the endless void Hold the weight of the galaxies",
        );
        let (selected_copy, _, first_score, second_score) =
            select_music_context_candidate(lyrics, first, second);
        assert_eq!(selected_copy, 2);
        assert!(second_score > first_score);
    }

    #[test]
    fn music_prompt_keeps_only_a_bounded_opening_lyric_excerpt() {
        let lyrics = "[Intro]\n(whispered)\nFirst lyric line\nSecond lyric line\n[Chorus]\nThird lyric line\nFourth lyric line\nFifth lyric line";
        assert_eq!(
            music_opening_prompt(lyrics),
            "First lyric line\nSecond lyric line\nThird lyric line\nFourth lyric line"
        );

        let multibyte = format!("{} trailing text", "歌".repeat(400));
        let prompt = music_opening_prompt(&multibyte);
        assert!(prompt.len() <= MAX_MUSIC_OPENING_PROMPT_BYTES);
        assert!(prompt.is_char_boundary(prompt.len()));
        assert!(multibyte.starts_with(&prompt));
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
        let voice = default_conditioning();
        let target = speech.cache_target(&synthesis, &voice).unwrap();
        let mut audio = vec![0_u8; 96];
        audio[..4].copy_from_slice(b"OggS");
        write_recording_atomic(&target, &audio).unwrap();
        let clip = clip_receipt(&speech.cache_root, &synthesis, &voice, &target, false).unwrap();
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
            &voice,
            &segments,
            &words,
            Some(alignment.alignment_model_id.as_str()),
            true,
        )
        .unwrap();

        assert_eq!(fs::read(&target).unwrap(), audio);
        let cached = speech
            .cached_alignment(&comfy.path().to_string_lossy(), &alignment, &voice)
            .unwrap()
            .unwrap();
        assert!(cached.cache_hit);
        assert_eq!(cached.words[0].start, 0.0);
        assert_eq!(
            speech
                .cached_clip(&comfy.path().to_string_lossy(), &synthesis, &voice)
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
            .cached_alignment(&comfy.path().to_string_lossy(), &alignment, &voice)
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

        let dashboard = clean_speech_text("O₂ ≥ 20%; CO₂ ≤ 0.45%; Δ O₂ → stable ↑ while reserve ↓");
        assert!(dashboard.contains("oxygen at least 20%"));
        assert!(dashboard.contains("carbon dioxide at most 0 point 45%"));
        assert!(dashboard.contains("change in oxygen then stable rising"));
        assert!(dashboard.ends_with("reserve falling"));
        assert_eq!(
            expand_decimal_points("19.8 and v1.2.3"),
            "19 point 8 and v1 point 2 point 3"
        );

        let unterminated = clean_speech_text("Intro text\n```json\n{\"still_open\": true}");
        assert!(unterminated.contains("Intro text"));
        assert!(!unterminated.contains("```"));
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
        let voice = default_conditioning();
        speech
            .ensure_comfy(&comfy_root.to_string_lossy(), &cancel)
            .await
            .unwrap();
        let clip = speech
            .synthesize(
                &comfy_root.to_string_lossy(),
                &request,
                &voice,
                &cancel,
                None,
            )
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
                .cached_clip(&comfy_root.to_string_lossy(), &request, &voice)
                .unwrap()
                .unwrap()
                .cache_hit
        );
    }

    #[tokio::test]
    #[ignore = "requires KESTREL_LIVE_COMFY_ROOT and an installed local ComfyUI-Chatterbox voice pack"]
    async fn live_custom_voice_reference_conditions_chatterbox() {
        let Some(comfy_root) = std::env::var_os("KESTREL_LIVE_COMFY_ROOT") else {
            panic!("KESTREL_LIVE_COMFY_ROOT is required");
        };
        let library = tempfile::tempdir().unwrap();
        let speech = LocalSpeech::new(library.path()).unwrap();
        let snapshot = speech.snapshot(&comfy_root.to_string_lossy()).await;
        assert!(snapshot.narration_available, "{}", snapshot.detail);
        let cancel = CancellationToken::new();
        speech
            .ensure_comfy(&comfy_root.to_string_lossy(), &cancel)
            .await
            .unwrap();

        let default_voice = default_conditioning();
        let reference_request = SpeechSynthesisRequest {
            model_id: snapshot.voices[0].id.clone(),
            ..request("This clean reference establishes a calm and precise local narrator voice.")
        };
        let reference_clip = speech
            .synthesize(
                &comfy_root.to_string_lossy(),
                &reference_request,
                &default_voice,
                &cancel,
                None,
            )
            .await
            .unwrap();
        let reference_path = library
            .path()
            .join("speech-cache")
            .join(reference_clip.relative_path);
        let reference_hash = sha256_path(&reference_path).unwrap();
        let custom_voice = VoiceConditioning {
            profile_id: "voice-live-reference".into(),
            reference_path: Some(reference_path),
            reference_sha256: Some(reference_hash),
            performance: "expressive".into(),
        };
        let custom_request = SpeechSynthesisRequest {
            job_id: "live-custom-voice".into(),
            passage_id: "custom-voice-result".into(),
            text: "The same private voice now delivers a more expressive second passage.".into(),
            model_id: snapshot.voices[0].id.clone(),
            voice_profile_id: custom_voice.profile_id.clone(),
            ..request("")
        };
        let custom_clip = speech
            .synthesize(
                &comfy_root.to_string_lossy(),
                &custom_request,
                &custom_voice,
                &cancel,
                None,
            )
            .await
            .unwrap();

        assert!(valid_cached_audio(
            &library
                .path()
                .join("speech-cache")
                .join(&custom_clip.relative_path)
        ));
        assert_eq!(custom_clip.voice_profile_id, custom_voice.profile_id);
        assert_ne!(
            cache_key(&reference_request, &default_voice),
            cache_key(&custom_request, &custom_voice)
        );
        speech.release_model_memory().await;
        speech.stop_comfy().await;
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
        let voice = default_conditioning();
        speech
            .ensure_comfy(&comfy_root.to_string_lossy(), &cancel)
            .await
            .unwrap();
        let synthesis = SpeechSynthesisRequest {
            model_id: snapshot.voices[0].id.clone(),
            ..request("Kestrel saves this private local voice recording with exact word timing.")
        };
        let clip = speech
            .synthesize(
                &comfy_root.to_string_lossy(),
                &synthesis,
                &voice,
                &cancel,
                None,
            )
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
            voice_profile_id: synthesis.voice_profile_id.clone(),
            alignment_model_id: snapshot.transcribers[0].id.clone(),
        };
        let aligned = speech
            .align(
                &comfy_root.to_string_lossy(),
                &alignment,
                &voice,
                &cancel,
                None,
            )
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
                .cached_alignment(&comfy_root.to_string_lossy(), &alignment, &voice)
                .unwrap()
                .unwrap()
                .cache_hit
        );
        speech.release_model_memory().await;
    }

    #[tokio::test]
    #[ignore = "requires KESTREL_LIVE_COMFY_ROOT, KESTREL_LIVE_SPEECH_AUDIT, and KESTREL_LIVE_SPEECH_OUTPUT plus installed Chatterbox and Whisper models"]
    async fn live_producer_passages_emit_replayable_highlight_timings() {
        let comfy_root = PathBuf::from(
            std::env::var_os("KESTREL_LIVE_COMFY_ROOT")
                .expect("KESTREL_LIVE_COMFY_ROOT is required"),
        );
        let input_path = PathBuf::from(
            std::env::var_os("KESTREL_LIVE_SPEECH_AUDIT")
                .expect("KESTREL_LIVE_SPEECH_AUDIT is required"),
        );
        let output_root = PathBuf::from(
            std::env::var_os("KESTREL_LIVE_SPEECH_OUTPUT")
                .expect("KESTREL_LIVE_SPEECH_OUTPUT is required"),
        );
        assert!(comfy_root.is_absolute());
        assert!(input_path.is_absolute() && input_path.is_file());
        assert!(output_root.is_absolute());
        fs::create_dir_all(&output_root).unwrap();

        let input: Value = serde_json::from_slice(&fs::read(&input_path).unwrap()).unwrap();
        let passage_audits = input
            .get("passageAudits")
            .and_then(Value::as_array)
            .expect("speech audit must contain passageAudits");
        assert!(!passage_audits.is_empty());

        let speech = LocalSpeech::new(&output_root).unwrap();
        let snapshot = speech.snapshot(&comfy_root.to_string_lossy()).await;
        assert!(snapshot.narration_available, "{}", snapshot.detail);
        assert!(snapshot.transcription_available, "{}", snapshot.detail);
        let voice_model = snapshot.voices[0].id.clone();
        let voice = default_conditioning();
        let transcriber = snapshot.transcribers[0].id.clone();
        let cancel = CancellationToken::new();
        speech
            .ensure_comfy(&comfy_root.to_string_lossy(), &cancel)
            .await
            .unwrap();

        let mut generated = Vec::with_capacity(passage_audits.len());
        for (index, passage) in passage_audits.iter().enumerate() {
            let passage_id = passage
                .get("passageId")
                .and_then(Value::as_str)
                .expect("passageId must be a string");
            let text = passage
                .get("cleanedText")
                .and_then(Value::as_str)
                .expect("cleanedText must be a string");
            let synthesis = SpeechSynthesisRequest {
                job_id: format!("aethelgard-tts-{index}"),
                source_kind: "chat".into(),
                source_id: "aethelgard-highlight-live".into(),
                passage_id: passage_id.into(),
                text: text.into(),
                model_id: voice_model.clone(),
                voice_profile_id: voice.profile_id.clone(),
            };
            let clip = speech
                .synthesize(
                    &comfy_root.to_string_lossy(),
                    &synthesis,
                    &voice,
                    &cancel,
                    None,
                )
                .await
                .unwrap();
            generated.push((synthesis, clip));
        }

        let mut passages = Vec::with_capacity(generated.len());
        for (index, (synthesis, clip)) in generated.into_iter().enumerate() {
            let alignment = SpeechAlignmentRequest {
                job_id: format!("aethelgard-align-{index}"),
                source_kind: synthesis.source_kind.clone(),
                source_id: synthesis.source_id.clone(),
                passage_id: synthesis.passage_id.clone(),
                text: synthesis.text.clone(),
                relative_path: clip.relative_path.clone(),
                voice_model_id: voice_model.clone(),
                voice_profile_id: voice.profile_id.clone(),
                alignment_model_id: transcriber.clone(),
            };
            let aligned = speech
                .align(
                    &comfy_root.to_string_lossy(),
                    &alignment,
                    &voice,
                    &cancel,
                    None,
                )
                .await
                .unwrap();
            assert!(!aligned.words.is_empty());
            assert!(aligned.words.windows(2).all(|pair| {
                pair[0].start.is_finite()
                    && pair[0].end.is_finite()
                    && pair[0].start <= pair[0].end
                    && pair[0].end <= pair[1].end
            }));
            let duration = aligned
                .words
                .last()
                .map(|word| word.end)
                .expect("aligned passage must have a duration");
            passages.push(json!({
                "passageId": synthesis.passage_id,
                "text": synthesis.text,
                "durationSec": duration,
                "words": aligned.words,
                "segments": aligned.segments,
                "audioRelativePath": aligned.relative_path,
                "voiceModelId": voice_model,
                "voiceProfileId": voice.profile_id,
                "alignmentModelId": transcriber,
            }));
        }

        let output = json!({
            "fixture": input_path,
            "comfyRoot": comfy_root,
            "passages": passages,
            "generatedAt": chrono::Utc::now().to_rfc3339(),
        });
        write_json_atomic(
            &output_root.join("aethelgard-live-highlight-timings.json"),
            &output,
        )
        .unwrap();
        speech.release_model_memory().await;
        speech.stop_comfy().await;
    }
}
