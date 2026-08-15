//! Durable offline music production built on the user's loopback-only ComfyUI.
//!
//! The music workspace owns producer-authored structure and immutable generated takes. MiniMax
//! Music 3 produces a stereo master, not stems, so this module deliberately does not invent
//! per-instrument media. Optional MuScriptor transcription is an explicit, fixed-argument
//! audio-to-MIDI export using a producer-supplied executable and locally accepted checkpoint.

use super::{
    comfy_execution_error, find_output_media, truncate, MovieStudio, StudioError, MUSIC_COMFY_BASE,
};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const MUSIC_SCHEMA_VERSION: u32 = 1;
const MAX_MUSIC_TEXT_BYTES: usize = 64 * 1024;
const MAX_SECTIONS: usize = 64;
const MAX_TAKES: usize = 128;
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
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiRequest {
    pub project_id: String,
    pub take_id: String,
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
            midi: MusicMidiSettings::default(),
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

    pub async fn transcribe_midi(
        &self,
        request: MusicMidiRequest,
    ) -> Result<MusicProject, StudioError> {
        let mut project = self.get(&request.project_id)?;
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
        let output = self
            .project_dir(&project.id)
            .join("midi")
            .join(format!("{}.mid", request.take_id));
        let mut command = tokio::process::Command::new(&project.midi.executable_path);
        command
            .arg("transcribe")
            .args(["--model", project.midi.model_path.as_str()]);
        if !project.midi.instruments.trim().is_empty() {
            command.args(["--instruments", project.midi.instruments.trim()]);
        }
        command
            .arg(&source)
            .arg("-o")
            .arg(&output)
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
        if !result.status.success() || !output.is_file() {
            return Err(StudioError::Render(format!(
                "MuScriptor transcription failed: {}",
                truncate(&String::from_utf8_lossy(&result.stderr), 1_000)
            )));
        }
        let receipt = json!({
            "schemaVersion": 1,
            "tool": "MuScriptor",
            "createdAt": Utc::now().to_rfc3339(),
            "takeId": request.take_id,
            "sourceSha256": project.takes[take_index].sha256,
            "modelPath": project.midi.model_path,
            "executablePath": project.midi.executable_path,
            "instruments": project.midi.instruments,
            "licenseNotice": "Producer supplied a locally accepted MuScriptor checkpoint. Kestrel does not grant commercial rights to CC-BY-NC weights.",
            "stdout": truncate(&String::from_utf8_lossy(&result.stdout), 8_000),
            "stderr": truncate(&String::from_utf8_lossy(&result.stderr), 8_000),
        });
        let receipt_path = output.with_extension("receipt.json");
        write_json_recoverable(&receipt_path, &receipt)?;
        project.takes[take_index].midi_path = output.to_string_lossy().into_owned();
        project.takes[take_index].midi_receipt_path = receipt_path.to_string_lossy().into_owned();
        project.phase = "midi-ready".into();
        project.detail = "MuScriptor created an editable MIDI interpretation. The generated master is unchanged.".into();
        project.updated_at = Utc::now().to_rfc3339();
        self.persist(&project)?;
        Ok(project)
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
            "choose the local muscriptor executable before transcribing to MIDI".into(),
        ));
    }
    if !model.is_absolute() || !model.is_file() {
        return Err(StudioError::Invalid(
            "choose a locally accepted MuScriptor .safetensors checkpoint before transcribing"
                .into(),
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

fn hash_file(path: &Path) -> Result<(u64, String), StudioError> {
    let mut input = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        bytes = bytes.saturating_add(count as u64);
    }
    Ok((bytes, hex::encode(hasher.finalize())))
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
    use tempfile::TempDir;

    fn project(studio: &MusicStudio) -> MusicProject {
        let mut project = studio
            .create(CreateMusicProjectRequest {
                title: "Night signal".into(),
                idea: "Warm analog synth-pop at night".into(),
                comfy_root: r"D:\AI\ComfyUI".into(),
            })
            .unwrap();
        project.caption = "Global Metadata: synth-pop, 112 BPM.\n\nVocal Details: intimate alto.\n\nArrangement: analog drums and wide pads.".into();
        project.sections[1].lyrics = "The streetlights answer me".into();
        studio.save_editable(project).unwrap()
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
}
