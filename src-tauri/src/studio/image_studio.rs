//! Durable, producer-owned still-image production through native ComfyUI MOSS-Image nodes.
//!
//! The frontend may edit only these typed fields. Kestrel compiles them into a fixed graph and
//! never accepts executable workflow JSON from a model or producer. Completed PNGs, prompt JSON,
//! graph receipts, hashes, and earlier takes are immutable backend truth.

pub use kestrel_app_core::image::{
    CreateImageProjectRequest, ImageGenerationEvent, ImageProject, ImageSettings, ImageStyle,
    ImageSummary, ImageTake,
};

use super::{
    comfy_execution_error, hash_file, truncate, ComfyWorkload, MovieStudio, StudioError, COMFY_BASE,
};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const IMAGE_SCHEMA_VERSION: u32 = 4;
const MAX_IMAGE_TEXT_BYTES: usize = 64 * 1024;
const MAX_IMAGE_ELEMENTS: usize = 64;
const MAX_IMAGE_TAKES: usize = 256;
const IMAGE_RENDER_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
const MOSS_MODEL: &str = "moss_image_base_int8_convrot.safetensors";
const MOSS_TEXT_ENCODER: &str = "qwen3.5_9b_int8_convrot.safetensors";
const MOSS_VAE: &str = "flux2-vae.safetensors";
pub const MOSS_LICENSE_NOTICE: &str = "MOSS-Image Base is released under Apache 2.0. Earlier takes retain their original model and license receipts.";

pub trait ImageGenerationEventPolicy: Sized {
    fn new(
        project_id: &str,
        take_id: &str,
        kind: &str,
        phase: &str,
        detail: impl Into<String>,
    ) -> Self;
}

impl ImageGenerationEventPolicy for ImageGenerationEvent {
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
    backend: ImageBackend,
    root: PathBuf,
    http: Client,
}

impl ImageStudio {
    pub fn new(library_root: &Path) -> Result<Self, StudioError> {
        let root = library_root.join("images");
        fs::create_dir_all(&root)?;
        let studio = Self {
            backend: ImageBackend::default(),
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
        let mut project: ImageProject =
            read_recoverable(&self.project_dir(id).join("project.json"))?;
        project.schema_version = IMAGE_SCHEMA_VERSION;
        project.license_notice = self.backend.license().into();
        Ok(project)
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
            license_notice: self.backend.license().into(),
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
    pub fn save_editable(
        &self,
        edited: impl Into<kestrel_app_core::ImageProjectEdit>,
    ) -> Result<ImageProject, StudioError> {
        let edited = edited.into();
        let mut stored = self.get(&edited.id)?;
        if stored.status == "generating" {
            return Err(StudioError::Invalid(
                "image composition and generation settings are locked while a take is rendering"
                    .into(),
            ));
        }
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
        validate_editable(&stored)?;
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
        configured_comfy_root: &str,
        app: Option<&AppHandle>,
    ) -> Result<(ImageProject, Vec<String>), StudioError> {
        let mut project = self.get(id)?;
        // Setup owns the installation used for new renders, including older image projects.
        // Freeze its current location with this batch; completed takes retain their receipts.
        if !Path::new(configured_comfy_root.trim()).is_absolute() {
            return Err(StudioError::Invalid(
                "choose an absolute ComfyUI folder in Setup before generating images".into(),
            ));
        }
        project.settings.comfy_root = configured_comfy_root.trim().into();
        validate_render_ready(&project)?;
        let batch_size = project.settings.batch_size as usize;
        if project.takes.len().saturating_add(batch_size) > MAX_IMAGE_TAKES {
            return Err(StudioError::Invalid(
                "this batch would exceed the project's 256 preserved-take limit; reduce the batch or start a new project".into(),
            ));
        }
        let seed = resolved_seed(project.settings.seed);
        let (prompt, prompt_text) = self.backend.prompt(&project)?;
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
                model_profile: self.backend.profile().into(),
                license_notice: self.backend.license().into(),
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
                    "{batch_size} variation{} queued. Kestrel is releasing other GPU models before loading MOSS-Image.",
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
                "Preparing local ComfyUI and checking the native MOSS-Image nodes.",
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
        self.backend.verify(&self.http, &comfy_root).await?;
        let prefix = format!("kestrel_image/{project_id}/take_{:03}", take_index + 1);
        let graph = self.backend.graph(
            &project.settings,
            project.takes[take_index].seed,
            &project.takes[take_index].exact_prompt_text,
            &prefix,
        );
        for index in &take_indexes {
            project.takes[*index].exact_graph = graph.clone();
            project.takes[*index].status = "generating".into();
            project.takes[*index].detail =
                "MOSS-Image is sampling the full-resolution image locally.".into();
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
                "ComfyUI rejected the native MOSS-Image workflow: {}",
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
        project.license_notice = self.backend.license().into();
        project.updated_at = Utc::now().to_rfc3339();
        self.persist(project)?;
        if let Some(app) = app {
            let _ = crate::ipc_events::emit::<kestrel_app_core::events::ImageProject>(
                app,
                &project.clone(),
            );
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
            project.schema_version = IMAGE_SCHEMA_VERSION;
            project.license_notice = self.backend.license().into();
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
        let mut url = url::Url::parse(COMFY_BASE).ok()?;
        url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
            .ok()?;
        url.set_port(Some(ComfyWorkload::Shared.port())).ok()?;
        url.set_path("/ws");
        url.set_query(None);
        url.query_pairs_mut().append_pair("clientId", client_id);
        let (stream, _) = tokio_tungstenite::connect_async(url.as_str()).await.ok()?;
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
                            let _ = crate::ipc_events::emit::<kestrel_app_core::events::ImageGeneration>(&task_app, &event);
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
                "Loading the verified MOSS-Image model, text encoder, and full-quality decoder.",
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
        let _ = crate::ipc_events::emit::<kestrel_app_core::events::ImageGeneration>(app, &event);
    }
}

/// Exact native model input, preserved beside every take.
fn structured_prompt(project: &ImageProject) -> Result<(Value, String), StudioError> {
    Ok(kestrel_app_core::moss::prompt(project)?)
}

fn moss_graph(settings: &ImageSettings, seed: u64, prompt: &str, prefix: &str) -> Value {
    let steps = match settings.preset.as_str() {
        "quality" => 75,
        "turbo" => 30,
        _ => 50,
    };
    // Native MOSS uses shift 5 / timestep multiplier 1. Do not insert the plugin's SD3 patch.
    json!({
        "1":{"class_type":"UNETLoader","inputs":{"unet_name":MOSS_MODEL,"weight_dtype":"default"}},
        "3":{"class_type":"CLIPLoader","inputs":{"clip_name":MOSS_TEXT_ENCODER,"type":"moss_image","device":"default"}},
        "4":{"class_type":"VAELoader","inputs":{"vae_name":MOSS_VAE}},
        "5":{"class_type":"CLIPTextEncode","inputs":{"clip":["3",0],"text":prompt}},
        "6":{"class_type":"CLIPTextEncode","inputs":{"clip":["3",0],"text":settings.negative_prompt}},
        "7":{"class_type":"SkipLayerGuidanceDiTSimple","inputs":{"model":["1",0],"double_layers":"","single_layers":"22","start_percent":0.0,"end_percent":1.0}},
        "8":{"class_type":"CFGGuider","inputs":{"model":["7",0],"positive":["5",0],"negative":["6",0],"cfg":3.0}},
        "9":{"class_type":"RandomNoise","inputs":{"noise_seed":seed}},
        "10":{"class_type":"KSamplerSelect","inputs":{"sampler_name":"euler"}},
        "11":{"class_type":"BasicScheduler","inputs":{"model":["7",0],"scheduler":"sgm_uniform","steps":steps,"denoise":1.0}},
        "12":{"class_type":"EmptyFlux2LatentImage","inputs":{"width":settings.width,"height":settings.height,"batch_size":settings.batch_size}},
        "13":{"class_type":"SamplerCustomAdvanced","inputs":{"noise":["9",0],"guider":["8",0],"sampler":["10",0],"sigmas":["11",0],"latent_image":["12",0]}},
        "14":{"class_type":"VAEDecode","inputs":{"samples":["13",0],"vae":["4",0]}},
        "15":{"class_type":"SaveImage","inputs":{"images":["14",0],"filename_prefix":prefix}}
    })
}

#[cfg(test)]
async fn verify_running_comfy_root(http: &Client, selected_root: &Path) -> Result<(), StudioError> {
    super::verify_comfy_installation(http, COMFY_BASE, selected_root).await
}

#[cfg(test)]
fn validate_running_comfy_root(info: &Value, selected_root: &Path) -> Result<(), StudioError> {
    super::validate_comfy_installation(info, COMFY_BASE, selected_root)
}

#[derive(Clone, Copy, Default)]
enum ImageBackend {
    #[default]
    Moss,
    #[cfg(test)]
    Ideogram,
}

impl ImageBackend {
    fn profile(self) -> &'static str {
        match self {
            Self::Moss => "MOSS-Image Base INT8 ConvRot + Qwen3.5 9B INT8 ConvRot + Flux 2 VAE; ComfyUI v0.42.1; model revision ee64003102c8470ee32501cf883419235db81884",
            #[cfg(test)]
            Self::Ideogram => "Ideogram 4 NVFP4 mixed + Qwen3-VL 8B NVFP4 + Flux 2 VAE",
        }
    }
    fn license(self) -> &'static str {
        match self {
            Self::Moss => MOSS_LICENSE_NOTICE,
            #[cfg(test)]
            Self::Ideogram => ideogram_baseline::IDEOGRAM_LICENSE_NOTICE,
        }
    }
    fn prompt(self, project: &ImageProject) -> Result<(Value, String), StudioError> {
        match self {
            Self::Moss => structured_prompt(project),
            #[cfg(test)]
            Self::Ideogram => ideogram_baseline::structured_prompt(project),
        }
    }
    fn graph(self, settings: &ImageSettings, seed: u64, prompt: &str, prefix: &str) -> Value {
        match self {
            Self::Moss => moss_graph(settings, seed, prompt, prefix),
            #[cfg(test)]
            Self::Ideogram => ideogram_baseline::ideogram_graph(settings, seed, prompt, prefix),
        }
    }
    async fn verify(self, http: &Client, root: &Path) -> Result<(), StudioError> {
        #[cfg(test)]
        if matches!(self, Self::Ideogram) {
            ideogram_baseline::verify_ideogram_nodes(http).await?;
            return ideogram_baseline::verify_ideogram_assets(root);
        }
        let info: Value = http
            .get(format!("{COMFY_BASE}/object_info"))
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        for node in [
            "MOSSImageLayoutPrompt",
            "SkipLayerGuidanceDiTSimple",
            "CFGGuider",
            "BasicScheduler",
            "EmptyFlux2LatentImage",
            "SamplerCustomAdvanced",
        ] {
            if info.get(node).is_none() {
                return Err(StudioError::Render(format!("the running ComfyUI does not expose {node}. Install ComfyUI v0.42.1 or newer, restart it, then resume Image Studio in Setup")));
            }
        }
        let types = info
            .pointer("/CLIPLoader/input/required/type/0")
            .and_then(Value::as_array);
        if !types.is_some_and(|types| types.iter().any(|kind| kind == "moss_image")) {
            return Err(StudioError::Render("the running ComfyUI text encoder does not support moss_image; install ComfyUI v0.42.1 or newer and restart it".into()));
        }
        for relative in [
            format!("models/diffusion_models/{MOSS_MODEL}"),
            format!("models/text_encoders/{MOSS_TEXT_ENCODER}"),
            format!("models/vae/{MOSS_VAE}"),
        ] {
            if !root.join(&relative).is_file() {
                return Err(StudioError::Render(format!(
                    "MOSS-Image asset is missing: {}. Resume Image Studio in Setup.",
                    root.join(relative).display()
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod ideogram_baseline;

#[cfg(test)]
mod acceptance;

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
            StudioError::Render("the completed image graph exposed no saved image output".into())
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
            "ComfyUI completed the image batch with {} image{}, but Kestrel expected {expected_count}; no partial batch was registered",
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
    validate_bounded_text(
        "negative prompt",
        &project.settings.negative_prompt,
        4_000,
        true,
    )?;
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
    // The guide permits extrapolation up to roughly 4 MP. Keep the previous
    // 2048-square area budget while allowing native 2560x1440 QHD and its portrait.
    if !(256..=2560).contains(&settings.width)
        || !(256..=2560).contains(&settings.height)
        || u64::from(settings.width) * u64::from(settings.height) > 2048 * 2048
        || !settings.width.is_multiple_of(16)
        || !settings.height.is_multiple_of(16)
        || settings.width.max(settings.height) > settings.width.min(settings.height) * 6
    {
        return Err(StudioError::Invalid(
            "Image dimensions must be 256 to 2560 pixels, multiples of 16, at most 4,194,304 pixels total, with an aspect ratio no wider than 6:1. Use 2048×1152 for 2K or 2560×1440 for QHD; native 4K is outside the supported range.".into(),
        ));
    }
    if !matches!(settings.preset.as_str(), "quality" | "standard" | "turbo") {
        return Err(StudioError::Invalid(
            "Image sampling preset must be quality, standard, or turbo".into(),
        ));
    }
    if !(1..=4).contains(&settings.batch_size) {
        return Err(StudioError::Invalid(
            "an image batch may contain one to four variations".into(),
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
    use kestrel_app_core::ImageElement;
    use tempfile::TempDir;

    #[test]
    fn native_2k_and_qhd_preserve_dimensions_and_boxes_without_upscaling() {
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let mut project = project(&studio);
        let boxes = project.elements[0].bbox;
        for (width, height) in [(2048, 1152), (2560, 1440), (1440, 2560), (2048, 2048)] {
            project.settings.width = width;
            project.settings.height = height;
            project = studio.save_editable(project).unwrap();
            project = studio.get(&project.id).unwrap();
            assert_eq!(
                (project.settings.width, project.settings.height),
                (width, height)
            );
            assert_eq!(project.elements[0].bbox, boxes);
            let (prompt, text) = structured_prompt(&project).unwrap();
            assert_eq!(
                prompt["items"][0]["bbox"],
                json!(kestrel_app_core::moss::centered_pixel_box(
                    boxes, width, height
                ))
            );
            let graph = moss_graph(&project.settings, 42, &text, "native-resolution");
            assert_eq!(graph["12"]["inputs"]["width"], width);
            assert_eq!(graph["12"]["inputs"]["height"], height);
            assert!(!graph.to_string().contains("Upscale"));
        }
        for (width, height) in [
            (2560, 2560),
            (3840, 2160),
            (2048, 1080),
            (2561, 1440),
            (256, 2560),
            (u32::MAX, u32::MAX),
        ] {
            project.settings.width = width;
            project.settings.height = height;
            assert!(validate_editable(&project).is_err(), "{width}x{height}");
        }
    }

    #[test]
    fn moss_preserves_text_palette_layer_order_and_converts_boxes() {
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let mut project = project(&studio);
        project.elements[0].text = "CAFÉ\n東京 — 2026".into();
        project.elements.push(ImageElement {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "obj".into(),
            bbox: [250, 0, 1000, 1000],
            text: String::new(),
            description: "A cobalt bird behind the title".into(),
            color_palette: vec!["#0044CC".into()],
        });
        let (prompt, text) = structured_prompt(&project).unwrap();
        assert_eq!(prompt["items"][0]["bbox"], json!([-441, -646, -276, 646]));
        assert_eq!(prompt["items"][0]["text"], "CAFÉ\n東京 — 2026");
        assert_eq!(prompt["items"][1]["label"], "object");
        assert_eq!(prompt["items"][1]["palette"], json!(["#0044CC"]));
        assert!(prompt["items"][1].get("text").is_none());
        assert_eq!(prompt["bbox_coord_space"], "abs_centralized");
        assert_eq!(prompt["bbox_format"], "yxyx");
        assert!(!text.contains("compositional_deconstruction"));
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap(), prompt);
        project.elements.clear();
        let (prompt, text) = structured_prompt(&project).unwrap();
        assert_eq!(prompt.as_str(), Some(text.as_str()));
        assert!(text.contains(project.style.photo.trim()));
        project.style.mode = "art".into();
        let (_, text) = structured_prompt(&project).unwrap();
        assert!(text.contains(project.style.art_style.trim()));
        assert!(!text.contains(project.style.photo.trim()));
    }

    #[test]
    fn moss_native_graph_uses_guidance_and_negative_prompt_for_every_preset() {
        for (preset, steps) in [("quality", 75), ("standard", 50), ("turbo", 30)] {
            let settings = ImageSettings {
                preset: preset.into(),
                negative_prompt: "avoid red".into(),
                batch_size: 4,
                ..ImageSettings::default()
            };
            let graph = moss_graph(&settings, 42, "a blue bird", "test");
            assert_eq!(graph["3"]["inputs"]["type"], "moss_image");
            assert_eq!(graph["11"]["inputs"]["steps"], steps);
            assert_eq!(graph["11"]["inputs"]["scheduler"], "sgm_uniform");
            assert_eq!(graph["7"]["inputs"]["single_layers"], "22");
            assert_eq!(graph["6"]["inputs"]["text"], "avoid red");
            assert_eq!(graph["12"]["inputs"]["batch_size"], 4);
            assert_eq!(graph["9"]["inputs"]["noise_seed"], 42);
            assert!(!graph.to_string().contains("ModelSamplingSD3"));
            assert!(!graph.to_string().contains("Ideogram"));
        }
    }

    #[test]
    fn legacy_take_receipts_survive_moss_migration_and_negative_prompt_is_bounded() {
        let root = TempDir::new().unwrap();
        let mut studio = ImageStudio::new(root.path()).unwrap();
        studio.backend = ImageBackend::Ideogram;
        let project = project(&studio);
        let (_, ids) = studio
            .begin_generation(&project.id, &project.settings.comfy_root, None)
            .unwrap();
        studio.fail_generation(&project.id, &ids, "baseline receipt".into(), true, None);
        let baseline = studio.get(&project.id).unwrap();
        let receipt = serde_json::to_value(&baseline.takes[0]).unwrap();
        let mut old = serde_json::to_value(&baseline).unwrap();
        old["schemaVersion"] = json!(3);
        old["settings"]
            .as_object_mut()
            .unwrap()
            .remove("negativePrompt");
        write_json_recoverable(&studio.project_dir(&project.id).join("project.json"), &old)
            .unwrap();
        studio.backend = ImageBackend::Moss;
        let mut migrated = studio.get(&project.id).unwrap();
        assert!(!migrated.settings.negative_prompt.is_empty());
        assert_eq!(serde_json::to_value(&migrated.takes[0]).unwrap(), receipt);
        assert!(migrated.license_notice.contains("Apache 2.0"));
        migrated.settings.negative_prompt = "x".repeat(4_001);
        assert!(studio.save_editable(migrated.clone()).is_err());
        migrated.settings.negative_prompt.clear();
        studio.save_editable(migrated).unwrap();
        let selected_installation = root.path().join("new-comfy");
        let selected_root = selected_installation.to_str().unwrap();
        let (next, _) = studio
            .begin_generation(&project.id, selected_root, None)
            .unwrap();
        assert_eq!(next.settings.comfy_root, selected_root);
        assert_eq!(
            studio.get(&project.id).unwrap().settings.comfy_root,
            selected_root
        );
        assert_eq!(serde_json::to_value(&next.takes[0]).unwrap(), receipt);
        assert!(next.takes[1].model_profile.starts_with("MOSS-Image"));
    }

    #[test]
    fn generation_rejects_an_invalid_setup_location_without_changing_the_project() {
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let before = serde_json::to_value(&project).unwrap();
        assert!(studio
            .begin_generation(&project.id, "relative/comfy", None)
            .is_err());
        assert_eq!(
            serde_json::to_value(studio.get(&project.id).unwrap()).unwrap(),
            before
        );
    }

    #[test]
    fn running_comfy_must_match_the_selected_installation() {
        let selected = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        for root in [&selected, &other] {
            fs::write(root.path().join("main.py"), "# fixture").unwrap();
        }
        let correct = json!({"system":{"argv":[selected.path().join("main.py"),"--port","8188"]}});
        validate_running_comfy_root(&correct, selected.path()).unwrap();
        let wrong = json!({"system":{"argv":[other.path().join("main.py"),"--port","8188"]}});
        let error = validate_running_comfy_root(&wrong, selected.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("Stop the existing ComfyUI server"));
        assert!(error.contains(selected.path().to_str().unwrap()));
        for unverified in [json!({}), json!({"system":{"argv":["main.py"]}})] {
            assert!(validate_running_comfy_root(&unverified, selected.path()).is_err());
        }
    }

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
        let (prompt, text) = ideogram_baseline::structured_prompt(&project).unwrap();
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
        let (_, photo) = ideogram_baseline::structured_prompt(&project).unwrap();
        assert!(photo.contains("\"photo\":"));
        assert!(!photo.contains("\"art_style\":"));
        assert!(photo.find("\"photo\":") < photo.find("\"medium\":"));

        project.style.mode = "art".into();
        project.style.medium = "Risograph print".into();
        project.style.art_style = "Geometric editorial illustration".into();
        let (_, art) = ideogram_baseline::structured_prompt(&project).unwrap();
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
        let graph = ideogram_baseline::ideogram_graph(
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

        let standard = ideogram_baseline::ideogram_graph(
            &ImageSettings::default(),
            42,
            "{\"high_level_description\":\"test\"}",
            "test",
        );
        assert_eq!(standard["11"]["inputs"]["steps"], 20);
        assert_eq!(standard["11"]["inputs"]["mu"], 0.0);
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
        let (_, take_ids) = studio
            .begin_generation(&project.id, &project.settings.comfy_root, None)
            .unwrap();
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
    fn old_numeric_seed_survives_open_edit_save_with_explicit_new_schema() {
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let original = project(&studio);
        let seed = i64::MAX as u64;
        let path = studio.project_dir(&original.id).join("project.json");
        let mut legacy = serde_json::to_value(&original).unwrap();
        legacy["schemaVersion"] = json!(2);
        legacy["settings"]["seed"] = json!(seed);
        let old_bytes = serde_json::to_vec(&legacy).unwrap();
        fs::write(&path, &old_bytes).unwrap();
        let opened = studio.get(&original.id).unwrap();
        assert_eq!(opened.settings.seed, seed);
        assert_eq!(fs::read(&path).unwrap(), old_bytes);
        let mut wire = serde_json::to_value(opened).unwrap();
        assert_eq!(wire["settings"]["seed"], seed.to_string());
        wire["title"] = json!("Changed through the UI");
        let mut invalid = wire.clone();
        invalid["settings"]["seed"] = json!(u64::MAX.to_string());
        let invalid: kestrel_app_core::ImageProjectEdit = serde_json::from_value(invalid).unwrap();
        assert!(studio
            .save_editable(invalid)
            .unwrap_err()
            .to_string()
            .contains("integer boundary"));
        assert_eq!(fs::read(&path).unwrap(), old_bytes);
        let edit: kestrel_app_core::ImageProjectEdit = serde_json::from_value(wire).unwrap();
        studio.save_editable(edit).unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["schemaVersion"], IMAGE_SCHEMA_VERSION);
        assert_eq!(saved["settings"]["seed"], seed.to_string());
    }

    #[test]
    fn frontend_cannot_replace_immutable_take_truth() {
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let (_, take_ids) = studio
            .begin_generation(&project.id, &project.settings.comfy_root, None)
            .unwrap();
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
    #[ignore = "requires KESTREL_LIVE_COMFY_ROOT, native MOSS-Image nodes and verified INT8 weights, and several minutes"]
    async fn live_moss_graph_preserves_a_full_resolution_png() {
        let comfy_root = std::env::var("KESTREL_LIVE_COMFY_ROOT")
            .expect("set KESTREL_LIVE_COMFY_ROOT to the absolute ComfyUI folder");
        let root = TempDir::new().unwrap();
        let studio = ImageStudio::new(root.path()).unwrap();
        let renderer = MovieStudio::new(root.path()).unwrap();
        let mut project = studio
            .create(CreateImageProjectRequest {
                title: "MOSS-Image acceptance".into(),
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
        let (_, take_ids) = studio
            .begin_generation(&project.id, &project.settings.comfy_root, None)
            .unwrap();
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
