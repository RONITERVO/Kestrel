//! Durable offline Movie, Image, and Music Studio facade.
//!
//! Maintainers should read `studio/README.md` before changing model-assisted flows or persistence
//! boundaries. Child modules own MiniMax H3 multi-scene timeline direction, Reference Library
//! `@Handle` compilation, stock ComfyUI rendering, Image Studio, Music Studio, and FFmpeg exports.

pub use kestrel_app_core::movie::{
    default_export_preset, ClipEdit, MovieExport, MovieProject, MovieReference,
    MovieReferenceAsset, MovieReferenceImport, MovieSummary, RenderedClip,
};

use chrono::Utc;
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tauri::AppHandle;
use thiserror::Error;
use tokio::{process::Child, sync::Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;

mod export;
mod h3_digest;
mod h3_engine;
mod h3_geometry;
mod h3_graph;
mod h3_media;
mod h3_prompt;
mod h3_references;
mod image_assets;
mod image_studio;
mod live_preview;
mod model_stream;
mod music;
mod music_lyrics_model;
mod music_midi;
mod music_yue2;
mod prompt_draft;

pub use image_assets::MovieImageAssetGeneration;
pub use image_studio::{CreateImageProjectRequest, ImageProject, ImageStudio, ImageSummary};
pub use live_preview::MovieRenderState;
use live_preview::LivePreviewRegistry;
pub use music::{
    CreateMusicProjectRequest, DraftLyricsFromAudioRangeRequest, DraftLyricsFromAudioRangeResult,
    MusicLyricsRequest, MusicLyricsSaveResult, MusicMidiRequest, MusicMidiSaveResult, MusicProject,
    MusicStudio, MusicSummary, RepairMusicLyricsRangeRequest, SaveMusicLyricsDocumentRequest,
    SaveMusicMidiDocumentRequest, TranscribeMusicLyricsRequest, TranslateMusicLyricsRequest,
    TranslateMusicLyricsResult,
};
pub(crate) use music_lyrics_model::{
    draft_from_audio as draft_music_lyrics_from_audio,
    translate as translate_music_lyrics_with_model,
    validate_translation_request as validate_music_lyrics_translation,
};
pub(crate) use prompt_draft::{
    emit_error as emit_prompt_draft_error, emit_settled as emit_prompt_draft_settled,
    validate_request as validate_prompt_draft_request, IMAGE_COMPOSITION_SCHEMA,
};
pub use prompt_draft::{PromptDraftJob, PromptDraftRequest};

const SCHEMA_VERSION: u32 = 9;
const COMFY_BASE: &str = "http://127.0.0.1:8188";
pub(super) const MUSIC_COMFY_BASE: &str = "http://127.0.0.1:8189";
const MAX_REFERENCE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_AUDIO_BYTES: u64 = 256 * 1024 * 1024;
const HASH_BUFFER_BYTES: usize = 64 * 1024;
pub(super) const MAX_MOVIE_PROMPT_BYTES: usize = 64 * 1024;
const MAX_REFERENCE_SECONDS: f64 = 3_600.0;

const COMFY_RENDER_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Copy)]
pub(crate) enum ComfyWorkload {
    Shared,
    Music,
}

impl ComfyWorkload {
    pub(crate) fn from_music(music: bool) -> Self {
        if music {
            Self::Music
        } else {
            Self::Shared
        }
    }

    pub(crate) fn base_url(self) -> &'static str {
        match self {
            Self::Shared => COMFY_BASE,
            Self::Music => MUSIC_COMFY_BASE,
        }
    }

    pub(crate) fn port(self) -> u16 {
        url::Url::parse(self.base_url())
            .ok()
            .and_then(|url| url.port_or_known_default())
            .expect("fixed ComfyUI base URL must include a valid port")
    }

    pub(crate) fn script_names(self) -> &'static [&'static str] {
        match self {
            Self::Shared => &["Start-Kestrel-ComfyUI.ps1", "Start-ComfyUI-MiniMax-H3.ps1"],
            Self::Music => &["Start-Kestrel-ComfyUI-Music.ps1"],
        }
    }
}

pub(crate) fn media_program(name: &str) -> PathBuf {
    let key = if name == "ffprobe" {
        "KESTREL_FFPROBE_PATH"
    } else {
        "KESTREL_FFMPEG_PATH"
    };
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

pub fn media_response(request: tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    match read_media_response(&request) {
        Ok(response) => response,
        Err((status, message)) => tauri::http::Response::builder()
            .status(status)
            .header("Content-Type", "text/plain; charset=utf-8")
            .header("Access-Control-Allow-Origin", "*")
            .body(message.into_bytes())
            .expect("fixed media error response"),
    }
}

fn read_media_response(
    request: &tauri::http::Request<Vec<u8>>,
) -> Result<tauri::http::Response<Vec<u8>>, (u16, String)> {
    let library = directories::UserDirs::new()
        .map(|dirs| dirs.home_dir().join("Kestrel Research"))
        .ok_or_else(|| (500, "local media library is unavailable".to_string()))?;
    read_media_response_from_library(request, &library)
}

fn read_media_response_from_library(
    request: &tauri::http::Request<Vec<u8>>,
    library: &Path,
) -> Result<tauri::http::Response<Vec<u8>>, (u16, String)> {
    if !matches!(
        *request.method(),
        tauri::http::Method::GET | tauri::http::Method::HEAD
    ) {
        return Err((
            405,
            "local media supports only GET and HEAD requests".into(),
        ));
    }
    let relative =
        percent_encoding::percent_decode_str(request.uri().path().trim_start_matches('/'))
            .decode_utf8()
            .map_err(|_| (400, "invalid movie media path".to_string()))?;
    if relative.is_empty()
        || relative.contains("..")
        || relative.contains(['\\', ':', '\0', '\r', '\n'])
    {
        return Err((403, "unsafe movie media path".into()));
    }
    let (root, relative) = if let Some(relative) = relative.strip_prefix("music/") {
        (library.join("music"), relative)
    } else if let Some(relative) = relative.strip_prefix("images/") {
        (library.join("images"), relative)
    } else {
        (library.join("movies"), relative.as_ref())
    };
    let canonical_root = fs::canonicalize(&root)
        .map_err(|_| (404, "local movie library does not exist".to_string()))?;
    let target = fs::canonicalize(root.join(relative))
        .map_err(|_| (404, "local media was not found".to_string()))?;
    if !target.starts_with(&canonical_root) || !target.is_file() {
        return Err((403, "media is outside the private library".into()));
    }
    let mut file = fs::File::open(&target).map_err(|error| (500, error.to_string()))?;
    let length = file
        .metadata()
        .map_err(|error| (500, error.to_string()))?
        .len();
    let content_type = match target
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "m4a" | "aac" => "audio/mp4",
        _ => "application/octet-stream",
    };
    let mut builder = tauri::http::Response::builder()
        .header("Content-Type", content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header("Accept-Ranges", "bytes");
    if request
        .uri()
        .query()
        .is_some_and(|query| query.split('&').any(|parameter| parameter == "download=1"))
    {
        let filename = target
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("kestrel-media");
        let encoded =
            percent_encoding::utf8_percent_encode(filename, percent_encoding::NON_ALPHANUMERIC);
        builder = builder.header(
            "Content-Disposition",
            format!("attachment; filename*=UTF-8''{encoded}"),
        );
    }
    if request.method() == tauri::http::Method::HEAD {
        return builder
            .header("Content-Length", length)
            .body(Vec::new())
            .map_err(|error| (500, error.to_string()));
    }
    let range = request
        .headers()
        .get("range")
        .and_then(|value| value.to_str().ok());
    if let Some(range) = range {
        let value = range
            .strip_prefix("bytes=")
            .and_then(|value| value.split(',').next())
            .ok_or_else(|| (416, "unsupported media byte range".to_string()))?;
        let (start_text, end_text) = value
            .split_once('-')
            .ok_or_else(|| (416, "invalid media byte range".to_string()))?;
        let start = start_text
            .parse::<u64>()
            .map_err(|_| (416, "suffix byte ranges are unsupported".to_string()))?;
        if start >= length {
            return Err((416, "media byte range starts beyond the file".into()));
        }
        const MAX_CHUNK: u64 = 4 * 1024 * 1024;
        let requested_end = if end_text.is_empty() {
            length - 1
        } else {
            end_text
                .parse::<u64>()
                .map_err(|_| (416, "invalid media byte range end".to_string()))?
        };
        if requested_end < start {
            return Err((416, "media byte range ends before it starts".into()));
        }
        let end = requested_end
            .min(length - 1)
            .min(start.saturating_add(MAX_CHUNK - 1));
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
        builder.body(body).map_err(|error| (500, error.to_string()))
    } else {
        const MAX_UNRANGED_MEDIA_BYTES: u64 = 64 * 1024 * 1024;
        if length > MAX_UNRANGED_MEDIA_BYTES {
            return Err((
                413,
                "local media is too large for one in-memory response; retry playback so the WebView requests byte ranges, or reveal the project files"
                    .into(),
            ));
        }
        let mut body = Vec::with_capacity(length as usize);
        file.read_to_end(&mut body)
            .map_err(|error| (500, error.to_string()))?;
        builder
            .header("Content-Length", length)
            .body(body)
            .map_err(|error| (500, error.to_string()))
    }
}

#[derive(Debug, Error)]
pub enum StudioError {
    #[error("movie studio file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("movie studio JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("local movie service error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("movie project not found: {0}")]
    NotFound(String),
    #[error("movie request is invalid: {0}")]
    Invalid(String),
    #[error("local media render failed: {0}")]
    Render(String),
    #[error("studio operation was stopped")]
    Cancelled,
}

#[derive(Clone)]
pub struct MovieStudio {
    root: PathBuf,
    http: Client,
    comfy_child: Arc<AsyncMutex<Option<Child>>>,
    music_comfy_child: Arc<AsyncMutex<Option<Child>>>,
    comfy_preview_available: Arc<AsyncMutex<Option<bool>>>,
    live_previews: LivePreviewRegistry,
    project_locks: Arc<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
}

impl MovieStudio {
    pub fn new(library_root: &Path) -> Result<Self, StudioError> {
        let root = library_root.join("movies");
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join("_references").join("objects"))?;
        fs::create_dir_all(root.join("_references").join("meta"))?;
        let studio = Self {
            root,
            http: Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3_600))
                .build()?,
            comfy_child: Arc::new(AsyncMutex::new(None)),
            music_comfy_child: Arc::new(AsyncMutex::new(None)),
            comfy_preview_available: Arc::new(AsyncMutex::new(None)),
            live_previews: LivePreviewRegistry::default(),
            project_locks: Arc::new(StdMutex::new(HashMap::new())),
        };
        studio.recover_interrupted()?;
        studio.recover_image_asset_generations()?;
        studio.recover_h3_render_jobs()?;
        Ok(studio)
    }

    pub fn movie_render_state(
        &self,
        project_id: &str,
        active: bool,
    ) -> Result<MovieRenderState, StudioError> {
        validate_id(project_id)?;
        if !active {
            self.live_previews.clear_movie(project_id);
        }
        Ok(MovieRenderState {
            active,
            preview: if active {
                self.live_previews.movie(project_id)
            } else {
                None
            },
        })
    }

    pub fn import_reference_path(&self, source: &Path) -> Result<MovieReferenceAsset, StudioError> {
        self.import_reference_path_named(source, None)
    }

    /// Imports a media file into the content-addressed reference library; `name` replaces the file
    /// name shown to the producer (e.g., an Image Studio take, Speech Studio clip, or Music take).
    pub fn import_reference_path_named(
        &self,
        source: &Path,
        name: Option<&str>,
    ) -> Result<MovieReferenceAsset, StudioError> {
        let source = source.canonicalize().map_err(|error| {
            StudioError::Invalid(format!("Cannot open {}: {error}", source.display()))
        })?;
        let metadata = source.metadata()?;
        if !metadata.is_file() {
            return Err(StudioError::Invalid(format!(
                "Only regular media files can be references: {}",
                source.display()
            )));
        }
        let (kind, mime_type, limit) = classify_reference(&source)?;
        if metadata.len() > limit {
            return Err(StudioError::Invalid(format!(
                "{} is larger than the {} reference limit.",
                reference_name(&source),
                readable_bytes(limit)
            )));
        }
        let probe = probe_reference(&source, &kind)?;
        let reference_root = self.root.join("_references");
        let temporary = reference_root
            .join("objects")
            .join(format!("import-{}.tmp", uuid::Uuid::new_v4()));
        let (id, bytes) = copy_reference_and_hash(&source, &temporary, limit)?;
        let extension = reference_extension(&source);
        let object_name = format!("{id}.{extension}");
        let object_path = reference_root.join("objects").join(&object_name);
        if object_path.is_file() {
            fs::remove_file(&temporary)?;
        } else {
            fs::rename(&temporary, &object_path)?;
        }
        let meta_path = reference_root.join("meta").join(format!("{id}.json"));
        if meta_path.is_file() {
            return self.resolve_reference_asset(&id);
        }
        let asset = MovieReferenceAsset {
            id,
            name: name.map_or_else(|| reference_name(&source), str::to_string),
            kind,
            mime_type,
            bytes,
            duration_seconds: probe.duration_seconds,
            width: probe.width,
            height: probe.height,
            has_audio: probe.has_audio,
            path: object_path.to_string_lossy().into_owned(),
            created_at: Utc::now().to_rfc3339(),
            generation: None,
        };
        write_json_atomic(&meta_path, &asset)?;
        Ok(asset)
    }

    fn resolve_reference_asset(&self, id: &str) -> Result<MovieReferenceAsset, StudioError> {
        if id.len() != 64 || !id.bytes().all(|value| value.is_ascii_hexdigit()) {
            return Err(StudioError::Invalid("invalid movie reference id".into()));
        }
        let reference_root = self.root.join("_references");
        let meta = reference_root.join("meta").join(format!("{id}.json"));
        let mut asset: MovieReferenceAsset = serde_json::from_slice(&fs::read(meta)?)?;
        if asset.id != id {
            return Err(StudioError::Invalid(
                "movie reference metadata does not match its content id".into(),
            ));
        }
        let stored_name = Path::new(&asset.path)
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| value.starts_with(&format!("{id}.")))
            .ok_or_else(|| StudioError::Invalid("invalid movie reference object name".into()))?;
        let object = reference_root.join("objects").join(stored_name);
        if !object.is_file() || hash_reference(&object)? != id {
            return Err(StudioError::Invalid(format!(
                "Reference {} is missing or failed its integrity check; attach it again.",
                asset.name
            )));
        }
        asset.path = object.to_string_lossy().into_owned();
        Ok(asset)
    }

    pub fn list(&self) -> Result<Vec<MovieSummary>, StudioError> {
        let mut projects = fs::read_dir(&self.root)?
            .filter_map(Result::ok)
            .filter_map(|entry| self.load_path(&entry.path().join("project.json")).ok())
            .map(|project| MovieSummary::from(&project))
            .collect::<Vec<_>>();
        projects.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(projects)
    }

    pub fn get(&self, id: &str) -> Result<MovieProject, StudioError> {
        validate_id(id)?;
        self.load_path(&self.project_dir(id).join("project.json"))
            .map_err(|error| {
                if matches!(error, StudioError::Io(ref io) if io.kind() == std::io::ErrorKind::NotFound) {
                    StudioError::NotFound(id.into())
                } else {
                    error
                }
            })
    }

    fn project_lock(&self, id: &str) -> Result<Arc<AsyncMutex<()>>, StudioError> {
        validate_id(id)?;
        let mut locks = self.project_locks.lock().map_err(|_| {
            StudioError::Invalid("movie project lock registry is unavailable".into())
        })?;
        Ok(locks
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone())
    }

    pub fn fail(&self, id: &str, error: impl ToString, app: Option<&AppHandle>) {
        if let Ok(mut project) = self.get(id) {
            project.status = "failed".into();
            project.phase = "failed".into();
            project.detail = "Production stopped with a recoverable error.".into();
            project.error = error.to_string();
            let _ = self.persist_emit(&mut project, app);
        }
    }

    pub fn stop(&self, id: &str, app: Option<&AppHandle>) -> Result<MovieProject, StudioError> {
        let mut project = self.get(id)?;
        let now = Utc::now().to_rfc3339();
        for job in &mut project.render_jobs {
            if matches!(
                job.status,
                kestrel_app_core::H3RenderJobStatus::Queued
                    | kestrel_app_core::H3RenderJobStatus::Preparing
                    | kestrel_app_core::H3RenderJobStatus::Rendering
                    | kestrel_app_core::H3RenderJobStatus::PostProcessing
            ) {
                job.status = kestrel_app_core::H3RenderJobStatus::Cancelled;
                job.error = "Stopped by producer.".into();
                job.updated_at = now.clone();
            }
        }
        project.status = "cancelled".into();
        project.phase = "cancelled".into();
        project.detail =
            "Production stopped. Every completed take and timeline edit is safe.".into();
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub async fn release_comfy_memory(&self) {
        let _ = self
            .http
            .post(format!("{COMFY_BASE}/free"))
            .json(&json!({"unload_models":true,"free_memory":true}))
            .timeout(Duration::from_secs(30))
            .send()
            .await;
    }

    async fn ensure_comfy(
        &self,
        root: &str,
        project: &mut MovieProject,
        app: Option<&AppHandle>,
    ) -> Result<(), StudioError> {
        if self.comfy_ready().await {
            return verify_comfy_installation(&self.http, COMFY_BASE, Path::new(root)).await;
        }
        project.phase = "starting-renderer".into();
        project.detail = "Starting the private ComfyUI MiniMax H3 renderer.".into();
        self.persist_emit(project, app)?;
        let logs = self.project_dir(&project.id).join("logs");
        self.ensure_comfy_process(root, &logs, None).await
    }

    pub(super) async fn ensure_comfy_process(
        &self,
        root: &str,
        logs: &Path,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), StudioError> {
        self.ensure_comfy_process_at(root, logs, cancel, ComfyWorkload::Shared)
            .await
    }

    pub(super) async fn ensure_music_comfy_process(
        &self,
        root: &str,
        logs: &Path,
        cancel: Option<&CancellationToken>,
    ) -> Result<(), StudioError> {
        self.ensure_comfy_process_at(root, logs, cancel, ComfyWorkload::Music)
            .await
    }

    async fn ensure_comfy_process_at(
        &self,
        root: &str,
        logs: &Path,
        cancel: Option<&CancellationToken>,
        workload: ComfyWorkload,
    ) -> Result<(), StudioError> {
        let base_url = workload.base_url();
        if self.comfy_ready_at(base_url).await {
            return verify_comfy_installation(&self.http, base_url, Path::new(root)).await;
        }
        let root = PathBuf::from(root);
        let script = workload
            .script_names()
            .iter()
            .map(|name| root.join(name))
            .find(|candidate| candidate.is_file())
            .ok_or_else(|| {
                StudioError::Render(format!(
                    "Kestrel's local ComfyUI starter is missing from {}. Open Setup and resume Movie Studio or Music Production.",
                    root.display()
                ))
            })?;
        fs::create_dir_all(logs)?;
        let stdout = fs::File::create(logs.join("comfy.stdout.log"))?;
        let stderr = fs::File::create(logs.join("comfy.stderr.log"))?;
        let port = workload.port().to_string();
        let mut command = tokio::process::Command::new("powershell.exe");
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .args(["-Port", &port, "-NoBrowser"])
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .kill_on_drop(false);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let child = command.spawn()?;
        match workload {
            ComfyWorkload::Shared => {
                *self.comfy_preview_available.lock().await = None;
                *self.comfy_child.lock().await = Some(child);
            }
            ComfyWorkload::Music => *self.music_comfy_child.lock().await = Some(child),
        }
        for _ in 0..180 {
            if self.comfy_ready_at(base_url).await {
                return verify_comfy_installation(&self.http, base_url, &root).await;
            }
            if let Some(cancel) = cancel {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {},
                    _ = cancel.cancelled() => return Err(StudioError::Cancelled),
                }
            } else {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
        Err(StudioError::Render(
            "ComfyUI did not become ready within six minutes; see the project logs".into(),
        ))
    }

    pub(super) async fn comfy_ready(&self) -> bool {
        self.comfy_ready_at(COMFY_BASE).await
    }

    async fn comfy_ready_at(&self, base_url: &str) -> bool {
        self.http
            .get(format!("{base_url}/system_stats"))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    pub(super) async fn comfy_preview_available(&self) -> bool {
        let mut cached = self.comfy_preview_available.lock().await;
        if let Some(available) = *cached {
            return available;
        }
        let available = match self
            .http
            .get(format!("{COMFY_BASE}/object_info"))
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => response
                .json::<Value>()
                .await
                .ok()
                .is_some_and(|value| value.get("ModelPreviewOverrideKJ").is_some()),
            _ => false,
        };
        *cached = Some(available);
        available
    }

    pub fn project_dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn save(&self, project: &MovieProject) -> Result<(), StudioError> {
        write_json_atomic(&self.project_dir(&project.id).join("project.json"), project)
    }

    fn persist_emit(
        &self,
        project: &mut MovieProject,
        app: Option<&AppHandle>,
    ) -> Result<(), StudioError> {
        project.updated_at = Utc::now().to_rfc3339();
        self.save(project)?;
        if let Some(app) = app {
            let _ = crate::ipc_events::emit::<kestrel_app_core::events::MovieProject>(
                app,
                &project.clone(),
            );
        }
        Ok(())
    }

    fn load_path(&self, path: &Path) -> Result<MovieProject, StudioError> {
        let backup = path.with_extension("json.backup");
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(primary) => match fs::read(&backup) {
                Ok(bytes) => {
                    fs::copy(&backup, path)?;
                    bytes
                }
                Err(_) => return Err(primary.into()),
            },
        };
        let mut project: MovieProject = match serde_json::from_slice(&bytes) {
            Ok(project) => project,
            Err(primary) => match fs::read(&backup) {
                Ok(bytes) => {
                    let project = serde_json::from_slice(&bytes)?;
                    fs::copy(&backup, path)?;
                    project
                }
                Err(_) => return Err(primary.into()),
            },
        };
        normalize_movie_project(&mut project);
        h3_engine::migrate_legacy_project_in_memory(
            path.parent().unwrap_or_else(|| Path::new("")),
            &mut project,
        );
        Ok(project)
    }

    fn recover_interrupted(&self) -> Result<(), StudioError> {
        for entry in fs::read_dir(&self.root)?.filter_map(Result::ok) {
            let path = entry.path().join("project.json");
            let Ok(mut project) = self.load_path(&path) else {
                continue;
            };
            if project.status == "running" {
                project.status = "interrupted".into();
                project.phase = "interrupted".into();
                project.detail =
                    "Kestrel closed during production. Completed masters are safe; resume when ready."
                        .into();
                project.updated_at = Utc::now().to_rfc3339();
                write_json_atomic(&path, &project)?;
            }
        }
        Ok(usize::default()).map(|_| ())
    }
}

struct SelectedClipSource<'a> {
    path: &'a str,
    duration_seconds: f32,
}

fn selected_clip_source<'a>(
    project: &'a MovieProject,
    edit: &ClipEdit,
) -> Result<SelectedClipSource<'a>, StudioError> {
    let clip = project
        .clips
        .iter()
        .find(|clip| clip.id == edit.clip_id)
        .ok_or_else(|| {
            StudioError::Invalid(format!(
                "timeline item {} references unknown clip {}",
                edit.id, edit.clip_id
            ))
        })?;
    if edit.source_version_id.is_empty() {
        return Ok(SelectedClipSource {
            path: &clip.path,
            duration_seconds: clip.duration_seconds,
        });
    }
    let version = clip
        .versions
        .iter()
        .find(|version| version.id == edit.source_version_id)
        .ok_or_else(|| {
            StudioError::Invalid(format!(
                "timeline item {} references missing preserved version {}",
                edit.id, edit.source_version_id
            ))
        })?;
    Ok(SelectedClipSource {
        path: &version.path,
        duration_seconds: version.duration_seconds,
    })
}

fn normalize_movie_project(project: &mut MovieProject) {
    project.schema_version = SCHEMA_VERSION;
    if project.edit.export_preset.is_empty() {
        project.edit.export_preset = default_export_preset();
    }
    let mut known = HashSet::new();
    for (index, edit) in project.edit.clips.iter_mut().enumerate() {
        if edit.id.is_empty() || !known.insert(edit.id.clone()) {
            let base = format!("edit-{}-{}", edit.clip_id, index + 1);
            let mut candidate = base.clone();
            let mut suffix = 2_u32;
            while known.contains(&candidate) {
                candidate = format!("{base}-{suffix}");
                suffix += 1;
            }
            edit.id = candidate.clone();
            known.insert(candidate);
        }
    }
}

fn atempo_filters(speed: f32) -> Vec<f32> {
    let mut remaining = speed;
    let mut filters = Vec::new();
    while remaining < 0.5 - f32::EPSILON {
        filters.push(0.5);
        remaining /= 0.5;
    }
    while remaining > 2.0 + f32::EPSILON {
        filters.push(2.0);
        remaining /= 2.0;
    }
    if (remaining - 1.0).abs() > f32::EPSILON {
        filters.push(remaining);
    }
    filters
}

fn safe_export_stem(title: &str) -> String {
    let mut result = String::new();
    let mut previous_dash = false;
    for character in title.chars().take(96) {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash && !result.is_empty() {
            result.push('-');
            previous_dash = true;
        }
        if result.len() >= 64 {
            break;
        }
    }
    let result = result.trim_matches('-');
    if result.is_empty() {
        "kestrel-movie".into()
    } else {
        result.into()
    }
}

fn check_cancel(cancel: &CancellationToken) -> Result<(), StudioError> {
    if cancel.is_cancelled() {
        Err(StudioError::Cancelled)
    } else {
        Ok(())
    }
}

pub(crate) fn find_output_media(entry: &Value, category: &str) -> Option<(String, String)> {
    let outputs = entry.get("outputs")?.as_object()?;
    let is_video_file = |name: &str| {
        let lower = name.to_lowercase();
        lower.ends_with(".mp4")
            || lower.ends_with(".webm")
            || lower.ends_with(".mov")
            || lower.ends_with(".mkv")
            || lower.ends_with(".avi")
            || lower.ends_with(".gif")
    };
    let is_image_file = |name: &str| {
        let lower = name.to_lowercase();
        lower.ends_with(".png")
            || lower.ends_with(".jpg")
            || lower.ends_with(".jpeg")
            || lower.ends_with(".webp")
    };
    let is_audio_file = |name: &str| {
        let lower = name.to_lowercase();
        lower.ends_with(".flac")
            || lower.ends_with(".wav")
            || lower.ends_with(".mp3")
            || lower.ends_with(".ogg")
            || lower.ends_with(".m4a")
    };

    let fallback = [category];
    let categories: &[&str] = match category {
        "videos" => &["videos", "gifs", "images"],
        "images" => &["images"],
        "audio" => &["audio", "sounds"],
        _ => &fallback,
    };

    for cat in categories {
        for output in outputs.values() {
            if let Some(media_list) = output.get(*cat).and_then(Value::as_array) {
                for media in media_list {
                    if let Some(filename) = media.get("filename").and_then(Value::as_str) {
                        let matches_cat = match category {
                            "videos" => {
                                is_video_file(filename) || *cat == "videos" || *cat == "gifs"
                            }
                            "images" => is_image_file(filename) || *cat == "images",
                            "audio" => is_audio_file(filename) || *cat == "audio",
                            _ => true,
                        };
                        if matches_cat {
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
            .unwrap_or("failed without an exception message")
            .trim();
        return Some(format!(
            "{node_type} node {node_id} failed: {}",
            truncate(exception, 800)
        ));
    }
    None
}

struct ReferenceProbe {
    duration_seconds: f64,
    width: u32,
    height: u32,
    has_audio: bool,
}

fn classify_reference(path: &Path) -> Result<(String, String, u64), StudioError> {
    let extension = reference_extension(path);
    let (kind, mime, limit) = match extension.as_str() {
        "png" => ("image", "image/png", MAX_IMAGE_BYTES),
        "jpg" | "jpeg" => ("image", "image/jpeg", MAX_IMAGE_BYTES),
        "webp" => ("image", "image/webp", MAX_IMAGE_BYTES),
        "bmp" => ("image", "image/bmp", MAX_IMAGE_BYTES),
        "mp4" | "m4v" => ("video", "video/mp4", MAX_REFERENCE_BYTES),
        "mov" => ("video", "video/quicktime", MAX_REFERENCE_BYTES),
        "mkv" => ("video", "video/x-matroska", MAX_REFERENCE_BYTES),
        "webm" => ("video", "video/webm", MAX_REFERENCE_BYTES),
        "wav" => ("audio", "audio/wav", MAX_AUDIO_BYTES),
        "mp3" => ("audio", "audio/mpeg", MAX_AUDIO_BYTES),
        "flac" => ("audio", "audio/flac", MAX_AUDIO_BYTES),
        "ogg" | "oga" | "opus" => ("audio", "audio/ogg", MAX_AUDIO_BYTES),
        "m4a" | "aac" => ("audio", "audio/mp4", MAX_AUDIO_BYTES),
        _ => {
            return Err(StudioError::Invalid(format!(
                "{} is not a supported H3 picture, video, or audio reference",
                reference_name(path)
            )))
        }
    };
    Ok((kind.into(), mime.into(), limit))
}

fn probe_reference(path: &Path, kind: &str) -> Result<ReferenceProbe, StudioError> {
    let output = std::process::Command::new(media_program("ffprobe"))
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|error| {
            StudioError::Invalid(format!(
                "FFprobe is required to validate producer references: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(StudioError::Invalid(format!(
            "{} is not readable media: {}",
            reference_name(path),
            truncate(&String::from_utf8_lossy(&output.stderr), 500)
        )));
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    let streams = value
        .get("streams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let video = streams
        .iter()
        .find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("video"));
    let audio = streams
        .iter()
        .any(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("audio"));
    if matches!(kind, "image" | "video") && video.is_none() {
        return Err(StudioError::Invalid(format!(
            "{} contains no readable visual stream",
            reference_name(path)
        )));
    }
    if kind == "audio" && !audio {
        return Err(StudioError::Invalid(format!(
            "{} contains no readable audio stream",
            reference_name(path)
        )));
    }
    let duration = value
        .pointer("/format/duration")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| {
            streams.iter().find_map(|stream| {
                stream
                    .get("duration")
                    .and_then(Value::as_str)
                    .and_then(|value| value.parse::<f64>().ok())
            })
        })
        .unwrap_or(0.0);
    if kind == "video" && !(0.15..=MAX_REFERENCE_SECONDS).contains(&duration) {
        return Err(StudioError::Invalid(format!(
            "{} is {:.2}s; video references must be between 0.15s and 1 hour",
            reference_name(path),
            duration
        )));
    }
    if kind == "audio" && !(0.15..=MAX_REFERENCE_SECONDS).contains(&duration) {
        return Err(StudioError::Invalid(format!(
            "{} is {:.2}s; audio references must be between 0.15s and 1 hour",
            reference_name(path),
            duration
        )));
    }
    Ok(ReferenceProbe {
        duration_seconds: duration,
        width: video
            .and_then(|stream| stream.get("width"))
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32,
        height: video
            .and_then(|stream| stream.get("height"))
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32,
        has_audio: audio,
    })
}

fn copy_reference_and_hash(
    source: &Path,
    destination: &Path,
    limit: u64,
) -> Result<(String, u64), StudioError> {
    let result = (|| {
        let mut input = fs::File::open(source)?;
        let mut output = fs::File::create(destination)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 1024 * 1024];
        let mut total = 0u64;
        loop {
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            total = total.saturating_add(count as u64);
            if total > limit {
                return Err(StudioError::Invalid(
                    "the reference grew beyond its size limit while importing".into(),
                ));
            }
            hasher.update(&buffer[..count]);
            output.write_all(&buffer[..count])?;
        }
        output.sync_all()?;
        Ok((hex::encode(hasher.finalize()), total))
    })();
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}

pub(crate) fn hash_file(path: &Path) -> Result<(u64, String), StudioError> {
    let mut input = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    let mut bytes = 0_u64;
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

fn hash_reference(path: &Path) -> Result<String, StudioError> {
    hash_file(path).map(|(_, sha256)| sha256)
}

fn reference_extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn reference_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("reference")
        .chars()
        .take(240)
        .collect()
}

fn readable_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{} GiB", bytes / (1024 * 1024 * 1024))
    } else {
        format!("{} MiB", bytes / (1024 * 1024))
    }
}

fn validate_id(id: &str) -> Result<(), StudioError> {
    if uuid::Uuid::parse_str(id).is_err() {
        Err(StudioError::Invalid("invalid movie project id".into()))
    } else {
        Ok(())
    }
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), StudioError> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    let backup = path.with_extension("json.backup");
    if path.exists() {
        fs::copy(path, &backup)?;
        fs::remove_file(path)?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.is_file() {
            let _ = fs::copy(&backup, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

/// Verify a local endpoint before attaching, including a server started by another application.
pub(crate) async fn verify_comfy_installation(
    http: &reqwest::Client,
    base: &str,
    selected_root: &Path,
) -> Result<(), StudioError> {
    let info: Value = http
        .get(format!("{base}/system_stats"))
        .timeout(Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    validate_comfy_installation(&info, base, selected_root)
}

pub(crate) fn validate_comfy_installation(
    info: &Value,
    base: &str,
    selected_root: &Path,
) -> Result<(), StudioError> {
    let selected_main = selected_root.join("main.py").canonicalize()?;
    let running_main = info.pointer("/system/argv/0").and_then(Value::as_str);
    let verified_main = running_main
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .and_then(|path| path.canonicalize().ok());
    if verified_main.as_ref() != Some(&selected_main) {
        return Err(StudioError::Render(format!(
            "ComfyUI at {base} is running {} instead of the installation selected in Setup ({}). Stop the existing ComfyUI server, then try again so Kestrel can start the selected installation.",
            running_main.unwrap_or("from an unverified folder"), selected_root.display(),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestrel_app_core::CreateMovieH3ProjectRequest;
    use tempfile::tempdir;

    #[test]
    #[ignore = "needs FFprobe on PATH or KESTREL_FFPROBE_PATH"]
    fn an_image_studio_take_keeps_its_readable_name_in_the_reference_library() {
        const ONE_PIXEL_PNG: &[u8] = &[
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        let root = tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let take = root.path().join("take-001-preview.png");
        fs::write(&take, ONE_PIXEL_PNG).unwrap();

        let asset = studio
            .import_reference_path_named(&take, Some("Night / Form · take 1"))
            .unwrap();
        assert_eq!(asset.name, "Night / Form · take 1");
        assert!(Path::new(&asset.path).is_file());
        // The library is content-addressed: the same take imported again is the same reference.
        assert_eq!(studio.import_reference_path(&take).unwrap().id, asset.id);
    }

    #[test]
    fn corrupt_project_json_recovers_from_the_last_complete_copy() {
        let root = tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let mut project = studio
            .create_h3_project(
                CreateMovieH3ProjectRequest {
                    title: "Fog Signal".into(),
                    story_markdown: "A keeper waits beside a fog signal.".into(),
                    width: None,
                    height: None,
                    duration_sec: None,
                },
                &root.path().to_string_lossy(),
                false,
            )
            .unwrap();
        project.detail = "second durable state".into();
        studio.save(&project).unwrap();
        let path = studio.project_dir(&project.id).join("project.json");
        fs::write(&path, b"{broken").unwrap();

        let recovered = studio.get(&project.id).unwrap();
        assert_eq!(recovered.id, project.id);
        assert_ne!(recovered.detail, "second durable state");
        assert!(serde_json::from_slice::<Value>(&fs::read(path).unwrap()).is_ok());
    }

    #[test]
    fn retired_project_payloads_round_trip_without_becoming_current_authority() {
        let root = tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let project = studio
            .create_h3_project(
                CreateMovieH3ProjectRequest {
                    title: "Fog Signal".into(),
                    story_markdown: "A keeper waits beside a fog signal.".into(),
                    width: None,
                    height: None,
                    duration_sec: None,
                },
                &root.path().to_string_lossy(),
                false,
            )
            .unwrap();
        let mut value = serde_json::to_value(project).unwrap();
        value["modelRoles"] = json!({"oldRole":{"modelId":"preserved"}});
        value["producerFeedback"] = json!([{"feedback":"preserved note"}]);
        value["copilotHistory"] = json!([{"response":"preserved response"}]);

        let loaded: MovieProject = serde_json::from_value(value).unwrap();
        assert_eq!(loaded.legacy_model_roles["oldRole"]["modelId"], "preserved");
        let saved = serde_json::to_value(loaded).unwrap();
        assert_eq!(saved["producerFeedback"][0]["feedback"], "preserved note");
        assert_eq!(saved["copilotHistory"][0]["response"], "preserved response");
    }
}
