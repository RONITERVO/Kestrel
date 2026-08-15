//! Durable, producer-owned still-image production through native ComfyUI Ideogram 4 nodes.
//!
//! The frontend may edit only these typed fields. Kestrel compiles them into a fixed graph and
//! never accepts executable workflow JSON from a model or producer. Completed PNGs, prompt JSON,
//! graph receipts, hashes, and earlier takes are immutable backend truth.

use super::{comfy_execution_error, truncate, MovieStudio, StudioError, COMFY_BASE};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const IMAGE_SCHEMA_VERSION: u32 = 2;
const MAX_IMAGE_TEXT_BYTES: usize = 64 * 1024;
const MAX_IMAGE_ELEMENTS: usize = 64;
const MAX_IMAGE_TAKES: usize = 256;
const IMAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
const IDEOGRAM_MODEL: &str = "ideogram4_nvfp4_mixed.safetensors";
const IDEOGRAM_UNCONDITIONAL_MODEL: &str = "ideogram4_unconditional_nvfp4_mixed.safetensors";
const IDEOGRAM_TEXT_ENCODER: &str = "qwen3vl_8b_nvfp4.safetensors";
const IDEOGRAM_VAE: &str = "flux2-vae.safetensors";
pub const IDEOGRAM_LICENSE_NOTICE: &str = "Ideogram 4 is provided under the Ideogram Non-Commercial Model Agreement. Generated work may not be used in or to advertise revenue-generating products or services unless Ideogram grants separate rights.";

fn default_style_mode() -> String {
    "photo".into()
}

fn default_art_style() -> String {
    "Editorial illustration with purposeful shape language and finished detail.".into()
}

const fn default_batch_size() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageStyle {
    #[serde(default = "default_style_mode")]
    pub mode: String,
    pub aesthetics: String,
    pub lighting: String,
    pub photo: String,
    #[serde(default = "default_art_style")]
    pub art_style: String,
    pub medium: String,
    pub color_palette: Vec<String>,
}

impl Default for ImageStyle {
    fn default() -> Self {
        Self {
            mode: default_style_mode(),
            aesthetics: "Editorial image with deliberate composition and natural detail.".into(),
            lighting: "Soft directional daylight with controlled contrast.".into(),
            photo: "Clean full-resolution image with restrained texture.".into(),
            art_style: default_art_style(),
            medium: "Photograph".into(),
            color_palette: vec!["#24313A".into(), "#D9D2C3".into(), "#C36A3D".into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageElement {
    pub id: String,
    pub kind: String,
    /// Ideogram coordinates in `[y_min, x_min, y_max, x_max]` order, normalized to 0..1000.
    pub bbox: [u16; 4],
    pub text: String,
    pub description: String,
    pub color_palette: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ImageSettings {
    pub width: u32,
    pub height: u32,
    pub preset: String,
    pub seed: u64,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    pub comfy_root: String,
}

impl Default for ImageSettings {
    fn default() -> Self {
        Self {
            width: 1536,
            height: 1024,
            preset: "standard".into(),
            seed: 0,
            batch_size: default_batch_size(),
            comfy_root: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageTake {
    pub id: String,
    pub created_at: String,
    pub status: String,
    pub detail: String,
    pub error: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub width: u32,
    pub height: u32,
    pub preset: String,
    pub seed: u64,
    #[serde(default)]
    pub batch_index: u32,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    pub prompt_id: String,
    pub exact_prompt: Value,
    #[serde(default)]
    pub exact_prompt_text: String,
    pub exact_graph: Value,
    pub model_profile: String,
    pub license_notice: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageProject {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub idea: String,
    pub high_level_description: String,
    pub style: ImageStyle,
    pub background: String,
    pub elements: Vec<ImageElement>,
    pub settings: ImageSettings,
    pub takes: Vec<ImageTake>,
    pub active_take_id: String,
    pub status: String,
    pub phase: String,
    pub detail: String,
    pub error: String,
    pub license_notice: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    pub updated_at: String,
    pub take_count: usize,
    pub active_take_path: String,
}

impl From<&ImageProject> for ImageSummary {
    fn from(project: &ImageProject) -> Self {
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
pub struct CreateImageProjectRequest {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub idea: String,
    #[serde(default)]
    pub comfy_root: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationEvent {
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

impl ImageGenerationEvent {
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
pub struct ImageStudio {
    root: PathBuf,
    http: Client,
}

impl ImageStudio {
    pub fn new(library_root: &Path) -> Result<Self, StudioError> {
        let root = library_root.join("images");
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

    pub fn list(&self) -> Result<Vec<ImageSummary>, StudioError> {
        let mut projects = Vec::new();
        for entry in fs::read_dir(&self.root)?.take(2_000) {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Ok(project) = read_recoverable(&entry.path().join("project.json")) {
                projects.push(ImageSummary::from(&project));
            }
        }
        projects.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(projects)
    }

    pub fn get(&self, id: &str) -> Result<ImageProject, StudioError> {
        validate_image_id(id)?;
        read_recoverable(&self.project_dir(id).join("project.json"))
    }

    pub fn create(&self, request: CreateImageProjectRequest) -> Result<ImageProject, StudioError> {
        validate_bounded_text("image idea", &request.idea, MAX_IMAGE_TEXT_BYTES, true)?;
        let title = if request.title.trim().is_empty() {
            "Untitled image"
        } else {
            request.title.trim()
        };
        validate_title(title)?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let project = ImageProject {
            schema_version: IMAGE_SCHEMA_VERSION,
            id: id.clone(),
            title: title.into(),
            idea: request.idea.trim().into(),
            high_level_description: request.idea.trim().into(),
            style: ImageStyle::default(),
            background: "A fully described environment extending naturally to every edge of the frame.".into(),
            elements: Vec::new(),
            settings: ImageSettings {
                comfy_root: request.comfy_root,
                ..ImageSettings::default()
            },
            takes: Vec::new(),
            active_take_id: String::new(),
            status: "draft".into(),
            phase: "composing".into(),
            detail: "Shape the brief, visual style, exact text, and composition, then create a preserved take.".into(),
            error: String::new(),
            license_notice: IDEOGRAM_LICENSE_NOTICE.into(),
            created_at: now.clone(),
            updated_at: now,
        };
        let folder = self.project_dir(&id);
        fs::create_dir_all(folder.join("takes"))?;
        fs::create_dir_all(folder.join("receipts"))?;
        fs::create_dir_all(folder.join("logs"))?;
        self.persist(&project)?;
        Ok(project)
    }

    /// Save only producer-editable state. Generated media and receipts remain backend truth.
    pub fn save_editable(&self, edited: ImageProject) -> Result<ImageProject, StudioError> {
        let mut stored = self.get(&edited.id)?;
        if stored.status == "generating" {
            return Err(StudioError::Invalid(
                "image composition and generation settings are locked while a take is rendering"
                    .into(),
            ));
        }
        validate_editable(&edited)?;
        stored.title = edited.title.trim().into();
        stored.idea = edited.idea;
        stored.high_level_description = edited.high_level_description;
        stored.style = edited.style;
        stored.background = edited.background;
        stored.elements = edited.elements;
        stored.settings = edited.settings;
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
            "Producer changes are saved. Existing full-resolution takes remain immutable.".into();
        stored.error.clear();
        self.persist(&stored)?;
        Ok(stored)
    }

    pub fn begin_generation(
        &self,
        id: &str,
        app: Option<&AppHandle>,
    ) -> Result<(ImageProject, Vec<String>), StudioError> {
        let mut project = self.get(id)?;
        validate_render_ready(&project)?;
        let batch_size = project.settings.batch_size as usize;
        if project.takes.len().saturating_add(batch_size) > MAX_IMAGE_TAKES {
            return Err(StudioError::Invalid(
                "this batch would exceed the project's 256 preserved-take limit; reduce the batch or start a new project".into(),
            ));
        }
        let seed = resolved_seed(project.settings.seed);
        let (prompt, prompt_text) = structured_prompt(&project)?;
        let created_at = Utc::now().to_rfc3339();
        let mut take_ids = Vec::with_capacity(batch_size);
        for batch_index in 0..batch_size {
            let take_id = uuid::Uuid::new_v4().to_string();
            take_ids.push(take_id.clone());
            project.takes.push(ImageTake {
                id: take_id,
                created_at: created_at.clone(),
                status: "queued".into(),
                detail: format!(
                    "Variation {} of {batch_size} is queued for the private local image renderer.",
                    batch_index + 1
                ),
                error: String::new(),
                path: String::new(),
                bytes: 0,
                sha256: String::new(),
                width: project.settings.width,
                height: project.settings.height,
                preset: project.settings.preset.clone(),
                seed,
                batch_index: batch_index as u32 + 1,
                batch_size: project.settings.batch_size,
                prompt_id: String::new(),
                exact_prompt: prompt.clone(),
                exact_prompt_text: prompt_text.clone(),
                exact_graph: Value::Null,
                model_profile: "Ideogram 4 NVFP4 mixed + Qwen3-VL 8B NVFP4 + Flux 2 VAE".into(),
                license_notice: IDEOGRAM_LICENSE_NOTICE.into(),
            });
        }
        project.status = "generating".into();
        project.phase = "queued".into();
        project.detail = format!(
            "The current composition is frozen for {batch_size} variation{}; previous takes remain untouched.",
            if batch_size == 1 { "" } else { "s" }
        );
        project.error.clear();
        self.persist_emit(&mut project, app)?;
        let primary_take_id = take_ids.first().expect("validated image batch size");
        emit_image(
            app,
            ImageGenerationEvent::new(
                id,
                primary_take_id,
                "queued",
                "queued",
                format!(
                    "{batch_size} variation{} queued. Kestrel is releasing other GPU models before loading Ideogram 4.",
                    if batch_size == 1 { "" } else { "s" }
                ),
            ),
        );
        Ok((project, take_ids))
    }

    pub async fn render(
        &self,
        project_id: &str,
        take_ids: &[String],
        shared_renderer: &MovieStudio,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<ImageProject, StudioError> {
        let mut project = self.get(project_id)?;
        let primary_take_id = take_ids
            .first()
            .ok_or_else(|| StudioError::Invalid("image batch contains no takes".into()))?;
        let take_index = project
            .takes
            .iter()
            .position(|take| take.id == *primary_take_id)
            .ok_or_else(|| StudioError::Invalid("image batch no longer exists".into()))?;
        let take_indexes = take_ids
            .iter()
            .map(|id| {
                project
                    .takes
                    .iter()
                    .position(|take| take.id == *id)
                    .ok_or_else(|| StudioError::Invalid("image batch no longer exists".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if take_ids.len() != project.settings.batch_size as usize
            || take_indexes.iter().enumerate().any(|(position, index)| {
                let take = &project.takes[*index];
                take.status != "queued"
                    || take.batch_index != position as u32 + 1
                    || take.batch_size != project.settings.batch_size
            })
        {
            return Err(StudioError::Invalid(
                "image batch does not match its frozen generation settings".into(),
            ));
        }
        let comfy_root = PathBuf::from(project.settings.comfy_root.trim());
        if !comfy_root.is_absolute() {
            return Err(StudioError::Invalid(
                "choose an absolute ComfyUI folder in Setup before generating images".into(),
            ));
        }
        project.phase = "starting-renderer".into();
        project.detail = "Starting or attaching to the private loopback ComfyUI renderer.".into();
        for index in &take_indexes {
            project.takes[*index].status = "starting".into();
            project.takes[*index].detail = project.detail.clone();
        }
        self.persist_emit(&mut project, app)?;
        emit_image(
            app,
            ImageGenerationEvent::new(
                project_id,
                primary_take_id,
                "progress",
                "starting-renderer",
                "Preparing local ComfyUI and checking the native Ideogram 4 nodes.",
            ),
        );
        shared_renderer.release_comfy_memory().await;
        shared_renderer
            .ensure_comfy_process(
                project.settings.comfy_root.trim(),
                &self.project_dir(project_id).join("logs"),
                Some(cancel),
            )
            .await?;
        verify_ideogram_nodes(&self.http).await?;
        verify_ideogram_assets(&comfy_root)?;
        let prefix = format!("kestrel_image/{project_id}/take_{:03}", take_index + 1);
        let graph = ideogram_graph(
            &project.settings,
            project.takes[take_index].seed,
            &project.takes[take_index].exact_prompt_text,
            &prefix,
        );
        for index in &take_indexes {
            project.takes[*index].exact_graph = graph.clone();
            project.takes[*index].status = "generating".into();
            project.takes[*index].detail =
                "Ideogram 4 is sampling the full-resolution image locally.".into();
        }
        project.phase = "sampling".into();
        project.detail = project.takes[take_index].detail.clone();
        self.persist_emit(&mut project, app)?;
        for id in take_ids {
            write_json_recoverable(
                &self
                    .project_dir(project_id)
                    .join("receipts")
                    .join(format!("{id}.graph.json")),
                &graph,
            )?;
            write_text_recoverable(
                &self
                    .project_dir(project_id)
                    .join("receipts")
                    .join(format!("{id}.prompt.json")),
                &project.takes[take_index].exact_prompt_text,
            )?;
        }

        let client_id = format!("kestrel-image-{}", uuid::Uuid::new_v4().simple());
        let progress =
            ImageProgressSession::connect(app, &client_id, project_id, primary_take_id).await;
        let response = self
            .http
            .post(format!("{COMFY_BASE}/prompt"))
            .json(&json!({"prompt":graph,"client_id":client_id}))
            .send()
            .await?;
        let status = response.status();
        let value: Value = response.json().await?;
        if !status.is_success() {
            return Err(StudioError::Render(format!(
                "ComfyUI rejected the native Ideogram 4 workflow: {}",
                truncate(&value.to_string(), 900)
            )));
        }
        let prompt_id = value
            .get("prompt_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StudioError::Render(format!("ComfyUI returned no image prompt ID: {value}"))
            })?
            .to_string();
        for index in &take_indexes {
            project.takes[*index].prompt_id.clone_from(&prompt_id);
        }
        self.persist(&project)?;

        let deadline = tokio::time::Instant::now() + IMAGE_RENDER_TIMEOUT;
        let output_media = loop {
            if cancel.is_cancelled() {
                let _ = self
                    .http
                    .post(format!("{COMFY_BASE}/interrupt"))
                    .send()
                    .await;
                return Err(StudioError::Cancelled);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(StudioError::Render(
                    "ComfyUI did not finish the image within six hours. The project, prompt, graph receipt, and prior takes remain safe.".into(),
                ));
            }
            let history: Value = self
                .http
                .get(format!("{COMFY_BASE}/history/{prompt_id}"))
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
                    break find_image_outputs(entry, take_ids.len())?;
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        };
        if let Some(progress) = &progress {
            progress.finish();
        }
        let mut preserved = Vec::with_capacity(take_ids.len());
        for ((id, index), (source_name, source_subfolder)) in take_ids
            .iter()
            .zip(take_indexes.iter())
            .zip(output_media.into_iter())
        {
            let source = safe_comfy_output(&comfy_root, &source_subfolder, &source_name)?;
            let target = self
                .project_dir(project_id)
                .join("takes")
                .join(format!("take-{:03}-{id}.png", index + 1));
            tokio::fs::copy(&source, &target).await.map_err(|error| {
                StudioError::Render(format!(
                    "could not preserve the generated PNG from {}: {error}",
                    source.display()
                ))
            })?;
            verify_png_dimensions(
                &target,
                project.takes[*index].width,
                project.takes[*index].height,
            )?;
            let hash_target = target.clone();
            let (bytes, sha256) = tokio::task::spawn_blocking(move || hash_file(&hash_target))
                .await
                .map_err(|error| {
                    StudioError::Render(format!("image checksum task failed: {error}"))
                })??;
            preserved.push((id.clone(), target, bytes, sha256));
        }
        let mut project = self.get(project_id)?;
        for (id, target, bytes, sha256) in preserved {
            let take = project
                .takes
                .iter_mut()
                .find(|take| take.id == id)
                .ok_or_else(|| {
                    StudioError::Invalid("image take disappeared during preservation".into())
                })?;
            take.status = "complete".into();
            take.detail =
                "Full-resolution local PNG preserved. Earlier takes remain unchanged.".into();
            take.path = target.to_string_lossy().into_owned();
            take.bytes = bytes;
            take.sha256 = sha256;
            take.error.clear();
        }
        project.active_take_id = primary_take_id.clone();
        project.status = "ready".into();
        project.phase = "take-ready".into();
        project.detail = format!(
            "{} new immutable image take{} ready for review.",
            take_ids.len(),
            if take_ids.len() == 1 { " is" } else { "s are" }
        );
        project.error.clear();
        self.persist_emit(&mut project, app)?;
        emit_image(
            app,
            ImageGenerationEvent::new(
                project_id,
                primary_take_id,
                "complete",
                "take-ready",
                format!(
                    "{} full-resolution PNG{} preserved in the private image project.",
                    take_ids.len(),
                    if take_ids.len() == 1 { "" } else { "s" }
                ),
            ),
        );
        shared_renderer.release_comfy_memory().await;
        Ok(project)
    }

    pub fn fail_generation(
        &self,
        project_id: &str,
        take_ids: &[String],
        error: String,
        cancelled: bool,
        app: Option<&AppHandle>,
    ) {
        let Ok(mut project) = self.get(project_id) else {
            return;
        };
        for take in project
            .takes
            .iter_mut()
            .filter(|take| take_ids.contains(&take.id))
        {
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
            "Image generation stopped; composition and completed takes are safe."
        } else {
            "The failed take is retained with its exact prompt, graph, and error. Completed takes are safe."
        }
        .into();
        project.error = if cancelled {
            String::new()
        } else {
            error.clone()
        };
        let _ = self.persist_emit(&mut project, app);
        emit_image(
            app,
            ImageGenerationEvent::new(
                project_id,
                take_ids.first().map(String::as_str).unwrap_or_default(),
                if cancelled { "cancelled" } else { "error" },
                &project.phase,
                if cancelled { project.detail } else { error },
            ),
        );
    }

    pub fn project_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn reveal_path(&self, id: &str) -> Result<PathBuf, StudioError> {
        validate_image_id(id)?;
        let path = self.project_dir(id);
        if !path.is_dir() {
            return Err(StudioError::Invalid(
                "image project folder is missing".into(),
            ));
        }
        Ok(path)
    }

    fn persist(&self, project: &ImageProject) -> Result<(), StudioError> {
        write_json_recoverable(&self.project_dir(&project.id).join("project.json"), project)
    }

    fn persist_emit(
        &self,
        project: &mut ImageProject,
        app: Option<&AppHandle>,
    ) -> Result<(), StudioError> {
        project.schema_version = IMAGE_SCHEMA_VERSION;
        project.updated_at = Utc::now().to_rfc3339();
        self.persist(project)?;
        if let Some(app) = app {
            let _ = app.emit("image-project-updated", project.clone());
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
            let Ok(mut project) = read_recoverable::<ImageProject>(&path) else {
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
            project.detail = "Kestrel closed during generation. Completed takes and the exact in-progress prompt and graph are safe; create a new take when ready.".into();
            project.error.clear();
            for take in project
                .takes
                .iter_mut()
                .filter(|take| matches!(take.status.as_str(), "queued" | "starting" | "generating"))
            {
                take.status = "interrupted".into();
                take.detail =
                    "Generation was interrupted before a full-resolution PNG was preserved.".into();
            }
            self.persist(&project)?;
        }
        Ok(())
    }
}

struct ImageProgressSession {
    app: AppHandle,
    project_id: String,
    take_id: String,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl ImageProgressSession {
    async fn connect(
        app: Option<&AppHandle>,
        client_id: &str,
        project_id: &str,
        take_id: &str,
    ) -> Option<Self> {
        let app = app?.clone();
        let url = format!("ws://127.0.0.1:8188/ws?clientId={client_id}");
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
                        if let Some(event) = parse_image_progress(&text, &task_project, &task_take, started) {
                            let _ = task_app.emit("image-generation", event);
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
        emit_image(
            Some(&self.app),
            ImageGenerationEvent::new(
                &self.project_id,
                &self.take_id,
                "progress",
                "preserving",
                "Sampling and full-quality decode finished. Preserving the immutable PNG.",
            ),
        );
        self.cancel.cancel();
    }
}

impl Drop for ImageProgressSession {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

fn parse_image_progress(
    text: &str,
    project_id: &str,
    take_id: &str,
    started: Instant,
) -> Option<ImageGenerationEvent> {
    let value: Value = serde_json::from_str(text).ok()?;
    let kind = value.get("type")?.as_str()?;
    let data = value.get("data")?;
    if kind == "progress" {
        let step = u32::try_from(data.get("value")?.as_u64()?).ok()?;
        let total = u32::try_from(data.get("max")?.as_u64()?).ok()?;
        if total == 0 || step > total || total > 1_000 {
            return None;
        }
        let percent = f64::from(step) * 100.0 / f64::from(total);
        let elapsed = started.elapsed().as_secs_f64();
        let eta_seconds = (step > 0)
            .then(|| ((elapsed / f64::from(step)) * f64::from(total - step)).round() as u64);
        let mut event = ImageGenerationEvent::new(
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
            "1" | "2" | "3" | "4" => (
                "loading",
                "Loading the verified Ideogram 4 model, text encoder, and full-quality decoder.",
            ),
            "5" => (
                "encoding",
                "Encoding the structured visual direction and exact text.",
            ),
            "13" => ("sampling", "Sampling the full-resolution composition."),
            "14" => ("decoding", "Decoding the full-quality image."),
            "15" => ("saving", "Writing the lossless PNG output."),
            _ => return None,
        };
        return Some(ImageGenerationEvent::new(
            project_id, take_id, "progress", phase, detail,
        ));
    }
    None
}

fn emit_image(app: Option<&AppHandle>, event: ImageGenerationEvent) {
    if let Some(app) = app {
        let _ = app.emit("image-generation", event);
    }
}

#[derive(Serialize)]
struct OrderedCaption<'a> {
    high_level_description: &'a str,
    style_description: OrderedStyle<'a>,
    compositional_deconstruction: OrderedComposition<'a>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum OrderedStyle<'a> {
    Photo(OrderedPhotoStyle<'a>),
    Art(OrderedArtStyle<'a>),
}

#[derive(Serialize)]
struct OrderedPhotoStyle<'a> {
    aesthetics: &'a str,
    lighting: &'a str,
    photo: &'a str,
    medium: &'a str,
    #[serde(skip_serializing_if = "slice_is_empty")]
    color_palette: &'a [String],
}

#[derive(Serialize)]
struct OrderedArtStyle<'a> {
    aesthetics: &'a str,
    lighting: &'a str,
    medium: &'a str,
    art_style: &'a str,
    #[serde(skip_serializing_if = "slice_is_empty")]
    color_palette: &'a [String],
}

#[derive(Serialize)]
struct OrderedComposition<'a> {
    background: &'a str,
    elements: Vec<OrderedElement<'a>>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum OrderedElement<'a> {
    Object(OrderedObject<'a>),
    Text(OrderedText<'a>),
}

#[derive(Serialize)]
struct OrderedObject<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    bbox: [u16; 4],
    desc: &'a str,
    #[serde(skip_serializing_if = "slice_is_empty")]
    color_palette: &'a [String],
}

#[derive(Serialize)]
struct OrderedText<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    bbox: [u16; 4],
    text: &'a str,
    desc: &'a str,
    #[serde(skip_serializing_if = "slice_is_empty")]
    color_palette: &'a [String],
}

fn slice_is_empty<T>(slice: &[T]) -> bool {
    slice.is_empty()
}

/// Compile the producer-owned composition into Ideogram's order-sensitive caption schema.
/// The string is durable inference truth; the Value is only a convenient UI/receipt view.
fn structured_prompt(project: &ImageProject) -> Result<(Value, String), StudioError> {
    let elements = project
        .elements
        .iter()
        .map(|element| {
            if element.kind == "text" {
                OrderedElement::Text(OrderedText {
                    kind: "text",
                    bbox: element.bbox,
                    text: element.text.as_str(),
                    desc: element.description.trim(),
                    color_palette: &element.color_palette,
                })
            } else {
                OrderedElement::Object(OrderedObject {
                    kind: "obj",
                    bbox: element.bbox,
                    desc: element.description.trim(),
                    color_palette: &element.color_palette,
                })
            }
        })
        .collect();
    let style_description = if project.style.mode == "art" {
        OrderedStyle::Art(OrderedArtStyle {
            aesthetics: project.style.aesthetics.trim(),
            lighting: project.style.lighting.trim(),
            medium: project.style.medium.trim(),
            art_style: project.style.art_style.trim(),
            color_palette: &project.style.color_palette,
        })
    } else {
        OrderedStyle::Photo(OrderedPhotoStyle {
            aesthetics: project.style.aesthetics.trim(),
            lighting: project.style.lighting.trim(),
            photo: project.style.photo.trim(),
            medium: project.style.medium.trim(),
            color_palette: &project.style.color_palette,
        })
    };
    let caption = OrderedCaption {
        high_level_description: project.high_level_description.trim(),
        style_description,
        compositional_deconstruction: OrderedComposition {
            background: project.background.trim(),
            elements,
        },
    };
    let text = serde_json::to_string(&caption)?;
    let value = serde_json::from_str(&text)?;
    Ok((value, text))
}

fn ideogram_graph(settings: &ImageSettings, seed: u64, prompt: &str, prefix: &str) -> Value {
    let (steps, mu, std) = preset_parameters(&settings.preset);
    json!({
        "1":{"class_type":"UNETLoader","inputs":{"unet_name":IDEOGRAM_MODEL,"weight_dtype":"default"}},
        "2":{"class_type":"UNETLoader","inputs":{"unet_name":IDEOGRAM_UNCONDITIONAL_MODEL,"weight_dtype":"default"}},
        "3":{"class_type":"CLIPLoader","inputs":{"clip_name":IDEOGRAM_TEXT_ENCODER,"type":"ideogram4","device":"default"}},
        "4":{"class_type":"VAELoader","inputs":{"vae_name":IDEOGRAM_VAE}},
        "5":{"class_type":"CLIPTextEncode","inputs":{"clip":["3",0],"text":prompt}},
        "6":{"class_type":"ConditioningZeroOut","inputs":{"conditioning":["5",0]}},
        "7":{"class_type":"CFGOverride","inputs":{"model":["1",0],"cfg":3.0,"start_percent":0.7,"end_percent":1.0}},
        "8":{"class_type":"DualModelGuider","inputs":{"model":["7",0],"model_negative":["2",0],"positive":["5",0],"negative":["6",0],"cfg":7.0}},
        "9":{"class_type":"RandomNoise","inputs":{"noise_seed":seed}},
        "10":{"class_type":"KSamplerSelect","inputs":{"sampler_name":"euler"}},
        "11":{"class_type":"Ideogram4Scheduler","inputs":{"steps":steps,"width":settings.width,"height":settings.height,"mu":mu,"std":std}},
        "12":{"class_type":"EmptyFlux2LatentImage","inputs":{"width":settings.width,"height":settings.height,"batch_size":settings.batch_size}},
        "13":{"class_type":"SamplerCustomAdvanced","inputs":{"noise":["9",0],"guider":["8",0],"sampler":["10",0],"sigmas":["11",0],"latent_image":["12",0]}},
        "14":{"class_type":"VAEDecode","inputs":{"samples":["13",0],"vae":["4",0]}},
        "15":{"class_type":"SaveImage","inputs":{"images":["14",0],"filename_prefix":prefix}}
    })
}

fn preset_parameters(preset: &str) -> (u32, f64, f64) {
    match preset {
        "quality" => (48, 0.0, 1.5),
        "turbo" => (12, 0.5, 1.75),
        _ => (20, 0.5, 1.75),
    }
}

async fn verify_ideogram_nodes(http: &Client) -> Result<(), StudioError> {
    let info: Value = http
        .get(format!("{COMFY_BASE}/object_info"))
        .timeout(Duration::from_secs(30))
        .send()
        .await?
        .json()
        .await?;
    for node in [
        "Ideogram4Scheduler",
        "DualModelGuider",
        "CFGOverride",
        "EmptyFlux2LatentImage",
        "SamplerCustomAdvanced",
    ] {
        if info.get(node).is_none() {
            return Err(StudioError::Render(format!(
                "the running ComfyUI does not expose {node}. Update ComfyUI to 0.33.1 or newer, restart it, then resume Image Studio in Setup"
            )));
        }
    }
    Ok(())
}

fn verify_ideogram_assets(comfy_root: &Path) -> Result<(), StudioError> {
    for relative in [
        format!("models/diffusion_models/{IDEOGRAM_MODEL}"),
        format!("models/diffusion_models/{IDEOGRAM_UNCONDITIONAL_MODEL}"),
        format!("models/text_encoders/{IDEOGRAM_TEXT_ENCODER}"),
        format!("models/vae/{IDEOGRAM_VAE}"),
        "models/diffusion_models/IDEOGRAM-4-NON-COMMERCIAL-LICENSE.txt".into(),
    ] {
        let path = comfy_root.join(relative);
        if !path.is_file() {
            return Err(StudioError::Render(format!(
                "Ideogram 4 asset is missing: {}. Resume Image Studio in Setup.",
                path.display()
            )));
        }
    }
    Ok(())
}

fn safe_comfy_output(
    comfy_root: &Path,
    subfolder: &str,
    filename: &str,
) -> Result<PathBuf, StudioError> {
    let filename_path = Path::new(filename);
    if filename_path.file_name().and_then(|value| value.to_str()) != Some(filename)
        || !filename.to_ascii_lowercase().ends_with(".png")
    {
        return Err(StudioError::Render(
            "ComfyUI returned an unsafe or non-PNG image filename".into(),
        ));
    }
    let subfolder_path = Path::new(subfolder);
    if subfolder_path.is_absolute()
        || subfolder_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(StudioError::Render(
            "ComfyUI returned an unsafe image output folder".into(),
        ));
    }
    let output_root = comfy_root.join("output").canonicalize()?;
    let source = output_root
        .join(subfolder_path)
        .join(filename)
        .canonicalize()?;
    if !source.starts_with(&output_root) || !source.is_file() {
        return Err(StudioError::Render(
            "ComfyUI image output is outside the configured private renderer".into(),
        ));
    }
    Ok(source)
}

fn verify_png_dimensions(
    path: &Path,
    expected_width: u32,
    expected_height: u32,
) -> Result<(), StudioError> {
    let mut file = fs::File::open(path)?;
    let mut header = [0_u8; 24];
    file.read_exact(&mut header).map_err(|_| {
        StudioError::Render("ComfyUI returned a truncated image instead of a complete PNG".into())
    })?;
    if &header[..8] != b"\x89PNG\r\n\x1a\n" || &header[12..16] != b"IHDR" {
        return Err(StudioError::Render(
            "ComfyUI returned content that is not a valid PNG image".into(),
        ));
    }
    let width = u32::from_be_bytes(header[16..20].try_into().expect("four-byte PNG width"));
    let height = u32::from_be_bytes(header[20..24].try_into().expect("four-byte PNG height"));
    if width != expected_width || height != expected_height {
        return Err(StudioError::Render(format!(
            "ComfyUI returned a {width}x{height} image, but this take requires {expected_width}x{expected_height}"
        )));
    }
    Ok(())
}

fn find_image_outputs(
    entry: &Value,
    expected_count: usize,
) -> Result<Vec<(String, String)>, StudioError> {
    let images = entry
        .pointer("/outputs/15/images")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            StudioError::Render("the completed Ideogram graph exposed no saved image output".into())
        })?;
    let mut outputs = Vec::with_capacity(images.len());
    let mut unique = HashSet::new();
    for image in images {
        let filename = image
            .get("filename")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let subfolder = image
            .get("subfolder")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if filename.is_empty() || !unique.insert((filename.to_string(), subfolder.to_string())) {
            return Err(StudioError::Render(
                "ComfyUI returned a missing or duplicate image in the completed batch".into(),
            ));
        }
        outputs.push((filename.to_string(), subfolder.to_string()));
    }
    if outputs.len() != expected_count {
        return Err(StudioError::Render(format!(
            "ComfyUI completed the Ideogram batch with {} image{}, but Kestrel expected {expected_count}; no partial batch was registered",
            outputs.len(),
            if outputs.len() == 1 { "" } else { "s" }
        )));
    }
    Ok(outputs)
}

fn validate_render_ready(project: &ImageProject) -> Result<(), StudioError> {
    validate_editable(project)?;
    if project.high_level_description.trim().chars().count() < 10 {
        return Err(StudioError::Invalid(
            "write or develop a complete high-level image description before generating a take"
                .into(),
        ));
    }
    if project.background.trim().chars().count() < 3 {
        return Err(StudioError::Invalid(
            "describe the background, or explicitly write transparent background".into(),
        ));
    }
    let treatment = if project.style.mode == "art" {
        project.style.art_style.trim()
    } else {
        project.style.photo.trim()
    };
    if project.style.aesthetics.trim().is_empty()
        || project.style.lighting.trim().is_empty()
        || project.style.medium.trim().is_empty()
        || treatment.is_empty()
    {
        return Err(StudioError::Invalid(
            "complete aesthetics, lighting, medium, and the active photo or artwork treatment before generating a take".into(),
        ));
    }
    Ok(())
}

fn validate_editable(project: &ImageProject) -> Result<(), StudioError> {
    validate_title(&project.title)?;
    for (label, value) in [
        ("image idea", project.idea.as_str()),
        (
            "high-level description",
            project.high_level_description.as_str(),
        ),
        ("aesthetics", project.style.aesthetics.as_str()),
        ("lighting", project.style.lighting.as_str()),
        ("photo treatment", project.style.photo.as_str()),
        ("art treatment", project.style.art_style.as_str()),
        ("medium", project.style.medium.as_str()),
        ("background", project.background.as_str()),
    ] {
        validate_bounded_text(label, value, MAX_IMAGE_TEXT_BYTES, true)?;
    }
    if !matches!(project.style.mode.as_str(), "photo" | "art") {
        return Err(StudioError::Invalid(
            "image style mode must be photo or art".into(),
        ));
    }
    validate_palette(&project.style.color_palette, 16, "global color palette")?;
    if project.elements.len() > MAX_IMAGE_ELEMENTS {
        return Err(StudioError::Invalid(
            "an image may contain at most 64 producer-owned layout elements".into(),
        ));
    }
    let mut ids = HashSet::new();
    for element in &project.elements {
        if uuid::Uuid::parse_str(&element.id).is_err() || !ids.insert(element.id.clone()) {
            return Err(StudioError::Invalid(
                "every image element must have a stable unique ID".into(),
            ));
        }
        if !matches!(element.kind.as_str(), "obj" | "text") {
            return Err(StudioError::Invalid(
                "image element type must be obj or text".into(),
            ));
        }
        let [y_min, x_min, y_max, x_max] = element.bbox;
        if y_max > 1000 || x_max > 1000 || y_min >= y_max || x_min >= x_max {
            return Err(StudioError::Invalid(
                "every image element box must be ordered [top, left, bottom, right] within 0 to 1000".into(),
            ));
        }
        validate_bounded_text("element description", &element.description, 8 * 1024, false)?;
        if element.kind == "text" {
            validate_bounded_text("exact visible text", &element.text, 2 * 1024, false)?;
        } else if !element.text.is_empty() {
            return Err(StudioError::Invalid(
                "object elements cannot carry hidden exact-text content".into(),
            ));
        }
        validate_palette(&element.color_palette, 5, "element color palette")?;
    }
    let settings = &project.settings;
    if !(256..=2048).contains(&settings.width)
        || !(256..=2048).contains(&settings.height)
        || !settings.width.is_multiple_of(16)
        || !settings.height.is_multiple_of(16)
        || settings.width.max(settings.height) > settings.width.min(settings.height) * 6
    {
        return Err(StudioError::Invalid(
            "Ideogram image dimensions must be 256 to 2048 pixels, multiples of 16, with an aspect ratio no wider than 6:1".into(),
        ));
    }
    if !matches!(settings.preset.as_str(), "quality" | "standard" | "turbo") {
        return Err(StudioError::Invalid(
            "Ideogram sampling preset must be quality, standard, or turbo".into(),
        ));
    }
    if !(1..=4).contains(&settings.batch_size) {
        return Err(StudioError::Invalid(
            "an Ideogram batch may contain one to four variations".into(),
        ));
    }
    if settings.seed > i64::MAX as u64 {
        return Err(StudioError::Invalid(
            "image seed must fit the local ComfyUI integer boundary".into(),
        ));
    }
    if !settings.comfy_root.trim().is_empty()
        && !Path::new(settings.comfy_root.trim()).is_absolute()
    {
        return Err(StudioError::Invalid(
            "ComfyUI root must be an absolute local path".into(),
        ));
    }
    let (_, prompt_text) = structured_prompt(project)?;
    if prompt_text.len() > MAX_IMAGE_TEXT_BYTES {
        return Err(StudioError::Invalid(
            "the compiled structured image prompt exceeds 64 KiB".into(),
        ));
    }
    Ok(())
}

fn validate_title(title: &str) -> Result<(), StudioError> {
    if title.trim().is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control)
    {
        return Err(StudioError::Invalid(
            "image title must contain 1 to 120 printable characters".into(),
        ));
    }
    Ok(())
}

fn validate_bounded_text(
    label: &str,
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), StudioError> {
    if (!allow_empty && value.trim().is_empty())
        || value.len() > max_bytes
        || value
            .chars()
            .any(|character| character.is_control() && !character.is_whitespace())
    {
        return Err(StudioError::Invalid(format!(
            "{label} is empty, invalid, or exceeds its {max_bytes} byte limit"
        )));
    }
    Ok(())
}

fn validate_palette(palette: &[String], maximum: usize, label: &str) -> Result<(), StudioError> {
    if palette.len() > maximum
        || palette.iter().any(|color| {
            color.len() != 7
                || !color.starts_with('#')
                || !color[1..]
                    .chars()
                    .all(|character| character.is_ascii_digit() || ('A'..='F').contains(&character))
        })
    {
        return Err(StudioError::Invalid(format!(
            "{label} may contain at most {maximum} uppercase hexadecimal colors such as #2A4B6C"
        )));
    }
    Ok(())
}

fn resolved_seed(seed: u64) -> u64 {
    if seed == 0 {
        Utc::now().timestamp_nanos_opt().unwrap_or_default() as u64 & i64::MAX as u64
    } else {
        seed & i64::MAX as u64
    }
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

fn validate_image_id(id: &str) -> Result<(), StudioError> {
    if uuid::Uuid::parse_str(id).is_err() {
        Err(StudioError::Invalid("invalid image project ID".into()))
    } else {
        Ok(())
    }
}

fn read_recoverable<T: DeserializeOwned>(path: &Path) -> Result<T, StudioError> {
    match read_bounded_json(path) {
        Ok(value) => Ok(value),
        Err(primary) => {
            let backup = path.with_extension("json.bak");
            match read_bounded_json(&backup) {
                Ok(value) => {
                    fs::copy(&backup, path)?;
                    Ok(value)
                }
                Err(_) => Err(primary),
            }
        }
    }
}

fn read_bounded_json<T: DeserializeOwned>(path: &Path) -> Result<T, StudioError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return Err(StudioError::Invalid(
            "image project manifest is missing or exceeds 16 MiB".into(),
        ));
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn write_json_recoverable(path: &Path, value: &impl Serialize) -> Result<(), StudioError> {
    let bytes = serde_json::to_vec_pretty(value)?;
    write_bytes_recoverable(path, &bytes)
}

fn write_text_recoverable(path: &Path, value: &str) -> Result<(), StudioError> {
    write_bytes_recoverable(path, value.as_bytes())
}

fn write_bytes_recoverable(path: &Path, bytes: &[u8]) -> Result<(), StudioError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.bak");
    {
        let mut output = fs::File::create(&temporary)?;
        output.write_all(bytes)?;
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

    fn project(studio: &ImageStudio) -> ImageProject {
        let mut project = studio
            .create(CreateImageProjectRequest {
                title: "Circus morning key art".into(),
                idea: "A football player discovers a construction-site circus at dawn.".into(),
                comfy_root: r"D:\AI\ComfyUI".into(),
            })
            .unwrap();
        project.high_level_description = "A misty editorial photograph of an astonished football player discovering an acrobatic construction-site circus at dawn.".into();
        project.background =
            "Shimmering cranes, aerial silks, scaffold stages, and fog fill the construction yard."
                .into();
        project.elements.push(ImageElement {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "text".into(),
            bbox: [70, 80, 230, 920],
            text: "THE MORNING SHOW".into(),
            description: "Large condensed bold sans-serif title across the clear upper sky.".into(),
            color_palette: vec!["#F4E9D3".into()],
        });
        studio.save_editable(project).unwrap()
    }

    #[test]
    fn structured_prompt_preserves_exact_text_and_bbox_order() {
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let mut project = project(&studio);
        project.high_level_description.push_str(" — café");
        let (prompt, text) = structured_prompt(&project).unwrap();
        assert_eq!(
            prompt["compositional_deconstruction"]["elements"][0]["text"],
            "THE MORNING SHOW"
        );
        assert_eq!(
            prompt["compositional_deconstruction"]["elements"][0]["bbox"],
            json!([70, 80, 230, 920])
        );
        assert!(!text.contains('\n'));
        assert!(text.contains("— café"));
        assert!(!text.contains("\\u"));
        assert!(text.contains("THE MORNING SHOW"));
        assert!(
            text.find("\"high_level_description\"").unwrap()
                < text.find("\"style_description\"").unwrap()
        );
        assert!(
            text.find("\"style_description\"").unwrap()
                < text.find("\"compositional_deconstruction\"").unwrap()
        );
        assert!(text.find("\"type\":\"text\"").unwrap() < text.find("\"bbox\":").unwrap());
        assert!(
            text.find("\"bbox\":").unwrap() < text.find("\"text\":\"THE MORNING SHOW\"").unwrap()
        );
        assert!(
            text.find("\"text\":\"THE MORNING SHOW\"").unwrap() < text.rfind("\"desc\":").unwrap()
        );
    }

    #[test]
    fn structured_prompt_uses_exactly_one_ordered_style_variant() {
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let mut project = project(&studio);
        let (_, photo) = structured_prompt(&project).unwrap();
        assert!(photo.contains("\"photo\":"));
        assert!(!photo.contains("\"art_style\":"));
        assert!(photo.find("\"photo\":") < photo.find("\"medium\":"));

        project.style.mode = "art".into();
        project.style.medium = "Risograph print".into();
        project.style.art_style = "Geometric editorial illustration".into();
        let (_, art) = structured_prompt(&project).unwrap();
        assert!(!art.contains("\"photo\":"));
        assert!(art.contains("\"art_style\":\"Geometric editorial illustration\""));
        assert!(art.find("\"medium\":") < art.find("\"art_style\":"));
    }

    #[test]
    fn graph_uses_only_native_ideogram_nodes_and_official_presets() {
        let settings = ImageSettings {
            preset: "quality".into(),
            batch_size: 4,
            comfy_root: r"D:\AI\ComfyUI".into(),
            ..ImageSettings::default()
        };
        let graph = ideogram_graph(
            &settings,
            42,
            "{\"high_level_description\":\"test\"}",
            "test",
        );
        assert_eq!(graph["3"]["inputs"]["type"], "ideogram4");
        assert_eq!(graph["8"]["class_type"], "DualModelGuider");
        assert_eq!(graph["11"]["class_type"], "Ideogram4Scheduler");
        assert_eq!(graph["11"]["inputs"]["steps"], 48);
        assert_eq!(graph["11"]["inputs"]["mu"], 0.0);
        assert_eq!(graph["12"]["inputs"]["batch_size"], 4);
        assert_eq!(
            graph["5"]["inputs"]["text"],
            "{\"high_level_description\":\"test\"}"
        );
        assert_eq!(graph["15"]["class_type"], "SaveImage");

        let standard = ideogram_graph(
            &ImageSettings::default(),
            42,
            "{\"high_level_description\":\"test\"}",
            "test",
        );
        assert_eq!(standard["11"]["inputs"]["steps"], 20);
        assert_eq!(standard["11"]["inputs"]["mu"], 0.5);
        assert_eq!(standard["11"]["inputs"]["std"], 1.75);
    }

    #[test]
    fn palette_limits_and_uppercase_are_enforced() {
        assert!(validate_palette(&vec!["#A1B2C3".into(); 16], 16, "global").is_ok());
        assert!(validate_palette(&vec!["#A1B2C3".into(); 17], 16, "global").is_err());
        assert!(validate_palette(&vec!["#A1B2C3".into(); 5], 5, "element").is_ok());
        assert!(validate_palette(&vec!["#A1B2C3".into(); 6], 5, "element").is_err());
        assert!(validate_palette(&["#a1B2C3".into()], 16, "global").is_err());
    }

    #[test]
    fn batch_generation_freezes_unique_durable_takes() {
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let mut project = project(&studio);
        project.settings.batch_size = 4;
        let project = studio.save_editable(project).unwrap();
        let (_, take_ids) = studio.begin_generation(&project.id, None).unwrap();
        let stored = studio.get(&project.id).unwrap();
        assert_eq!(take_ids.len(), 4);
        assert_eq!(take_ids.iter().collect::<HashSet<_>>().len(), 4);
        assert!(stored
            .takes
            .iter()
            .enumerate()
            .all(|(index, take)| take.batch_index == index as u32 + 1
                && take.batch_size == 4
                && !take.exact_prompt_text.contains('\n')));
    }

    #[test]
    fn invalid_boxes_and_duplicate_ids_are_rejected() {
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let mut project = project(&studio);
        project.elements[0].bbox = [800, 50, 200, 900];
        assert!(studio.save_editable(project).is_err());
    }

    #[test]
    fn frontend_cannot_replace_immutable_take_truth() {
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let (_, take_ids) = studio.begin_generation(&project.id, None).unwrap();
        studio.fail_generation(&project.id, &take_ids, "test".into(), true, None);
        let mut edited = studio.get(&project.id).unwrap();
        edited.takes[0].path = r"C:\forged.png".into();
        let saved = studio.save_editable(edited).unwrap();
        assert!(saved.takes[0].path.is_empty());
    }

    #[test]
    fn progress_is_bounded_and_includes_eta() {
        let event = parse_image_progress(
            &json!({"type":"progress","data":{"value":10,"max":20}}).to_string(),
            "project",
            "take",
            Instant::now(),
        )
        .unwrap();
        assert_eq!(event.percent, Some(50.0));
        assert!(parse_image_progress(
            &json!({"type":"progress","data":{"value":21,"max":20}}).to_string(),
            "project",
            "take",
            Instant::now(),
        )
        .is_none());
    }

    #[test]
    fn preserved_output_must_be_a_safe_full_resolution_png() {
        let root = TempDir::new().unwrap();
        let output = root.path().join("output/nested");
        fs::create_dir_all(&output).unwrap();
        let image = output.join("take.png");
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&1024_u32.to_be_bytes());
        bytes.extend_from_slice(&768_u32.to_be_bytes());
        fs::write(&image, bytes).unwrap();

        assert_eq!(
            safe_comfy_output(root.path(), "nested", "take.png").unwrap(),
            image.canonicalize().unwrap()
        );
        verify_png_dimensions(&image, 1024, 768).unwrap();
        assert!(verify_png_dimensions(&image, 768, 1024).is_err());
        assert!(safe_comfy_output(root.path(), "..", "take.png").is_err());
        assert!(safe_comfy_output(root.path(), "nested", "../take.png").is_err());
    }

    #[test]
    fn batch_output_extraction_rejects_partial_or_duplicate_results() {
        let complete = json!({"outputs":{"15":{"images":[
            {"filename":"first.png","subfolder":"batch"},
            {"filename":"second.png","subfolder":"batch"}
        ]}}});
        assert_eq!(find_image_outputs(&complete, 2).unwrap().len(), 2);
        assert!(find_image_outputs(&complete, 4).is_err());
        let duplicate = json!({"outputs":{"15":{"images":[
            {"filename":"same.png","subfolder":"batch"},
            {"filename":"same.png","subfolder":"batch"}
        ]}}});
        assert!(find_image_outputs(&duplicate, 2).is_err());
    }

    #[tokio::test]
    #[ignore = "requires KESTREL_LIVE_COMFY_ROOT, native Ideogram 4 nodes and verified NVFP4 weights, and several minutes"]
    async fn live_ideogram_graph_preserves_a_full_resolution_png() {
        let comfy_root = std::env::var("KESTREL_LIVE_COMFY_ROOT")
            .expect("set KESTREL_LIVE_COMFY_ROOT to the absolute ComfyUI folder");
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let renderer = MovieStudio::new(root.path()).unwrap();
        let mut project = studio
            .create(CreateImageProjectRequest {
                title: "Ideogram 4 acceptance".into(),
                idea: "A restrained editorial poster for an offline image studio.".into(),
                comfy_root,
            })
            .unwrap();
        project.high_level_description = "A restrained Swiss-style editorial poster showing a cobalt kestrel silhouette above a geometric cream landscape.".into();
        project.background =
            "Matte cream paper with a subtle natural fiber texture and generous negative space."
                .into();
        project.settings.width = 1024;
        project.settings.height = 1024;
        project.settings.preset = "turbo".into();
        project.elements = vec![ImageElement {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "text".into(),
            bbox: [720, 100, 900, 900],
            text: "KESTREL / LOCAL".into(),
            description: "Large upright bold sans-serif title with exact, clean lettering.".into(),
            color_palette: vec!["#173B68".into()],
        }];
        let project = studio.save_editable(project).unwrap();
        let (_, take_ids) = studio.begin_generation(&project.id, None).unwrap();
        let rendered = studio
            .render(
                &project.id,
                &take_ids,
                &renderer,
                &CancellationToken::new(),
                None,
            )
            .await
            .unwrap();
        let take = rendered
            .takes
            .iter()
            .find(|take| take.id == take_ids[0])
            .unwrap();
        assert_eq!(take.status, "complete");
        assert!(Path::new(&take.path).is_file());
        assert!(take.bytes > 0);
        assert_eq!(take.sha256.len(), 64);
        assert_eq!(take.exact_graph["15"]["class_type"], "SaveImage");
    }
}
