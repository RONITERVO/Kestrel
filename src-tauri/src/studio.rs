use crate::{
    kiwix::{KiwixClient, SNAPSHOT},
    models::ResearchSettings,
    runtime::{authorized, ModelConnection},
};
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use thiserror::Error;
use tokio::{process::Child, sync::Mutex};
use tokio_util::sync::CancellationToken;

const SCHEMA_VERSION: u32 = 2;
const COMFY_BASE: &str = "http://127.0.0.1:8188";
const DEFAULT_COMFY_ROOT: &str = r"D:\AI\ComfyUI";
const MAX_REFERENCE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_AUDIO_BYTES: u64 = 256 * 1024 * 1024;
const MAX_REFERENCE_SECONDS: f64 = 15.1;

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
    let root = directories::UserDirs::new()
        .map(|dirs| dirs.home_dir().join("Kestrel Research").join("movies"))
        .ok_or_else(|| (500, "local movie library is unavailable".to_string()))?;
    let canonical_root = fs::canonicalize(&root)
        .map_err(|_| (404, "local movie library does not exist".to_string()))?;
    let target = fs::canonicalize(root.join(relative.as_ref()))
        .map_err(|_| (404, "movie media was not found".to_string()))?;
    if !target.starts_with(&canonical_root) || !target.is_file() {
        return Err((403, "movie media is outside the private library".into()));
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
        "ogg" | "oga" => "audio/ogg",
        "m4a" | "aac" => "audio/mp4",
        _ => "application/octet-stream",
    };
    let mut builder = tauri::http::Response::builder()
        .header("Content-Type", content_type)
        .header("Access-Control-Allow-Origin", "*")
        .header("Accept-Ranges", "bytes");
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
        builder.body(body).map_err(|error| (500, error.to_string()))
    } else {
        let mut body = Vec::with_capacity(length.min(16 * 1024 * 1024) as usize);
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
    #[error("Bonsai could not produce a movie plan: {0}")]
    Planning(String),
    #[error("MiniMax H3 render failed: {0}")]
    Render(String),
    #[error("movie production was stopped")]
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartMovieRequest {
    pub prompt: String,
    #[serde(default)]
    pub settings: MovieSettings,
    #[serde(default)]
    pub references: Vec<ProducerReferenceRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerReferenceRequest {
    pub asset_id: String,
    pub description: String,
    #[serde(default)]
    pub use_embedded_audio: bool,
    #[serde(default)]
    pub embedded_audio_description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieReferenceAsset {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub mime_type: String,
    pub bytes: u64,
    pub duration_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
    pub path: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieReference {
    pub asset_id: String,
    pub tag: String,
    #[serde(default)]
    pub audio_tag: String,
    pub name: String,
    pub kind: String,
    pub mime_type: String,
    pub bytes: u64,
    pub duration_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub has_audio: bool,
    pub path: String,
    pub description: String,
    #[serde(default)]
    pub use_embedded_audio: bool,
    #[serde(default)]
    pub embedded_audio_description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieReferenceImport {
    pub references: Vec<MovieReferenceAsset>,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieSettings {
    #[serde(default = "default_research_mode")]
    pub research_mode: String,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_clip_seconds")]
    pub clip_seconds: f32,
    #[serde(default = "default_steps")]
    pub steps: u32,
    #[serde(default = "default_max_clips")]
    pub max_clips: u32,
    #[serde(default)]
    pub seed: u64,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_top_p")]
    pub top_p: f32,
    #[serde(default = "default_top_k")]
    pub top_k: u32,
    #[serde(default = "default_thinking")]
    pub thinking_budget: u32,
    #[serde(default = "default_output")]
    pub max_output_tokens: u32,
    #[serde(default = "default_comfy_root")]
    pub comfy_root: String,
    #[serde(default = "default_ref_image_size")]
    pub ref_image_size: String,
}

impl Default for MovieSettings {
    fn default() -> Self {
        Self {
            research_mode: default_research_mode(),
            width: default_width(),
            height: default_height(),
            clip_seconds: default_clip_seconds(),
            steps: default_steps(),
            max_clips: default_max_clips(),
            seed: 0,
            temperature: default_temperature(),
            top_p: default_top_p(),
            top_k: default_top_k(),
            thinking_budget: default_thinking(),
            max_output_tokens: default_output(),
            comfy_root: default_comfy_root(),
            ref_image_size: default_ref_image_size(),
        }
    }
}

fn default_research_mode() -> String {
    "auto".into()
}
fn default_width() -> u32 {
    1_344
}
fn default_height() -> u32 {
    768
}
fn default_clip_seconds() -> f32 {
    5.0
}
fn default_steps() -> u32 {
    20
}
fn default_max_clips() -> u32 {
    12
}
fn default_temperature() -> f32 {
    0.7
}
fn default_top_p() -> f32 {
    0.95
}
fn default_top_k() -> u32 {
    20
}
fn default_thinking() -> u32 {
    4_096
}
fn default_output() -> u32 {
    32_768
}
fn default_comfy_root() -> String {
    DEFAULT_COMFY_ROOT.into()
}
fn default_ref_image_size() -> String {
    "match".into()
}

impl MovieSettings {
    pub fn validate(mut self, advanced: bool) -> Result<Self, StudioError> {
        if !matches!(self.research_mode.as_str(), "auto" | "never" | "always") {
            return Err(StudioError::Invalid(
                "research mode must be auto, never, or always".into(),
            ));
        }
        if !self.width.is_multiple_of(32) || !self.height.is_multiple_of(32) {
            return Err(StudioError::Invalid(
                "H3 width and height must be multiples of 32".into(),
            ));
        }
        let maximum_edge = if advanced { 2_048 } else { 1_344 };
        if self.width < 320
            || self.height < 320
            || self.width > maximum_edge
            || self.height > maximum_edge
        {
            return Err(StudioError::Invalid(format!(
                "resolution must be between 320 and {maximum_edge} pixels per edge"
            )));
        }
        self.clip_seconds = self.clip_seconds.clamp(5.0, 15.0);
        self.steps = self.steps.clamp(1, if advanced { 100 } else { 40 });
        self.max_clips = self.max_clips.clamp(1, if advanced { 96 } else { 24 });
        self.temperature = self.temperature.clamp(0.0, 2.0);
        self.top_p = self.top_p.clamp(0.05, 1.0);
        self.top_k = self.top_k.clamp(1, 200);
        self.thinking_budget = self.thinking_budget.clamp(0, 32_768);
        self.max_output_tokens = self.max_output_tokens.clamp(1_024, 32_768);
        let root = PathBuf::from(&self.comfy_root);
        if !root.is_absolute() {
            return Err(StudioError::Invalid(
                "ComfyUI root must be an absolute local path".into(),
            ));
        }
        if !matches!(self.ref_image_size.as_str(), "match" | "max") {
            return Err(StudioError::Invalid(
                "reference image size must be match or max".into(),
            ));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoviePlan {
    pub title: String,
    pub logline: String,
    pub audience: String,
    pub creative_direction: String,
    #[serde(default)]
    pub continuity_bible: Vec<String>,
    #[serde(default)]
    pub source_credits: Vec<String>,
    pub clips: Vec<PlannedClip>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedClip {
    #[serde(default)]
    pub id: String,
    pub title: String,
    pub purpose: String,
    pub duration_seconds: f32,
    pub prompt: String,
    pub continuity_in: String,
    pub continuity_out: String,
    pub transition: String,
    pub use_previous_frame: bool,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub reference_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieSource {
    pub id: String,
    pub title: String,
    pub reference: String,
    pub snapshot: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedClip {
    pub id: String,
    pub index: u32,
    pub title: String,
    pub prompt: String,
    pub duration_seconds: f32,
    pub seed: u64,
    pub status: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipEdit {
    pub clip_id: String,
    pub enabled: bool,
    pub order: u32,
    pub trim_start: f32,
    pub trim_end: f32,
    #[serde(default = "default_gain")]
    pub audio_gain: f32,
}

fn default_gain() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieEdit {
    #[serde(default)]
    pub clips: Vec<ClipEdit>,
    #[serde(default = "default_export_title")]
    pub export_title: String,
}

fn default_export_title() -> String {
    "Kestrel Movie".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieProject {
    pub schema_version: u32,
    pub id: String,
    pub prompt: String,
    pub title: String,
    pub status: String,
    pub phase: String,
    pub detail: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub renderer: String,
    pub settings: MovieSettings,
    #[serde(default)]
    pub references: Vec<MovieReference>,
    #[serde(default)]
    pub plan: Option<MoviePlan>,
    #[serde(default)]
    pub sources: Vec<MovieSource>,
    #[serde(default)]
    pub clips: Vec<RenderedClip>,
    pub edit: MovieEdit,
    #[serde(default)]
    pub final_path: String,
    #[serde(default)]
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    pub phase: String,
    pub updated_at: String,
    pub clip_count: usize,
    pub final_path: String,
}

impl From<&MovieProject> for MovieSummary {
    fn from(project: &MovieProject) -> Self {
        Self {
            id: project.id.clone(),
            title: project.title.clone(),
            status: project.status.clone(),
            phase: project.phase.clone(),
            updated_at: project.updated_at.clone(),
            clip_count: project.clips.len(),
            final_path: project.final_path.clone(),
        }
    }
}

#[derive(Clone)]
pub struct MovieStudio {
    root: PathBuf,
    http: Client,
    kiwix: KiwixClient,
    comfy_child: Arc<Mutex<Option<Child>>>,
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
            kiwix: KiwixClient::new(),
            comfy_child: Arc::new(Mutex::new(None)),
        };
        studio.recover_interrupted()?;
        Ok(studio)
    }

    pub fn import_reference_path(&self, source: &Path) -> Result<MovieReferenceAsset, StudioError> {
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
            name: reference_name(&source),
            kind,
            mime_type,
            bytes,
            duration_seconds: probe.duration_seconds,
            width: probe.width,
            height: probe.height,
            has_audio: probe.has_audio,
            path: object_path.to_string_lossy().into_owned(),
            created_at: Utc::now().to_rfc3339(),
        };
        write_json_atomic(&meta_path, &asset)?;
        Ok(asset)
    }

    pub fn create(
        &self,
        request: StartMovieRequest,
        advanced: bool,
    ) -> Result<MovieProject, StudioError> {
        let StartMovieRequest {
            prompt,
            settings,
            references,
        } = request;
        let meaningful_prompt = prompt.trim();
        if meaningful_prompt.chars().count() < 3 || prompt.len() > 65_536 {
            return Err(StudioError::Invalid(
                "movie prompt must be between 3 characters and 64 KiB".into(),
            ));
        }
        let settings = settings.validate(advanced)?;
        let now = Utc::now().to_rfc3339();
        let id = uuid::Uuid::new_v4().to_string();
        let folder = self.project_dir(&id);
        fs::create_dir_all(folder.join("raw"))?;
        fs::create_dir_all(folder.join("exports"))?;
        fs::create_dir_all(folder.join("references"))?;
        let references = match self.materialize_references(&id, references) {
            Ok(references) => references,
            Err(error) => {
                let _ = fs::remove_dir_all(&folder);
                return Err(error);
            }
        };
        let project = MovieProject {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            prompt,
            title: "Untitled movie".into(),
            status: "running".into(),
            phase: "planning".into(),
            detail: "Bonsai is shaping the story, continuity, and production plan.".into(),
            created_at: now.clone(),
            updated_at: now,
            model: "Ternary Bonsai 27B Q2_0".into(),
            renderer: "MiniMax H3 / ComfyUI native".into(),
            settings,
            references,
            plan: None,
            sources: Vec::new(),
            clips: Vec::new(),
            edit: MovieEdit {
                clips: Vec::new(),
                export_title: "Kestrel Movie".into(),
            },
            final_path: String::new(),
            error: String::new(),
        };
        write_json_atomic(
            &folder.join("request.json"),
            &json!({"prompt":project.prompt,"settings":project.settings,"references":project.references,"createdAt":project.created_at}),
        )?;
        write_json_atomic(&folder.join("references.json"), &project.references)?;
        self.save(&project)?;
        Ok(project)
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

    fn materialize_references(
        &self,
        project_id: &str,
        requests: Vec<ProducerReferenceRequest>,
    ) -> Result<Vec<MovieReference>, StudioError> {
        let mut seen = HashSet::new();
        let mut prepared = Vec::new();
        for request in requests {
            if !seen.insert(request.asset_id.clone()) {
                return Err(StudioError::Invalid(
                    "the same producer reference cannot be attached twice".into(),
                ));
            }
            let description = request.description.trim();
            if description.len() < 3 || description.len() > 4_000 {
                return Err(StudioError::Invalid(
                    "describe what each reference should control in 3 to 4,000 characters".into(),
                ));
            }
            if contains_reference_tag(description)
                || contains_reference_tag(&request.embedded_audio_description)
            {
                return Err(StudioError::Invalid(
                    "reference descriptions cannot contain reserved H3 tags such as <Picture 1> or <Audio 1>"
                        .into(),
                ));
            }
            let asset = self.resolve_reference_asset(&request.asset_id)?;
            if request.use_embedded_audio && (asset.kind != "video" || !asset.has_audio) {
                return Err(StudioError::Invalid(format!(
                    "{} has no embedded audio track to use",
                    asset.name
                )));
            }
            if request.use_embedded_audio && request.embedded_audio_description.trim().len() < 3 {
                return Err(StudioError::Invalid(format!(
                    "describe what the embedded audio from {} should control",
                    asset.name
                )));
            }
            prepared.push((asset, request));
        }
        let images = prepared
            .iter()
            .filter(|(asset, _)| asset.kind == "image")
            .count();
        let videos = prepared
            .iter()
            .filter(|(asset, _)| asset.kind == "video")
            .count();
        let embedded_audio = prepared
            .iter()
            .filter(|(asset, request)| asset.kind == "video" && request.use_embedded_audio)
            .count();
        let standalone_audio = prepared
            .iter()
            .filter(|(asset, _)| asset.kind == "audio")
            .count();
        if images > 9 || videos > 3 || embedded_audio + standalone_audio > 3 {
            return Err(StudioError::Invalid(
                "H3 supports at most 9 pictures, 3 videos, and 3 audio signals per movie".into(),
            ));
        }
        let project_root = self.project_dir(project_id).join("references");
        let mut picture_index = 0usize;
        let mut video_index = 0usize;
        let mut embedded_index = 0usize;
        let mut standalone_index = 0usize;
        let mut result = Vec::with_capacity(prepared.len());
        for (asset, request) in prepared {
            let (tag, audio_tag, file_stem) = match asset.kind.as_str() {
                "image" => {
                    picture_index += 1;
                    (
                        format!("<Picture {picture_index}>"),
                        String::new(),
                        format!("picture-{picture_index:02}"),
                    )
                }
                "video" => {
                    video_index += 1;
                    let audio_tag = if request.use_embedded_audio {
                        embedded_index += 1;
                        format!("<Audio {embedded_index}>")
                    } else {
                        String::new()
                    };
                    (
                        format!("<Video {video_index}>"),
                        audio_tag,
                        format!("video-{video_index:02}"),
                    )
                }
                "audio" => {
                    standalone_index += 1;
                    let index = embedded_audio + standalone_index;
                    (
                        format!("<Audio {index}>"),
                        String::new(),
                        format!("audio-{index:02}"),
                    )
                }
                _ => unreachable!("validated reference kind"),
            };
            let extension = Path::new(&asset.path)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("bin");
            let target =
                project_root.join(format!("{file_stem}-{}.{}", &asset.id[..12], extension));
            if fs::hard_link(&asset.path, &target).is_err() {
                fs::copy(&asset.path, &target)?;
            }
            result.push(MovieReference {
                asset_id: asset.id,
                tag,
                audio_tag,
                name: asset.name,
                kind: asset.kind,
                mime_type: asset.mime_type,
                bytes: asset.bytes,
                duration_seconds: asset.duration_seconds,
                width: asset.width,
                height: asset.height,
                has_audio: asset.has_audio,
                path: target.to_string_lossy().into_owned(),
                description: request.description.trim().into(),
                use_embedded_audio: request.use_embedded_audio,
                embedded_audio_description: request.embedded_audio_description.trim().into(),
            });
        }
        Ok(result)
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
            .map_err(|error| if matches!(error, StudioError::Io(ref io) if io.kind() == std::io::ErrorKind::NotFound) { StudioError::NotFound(id.into()) } else { error })
    }

    pub fn save_edits(&self, id: &str, edit: MovieEdit) -> Result<MovieProject, StudioError> {
        let mut project = self.get(id)?;
        for item in &edit.clips {
            if !project.clips.iter().any(|clip| clip.id == item.clip_id) {
                return Err(StudioError::Invalid(format!(
                    "unknown clip in edit: {}",
                    item.clip_id
                )));
            }
            if item.trim_start < 0.0
                || item.trim_end < 0.0
                || item.audio_gain < 0.0
                || item.audio_gain > 4.0
            {
                return Err(StudioError::Invalid(
                    "clip trims and audio gain are outside supported bounds".into(),
                ));
            }
        }
        project.edit = edit;
        project.updated_at = Utc::now().to_rfc3339();
        self.save(&project)?;
        Ok(project)
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
        project.status = "cancelled".into();
        project.phase = "cancelled".into();
        project.detail =
            "Production stopped. The plan and every completed master are safe to resume.".into();
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub fn begin_resume(
        &self,
        id: &str,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let mut project = self.get(id)?;
        project.status = "running".into();
        project.phase = "resuming".into();
        project.error.clear();
        project.detail = "Resuming from the last preserved H3 master.".into();
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

    pub async fn plan(
        &self,
        id: &str,
        connection: &ModelConnection,
        research: &ResearchSettings,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let mut project = self.get(id)?;
        let user_prompt = project.prompt.clone();
        let movie_settings = project.settings.clone();
        let (mut plan, sources) = self
            .direct(
                &user_prompt,
                &movie_settings,
                connection,
                research,
                cancel,
                (&mut project, app),
            )
            .await?;
        if plan.clips.is_empty() {
            return Err(StudioError::Planning(
                "the returned plan contained no clips".into(),
            ));
        }
        plan.clips.truncate(project.settings.max_clips as usize);
        let allowed_references = project
            .references
            .iter()
            .map(|reference| reference.asset_id.as_str())
            .collect::<HashSet<_>>();
        for (index, clip) in plan.clips.iter_mut().enumerate() {
            clip.id = format!("clip-{:03}", index + 1);
            clip.duration_seconds = clip.duration_seconds.clamp(5.0, 15.0);
            if clip.prompt.trim().len() < 20 {
                return Err(StudioError::Planning(format!(
                    "clip {} has no usable H3 prompt",
                    index + 1
                )));
            }
            let mut seen_references = HashSet::new();
            clip.reference_ids.retain(|reference| {
                allowed_references.contains(reference.as_str())
                    && seen_references.insert(reference.clone())
            });
            if !clip.reference_ids.is_empty() {
                // H3's native ref2va node owns the conditioning path for this shot. It cannot be
                // combined with fl2va's prior-frame input in the same native graph.
                clip.use_previous_frame = false;
            }
        }
        if !project.references.is_empty() {
            let selected = plan
                .clips
                .iter()
                .flat_map(|clip| clip.reference_ids.iter())
                .cloned()
                .collect::<HashSet<_>>();
            let missing = project
                .references
                .iter()
                .filter(|reference| !selected.contains(&reference.asset_id))
                .map(|reference| reference.asset_id.clone())
                .collect::<Vec<_>>();
            if let Some(first) = plan.clips.first_mut() {
                first.reference_ids.extend(missing);
                if !first.reference_ids.is_empty() {
                    first.use_previous_frame = false;
                }
            }
        }
        project.title.clone_from(&plan.title);
        project.edit.export_title.clone_from(&plan.title);
        project.sources = sources;
        project.clips = plan
            .clips
            .iter()
            .enumerate()
            .map(|(index, clip)| RenderedClip {
                id: clip.id.clone(),
                index: index as u32,
                title: clip.title.clone(),
                prompt: clip.prompt.clone(),
                duration_seconds: clip.duration_seconds,
                seed: derive_seed(project.settings.seed, index as u64),
                status: "queued".into(),
                path: String::new(),
                error: String::new(),
            })
            .collect();
        project.edit.clips = project
            .clips
            .iter()
            .map(|clip| ClipEdit {
                clip_id: clip.id.clone(),
                enabled: true,
                order: clip.index,
                trim_start: 0.0,
                trim_end: 0.0,
                audio_gain: 1.0,
            })
            .collect();
        project.plan = Some(plan.clone());
        project.phase = "plan-ready".into();
        project.detail = format!(
            "The production plan is ready with {} H3 clips.",
            project.clips.len()
        );
        self.persist_emit(&mut project, app)?;
        let folder = self.project_dir(id);
        write_json_atomic(&folder.join("plan.json"), &plan)?;
        write_json_atomic(&folder.join("sources.json"), &project.sources)?;
        Ok(project)
    }

    async fn direct(
        &self,
        prompt: &str,
        settings: &MovieSettings,
        connection: &ModelConnection,
        research: &ResearchSettings,
        cancel: &CancellationToken,
        progress: (&mut MovieProject, Option<&AppHandle>),
    ) -> Result<(MoviePlan, Vec<MovieSource>), StudioError> {
        let (project, app) = progress;
        let system = director_prompt(settings);
        let mut messages = vec![
            json!({"role":"system","content":system}),
            json!({"role":"user","content":prompt}),
        ];
        if !project.references.is_empty() {
            messages.push(json!({"role":"user","content":reference_manifest(&project.references)}));
        }
        let tools = movie_tools();
        let mut sources = Vec::<MovieSource>::new();
        let use_tools = settings.research_mode != "never";
        if use_tools {
            for _ in 0..6 {
                check_cancel(cancel)?;
                let response = self
                    .complete(
                        connection,
                        &messages,
                        CompletionOptions {
                            tools: Some(&tools),
                            response_format: None,
                            max_tokens: 1_600,
                            settings,
                            research,
                        },
                    )
                    .await?;
                let assistant = response_message(&response)?;
                let calls = assistant
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                messages.push(assistant);
                if calls.is_empty() {
                    break;
                }
                let call = &calls[0];
                let id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool-call");
                let function = call.get("function").unwrap_or(&Value::Null);
                let name = function
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments: Value = serde_json::from_str(
                    function
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("{}"),
                )?;
                let output = match name {
                    "search_archive" => {
                        let query = arguments
                            .get("query")
                            .and_then(Value::as_str)
                            .unwrap_or(prompt);
                        let results = self
                            .kiwix
                            .search(query, 6)
                            .await
                            .map_err(|error| StudioError::Planning(error.to_string()))?;
                        project.phase = "researching".into();
                        project.detail = format!(
                            "Bonsai searched the offline January 2024 archive for “{query}”."
                        );
                        self.persist_emit(project, app)?;
                        serde_json::to_string(&results)?
                    }
                    "read_source" => {
                        let reference = arguments
                            .get("source_ref")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let section = arguments.get("section").and_then(Value::as_str);
                        let article = self
                            .kiwix
                            .read(reference, section, 6_000)
                            .await
                            .map_err(|error| StudioError::Planning(error.to_string()))?;
                        let source_id = format!("S{}", sources.len() + 1);
                        sources.push(MovieSource {
                            id: source_id.clone(),
                            title: article.title.clone(),
                            reference: article.reference.clone(),
                            snapshot: SNAPSHOT.into(),
                            excerpt: article.text.chars().take(900).collect(),
                        });
                        project.phase = "researching".into();
                        project.detail = format!(
                            "Bonsai opened “{}” from the offline archive.",
                            article.title
                        );
                        self.persist_emit(project, app)?;
                        format!("Evidence ID: {source_id}\nTitle: {}\nSnapshot: {SNAPSHOT}\nSource ref: {}\n\n{}", article.title, article.reference, article.text)
                    }
                    _ => "Unknown tool. Available tools are search_archive and read_source.".into(),
                };
                messages.push(json!({"role":"tool","tool_call_id":id,"content":output}));
            }
        }
        if settings.research_mode == "always" && sources.is_empty() {
            let result = self
                .kiwix
                .search(prompt, 1)
                .await
                .map_err(|error| StudioError::Planning(error.to_string()))?;
            if let Some(first) = result.first() {
                let article = self
                    .kiwix
                    .read(&first.reference, None, 6_000)
                    .await
                    .map_err(|error| StudioError::Planning(error.to_string()))?;
                let source = MovieSource {
                    id: "S1".into(),
                    title: article.title.clone(),
                    reference: article.reference.clone(),
                    snapshot: SNAPSHOT.into(),
                    excerpt: article.text.chars().take(900).collect(),
                };
                messages.push(json!({"role":"user","content":format!("Required opened archive evidence S1 (snapshot {SNAPSHOT}), {}:\n{}", article.title, article.text)}));
                sources.push(source);
            }
        }
        check_cancel(cancel)?;
        project.phase = "writing".into();
        project.detail =
            "Bonsai is committing the screenplay, continuity bible, and H3 shot prompts.".into();
        self.persist_emit(project, app)?;
        messages.push(json!({"role":"user","content":"Deliver the complete movie plan now in the required JSON shape."}));
        let response = self
            .complete(
                connection,
                &messages,
                CompletionOptions {
                    tools: None,
                    response_format: Some(movie_schema(settings.max_clips)),
                    max_tokens: settings.max_output_tokens,
                    settings,
                    research,
                },
            )
            .await?;
        let content = response_message(&response)?
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let plan = serde_json::from_str(&content)
            .map_err(|error| StudioError::Planning(format!("invalid structured plan: {error}")))?;
        Ok((plan, sources))
    }

    async fn complete(
        &self,
        connection: &ModelConnection,
        messages: &[Value],
        options: CompletionOptions<'_>,
    ) -> Result<Value, StudioError> {
        let CompletionOptions {
            tools,
            response_format,
            max_tokens,
            settings,
            research,
        } = options;
        let mut body = json!({
            "model": connection.model_id, "messages": messages, "stream": false,
            "temperature": settings.temperature, "top_p": settings.top_p, "top_k": settings.top_k,
            "max_tokens": max_tokens.min(research.max_output_tokens),
            "thinking_budget_tokens": settings.thinking_budget,
        });
        if let Some(tools) = tools {
            body["tools"] = tools.clone();
            body["tool_choice"] = json!("auto");
            body["parallel_tool_calls"] = json!(false);
        }
        if let Some(format) = response_format {
            body["response_format"] = format;
        }
        let response = authorized(
            self.http
                .post(format!("{}/chat/completions", connection.endpoint)),
            connection,
        )
        .json(&body)
        .send()
        .await?;
        let status = response.status();
        let text = response.text().await?;
        if !status.is_success() {
            return Err(StudioError::Planning(format!(
                "HTTP {status}: {}",
                truncate(&text, 500)
            )));
        }
        serde_json::from_str(&text).map_err(StudioError::from)
    }

    pub async fn render(
        &self,
        id: &str,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let mut project = self.get(id)?;
        let comfy_root = project.settings.comfy_root.clone();
        self.ensure_comfy(&comfy_root, &mut project, app).await?;
        let plan = project
            .plan
            .clone()
            .ok_or_else(|| StudioError::Invalid("project has no saved plan".into()))?;
        for (index, planned) in plan.clips.iter().enumerate() {
            check_cancel(cancel)?;
            if project
                .clips
                .get(index)
                .is_some_and(|clip| clip.status == "complete" && Path::new(&clip.path).is_file())
            {
                continue;
            }
            project.phase = "rendering".into();
            project.detail = format!(
                "Rendering clip {} of {} — {}",
                index + 1,
                plan.clips.len(),
                planned.title
            );
            if let Some(clip) = project.clips.get_mut(index) {
                clip.status = "rendering".into();
            }
            self.persist_emit(&mut project, app)?;
            let seed = project.clips[index].seed;
            match self
                .render_clip(&project, planned, index, seed, cancel)
                .await
            {
                Ok(path) => {
                    let clip = &mut project.clips[index];
                    clip.status = "complete".into();
                    clip.path = path;
                    clip.error.clear();
                    self.extract_last_frame(&project, index).await?;
                    self.persist_emit(&mut project, app)?;
                }
                Err(error) => {
                    let clip = &mut project.clips[index];
                    clip.status = "failed".into();
                    clip.error = error.to_string();
                    self.persist_emit(&mut project, app)?;
                    return Err(error);
                }
            }
        }
        project.phase = "assembling".into();
        project.detail = "Assembling immutable H3 masters into the first publishable cut.".into();
        self.persist_emit(&mut project, app)?;
        let final_path = self.assemble_default(&project).await?;
        project.final_path = final_path;
        project.status = "complete".into();
        project.phase = "complete".into();
        project.detail = "The first cut is ready. Every source clip remains available for non-destructive editing.".into();
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    async fn ensure_comfy(
        &self,
        root: &str,
        project: &mut MovieProject,
        app: Option<&AppHandle>,
    ) -> Result<(), StudioError> {
        if self.comfy_ready().await {
            return Ok(());
        }
        let root = PathBuf::from(root);
        let script = root.join("Start-ComfyUI-MiniMax-H3.ps1");
        if !script.is_file() {
            return Err(StudioError::Render(format!(
                "MiniMax H3 starter is missing: {}",
                script.display()
            )));
        }
        project.phase = "starting-renderer".into();
        project.detail = "Starting the private ComfyUI MiniMax H3 renderer.".into();
        self.persist_emit(project, app)?;
        let logs = self.project_dir(&project.id).join("logs");
        fs::create_dir_all(&logs)?;
        let stdout = fs::File::create(logs.join("comfy.stdout.log"))?;
        let stderr = fs::File::create(logs.join("comfy.stderr.log"))?;
        let mut command = tokio::process::Command::new("powershell.exe");
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .args(["-Port", "8188", "-NoBrowser"])
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .kill_on_drop(false);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let child = command.spawn()?;
        *self.comfy_child.lock().await = Some(child);
        for _ in 0..180 {
            if self.comfy_ready().await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        Err(StudioError::Render(
            "ComfyUI did not become ready within six minutes; see the project logs".into(),
        ))
    }

    async fn comfy_ready(&self) -> bool {
        self.http
            .get(format!("{COMFY_BASE}/system_stats"))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    async fn render_clip(
        &self,
        project: &MovieProject,
        planned: &PlannedClip,
        index: usize,
        seed: u64,
        cancel: &CancellationToken,
    ) -> Result<String, StudioError> {
        let prefix = format!("kestrel_movies/{}/shot_{:03}", project.id, index + 1);
        let selected_ids = planned.reference_ids.iter().collect::<HashSet<_>>();
        let selected_references = project
            .references
            .iter()
            .filter(|reference| selected_ids.contains(&reference.asset_id))
            .collect::<Vec<_>>();
        let mut graph_references = Vec::with_capacity(selected_references.len());
        for reference in &selected_references {
            let file_name = Path::new(&reference.path)
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| StudioError::Render("invalid project reference filename".into()))?;
            let relative = format!("kestrel/{}/references/{file_name}", project.id);
            let target = PathBuf::from(&project.settings.comfy_root)
                .join("input")
                .join(&relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            tokio::fs::copy(&reference.path, &target).await?;
            graph_references.push(H3ReferenceInput {
                kind: reference.kind.as_str(),
                file: relative,
                description: reference.description.as_str(),
                use_embedded_audio: reference.use_embedded_audio,
                embedded_audio_description: reference.embedded_audio_description.as_str(),
            });
        }
        let continuity_input =
            if graph_references.is_empty() && index > 0 && planned.use_previous_frame {
                let source = self
                    .project_dir(&project.id)
                    .join("stills")
                    .join(format!("clip-{:03}-last.png", index));
                if source.is_file() {
                    let relative = format!("kestrel/{}/clip-{:03}-last.png", project.id, index);
                    let target = PathBuf::from(&project.settings.comfy_root)
                        .join("input")
                        .join(&relative);
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    tokio::fs::copy(source, target).await?;
                    Some(relative)
                } else {
                    None
                }
            } else {
                None
            };
        let render_prompt = if graph_references.is_empty() {
            planned.prompt.clone()
        } else {
            format!(
                "{}\n\n{}",
                bound_reference_prompt(&graph_references),
                planned.prompt
            )
        };
        let graph = h3_graph(H3GraphRequest {
            prompt: &render_prompt,
            width: project.settings.width,
            height: project.settings.height,
            seconds: planned.duration_seconds,
            steps: project.settings.steps,
            seed,
            prefix: &prefix,
            first_frame: continuity_input.as_deref(),
            references: &graph_references,
            ref_image_size: &project.settings.ref_image_size,
        });
        let response = self
            .http
            .post(format!("{COMFY_BASE}/prompt"))
            .json(&json!({"prompt":graph,"client_id":format!("kestrel-{}",project.id)}))
            .send()
            .await?;
        let status = response.status();
        let value: Value = response.json().await?;
        if !status.is_success() {
            return Err(StudioError::Render(format!(
                "ComfyUI rejected clip: {}",
                truncate(&value.to_string(), 700)
            )));
        }
        let prompt_id = value
            .get("prompt_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                StudioError::Render(format!("ComfyUI returned no prompt id: {value}"))
            })?;
        loop {
            if cancel.is_cancelled() {
                let _ = self
                    .http
                    .post(format!("{COMFY_BASE}/interrupt"))
                    .send()
                    .await;
                return Err(StudioError::Cancelled);
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
                    let detail = comfy_execution_error(entry).unwrap_or_else(|| {
                        format!("execution failed: {}", truncate(&entry.to_string(), 1_000))
                    });
                    return Err(StudioError::Render(format!(
                        "ComfyUI {detail}. The project and completed masters are safe; review the error, then resume."
                    )));
                }
                if entry.pointer("/status/completed").and_then(Value::as_bool) == Some(true) {
                    let media = find_output_media(entry).ok_or_else(|| {
                        StudioError::Render("completed H3 job exposed no saved video".into())
                    })?;
                    let source = PathBuf::from(&project.settings.comfy_root)
                        .join("output")
                        .join(&media.1)
                        .join(&media.0);
                    let target = self
                        .project_dir(&project.id)
                        .join("raw")
                        .join(format!("clip-{:03}.mp4", index + 1));
                    tokio::fs::copy(&source, &target).await.map_err(|error| {
                        StudioError::Render(format!(
                            "could not preserve {}: {error}",
                            source.display()
                        ))
                    })?;
                    return Ok(target.to_string_lossy().into_owned());
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    async fn extract_last_frame(
        &self,
        project: &MovieProject,
        index: usize,
    ) -> Result<(), StudioError> {
        let clip = project.clips.get(index).ok_or_else(|| {
            StudioError::Render("completed clip disappeared before continuity extraction".into())
        })?;
        let stills = self.project_dir(&project.id).join("stills");
        fs::create_dir_all(&stills)?;
        let target = stills.join(format!("clip-{:03}-last.png", index + 1));
        let output = tokio::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-sseof",
                "-0.08",
                "-i",
            ])
            .arg(&clip.path)
            .args(["-frames:v", "1"])
            .arg(&target)
            .output()
            .await?;
        if !output.status.success() {
            return Err(StudioError::Render(format!(
                "continuity-frame extraction failed: {}",
                truncate(&String::from_utf8_lossy(&output.stderr), 500)
            )));
        }
        Ok(())
    }

    async fn assemble_default(&self, project: &MovieProject) -> Result<String, StudioError> {
        let folder = self.project_dir(&project.id);
        let concat_path = folder.join("assembly.txt");
        let mut concat = String::new();
        for clip in &project.clips {
            if clip.status == "complete" {
                concat.push_str(&format!(
                    "file '{}'\n",
                    clip.path.replace('\\', "/").replace('\'', "'\\''")
                ));
            }
        }
        if concat.is_empty() {
            return Err(StudioError::Render(
                "there are no completed clips to assemble".into(),
            ));
        }
        fs::write(&concat_path, concat)?;
        let target = folder.join("exports").join("first-cut.mp4");
        let output = tokio::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "concat",
                "-safe",
                "0",
                "-i",
            ])
            .arg(&concat_path)
            .args(["-c", "copy", "-movflags", "+faststart"])
            .arg(&target)
            .output()
            .await?;
        if !output.status.success() {
            return Err(StudioError::Render(format!(
                "FFmpeg assembly failed: {}",
                truncate(&String::from_utf8_lossy(&output.stderr), 1_000)
            )));
        }
        Ok(target.to_string_lossy().into_owned())
    }

    pub async fn render_edit(&self, id: &str) -> Result<MovieProject, StudioError> {
        let mut project = self.get(id)?;
        let mut edits = project
            .edit
            .clips
            .iter()
            .filter(|edit| edit.enabled)
            .cloned()
            .collect::<Vec<_>>();
        edits.sort_by_key(|edit| edit.order);
        if edits.is_empty() {
            return Err(StudioError::Invalid(
                "enable at least one clip before exporting".into(),
            ));
        }
        let mut command = tokio::process::Command::new("ffmpeg");
        command.args(["-y", "-hide_banner", "-loglevel", "error"]);
        let mut filters = Vec::new();
        for (index, edit) in edits.iter().enumerate() {
            let clip = project
                .clips
                .iter()
                .find(|clip| clip.id == edit.clip_id)
                .ok_or_else(|| StudioError::Invalid("edit references a missing clip".into()))?;
            command.arg("-i").arg(&clip.path);
            let end = (clip.duration_seconds - edit.trim_end).max(edit.trim_start + 0.1);
            filters.push(format!("[{index}:v]trim=start={}:end={},setpts=PTS-STARTPTS[v{index}];[{index}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS,volume={}[a{index}]", edit.trim_start, end, edit.trim_start, end, edit.audio_gain));
        }
        let streams = (0..edits.len())
            .map(|index| format!("[v{index}][a{index}]"))
            .collect::<String>();
        filters.push(format!("{streams}concat=n={}:v=1:a=1[v][a]", edits.len()));
        let target = self
            .project_dir(id)
            .join("exports")
            .join(format!("edit-{}.mp4", Utc::now().format("%Y%m%d-%H%M%S")));
        command
            .arg("-filter_complex")
            .arg(filters.join(";"))
            .args([
                "-map",
                "[v]",
                "-map",
                "[a]",
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-crf",
                "18",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-movflags",
                "+faststart",
            ])
            .arg(&target);
        let output = command.output().await?;
        if !output.status.success() {
            return Err(StudioError::Render(format!(
                "edit export failed: {}",
                truncate(&String::from_utf8_lossy(&output.stderr), 1_000)
            )));
        }
        project.final_path = target.to_string_lossy().into_owned();
        project.updated_at = Utc::now().to_rfc3339();
        project.detail = "A new non-destructive edit is ready.".into();
        self.save(&project)?;
        Ok(project)
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
            let _ = app.emit("movie-project", project.clone());
        }
        Ok(())
    }

    fn load_path(&self, path: &Path) -> Result<MovieProject, StudioError> {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
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
                project.detail = "Kestrel closed during production. Completed masters are safe; resume when ready.".into();
                project.updated_at = Utc::now().to_rfc3339();
                write_json_atomic(&path, &project)?;
            }
        }
        Ok(())
    }
}

fn check_cancel(cancel: &CancellationToken) -> Result<(), StudioError> {
    if cancel.is_cancelled() {
        Err(StudioError::Cancelled)
    } else {
        Ok(())
    }
}

fn response_message(response: &Value) -> Result<Value, StudioError> {
    response
        .pointer("/choices/0/message")
        .cloned()
        .ok_or_else(|| {
            StudioError::Planning(format!(
                "missing model message: {}",
                truncate(&response.to_string(), 500)
            ))
        })
}

fn movie_tools() -> Value {
    json!([
        {"type":"function","function":{"name":"search_archive","description":"Search the offline January 2024 English Wikipedia archive when factual grounding would improve the movie.","parameters":{"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}}},
        {"type":"function","function":{"name":"read_source","description":"Open one exact sourceRef returned by search_archive. Only opened sources are evidence.","parameters":{"type":"object","properties":{"source_ref":{"type":"string"},"section":{"type":"string"}},"required":["source_ref"]}}}
    ])
}

fn director_prompt(settings: &MovieSettings) -> String {
    format!(
        "You are the writer-director-editor inside Kestrel Movie Studio. Turn the user's request into a complete original MiniMax H3 plan. Infer missing decisions. Keep identity, world, visual language, causality, and audio coherent. Each 5-15 second prompt must stand alone and describe timed action, camera, dialogue, sound effects, and music. Select producer assets by exact ID in referenceIds; never invent IDs or write H3 reference tags because the runtime binds them. Chain a prior frame only without native references. Use archive tools only when facts improve the film. Return the required plan, not commentary. Renderer: 24fps, {}x{}, at most {} clips.",
        settings.width, settings.height, settings.max_clips
    )
}

fn reference_manifest(references: &[MovieReference]) -> String {
    let mut manifest = String::from(
        "Producer references are immutable native H3 inputs. Use each where its stated job applies. Put exact asset IDs in each clip's referenceIds; Kestrel injects correctly renumbered <Picture n>, <Video n>, and <Audio n> assignments at render time. Do not claim to have visually or audibly inspected media beyond the producer's description.\n",
    );
    for reference in references {
        manifest.push_str(&format!(
            "\nAsset ID: {}\nType: {}\nProducer description: {}\n",
            reference.asset_id, reference.kind, reference.description
        ));
        if reference.use_embedded_audio {
            manifest.push_str(&format!(
                "Embedded video audio job: {}\n",
                reference.embedded_audio_description
            ));
        }
    }
    manifest
}

fn movie_schema(max_clips: u32) -> Value {
    json!({"type":"json_schema","json_schema":{"name":"kestrel_movie_plan","strict":true,"schema":{
        "type":"object","additionalProperties":false,
        "properties":{
            "title":{"type":"string"},"logline":{"type":"string"},"audience":{"type":"string"},"creativeDirection":{"type":"string"},
            "continuityBible":{"type":"array","items":{"type":"string"}},"sourceCredits":{"type":"array","items":{"type":"string"}},
            "clips":{"type":"array","minItems":1,"maxItems":max_clips,"items":{"type":"object","additionalProperties":false,"properties":{
                "title":{"type":"string"},"purpose":{"type":"string"},"durationSeconds":{"type":"number","minimum":5,"maximum":15},"prompt":{"type":"string"},
                "continuityIn":{"type":"string"},"continuityOut":{"type":"string"},"transition":{"type":"string"},"usePreviousFrame":{"type":"boolean"},"sourceRefs":{"type":"array","items":{"type":"string"}},"referenceIds":{"type":"array","items":{"type":"string"}}
            },"required":["title","purpose","durationSeconds","prompt","continuityIn","continuityOut","transition","usePreviousFrame","sourceRefs","referenceIds"]}}
        },"required":["title","logline","audience","creativeDirection","continuityBible","sourceCredits","clips"]
    }}})
}

struct CompletionOptions<'a> {
    tools: Option<&'a Value>,
    response_format: Option<Value>,
    max_tokens: u32,
    settings: &'a MovieSettings,
    research: &'a ResearchSettings,
}

struct H3GraphRequest<'a> {
    prompt: &'a str,
    width: u32,
    height: u32,
    seconds: f32,
    steps: u32,
    seed: u64,
    prefix: &'a str,
    first_frame: Option<&'a str>,
    references: &'a [H3ReferenceInput<'a>],
    ref_image_size: &'a str,
}

struct H3ReferenceInput<'a> {
    kind: &'a str,
    file: String,
    description: &'a str,
    use_embedded_audio: bool,
    embedded_audio_description: &'a str,
}

fn bound_reference_prompt(references: &[H3ReferenceInput<'_>]) -> String {
    let mut prompt = String::from("Native producer-reference assignments for this shot:");
    let mut picture = 0usize;
    for reference in references
        .iter()
        .filter(|reference| reference.kind == "image")
    {
        picture += 1;
        prompt.push_str(&format!("\n<Picture {picture}>: {}", reference.description));
    }
    let mut video = 0usize;
    let mut audio = 0usize;
    for reference in references
        .iter()
        .filter(|reference| reference.kind == "video")
    {
        if reference.use_embedded_audio {
            audio += 1;
            prompt.push_str(&format!(
                "\n<Audio {audio}> from the same source as <Video {}>: {}",
                video + 1,
                reference.embedded_audio_description
            ));
        }
        video += 1;
        prompt.push_str(&format!("\n<Video {video}>: {}", reference.description));
    }
    for reference in references
        .iter()
        .filter(|reference| reference.kind == "audio")
    {
        audio += 1;
        prompt.push_str(&format!("\n<Audio {audio}>: {}", reference.description));
    }
    prompt.push_str("\nHonor these assignments explicitly and use no unbound reference labels.");
    prompt
}

fn h3_graph(request: H3GraphRequest<'_>) -> Value {
    let H3GraphRequest {
        prompt,
        width,
        height,
        seconds,
        steps,
        seed,
        prefix,
        first_frame,
        references,
        ref_image_size,
    } = request;
    let raw_frames = (seconds * 24.0).round().max(5.0) as u32;
    let length = raw_frames + (5 + 17 - raw_frames % 17) % 17;
    let using_references = !references.is_empty();
    let unet = if using_references {
        "minimax_h3_ref2va_pruned_int8_convrot.safetensors"
    } else {
        "minimax_h3_fl2va_pruned_int8_convrot.safetensors"
    };
    let conditioning = if using_references {
        json!({"class_type":"MiniMaxH3ReferenceToVideo","inputs":{"clip":["2",0],"vae":["3",0],"audio_vae":["4",0],"prompt":prompt,"width":width,"height":height,"length":length,"ref_image_size":ref_image_size}})
    } else {
        json!({"class_type":"MiniMaxH3ImageToVideo","inputs":{"clip":["2",0],"vae":["3",0],"prompt":prompt,"width":width,"height":height,"length":length}})
    };
    let mut graph = json!({
        "1":{"class_type":"UNETLoader","inputs":{"unet_name":unet,"weight_dtype":"default"}},
        "2":{"class_type":"CLIPLoader","inputs":{"clip_name":"qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors","type":"minimax","device":"default"}},
        "3":{"class_type":"VAELoader","inputs":{"vae_name":"minimax_h3_video_vae_fp16.safetensors"}},
        "4":{"class_type":"VAELoader","inputs":{"vae_name":"minimax_h3_audio_vae_fp32.safetensors"}},
        "5":conditioning,
        "6":{"class_type":"RandomNoise","inputs":{"noise_seed":seed}},
        "7":{"class_type":"BasicScheduler","inputs":{"model":["1",0],"scheduler":"simple","steps":steps,"denoise":1.0}},
        "8":{"class_type":"KSamplerSelect","inputs":{"sampler_name":"res_multistep"}},
        "9":{"class_type":"BasicGuider","inputs":{"model":["1",0],"conditioning":["5",0]}},
        "10":{"class_type":"SamplerCustomAdvanced","inputs":{"noise":["6",0],"guider":["9",0],"sampler":["8",0],"sigmas":["7",0],"latent_image":["5",1]}},
        "11":{"class_type":"VAEDecode","inputs":{"samples":["10",0],"vae":["3",0]}},
        "12":{"class_type":"VAEDecodeAudio","inputs":{"samples":["10",0],"vae":["4",0]}},
        "13":{"class_type":"CreateVideo","inputs":{"images":["11",0],"audio":["12",0],"fps":24.0,"bit_depth":8}},
        "14":{"class_type":"SaveVideo","inputs":{"video":["13",0],"filename_prefix":prefix,"format":"auto","codec":"auto"}}
    });
    if !using_references {
        if let Some(image) = first_frame {
            graph["15"] = json!({"class_type":"LoadImage","inputs":{"image":image}});
            graph["5"]["inputs"]["first_frame"] = json!(["15", 0]);
        }
        return graph;
    }
    let mut node = 15usize;
    // Comfy's V3 autogrow API accepts dotted, zero-based dynamic paths. The H3 prompt labels
    // those same values with one-based ordinals, which bound_reference_prompt owns separately.
    let mut picture = 0usize;
    for reference in references
        .iter()
        .filter(|reference| reference.kind == "image")
    {
        picture += 1;
        let node_id = node.to_string();
        graph[&node_id] = json!({"class_type":"LoadImage","inputs":{"image":reference.file}});
        graph["5"]["inputs"][format!("ref_images.ref_image_{}", picture - 1)] = json!([node_id, 0]);
        node += 1;
    }
    let mut video = 0usize;
    for reference in references
        .iter()
        .filter(|reference| reference.kind == "video")
    {
        video += 1;
        let load_id = node.to_string();
        let components_id = (node + 1).to_string();
        graph[&load_id] = json!({"class_type":"LoadVideo","inputs":{"file":reference.file}});
        graph[&components_id] =
            json!({"class_type":"GetVideoComponents","inputs":{"video":[load_id,0]}});
        graph["5"]["inputs"][format!("ref_videos.ref_video_{}", video - 1)] =
            json!([components_id, 0]);
        if reference.use_embedded_audio {
            graph["5"]["inputs"][format!("ref_video_audios.ref_video_audio_{}", video - 1)] =
                json!([components_id, 1]);
        }
        node += 2;
    }
    let mut audio = 0usize;
    for reference in references
        .iter()
        .filter(|reference| reference.kind == "audio")
    {
        audio += 1;
        let node_id = node.to_string();
        graph[&node_id] = json!({"class_type":"LoadAudio","inputs":{"audio":reference.file}});
        graph["5"]["inputs"][format!("ref_audios.ref_audio_{}", audio - 1)] = json!([node_id, 0]);
        node += 1;
    }
    graph
}

fn find_output_media(entry: &Value) -> Option<(String, String)> {
    let outputs = entry.get("outputs")?.as_object()?;
    for output in outputs.values() {
        for key in ["images", "videos"] {
            for media in output
                .get(key)
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
        "ogg" | "oga" => ("audio", "audio/ogg", MAX_AUDIO_BYTES),
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
    let output = std::process::Command::new("ffprobe")
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
    if kind == "video" && !(2.0..=MAX_REFERENCE_SECONDS).contains(&duration) {
        return Err(StudioError::Invalid(format!(
            "{} is {:.2}s; H3 reference videos must be trimmed to 2-15 seconds",
            reference_name(path),
            duration
        )));
    }
    if kind == "audio" && !(0.2..=MAX_REFERENCE_SECONDS).contains(&duration) {
        return Err(StudioError::Invalid(format!(
            "{} is {:.2}s; trim H3 audio references to 0.2-15 seconds for predictable conditioning",
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

fn hash_reference(path: &Path) -> Result<String, StudioError> {
    let mut input = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
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

fn contains_reference_tag(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    ["<picture", "<video", "<audio", "<subject"]
        .iter()
        .any(|tag| value.contains(tag))
}

fn derive_seed(base: u64, index: u64) -> u64 {
    let base = if base == 0 {
        Utc::now().timestamp_nanos_opt().unwrap_or_default() as u64
    } else {
        base
    };
    base.wrapping_add(index.wrapping_mul(0x9E37_79B9_7F4A_7C15)) & i64::MAX as u64
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
    fs::write(&temporary, bytes)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::RuntimeManager;
    use std::sync::Arc;

    #[test]
    fn h3_frame_count_uses_native_grid() {
        let graph = h3_graph(H3GraphRequest {
            prompt: "test",
            width: 864,
            height: 480,
            seconds: 5.0,
            steps: 20,
            seed: 7,
            prefix: "test",
            first_frame: None,
            references: &[],
            ref_image_size: "match",
        });
        assert_eq!(graph["5"]["inputs"]["length"], 124);
    }

    #[test]
    fn h3_graph_can_chain_a_preserved_continuity_frame() {
        let graph = h3_graph(H3GraphRequest {
            prompt: "test",
            width: 864,
            height: 480,
            seconds: 5.0,
            steps: 20,
            seed: 7,
            prefix: "test",
            first_frame: Some("kestrel/frame.png"),
            references: &[],
            ref_image_size: "match",
        });
        assert_eq!(graph["15"]["class_type"], "LoadImage");
        assert_eq!(graph["5"]["inputs"]["first_frame"], json!(["15", 0]));
    }

    #[test]
    fn h3_reference_graph_uses_native_ref2va_inputs_and_local_tag_order() {
        let references = [
            H3ReferenceInput {
                kind: "audio",
                file: "kestrel/audio.wav".into(),
                description: "Use this quiet pulse as the score's rhythm.",
                use_embedded_audio: false,
                embedded_audio_description: "",
            },
            H3ReferenceInput {
                kind: "image",
                file: "kestrel/hero.png".into(),
                description: "Keep this character's face and red coat.",
                use_embedded_audio: false,
                embedded_audio_description: "",
            },
            H3ReferenceInput {
                kind: "video",
                file: "kestrel/move.mp4".into(),
                description: "Reuse this circular dolly move, not its subject.",
                use_embedded_audio: true,
                embedded_audio_description: "Keep the speaker's calm voice timbre.",
            },
        ];
        let prompt = bound_reference_prompt(&references);
        assert!(prompt.contains("<Picture 1>: Keep this character's face and red coat."));
        assert!(prompt.contains("<Audio 1> from the same source as <Video 1>"));
        assert!(prompt.contains("<Video 1>: Reuse this circular dolly move"));
        assert!(prompt.contains("<Audio 2>: Use this quiet pulse"));

        let graph = h3_graph(H3GraphRequest {
            prompt: &prompt,
            width: 864,
            height: 480,
            seconds: 5.0,
            steps: 20,
            seed: 7,
            prefix: "test",
            first_frame: Some("must-not-be-used.png"),
            references: &references,
            ref_image_size: "max",
        });
        assert_eq!(
            graph["1"]["inputs"]["unet_name"],
            "minimax_h3_ref2va_pruned_int8_convrot.safetensors"
        );
        assert_eq!(graph["5"]["class_type"], "MiniMaxH3ReferenceToVideo");
        assert_eq!(graph["5"]["inputs"]["audio_vae"], json!(["4", 0]));
        assert_eq!(graph["5"]["inputs"]["ref_image_size"], "max");
        assert_eq!(graph["15"]["class_type"], "LoadImage");
        assert_eq!(graph["16"]["class_type"], "LoadVideo");
        assert_eq!(graph["17"]["class_type"], "GetVideoComponents");
        assert_eq!(graph["18"]["class_type"], "LoadAudio");
        assert_eq!(
            graph["5"]["inputs"]["ref_images.ref_image_0"],
            json!(["15", 0])
        );
        assert_eq!(
            graph["5"]["inputs"]["ref_videos.ref_video_0"],
            json!(["17", 0])
        );
        assert_eq!(
            graph["5"]["inputs"]["ref_video_audios.ref_video_audio_0"],
            json!(["17", 1])
        );
        assert_eq!(
            graph["5"]["inputs"]["ref_audios.ref_audio_0"],
            json!(["18", 0])
        );
        assert!(graph["5"]["inputs"].get("first_frame").is_none());
    }

    #[test]
    fn settings_require_h3_grid() {
        let settings = MovieSettings {
            width: 865,
            ..MovieSettings::default()
        };
        assert!(settings.validate(false).is_err());
    }

    #[test]
    fn planner_contract_stays_small_and_tool_set_stays_bounded() {
        assert_eq!(movie_tools().as_array().unwrap().len(), 2);
        assert!(
            director_prompt(&MovieSettings::default())
                .split_whitespace()
                .count()
                < 120
        );
        let schema = movie_schema(12);
        assert_eq!(
            schema.pointer("/json_schema/schema/properties/clips/maxItems"),
            Some(&json!(12))
        );
    }

    #[test]
    fn project_preserves_the_users_prompt_verbatim() {
        let root = tempfile::tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let prompt = "  A home movie about Miso the cat — keep my punctuation!  ";
        let project = studio
            .create(
                StartMovieRequest {
                    prompt: prompt.into(),
                    settings: MovieSettings::default(),
                    references: vec![],
                },
                false,
            )
            .unwrap();
        assert_eq!(project.prompt, prompt);
        let request: Value = serde_json::from_slice(
            &fs::read(studio.project_dir(&project.id).join("request.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(request["prompt"], prompt);
    }

    #[test]
    fn reference_descriptions_reject_runtime_owned_h3_tags() {
        assert!(contains_reference_tag(
            "Make the costume match <Picture 1>."
        ));
        assert!(!contains_reference_tag(
            "Keep the reference costume and facial identity."
        ));
    }

    #[test]
    fn comfy_execution_errors_surface_the_failing_node_instead_of_the_whole_graph() {
        let entry = json!({"status":{"messages":[["execution_error",{
            "node_id":"5","node_type":"MiniMaxH3ReferenceToVideo",
            "exception_message":"reference audio could not be decoded\n"
        }]]}});
        assert_eq!(
            comfy_execution_error(&entry).as_deref(),
            Some("MiniMaxH3ReferenceToVideo node 5 failed: reference audio could not be decoded")
        );
    }

    #[tokio::test]
    #[ignore = "requires the installed Bonsai, ComfyUI MiniMax H3 stack, FFmpeg, and several minutes"]
    async fn live_one_prompt_movie_produces_a_native_audio_first_cut() {
        let root = tempfile::tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let research = ResearchSettings::default();
        let runtime = Arc::new(RuntimeManager::new());
        let cancel = CancellationToken::new();
        let result: Result<MovieProject, String> = async {
            eprintln!("live movie: releasing Comfy memory");
            studio.release_comfy_memory().await;
            eprintln!("live movie: creating durable project");
            let project = studio.create(StartMovieRequest {
                prompt: "Create one five-second cinematic nature shot of a dew-covered fern unfolding at sunrise, with coherent birdsong and a gentle breeze. No research is needed.".into(),
                settings: MovieSettings { research_mode: "never".into(), width: 864, height: 480, max_clips: 1, ..MovieSettings::default() },
                references: vec![],
            }, false).map_err(|error| error.to_string())?;
            let lease = runtime.lease_research(&research).await.map_err(|error| error.to_string())?;
            eprintln!("live movie: directing with Bonsai");
            studio.plan(&project.id, &lease.connection, &research, &cancel, None).await.map_err(|error| error.to_string())?;
            eprintln!("live movie: returning the Bonsai lease and GPU");
            drop(lease);
            runtime.stop_managed().await.map_err(|error| error.to_string())?;
            crate::services::stop_bonsai(&research.bonsai_root).await.map_err(|error| error.to_string())?;
            eprintln!("live movie: rendering and assembling with MiniMax H3");
            studio.render(&project.id, &cancel, None).await.map_err(|error| error.to_string())
        }.await;
        let _ = runtime.stop_managed().await;
        let _ = crate::services::stop_bonsai(&research.bonsai_root).await;
        let project = result.unwrap();
        assert_eq!(project.status, "complete");
        assert!(Path::new(&project.final_path).is_file());
        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "csv=p=0",
            ])
            .arg(&project.final_path)
            .output()
            .unwrap();
        let streams = String::from_utf8_lossy(&probe.stdout);
        assert!(streams.contains("video"));
        assert!(streams.contains("audio"));
    }

    #[tokio::test]
    #[ignore = "requires installed Bonsai, MiniMax H3 ref2va, ComfyUI, FFmpeg, and several minutes"]
    async fn live_one_prompt_movie_uses_native_picture_and_audio_references() {
        let root = tempfile::tempdir().unwrap();
        let picture = root.path().join("producer-sunrise.png");
        let audio = root.path().join("producer-tone.wav");
        let picture_output = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=0xd56b31:s=864x480",
                "-frames:v",
                "1",
            ])
            .arg(&picture)
            .output()
            .unwrap();
        assert!(
            picture_output.status.success(),
            "{}",
            String::from_utf8_lossy(&picture_output.stderr)
        );
        let audio_output = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=220:duration=5",
                "-c:a",
                "pcm_s16le",
            ])
            .arg(&audio)
            .output()
            .unwrap();
        assert!(
            audio_output.status.success(),
            "{}",
            String::from_utf8_lossy(&audio_output.stderr)
        );

        let studio = MovieStudio::new(root.path()).unwrap();
        let picture_asset = studio.import_reference_path(&picture).unwrap();
        let audio_asset = studio.import_reference_path(&audio).unwrap();
        let research = ResearchSettings::default();
        let runtime = Arc::new(RuntimeManager::new());
        let cancel = CancellationToken::new();
        let result: Result<MovieProject, String> = async {
            eprintln!("live reference movie: releasing Comfy memory");
            studio.release_comfy_memory().await;
            let project = studio
                .create(
                    StartMovieRequest {
                        prompt: "Make one five-second cinematic abstract sunrise shot. Use the attached picture for the exact burnt-orange color field and composition. Use the attached audio only as a subtle low synthesizer reference. No research is needed.".into(),
                        settings: MovieSettings {
                            research_mode: "never".into(),
                            width: 864,
                            height: 480,
                            max_clips: 1,
                            ..MovieSettings::default()
                        },
                        references: vec![
                            ProducerReferenceRequest {
                                asset_id: picture_asset.id.clone(),
                                description: "Preserve the exact burnt-orange color field and centered composition; do not copy any unrelated visual detail.".into(),
                                use_embedded_audio: false,
                                embedded_audio_description: String::new(),
                            },
                            ProducerReferenceRequest {
                                asset_id: audio_asset.id.clone(),
                                description: "Use the 220 Hz tone only as the timbral basis for a very quiet synthesizer bed under native ambience.".into(),
                                use_embedded_audio: false,
                                embedded_audio_description: String::new(),
                            },
                        ],
                    },
                    false,
                )
                .map_err(|error| error.to_string())?;
            let lease = runtime
                .lease_research(&research)
                .await
                .map_err(|error| error.to_string())?;
            eprintln!("live reference movie: directing with Bonsai and producer manifest");
            let planned = studio
                .plan(&project.id, &lease.connection, &research, &cancel, None)
                .await
                .map_err(|error| error.to_string())?;
            let plan = planned.plan.as_ref().ok_or("Bonsai committed no plan")?;
            assert_eq!(plan.clips.len(), 1);
            assert_eq!(plan.clips[0].reference_ids.len(), 2);
            drop(lease);
            runtime
                .stop_managed()
                .await
                .map_err(|error| error.to_string())?;
            crate::services::stop_bonsai(&research.bonsai_root)
                .await
                .map_err(|error| error.to_string())?;
            eprintln!("live reference movie: rendering with installed H3 ref2va");
            studio
                .render(&project.id, &cancel, None)
                .await
                .map_err(|error| error.to_string())
        }
        .await;
        let _ = runtime.stop_managed().await;
        let _ = crate::services::stop_bonsai(&research.bonsai_root).await;
        let project = result.unwrap();
        assert_eq!(project.status, "complete");
        assert_eq!(project.references.len(), 2);
        assert!(Path::new(&project.references[0].path).is_file());
        assert!(Path::new(&project.references[1].path).is_file());
        assert!(Path::new(&project.final_path).is_file());
        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "csv=p=0",
            ])
            .arg(&project.final_path)
            .output()
            .unwrap();
        let streams = String::from_utf8_lossy(&probe.stdout);
        assert!(streams.contains("video"));
        assert!(streams.contains("audio"));
    }
}
