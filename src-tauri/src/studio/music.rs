//! Durable offline music production built on the user's loopback-only ComfyUI.
//!
//! The music workspace owns producer-authored structure and immutable generated takes. MiniMax
//! Music 3 produces a stereo master, not stems, so this module deliberately does not invent
//! per-instrument media. Optional MuScriptor transcription is an explicit, fixed-argument
//! audio-to-MIDI export using a producer-supplied executable and locally accepted checkpoint.

use super::music_midi::{
    normalize_midi_document, parse_midi_document, validate_midi_document, write_bytes_recoverable,
    write_midi_document, MusicMidiDocument,
};
use super::{
    comfy_execution_error, find_output_media, hash_file, truncate, MovieStudio, StudioError,
    MUSIC_COMFY_BASE,
};
use crate::local_speech::{SpeechFileTranscription, SpeechTiming};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const MUSIC_SCHEMA_VERSION: u32 = 1;
const MUSCRIPTOR_MODEL_BYTES: u64 = 5_465_642_136;
const MAX_MUSIC_TEXT_BYTES: usize = 64 * 1024;
const MAX_SECTIONS: usize = 64;
const MAX_TAKES: usize = 128;
const MAX_LYRIC_SEGMENTS: usize = 4_096;
const MAX_LYRIC_WORDS: usize = 65_536;
const MAX_LYRIC_TEXT_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_LYRIC_THEME: &str = "sketchbook";
const SIGNAL_BLOOM_LYRIC_THEME: &str = "signal-bloom";
const MUSIC_RENDER_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const MUSIC_DIT_INT8: &str = "minimax_music3_dit_int8_convrot.safetensors";
const MUSIC_DIT_FP16: &str = "minimax_music3_dit_fp16.safetensors";
const MUSIC_TEXT_ENCODER: &str = "minimax_music3_text_encoder_pruned_int8_convrot.safetensors";
const MUSIC_VAE: &str = "minimax_music3_dav.safetensors";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MusicSettings {
    pub max_duration_seconds: f64,
    pub steps: u32,
    pub cfg_scale: f64,
    pub top_k: u32,
    pub seed: u64,
    pub tiled_decode: bool,
    pub model_variant: String,
    pub comfy_root: String,
}

impl Default for MusicSettings {
    fn default() -> Self {
        Self {
            max_duration_seconds: 120.0,
            steps: 30,
            cfg_scale: 1.7,
            top_k: 50,
            seed: 0,
            tiled_decode: true,
            model_variant: "auto".into(),
            comfy_root: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MusicSection {
    pub id: String,
    pub tag: String,
    pub name: String,
    pub bars: u32,
    pub lyrics: String,
    pub direction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiSettings {
    pub executable_path: String,
    pub model_path: String,
    pub instruments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicTake {
    pub id: String,
    pub created_at: String,
    pub status: String,
    pub detail: String,
    pub error: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub duration_seconds: f64,
    pub seed: u64,
    pub resolved_model: String,
    pub caption: String,
    pub lyrics: String,
    pub prompt_id: String,
    pub exact_graph: Value,
    pub midi_path: String,
    pub midi_receipt_path: String,
    #[serde(default)]
    pub midi_source_path: String,
    #[serde(default)]
    pub midi_document_path: String,
    #[serde(default)]
    pub midi_revision: u32,
    #[serde(default)]
    pub lyrics_document_path: String,
    #[serde(default)]
    pub lyrics_receipt_path: String,
    #[serde(default)]
    pub lyrics_revision: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicProject {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub idea: String,
    pub caption: String,
    pub instrumental: bool,
    pub sections: Vec<MusicSection>,
    pub settings: MusicSettings,
    pub midi: MusicMidiSettings,
    pub takes: Vec<MusicTake>,
    pub active_take_id: String,
    pub status: String,
    pub phase: String,
    pub detail: String,
    pub error: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    pub updated_at: String,
    pub take_count: usize,
    pub active_take_path: String,
}

impl From<&MusicProject> for MusicSummary {
    fn from(project: &MusicProject) -> Self {
        Self {
            id: project.id.clone(),
            title: project.title.clone(),
            status: project.status.clone(),
            updated_at: project.updated_at.clone(),
            take_count: project.takes.len(),
            active_take_path: project
                .takes
                .iter()
                .find(|take| take.id == project.active_take_id)
                .map(|take| take.path.clone())
                .unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMusicProjectRequest {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub idea: String,
    #[serde(default)]
    pub comfy_root: String,
    #[serde(default)]
    pub muscriptor_executable_path: String,
    #[serde(default)]
    pub muscriptor_model_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiRequest {
    pub project_id: String,
    pub take_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveMusicMidiDocumentRequest {
    pub project_id: String,
    pub take_id: String,
    pub document: MusicMidiDocument,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiSaveResult {
    pub project: MusicProject,
    pub document: MusicMidiDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MusicLyricWord {
    pub value: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MusicLyricSegment {
    pub id: String,
    pub start: f64,
    pub end: f64,
    pub primary: String,
    #[serde(default)]
    pub translation: String,
    #[serde(default)]
    pub words: Vec<MusicLyricWord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MusicLyricsDocument {
    pub schema_version: u32,
    pub take_id: String,
    pub source_sha256: String,
    pub revision: u32,
    pub language: String,
    pub source: String,
    pub transcript: String,
    pub theme: String,
    pub show_translation: bool,
    pub created_at: String,
    pub updated_at: String,
    pub segments: Vec<MusicLyricSegment>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicLyricsRequest {
    pub project_id: String,
    pub take_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeMusicLyricsRequest {
    pub project_id: String,
    pub take_id: String,
    pub job_id: String,
    pub model_id: String,
    #[serde(default = "default_lyrics_language")]
    pub language: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairMusicLyricsRangeRequest {
    pub project_id: String,
    pub take_id: String,
    pub job_id: String,
    pub model_id: String,
    #[serde(default = "default_lyrics_language")]
    pub language: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    #[serde(default)]
    pub prompt: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftLyricsFromAudioRangeRequest {
    pub project_id: String,
    pub take_id: String,
    pub model_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftLyricsFromAudioRangeResult {
    pub transcription: String,
    pub model_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateMusicLyricsRequest {
    pub project_id: String,
    pub take_id: String,
    pub model_id: String,
    pub target_language: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateMusicLyricsResult {
    pub translations: Vec<String>,
    pub model_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveMusicLyricsDocumentRequest {
    pub project_id: String,
    pub take_id: String,
    pub document: MusicLyricsDocument,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicLyricsSaveResult {
    pub project: MusicProject,
    pub document: MusicLyricsDocument,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicGenerationEvent {
    pub project_id: String,
    pub take_id: String,
    pub kind: String,
    pub phase: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
    pub at: String,
}

impl MusicGenerationEvent {
    fn new(
        project_id: &str,
        take_id: &str,
        kind: &str,
        phase: &str,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            take_id: take_id.into(),
            kind: kind.into(),
            phase: phase.into(),
            detail: detail.into(),
            step: None,
            total: None,
            percent: None,
            eta_seconds: None,
            at: Utc::now().to_rfc3339(),
        }
    }
}

#[derive(Clone)]
pub struct MusicStudio {
    root: PathBuf,
    http: Client,
}

impl MusicStudio {
    pub fn new(library_root: &Path) -> Result<Self, StudioError> {
        let root = library_root.join("music");
        fs::create_dir_all(&root)?;
        let studio = Self {
            root,
            http: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3_600))
                .build()?,
        };
        studio.recover_interrupted()?;
        Ok(studio)
    }

    /// Release every model retained by the dedicated Music 3 ComfyUI service. The server stays
    /// warm on port 8189, while its CUDA allocations are returned before another local model runs.
    pub async fn release_comfy_memory(&self) {
        let _ = self
            .http
            .post(format!("{MUSIC_COMFY_BASE}/free"))
            .json(&json!({"unload_models":true,"free_memory":true}))
            .timeout(Duration::from_secs(30))
            .send()
            .await;
    }

    pub fn list(&self) -> Result<Vec<MusicSummary>, StudioError> {
        let mut projects = Vec::new();
        for entry in fs::read_dir(&self.root)?.take(2_000) {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path().join("project.json");
            if let Ok(project) = read_project(&path) {
                projects.push(MusicSummary::from(&project));
            }
        }
        projects.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(projects)
    }

    pub fn get(&self, id: &str) -> Result<MusicProject, StudioError> {
        validate_music_id(id)?;
        read_project(&self.project_dir(id).join("project.json"))
    }

    pub fn create(&self, request: CreateMusicProjectRequest) -> Result<MusicProject, StudioError> {
        if request.idea.len() > MAX_MUSIC_TEXT_BYTES {
            return Err(StudioError::Invalid(
                "song idea must not exceed 64 KiB".into(),
            ));
        }
        let title = if request.title.trim().is_empty() {
            "Untitled song"
        } else {
            request.title.trim()
        };
        validate_title(title)?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let settings = MusicSettings {
            comfy_root: request.comfy_root,
            ..MusicSettings::default()
        };
        let project = MusicProject {
            schema_version: MUSIC_SCHEMA_VERSION,
            id: id.clone(),
            title: title.into(),
            idea: request.idea.trim().into(),
            caption: request.idea.trim().into(),
            instrumental: false,
            sections: default_sections(),
            settings,
            midi: MusicMidiSettings {
                executable_path: request.muscriptor_executable_path,
                model_path: request.muscriptor_model_path,
                instruments: String::new(),
            },
            takes: Vec::new(),
            active_take_id: String::new(),
            status: "draft".into(),
            phase: "arranging".into(),
            detail:
                "Shape the description, lyrics, and section order, then create a preserved take."
                    .into(),
            error: String::new(),
            created_at: now.clone(),
            updated_at: now,
        };
        let folder = self.project_dir(&id);
        fs::create_dir_all(folder.join("takes"))?;
        fs::create_dir_all(folder.join("receipts"))?;
        fs::create_dir_all(folder.join("midi"))?;
        fs::create_dir_all(folder.join("logs"))?;
        self.persist(&project)?;
        Ok(project)
    }

    /// Save only producer-editable state. Generated paths, hashes, receipts, and take history are
    /// durable backend truth and cannot be replaced by a frontend payload.
    pub fn save_editable(&self, edited: MusicProject) -> Result<MusicProject, StudioError> {
        let mut stored = self.get(&edited.id)?;
        if stored.status == "generating" {
            return Err(StudioError::Invalid(
                "song structure and generation settings are locked while a take is rendering"
                    .into(),
            ));
        }
        validate_editable(&edited)?;
        stored.title = edited.title.trim().into();
        stored.idea = edited.idea;
        stored.caption = edited.caption;
        stored.instrumental = edited.instrumental;
        stored.sections = edited.sections;
        stored.settings = edited.settings;
        stored.midi = edited.midi;
        if edited.active_take_id.is_empty()
            || stored
                .takes
                .iter()
                .any(|take| take.id == edited.active_take_id && take.status == "complete")
        {
            stored.active_take_id = edited.active_take_id;
        }
        stored.updated_at = Utc::now().to_rfc3339();
        stored.detail =
            "Producer changes are saved. Existing generated takes remain immutable.".into();
        stored.error.clear();
        self.persist(&stored)?;
        Ok(stored)
    }

    pub fn begin_generation(
        &self,
        id: &str,
        app: Option<&AppHandle>,
    ) -> Result<(MusicProject, String), StudioError> {
        let mut project = self.get(id)?;
        validate_render_ready(&project)?;
        if project.takes.len() >= MAX_TAKES {
            return Err(StudioError::Invalid(
                "this song already has 128 preserved takes; start a new project before generating more".into(),
            ));
        }
        let take_id = uuid::Uuid::new_v4().to_string();
        let seed = resolved_seed(project.settings.seed);
        project.takes.push(MusicTake {
            id: take_id.clone(),
            created_at: Utc::now().to_rfc3339(),
            status: "queued".into(),
            detail: "Queued for the local music renderer.".into(),
            error: String::new(),
            path: String::new(),
            bytes: 0,
            sha256: String::new(),
            duration_seconds: 0.0,
            seed,
            resolved_model: String::new(),
            caption: render_caption(&project),
            lyrics: render_lyrics(&project),
            prompt_id: String::new(),
            exact_graph: Value::Null,
            midi_path: String::new(),
            midi_receipt_path: String::new(),
            midi_source_path: String::new(),
            midi_document_path: String::new(),
            midi_revision: 0,
            lyrics_document_path: String::new(),
            lyrics_receipt_path: String::new(),
            lyrics_revision: 0,
        });
        project.active_take_id = take_id.clone();
        project.status = "generating".into();
        project.phase = "queued".into();
        project.detail =
            "The arrangement is frozen for this take; prior takes remain untouched.".into();
        project.error.clear();
        self.persist_emit(&mut project, app)?;
        emit_music(
            app,
            MusicGenerationEvent::new(
                id,
                &take_id,
                "queued",
                "queued",
                "Take queued. Kestrel is releasing the text model before loading music weights.",
            ),
        );
        Ok((project, take_id))
    }

    pub async fn render(
        &self,
        project_id: &str,
        take_id: &str,
        shared_renderer: &MovieStudio,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<MusicProject, StudioError> {
        let mut project = self.get(project_id)?;
        let take_index = project
            .takes
            .iter()
            .position(|take| take.id == take_id)
            .ok_or_else(|| StudioError::Invalid("music take no longer exists".into()))?;
        let comfy_root = PathBuf::from(project.settings.comfy_root.trim());
        if !comfy_root.is_absolute() {
            return Err(StudioError::Invalid(
                "choose an absolute ComfyUI folder in Setup before generating music".into(),
            ));
        }
        project.phase = "starting-renderer".into();
        project.detail = "Starting or attaching to the private loopback ComfyUI renderer.".into();
        project.takes[take_index].status = "starting".into();
        project.takes[take_index].detail = project.detail.clone();
        self.persist_emit(&mut project, app)?;
        emit_music(
            app,
            MusicGenerationEvent::new(
                project_id,
                take_id,
                "progress",
                "starting-renderer",
                "Preparing local ComfyUI and checking the native MiniMax Music 3 nodes.",
            ),
        );
        self.release_comfy_memory().await;
        shared_renderer
            .ensure_music_comfy_process(
                project.settings.comfy_root.trim(),
                &self.project_dir(project_id).join("logs"),
                Some(cancel),
            )
            .await?;
        verify_music_nodes(&self.http, project.settings.tiled_decode).await?;
        let model_file = resolve_music_model(&comfy_root, &project.settings.model_variant)?;
        verify_music_assets(&comfy_root, &model_file)?;
        let prefix = format!("kestrel_music/{project_id}/take_{}", take_index + 1);
        let caption = project.takes[take_index].caption.clone();
        let lyrics = project.takes[take_index].lyrics.clone();
        let graph = minimax_music_graph(
            &project.settings,
            &caption,
            &lyrics,
            project.takes[take_index].seed,
            &model_file,
            &prefix,
        );
        project.takes[take_index]
            .resolved_model
            .clone_from(&model_file);
        project.takes[take_index].exact_graph = graph.clone();
        project.takes[take_index].status = "generating".into();
        project.takes[take_index].detail =
            "MiniMax Music 3 is composing the preserved stereo master.".into();
        project.phase = "generating".into();
        project.detail = project.takes[take_index].detail.clone();
        self.persist_emit(&mut project, app)?;
        write_json_recoverable(
            &self
                .project_dir(project_id)
                .join("receipts")
                .join(format!("{take_id}.graph.json")),
            &graph,
        )?;

        let client_id = format!("kestrel-music-{}", uuid::Uuid::new_v4().simple());
        let progress = MusicProgressSession::connect(app, &client_id, project_id, take_id).await;
        let response = self
            .http
            .post(format!("{MUSIC_COMFY_BASE}/prompt"))
            .json(&json!({"prompt":graph,"client_id":client_id}))
            .send()
            .await?;
        let status = response.status();
        let value: Value = response.json().await?;
        if !status.is_success() {
            return Err(StudioError::Render(format!(
                "ComfyUI rejected the MiniMax Music 3 workflow: {}",
                truncate(&value.to_string(), 900)
            )));
        }
        let prompt_id = value
            .get("prompt_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StudioError::Render(format!("ComfyUI returned no music prompt ID: {value}"))
            })?
            .to_string();
        project.takes[take_index].prompt_id.clone_from(&prompt_id);
        self.persist(&project)?;

        let deadline = tokio::time::Instant::now() + MUSIC_RENDER_TIMEOUT;
        let (source_name, source_subfolder) = loop {
            if cancel.is_cancelled() {
                let _ = self
                    .http
                    .post(format!("{MUSIC_COMFY_BASE}/interrupt"))
                    .send()
                    .await;
                return Err(StudioError::Cancelled);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(StudioError::Render(
                    "ComfyUI did not finish the music take within 24 hours. The project, graph receipt, and prior takes remain safe.".into(),
                ));
            }
            let history: Value = self
                .http
                .get(format!("{MUSIC_COMFY_BASE}/history/{prompt_id}"))
                .send()
                .await?
                .json()
                .await?;
            if let Some(entry) = history.get(&prompt_id) {
                if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
                    let detail = comfy_execution_error(entry).unwrap_or_else(|| {
                        format!("execution failed: {}", truncate(&entry.to_string(), 1_200))
                    });
                    return Err(StudioError::Render(format!("ComfyUI {detail}")));
                }
                if entry.pointer("/status/completed").and_then(Value::as_bool) == Some(true) {
                    let media = find_output_media(entry, "audio").ok_or_else(|| {
                        StudioError::Render(
                            "the completed music graph exposed no saved audio output".into(),
                        )
                    })?;
                    break media;
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        };
        if let Some(progress) = &progress {
            progress.finish();
        }
        let source = comfy_root
            .join("output")
            .join(&source_subfolder)
            .join(&source_name);
        let extension = Path::new(&source_name)
            .extension()
            .and_then(|value| value.to_str())
            .filter(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "flac" | "wav" | "mp3" | "ogg"
                )
            })
            .unwrap_or("flac");
        let target = self
            .project_dir(project_id)
            .join("takes")
            .join(format!("take-{:03}-{take_id}.{extension}", take_index + 1));
        tokio::fs::copy(&source, &target).await.map_err(|error| {
            StudioError::Render(format!(
                "could not preserve the generated master from {}: {error}",
                source.display()
            ))
        })?;
        let hash_target = target.clone();
        let duration_target = target.clone();
        let (hash_result, duration_result) = tokio::join!(
            tokio::task::spawn_blocking(move || hash_file(&hash_target)),
            tokio::task::spawn_blocking(move || probe_audio_duration(&duration_target)),
        );
        let (bytes, sha256) = hash_result.map_err(|error| {
            StudioError::Render(format!("music checksum task failed: {error}"))
        })??;
        let duration = duration_result
            .map_err(|error| StudioError::Render(format!("audio probe task failed: {error}")))?
            .unwrap_or(project.settings.max_duration_seconds);
        let mut project = self.get(project_id)?;
        let take = project
            .takes
            .iter_mut()
            .find(|take| take.id == take_id)
            .ok_or_else(|| {
                StudioError::Invalid("music take disappeared during preservation".into())
            })?;
        take.status = "complete".into();
        take.detail = "Full-quality local master preserved. Earlier takes remain unchanged.".into();
        take.path = target.to_string_lossy().into_owned();
        take.bytes = bytes;
        take.sha256 = sha256;
        take.duration_seconds = duration;
        take.error.clear();
        project.active_take_id = take_id.into();
        project.status = "ready".into();
        project.phase = "take-ready".into();
        project.detail = "A new immutable stereo take is ready for review.".into();
        project.error.clear();
        self.persist_emit(&mut project, app)?;
        emit_music(
            app,
            MusicGenerationEvent::new(
                project_id,
                take_id,
                "complete",
                "take-ready",
                "Full-quality stereo take preserved in the private project.",
            ),
        );
        self.release_comfy_memory().await;
        Ok(project)
    }

    pub fn fail_generation(
        &self,
        project_id: &str,
        take_id: &str,
        error: String,
        cancelled: bool,
        app: Option<&AppHandle>,
    ) {
        let Ok(mut project) = self.get(project_id) else {
            return;
        };
        if let Some(take) = project.takes.iter_mut().find(|take| take.id == take_id) {
            take.status = if cancelled { "cancelled" } else { "failed" }.into();
            take.detail = if cancelled {
                "Generation stopped. No completed take was replaced."
            } else {
                "Generation stopped with a recoverable error."
            }
            .into();
            take.error.clone_from(&error);
        }
        project.status = if project.takes.iter().any(|take| take.status == "complete") {
            "ready"
        } else if cancelled {
            "draft"
        } else {
            "failed"
        }
        .into();
        project.phase = if cancelled { "cancelled" } else { "failed" }.into();
        project.detail = if cancelled {
            "Take generation stopped; structure and existing takes are safe."
        } else {
            "The failed take is retained with its exact graph and error. Existing takes are safe."
        }
        .into();
        project.error = if cancelled {
            String::new()
        } else {
            error.clone()
        };
        let _ = self.persist_emit(&mut project, app);
        emit_music(
            app,
            MusicGenerationEvent::new(
                project_id,
                take_id,
                if cancelled { "cancelled" } else { "error" },
                &project.phase,
                if cancelled { project.detail } else { error },
            ),
        );
    }

    pub fn lyrics_audio_source(
        &self,
        request: &MusicLyricsRequest,
    ) -> Result<(PathBuf, String, String), StudioError> {
        let project = self.get(&request.project_id)?;
        let take = project
            .takes
            .iter()
            .find(|take| take.id == request.take_id && take.status == "complete")
            .ok_or_else(|| {
                StudioError::Invalid("choose a completed music take for lyric syncing".into())
            })?;
        let root = fs::canonicalize(self.project_dir(&project.id).join("takes"))?;
        let source = fs::canonicalize(Path::new(&take.path)).map_err(|_| {
            StudioError::Invalid("the selected preserved master is missing from disk".into())
        })?;
        if !source.starts_with(root) || !source.is_file() {
            return Err(StudioError::Invalid(
                "the lyric-sync audio is outside this private music project".into(),
            ));
        }
        let (_, actual_sha256) = hash_file(&source)?;
        if actual_sha256 != take.sha256 {
            return Err(StudioError::Invalid(
                "the preserved master changed after generation; Kestrel will not attach lyric timings to altered audio".into(),
            ));
        }
        Ok((source, take.lyrics.clone(), take.sha256.clone()))
    }

    pub fn create_lyrics_draft(
        &self,
        request: MusicLyricsRequest,
    ) -> Result<MusicLyricsSaveResult, StudioError> {
        let project = self.get(&request.project_id)?;
        let take = project
            .takes
            .iter()
            .find(|take| take.id == request.take_id && take.status == "complete")
            .ok_or_else(|| {
                StudioError::Invalid("choose a completed music take for the lyric stage".into())
            })?;
        if !take.lyrics_document_path.trim().is_empty() {
            return self.load_lyrics_document(request);
        }
        let take_id = take.id.clone();
        let take_sha256 = take.sha256.clone();
        let take_lyrics = take.lyrics.clone();
        let duration_seconds = take.duration_seconds;
        let now = Utc::now().to_rfc3339();
        let document = normalize_lyrics_document(
            MusicLyricsDocument {
                schema_version: 1,
                take_id,
                source_sha256: take_sha256.clone(),
                revision: 0,
                language: "auto".into(),
                source: "producer-timing-draft".into(),
                transcript: take_lyrics.clone(),
                theme: DEFAULT_LYRIC_THEME.into(),
                show_translation: true,
                created_at: now.clone(),
                updated_at: now,
                segments: estimated_lyric_segments(&take_lyrics, duration_seconds),
            },
            duration_seconds,
        )?;
        self.persist_new_lyrics_session(
            project,
            &request.take_id,
            document,
            json!({
                "schemaVersion": 1,
                "createdAt": Utc::now().to_rfc3339(),
                "takeId": request.take_id,
                "sourceSha256": take_sha256,
                "source": "producer-timing-draft",
                "detail": "Initial cue positions were estimated from the immutable generated lyrics and can be edited or replaced by local Whisper sync."
            }),
        )
    }

    pub fn persist_lyrics_transcription(
        &self,
        request: &TranscribeMusicLyricsRequest,
        transcription: SpeechFileTranscription,
    ) -> Result<MusicLyricsSaveResult, StudioError> {
        let project = self.get(&request.project_id)?;
        let take = project
            .takes
            .iter()
            .find(|take| take.id == request.take_id && take.status == "complete")
            .ok_or_else(|| {
                StudioError::Invalid("the lyric-sync music take no longer exists".into())
            })?;
        let take_id = take.id.clone();
        let take_sha256 = take.sha256.clone();
        let duration_seconds = take.duration_seconds;
        let (theme, show_translation) = if take.lyrics_document_path.trim().is_empty() {
            (DEFAULT_LYRIC_THEME.to_string(), true)
        } else {
            let previous = self.load_lyrics_document(MusicLyricsRequest {
                project_id: request.project_id.clone(),
                take_id: request.take_id.clone(),
            })?;
            (previous.document.theme, previous.document.show_translation)
        };
        let segment_count = transcription.segments.len();
        let word_count = transcription.words.len();
        let transcription_strategy = transcription.strategy.clone();
        let context_copies = transcription.context_copies;
        let context_seam_seconds = transcription.context_seam_seconds;
        let selected_context_copy = transcription.selected_context_copy;
        let first_context_score = transcription.first_context_score;
        let second_context_score = transcription.second_context_score;
        let now = Utc::now().to_rfc3339();
        let document = normalize_lyrics_document(
            MusicLyricsDocument {
                schema_version: 1,
                take_id,
                source_sha256: take_sha256.clone(),
                revision: 0,
                language: request.language.clone(),
                source: request.model_id.clone(),
                transcript: transcription.text.clone(),
                theme: theme.clone(),
                show_translation,
                created_at: now.clone(),
                updated_at: now,
                segments: lyric_segments_from_speech(transcription.segments, &transcription.words),
            },
            duration_seconds,
        )?;
        self.persist_new_lyrics_session(
            project,
            &request.take_id,
            document,
            json!({
                "schemaVersion": 1,
                "createdAt": Utc::now().to_rfc3339(),
                "takeId": request.take_id,
                "sourceSha256": take_sha256,
                "tool": "Kestrel Whisper",
                "modelId": request.model_id,
                "language": request.language,
                "transcript": transcription.text,
                "segmentCount": segment_count,
                "wordCount": word_count,
                "transcriptionStrategy": transcription_strategy,
                "contextCopies": context_copies,
                "contextSeamSeconds": context_seam_seconds,
                "selectedContextCopy": selected_context_copy,
                "firstContextScore": first_context_score,
                "secondContextScore": second_context_score,
                "theme": theme,
                "network": "disabled"
            }),
        )
    }

    pub fn persist_lyrics_range_repair(
        &self,
        request: &RepairMusicLyricsRangeRequest,
        transcription: SpeechFileTranscription,
    ) -> Result<MusicLyricsSaveResult, StudioError> {
        let project = self.get(&request.project_id)?;
        let take = project
            .takes
            .iter()
            .find(|take| take.id == request.take_id && take.status == "complete")
            .ok_or_else(|| {
                StudioError::Invalid("the lyric-sync music take no longer exists".into())
            })?;
        let take_id = take.id.clone();
        let take_sha256 = take.sha256.clone();
        let duration_seconds = take.duration_seconds;
        let previous = if !take.lyrics_document_path.trim().is_empty() {
            self.load_lyrics_document(MusicLyricsRequest {
                project_id: request.project_id.clone(),
                take_id: request.take_id.clone(),
            })?
        } else {
            MusicLyricsSaveResult {
                project: project.clone(),
                document: MusicLyricsDocument {
                    schema_version: 1,
                    take_id: take_id.clone(),
                    source_sha256: take_sha256.clone(),
                    revision: 0,
                    language: request.language.clone(),
                    source: request.model_id.clone(),
                    transcript: String::new(),
                    theme: DEFAULT_LYRIC_THEME.to_string(),
                    show_translation: true,
                    created_at: Utc::now().to_rfc3339(),
                    updated_at: Utc::now().to_rfc3339(),
                    segments: Vec::new(),
                },
            }
        };

        let repaired_segments = lyric_segments_from_speech(transcription.segments, &transcription.words);
        let spliced_segments = splice_repaired_lyrics_segments(
            &previous.document.segments,
            repaired_segments,
            request.start_seconds,
            request.end_seconds,
        );

        let full_transcript = spliced_segments
            .iter()
            .map(|s| s.primary.as_str())
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        let now = Utc::now().to_rfc3339();
        let mut project = project;
        let take_index = project
            .takes
            .iter()
            .position(|t| t.id == request.take_id && t.status == "complete")
            .ok_or_else(|| {
                StudioError::Invalid("the lyric-sync music take no longer exists".into())
            })?;

        let has_existing_session = !project.takes[take_index].lyrics_document_path.trim().is_empty();

        if has_existing_session {
            let previous_path = self.validated_lyrics_artifact(
                &project.id,
                &project.takes[take_index].lyrics_document_path,
            )?;
            let revisions = previous_path.parent().ok_or_else(|| {
                StudioError::Invalid("the lyric revision folder is unavailable".into())
            })?;
            let revision = previous.document.revision.checked_add(1).ok_or_else(|| {
                StudioError::Invalid("the lyric revision counter is exhausted".into())
            })?;
            let document = normalize_lyrics_document(
                MusicLyricsDocument {
                    schema_version: 1,
                    take_id: take_id.clone(),
                    source_sha256: take_sha256.clone(),
                    revision,
                    language: request.language.clone(),
                    source: format!("{}-range-repair", request.model_id),
                    transcript: full_transcript,
                    theme: previous.document.theme.clone(),
                    show_translation: previous.document.show_translation,
                    created_at: previous.document.created_at,
                    updated_at: now,
                    segments: spliced_segments,
                },
                duration_seconds,
            )?;
            let document_path = revisions.join(format!("{revision:03}.json"));
            let receipt_path = revisions.join(format!("{revision:03}.receipt.json"));
            write_json_recoverable(&document_path, &document)?;
            write_json_recoverable(
                &receipt_path,
                &json!({
                    "schemaVersion": 1,
                    "createdAt": document.updated_at,
                    "takeId": request.take_id,
                    "revision": revision,
                    "sourceSha256": take_sha256,
                    "tool": "Kestrel Whisper Range Repair",
                    "modelId": request.model_id,
                    "language": request.language,
                    "rangeStart": request.start_seconds,
                    "rangeEnd": request.end_seconds,
                    "prompt": request.prompt,
                    "theme": document.theme,
                    "network": "disabled"
                }),
            )?;
            project.takes[take_index].lyrics_document_path =
                document_path.to_string_lossy().into_owned();
            project.takes[take_index].lyrics_revision = revision;
            project.phase = "lyrics-ready".into();
            project.detail = format!(
                "Lyric revision {revision} saved after Whisper range repair ({:.1}s–{:.1}s). Earlier revisions and raw take audio remain untouched.",
                request.start_seconds, request.end_seconds
            );
            project.updated_at = Utc::now().to_rfc3339();
            self.persist(&project)?;
            Ok(MusicLyricsSaveResult { project, document })
        } else {
            let document = normalize_lyrics_document(
                MusicLyricsDocument {
                    schema_version: 1,
                    take_id: take_id.clone(),
                    source_sha256: take_sha256.clone(),
                    revision: 0,
                    language: request.language.clone(),
                    source: format!("{}-range-repair", request.model_id),
                    transcript: full_transcript,
                    theme: DEFAULT_LYRIC_THEME.to_string(),
                    show_translation: true,
                    created_at: now.clone(),
                    updated_at: now,
                    segments: spliced_segments,
                },
                duration_seconds,
            )?;
            self.persist_new_lyrics_session(
                project,
                &request.take_id,
                document,
                json!({
                    "schemaVersion": 1,
                    "createdAt": Utc::now().to_rfc3339(),
                    "takeId": request.take_id,
                    "sourceSha256": take_sha256,
                    "tool": "Kestrel Whisper Range Repair",
                    "modelId": request.model_id,
                    "language": request.language,
                    "rangeStart": request.start_seconds,
                    "rangeEnd": request.end_seconds,
                    "prompt": request.prompt,
                    "theme": DEFAULT_LYRIC_THEME,
                    "network": "disabled"
                }),
            )
        }
    }

    pub fn load_lyrics_document(
        &self,
        request: MusicLyricsRequest,
    ) -> Result<MusicLyricsSaveResult, StudioError> {
        let project = self.get(&request.project_id)?;
        let take = project
            .takes
            .iter()
            .find(|take| {
                take.id == request.take_id
                    && take.status == "complete"
                    && !take.lyrics_document_path.trim().is_empty()
            })
            .ok_or_else(|| {
                StudioError::Invalid(
                    "this take has no lyric document yet; prepare the lyric stage first".into(),
                )
            })?;
        let path = self.validated_lyrics_artifact(&project.id, &take.lyrics_document_path)?;
        let document = read_lyrics_document(&path)?;
        if document.take_id != request.take_id
            || document.source_sha256 != take.sha256
        {
            return Err(StudioError::Invalid(
                "the lyric document no longer matches its immutable music take; resync instead of overwriting provenance".into(),
            ));
        }
        let duration_seconds = take.duration_seconds;
        let take_lyrics_revision = take.lyrics_revision;
        if document.revision != take_lyrics_revision {
            let mut project = project;
            let take_index = project
                .takes
                .iter()
                .position(|t| t.id == request.take_id)
                .ok_or_else(|| StudioError::Invalid("the lyric take no longer exists".into()))?;
            project.takes[take_index].lyrics_revision = document.revision;
            project.updated_at = Utc::now().to_rfc3339();
            self.persist(&project)?;
            validate_lyrics_document(&document, duration_seconds)?;
            return Ok(MusicLyricsSaveResult { project, document });
        }
        validate_lyrics_document(&document, duration_seconds)?;
        Ok(MusicLyricsSaveResult { project, document })
    }

    pub fn save_lyrics_document(
        &self,
        request: SaveMusicLyricsDocumentRequest,
    ) -> Result<MusicLyricsSaveResult, StudioError> {
        let loaded = self.load_lyrics_document(MusicLyricsRequest {
            project_id: request.project_id.clone(),
            take_id: request.take_id.clone(),
        })?;
        let mut project = loaded.project;
        let take_index = project
            .takes
            .iter()
            .position(|take| take.id == request.take_id)
            .ok_or_else(|| StudioError::Invalid("the lyric take no longer exists".into()))?;
        if request.document.take_id != request.take_id
            || request.document.source_sha256 != loaded.document.source_sha256
            || request.document.revision != loaded.document.revision
            || request.document.created_at != loaded.document.created_at
        {
            return Err(StudioError::Invalid(
                "the lyric edit is stale or belongs to another take; reopen the lyric stage before saving".into(),
            ));
        }
        let revision = loaded.document.revision.checked_add(1).ok_or_else(|| {
            StudioError::Invalid("the lyric revision counter is exhausted".into())
        })?;
        let mut document = request.document;
        document.revision = revision;
        document.schema_version = loaded.document.schema_version;
        document.take_id = loaded.document.take_id;
        document.source_sha256 = loaded.document.source_sha256;
        document.language = loaded.document.language;
        document.source = loaded.document.source;
        document.transcript = loaded.document.transcript;
        document.created_at = loaded.document.created_at;
        document.updated_at = Utc::now().to_rfc3339();
        document = normalize_lyrics_document(document, project.takes[take_index].duration_seconds)?;
        let previous = self.validated_lyrics_artifact(
            &project.id,
            &project.takes[take_index].lyrics_document_path,
        )?;
        let revisions = previous.parent().ok_or_else(|| {
            StudioError::Invalid("the lyric revision folder is unavailable".into())
        })?;
        let document_path = revisions.join(format!("{revision:03}.json"));
        let receipt_path = revisions.join(format!("{revision:03}.receipt.json"));
        if document_path.exists() || receipt_path.exists() {
            return Err(StudioError::Invalid(
                "the next immutable lyric revision already exists; reopen the project before saving"
                    .into(),
            ));
        }
        write_json_recoverable(&document_path, &document)?;
        write_json_recoverable(
            &receipt_path,
            &json!({
                "schemaVersion": 1,
                "createdAt": document.updated_at,
                "takeId": request.take_id,
                "revision": revision,
                "sourceSha256": document.source_sha256,
                "segmentCount": document.segments.len(),
                "wordCount": document.segments.iter().map(|segment| segment.words.len()).sum::<usize>(),
                "theme": document.theme,
                "operation": "producer lyric timing edit"
            }),
        )?;
        project.takes[take_index].lyrics_document_path =
            document_path.to_string_lossy().into_owned();
        project.takes[take_index].lyrics_revision = revision;
        project.phase = "lyrics-ready".into();
        project.detail = format!(
            "Lyric revision {revision} is saved. The master, transcription receipt, and every earlier cue revision remain unchanged."
        );
        project.updated_at = Utc::now().to_rfc3339();
        self.persist(&project)?;
        Ok(MusicLyricsSaveResult { project, document })
    }

    fn persist_new_lyrics_session(
        &self,
        mut project: MusicProject,
        take_id: &str,
        document: MusicLyricsDocument,
        receipt: Value,
    ) -> Result<MusicLyricsSaveResult, StudioError> {
        let take_index = project
            .takes
            .iter()
            .position(|take| take.id == take_id && take.status == "complete")
            .ok_or_else(|| StudioError::Invalid("the lyric take no longer exists".into()))?;
        validate_lyrics_document(&document, project.takes[take_index].duration_seconds)?;
        let session = self
            .project_dir(&project.id)
            .join("lyrics")
            .join(take_id)
            .join(uuid::Uuid::new_v4().to_string());
        let revisions = session.join("revisions");
        fs::create_dir_all(&revisions)?;
        let document_path = revisions.join("000.json");
        let receipt_path = session.join("transcription.receipt.json");
        write_json_recoverable(&document_path, &document)?;
        write_json_recoverable(&receipt_path, &receipt)?;
        project.takes[take_index].lyrics_document_path =
            document_path.to_string_lossy().into_owned();
        project.takes[take_index].lyrics_receipt_path = receipt_path.to_string_lossy().into_owned();
        project.takes[take_index].lyrics_revision = 0;
        project.phase = "lyrics-ready".into();
        project.detail = if document.source == "producer-timing-draft" {
            "A durable lyric timing draft is ready. Refine cues by hand or sync the preserved take with local Whisper."
        } else {
            "Local Whisper synced durable lyric segments and word timings to the preserved take."
        }
        .into();
        project.updated_at = Utc::now().to_rfc3339();
        self.persist(&project)?;
        Ok(MusicLyricsSaveResult { project, document })
    }

    fn validated_lyrics_artifact(
        &self,
        project_id: &str,
        value: &str,
    ) -> Result<PathBuf, StudioError> {
        let root = fs::canonicalize(self.project_dir(project_id).join("lyrics"))?;
        let path = fs::canonicalize(Path::new(value)).map_err(|_| {
            StudioError::Invalid(format!("the lyric project file is unavailable at {value}"))
        })?;
        if !path.starts_with(root) || !path.is_file() {
            return Err(StudioError::Invalid(
                "the lyric path is outside this private music project".into(),
            ));
        }
        Ok(path)
    }

    pub async fn transcribe_midi(
        &self,
        request: MusicMidiRequest,
    ) -> Result<MusicProject, StudioError> {
        let mut project = self.get(&request.project_id)?;
        if repair_muscriptor_settings(&mut project.midi) {
            project.updated_at = Utc::now().to_rfc3339();
            self.persist(&project)?;
        }
        validate_muscriptor_settings(&project.midi)?;
        let take_index = project
            .takes
            .iter()
            .position(|take| take.id == request.take_id && take.status == "complete")
            .ok_or_else(|| {
                StudioError::Invalid("choose a completed music take for MIDI transcription".into())
            })?;
        let source = PathBuf::from(&project.takes[take_index].path);
        if !source.is_file() {
            return Err(StudioError::Invalid(
                "the selected preserved master is missing from disk".into(),
            ));
        }
        let session_id = uuid::Uuid::new_v4().to_string();
        let midi_session = self
            .project_dir(&project.id)
            .join("midi")
            .join(&request.take_id)
            .join(session_id);
        let source_output = midi_session.join("source.mid");
        let revisions = midi_session.join("revisions");
        fs::create_dir_all(&revisions)?;
        let mut command = tokio::process::Command::new(&project.midi.executable_path);
        if is_managed_muscriptor_uvx(
            Path::new(&project.midi.executable_path),
            Path::new(&project.midi.model_path),
        ) {
            let root = Path::new(&project.midi.executable_path)
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| {
                    StudioError::Invalid("the managed MuScriptor runner path is incomplete".into())
                })?;
            command
                .args([
                    "--offline",
                    "--python",
                    "3.12",
                    "--torch-backend",
                    "cu128",
                    "--from",
                    "muscriptor==0.3.0",
                    "muscriptor",
                    "transcribe",
                ])
                .env("UV_CACHE_DIR", root.join("cache"))
                .env("UV_PYTHON_INSTALL_DIR", root.join("python"))
                .env("UV_NO_PROGRESS", "1")
                .env("UV_LINK_MODE", "copy")
                .env("HF_HUB_OFFLINE", "1")
                .env("TRANSFORMERS_OFFLINE", "1");
        } else {
            command.arg("transcribe");
        }
        command.args(["--model", project.midi.model_path.as_str()]);
        if !project.midi.instruments.trim().is_empty() {
            command.args(["--instruments", project.midi.instruments.trim()]);
        }
        command
            .arg(&source)
            .arg("-o")
            .arg(&source_output)
            .env("PYTHONUTF8", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let result = tokio::time::timeout(Duration::from_secs(6 * 60 * 60), command.output())
            .await
            .map_err(|_| {
                StudioError::Render("MuScriptor did not finish within six hours".into())
            })??;
        if !result.status.success() || !source_output.is_file() {
            return Err(StudioError::Render(format!(
                "MuScriptor transcription failed: {}",
                truncate(&String::from_utf8_lossy(&result.stderr), 1_000)
            )));
        }
        let (_, source_midi_sha256) = hash_file(&source_output)?;
        let document =
            parse_midi_document(&source_output, &request.take_id, &source_midi_sha256, 0)?;
        let editable_output = revisions.join("000.mid");
        let document_path = revisions.join("000.json");
        write_midi_document(&editable_output, &document)?;
        write_json_recoverable(&document_path, &document)?;
        let receipt = json!({
            "schemaVersion": 1,
            "tool": "MuScriptor",
            "createdAt": Utc::now().to_rfc3339(),
            "takeId": request.take_id,
            "sourceSha256": project.takes[take_index].sha256,
            "sourceMidiSha256": source_midi_sha256,
            "modelPath": project.midi.model_path,
            "executablePath": project.midi.executable_path,
            "instruments": project.midi.instruments,
            "licenseNotice": "Producer supplied a locally accepted MuScriptor checkpoint. Kestrel does not grant commercial rights to CC-BY-NC weights.",
            "stdout": truncate(&String::from_utf8_lossy(&result.stdout), 8_000),
            "stderr": truncate(&String::from_utf8_lossy(&result.stderr), 8_000),
        });
        let receipt_path = midi_session.join("transcription.receipt.json");
        write_json_recoverable(&receipt_path, &receipt)?;
        project.takes[take_index].midi_path = editable_output.to_string_lossy().into_owned();
        project.takes[take_index].midi_receipt_path = receipt_path.to_string_lossy().into_owned();
        project.takes[take_index].midi_source_path = source_output.to_string_lossy().into_owned();
        project.takes[take_index].midi_document_path = document_path.to_string_lossy().into_owned();
        project.takes[take_index].midi_revision = 0;
        project.phase = "midi-ready".into();
        project.detail = "MuScriptor created an immutable source transcription and editable MIDI revision 0. The generated master is unchanged.".into();
        project.updated_at = Utc::now().to_rfc3339();
        self.persist(&project)?;
        Ok(project)
    }

    pub fn load_midi_document(
        &self,
        request: MusicMidiRequest,
    ) -> Result<MusicMidiSaveResult, StudioError> {
        let mut project = self.get(&request.project_id)?;
        let take_index = project
            .takes
            .iter()
            .position(|take| {
                take.id == request.take_id
                    && take.status == "complete"
                    && !take.midi_path.trim().is_empty()
            })
            .ok_or_else(|| StudioError::Invalid("choose a take with completed MIDI".into()))?;
        let current_path =
            self.validated_midi_artifact(&project.id, &project.takes[take_index].midi_path)?;
        let stored_document = PathBuf::from(&project.takes[take_index].midi_document_path);
        let stored_source = PathBuf::from(&project.takes[take_index].midi_source_path);
        let document = if stored_document.is_file() && stored_source.is_file() {
            let document = read_midi_document(&stored_document)?;
            let (_, source_sha256) = hash_file(&stored_source)?;
            if document.take_id != request.take_id
                || document.source_sha256 != source_sha256
                || document.revision != project.takes[take_index].midi_revision
            {
                return Err(StudioError::Invalid(
                    "the MIDI edit document no longer matches its immutable source; start a new transcription instead of overwriting provenance".into(),
                ));
            }
            document
        } else {
            let session = self
                .project_dir(&project.id)
                .join("midi")
                .join(&request.take_id)
                .join(uuid::Uuid::new_v4().to_string());
            let revisions = session.join("revisions");
            fs::create_dir_all(&revisions)?;
            let source_path = session.join("source.mid");
            write_bytes_recoverable(&source_path, &fs::read(&current_path)?)?;
            let (_, source_sha256) = hash_file(&source_path)?;
            let document = parse_midi_document(&current_path, &request.take_id, &source_sha256, 0)?;
            let revision_path = revisions.join("000.mid");
            let document_path = revisions.join("000.json");
            write_midi_document(&revision_path, &document)?;
            write_json_recoverable(&document_path, &document)?;
            project.takes[take_index].midi_source_path = source_path.to_string_lossy().into_owned();
            project.takes[take_index].midi_path = revision_path.to_string_lossy().into_owned();
            project.takes[take_index].midi_document_path =
                document_path.to_string_lossy().into_owned();
            project.takes[take_index].midi_revision = 0;
            project.updated_at = Utc::now().to_rfc3339();
            project.detail =
                "The original MIDI is preserved and revision 0 is ready in the piano roll.".into();
            self.persist(&project)?;
            document
        };
        Ok(MusicMidiSaveResult { project, document })
    }

    pub fn save_midi_document(
        &self,
        request: SaveMusicMidiDocumentRequest,
    ) -> Result<MusicMidiSaveResult, StudioError> {
        let loaded = self.load_midi_document(MusicMidiRequest {
            project_id: request.project_id.clone(),
            take_id: request.take_id.clone(),
        })?;
        let mut project = loaded.project;
        let take_index = project
            .takes
            .iter()
            .position(|take| take.id == request.take_id)
            .ok_or_else(|| StudioError::Invalid("the MIDI take no longer exists".into()))?;
        if request.document.take_id != request.take_id
            || request.document.source_sha256 != loaded.document.source_sha256
            || request.document.revision != loaded.document.revision
        {
            return Err(StudioError::Invalid(
                "the piano-roll edit is stale or belongs to another take; reopen MIDI before saving"
                    .into(),
            ));
        }
        let revision =
            loaded.document.revision.checked_add(1).ok_or_else(|| {
                StudioError::Invalid("the MIDI revision counter is exhausted".into())
            })?;
        let mut document = request.document;
        document.revision = revision;
        document = normalize_midi_document(document)?;
        validate_midi_document(&document)?;
        let previous_document = self
            .validated_midi_artifact(&project.id, &project.takes[take_index].midi_document_path)?;
        let revisions = previous_document.parent().ok_or_else(|| {
            StudioError::Invalid("the MIDI revision folder is unavailable".into())
        })?;
        let midi_path = revisions.join(format!("{revision:03}.mid"));
        let document_path = revisions.join(format!("{revision:03}.json"));
        let receipt_path = revisions.join(format!("{revision:03}.receipt.json"));
        if midi_path.exists() || document_path.exists() || receipt_path.exists() {
            return Err(StudioError::Invalid(
                "the next immutable MIDI revision already exists; reopen the project before saving"
                    .into(),
            ));
        }
        write_midi_document(&midi_path, &document)?;
        write_json_recoverable(&document_path, &document)?;
        let (bytes, sha256) = hash_file(&midi_path)?;
        write_json_recoverable(
            &receipt_path,
            &json!({
                "schemaVersion": 1,
                "createdAt": Utc::now().to_rfc3339(),
                "takeId": request.take_id,
                "revision": revision,
                "sourceMidiSha256": document.source_sha256,
                "exportSha256": sha256,
                "bytes": bytes,
                "trackCount": document.tracks.len(),
                "noteCount": document.tracks.iter().map(|track| track.notes.len()).sum::<usize>(),
                "mutedTrackCount": document.tracks.iter().filter(|track| track.muted).count(),
                "operation": "producer MIDI edit"
            }),
        )?;
        project.takes[take_index].midi_path = midi_path.to_string_lossy().into_owned();
        project.takes[take_index].midi_document_path = document_path.to_string_lossy().into_owned();
        project.takes[take_index].midi_revision = revision;
        project.phase = "midi-ready".into();
        project.detail = format!(
            "MIDI revision {revision} is saved. The MuScriptor source and every earlier revision remain unchanged."
        );
        project.updated_at = Utc::now().to_rfc3339();
        self.persist(&project)?;
        Ok(MusicMidiSaveResult { project, document })
    }

    pub fn midi_artifact(
        &self,
        request: &MusicMidiRequest,
    ) -> Result<(PathBuf, String), StudioError> {
        let project = self.get(&request.project_id)?;
        let take = project
            .takes
            .iter()
            .find(|take| take.id == request.take_id && !take.midi_path.trim().is_empty())
            .ok_or_else(|| StudioError::Invalid("choose a take with completed MIDI".into()))?;
        let path = self.validated_midi_artifact(&project.id, &take.midi_path)?;
        Ok((
            path,
            format!("{} - MIDI r{}", project.title, take.midi_revision),
        ))
    }

    pub fn export_midi_artifact(
        &self,
        request: &MusicMidiRequest,
        destination: &Path,
    ) -> Result<(), StudioError> {
        if !destination.is_absolute()
            || !destination
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| {
                    value.eq_ignore_ascii_case("mid") || value.eq_ignore_ascii_case("midi")
                })
        {
            return Err(StudioError::Invalid(
                "choose an absolute destination ending in .mid or .midi".into(),
            ));
        }
        let (source, _) = self.midi_artifact(request)?;
        write_bytes_recoverable(destination, &fs::read(source)?)
    }

    fn validated_midi_artifact(
        &self,
        project_id: &str,
        value: &str,
    ) -> Result<PathBuf, StudioError> {
        let root = fs::canonicalize(self.project_dir(project_id).join("midi"))?;
        let path = fs::canonicalize(Path::new(value)).map_err(|_| {
            StudioError::Invalid(format!("the MIDI project file is unavailable at {value}"))
        })?;
        if !path.starts_with(root) || !path.is_file() {
            return Err(StudioError::Invalid(
                "the MIDI path is outside this private music project".into(),
            ));
        }
        Ok(path)
    }

    pub fn project_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn reveal_path(&self, id: &str) -> Result<PathBuf, StudioError> {
        validate_music_id(id)?;
        let path = self.project_dir(id);
        if !path.is_dir() {
            return Err(StudioError::Invalid(
                "music project folder is missing".into(),
            ));
        }
        Ok(path)
    }

    fn persist(&self, project: &MusicProject) -> Result<(), StudioError> {
        write_json_recoverable(&self.project_dir(&project.id).join("project.json"), project)
    }

    fn persist_emit(
        &self,
        project: &mut MusicProject,
        app: Option<&AppHandle>,
    ) -> Result<(), StudioError> {
        project.schema_version = MUSIC_SCHEMA_VERSION;
        project.updated_at = Utc::now().to_rfc3339();
        self.persist(project)?;
        if let Some(app) = app {
            let _ = app.emit("music-project-updated", project.clone());
        }
        Ok(())
    }

    fn recover_interrupted(&self) -> Result<(), StudioError> {
        for entry in fs::read_dir(&self.root)?.take(2_000) {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path().join("project.json");
            let Ok(mut project) = read_project(&path) else {
                continue;
            };
            if project.status != "generating" {
                continue;
            }
            project.status = if project.takes.iter().any(|take| take.status == "complete") {
                "ready"
            } else {
                "draft"
            }
            .into();
            project.phase = "interrupted".into();
            project.detail = "Kestrel closed during generation. Existing takes and the exact in-progress graph are safe; create a new take when ready.".into();
            project.error.clear();
            if let Some(take) =
                project.takes.iter_mut().rev().find(|take| {
                    matches!(take.status.as_str(), "queued" | "starting" | "generating")
                })
            {
                take.status = "interrupted".into();
                take.detail =
                    "Generation was interrupted before a complete master was preserved.".into();
            }
            self.persist(&project)?;
        }
        Ok(())
    }
}

struct MusicProgressSession {
    app: AppHandle,
    project_id: String,
    take_id: String,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl MusicProgressSession {
    async fn connect(
        app: Option<&AppHandle>,
        client_id: &str,
        project_id: &str,
        take_id: &str,
    ) -> Option<Self> {
        let app = app?.clone();
        let url = format!("ws://127.0.0.1:8189/ws?clientId={client_id}");
        let (stream, _) = tokio_tungstenite::connect_async(&url).await.ok()?;
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task_app = app.clone();
        let task_project = project_id.to_string();
        let task_take = take_id.to_string();
        let task = tokio::spawn(async move {
            let (_, mut reader) = stream.split();
            let started = Instant::now();
            loop {
                tokio::select! {
                    _ = task_cancel.cancelled() => break,
                    message = reader.next() => {
                        let Some(Ok(message)) = message else { break; };
                        if !message.is_text() { continue; }
                        let Ok(text) = message.into_text() else { continue; };
                        if let Some(event) = parse_music_progress(&text, &task_project, &task_take, started) {
                            let _ = task_app.emit("music-generation", event);
                        }
                    }
                }
            }
        });
        Some(Self {
            app,
            project_id: project_id.into(),
            take_id: take_id.into(),
            cancel,
            task,
        })
    }

    fn finish(&self) {
        emit_music(
            Some(&self.app),
            MusicGenerationEvent::new(
                &self.project_id,
                &self.take_id,
                "progress",
                "preserving",
                "Sampling and full-quality decode finished. Preserving the immutable master.",
            ),
        );
        self.cancel.cancel();
    }
}

impl Drop for MusicProgressSession {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

fn parse_music_progress(
    text: &str,
    project_id: &str,
    take_id: &str,
    started: Instant,
) -> Option<MusicGenerationEvent> {
    let value: Value = serde_json::from_str(text).ok()?;
    let kind = value.get("type")?.as_str()?;
    let data = value.get("data")?;
    if kind == "progress" {
        let step = u32::try_from(data.get("value")?.as_u64()?).ok()?;
        let total = u32::try_from(data.get("max")?.as_u64()?).ok()?;
        if total == 0 || step > total || total > 100_000 {
            return None;
        }
        let percent = f64::from(step) * 100.0 / f64::from(total);
        let elapsed = started.elapsed().as_secs_f64();
        let eta_seconds = (step > 0)
            .then(|| ((elapsed / f64::from(step)) * f64::from(total - step)).round() as u64);
        let mut event = MusicGenerationEvent::new(
            project_id,
            take_id,
            "progress",
            "sampling",
            format!("Rendering locally · step {step} of {total}"),
        );
        event.step = Some(step);
        event.total = Some(total);
        event.percent = Some(percent);
        event.eta_seconds = eta_seconds;
        return Some(event);
    }
    if kind == "executing" {
        let node = data.get("node").and_then(Value::as_str).unwrap_or_default();
        let (phase, detail) = match node {
            "4" => (
                "composing",
                "Composing long-range structure, performance, and lyrics locally.",
            ),
            "7" => ("sampling", "Sampling the full acoustic master."),
            "8" => ("decoding", "Decoding full-quality stereo audio."),
            "10" => ("saving", "Writing the lossless ComfyUI output."),
            _ => return None,
        };
        return Some(MusicGenerationEvent::new(
            project_id, take_id, "progress", phase, detail,
        ));
    }
    None
}

fn emit_music(app: Option<&AppHandle>, event: MusicGenerationEvent) {
    if let Some(app) = app {
        let _ = app.emit("music-generation", event);
    }
}

fn default_sections() -> Vec<MusicSection> {
    [
        ("Intro", "Intro", 4),
        ("Verse", "Verse 1", 8),
        ("Chorus", "Chorus 1", 8),
        ("Verse", "Verse 2", 8),
        ("Chorus", "Chorus 2", 8),
        ("Bridge", "Bridge", 8),
        ("Chorus", "Final chorus", 8),
        ("Outro", "Outro", 4),
    ]
    .into_iter()
    .map(|(tag, name, bars)| MusicSection {
        id: uuid::Uuid::new_v4().to_string(),
        tag: tag.into(),
        name: name.into(),
        bars,
        lyrics: String::new(),
        direction: String::new(),
    })
    .collect()
}

fn default_lyrics_language() -> String {
    "auto".into()
}

fn estimated_lyric_segments(lyrics: &str, duration_seconds: f64) -> Vec<MusicLyricSegment> {
    let lines = lyrics
        .lines()
        .map(str::trim)
        .filter(|line| !(line.is_empty() || line.starts_with('[') && line.ends_with(']')))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if lines.is_empty() || !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return Vec::new();
    }
    let padding = (duration_seconds * 0.025).min(2.0);
    let usable = (duration_seconds - padding * 2.0).max(duration_seconds * 0.5);
    let weights = lines
        .iter()
        .map(|line| line.split_whitespace().count().max(1) as f64)
        .collect::<Vec<_>>();
    let total_weight = weights.iter().sum::<f64>().max(1.0);
    let mut cursor = padding;
    lines
        .into_iter()
        .zip(weights)
        .map(|(primary, weight)| {
            let start = cursor;
            let end = (start + usable * weight / total_weight).min(duration_seconds);
            cursor = end;
            MusicLyricSegment {
                id: uuid::Uuid::new_v4().to_string(),
                start,
                end,
                words: estimated_words(&primary, start, end),
                primary,
                translation: String::new(),
            }
        })
        .collect()
}

fn starts_with_capital(val: &str) -> bool {
    val.chars()
        .find(|c| c.is_alphabetic())
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
}

fn is_pronoun_i(val: &str) -> bool {
    let trimmed = val.trim_matches(|c: char| !c.is_alphabetic());
    trimmed == "I"
        || trimmed.starts_with("I'")
        || trimmed.starts_with("I’")
        || trimmed == "Im"
        || trimmed == "Ive"
        || trimmed == "Ill"
        || trimmed == "Id"
}

fn has_terminal_punctuation(val: &str) -> bool {
    let trimmed = val.trim();
    trimmed.ends_with("...")
        || trimmed.ends_with('…')
        || trimmed.ends_with('.')
        || trimmed.ends_with('!')
        || trimmed.ends_with('?')
        || trimmed.ends_with(';')
        || trimmed.ends_with(':')
        || trimmed.ends_with('—')
        || trimmed.ends_with("--")
}

fn has_clause_punctuation(val: &str) -> bool {
    has_terminal_punctuation(val) || val.trim().ends_with(',')
}

fn should_split_lyric_cue(
    current_cue_words: &[MusicLyricWord],
    next_word: &SpeechTiming,
) -> bool {
    if current_cue_words.is_empty() {
        return false;
    }
    let prev = &current_cue_words[current_cue_words.len() - 1];
    let gap = next_word.start - prev.end;
    let word_count = current_cue_words.len();
    let cue_duration = next_word.start - current_cue_words[0].start;

    // 1. Long vocal pause / breath boundary in singing:
    if gap >= 0.60 {
        return true;
    }

    // 2. Terminal punctuation on previous word (like "...", ".", "!", "?"):
    if has_terminal_punctuation(&prev.value)
        && (gap >= 0.15 || starts_with_capital(&next_word.value) || word_count >= 3)
    {
        return true;
    }

    // 3. Next word starts with a capital letter (Whisper sentence/line start):
    if starts_with_capital(&next_word.value) {
        if is_pronoun_i(&next_word.value) {
            // Standalone pronoun "I" / "I'm":
            // Split if preceded by a vocal breath pause, clause punctuation, or after an established phrase
            if gap >= 0.30
                || has_clause_punctuation(&prev.value)
                || word_count >= 6
                || cue_duration >= 4.5
            {
                return true;
            }
        } else {
            // General capitalized words ("Deep", "A", "The", "Hold", "Before", "Rain", etc.):
            // In Whisper transcription, capitalized words mark the start of a new line or sentence.
            if gap >= 0.10
                || has_clause_punctuation(&prev.value)
                || word_count >= 3
                || cue_duration >= 2.5
            {
                return true;
            }
        }
    }

    // 4. Safety maximum bounds to prevent runaway unpunctuated cues from overflowing the screen:
    if (word_count >= 10 || cue_duration >= 7.0)
        && (gap >= 0.15 || has_clause_punctuation(&prev.value) || word_count >= 14 || cue_duration >= 9.0)
    {
        return true;
    }

    false
}

fn lyric_segments_from_speech(
    segments: Vec<SpeechTiming>,
    words: &[SpeechTiming],
) -> Vec<MusicLyricSegment> {
    let valid_words = words
        .iter()
        .filter(|w| !w.value.trim().is_empty() && w.start.is_finite() && w.end.is_finite() && w.end >= w.start)
        .collect::<Vec<_>>();

    if !valid_words.is_empty() {
        let mut sorted_words = valid_words;
        sorted_words.sort_by(|left, right| left.start.total_cmp(&right.start));

        let mut output = Vec::new();
        let mut current_words: Vec<MusicLyricWord> = Vec::new();

        for word in sorted_words {
            if should_split_lyric_cue(&current_words, word) {
                let start = current_words.first().unwrap().start;
                let end = current_words.last().unwrap().end;
                let primary = current_words
                    .iter()
                    .map(|w| w.value.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                output.push(MusicLyricSegment {
                    id: uuid::Uuid::new_v4().to_string(),
                    start,
                    end,
                    primary,
                    translation: String::new(),
                    words: std::mem::take(&mut current_words),
                });
            }
            current_words.push(MusicLyricWord {
                value: word.value.trim().to_string(),
                start: word.start,
                end: word.end.max(word.start),
            });
        }

        if !current_words.is_empty() {
            let start = current_words.first().unwrap().start;
            let end = current_words.last().unwrap().end;
            let primary = current_words
                .iter()
                .map(|w| w.value.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            output.push(MusicLyricSegment {
                id: uuid::Uuid::new_v4().to_string(),
                start,
                end,
                primary,
                translation: String::new(),
                words: current_words,
            });
        }

        return output;
    }

    segments
        .into_iter()
        .filter(|segment| !segment.value.trim().is_empty())
        .map(|segment| MusicLyricSegment {
            id: uuid::Uuid::new_v4().to_string(),
            start: segment.start,
            end: segment.end,
            primary: segment.value,
            translation: String::new(),
            words: Vec::new(),
        })
        .collect()
}

pub(crate) fn splice_repaired_lyrics_segments(
    existing: &[MusicLyricSegment],
    repaired: Vec<MusicLyricSegment>,
    range_start: f64,
    range_end: f64,
) -> Vec<MusicLyricSegment> {
    let mut result = Vec::new();

    for segment in existing {
        if segment.end <= range_start + 0.05 {
            result.push(segment.clone());
            continue;
        }
        if segment.start >= range_end - 0.05 {
            result.push(segment.clone());
            continue;
        }

        if segment.start < range_start - 0.05 && segment.end > range_start {
            let truncated_words: Vec<MusicLyricWord> = segment
                .words
                .iter()
                .filter(|w| w.end <= range_start + 0.05)
                .cloned()
                .collect();
            let primary = if !truncated_words.is_empty() {
                truncated_words
                    .iter()
                    .map(|w| w.value.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                segment.primary.clone()
            };
            if range_start > segment.start + 0.1 {
                result.push(MusicLyricSegment {
                    id: segment.id.clone(),
                    start: segment.start,
                    end: range_start,
                    primary,
                    translation: segment.translation.clone(),
                    words: truncated_words,
                });
            }
            continue;
        }

        if segment.start < range_end && segment.end > range_end + 0.05 {
            let truncated_words: Vec<MusicLyricWord> = segment
                .words
                .iter()
                .filter(|w| w.start >= range_end - 0.05)
                .cloned()
                .collect();
            let primary = if !truncated_words.is_empty() {
                truncated_words
                    .iter()
                    .map(|w| w.value.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                segment.primary.clone()
            };
            if segment.end > range_end + 0.1 {
                result.push(MusicLyricSegment {
                    id: segment.id.clone(),
                    start: range_end,
                    end: segment.end,
                    primary,
                    translation: segment.translation.clone(),
                    words: truncated_words,
                });
            }
            continue;
        }
    }

    for seg in repaired {
        if !seg.primary.trim().is_empty() && seg.end > seg.start {
            result.push(seg);
        }
    }

    result.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap_or(std::cmp::Ordering::Equal));
    result
}

fn estimated_words(primary: &str, start: f64, end: f64) -> Vec<MusicLyricWord> {
    let words = primary.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return Vec::new();
    }
    let step = (end - start).max(0.01) / words.len() as f64;
    words
        .into_iter()
        .enumerate()
        .map(|(index, value)| MusicLyricWord {
            value: value.into(),
            start: start + step * index as f64,
            end: start + step * (index + 1) as f64,
        })
        .collect()
}

fn normalized_words(value: &str) -> String {
    value
        .split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_lyrics_document(
    mut document: MusicLyricsDocument,
    duration_seconds: f64,
) -> Result<MusicLyricsDocument, StudioError> {
    document.schema_version = 1;
    document.language = document.language.trim().to_string();
    document.source = document.source.trim().to_string();
    document.theme = document.theme.trim().to_ascii_lowercase();
    if document.theme.is_empty() {
        document.theme = DEFAULT_LYRIC_THEME.into();
    }
    document
        .segments
        .retain(|segment| !segment.primary.trim().is_empty());
    document
        .segments
        .sort_by(|left, right| left.start.total_cmp(&right.start));
    let mut ids = HashSet::new();
    for segment in &mut document.segments {
        if uuid::Uuid::parse_str(&segment.id).is_err() || !ids.insert(segment.id.clone()) {
            segment.id = uuid::Uuid::new_v4().to_string();
            ids.insert(segment.id.clone());
        }
        segment.primary = segment.primary.trim().to_string();
        segment.translation = segment.translation.trim().to_string();
        if !segment.start.is_finite() || !segment.end.is_finite() {
            return Err(StudioError::Invalid(
                "lyric cue times must be finite numbers".into(),
            ));
        }
        segment.start = segment.start.max(0.0).min(duration_seconds);
        segment.end = segment.end.max(segment.start + 0.01).min(duration_seconds);
        if segment.end <= segment.start {
            return Err(StudioError::Invalid(
                "every lyric cue must have time to appear before the take ends".into(),
            ));
        }
        segment.words.retain(|word| {
            !word.value.trim().is_empty()
                && word.start.is_finite()
                && word.end.is_finite()
                && word.end >= word.start
        });
        segment
            .words
            .sort_by(|left, right| left.start.total_cmp(&right.start));
        let timed_text = segment
            .words
            .iter()
            .map(|word| word.value.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if segment.words.is_empty()
            || normalized_words(&timed_text) != normalized_words(&segment.primary)
        {
            segment.words = estimated_words(&segment.primary, segment.start, segment.end);
        } else {
            for word in &mut segment.words {
                word.value = word.value.trim().to_string();
                word.start = word.start.max(segment.start).min(segment.end);
                word.end = word.end.max(word.start).min(segment.end);
            }
        }
    }
    validate_lyrics_document(&document, duration_seconds)?;
    Ok(document)
}

fn validate_lyrics_document(
    document: &MusicLyricsDocument,
    duration_seconds: f64,
) -> Result<(), StudioError> {
    if document.schema_version != 1 {
        return Err(StudioError::Invalid(
            "unsupported music lyric document version".into(),
        ));
    }
    if uuid::Uuid::parse_str(&document.take_id).is_err()
        || document.source_sha256.len() != 64
        || !document
            .source_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(StudioError::Invalid(
            "the lyric document has no valid immutable take identity".into(),
        ));
    }
    if document.language.is_empty()
        || document.language.len() > 64
        || !document
            .language
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic() || matches!(byte, b' ' | b'-'))
    {
        return Err(StudioError::Invalid("unsafe lyric language".into()));
    }
    if document.source.is_empty()
        || document.source.len() > 256
        || !is_supported_lyric_theme(&document.theme)
    {
        return Err(StudioError::Invalid(
            "the lyric source or visual theme is unsupported".into(),
        ));
    }
    if document.segments.len() > MAX_LYRIC_SEGMENTS
        || document
            .segments
            .iter()
            .map(|segment| segment.words.len())
            .sum::<usize>()
            > MAX_LYRIC_WORDS
    {
        return Err(StudioError::Invalid(
            "the lyric document exceeds its bounded cue or word count".into(),
        ));
    }
    let text_bytes = document.transcript.len()
        + document
            .segments
            .iter()
            .map(|segment| {
                segment.primary.len()
                    + segment.translation.len()
                    + segment
                        .words
                        .iter()
                        .map(|word| word.value.len())
                        .sum::<usize>()
            })
            .sum::<usize>();
    if text_bytes > MAX_LYRIC_TEXT_BYTES {
        return Err(StudioError::Invalid(
            "the lyric document exceeds the 2 MiB text boundary".into(),
        ));
    }
    let mut ids = HashSet::new();
    for segment in &document.segments {
        if uuid::Uuid::parse_str(&segment.id).is_err()
            || !ids.insert(&segment.id)
            || segment.primary.is_empty()
            || segment.primary.len() > 16 * 1024
            || segment.translation.len() > 16 * 1024
            || !segment.start.is_finite()
            || !segment.end.is_finite()
            || segment.start < 0.0
            || segment.end <= segment.start
            || segment.end > duration_seconds + 0.05
        {
            return Err(StudioError::Invalid(
                "the lyric document contains an invalid cue".into(),
            ));
        }
        if segment.words.iter().any(|word| {
            word.value.is_empty()
                || word.value.len() > 1_024
                || !word.start.is_finite()
                || !word.end.is_finite()
                || word.start < segment.start
                || word.end < word.start
                || word.end > segment.end + 0.05
        }) {
            return Err(StudioError::Invalid(
                "the lyric document contains an invalid word timing".into(),
            ));
        }
    }
    Ok(())
}

fn is_supported_lyric_theme(theme: &str) -> bool {
    matches!(theme, DEFAULT_LYRIC_THEME | SIGNAL_BLOOM_LYRIC_THEME)
}

fn render_lyrics(project: &MusicProject) -> String {
    project
        .sections
        .iter()
        .map(|section| {
            let body = if project.instrumental || section.lyrics.trim().is_empty() {
                String::new()
            } else {
                format!("\n{}", section.lyrics.trim())
            };
            format!("[{}]{body}", section.tag.trim())
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_caption(project: &MusicProject) -> String {
    let directions = project
        .sections
        .iter()
        .filter(|section| !section.direction.trim().is_empty())
        .map(|section| format!("- {}: {}", section.name.trim(), section.direction.trim()))
        .collect::<Vec<_>>();
    if directions.is_empty() {
        project.caption.trim().into()
    } else {
        format!(
            "{}\n\nSection direction:\n{}",
            project.caption.trim(),
            directions.join("\n")
        )
    }
}

fn validate_editable(project: &MusicProject) -> Result<(), StudioError> {
    validate_title(&project.title)?;
    for (name, value) in [
        ("song idea", &project.idea),
        ("music description", &project.caption),
    ] {
        if value.len() > MAX_MUSIC_TEXT_BYTES {
            return Err(StudioError::Invalid(format!(
                "{name} must not exceed 64 KiB"
            )));
        }
    }
    if project.sections.is_empty() || project.sections.len() > MAX_SECTIONS {
        return Err(StudioError::Invalid(
            "a song must contain 1 to 64 producer-owned sections".into(),
        ));
    }
    let mut ids = HashSet::new();
    for section in &project.sections {
        if uuid::Uuid::parse_str(&section.id).is_err() || !ids.insert(section.id.clone()) {
            return Err(StudioError::Invalid(
                "every song section must have a stable unique ID".into(),
            ));
        }
        if !valid_section_tag(&section.tag) {
            return Err(StudioError::Invalid(format!(
                "{} is not a supported MiniMax Music section tag",
                section.tag
            )));
        }
        if section.name.trim().is_empty() || section.name.chars().count() > 80 {
            return Err(StudioError::Invalid(
                "section names must contain 1 to 80 characters".into(),
            ));
        }
        if !(1..=128).contains(&section.bars) {
            return Err(StudioError::Invalid(
                "section length must be between 1 and 128 bars".into(),
            ));
        }
        if section.lyrics.len() > 16 * 1024 || section.direction.len() > 8 * 1024 {
            return Err(StudioError::Invalid(
                "a section exceeds the 16 KiB lyrics or 8 KiB direction limit".into(),
            ));
        }
    }
    let settings = &project.settings;
    if !settings.max_duration_seconds.is_finite()
        || !(1.0..=300.0).contains(&settings.max_duration_seconds)
    {
        return Err(StudioError::Invalid(
            "maximum song duration must be between 1 and 300 seconds".into(),
        ));
    }
    if !(1..=100).contains(&settings.steps)
        || !settings.cfg_scale.is_finite()
        || !(0.0..=100.0).contains(&settings.cfg_scale)
        || !(1..=16_384).contains(&settings.top_k)
    {
        return Err(StudioError::Invalid(
            "music sampling settings are outside the native MiniMax Music 3 limits".into(),
        ));
    }
    if !matches!(settings.model_variant.as_str(), "auto" | "int8" | "fp16") {
        return Err(StudioError::Invalid(
            "music model variant must be auto, int8, or fp16".into(),
        ));
    }
    if !settings.comfy_root.trim().is_empty()
        && !Path::new(settings.comfy_root.trim()).is_absolute()
    {
        return Err(StudioError::Invalid(
            "ComfyUI root must be an absolute local path".into(),
        ));
    }
    Ok(())
}

fn validate_render_ready(project: &MusicProject) -> Result<(), StudioError> {
    validate_editable(project)?;
    if project.caption.trim().chars().count() < 10 {
        return Err(StudioError::Invalid(
            "write or develop a music description before generating a take".into(),
        ));
    }
    if !project.instrumental
        && project
            .sections
            .iter()
            .all(|section| section.lyrics.trim().is_empty())
    {
        return Err(StudioError::Invalid(
            "add lyrics to at least one section or enable Instrumental".into(),
        ));
    }
    if render_lyrics(project).len() > MAX_MUSIC_TEXT_BYTES
        || render_caption(project).len() > MAX_MUSIC_TEXT_BYTES
    {
        return Err(StudioError::Invalid(
            "the compiled description or lyrics exceed the 64 KiB renderer boundary".into(),
        ));
    }
    Ok(())
}

fn validate_title(title: &str) -> Result<(), StudioError> {
    if title.trim().is_empty()
        || title.chars().count() > 120
        || title.chars().any(|value| value.is_control())
    {
        return Err(StudioError::Invalid(
            "song title must contain 1 to 120 printable characters".into(),
        ));
    }
    Ok(())
}

fn valid_section_tag(tag: &str) -> bool {
    matches!(
        tag,
        "Intro"
            | "Verse"
            | "Pre-Chorus"
            | "Chorus"
            | "Post-Chorus"
            | "Bridge"
            | "Instrumental"
            | "Solo"
            | "Break"
            | "Outro"
    )
}

fn validate_muscriptor_settings(settings: &MusicMidiSettings) -> Result<(), StudioError> {
    let executable = Path::new(settings.executable_path.trim());
    let model = Path::new(settings.model_path.trim());
    if !executable.is_absolute() || !executable.is_file() {
        return Err(StudioError::Invalid(
            format!(
                "MuScriptor runner was not found at {}. Choose the existing muscriptor.exe or finish MuScriptor in Setup before transcribing.",
                executable.display()
            ),
        ));
    }
    if !is_muscriptor_checkpoint(model) {
        return Err(StudioError::Invalid(
            format!(
                "The official {MUSCRIPTOR_MODEL_BYTES}-byte MuScriptor model.safetensors checkpoint was not found at {}. Choose that completed file or finish MuScriptor in Setup.",
                model.display()
            ),
        ));
    }
    if settings.instruments.len() > 2_000
        || settings
            .instruments
            .chars()
            .any(|value| value.is_control() && !value.is_whitespace())
    {
        return Err(StudioError::Invalid(
            "MuScriptor instrument guidance is invalid or too long".into(),
        ));
    }
    Ok(())
}

/// Repair a stale companion path only within the producer-selected MuScriptor installation.
/// This handles common source installs (`.venv/Scripts/muscriptor.exe` plus
/// `models/muscriptor-large/model.safetensors`) and Kestrel's isolated layout without searching
/// unrelated drives or accepting another model with the same filename.
fn repair_muscriptor_settings(settings: &mut MusicMidiSettings) -> bool {
    let executable = PathBuf::from(settings.executable_path.trim());
    let model = PathBuf::from(settings.model_path.trim());
    let mut changed = false;

    if executable.is_file() && !is_muscriptor_checkpoint(&model) {
        if let Some(root) = muscriptor_root_from_executable(&executable) {
            if let Some(found) = muscriptor_model_candidates(&root)
                .into_iter()
                .find(|path| is_muscriptor_checkpoint(path))
            {
                settings.model_path = found.to_string_lossy().into_owned();
                changed = true;
            }
        }
    }

    if !Path::new(settings.executable_path.trim()).is_file()
        && is_muscriptor_checkpoint(Path::new(settings.model_path.trim()))
    {
        if let Some(root) = muscriptor_root_from_model(Path::new(settings.model_path.trim())) {
            if let Some(found) = muscriptor_executable_candidates(&root)
                .into_iter()
                .find(|path| path.is_file())
            {
                settings.executable_path = found.to_string_lossy().into_owned();
                changed = true;
            }
        }
    }
    changed
}

fn is_muscriptor_checkpoint(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("model.safetensors"))
        && fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == MUSCRIPTOR_MODEL_BYTES)
}

fn muscriptor_root_from_executable(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    if name.eq_ignore_ascii_case("uvx.exe") {
        return path.parent()?.parent().map(Path::to_path_buf);
    }
    if name.eq_ignore_ascii_case("muscriptor.exe") {
        return path.parent()?.parent()?.parent().map(Path::to_path_buf);
    }
    None
}

fn muscriptor_root_from_model(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    if parent.file_name()?.to_str()?.eq_ignore_ascii_case("models") {
        return parent.parent().map(Path::to_path_buf);
    }
    let models = parent.parent()?;
    if models.file_name()?.to_str()?.eq_ignore_ascii_case("models") {
        return models.parent().map(Path::to_path_buf);
    }
    None
}

fn muscriptor_model_candidates(root: &Path) -> [PathBuf; 2] {
    [
        root.join("models/model.safetensors"),
        root.join("models/muscriptor-large/model.safetensors"),
    ]
}

fn muscriptor_executable_candidates(root: &Path) -> [PathBuf; 2] {
    [
        root.join("runtime/uvx.exe"),
        root.join(".venv/Scripts/muscriptor.exe"),
    ]
}

fn is_managed_muscriptor_uvx(executable: &Path, model: &Path) -> bool {
    let Some(runtime) = executable.parent() else {
        return false;
    };
    let Some(root) = runtime.parent() else {
        return false;
    };
    executable
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("uvx.exe"))
        && runtime
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("runtime"))
        && model == root.join("models/model.safetensors")
}

async fn verify_music_nodes(http: &Client, tiled_decode: bool) -> Result<(), StudioError> {
    let info: Value = http
        .get(format!("{MUSIC_COMFY_BASE}/object_info"))
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .json()
        .await?;
    for node in [
        "MiniMaxMusic3TextEncode",
        "EmptyMiniMaxMusic3LatentAudio",
        "SaveAudioAdvanced",
        if tiled_decode {
            "VAEDecodeAudioTiled"
        } else {
            "VAEDecodeAudio"
        },
    ] {
        if info.get(node).is_none() {
            return Err(StudioError::Render(format!(
                "the running ComfyUI does not expose {node}. Update ComfyUI to 0.33.0 or newer, restart it, then resume Music Production in Setup"
            )));
        }
    }
    Ok(())
}

fn resolve_music_model(comfy_root: &Path, variant: &str) -> Result<String, StudioError> {
    let models = comfy_root.join("models").join("diffusion_models");
    let fp16 = models.join(MUSIC_DIT_FP16);
    let int8 = models.join(MUSIC_DIT_INT8);
    let selected = match variant {
        "fp16" => &fp16,
        "int8" => &int8,
        "auto" if fp16.is_file() => &fp16,
        "auto" => &int8,
        _ => {
            return Err(StudioError::Invalid(
                "unknown MiniMax Music 3 model variant".into(),
            ))
        }
    };
    if !selected.is_file() {
        return Err(StudioError::Render(format!(
            "{} is missing. Open Setup and install Music Production, or choose a model variant that is present.",
            selected.display()
        )));
    }
    Ok(selected
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(MUSIC_DIT_INT8)
        .into())
}

fn verify_music_assets(comfy_root: &Path, model_file: &str) -> Result<(), StudioError> {
    for relative in [
        format!("diffusion_models/{model_file}"),
        format!("text_encoders/{MUSIC_TEXT_ENCODER}"),
        format!("vae/{MUSIC_VAE}"),
    ] {
        let path = comfy_root.join("models").join(&relative);
        if !path.is_file() {
            return Err(StudioError::Render(format!(
                "MiniMax Music 3 asset is missing: {}. Resume Music Production in Setup.",
                path.display()
            )));
        }
    }
    Ok(())
}

fn minimax_music_graph(
    settings: &MusicSettings,
    caption: &str,
    lyrics: &str,
    seed: u64,
    model_file: &str,
    prefix: &str,
) -> Value {
    let decode_node = if settings.tiled_decode {
        json!({
            "class_type":"VAEDecodeAudioTiled",
            "inputs":{"samples":["7",0],"vae":["3",0],"tile_size":1536,"overlap":64}
        })
    } else {
        json!({
            "class_type":"VAEDecodeAudio",
            "inputs":{"samples":["7",0],"vae":["3",0]}
        })
    };
    json!({
        "1":{"class_type":"UNETLoader","inputs":{"unet_name":model_file,"weight_dtype":"default"}},
        "2":{"class_type":"CLIPLoader","inputs":{"clip_name":MUSIC_TEXT_ENCODER,"type":"minimax","device":"default"}},
        "3":{"class_type":"VAELoader","inputs":{"vae_name":MUSIC_VAE}},
        "4":{"class_type":"MiniMaxMusic3TextEncode","inputs":{
            "clip":["2",0],"caption":caption,"lyrics":lyrics,"seed":seed,
            "max_duration":settings.max_duration_seconds,"cfg_scale":settings.cfg_scale,"top_k":settings.top_k
        }},
        "5":{"class_type":"ConditioningZeroOut","inputs":{"conditioning":["4",0]}},
        "6":{"class_type":"EmptyMiniMaxMusic3LatentAudio","inputs":{"seconds":["4",1],"batch_size":1}},
        "7":{"class_type":"KSampler","inputs":{
            "model":["1",0],"positive":["4",0],"negative":["5",0],"latent_image":["6",0],
            "seed":seed,"steps":settings.steps,"cfg":settings.cfg_scale,"sampler_name":"euler",
            "scheduler":"simple","denoise":1.0
        }},
        "8":decode_node,
        "10":{"class_type":"SaveAudioAdvanced","inputs":{
            "audio":["8",0],"filename_prefix":prefix,"format":"flac"
        }}
    })
}

fn resolved_seed(seed: u64) -> u64 {
    if seed == 0 {
        Utc::now().timestamp_nanos_opt().unwrap_or_default() as u64 & i64::MAX as u64
    } else {
        seed & i64::MAX as u64
    }
}

fn probe_audio_duration(path: &Path) -> Option<f64> {
    let program = std::env::var_os("KESTREL_FFPROBE_PATH")
        .map(PathBuf::from)
        .filter(|value| value.is_file())
        .unwrap_or_else(|| PathBuf::from("ffprobe"));
    let output = std::process::Command::new(program)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn read_project(path: &Path) -> Result<MusicProject, StudioError> {
    match read_project_file(path) {
        Ok(project) => Ok(project),
        Err(primary_error) => {
            let backup = path.with_extension("json.bak");
            match read_project_file(&backup) {
                Ok(project) => {
                    fs::copy(&backup, path)?;
                    Ok(project)
                }
                Err(_) => Err(primary_error),
            }
        }
    }
}

fn read_project_file(path: &Path) -> Result<MusicProject, StudioError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return Err(StudioError::Invalid(
            "music project manifest is missing or exceeds 16 MiB".into(),
        ));
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn read_midi_document(path: &Path) -> Result<MusicMidiDocument, StudioError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 64 * 1024 * 1024 {
        return Err(StudioError::Invalid(
            "the MIDI edit document is missing or exceeds 64 MiB".into(),
        ));
    }
    let document: MusicMidiDocument = serde_json::from_slice(&fs::read(path)?)?;
    validate_midi_document(&document)?;
    Ok(document)
}

fn read_lyrics_document(path: &Path) -> Result<MusicLyricsDocument, StudioError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 16 * 1024 * 1024 {
        return Err(StudioError::Invalid(
            "the lyric edit document is missing or exceeds 16 MiB".into(),
        ));
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn validate_music_id(id: &str) -> Result<(), StudioError> {
    if uuid::Uuid::parse_str(id).is_err() {
        Err(StudioError::Invalid("invalid music project ID".into()))
    } else {
        Ok(())
    }
}

fn write_json_recoverable(path: &Path, value: &impl Serialize) -> Result<(), StudioError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("json")
    ));
    let backup = path.with_extension(format!(
        "{}.bak",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("json")
    ));
    {
        let mut output = fs::File::create(&temporary)?;
        output.write_all(&bytes)?;
        output.sync_all()?;
    }
    if !path.exists() {
        fs::rename(&temporary, path)?;
        return Ok(());
    }
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    fs::rename(path, &backup)?;
    match fs::rename(&temporary, path) {
        Ok(()) => {
            let _ = fs::remove_file(&backup);
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup, path);
            let _ = fs::remove_file(&temporary);
            Err(StudioError::Io(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::music_midi::{
        MusicMidiNote, MusicMidiTempo, MusicMidiTimeSignature, MusicMidiTrack,
    };
    use tempfile::TempDir;

    fn project(studio: &MusicStudio) -> MusicProject {
        let mut project = studio
            .create(CreateMusicProjectRequest {
                title: "Night signal".into(),
                idea: "Warm analog synth-pop at night".into(),
                comfy_root: r"D:\AI\ComfyUI".into(),
                muscriptor_executable_path: String::new(),
                muscriptor_model_path: String::new(),
            })
            .unwrap();
        project.caption = "Global Metadata: synth-pop, 112 BPM.\n\nVocal Details: intimate alto.\n\nArrangement: analog drums and wide pads.".into();
        project.sections[1].lyrics = "The streetlights answer me".into();
        studio.save_editable(project).unwrap()
    }

    fn completed_take(studio: &MusicStudio, project: &MusicProject) -> (MusicProject, String) {
        let (_, take_id) = studio.begin_generation(&project.id, None).unwrap();
        let mut stored = studio.get(&project.id).unwrap();
        let master = studio
            .project_dir(&project.id)
            .join("takes")
            .join(format!("{take_id}.flac"));
        fs::write(&master, b"immutable generated music master").unwrap();
        let (bytes, sha256) = hash_file(&master).unwrap();
        let take = stored
            .takes
            .iter_mut()
            .find(|take| take.id == take_id)
            .unwrap();
        take.status = "complete".into();
        take.path = master.to_string_lossy().into_owned();
        take.bytes = bytes;
        take.sha256 = sha256;
        take.duration_seconds = 42.0;
        stored.status = "ready".into();
        stored.phase = "take-ready".into();
        studio.persist(&stored).unwrap();
        (stored, take_id)
    }

    #[test]
    fn lyric_drafts_and_producer_edits_are_durable_immutable_revisions() {
        let root = TempDir::new().unwrap();
        let studio = MusicStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let (project, take_id) = completed_take(&studio, &project);
        let draft = studio
            .create_lyrics_draft(MusicLyricsRequest {
                project_id: project.id.clone(),
                take_id: take_id.clone(),
            })
            .unwrap();
        assert_eq!(draft.document.source, "producer-timing-draft");
        assert!(!draft.document.segments.is_empty());
        assert!(!draft.document.segments[0].words.is_empty());
        let original_path = PathBuf::from(&draft.project.takes[0].lyrics_document_path);
        assert!(original_path.is_file());

        let mut edited = draft.document.clone();
        edited.segments[0].primary = "A producer rewrote this cue".into();
        edited.segments[0].translation = "Tuottaja muokkasi tämän rivin".into();
        edited.theme = SIGNAL_BLOOM_LYRIC_THEME.into();
        edited.source = "forged-remote-service".into();
        let saved = studio
            .save_lyrics_document(SaveMusicLyricsDocumentRequest {
                project_id: project.id.clone(),
                take_id: take_id.clone(),
                document: edited,
            })
            .unwrap();
        assert_eq!(saved.document.revision, 1);
        assert_eq!(saved.document.source, "producer-timing-draft");
        assert_eq!(saved.document.theme, SIGNAL_BLOOM_LYRIC_THEME);
        assert_eq!(saved.document.segments[0].words[0].value, "A");
        assert!(original_path.is_file());
        assert_ne!(
            saved.project.takes[0].lyrics_document_path,
            original_path.to_string_lossy()
        );

        let mut unsupported_theme = saved.document.clone();
        unsupported_theme.theme = "downloaded-javascript-theme".into();
        assert!(studio
            .save_lyrics_document(SaveMusicLyricsDocumentRequest {
                project_id: project.id.clone(),
                take_id: take_id.clone(),
                document: unsupported_theme,
            })
            .is_err());

        let synced = studio
            .persist_lyrics_transcription(
                &TranscribeMusicLyricsRequest {
                    project_id: project.id.clone(),
                    take_id: take_id.clone(),
                    job_id: uuid::Uuid::new_v4().to_string(),
                    model_id: "whisper-local".into(),
                    language: "English".into(),
                },
                SpeechFileTranscription {
                    text: "A producer rewrote this cue".into(),
                    segments: vec![SpeechTiming {
                        value: "A producer rewrote this cue".into(),
                        start: 1.0,
                        end: 4.0,
                    }],
                    words: vec![
                        SpeechTiming {
                            value: "A".into(),
                            start: 1.0,
                            end: 1.2,
                        },
                        SpeechTiming {
                            value: "producer".into(),
                            start: 1.2,
                            end: 2.0,
                        },
                        SpeechTiming {
                            value: "rewrote".into(),
                            start: 2.0,
                            end: 2.8,
                        },
                        SpeechTiming {
                            value: "this".into(),
                            start: 2.8,
                            end: 3.2,
                        },
                        SpeechTiming {
                            value: "cue".into(),
                            start: 3.2,
                            end: 4.0,
                        },
                    ],
                    strategy: "whisper-repeat-context-v1".into(),
                    context_copies: 2,
                    context_seam_seconds: 1.0,
                    selected_context_copy: 2,
                    first_context_score: 0.4,
                    second_context_score: 0.8,
                },
            )
            .unwrap();
        assert_eq!(synced.document.theme, SIGNAL_BLOOM_LYRIC_THEME);

        assert!(studio
            .save_lyrics_document(SaveMusicLyricsDocumentRequest {
                project_id: project.id,
                take_id,
                document: draft.document,
            })
            .is_err());
    }

    #[test]
    fn speech_segments_partition_words_into_sentence_cues() {
        let segments = lyric_segments_from_speech(
            vec![SpeechTiming {
                value: "sing it now outside".into(),
                start: 3.0,
                end: 8.5,
            }],
            &[
                SpeechTiming {
                    value: "sing".into(),
                    start: 3.0,
                    end: 3.5,
                },
                SpeechTiming {
                    value: "it".into(),
                    start: 3.6,
                    end: 4.0,
                },
                SpeechTiming {
                    value: "outside".into(),
                    start: 8.0,
                    end: 8.5,
                },
            ],
        );
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].primary, "sing it");
        assert_eq!(segments[0].words.len(), 2);
        assert_eq!(segments[1].primary, "outside");
        assert_eq!(segments[1].words.len(), 1);

        // Screenshot 1 pattern: Ellipsis and capital letter starts
        let screenshot_1 = lyric_segments_from_speech(
            Vec::new(),
            &[
                SpeechTiming { value: "Hello...".into(), start: 0.5, end: 1.2 },
                SpeechTiming { value: "Hello...".into(), start: 2.0, end: 2.8 },
                SpeechTiming { value: "I'm".into(), start: 4.5, end: 4.8 },
                SpeechTiming { value: "sitting".into(), start: 4.9, end: 5.3 },
                SpeechTiming { value: "by".into(), start: 5.4, end: 5.5 },
                SpeechTiming { value: "the".into(), start: 5.6, end: 5.7 },
                SpeechTiming { value: "window...".into(), start: 5.8, end: 6.4 },
                SpeechTiming { value: "Rain".into(), start: 7.2, end: 7.6 },
                SpeechTiming { value: "on".into(), start: 7.7, end: 7.8 },
                SpeechTiming { value: "the".into(), start: 7.9, end: 8.0 },
                SpeechTiming { value: "glass...".into(), start: 8.1, end: 8.7 },
            ],
        );
        assert_eq!(screenshot_1.len(), 4);
        assert_eq!(screenshot_1[0].primary, "Hello...");
        assert_eq!(screenshot_1[1].primary, "Hello...");
        assert_eq!(screenshot_1[2].primary, "I'm sitting by the window...");
        assert_eq!(screenshot_1[3].primary, "Rain on the glass...");

        // Screenshot 2 pattern: Capital letters at line starts without punctuation
        let screenshot_2 = lyric_segments_from_speech(
            Vec::new(),
            &[
                SpeechTiming { value: "Deep".into(), start: 0.5, end: 0.9 },
                SpeechTiming { value: "and".into(), start: 1.0, end: 1.2 },
                SpeechTiming { value: "steep,".into(), start: 1.3, end: 1.7 },
                SpeechTiming { value: "a".into(), start: 1.8, end: 1.9 },
                SpeechTiming { value: "silent".into(), start: 2.0, end: 2.4 },
                SpeechTiming { value: "geometry".into(), start: 2.5, end: 3.1 },
                SpeechTiming { value: "A".into(), start: 3.4, end: 3.6 },
                SpeechTiming { value: "promise".into(), start: 3.7, end: 4.1 },
                SpeechTiming { value: "I".into(), start: 4.2, end: 4.3 },
                SpeechTiming { value: "keep".into(), start: 4.4, end: 4.9 },
                SpeechTiming { value: "The".into(), start: 5.2, end: 5.4 },
                SpeechTiming { value: "universe".into(), start: 5.5, end: 6.0 },
                SpeechTiming { value: "spins".into(), start: 6.1, end: 6.5 },
            ],
        );
        assert_eq!(screenshot_2.len(), 3);
        assert_eq!(screenshot_2[0].primary, "Deep and steep, a silent geometry");
        assert_eq!(screenshot_2[1].primary, "A promise I keep");
        assert_eq!(screenshot_2[2].primary, "The universe spins");

        // Mid-sentence pronoun "I" without pause should not split
        let mid_sentence_i = lyric_segments_from_speech(
            Vec::new(),
            &[
                SpeechTiming { value: "when".into(), start: 1.0, end: 1.3 },
                SpeechTiming { value: "I".into(), start: 1.35, end: 1.5 },
                SpeechTiming { value: "look".into(), start: 1.55, end: 1.8 },
                SpeechTiming { value: "outside".into(), start: 1.85, end: 2.2 },
            ],
        );
        assert_eq!(mid_sentence_i.len(), 1);
        assert_eq!(mid_sentence_i[0].primary, "when I look outside");

        // Fallback to coarse segments when words are empty
        let fallback = lyric_segments_from_speech(
            vec![
                SpeechTiming { value: "first".into(), start: 0.0, end: 2.0 },
                SpeechTiming { value: "second".into(), start: 2.0, end: 4.0 },
            ],
            &[],
        );
        assert_eq!(fallback.len(), 2);
        assert_eq!(fallback[0].primary, "first");
        assert_eq!(fallback[1].primary, "second");
    }

    #[test]
    fn lyric_sync_refuses_a_master_that_changed_after_generation() {
        let root = TempDir::new().unwrap();
        let studio = MusicStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let (project, take_id) = completed_take(&studio, &project);
        fs::write(&project.takes[0].path, b"altered after preservation").unwrap();
        assert!(studio
            .lyrics_audio_source(&MusicLyricsRequest {
                project_id: project.id,
                take_id,
            })
            .is_err());
    }

    #[test]
    fn range_repair_maintains_consistent_take_and_document_revisions() {
        let root = TempDir::new().unwrap();
        let studio = MusicStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let (project, take_id) = completed_take(&studio, &project);

        // 1. Initial draft (revision 0)
        let initial = studio
            .create_lyrics_draft(MusicLyricsRequest {
                project_id: project.id.clone(),
                take_id: take_id.clone(),
            })
            .unwrap();
        assert_eq!(initial.document.revision, 0);
        assert_eq!(initial.project.takes[0].lyrics_revision, 0);

        // 2. Range repair creates revision 1
        let repaired_1 = studio
            .persist_lyrics_range_repair(
                &RepairMusicLyricsRangeRequest {
                    project_id: project.id.clone(),
                    take_id: take_id.clone(),
                    job_id: "test-job-1".into(),
                    model_id: "whisper-test".into(),
                    language: "en".into(),
                    start_seconds: 0.0,
                    end_seconds: 3.0,
                    prompt: "repaired first line".into(),
                },
                SpeechFileTranscription {
                    text: "repaired first line".into(),
                    segments: vec![SpeechTiming {
                        value: "repaired first line".into(),
                        start: 0.0,
                        end: 2.5,
                    }],
                    words: vec![
                        SpeechTiming { value: "repaired".into(), start: 0.0, end: 0.8 },
                        SpeechTiming { value: "first".into(), start: 0.8, end: 1.5 },
                        SpeechTiming { value: "line".into(), start: 1.5, end: 2.5 },
                    ],
                    strategy: "whisper-range-repair".into(),
                    context_copies: 1,
                    context_seam_seconds: 0.0,
                    selected_context_copy: 1,
                    first_context_score: 1.0,
                    second_context_score: 1.0,
                },
            )
            .unwrap();

        assert_eq!(repaired_1.document.revision, 1);
        assert_eq!(repaired_1.project.takes[0].lyrics_revision, 1);

        // 3. load_lyrics_document succeeds immediately with matching revision
        let loaded = studio
            .load_lyrics_document(MusicLyricsRequest {
                project_id: project.id.clone(),
                take_id: take_id.clone(),
            })
            .unwrap();
        assert_eq!(loaded.document.revision, 1);
        assert_eq!(loaded.project.takes[0].lyrics_revision, 1);

        // 4. Consecutive range repair creates revision 2
        let repaired_2 = studio
            .persist_lyrics_range_repair(
                &RepairMusicLyricsRangeRequest {
                    project_id: project.id.clone(),
                    take_id: take_id.clone(),
                    job_id: "test-job-2".into(),
                    model_id: "whisper-test".into(),
                    language: "en".into(),
                    start_seconds: 2.5,
                    end_seconds: 5.0,
                    prompt: "repaired second line".into(),
                },
                SpeechFileTranscription {
                    text: "repaired second line".into(),
                    segments: vec![SpeechTiming {
                        value: "repaired second line".into(),
                        start: 2.6,
                        end: 4.8,
                    }],
                    words: vec![
                        SpeechTiming { value: "repaired".into(), start: 2.6, end: 3.2 },
                        SpeechTiming { value: "second".into(), start: 3.2, end: 4.0 },
                        SpeechTiming { value: "line".into(), start: 4.0, end: 4.8 },
                    ],
                    strategy: "whisper-range-repair".into(),
                    context_copies: 1,
                    context_seam_seconds: 0.0,
                    selected_context_copy: 1,
                    first_context_score: 1.0,
                    second_context_score: 1.0,
                },
            )
            .unwrap();

        assert_eq!(repaired_2.document.revision, 2);
        assert_eq!(repaired_2.project.takes[0].lyrics_revision, 2);

        // 5. Reopen succeeds without provenance mismatch error
        let loaded_2 = studio
            .load_lyrics_document(MusicLyricsRequest {
                project_id: project.id.clone(),
                take_id,
            })
            .unwrap();
        assert_eq!(loaded_2.document.revision, 2);
        assert_eq!(loaded_2.project.takes[0].lyrics_revision, 2);
    }

    #[test]
    #[cfg(windows)]
    fn only_the_managed_uvx_runner_selects_the_pinned_offline_invocation() {
        let managed_model = Path::new(r"C:\Kestrel AI\MuScriptor\models\model.safetensors");
        assert!(is_managed_muscriptor_uvx(
            Path::new(r"C:\Kestrel AI\MuScriptor\runtime\uvx.exe"),
            managed_model,
        ));
        assert!(!is_managed_muscriptor_uvx(
            Path::new(r"C:\Tools\uvx.exe"),
            managed_model,
        ));
        assert!(!is_managed_muscriptor_uvx(
            Path::new(r"C:\Tools\muscriptor.exe"),
            managed_model,
        ));
    }

    #[test]
    fn repairs_a_stale_checkpoint_from_the_selected_source_install() {
        let root = TempDir::new().unwrap();
        let executable = root.path().join(".venv/Scripts/muscriptor.exe");
        let checkpoint = root
            .path()
            .join("models/muscriptor-large/model.safetensors");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::create_dir_all(checkpoint.parent().unwrap()).unwrap();
        fs::write(&executable, b"test runner").unwrap();
        fs::File::create(&checkpoint)
            .unwrap()
            .set_len(MUSCRIPTOR_MODEL_BYTES)
            .unwrap();
        let mut settings = MusicMidiSettings {
            executable_path: executable.to_string_lossy().into_owned(),
            model_path: root
                .path()
                .join("old-location/model.safetensors")
                .to_string_lossy()
                .into_owned(),
            instruments: String::new(),
        };

        assert!(repair_muscriptor_settings(&mut settings));
        assert_eq!(Path::new(&settings.model_path), checkpoint);
        validate_muscriptor_settings(&settings).unwrap();
    }

    #[test]
    fn producer_structure_compiles_to_tagged_lyrics_and_section_direction() {
        let root = TempDir::new().unwrap();
        let studio = MusicStudio::new(root.path()).unwrap();
        let mut project = project(&studio);
        project.sections[1].direction = "bass enters after two bars".into();
        assert!(render_lyrics(&project).contains("[Verse]\nThe streetlights answer me"));
        assert!(render_caption(&project).contains("Verse 1: bass enters after two bars"));
    }

    #[test]
    fn saves_only_editable_state_and_preserves_take_truth() {
        let root = TempDir::new().unwrap();
        let studio = MusicStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let (mut rendering, take_id) = studio.begin_generation(&project.id, None).unwrap();
        rendering.status = "draft".into();
        rendering.takes[0].path = "C:\\forged.wav".into();
        assert!(studio.save_editable(rendering).is_err());
        studio.fail_generation(&project.id, &take_id, "test".into(), true, None);
        let mut editable = studio.get(&project.id).unwrap();
        editable.title = "Renamed".into();
        editable.takes[0].path = "C:\\forged.wav".into();
        let saved = studio.save_editable(editable).unwrap();
        assert_eq!(saved.title, "Renamed");
        assert!(saved.takes[0].path.is_empty());
    }

    #[test]
    fn legacy_midi_is_preserved_then_producer_edits_create_new_revisions() {
        let root = TempDir::new().unwrap();
        let studio = MusicStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let (_, take_id) = studio.begin_generation(&project.id, None).unwrap();
        let mut stored = studio.get(&project.id).unwrap();
        let take = stored
            .takes
            .iter_mut()
            .find(|take| take.id == take_id)
            .unwrap();
        take.status = "complete".into();
        let master = studio.project_dir(&project.id).join("takes/master.flac");
        fs::write(&master, b"preserved master").unwrap();
        take.path = master.to_string_lossy().into_owned();
        let legacy = studio
            .project_dir(&project.id)
            .join("midi")
            .join(format!("{take_id}.mid"));
        let source_document = MusicMidiDocument {
            schema_version: 1,
            take_id: take_id.clone(),
            source_sha256: "a".repeat(64),
            revision: 0,
            ticks_per_quarter: 480,
            duration_ticks: 480,
            duration_seconds: 0.5,
            tempos: vec![MusicMidiTempo {
                tick: 0,
                microseconds_per_quarter: 500_000,
            }],
            time_signatures: vec![MusicMidiTimeSignature {
                tick: 0,
                numerator: 4,
                denominator: 4,
            }],
            tracks: vec![MusicMidiTrack {
                id: "track-1".into(),
                name: "Piano".into(),
                channel: 0,
                program: 0,
                muted: false,
                notes: vec![MusicMidiNote {
                    id: "note-1".into(),
                    pitch: 60,
                    start_tick: 0,
                    duration_ticks: 480,
                    velocity: 96,
                    channel: 0,
                }],
            }],
        };
        write_midi_document(&legacy, &source_document).unwrap();
        take.midi_path = legacy.to_string_lossy().into_owned();
        stored.status = "ready".into();
        studio.persist(&stored).unwrap();

        let loaded = studio
            .load_midi_document(MusicMidiRequest {
                project_id: project.id.clone(),
                take_id: take_id.clone(),
            })
            .unwrap();
        assert!(Path::new(&loaded.project.takes[0].midi_source_path).is_file());
        assert_eq!(loaded.document.revision, 0);
        let original_revision = loaded.project.takes[0].midi_path.clone();
        let mut edited = loaded.document;
        edited.tracks[0].notes[0].pitch = 64;
        let saved = studio
            .save_midi_document(SaveMusicMidiDocumentRequest {
                project_id: project.id,
                take_id,
                document: edited,
            })
            .unwrap();
        assert_eq!(saved.document.revision, 1);
        assert_eq!(saved.document.tracks[0].notes[0].pitch, 64);
        assert!(Path::new(&original_revision).is_file());
        assert!(Path::new(&saved.project.takes[0].midi_path).is_file());
        assert_ne!(saved.project.takes[0].midi_path, original_revision);
    }

    #[test]
    fn graph_matches_native_minimax_music_nodes_and_lossless_output() {
        let settings = MusicSettings {
            comfy_root: r"D:\AI\ComfyUI".into(),
            ..MusicSettings::default()
        };
        let graph = minimax_music_graph(
            &settings,
            "Global Metadata: jazz",
            "[Instrumental]",
            42,
            MUSIC_DIT_INT8,
            "kestrel_music/test/take_1",
        );
        assert_eq!(graph["4"]["class_type"], "MiniMaxMusic3TextEncode");
        assert_eq!(graph["6"]["inputs"]["seconds"], json!(["4", 1]));
        assert_eq!(graph["8"]["class_type"], "VAEDecodeAudioTiled");
        assert_eq!(graph["10"]["class_type"], "SaveAudioAdvanced");
        assert_eq!(graph["10"]["inputs"]["format"], "flac");
    }

    #[test]
    fn graph_uses_the_available_non_tiled_audio_decoder_when_requested() {
        let settings = MusicSettings {
            tiled_decode: false,
            comfy_root: r"D:\AI\ComfyUI".into(),
            ..MusicSettings::default()
        };
        let graph = minimax_music_graph(
            &settings,
            "Global Metadata: jazz",
            "[Instrumental]",
            42,
            MUSIC_DIT_INT8,
            "kestrel_music/test/take_1",
        );
        assert_eq!(graph["8"]["class_type"], "VAEDecodeAudio");
    }

    #[test]
    fn missing_primary_project_is_restored_from_its_backup() {
        let root = TempDir::new().unwrap();
        let studio = MusicStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let manifest = studio.project_dir(&project.id).join("project.json");
        let backup = manifest.with_extension("json.bak");
        fs::copy(&manifest, &backup).unwrap();
        fs::remove_file(&manifest).unwrap();

        let recovered = studio.get(&project.id).unwrap();
        assert_eq!(recovered.id, project.id);
        assert!(manifest.is_file());
    }

    #[test]
    fn progress_messages_are_bounded_and_include_eta() {
        let event = parse_music_progress(
            &json!({"type":"progress","data":{"value":5,"max":20}}).to_string(),
            "project",
            "take",
            Instant::now(),
        )
        .unwrap();
        assert_eq!(event.percent, Some(25.0));
        assert_eq!(event.step, Some(5));
        assert!(parse_music_progress(
            &json!({"type":"progress","data":{"value":21,"max":20}}).to_string(),
            "project",
            "take",
            Instant::now(),
        )
        .is_none());
    }

    #[tokio::test]
    #[ignore = "requires KESTREL_LIVE_COMFY_ROOT, native MiniMax Music 3 nodes and weights, and several minutes"]
    async fn live_minimax_music_graph_preserves_a_lossless_local_take() {
        let comfy_root = std::env::var("KESTREL_LIVE_COMFY_ROOT")
            .expect("set KESTREL_LIVE_COMFY_ROOT to the absolute ComfyUI folder");
        let root = TempDir::new().unwrap();
        let studio = MusicStudio::new(root.path()).unwrap();
        let renderer = MovieStudio::new(root.path()).unwrap();
        let mut project = studio
            .create(CreateMusicProjectRequest {
                title: "Music 3 acceptance".into(),
                idea: "A short, wordless chamber-electronic cue".into(),
                comfy_root,
                muscriptor_executable_path: String::new(),
                muscriptor_model_path: String::new(),
            })
            .unwrap();
        project.caption = "Global Metadata: chamber electronic, 92 BPM, D minor, intimate and resolved.\n\nVocal Details: instrumental, no voice.\n\nArrangement: felt piano and warm analog pulse, one clear eight-bar arc with a quiet ending.".into();
        project.instrumental = true;
        project.settings.max_duration_seconds = 15.0;
        project.sections = vec![MusicSection {
            id: uuid::Uuid::new_v4().to_string(),
            tag: "Instrumental".into(),
            name: "Complete cue".into(),
            bars: 8,
            lyrics: String::new(),
            direction: "piano states the motif, pulse enters, both resolve cleanly".into(),
        }];
        let project = studio.save_editable(project).unwrap();
        let (_, take_id) = studio.begin_generation(&project.id, None).unwrap();
        let rendered = studio
            .render(
                &project.id,
                &take_id,
                &renderer,
                &CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        let take = rendered
            .takes
            .iter()
            .find(|take| take.id == take_id)
            .unwrap();
        assert_eq!(take.status, "complete");
        assert!(Path::new(&take.path).is_file());
        assert!(take.bytes > 0);
        assert_eq!(take.sha256.len(), 64);
        assert_eq!(take.exact_graph["10"]["inputs"]["format"], "flac");
    }

    #[test]
    fn range_repair_splices_cues_within_target_window_and_preserves_surrounding() {
        let existing = vec![
            MusicLyricSegment {
                id: "cue-1".into(),
                start: 0.0,
                end: 10.0,
                primary: "intro line untouched".into(),
                translation: String::new(),
                words: vec![
                    MusicLyricWord { value: "intro".into(), start: 0.0, end: 3.0 },
                    MusicLyricWord { value: "line".into(), start: 3.2, end: 6.0 },
                    MusicLyricWord { value: "untouched".into(), start: 6.5, end: 9.5 },
                ],
            },
            MusicLyricSegment {
                id: "cue-2".into(),
                start: 11.0,
                end: 25.0,
                primary: "broken words to replace".into(),
                translation: String::new(),
                words: vec![
                    MusicLyricWord { value: "broken".into(), start: 11.0, end: 15.0 },
                    MusicLyricWord { value: "words".into(), start: 15.5, end: 20.0 },
                    MusicLyricWord { value: "to".into(), start: 20.5, end: 22.0 },
                    MusicLyricWord { value: "replace".into(), start: 22.5, end: 25.0 },
                ],
            },
            MusicLyricSegment {
                id: "cue-3".into(),
                start: 30.0,
                end: 40.0,
                primary: "outro line untouched".into(),
                translation: String::new(),
                words: vec![
                    MusicLyricWord { value: "outro".into(), start: 30.0, end: 34.0 },
                    MusicLyricWord { value: "line".into(), start: 34.5, end: 37.0 },
                    MusicLyricWord { value: "untouched".into(), start: 37.5, end: 40.0 },
                ],
            },
        ];

        let repaired = vec![
            MusicLyricSegment {
                id: "cue-rep-1".into(),
                start: 11.2,
                end: 17.5,
                primary: "deep and steep".into(),
                translation: String::new(),
                words: vec![
                    MusicLyricWord { value: "deep".into(), start: 11.2, end: 13.0 },
                    MusicLyricWord { value: "and".into(), start: 13.2, end: 14.5 },
                    MusicLyricWord { value: "steep".into(), start: 14.8, end: 17.5 },
                ],
            },
            MusicLyricSegment {
                id: "cue-rep-2".into(),
                start: 18.0,
                end: 24.8,
                primary: "a silent geometry".into(),
                translation: String::new(),
                words: vec![
                    MusicLyricWord { value: "a".into(), start: 18.0, end: 19.0 },
                    MusicLyricWord { value: "silent".into(), start: 19.2, end: 21.5 },
                    MusicLyricWord { value: "geometry".into(), start: 21.8, end: 24.8 },
                ],
            },
        ];

        let spliced = splice_repaired_lyrics_segments(&existing, repaired, 11.0, 26.0);
        assert_eq!(spliced.len(), 4);
        assert_eq!(spliced[0].id, "cue-1");
        assert_eq!(spliced[0].primary, "intro line untouched");
        assert_eq!(spliced[1].primary, "deep and steep");
        assert_eq!(spliced[2].primary, "a silent geometry");
        assert_eq!(spliced[3].id, "cue-3");
        assert_eq!(spliced[3].primary, "outro line untouched");
    }
}
