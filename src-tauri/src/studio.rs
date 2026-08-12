use crate::{
    models::ResearchSettings,
    runtime::{authorized, ModelConnection},
};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use thiserror::Error;
use tokio::{process::Child, sync::Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;

mod copilot;
mod image_assets;
mod live_preview;
mod movie_agent;
mod planning;
mod prompt_collaboration;
mod prompts;

pub use copilot::{
    emit_error as emit_copilot_error, emit_settled as emit_copilot_settled,
    validate_request as validate_copilot_request, MovieCopilotJob, MovieCopilotReceipt,
    MovieCopilotRequest,
};
pub use image_assets::{
    emit_image_asset_error, GeneratedImageProvenance, MovieImageAssetGeneration,
    MovieImageAssetRequest,
};
use live_preview::{
    emit_preview_unavailable, preview_node, LivePreviewSession, PreviewTarget, PREVIEW_NODE_ID,
};
use movie_agent::{MovieAgentWorkspace, WorkspaceToolRequest, WorkspaceToolResult};
pub use planning::{MoviePlanningEvent, MoviePlanningSnapshot};
pub use prompt_collaboration::{
    emit_error as emit_prompt_draft_error, emit_settled as emit_prompt_draft_settled,
    validate_request as validate_prompt_draft_request, PromptDraftJob, PromptDraftRequest,
};

const SCHEMA_VERSION: u32 = 6;
const COMFY_BASE: &str = "http://127.0.0.1:8188";
const MAX_REFERENCE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_AUDIO_BYTES: u64 = 256 * 1024 * 1024;
pub(super) const MAX_MOVIE_PROMPT_BYTES: usize = 64 * 1024;
const MAX_PLAN_EXCHANGE_BYTES: usize = 2 * 1024 * 1024;
const PLAN_EXCHANGE_FORMAT: &str = "kestrel.movie-plan";
const PLAN_EXCHANGE_VERSION: u32 = 1;
const MAX_REFERENCE_SECONDS: f64 = 15.1;
const MIN_H3_PROMPT_WORDS: usize = 120;
const MAX_H3_PROMPT_WORDS: usize = 450;
const MOVIE_AGENT_SESSION_STEPS: u32 = 96;
const MAX_MOVIE_AGENT_SESSIONS: u32 = 8;
const MOVIE_THINKING_BUDGET: u32 = 32_768;
const COMFY_RENDER_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

enum MovieAgentOutcome {
    Submitted(MoviePlan),
    Checkpointed,
}

fn media_program(name: &str) -> PathBuf {
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
    #[error("studio operation was stopped")]
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
    #[serde(default)]
    pub pause_after_plan: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoviePlanFeedbackRequest {
    pub id: String,
    pub feedback: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieClipAssistRequest {
    pub id: String,
    pub clip_id: String,
    pub feedback: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieClipSuggestion {
    pub clip_id: String,
    pub summary: String,
    pub checklist: Vec<String>,
    pub clip: PlannedClip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieClipRenderRequest {
    pub id: String,
    pub suggestion: MovieClipSuggestion,
    pub seed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerFeedbackRecord {
    pub created_at: String,
    pub scope: String,
    #[serde(default)]
    pub clip_id: String,
    pub feedback: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GeneratedImageProvenance>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<GeneratedImageProvenance>,
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
    MOVIE_THINKING_BUDGET
}
fn default_output() -> u32 {
    32_768
}
fn default_comfy_root() -> String {
    ResearchSettings::default().comfy_root
}
fn default_ref_image_size() -> String {
    "match".into()
}

impl MovieSettings {
    pub fn validate(mut self, advanced: bool) -> Result<Self, StudioError> {
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
        // Bonsai's movie-agent checkpoint is trained for its full reasoning mode.
        // Keep the serialized field for project/profile compatibility, but never
        // permit a movie request (including an old saved one) to disable it.
        self.thinking_budget = MOVIE_THINKING_BUDGET;
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
    #[serde(default)]
    pub quality_review: MovieQualityReview,
    pub clips: Vec<PlannedClip>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieQualityReview {
    pub attempts: u32,
    pub score: u32,
    pub verdict: String,
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
    #[serde(default)]
    pub versions: Vec<ClipVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipVersion {
    pub id: String,
    pub created_at: String,
    pub title: String,
    pub prompt: String,
    pub duration_seconds: f32,
    pub seed: u64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClipEdit {
    #[serde(default)]
    pub id: String,
    pub clip_id: String,
    pub enabled: bool,
    pub order: u32,
    pub trim_start: f32,
    pub trim_end: f32,
    #[serde(default = "default_gain")]
    pub audio_gain: f32,
    #[serde(default)]
    pub source_version_id: String,
    #[serde(default = "default_speed")]
    pub speed: f32,
    #[serde(default)]
    pub fade_in: f32,
    #[serde(default)]
    pub fade_out: f32,
    #[serde(default)]
    pub audio_fade_in: f32,
    #[serde(default)]
    pub audio_fade_out: f32,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub notes: String,
}

fn default_gain() -> f32 {
    1.0
}

fn default_speed() -> f32 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MovieEdit {
    #[serde(default)]
    pub clips: Vec<ClipEdit>,
    #[serde(default = "default_export_title")]
    pub export_title: String,
    #[serde(default = "default_export_preset")]
    pub export_preset: String,
    #[serde(default)]
    pub normalize_audio: bool,
    #[serde(default = "default_target_lufs")]
    pub target_lufs: f32,
    #[serde(default)]
    pub markers: Vec<TimelineMarker>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TimelineMarker {
    pub id: String,
    pub time_seconds: f32,
    pub label: String,
    #[serde(default = "default_marker_kind")]
    pub kind: String,
    #[serde(default)]
    pub completed: bool,
}

fn default_marker_kind() -> String {
    "marker".into()
}

fn default_export_title() -> String {
    "Kestrel Movie".into()
}

fn default_export_preset() -> String {
    "publish".into()
}

fn default_target_lufs() -> f32 {
    -14.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieExport {
    pub id: String,
    pub created_at: String,
    pub title: String,
    pub preset: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub duration_seconds: f32,
    pub clip_count: usize,
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
    pub exports: Vec<MovieExport>,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub producer_review_required: bool,
    #[serde(default)]
    pub producer_approved_at: String,
    #[serde(default)]
    pub producer_feedback: Vec<ProducerFeedbackRecord>,
    #[serde(default)]
    pub copilot_history: Vec<MovieCopilotTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieCopilotTurn {
    pub id: String,
    pub created_at: String,
    pub workspace: String,
    pub producer_request: String,
    pub model_id: String,
    pub response: String,
    pub status: String,
    #[serde(default)]
    pub proposal_summary: String,
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
    comfy_child: Arc<AsyncMutex<Option<Child>>>,
    comfy_preview_available: Arc<AsyncMutex<Option<bool>>>,
    project_locks: Arc<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
    planning_control: Arc<StdMutex<()>>,
    planning_sequence: Arc<AtomicU64>,
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
            comfy_preview_available: Arc::new(AsyncMutex::new(None)),
            project_locks: Arc::new(StdMutex::new(HashMap::new())),
            planning_control: Arc::new(StdMutex::new(())),
            planning_sequence: Arc::new(AtomicU64::new(1)),
        };
        studio.recover_interrupted()?;
        studio.recover_image_asset_generations()?;
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
            generation: None,
        };
        write_json_atomic(&meta_path, &asset)?;
        Ok(asset)
    }

    pub fn create(
        &self,
        request: StartMovieRequest,
        advanced: bool,
    ) -> Result<MovieProject, StudioError> {
        self.create_project(request, advanced, false)
    }

    pub fn create_manual(
        &self,
        request: StartMovieRequest,
        advanced: bool,
    ) -> Result<MovieProject, StudioError> {
        self.create_project(request, advanced, true)
    }

    fn create_project(
        &self,
        request: StartMovieRequest,
        advanced: bool,
        producer_authored: bool,
    ) -> Result<MovieProject, StudioError> {
        let StartMovieRequest {
            prompt,
            settings,
            references,
            pause_after_plan,
        } = request;
        let meaningful_prompt = prompt.trim();
        if prompt.len() > MAX_MOVIE_PROMPT_BYTES
            || (!producer_authored && meaningful_prompt.chars().count() < 3)
        {
            return Err(StudioError::Invalid(
                if producer_authored {
                    "optional movie notes must not exceed 64 KiB"
                } else {
                    "movie prompt must be between 3 characters and 64 KiB"
                }
                .into(),
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
        let plan = producer_authored.then(|| MoviePlan {
            title: "Untitled movie".into(),
            logline: String::new(),
            audience: String::new(),
            creative_direction: String::new(),
            continuity_bible: Vec::new(),
            source_credits: Vec::new(),
            quality_review: MovieQualityReview {
                attempts: 0,
                score: 0,
                verdict: "Producer-owned blank plan. Bonsai has not been used.".into(),
            },
            clips: Vec::new(),
        });
        let project = MovieProject {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            prompt,
            title: "Untitled movie".into(),
            status: if producer_authored {
                "awaiting-review"
            } else {
                "running"
            }
            .into(),
            phase: if producer_authored {
                "awaiting-producer"
            } else {
                "planning"
            }
            .into(),
            detail: if producer_authored {
                "A blank producer-owned plan is ready. Add scenes and approve when native checks pass; Bonsai has not been started."
            } else {
                "Bonsai is shaping the story, continuity, and production plan."
            }
            .into(),
            created_at: now.clone(),
            updated_at: now,
            model: if producer_authored {
                "Producer-authored; local model help is optional"
            } else {
                "Ternary Bonsai 27B Q2_0"
            }
            .into(),
            renderer: "MiniMax H3 / ComfyUI native".into(),
            settings,
            references,
            plan,
            sources: Vec::new(),
            clips: Vec::new(),
            edit: MovieEdit {
                clips: Vec::new(),
                export_title: "Kestrel Movie".into(),
                export_preset: default_export_preset(),
                normalize_audio: false,
                target_lufs: default_target_lufs(),
                markers: Vec::new(),
            },
            final_path: String::new(),
            exports: Vec::new(),
            error: String::new(),
            producer_review_required: producer_authored || pause_after_plan,
            producer_approved_at: String::new(),
            producer_feedback: Vec::new(),
            copilot_history: Vec::new(),
        };
        write_json_atomic(
            &folder.join("request.json"),
            &json!({"prompt":project.prompt,"settings":project.settings,"references":project.references,"createdAt":project.created_at}),
        )?;
        write_json_atomic(&folder.join("references.json"), &project.references)?;
        if let Some(plan) = project.plan.as_ref() {
            write_json_atomic(&folder.join("plan.json"), plan)?;
        }
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
            if request.embedded_audio_description.len() > 4_000 {
                return Err(StudioError::Invalid(
                    "embedded audio descriptions cannot exceed 4,000 bytes".into(),
                ));
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
                generation: asset.generation,
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

    fn planning_control_path(&self, id: &str) -> PathBuf {
        self.project_dir(id).join("planning-control.json")
    }

    fn emit_planning(
        &self,
        id: &str,
        kind: &str,
        stage: &str,
        text: impl Into<String>,
        position: (u32, u32),
        app: Option<&AppHandle>,
    ) {
        let Some(app) = app else { return };
        let event = MoviePlanningEvent {
            project_id: id.into(),
            sequence: self.planning_sequence.fetch_add(1, Ordering::Relaxed),
            kind: kind.into(),
            stage: stage.into(),
            text: text.into(),
            session: position.0,
            step: position.1,
            created_at: Utc::now().to_rfc3339(),
        };
        let _ = app.emit("movie-planning", event);
    }

    pub fn planning_snapshot(&self, id: &str) -> Result<MoviePlanningSnapshot, StudioError> {
        validate_id(id)?;
        let project = self.get(id)?;
        let folder = self.project_dir(id).join("agent-workspace");
        let transcript = planning::read_advanced_json(&folder.join("agent-transcript.json"))?;
        let last_request = planning::read_advanced_json(&folder.join("agent-last-request.json"))?;
        let control = {
            let _guard = self.planning_control.lock().map_err(|_| {
                StudioError::Invalid("movie planning controls are unavailable".into())
            })?;
            planning::load_control(&self.planning_control_path(id))?
        };
        let mut documents = prompts::prompt_catalog(&project.settings);
        for (name, id, title, category) in [
            (
                "README.md",
                "workspace-contract",
                "Durable workspace contract",
                "system",
            ),
            (
                "BRIEF.md",
                "producer-brief",
                "Producer brief sent to Bonsai",
                "input",
            ),
            (
                "REFERENCES.md",
                "producer-references",
                "Producer reference manifest",
                "input",
            ),
            (
                "PRODUCER-NOTES.md",
                "producer-notes",
                "Live producer directions",
                "input",
            ),
        ] {
            if let Some(document) =
                planning::read_prompt_document(&folder.join(name), id, title, category)?
            {
                documents.push(document);
            }
        }
        Ok(MoviePlanningSnapshot {
            project_id: id.into(),
            checkpoint_requested: control.checkpoint_requested,
            pending_directions: control.pending_directions,
            prompt_documents: documents,
            tool_schema: MovieAgentWorkspace::tools(),
            last_request,
            current_text: planning::latest_assistant_text(&transcript),
            transcript,
        })
    }

    pub fn queue_planning_direction(
        &self,
        id: &str,
        text: &str,
        app: Option<&AppHandle>,
    ) -> Result<MoviePlanningSnapshot, StudioError> {
        validate_id(id)?;
        let project = self.get(id)?;
        if project.status != "running"
            || !matches!(
                project.phase.as_str(),
                "writing" | "agent-workspace" | "resuming" | "producer-revision"
            )
        {
            return Err(StudioError::Invalid(
                "live direction is available while Bonsai is planning; resume a planning checkpoint first"
                    .into(),
            ));
        }
        let direction = {
            let _guard = self.planning_control.lock().map_err(|_| {
                StudioError::Invalid("movie planning controls are unavailable".into())
            })?;
            let (_, direction) = planning::add_direction(&self.planning_control_path(id), text)?;
            direction
        };
        self.emit_planning(
            id,
            "direction-queued",
            "producer",
            direction.text,
            (0, 0),
            app,
        );
        self.planning_snapshot(id)
    }

    pub fn request_planning_checkpoint(
        &self,
        id: &str,
        app: Option<&AppHandle>,
    ) -> Result<MoviePlanningSnapshot, StudioError> {
        validate_id(id)?;
        let project = self.get(id)?;
        if project.status != "running"
            || !matches!(
                project.phase.as_str(),
                "writing" | "agent-workspace" | "resuming" | "producer-revision"
            )
        {
            return Err(StudioError::Invalid(
                "a planning checkpoint can only be requested while Bonsai is planning".into(),
            ));
        }
        {
            let _guard = self.planning_control.lock().map_err(|_| {
                StudioError::Invalid("movie planning controls are unavailable".into())
            })?;
            planning::request_checkpoint(&self.planning_control_path(id))?;
        }
        self.emit_planning(
            id,
            "checkpoint-requested",
            "producer",
            "Checkpoint requested. Bonsai will stop after the current safe model turn.",
            (0, 0),
            app,
        );
        self.planning_snapshot(id)
    }

    fn consume_planning_control<F>(
        &self,
        id: &str,
        consume: F,
    ) -> Result<planning::PlanningControl, StudioError>
    where
        F: FnOnce(&planning::PlanningControl) -> Result<(), StudioError>,
    {
        let _guard = self
            .planning_control
            .lock()
            .map_err(|_| StudioError::Invalid("movie planning controls are unavailable".into()))?;
        let path = self.planning_control_path(id);
        let control = planning::load_control(&path)?;
        consume(&control)?;
        planning::acknowledge_pending(&path, &control)?;
        Ok(control)
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

    pub fn movie_plan_exchange_prompt(&self, id: &str) -> Result<String, StudioError> {
        let project = self.get(id)?;
        ensure_plan_is_unrendered(&project)?;
        let references = project
            .references
            .iter()
            .enumerate()
            .map(|(index, reference)| {
                json!({
                    "handle": exchange_reference_handle(index),
                    "name": reference.name,
                    "kind": reference.kind,
                    "description": reference.description,
                    "hasEmbeddedAudio": reference.use_embedded_audio,
                    "embeddedAudioDescription": reference.embedded_audio_description,
                })
            })
            .collect::<Vec<_>>();
        let current_plan = project
            .plan
            .as_ref()
            .map(|plan| plan_for_exchange(plan, &project.references))
            .transpose()?;
        let context = json!({
            "storyPrompt": project.prompt,
            "renderWidth": project.settings.width,
            "renderHeight": project.settings.height,
            "maximumScenes": project.settings.max_clips,
            "references": references,
            "currentPlan": current_plan,
        });
        let output_template = json!({
            "format": PLAN_EXCHANGE_FORMAT,
            "version": PLAN_EXCHANGE_VERSION,
            "plan": {
                "title": "Movie title",
                "logline": "One concise sentence",
                "audience": "Intended audience",
                "creativeDirection": "Overall visual, performance, editorial, and sound direction",
                "continuityBible": ["Concrete identity, wardrobe, world, geography, or time rule"],
                "sourceCredits": [],
                "clips": [{
                    "id": "scene-001",
                    "title": "Scene title",
                    "purpose": "The story job of this scene",
                    "durationSeconds": 5,
                    "prompt": "A complete 120-450 word MiniMax H3 direction with timed beats covering the exact duration, camera, lighting or visual texture, action, and sound.",
                    "continuityIn": "Truthful visible state at the first frame",
                    "continuityOut": "Truthful visible state at the final frame",
                    "transition": "hard cut",
                    "usePreviousFrame": false,
                    "sourceRefs": [],
                    "referenceIds": ["reference-1"]
                }]
            }
        });
        let prompt = format!(
            "You are preparing a production plan for Kestrel's offline MiniMax H3 video studio. You are a planning collaborator, not an agent: do not call tools, claim to render media, or invent file paths. Return exactly one JSON object and no Markdown commentary.\n\nYour output must use this versioned envelope and field spelling:\n{}\n\nRules:\n- Preserve the producer's story intent. If currentPlan contains useful work, revise or complete it rather than discarding it without reason.\n- Use 5-15 seconds per scene and no more than maximumScenes. Put scenes in final editorial order.\n- Every clip prompt must be 120-450 words and include explicit timed beats covering the exact clip endpoint, camera or lens/framing, lighting or visual texture, visible action, and sound. Each clip has its own local timeline beginning at 0 seconds: label beats with parseable local ranges such as [0s-2s], [2s-4s], and [4s-5s]. End the final range at that clip's durationSeconds; never continue film-global timestamps across clips. Include exact quoted words for any speech; otherwise explicitly direct no dialogue or narration.\n- continuityIn and continuityOut must describe concrete visible handoff states. usePreviousFrame may only be true after scene 1 and then referenceIds must be empty.\n- referenceIds may contain only the safe handles listed in references, such as reference-1. Never write a reference handle, native tag, or hidden asset ID inside renderer prose.\n- Use a reference on every independently generated appearance where its described identity, wardrobe, product, motion, audio, or style must be preserved. Do not attach character references to subject-free scenes.\n- sourceCredits must remain empty unless the producer context provides real sources.\n- JSON strings must be validly escaped. Do not add fields containing analysis or reasoning.\n\nComplete project context:\n{}",
            serde_json::to_string_pretty(&output_template)?,
            serde_json::to_string_pretty(&context)?,
        );
        if prompt.len() > MAX_PLAN_EXCHANGE_BYTES {
            return Err(StudioError::Invalid(
                "the current plan is too large for a single external-model exchange; shorten oversized renderer directions or exchange smaller revisions".into(),
            ));
        }
        Ok(prompt)
    }

    pub fn parse_movie_plan_exchange(
        &self,
        id: &str,
        text: &str,
    ) -> Result<MoviePlan, StudioError> {
        let project = self.get(id)?;
        ensure_plan_is_unrendered(&project)?;
        let root = parse_plan_exchange_json(text)?;
        if let Some(format) = root.get("format") {
            if format.as_str() != Some(PLAN_EXCHANGE_FORMAT) {
                return Err(StudioError::Invalid(format!(
                    "unsupported plan exchange format {}; expected {PLAN_EXCHANGE_FORMAT}",
                    format
                )));
            }
        }
        if let Some(version) = root.get("version") {
            if version.as_u64() != Some(u64::from(PLAN_EXCHANGE_VERSION)) {
                return Err(StudioError::Invalid(format!(
                    "unsupported plan exchange version {}; expected {PLAN_EXCHANGE_VERSION}",
                    version
                )));
            }
        }
        let plan_value = root.get("plan").cloned().unwrap_or(root);
        let mut plan: MoviePlan = serde_json::from_value(plan_value).map_err(|error| {
            StudioError::Invalid(format!(
                "the pasted JSON does not match the Kestrel plan schema: {error}"
            ))
        })?;
        let reference_handles = project
            .references
            .iter()
            .enumerate()
            .map(|(index, reference)| {
                (exchange_reference_handle(index), reference.asset_id.clone())
            })
            .collect::<HashMap<_, _>>();
        let known_asset_ids = project
            .references
            .iter()
            .map(|reference| reference.asset_id.as_str())
            .collect::<HashSet<_>>();
        for (index, clip) in plan.clips.iter_mut().enumerate() {
            for reference_id in &mut clip.reference_ids {
                if let Some(asset_id) = reference_handles.get(reference_id) {
                    reference_id.clone_from(asset_id);
                } else if !known_asset_ids.contains(reference_id.as_str()) {
                    return Err(StudioError::Invalid(format!(
                        "scene {} uses unknown reference handle '{}'; copy a fresh external-model brief so the model receives the current reference list",
                        index + 1,
                        reference_id
                    )));
                }
            }
        }
        prepare_producer_draft(&project, &mut plan)?;
        Ok(plan)
    }

    pub async fn save_producer_plan(
        &self,
        id: &str,
        mut plan: MoviePlan,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let lock = self.project_lock(id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(id)?;
        ensure_plan_is_unrendered(&project)?;
        prepare_producer_draft(&project, &mut plan)?;
        plan.quality_review = MovieQualityReview {
            attempts: 0,
            score: 0,
            verdict:
                "Producer-edited draft. Run Bonsai revision or approve explicitly before rendering."
                    .into(),
        };
        project.producer_feedback.push(ProducerFeedbackRecord {
            created_at: Utc::now().to_rfc3339(),
            scope: "manual-plan".into(),
            clip_id: String::new(),
            feedback: "Producer saved structured screenplay and scene changes.".into(),
        });
        self.replace_unrendered_plan(
            &mut project,
            plan,
            "Structured producer changes are saved. No H3 clip has started.",
            app,
        )?;
        Ok(project)
    }

    pub async fn approve_producer_plan(
        &self,
        id: &str,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let lock = self.project_lock(id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(id)?;
        ensure_plan_is_unrendered(&project)?;
        let mut plan = project
            .plan
            .as_ref()
            .ok_or_else(|| StudioError::Invalid("project has no saved movie plan".into()))?
            .clone();
        prepare_producer_plan(&project, &mut plan)?;
        let issues = prompt_quality_issues(&plan, &project.references);
        if !issues.is_empty() {
            return Err(StudioError::Invalid(format!(
                "producer plan is not render-ready: {}",
                issues.join(" ")
            )));
        }
        if plan.quality_review.attempts == 0 {
            plan.quality_review = MovieQualityReview {
                attempts: 0,
                score: 100,
                verdict: "Producer-authored plan passed Kestrel's native release checks without an agent review.".into(),
            };
        }
        write_json_atomic(&self.project_dir(id).join("plan.json"), &plan)?;
        project.plan = Some(plan);
        project.status = "running".into();
        project.phase = "producer-approved".into();
        project.detail = "Producer approved the structured plan. H3 rendering may begin.".into();
        project.producer_approved_at = Utc::now().to_rfc3339();
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    fn replace_unrendered_plan(
        &self,
        project: &mut MovieProject,
        plan: MoviePlan,
        detail: &str,
        app: Option<&AppHandle>,
    ) -> Result<(), StudioError> {
        let library_title = if plan.title.trim().is_empty() {
            "Untitled movie"
        } else {
            plan.title.as_str()
        };
        project.title = library_title.into();
        project.edit.export_title = library_title.into();
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
                versions: Vec::new(),
            })
            .collect();
        project.edit.clips = project
            .clips
            .iter()
            .map(|clip| ClipEdit {
                id: format!("edit-{}", clip.id),
                clip_id: clip.id.clone(),
                enabled: true,
                order: clip.index,
                trim_start: 0.0,
                trim_end: 0.0,
                audio_gain: 1.0,
                source_version_id: String::new(),
                speed: default_speed(),
                fade_in: 0.0,
                fade_out: 0.0,
                audio_fade_in: 0.0,
                audio_fade_out: 0.0,
                label: String::new(),
                notes: String::new(),
            })
            .collect();
        project.plan = Some(plan.clone());
        project.status = "awaiting-review".into();
        project.phase = "awaiting-producer".into();
        project.detail = detail.into();
        project.producer_approved_at.clear();
        self.persist_emit(project, app)?;
        write_json_atomic(&self.project_dir(&project.id).join("plan.json"), &plan)?;
        Ok(())
    }

    pub async fn save_edits(
        &self,
        id: &str,
        mut edit: MovieEdit,
    ) -> Result<MovieProject, StudioError> {
        let lock = self.project_lock(id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(id)?;
        validate_movie_edit(&project, &mut edit)?;
        project.edit = edit;
        project.schema_version = SCHEMA_VERSION;
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
        let resuming_planning = project.status == "planning-checkpoint" || project.plan.is_none();
        project.status = "running".into();
        project.phase = "resuming".into();
        project.error.clear();
        project.detail = if resuming_planning {
            "Resuming Bonsai from the durable movie workspace.".into()
        } else {
            "Resuming from the last preserved H3 master.".into()
        };
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
        let lock = self.project_lock(id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(id)?;
        let user_prompt = project.prompt.clone();
        let movie_settings = project.settings.clone();
        let (outcome, sources) = self
            .direct(
                &user_prompt,
                &movie_settings,
                connection,
                research,
                cancel,
                (&mut project, app),
            )
            .await?;
        let MovieAgentOutcome::Submitted(mut plan) = outcome else {
            project.status = "planning-checkpoint".into();
            project.phase = "planning-checkpoint".into();
            project.detail = "Bonsai stopped at a safe producer checkpoint. The workspace, exact model transcript, and unfinished scene files are preserved; resume whenever you are ready.".into();
            self.persist_emit(&mut project, app)?;
            self.emit_planning(
                id,
                "checkpoint-saved",
                "checkpoint",
                "Safe planning checkpoint saved.",
                (0, 0),
                app,
            );
            return Ok(project);
        };
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
            if clip.use_previous_frame && !clip.reference_ids.is_empty() {
                return Err(StudioError::Planning(format!(
                    "Bonsai review accepted clip {} with incompatible H3 continuation and native references; rendering was stopped before any media changed",
                    index + 1
                )));
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
                versions: Vec::new(),
            })
            .collect();
        project.edit.clips = project
            .clips
            .iter()
            .map(|clip| ClipEdit {
                id: format!("edit-{}", clip.id),
                clip_id: clip.id.clone(),
                enabled: true,
                order: clip.index,
                trim_start: 0.0,
                trim_end: 0.0,
                audio_gain: 1.0,
                source_version_id: String::new(),
                speed: default_speed(),
                fade_in: 0.0,
                fade_out: 0.0,
                audio_fade_in: 0.0,
                audio_fade_out: 0.0,
                label: String::new(),
                notes: String::new(),
            })
            .collect();
        project.plan = Some(plan.clone());
        if project.producer_review_required {
            project.status = "awaiting-review".into();
            project.phase = "awaiting-producer".into();
            project.detail = format!(
                "Bonsai's {}-scene plan is paused for structured producer review. No H3 clip has started.",
                project.clips.len()
            );
        } else {
            project.phase = "plan-ready".into();
            project.detail = format!(
                "The production plan is ready with {} H3 clips.",
                project.clips.len()
            );
        }
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
    ) -> Result<(MovieAgentOutcome, Vec<MovieSource>), StudioError> {
        let (project, app) = progress;
        check_cancel(cancel)?;
        project.phase = "writing".into();
        project.detail =
            "Bonsai is working in its durable movie codebase and running native H3 checks.".into();
        self.persist_emit(project, app)?;
        let manifest = reference_manifest(&project.references);
        let plan = self
            .run_movie_agent(
                prompt, &manifest, None, None, settings, connection, research, cancel, project, app,
            )
            .await?;
        Ok((plan, Vec::new()))
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_movie_agent(
        &self,
        prompt: &str,
        manifest: &str,
        seed: Option<&MoviePlan>,
        producer_feedback: Option<&str>,
        settings: &MovieSettings,
        connection: &ModelConnection,
        research: &ResearchSettings,
        cancel: &CancellationToken,
        project: &mut MovieProject,
        app: Option<&AppHandle>,
    ) -> Result<MovieAgentOutcome, StudioError> {
        let workspace_root = self.project_dir(&project.id).join("agent-workspace");
        let resuming_existing_workspace = workspace_root.join("movie.json").is_file();
        let transcript_path = workspace_root.join("agent-transcript.json");
        let mut workspace = MovieAgentWorkspace::open(
            workspace_root,
            prompt,
            manifest,
            settings,
            &project.references,
            seed,
            producer_feedback,
        )?;
        let tools = MovieAgentWorkspace::tools();
        let mut session = 1_u32;
        let mut absolute_step = 0_u32;
        let mut independent_review_round = 0_u32;
        'agent_sessions: loop {
            check_cancel(cancel)?;
            if session > MAX_MOVIE_AGENT_SESSIONS {
                return Err(StudioError::Planning(format!(
                    "Bonsai did not submit a valid movie after {MAX_MOVIE_AGENT_SESSIONS} context sessions; the durable workspace is intact for a later retry"
                )));
            }
            if session > 1 && transcript_path.is_file() {
                fs::copy(
                    &transcript_path,
                    workspace
                        .root()
                        .join(format!("agent-transcript-session-{:03}.json", session - 1)),
                )?;
            }
            let instruction = if session == 1 && !resuming_existing_workspace {
                prompts::INITIAL_INSTRUCTION
            } else {
                prompts::RESUME_INSTRUCTION
            };
            let mut messages = vec![
                json!({"role":"system","content":movie_agent_prompt()}),
                json!({"role":"user","content":instruction}),
            ];
            persist_movie_agent_transcript(&transcript_path, absolute_step, &messages)?;
            let mut no_tool_streak = 0_u32;

            for _ in 0..MOVIE_AGENT_SESSION_STEPS {
                check_cancel(cancel)?;
                let control = self.consume_planning_control(&project.id, |control| {
                    if control.pending_directions.is_empty() && !control.checkpoint_requested {
                        return Ok(());
                    }
                    if !control.pending_directions.is_empty() {
                        workspace.record_producer_directions(&control.pending_directions)?;
                        for direction in &control.pending_directions {
                            messages.push(json!({
                                "role":"user",
                                "content":prompts::producer_direction(&direction.text),
                            }));
                        }
                    }
                    persist_movie_agent_transcript(&transcript_path, absolute_step, &messages)?;
                    Ok(())
                })?;
                if !control.pending_directions.is_empty() {
                    for direction in control.pending_directions {
                        project.producer_feedback.push(ProducerFeedbackRecord {
                            created_at: direction.created_at,
                            scope: "live-planning".into(),
                            clip_id: String::new(),
                            feedback: direction.text,
                        });
                    }
                    project.detail = "Bonsai received the producer's latest direction and is revising the durable plan.".into();
                    self.persist_emit(project, app)?;
                }
                if control.checkpoint_requested {
                    return Ok(MovieAgentOutcome::Checkpointed);
                }
                absolute_step = absolute_step.saturating_add(1);
                project.phase = "agent-workspace".into();
                project.detail = format!(
                    "Bonsai is editing and checking the durable movie codebase (agent step {absolute_step}, context session {session})."
                );
                self.persist_emit(project, app)?;
                self.emit_planning(
                    &project.id,
                    "turn-start",
                    "planning",
                    format!("Bonsai is planning turn {absolute_step}."),
                    (session, absolute_step),
                    app,
                );
                let mut request_messages = messages.clone();
                request_messages.push(json!({
                    "role":"user",
                    "content":workspace.authoritative_story_memory()?,
                }));
                let request = self.complete_agent_stream(
                    connection,
                    &request_messages,
                    &tools,
                    settings,
                    research,
                    cancel,
                    &project.id,
                    session,
                    absolute_step,
                    app,
                );
                let response_result = tokio::select! {
                    result = request => result,
                    _ = cancel.cancelled() => return Err(StudioError::Cancelled),
                };
                let response = match response_result {
                    Ok(response) => response,
                    Err(error) => {
                        messages.push(json!({
                            "role":"user",
                            "content":prompts::response_checkpoint(&error.to_string())
                        }));
                        persist_movie_agent_transcript(&transcript_path, absolute_step, &messages)?;
                        project.detail = format!(
                            "Bonsai response failed safely at agent step {absolute_step}; Kestrel is checkpointing and resuming in a fresh context."
                        );
                        self.persist_emit(project, app)?;
                        session = session.saturating_add(1);
                        cancellable_agent_restart_delay(cancel).await?;
                        continue 'agent_sessions;
                    }
                };
                let message = response_message(&response)?;
                let tool_calls = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut history_message = message;
                if let Some(object) = history_message.as_object_mut() {
                    object.remove("reasoning");
                    object.remove("reasoning_content");
                }
                messages.push(history_message);
                persist_movie_agent_transcript(&transcript_path, absolute_step, &messages)?;
                if tool_calls.is_empty() {
                    no_tool_streak = no_tool_streak.saturating_add(1);
                    messages.push(json!({"role":"user","content":prompts::CONTINUE_WITH_TOOLS}));
                    persist_movie_agent_transcript(&transcript_path, absolute_step, &messages)?;
                    if no_tool_streak >= 3 {
                        project.detail = format!(
                            "Bonsai stopped using its workspace for three turns at agent step {absolute_step}; Kestrel is checkpointing and resuming in a fresh context."
                        );
                        self.persist_emit(project, app)?;
                        session = session.saturating_add(1);
                        cancellable_agent_restart_delay(cancel).await?;
                        continue 'agent_sessions;
                    }
                    continue;
                }
                no_tool_streak = 0;
                for call in tool_calls {
                    check_cancel(cancel)?;
                    let call_id = call
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("movie-tool");
                    let name = call
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let result = if name != "movie_workspace" {
                        WorkspaceToolResult {
                            message: format!("ERROR: unknown tool {name}"),
                            submitted: None,
                        }
                    } else {
                        match call.pointer("/function/arguments") {
                            Some(arguments) => {
                                let parsed = if let Some(text) = arguments.as_str() {
                                    serde_json::from_str::<WorkspaceToolRequest>(text)
                                } else {
                                    serde_json::from_value::<WorkspaceToolRequest>(
                                        arguments.clone(),
                                    )
                                };
                                match parsed {
                                    Ok(request) => {
                                        self.emit_planning(
                                            &project.id,
                                            "activity",
                                            &request.action,
                                            producer_activity(&request),
                                            (session, absolute_step),
                                            app,
                                        );
                                        let result = workspace.execute(request);
                                        self.emit_planning(
                                            &project.id,
                                            "tool-result",
                                            "native-check",
                                            producer_tool_result(&result.message),
                                            (session, absolute_step),
                                            app,
                                        );
                                        result
                                    }
                                    Err(error) => WorkspaceToolResult {
                                        message: format!(
                                            "ERROR: invalid movie_workspace arguments: {error}"
                                        ),
                                        submitted: None,
                                    },
                                }
                            }
                            None => WorkspaceToolResult {
                                message:
                                    "ERROR: invalid movie_workspace arguments: missing arguments"
                                        .into(),
                                submitted: None,
                            },
                        }
                    };
                    let submitted = result.submitted;
                    messages.push(json!({
                        "role":"tool",
                        "tool_call_id":call_id,
                        "content":result.message,
                    }));
                    persist_movie_agent_transcript(&transcript_path, absolute_step, &messages)?;
                    if let Some(mut plan) = submitted {
                        project.phase = "agent-submitted".into();
                        project.detail = format!(
                            "Bonsai submitted a checked {}-scene plan. A fresh-context reviewer is comparing every scene with the producer brief.",
                            plan.clips.len()
                        );
                        self.persist_emit(project, app)?;
                        let review = tokio::select! {
                            result = self.independently_review_movie_plan(
                                &project.id,
                                prompt,
                                &project.references,
                                &plan,
                                connection,
                                settings,
                                research,
                            ) => result?,
                            _ = cancel.cancelled() => return Err(StudioError::Cancelled),
                        };
                        let blocking = review
                            .issues
                            .into_iter()
                            .filter(|issue| {
                                issue.clip_number as usize <= plan.clips.len()
                                    && has_meaningful_prose(&issue.finding, 3)
                                    && has_meaningful_prose(&issue.required_fix, 3)
                            })
                            .collect::<Vec<_>>();
                        if blocking.is_empty() {
                            plan.quality_review.verdict = "Bonsai completed the durable workspace build, two clean native checks, a whole-codebase self-review, and a separate fresh-context fidelity review against the exact producer brief and references.".into();
                            project.detail = format!(
                                "Bonsai's {}-scene plan passed native lint, self-review, and an independent whole-film review.",
                                plan.clips.len()
                            );
                            self.persist_emit(project, app)?;
                            return Ok(MovieAgentOutcome::Submitted(plan));
                        }
                        independent_review_round = independent_review_round.saturating_add(1);
                        if independent_review_round >= 3 {
                            return Err(StudioError::Planning(format!(
                                "the independent whole-film reviewer still found {} blocking issue(s) after three repair rounds; the durable workspace and review input are preserved",
                                blocking.len()
                            )));
                        }
                        let review_feedback = json!({
                            "summary": review.summary,
                            "blockingIssues": blocking,
                        });
                        messages.push(json!({
                            "role":"user",
                            "content":format!(
                                "Independent whole-film review rejected the submitted plan. Treat these findings as blocking, re-read the fresh authoritative story memory, patch only the affected movie/scene files, then repeat both clean checks and submit again:\n{}",
                                review_feedback
                            ),
                        }));
                        persist_movie_agent_transcript(&transcript_path, absolute_step, &messages)?;
                        project.phase = "agent-workspace".into();
                        project.detail = format!(
                            "The independent reviewer found {} blocking issue(s). Bonsai is repairing the durable plan before H3 can render.",
                            review_feedback["blockingIssues"]
                                .as_array()
                                .map_or(0, Vec::len)
                        );
                        self.persist_emit(project, app)?;
                    }
                }
            }
            session = session.saturating_add(1);
        }
    }

    async fn complete_agent(
        &self,
        connection: &ModelConnection,
        messages: &[Value],
        tools: &Value,
        settings: &MovieSettings,
        research: &ResearchSettings,
    ) -> Result<Value, StudioError> {
        let body = movie_agent_request(
            &connection.model_id,
            messages,
            tools,
            settings,
            research.max_output_tokens,
        );
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
                "movie agent HTTP {status}: {}",
                truncate(&text, 500)
            )));
        }
        serde_json::from_str(&text).map_err(StudioError::from)
    }

    #[allow(clippy::too_many_arguments)]
    async fn complete_agent_stream(
        &self,
        connection: &ModelConnection,
        messages: &[Value],
        tools: &Value,
        settings: &MovieSettings,
        research: &ResearchSettings,
        cancel: &CancellationToken,
        project_id: &str,
        session: u32,
        step: u32,
        app: Option<&AppHandle>,
    ) -> Result<Value, StudioError> {
        let mut body = movie_agent_request(
            &connection.model_id,
            messages,
            tools,
            settings,
            research.max_output_tokens,
        );
        body["stream"] = json!(true);
        body["stream_options"] = json!({"include_usage": true});
        write_json_atomic(
            &self
                .project_dir(project_id)
                .join("agent-workspace")
                .join("agent-last-request.json"),
            &body,
        )?;
        let response = authorized(
            self.http
                .post(format!("{}/chat/completions", connection.endpoint)),
            connection,
        )
        .json(&body)
        .send()
        .await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(StudioError::Planning(format!(
                "movie agent HTTP {status}: {}",
                truncate(&text, 500)
            )));
        }

        let mut stream = response.bytes_stream();
        let mut buffer = Vec::<u8>::new();
        let mut content = String::new();
        let mut tool_calls = Vec::<StreamedMovieToolCall>::new();
        let mut completed = false;
        let mut reasoning_announced = false;
        loop {
            let next = tokio::select! {
                value = stream.next() => value,
                _ = cancel.cancelled() => return Err(StudioError::Cancelled),
            };
            let Some(chunk) = next else { break };
            buffer.extend_from_slice(&chunk?);
            while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
                let line = buffer.drain(..=end).collect::<Vec<_>>();
                let line = String::from_utf8_lossy(&line);
                let Some(data) = line.trim().strip_prefix("data:") else {
                    continue;
                };
                let data = data.trim();
                if data == "[DONE]" {
                    completed = true;
                    continue;
                }
                let Ok(value) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                if let Some(token) = value
                    .pointer("/choices/0/delta/content")
                    .and_then(Value::as_str)
                {
                    content.push_str(token);
                    self.emit_planning(
                        project_id,
                        "token",
                        "model-text",
                        token,
                        (session, step),
                        app,
                    );
                }
                if !reasoning_announced
                    && value
                        .pointer("/choices/0/delta/reasoning_content")
                        .or_else(|| value.pointer("/choices/0/delta/reasoning"))
                        .and_then(Value::as_str)
                        .is_some()
                {
                    reasoning_announced = true;
                    self.emit_planning(
                        project_id,
                        "reasoning",
                        "thinking",
                        "Bonsai is reasoning locally before its next production action.",
                        (session, step),
                        app,
                    );
                }
                if let Some(deltas) = value
                    .pointer("/choices/0/delta/tool_calls")
                    .and_then(Value::as_array)
                {
                    for delta in deltas {
                        let index = delta
                            .get("index")
                            .and_then(Value::as_u64)
                            .unwrap_or_default() as usize;
                        while tool_calls.len() <= index {
                            tool_calls.push(StreamedMovieToolCall::default());
                        }
                        let call = &mut tool_calls[index];
                        if let Some(fragment) = delta.get("id").and_then(Value::as_str) {
                            call.id.push_str(fragment);
                        }
                        if let Some(fragment) =
                            delta.pointer("/function/name").and_then(Value::as_str)
                        {
                            call.name.push_str(fragment);
                        }
                        if let Some(fragment) =
                            delta.pointer("/function/arguments").and_then(Value::as_str)
                        {
                            if !call.activity_announced {
                                call.activity_announced = true;
                                self.emit_planning(
                                    project_id,
                                    "activity",
                                    "planning",
                                    "Bonsai is streaming its next structured production action.",
                                    (session, step),
                                    app,
                                );
                            }
                            call.arguments.push_str(fragment);
                            self.emit_planning(
                                project_id,
                                "advanced-token",
                                "tool-arguments",
                                fragment,
                                (session, step),
                                app,
                            );
                        }
                    }
                }
            }
        }
        if !completed {
            return Err(StudioError::Planning(
                "movie agent stream ended before its completion marker; the durable workspace and previous accepted turns are intact"
                    .into(),
            ));
        }
        let tool_calls = tool_calls
            .into_iter()
            .enumerate()
            .filter(|(_, call)| !call.name.is_empty() || !call.arguments.is_empty())
            .map(|(index, call)| {
                json!({
                    "id": if call.id.is_empty() { format!("movie-tool-{step}-{index}") } else { call.id },
                    "type": "function",
                    "function": {"name": call.name, "arguments": call.arguments},
                })
            })
            .collect::<Vec<_>>();
        let assistant_content = if content.is_empty() {
            Value::Null
        } else {
            Value::String(content)
        };
        self.emit_planning(
            project_id,
            "turn-complete",
            "planning",
            "Bonsai completed the model turn and is applying its production action.",
            (session, step),
            app,
        );
        Ok(json!({"choices":[{"message":{
            "role":"assistant",
            "content":assistant_content,
            "tool_calls":tool_calls,
        }}]}))
    }

    pub async fn revise_with_producer_feedback(
        &self,
        id: &str,
        feedback: &str,
        connection: &ModelConnection,
        research: &ResearchSettings,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let feedback = feedback.trim();
        if feedback.chars().count() < 3 || feedback.chars().count() > 16_000 {
            return Err(StudioError::Invalid(
                "producer feedback must contain 3 to 16,000 characters".into(),
            ));
        }
        let lock = self.project_lock(id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(id)?;
        ensure_plan_is_unrendered(&project)?;
        let current = project
            .plan
            .clone()
            .ok_or_else(|| StudioError::Invalid("project has no saved movie plan".into()))?;
        project.status = "running".into();
        project.phase = "producer-revision".into();
        project.detail = "Bonsai is revising the structured plan from producer feedback.".into();
        self.persist_emit(&mut project, app)?;
        let manifest = reference_manifest(&project.references);
        let settings = project.settings.clone();
        let prompt = project.prompt.clone();
        let reviewed = self
            .run_movie_agent(
                &prompt,
                &manifest,
                Some(&current),
                Some(feedback),
                &settings,
                connection,
                research,
                cancel,
                &mut project,
                app,
            )
            .await?;
        let MovieAgentOutcome::Submitted(mut reviewed) = reviewed else {
            project.status = "planning-checkpoint".into();
            project.phase = "planning-checkpoint".into();
            project.detail = "Bonsai saved the producer revision at a safe checkpoint. The previous approved draft and the unfinished workspace are both preserved.".into();
            project.producer_feedback.push(ProducerFeedbackRecord {
                created_at: Utc::now().to_rfc3339(),
                scope: "full-plan-checkpoint".into(),
                clip_id: String::new(),
                feedback: feedback.into(),
            });
            self.persist_emit(&mut project, app)?;
            return Ok(project);
        };
        prepare_producer_plan(&project, &mut reviewed)?;
        project.producer_feedback.push(ProducerFeedbackRecord {
            created_at: Utc::now().to_rfc3339(),
            scope: "full-plan".into(),
            clip_id: String::new(),
            feedback: feedback.into(),
        });
        self.replace_unrendered_plan(
            &mut project,
            reviewed,
            "Bonsai's revised plan passed review and is paused for producer approval.",
            app,
        )?;
        Ok(project)
    }

    pub async fn assist_clip(
        &self,
        id: &str,
        clip_id: &str,
        feedback: &str,
        connection: &ModelConnection,
        research: &ResearchSettings,
    ) -> Result<MovieClipSuggestion, StudioError> {
        let feedback = feedback.trim();
        if feedback.chars().count() < 3 || feedback.chars().count() > 8_000 {
            return Err(StudioError::Invalid(
                "scene feedback must contain 3 to 8,000 characters".into(),
            ));
        }
        let project = self.get(id)?;
        let plan = project
            .plan
            .as_ref()
            .ok_or_else(|| StudioError::Invalid("project has no saved movie plan".into()))?;
        let index = plan
            .clips
            .iter()
            .position(|clip| clip.id == clip_id)
            .ok_or_else(|| StudioError::Invalid("unknown movie scene".into()))?;
        let start = index.saturating_sub(1);
        let end = (index + 2).min(plan.clips.len());
        let payload = json!({
            "producerRequest": project.prompt,
            "producerFeedback": feedback,
            "referenceManifest": reference_manifest(&project.references),
            "creativeDirection": plan.creative_direction,
            "continuityBible": plan.continuity_bible,
            "neighboringScenes": &plan.clips[start..end],
            "sceneToRepair": &plan.clips[index],
        });
        let messages = vec![
            json!({"role":"system","content":clip_assistant_prompt()}),
            json!({"role":"user","content":payload.to_string()}),
        ];
        let mut suggestion: MovieClipSuggestion = self
            .complete_tool_submission(
                connection,
                &messages,
                "submit_scene_suggestion",
                "Submit the organized replacement scene only after checking the producer feedback and neighboring continuity.",
                clip_suggestion_schema(),
                &project.settings,
                research,
                "movie scene suggestion",
                None,
            )
            .await?;
        suggestion.clip_id = clip_id.into();
        suggestion.clip.id = clip_id.into();
        let mut candidate = plan.clone();
        candidate.clips[index] = suggestion.clip.clone();
        prepare_producer_plan(&project, &mut candidate)?;
        let issues = prompt_quality_issues(&candidate, &project.references);
        if !issues.is_empty() {
            return Err(StudioError::Planning(format!(
                "Bonsai's scene suggestion was not render-ready: {}",
                issues.join(" ")
            )));
        }
        Ok(suggestion)
    }

    #[allow(clippy::too_many_arguments)]
    async fn complete_tool_submission<T: for<'de> Deserialize<'de>>(
        &self,
        connection: &ModelConnection,
        initial_messages: &[Value],
        tool_name: &str,
        tool_description: &str,
        response_format: Value,
        settings: &MovieSettings,
        research: &ResearchSettings,
        label: &str,
        audit_path: Option<&Path>,
    ) -> Result<T, StudioError> {
        let schema = response_format
            .pointer("/json_schema/schema")
            .cloned()
            .unwrap_or(response_format);
        let tools = json!([{
            "type":"function",
            "function":{
                "name":tool_name,
                "description":tool_description,
                "parameters":schema,
            }
        }]);
        let mut messages = initial_messages.to_vec();
        let mut last_error = String::new();
        for _ in 0..3 {
            if let Some(path) = audit_path {
                write_json_atomic(
                    path,
                    &movie_agent_request(
                        &connection.model_id,
                        &messages,
                        &tools,
                        settings,
                        research.max_output_tokens,
                    ),
                )?;
            }
            let response = self
                .complete_agent(connection, &messages, &tools, settings, research)
                .await?;
            let message = response_message(&response)?;
            let tool_call = message
                .get("tool_calls")
                .and_then(Value::as_array)
                .and_then(|calls| {
                    calls.iter().find(|call| {
                        call.pointer("/function/name").and_then(Value::as_str) == Some(tool_name)
                    })
                });
            if let Some(call) = tool_call {
                let arguments = call
                    .pointer("/function/arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let parsed = if let Some(text) = arguments.as_str() {
                    serde_json::from_str::<T>(text)
                } else {
                    serde_json::from_value::<T>(arguments)
                };
                match parsed {
                    Ok(value) => return Ok(value),
                    Err(error) => last_error = error.to_string(),
                }
            } else {
                last_error = format!("Bonsai did not call {tool_name}");
            }
            let mut history_message = message;
            if let Some(object) = history_message.as_object_mut() {
                object.remove("reasoning");
                object.remove("reasoning_content");
            }
            messages.push(history_message);
            messages.push(json!({"role":"user","content":format!(
                "The {label} submission failed validation: {last_error}. Correct it and call {tool_name}; do not answer in prose."
            )}));
        }
        Err(StudioError::Planning(format!(
            "{label} remained invalid after three attempts: {last_error}"
        )))
    }

    #[allow(clippy::too_many_arguments)]
    async fn independently_review_movie_plan(
        &self,
        project_id: &str,
        prompt: &str,
        references: &[MovieReference],
        plan: &MoviePlan,
        connection: &ModelConnection,
        settings: &MovieSettings,
        research: &ResearchSettings,
    ) -> Result<MovieCodeReview, StudioError> {
        let payload = json!({
            "exactProducerBrief": prompt,
            "producerReferences": references.iter().map(|reference| json!({
                "assetId": reference.asset_id,
                "kind": reference.kind,
                "description": reference.description,
                "useEmbeddedAudio": reference.use_embedded_audio,
                "embeddedAudioDescription": reference.embedded_audio_description,
            })).collect::<Vec<_>>(),
            "completeSubmittedPlan": plan,
        });
        let messages = vec![
            json!({"role":"system","content":prompts::INDEPENDENT_REVIEWER_SYSTEM}),
            json!({"role":"user","content":payload.to_string()}),
        ];
        write_json_atomic(
            &self
                .project_dir(project_id)
                .join("agent-workspace")
                .join("independent-review-input.json"),
            &json!({
                "messages": sanitize_chat_messages(&messages),
                "toolName": "submit_movie_code_review",
                "schema": code_review_schema(),
            }),
        )?;
        let mut review_settings = settings.clone();
        review_settings.temperature = 0.1;
        review_settings.top_p = 0.9;
        review_settings.top_k = 20;
        review_settings.max_output_tokens = 32_768;
        self.complete_tool_submission(
            connection,
            &messages,
            "submit_movie_code_review",
            "Submit only the independent whole-film review after comparing every scene with the exact producer brief and references.",
            code_review_schema(),
            &review_settings,
            research,
            "independent movie code review",
            Some(
                &self
                    .project_dir(project_id)
                    .join("agent-workspace")
                    .join("agent-last-request.json"),
            ),
        )
        .await
    }

    pub async fn render(
        &self,
        id: &str,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let mut project = self.get(id)?;
        ensure_producer_render_approval(&project)?;
        let comfy_root = project.settings.comfy_root.clone();
        self.ensure_comfy(&comfy_root, &mut project, app).await?;
        let plan = project
            .plan
            .clone()
            .ok_or_else(|| StudioError::Invalid("project has no saved plan".into()))?;
        for (index, planned) in plan.clips.iter().enumerate() {
            check_cancel(cancel)?;
            let rendered_clip = project.clips.get(index).ok_or_else(|| {
                StudioError::Invalid(format!("rendered scene record {} is missing", index + 1))
            })?;
            if rendered_clip.status == "complete" && Path::new(&rendered_clip.path).is_file() {
                continue;
            }
            let seed = rendered_clip.seed;
            project.phase = "rendering".into();
            project.detail = format!(
                "Rendering clip {} of {} — {}",
                index + 1,
                plan.clips.len(),
                planned.title
            );
            project
                .clips
                .get_mut(index)
                .ok_or_else(|| {
                    StudioError::Invalid(format!("rendered scene record {} is missing", index + 1))
                })?
                .status = "rendering".into();
            self.persist_emit(&mut project, app)?;
            match self
                .render_clip(
                    &project,
                    planned,
                    index,
                    seed,
                    cancel,
                    ClipRenderContext { variant: None, app },
                )
                .await
            {
                Ok(path) => {
                    let clip = project.clips.get_mut(index).ok_or_else(|| {
                        StudioError::Invalid(format!(
                            "rendered scene record {} is missing",
                            index + 1
                        ))
                    })?;
                    clip.status = "complete".into();
                    clip.path = path;
                    clip.error.clear();
                    self.extract_last_frame(&project, index).await?;
                    self.persist_emit(&mut project, app)?;
                }
                Err(error) => {
                    let clip = project.clips.get_mut(index).ok_or_else(|| {
                        StudioError::Invalid(format!(
                            "rendered scene record {} is missing",
                            index + 1
                        ))
                    })?;
                    clip.status = "failed".into();
                    clip.error = error.to_string();
                    self.persist_emit(&mut project, app)?;
                    return Err(error);
                }
            }
        }
        project.phase = "assembling".into();
        project.detail =
            "Joining the untouched H3 masters into a review cut without trimming or replacing audio."
                .into();
        self.persist_emit(&mut project, app)?;
        let final_path = self.assemble_default(&project).await?;
        project.final_path = final_path;
        project.status = "complete".into();
        project.phase = "complete".into();
        project.detail = "The untouched H3 review cut is ready. Producer edits are opt-in and every source master remains preserved.".into();
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub async fn render_clip_version(
        &self,
        request: MovieClipRenderRequest,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let MovieClipRenderRequest {
            id,
            suggestion,
            seed,
        } = request;
        let lock = self.project_lock(&id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&id)?;
        ensure_producer_render_approval(&project)?;
        let mut plan = project
            .plan
            .clone()
            .ok_or_else(|| StudioError::Invalid("project has no saved movie plan".into()))?;
        let index = plan
            .clips
            .iter()
            .position(|clip| clip.id == suggestion.clip_id)
            .ok_or_else(|| StudioError::Invalid("unknown movie scene".into()))?;
        if suggestion.clip.id != suggestion.clip_id {
            return Err(StudioError::Invalid(
                "scene suggestion does not match its target scene".into(),
            ));
        }
        let current = project
            .clips
            .get(index)
            .ok_or_else(|| StudioError::Invalid("rendered scene record is missing".into()))?;
        if current.status != "complete" || !Path::new(&current.path).is_file() {
            return Err(StudioError::Invalid(
                "render a scene version only after its original H3 master is complete".into(),
            ));
        }
        plan.clips[index] = suggestion.clip.clone();
        prepare_producer_plan(&project, &mut plan)?;
        let issues = prompt_quality_issues(&plan, &project.references);
        if !issues.is_empty() {
            return Err(StudioError::Invalid(format!(
                "scene version is not render-ready: {}",
                issues.join(" ")
            )));
        }
        let version_id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
        let previous_status = project.status.clone();
        project.status = "running".into();
        project.phase = "rendering-scene-version".into();
        project.detail = format!(
            "Producer requested a new version of scene {}. The current master remains untouched.",
            index + 1
        );
        project.clips[index].status = "rendering-version".into();
        self.persist_emit(&mut project, app)?;
        let comfy_root = project.settings.comfy_root.clone();
        self.ensure_comfy(&comfy_root, &mut project, app).await?;
        let result = self
            .render_clip(
                &project,
                &plan.clips[index],
                index,
                seed,
                cancel,
                ClipRenderContext {
                    variant: Some(&version_id),
                    app,
                },
            )
            .await;
        let path = match result {
            Ok(path) => path,
            Err(error) => {
                project.status = previous_status;
                project.phase = "scene-version-failed".into();
                project.detail =
                    "The new scene version failed; the active master is unchanged.".into();
                project.clips[index].status = "complete".into();
                project.clips[index].error = error.to_string();
                self.persist_emit(&mut project, app)?;
                return Err(error);
            }
        };
        let clip = &mut project.clips[index];
        if clip
            .versions
            .iter()
            .all(|version| version.path != clip.path)
        {
            clip.versions.push(ClipVersion {
                id: "original".into(),
                created_at: project.created_at.clone(),
                title: clip.title.clone(),
                prompt: clip.prompt.clone(),
                duration_seconds: clip.duration_seconds,
                seed: clip.seed,
                path: clip.path.clone(),
            });
        }
        clip.versions.push(ClipVersion {
            id: version_id,
            created_at: Utc::now().to_rfc3339(),
            title: suggestion.clip.title.clone(),
            prompt: suggestion.clip.prompt.clone(),
            duration_seconds: suggestion.clip.duration_seconds,
            seed,
            path: path.clone(),
        });
        clip.title.clone_from(&suggestion.clip.title);
        clip.prompt.clone_from(&suggestion.clip.prompt);
        clip.duration_seconds = suggestion.clip.duration_seconds;
        clip.seed = seed;
        clip.path = path;
        clip.status = "complete".into();
        clip.error.clear();
        project.plan = Some(plan.clone());
        project.producer_feedback.push(ProducerFeedbackRecord {
            created_at: Utc::now().to_rfc3339(),
            scope: "scene-version".into(),
            clip_id: suggestion.clip_id,
            feedback: suggestion.summary,
        });
        self.extract_last_frame(&project, index).await?;
        project.status = previous_status;
        project.phase = "complete".into();
        project.detail = "The producer's new scene version is active. The existing review cut is unchanged until Export new cut is chosen.".into();
        self.persist_emit(&mut project, app)?;
        write_json_atomic(&self.project_dir(&project.id).join("plan.json"), &plan)?;
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
        fs::create_dir_all(logs)?;
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
        *self.comfy_preview_available.lock().await = None;
        *self.comfy_child.lock().await = Some(child);
        for _ in 0..180 {
            if self.comfy_ready().await {
                return Ok(());
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
        self.http
            .get(format!("{COMFY_BASE}/system_stats"))
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

    async fn render_clip(
        &self,
        project: &MovieProject,
        planned: &PlannedClip,
        index: usize,
        seed: u64,
        cancel: &CancellationToken,
        context: ClipRenderContext<'_>,
    ) -> Result<String, StudioError> {
        let ClipRenderContext { variant, app } = context;
        let variant_suffix = variant.map(|value| format!("-{value}")).unwrap_or_default();
        let prefix = format!(
            "kestrel_movies/{}/shot_{:03}{variant_suffix}",
            project.id,
            index + 1
        );
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
                use_embedded_audio: reference.use_embedded_audio,
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
        let preview_available = self.comfy_preview_available().await;
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
            preview_available,
        });
        let client_id = format!("kestrel-preview-{}", uuid::Uuid::new_v4().simple());
        let job_id = variant
            .map(|value| format!("{}-{value}", planned.id))
            .unwrap_or_else(|| planned.id.clone());
        let preview_target = PreviewTarget::movie_clip(job_id, &project.id, &planned.id, index);
        let preview = if preview_available {
            LivePreviewSession::connect(app, &client_id, preview_target).await
        } else {
            emit_preview_unavailable(app, preview_target);
            None
        };
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
        let deadline = tokio::time::Instant::now() + COMFY_RENDER_TIMEOUT;
        loop {
            if cancel.is_cancelled() {
                let _ = self
                    .http
                    .post(format!("{COMFY_BASE}/interrupt"))
                    .send()
                    .await;
                return Err(StudioError::Cancelled);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(StudioError::Render(format!(
                    "ComfyUI did not finish prompt {prompt_id} within 24 hours. The project and completed masters are safe; verify the ComfyUI queue before resuming."
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
                    let detail = comfy_execution_error(entry).unwrap_or_else(|| {
                        format!("execution failed: {}", truncate(&entry.to_string(), 1_000))
                    });
                    return Err(StudioError::Render(format!(
                        "ComfyUI {detail}. The project and completed masters are safe; review the error, then resume."
                    )));
                }
                if entry.pointer("/status/completed").and_then(Value::as_bool) == Some(true) {
                    if let Some(preview) = &preview {
                        preview.finish();
                    }
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
                        .join(format!("clip-{:03}{variant_suffix}.mp4", index + 1));
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
        let output = tokio::process::Command::new(media_program("ffmpeg"))
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
        let output = tokio::process::Command::new(media_program("ffmpeg"))
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
        let lock = self.project_lock(id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(id)?;
        let mut edit = project.edit.clone();
        validate_movie_edit(&project, &mut edit)?;
        project.edit = edit;
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
        let mut command = tokio::process::Command::new(media_program("ffmpeg"));
        command.args(["-y", "-hide_banner", "-loglevel", "error"]);
        let mut filters = Vec::new();
        let mut duration_seconds = 0.0_f32;
        for (index, edit) in edits.iter().enumerate() {
            let source = selected_clip_source(&project, edit)?;
            if !Path::new(source.path).is_file() {
                return Err(StudioError::Invalid(format!(
                    "timeline item {} cannot be exported because its preserved source is missing: {}",
                    edit.id, source.path
                )));
            }
            command.arg("-i").arg(source.path);
            let end = source.duration_seconds - edit.trim_end;
            let output_duration = (end - edit.trim_start) / edit.speed;
            duration_seconds += output_duration;

            let mut video = format!(
                "[{index}:v]trim=start={}:end={},setpts=(PTS-STARTPTS)/{}",
                edit.trim_start, end, edit.speed
            );
            if edit.fade_in > 0.0 {
                video.push_str(&format!(",fade=t=in:st=0:d={}", edit.fade_in));
            }
            if edit.fade_out > 0.0 {
                video.push_str(&format!(
                    ",fade=t=out:st={}:d={}",
                    (output_duration - edit.fade_out).max(0.0),
                    edit.fade_out
                ));
            }
            video.push_str(&format!(
                ",scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p[v{index}]",
                project.settings.width,
                project.settings.height,
                project.settings.width,
                project.settings.height,
            ));

            let mut audio = format!(
                "[{index}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS",
                edit.trim_start, end
            );
            for stage in atempo_filters(edit.speed) {
                audio.push_str(&format!(",atempo={stage}"));
            }
            audio.push_str(&format!(",volume={}", edit.audio_gain));
            if edit.audio_fade_in > 0.0 {
                audio.push_str(&format!(",afade=t=in:st=0:d={}", edit.audio_fade_in));
            }
            if edit.audio_fade_out > 0.0 {
                audio.push_str(&format!(
                    ",afade=t=out:st={}:d={}",
                    (output_duration - edit.audio_fade_out).max(0.0),
                    edit.audio_fade_out
                ));
            }
            audio.push_str(&format!(
                ",aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,aresample=48000:async=1:first_pts=0[a{index}]"
            ));
            filters.push(format!("{video};{audio}"));
        }
        let streams = (0..edits.len())
            .map(|index| format!("[v{index}][a{index}]"))
            .collect::<String>();
        if project.edit.normalize_audio {
            filters.push(format!(
                "{streams}concat=n={}:v=1:a=1[v][mixed];[mixed]loudnorm=I={}:TP=-1.5:LRA=11[a]",
                edits.len(),
                project.edit.target_lufs
            ));
        } else {
            filters.push(format!("{streams}concat=n={}:v=1:a=1[v][a]", edits.len()));
        }
        let export_id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
        let file_stem = format!(
            "{}-{}-{export_id}",
            safe_export_stem(&project.edit.export_title),
            Utc::now().format("%Y%m%d-%H%M%S")
        );
        let exports = self.project_dir(id).join("exports");
        fs::create_dir_all(&exports)?;
        let target = exports.join(format!("{file_stem}.mp4"));
        let temporary = exports.join(format!("{file_stem}.partial.mp4"));
        let (preset, crf, audio_bitrate) = match project.edit.export_preset.as_str() {
            "archive" => ("slow", "14", "320k"),
            "review" => ("veryfast", "24", "128k"),
            _ => ("medium", "18", "192k"),
        };
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
                preset,
                "-crf",
                crf,
                "-c:a",
                "aac",
                "-b:a",
                audio_bitrate,
                "-map_metadata",
                "-1",
                "-movflags",
                "+faststart",
            ])
            .arg("-metadata")
            .arg(format!("title={}", project.edit.export_title))
            .arg(&temporary);
        let output = command.output().await?;
        if !output.status.success() {
            let _ = fs::remove_file(&temporary);
            return Err(StudioError::Render(format!(
                "edit export failed: {}",
                truncate(&String::from_utf8_lossy(&output.stderr), 1_000)
            )));
        }
        let export = MovieExport {
            id: export_id,
            created_at: Utc::now().to_rfc3339(),
            title: project.edit.export_title.clone(),
            preset: project.edit.export_preset.clone(),
            path: target.to_string_lossy().into_owned(),
            bytes: temporary.metadata()?.len(),
            sha256: hash_reference(&temporary)?,
            duration_seconds,
            clip_count: edits.len(),
        };
        let sidecar = exports.join(format!("{file_stem}.json"));
        if let Err(error) = write_json_atomic(
            &sidecar,
            &json!({
                "schemaVersion": SCHEMA_VERSION,
                "projectId": project.id,
                "export": export,
                "edit": project.edit,
            }),
        ) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        if let Err(error) = fs::rename(&temporary, &target) {
            let _ = fs::remove_file(&temporary);
            let _ = fs::remove_file(&sidecar);
            return Err(error.into());
        }
        project.final_path = target.to_string_lossy().into_owned();
        project.exports.push(export);
        project.schema_version = SCHEMA_VERSION;
        project.updated_at = Utc::now().to_rfc3339();
        project.detail = "A new immutable, non-destructive timeline export is ready.".into();
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
        let mut project: MovieProject = serde_json::from_slice(&fs::read(path)?)?;
        normalize_movie_project(&mut project);
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
                project.detail = "Kestrel closed during production. Completed masters are safe; resume when ready.".into();
                project.updated_at = Utc::now().to_rfc3339();
                write_json_atomic(&path, &project)?;
            }
        }
        Ok(())
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

fn validate_movie_edit(project: &MovieProject, edit: &mut MovieEdit) -> Result<(), StudioError> {
    if edit.clips.len() > 512 {
        return Err(StudioError::Invalid(
            "a movie timeline can contain at most 512 items".into(),
        ));
    }
    let title = edit.export_title.trim();
    if title.is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control) {
        return Err(StudioError::Invalid(
            "export title must contain 1 to 120 printable characters".into(),
        ));
    }
    edit.export_title = title.into();
    if !matches!(
        edit.export_preset.as_str(),
        "archive" | "publish" | "review"
    ) {
        return Err(StudioError::Invalid(
            "export preset must be archive, publish, or review".into(),
        ));
    }
    if !edit.target_lufs.is_finite() || !(-24.0..=-9.0).contains(&edit.target_lufs) {
        return Err(StudioError::Invalid(
            "audio normalization target must be between -24 and -9 LUFS".into(),
        ));
    }
    edit.clips.sort_by_key(|item| item.order);
    let mut ids = HashSet::new();
    for (index, item) in edit.clips.iter_mut().enumerate() {
        if item.id.is_empty() {
            item.id = format!("edit-{}-{}", item.clip_id, index + 1);
        }
        if item.id.len() > 128
            || item.id.chars().any(char::is_control)
            || !ids.insert(item.id.clone())
        {
            return Err(StudioError::Invalid(format!(
                "timeline item id must be unique and printable: {}",
                item.id
            )));
        }
        item.order = index as u32;
        let source = selected_clip_source(project, item)?;
        let values = [
            item.trim_start,
            item.trim_end,
            item.audio_gain,
            item.speed,
            item.fade_in,
            item.fade_out,
            item.audio_fade_in,
            item.audio_fade_out,
        ];
        if values.iter().any(|value| !value.is_finite())
            || item.trim_start < 0.0
            || item.trim_end < 0.0
            || item.audio_gain < 0.0
            || item.audio_gain > 4.0
            || !(0.25..=4.0).contains(&item.speed)
            || item.fade_in < 0.0
            || item.fade_out < 0.0
            || item.audio_fade_in < 0.0
            || item.audio_fade_out < 0.0
        {
            return Err(StudioError::Invalid(format!(
                "timeline item {} has trims, speed, fades, or audio gain outside supported bounds",
                item.id
            )));
        }
        if item.label.chars().count() > 120
            || item.label.chars().any(char::is_control)
            || item.notes.chars().count() > 4_000
            || item
                .notes
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        {
            return Err(StudioError::Invalid(format!(
                "timeline item {} has a label or producer note outside supported bounds",
                item.id
            )));
        }
        let source_duration = source.duration_seconds;
        if item.trim_start + item.trim_end > source_duration - 0.1 {
            return Err(StudioError::Invalid(format!(
                "timeline item {} trims away the entire preserved source",
                item.id
            )));
        }
        let output_duration = (source_duration - item.trim_start - item.trim_end) / item.speed;
        if item.fade_in + item.fade_out > output_duration + 0.001
            || item.audio_fade_in + item.audio_fade_out > output_duration + 0.001
        {
            return Err(StudioError::Invalid(format!(
                "timeline item {} fades overlap beyond its {:.2}-second edited duration",
                item.id, output_duration
            )));
        }
    }
    if edit.markers.len() > 256 {
        return Err(StudioError::Invalid(
            "a movie timeline can contain at most 256 markers and to-do items".into(),
        ));
    }
    let mut marker_ids = HashSet::new();
    edit.markers.sort_by(|left, right| {
        left.time_seconds
            .partial_cmp(&right.time_seconds)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for marker in &mut edit.markers {
        marker.label = marker.label.trim().into();
        if marker.id.is_empty()
            || marker.id.len() > 128
            || marker.id.chars().any(char::is_control)
            || !marker_ids.insert(marker.id.clone())
            || !marker.time_seconds.is_finite()
            || !(0.0..=86_400.0).contains(&marker.time_seconds)
            || marker.label.is_empty()
            || marker.label.chars().count() > 120
            || marker.label.chars().any(char::is_control)
            || !matches!(marker.kind.as_str(), "marker" | "todo" | "chapter")
        {
            return Err(StudioError::Invalid(
                "timeline markers require a unique ID, a printable label, a valid time, and marker, todo, or chapter type".into(),
            ));
        }
    }
    Ok(())
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

fn sanitize_chat_messages(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .cloned()
        .map(|mut message| {
            if let Some(content) = message.get_mut("content") {
                if let Some(text) = content.as_str() {
                    *content = Value::String(
                        text.replace("```", "''' ")
                            .replace("<think>", "[reasoning omitted]")
                            .replace("</think>", "[end reasoning]"),
                    );
                }
            }
            message
        })
        .collect()
}

fn persist_movie_agent_transcript(
    path: &Path,
    step: u32,
    messages: &[Value],
) -> Result<(), StudioError> {
    write_json_atomic(
        path,
        &json!({
            "updatedAt": Utc::now().to_rfc3339(),
            "step": step,
            "messages": sanitize_chat_messages(messages),
        }),
    )
}

fn ensure_plan_is_unrendered(project: &MovieProject) -> Result<(), StudioError> {
    if project.clips.iter().any(|clip| {
        clip.status == "rendering" || clip.status == "complete" || !clip.path.is_empty()
    }) {
        return Err(StudioError::Invalid(
            "the screenplay can only be replaced before H3 rendering; use versioned scene repair in the editor for rendered clips"
                .into(),
        ));
    }
    Ok(())
}

fn ensure_producer_render_approval(project: &MovieProject) -> Result<(), StudioError> {
    if project.status == "awaiting-review"
        || (project.producer_review_required && project.producer_approved_at.trim().is_empty())
    {
        return Err(StudioError::Invalid(
            "producer approval is required before starting an H3 render".into(),
        ));
    }
    Ok(())
}

fn exchange_reference_handle(index: usize) -> String {
    format!("reference-{}", index + 1)
}

fn plan_for_exchange(
    plan: &MoviePlan,
    references: &[MovieReference],
) -> Result<Value, StudioError> {
    let handles = references
        .iter()
        .enumerate()
        .map(|(index, reference)| {
            (
                reference.asset_id.as_str(),
                exchange_reference_handle(index),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut plan = plan.clone();
    for (index, clip) in plan.clips.iter_mut().enumerate() {
        for reference_id in &mut clip.reference_ids {
            let handle = handles.get(reference_id.as_str()).ok_or_else(|| {
                StudioError::Invalid(format!(
                    "scene {} contains a reference that is no longer in this project",
                    index + 1
                ))
            })?;
            reference_id.clone_from(handle);
        }
    }
    Ok(serde_json::to_value(plan)?)
}

fn parse_plan_exchange_json(text: &str) -> Result<Value, StudioError> {
    if text.trim().is_empty() || text.len() > MAX_PLAN_EXCHANGE_BYTES {
        return Err(StudioError::Invalid(
            "pasted plan JSON must be between 1 byte and 2 MiB".into(),
        ));
    }
    let trimmed = text.trim();
    if let Ok(value) = serde_json::from_str(trimmed) {
        return Ok(value);
    }
    if let Some(fence_start) = trimmed.find("```") {
        let fenced = &trimmed[fence_start + 3..];
        if let Some(line_end) = fenced.find('\n') {
            let body = &fenced[line_end + 1..];
            if let Some(fence_end) = body.find("```") {
                if let Ok(value) = serde_json::from_str(body[..fence_end].trim()) {
                    return Ok(value);
                }
            }
        }
    }
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            if let Ok(value) = serde_json::from_str(&trimmed[start..=end]) {
                return Ok(value);
            }
        }
    }
    Err(StudioError::Invalid(
        "could not find one valid JSON object in the pasted response; ask the external model to return only the Kestrel JSON envelope".into(),
    ))
}

fn prepare_producer_plan(project: &MovieProject, plan: &mut MoviePlan) -> Result<(), StudioError> {
    prepare_producer_draft(project, plan)?;
    if plan.title.trim().is_empty() || plan.clips.is_empty() {
        return Err(StudioError::Invalid(
            "the structured plan needs a title and at least one scene".into(),
        ));
    }
    for (index, clip) in plan.clips.iter().enumerate() {
        if clip.title.trim().is_empty() || clip.prompt.trim().is_empty() {
            return Err(StudioError::Invalid(format!(
                "scene {} needs a title and renderer direction before approval",
                index + 1
            )));
        }
    }
    Ok(())
}

fn prepare_producer_draft(project: &MovieProject, plan: &mut MoviePlan) -> Result<(), StudioError> {
    if plan.title.len() > 4_000
        || plan.logline.len() > 16_000
        || plan.audience.len() > 4_000
        || plan.creative_direction.len() > 64 * 1024
    {
        return Err(StudioError::Invalid(
            "producer plan fields exceed their durable checkpoint limits".into(),
        ));
    }
    if plan.clips.len() > project.settings.max_clips as usize {
        return Err(StudioError::Invalid(format!(
            "the plan has {} scenes but this production allows at most {}",
            plan.clips.len(),
            project.settings.max_clips
        )));
    }
    let allowed_references = project
        .references
        .iter()
        .map(|reference| reference.asset_id.as_str())
        .collect::<HashSet<_>>();
    for (index, clip) in plan.clips.iter_mut().enumerate() {
        clip.id = format!("clip-{:03}", index + 1);
        if index == 0 && clip.use_previous_frame {
            return Err(StudioError::Invalid(
                "scene 1 cannot continue from a previous frame; turn off first-frame continuation or move it later in the sequence".into(),
            ));
        }
        if !clip.duration_seconds.is_finite() || !(5.0..=15.0).contains(&clip.duration_seconds) {
            return Err(StudioError::Invalid(format!(
                "scene {} duration must be between 5 and 15 seconds",
                index + 1
            )));
        }
        if clip.title.len() > 4_000 || clip.prompt.len() > 64 * 1024 {
            return Err(StudioError::Invalid(format!(
                "scene {} title or renderer direction exceeds its checkpoint limit",
                index + 1
            )));
        }
        let mut seen = HashSet::new();
        for reference_id in &clip.reference_ids {
            if !allowed_references.contains(reference_id.as_str()) {
                return Err(StudioError::Invalid(format!(
                    "scene {} selects an unknown producer reference",
                    index + 1
                )));
            }
            if !seen.insert(reference_id) {
                return Err(StudioError::Invalid(format!(
                    "scene {} selects the same producer reference twice",
                    index + 1
                )));
            }
        }
        if clip.use_previous_frame && !clip.reference_ids.is_empty() {
            return Err(StudioError::Invalid(format!(
                "scene {} cannot combine a previous frame with native H3 references",
                index + 1
            )));
        }
    }
    Ok(())
}

fn check_cancel(cancel: &CancellationToken) -> Result<(), StudioError> {
    if cancel.is_cancelled() {
        Err(StudioError::Cancelled)
    } else {
        Ok(())
    }
}

async fn cancellable_agent_restart_delay(cancel: &CancellationToken) -> Result<(), StudioError> {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(2)) => Ok(()),
        _ = cancel.cancelled() => Err(StudioError::Cancelled),
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

#[derive(Default)]
struct StreamedMovieToolCall {
    id: String,
    name: String,
    arguments: String,
    activity_announced: bool,
}

fn producer_activity(request: &WorkspaceToolRequest) -> String {
    match request.action.as_str() {
        "list" => "Bonsai is reviewing the durable production workspace.".into(),
        "read" | "read_many" => {
            if request.path.is_empty() {
                "Bonsai is reviewing the brief and current scene work.".into()
            } else {
                format!("Bonsai is reviewing {}.", request.path)
            }
        }
        "write" | "write_batch" => "Bonsai is updating the screenplay and scene directions.".into(),
        "delete" => "Bonsai is removing an obsolete draft scene.".into(),
        "check" => "Bonsai is running native H3 lint and the full production review.".into(),
        "submit" => "Bonsai is submitting the checked plan for producer review.".into(),
        _ => "Bonsai is applying a production action.".into(),
    }
}

fn producer_tool_result(message: &str) -> String {
    if message.starts_with("CHECK PASS") {
        "Native checks passed. Bonsai is completing the required whole-film review.".into()
    } else if message.starts_with("CHECK FAIL") {
        let issues = message.lines().filter(|line| line.starts_with('-')).count();
        format!(
            "Native review found {} issue{}; Bonsai is repairing the affected scenes.",
            issues.max(1),
            if issues == 1 { "" } else { "s" }
        )
    } else if message.starts_with("SUBMITTED") {
        "The checked plan was submitted successfully.".into()
    } else if message.starts_with("ERROR") || message.starts_with("SUBMIT BLOCKED") {
        "The production action was rejected safely; Bonsai received the exact diagnostic and will repair it.".into()
    } else {
        "The durable workspace accepted Bonsai's production action.".into()
    }
}

fn movie_agent_prompt() -> &'static str {
    prompts::MOVIE_AGENT_SYSTEM
}

fn reference_manifest(references: &[MovieReference]) -> String {
    if references.is_empty() {
        return String::new();
    }
    let mut manifest = String::from(
        "Producer-reference manifest. Descriptions below guide planning and asset placement only; they are never sent verbatim to H3. Put exact asset IDs in referenceIds only where the native media should condition that clip. Audio references condition H3's generated sound or voice; they are not editorial tracks, need not match output duration, and must never be trimmed, padded, looped, crossfaded, replaced, or extended with silence. Do not claim to have inspected media beyond these descriptions.\n",
    );
    for reference in references {
        let reference_type = if reference.kind == "audio" {
            "native clip audio"
        } else {
            reference.kind.as_str()
        };
        manifest.push_str(&format!(
            "\nAsset ID: {}\nType: {}\nProducer description: {}\n",
            reference.asset_id, reference_type, reference.description
        ));
        if reference.use_embedded_audio {
            manifest.push_str(&format!(
                "Existing embedded clip audio placement: {}\n",
                reference.embedded_audio_description
            ));
        }
    }
    manifest
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ReviewIssue {
    clip_number: u32,
    category: String,
    finding: String,
    required_fix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct MovieCodeReview {
    summary: String,
    issues: Vec<ReviewIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct MovieAssessment {
    approved: bool,
    score: u32,
    verdict: String,
    blocking_issues: Vec<ReviewIssue>,
}

fn clip_assistant_prompt() -> &'static str {
    prompts::CLIP_ASSISTANT_SYSTEM
}

fn clip_suggestion_schema() -> Value {
    json!({"type":"json_schema","json_schema":{"name":"kestrel_movie_scene_suggestion","strict":true,"schema":{
        "type":"object","additionalProperties":false,"properties":{
            "clipId":{"type":"string","maxLength":80},
            "summary":{"type":"string","minLength":10,"maxLength":1200},
            "checklist":{"type":"array","maxItems":12,"items":{"type":"string","minLength":3,"maxLength":400}},
            "clip":{"type":"object","additionalProperties":false,"properties":{
                "id":{"type":"string","maxLength":80},
                "title":{"type":"string","minLength":1,"maxLength":160},
                "purpose":{"type":"string","minLength":1,"maxLength":600},
                "durationSeconds":{"type":"number","minimum":5,"maximum":15},
                "prompt":{"type":"string"},
                "continuityIn":{"type":"string","maxLength":800},
                "continuityOut":{"type":"string","maxLength":800},
                "transition":{"type":"string","maxLength":300},
                "usePreviousFrame":{"type":"boolean"},
                "sourceRefs":{"type":"array","description":"Textual source-credit IDs only; never producer image, video, or audio asset IDs.","maxItems":24,"items":{"type":"string","maxLength":800}},
                "referenceIds":{"type":"array","description":"Producer image, video, and audio asset IDs attached natively to this H3 clip.","maxItems":12,"items":{"type":"string","maxLength":128}}
            },"required":["id","title","purpose","durationSeconds","prompt","continuityIn","continuityOut","transition","usePreviousFrame","sourceRefs","referenceIds"]}
        },"required":["clipId","summary","checklist","clip"]
    }}})
}

#[allow(dead_code)]
fn review_issue_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "clipNumber":{"type":"integer","minimum":0},
        "category":{"type":"string","maxLength":80},
        "finding":{"type":"string","maxLength":600},
        "requiredFix":{"type":"string","maxLength":600}
    },"required":["clipNumber","category","finding","requiredFix"]})
}

#[allow(dead_code)]
fn code_review_schema() -> Value {
    json!({"type":"json_schema","json_schema":{"name":"kestrel_movie_code_review","strict":true,"schema":{
        "type":"object","additionalProperties":false,"properties":{
            "summary":{"type":"string","maxLength":1200},
            "issues":{"type":"array","maxItems":24,"items":review_issue_schema()}
        },"required":["summary","issues"]
    }}})
}

#[allow(dead_code)]
fn assessment_schema() -> Value {
    json!({"type":"json_schema","json_schema":{"name":"kestrel_movie_assessment","strict":true,"schema":{
        "type":"object","additionalProperties":false,"properties":{
            "approved":{"type":"boolean"},
            "score":{"type":"integer","minimum":0,"maximum":100},
            "verdict":{"type":"string","maxLength":1200},
            "blockingIssues":{"type":"array","maxItems":24,"items":review_issue_schema()}
        },"required":["approved","score","verdict","blockingIssues"]
    }}})
}

fn prompt_quality_issues(plan: &MoviePlan, references: &[MovieReference]) -> Vec<String> {
    let mut issues = Vec::new();
    if !has_meaningful_prose(&plan.audience, 2) {
        issues.push("Plan audience is missing or placeholder metadata.".into());
    }
    if plan.continuity_bible.is_empty()
        || plan
            .continuity_bible
            .iter()
            .any(|entry| !has_meaningful_prose(entry, 4))
    {
        issues.push(
            "Continuity bible must contain meaningful identity, wardrobe, world, or staging facts rather than placeholders."
                .into(),
        );
    }
    let allowed_references = references
        .iter()
        .map(|reference| reference.asset_id.as_str())
        .collect::<HashSet<_>>();
    let markers = [
        (
            "camera direction",
            ["camera", "lens", "angle", "framing"].as_slice(),
        ),
        (
            "lighting or visual texture",
            [
                "light",
                "lighting",
                "palette",
                "texture",
                "film",
                "live-action",
            ]
            .as_slice(),
        ),
        (
            "audio direction",
            [
                "audio", "sound", "ambience", "score", "music", "dialogue", "voice", "foley",
            ]
            .as_slice(),
        ),
    ];
    for (index, clip) in plan.clips.iter().enumerate() {
        if !clip.duration_seconds.is_finite() || !(5.0..=15.0).contains(&clip.duration_seconds) {
            issues.push(format!(
                "Clip {} duration is {:.1}s; native H3 clips must be 5-15 seconds. Rebalance the scene list without relying on Kestrel to clamp or trim generated masters.",
                index + 1,
                clip.duration_seconds
            ));
        }
        let words = clip.prompt.split_whitespace().count();
        if !(MIN_H3_PROMPT_WORDS..=MAX_H3_PROMPT_WORDS).contains(&words) {
            issues.push(format!(
                "Clip {} prompt has {words} words; production prompts must contain {MIN_H3_PROMPT_WORDS}-{MAX_H3_PROMPT_WORDS} words.",
                index + 1
            ));
        }
        let lower = clip.prompt.to_ascii_lowercase();
        let organized_direction =
            format!("{} {} {}", clip.title, clip.purpose, clip.prompt).to_ascii_lowercase();
        if directs_unquoted_speech(&organized_direction) && !has_quoted_spoken_line(&clip.prompt) {
            issues.push(format!(
                "Clip {} directs speech or narration without exact quoted words. H3 will otherwise invent language and can produce fluent-sounding nonsense. Write short dialogue in quotation marks that comfortably fits the {:.1}s native duration, or state `No dialogue or narration.` If an audible murmur, mumble, or whisper is intentional but contains no language, state that it is `wordless and nonverbal`.",
                index + 1,
                clip.duration_seconds
            ));
        }

        let claims_prior_visual = [
            "previous frame",
            "prior frame",
            "previous shot",
            "prior shot",
            "previous scene",
            "prior scene",
        ]
        .iter()
        .any(|claim| organized_direction.contains(claim));
        let claims_continuation = ["continuation", "continuous", "continues", "continue"]
            .iter()
            .any(|claim| organized_direction.contains(claim));
        if !clip.use_previous_frame
            && (claims_prior_visual && claims_continuation
                || [
                    "exact previous-frame continuation",
                    "exact prior-frame continuation",
                    "previous-frame continuation",
                    "prior-frame continuation",
                ]
                .iter()
                .any(|claim| organized_direction.contains(claim)))
        {
            issues.push(format!(
                "Clip {} claims an exact prior-frame continuation, but usePreviousFrame is false, so H3 will not receive that frame. Either enable a reference-free previous-frame handoff from a genuinely matching preceding endpoint, or rewrite the scene as an honest independently conditioned cut.",
                index + 1
            ));
        }
        for (label, alternatives) in markers {
            if !alternatives.iter().any(|marker| lower.contains(marker)) {
                issues.push(format!("Clip {} prompt lacks {label}.", index + 1));
            }
        }
        if !has_timed_structure(&lower) {
            issues.push(format!(
                "Clip {} prompt lacks timed shot or beat structure.",
                index + 1
            ));
        }
        if let Some(maximum_timecode) = maximum_timecode_seconds(&lower) {
            if maximum_timecode > clip.duration_seconds + 0.25 {
                issues.push(format!(
                    "Clip {} prompt directs action through {maximum_timecode:.2}s, beyond its {:.2}s H3 duration. Rewrite timed beats to end exactly at the native clip boundary.",
                    index + 1,
                    clip.duration_seconds
                ));
            } else if maximum_timecode < clip.duration_seconds - 0.75 {
                issues.push(format!(
                    "Clip {} prompt stops timed coverage at {maximum_timecode:.2}s before its {:.2}s H3 duration. Direct the final hold, settle, reaction, picture, camera, and sound through the exact endpoint.",
                    index + 1,
                    clip.duration_seconds
                ));
            }
        }
        if contains_reference_tag(&clip.prompt) {
            issues.push(format!(
                "Clip {} contains a native H3 reference tag; only Kestrel may bind and renumber tags.",
                index + 1
            ));
        }
        for reference in references {
            if clip.prompt.contains(&reference.asset_id) {
                issues.push(format!(
                    "Clip {} leaks an internal asset ID into renderer prose; keep the ID only in referenceIds.",
                    index + 1
                ));
            }
            let workspace_id = reference
                .tag
                .trim_matches(['<', '>'])
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("-")
                .to_ascii_lowercase();
            if !workspace_id.is_empty() && organized_direction.contains(&workspace_id) {
                issues.push(format!(
                    "Clip {} leaks workspace reference ID {workspace_id} into organized renderer direction. Keep producer media IDs only in referenceIds on the exact scene where H3 should receive that media; remove the ID from title, purpose, and prompt.",
                    index + 1
                ));
            }
        }
        if clip.use_previous_frame && !clip.reference_ids.is_empty() {
            issues.push(format!(
                "Clip {} requests both an exact prior-frame continuation and native references. H3 uses mutually exclusive fl2va and ref2va conditioning, so one input path would be absent and the cut can jump in identity, wardrobe, pose, product shape, lighting, camera position, or audio. Minimally reorganize this boundary: choose a reference-locked motivated cut, or end the preceding referenced clip on the required pose and make this a reference-free previous-frame continuation; if necessary move, split, extend, shorten, or rebalance only the affected 5-15 second scenes while preserving story and approximate total runtime.",
                index + 1,
            ));
        }
        if clip.use_previous_frame
            && (index == 0
                || !has_meaningful_prose(&clip.continuity_in, 3)
                || !has_meaningful_prose(&plan.clips[index - 1].continuity_out, 3))
        {
            issues.push(format!(
                "Clip {} chains the prior frame without meaningful matching continuityOut and continuityIn instructions.",
                index + 1
            ));
        }
        if clip.use_previous_frame
            && index > 0
            && !continuity_handoff_matches(
                &plan.clips[index - 1].continuity_out,
                &clip.continuity_in,
            )
        {
            issues.push(format!(
                "Clip {} chains the prior frame, but clip {} continuityOut and clip {} continuityIn do not share at least two concrete handoff anchors. An exact carried frame must preserve the same visible subject/object, pose or action, geography, and time state; do not chain a flashback walk into a present-day close-up merely because both scenes name the same character. Rewrite the adjacent endpoints to describe the truthful matching frame, or make clip {} an independent reference-locked cut.",
                index + 1,
                index,
                index + 1,
                index + 1
            ));
        }
        let transition = clip.transition.to_ascii_lowercase();
        let is_last = index + 1 == plan.clips.len();
        if !is_last
            && ["end of film", "end film", "film ends", "end credits"]
                .iter()
                .any(|marker| transition.contains(marker))
        {
            issues.push(format!(
                "Clip {} declares a terminal '{}' transition, but scene {} follows it. Keep numbered scene files in truthful editorial order: move the ending to the actual last scene or rewrite this as the intended intermediate transition.",
                index + 1,
                clip.transition,
                index + 2
            ));
        }
        if is_last
            && ["next scene", "next shot", "next clip"]
                .iter()
                .any(|marker| transition.contains(marker))
        {
            issues.push(format!(
                "Final clip {} declares '{}', but no later scene exists. Give the actual last scene a truthful terminal transition or reorder the files so the promised next scene follows.",
                index + 1,
                clip.transition
            ));
        }
        if describes_child_age(clip)
            && ["beard", "moustache", "mustache", "stubble"]
                .iter()
                .any(|marker| organized_direction.contains(marker))
        {
            issues.push(format!(
                "Clip {} describes a child while retaining adult facial hair. Remove the contradictory anatomy and wardrobe inheritance, or minimally reorganize the memory around an age-consistent subject.",
                index + 1
            ));
        }
        for reference_id in &clip.reference_ids {
            if !allowed_references.contains(reference_id.as_str()) {
                issues.push(format!(
                    "Clip {} invents unknown producer reference {}.",
                    index + 1,
                    reference_id
                ));
            }
        }
        let native_audio_count = clip
            .reference_ids
            .iter()
            .filter(|reference_id| {
                references.iter().any(|reference| {
                    reference.asset_id.as_str() == reference_id.as_str()
                        && reference.kind == "audio"
                })
            })
            .count();
        if native_audio_count > 1 {
            issues.push(format!(
                "Clip {} selects multiple exact native-audio tracks; use one deterministic clip soundtrack.",
                index + 1
            ));
        }
    }
    let distinctive_tokens = plan
        .clips
        .iter()
        .map(|clip| distinctive_prompt_tokens(&clip.prompt))
        .collect::<Vec<_>>();
    for first in 0..plan.clips.len() {
        for second in first + 1..plan.clips.len() {
            let left = &plan.clips[first];
            let right = &plan.clips[second];
            let same_organized_scene = left.title.trim().eq_ignore_ascii_case(right.title.trim())
                && left
                    .purpose
                    .trim()
                    .eq_ignore_ascii_case(right.purpose.trim());
            if same_organized_scene {
                issues.push(format!(
                    "Clips {} and {} duplicate the same organized title and purpose. An unattended movie must not replay an accidental copy: rewrite one as editorially distinct coverage with its own action, framing, timed beats, sound, and story purpose while preserving the requested total runtime.",
                    first + 1,
                    second + 1
                ));
                continue;
            }
            let similarity = distinctive_prompt_similarity(
                &distinctive_tokens[first],
                &distinctive_tokens[second],
            );
            if similarity >= 0.60 {
                issues.push(format!(
                    "Clips {} and {} repeat near-duplicate renderer direction ({:.0}% distinctive-token overlap). Do not pad runtime by replaying the same action, emotion, framing, and sound under different titles; replace one with editorially distinct story coverage while preserving native duration and the intended total runtime.",
                    first + 1,
                    second + 1,
                    similarity * 100.0
                ));
            }
        }
    }
    let selected = plan
        .clips
        .iter()
        .flat_map(|clip| clip.reference_ids.iter())
        .collect::<HashSet<_>>();
    for reference in references {
        if !selected.contains(&reference.asset_id) {
            issues.push(format!(
                "Producer reference {} is never assigned to a clip.",
                reference.asset_id
            ));
        }
    }
    issues
}

fn producer_intent_issues(
    prompt: &str,
    plan: &MoviePlan,
    references: &[MovieReference],
) -> Vec<String> {
    let mut issues = Vec::new();
    let supplied_attribution = std::iter::once(prompt)
        .chain(
            references
                .iter()
                .map(|reference| reference.description.as_str()),
        )
        .any(|text| {
            let lower = text.to_ascii_lowercase();
            [
                "credit:",
                "credits:",
                "source:",
                "sources:",
                "courtesy of",
                "licensed from",
                "http://",
                "https://",
            ]
            .iter()
            .any(|marker| lower.contains(marker))
        });
    if !plan.source_credits.is_empty() && !supplied_attribution {
        issues.push(
            "The plan invents source credits even though the producer supplied no attribution. Leave sourceCredits empty; an offline movie agent must never fabricate publications, organizations, research, licenses, or provenance."
                .into(),
        );
    }
    let planned_seconds = plan
        .clips
        .iter()
        .map(|clip| clip.duration_seconds.clamp(5.0, 15.0))
        .sum::<f32>();
    let requested_range = requested_duration_range(prompt);
    if let Some((minimum, maximum)) = requested_range {
        if !(minimum..=maximum).contains(&planned_seconds) {
            issues.push(format!(
                "Producer requested {:.0}-{:.0} seconds, but the plan totals {planned_seconds:.1} seconds. Add, remove, split, or rebalance 5-15 second scenes to stay inside the requested runtime without padding or trimming generated masters.",
                minimum, maximum
            ));
        }
    }
    if requested_range.is_some_and(|(_, maximum)| maximum >= 60.0) && plan.clips.len() >= 4 {
        if !plan.clips.iter().any(|clip| clip.use_previous_frame) {
            issues.push(
                "Long-form narrative plan has no exact previous-frame continuation. Create at least one semantically continuous adjacent scene pair while preserving independent cuts elsewhere. Choose a boundary where subject, action, geography, camera position, lighting, and screen direction genuinely continue; keep the first scene reference-locked when identity media is needed, end it on the handoff pose, then make the next scene usePreviousFrame=true with empty referenceIds. Do not chain into an unrelated establisher, insert, flashback, time jump, or different subject merely to satisfy this check."
                    .into(),
            );
        }
        if !plan.clips.iter().any(|clip| !clip.use_previous_frame) {
            issues.push(
                "Long-form narrative plan chains every scene. Preserve at least one independent motivated cut for a change of subject, place, time, scale, or editorial emphasis."
                    .into(),
            );
        }
    }
    for reference in references.iter().filter(|reference| {
        matches!(reference.kind.as_str(), "image" | "video")
            && is_identity_reference_description(&reference.description)
    }) {
        let Some(subject) = identity_reference_subject(&reference.description) else {
            continue;
        };
        let subject_aliases = identity_subject_aliases(plan, &subject);
        if requested_range.is_some_and(|(_, maximum)| maximum >= 60.0)
            && !plan.clips.iter().any(|clip| {
                !clip.use_previous_frame
                    && !clip.reference_ids.contains(&reference.asset_id)
                    && clip_is_independent_subject_free(clip, &subject_aliases)
            })
        {
            issues.push(format!(
                "Long-form film has no independently cut subject-free coverage for the referenced {subject}. Add one purposeful animal-only, environment-only, object insert, or establishing scene where the {subject} is explicitly absent; do not infer this merely from an empty referenceIds list, because continuations and age variants can still contain the same character."
            ));
        }
        let mut identity_carried = false;
        let mut uncovered = Vec::new();
        let mut overconditioned = Vec::new();
        for (index, clip) in plan.clips.iter().enumerate() {
            let visibly_features_subject = clip_visibly_names_subject(clip, &subject_aliases);
            let directly_referenced = clip.reference_ids.contains(&reference.asset_id);
            let carried_into_clip = clip.use_previous_frame && identity_carried;
            let covered = directly_referenced || carried_into_clip;
            identity_carried = covered;
            if visibly_features_subject && !covered {
                uncovered.push(index + 1);
            } else if directly_referenced && !visibly_features_subject {
                overconditioned.push(index + 1);
            }
            if directly_referenced
                && describes_child_age(clip)
                && !description_supplies_child_identity(&reference.description)
            {
                issues.push(format!(
                    "Clip {} asks H3 to turn identity reference {} into a child or materially younger version, but the producer did not describe that image as the matching child-age identity. Ref2va preserves the supplied appearance and cannot guarantee this age transformation. Minimally keep the referenced character at the supplied age, make the childhood memory POV/off-screen or subject-free, or use a separately supplied age-matched reference; do not preserve adult beard, wardrobe, or equipment on a child.",
                    index + 1,
                    reference.asset_id
                ));
            }
        }
        if !uncovered.is_empty() {
            issues.push(format!(
                "Producer identity reference {} is for the {subject}, but independently cut scene(s) {} visibly name that subject without native identity conditioning. Attach this reference to each independent appearance. A genuinely continuous scene may instead usePreviousFrame=true with empty referenceIds when the preceding covered scene ends on the exact handoff pose. Keep POV or off-screen scenes explicitly labeled so the reference does not force the subject into protagonist-free coverage.",
                reference.asset_id,
                uncovered
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !overconditioned.is_empty() {
            issues.push(format!(
                "Producer identity reference {} is for the {subject}, but scene(s) {} do not visibly feature that subject according to their organized title and purpose. Remove the reference from those protagonist-free, insert, animal, POV, or off-screen cuts so H3 does not inject the identity; if the subject truly is visible, make that explicit in the scene purpose and keep the reference.",
                reference.asset_id,
                overconditioned
                    .iter()
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    for reference in references.iter().filter(|reference| {
        reference.kind == "audio"
            && reference.description.to_ascii_lowercase().contains("only")
            && ["flashback", "memory", "past"]
                .iter()
                .any(|marker| reference.description.to_ascii_lowercase().contains(marker))
    }) {
        let assigned = plan
            .clips
            .iter()
            .filter(|clip| clip.reference_ids.contains(&reference.asset_id))
            .collect::<Vec<_>>();
        if assigned.len() != 1 {
            issues.push(format!(
                "Producer limited audio reference {} to one flashback/memory scene, but it is assigned to {} scenes.",
                reference.asset_id,
                assigned.len()
            ));
        } else {
            let scene = format!(
                "{} {} {}",
                assigned[0].title, assigned[0].purpose, assigned[0].prompt
            )
            .to_ascii_lowercase();
            if !["flashback", "memory", "past"]
                .iter()
                .any(|marker| scene.contains(marker))
            {
                issues.push(format!(
                    "Producer limited audio reference {} to a flashback/memory scene, but its assigned scene is not explicitly a flashback, memory, or past-time scene.",
                    reference.asset_id
                ));
            }
        }
    }
    for reference in references.iter().filter(|reference| {
        reference.kind == "audio" && audio_reference_describes_speech(&reference.description)
    }) {
        for (index, clip) in plan
            .clips
            .iter()
            .enumerate()
            .filter(|(_, clip)| clip.reference_ids.contains(&reference.asset_id))
        {
            let lower = clip.prompt.to_ascii_lowercase();
            let assigns_voice = [
                "attached voice",
                "supplied voice",
                "reference voice",
                "voice reference",
                "voice identity",
                "voice/timbre",
                "vocal timbre",
                "speaker's voice",
                "speaker’s voice",
            ]
            .iter()
            .any(|marker| lower.contains(marker));
            if !assigns_voice || !has_quoted_spoken_line(&clip.prompt) {
                issues.push(format!(
                    "Producer audio reference {} is described as speech or a voice, but clip {} does not both assign the supplied voice/timbre and state exact dialogue in quotation marks. H3 uses audio as generative voice conditioning rather than guaranteed waveform playback. Add a literal role sentence such as: Use the supplied voice reference as the speaker's voice identity and vocal timbre. Then give the speaker short exact words that fit the native {:.1}s scene so H3 does not invent or garble narration. Do not trim, mux, replace, or pad the generated master.",
                    reference.asset_id,
                    index + 1,
                    clip.duration_seconds
                ));
            }
        }
    }
    issues
}

fn is_identity_reference_description(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    [
        "identity reference",
        "identity and wardrobe reference",
        "identity/wardrobe reference",
        "character identity reference",
        "character reference",
        "appearance reference",
        "face reference",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
        || ((lower.contains("preserve the same face") || lower.contains("keep the same face"))
            && (lower.contains("whenever ") || lower.contains("when ")))
}

fn identity_reference_subject(description: &str) -> Option<String> {
    let lower = description.to_ascii_lowercase();
    if let Some(index) = lower.find("identity reference") {
        let owner = lower[..index]
            .trim_end_matches(|character: char| {
                character.is_whitespace() || matches!(character, '.' | ',' | ':' | ';')
            })
            .split(|character: char| !character.is_alphanumeric() && character != '\'')
            .rfind(|token| token.ends_with("'s"));
        if let Some(owner) = owner {
            return Some(owner.trim_end_matches("'s").to_owned());
        }
    }
    for marker in [
        "identity and wardrobe reference for ",
        "identity/wardrobe reference for ",
        "character identity reference for ",
        "identity reference for ",
        "character reference for ",
        "appearance reference for ",
        "face reference for ",
    ] {
        if let Some(after) = lower.split_once(marker).map(|(_, after)| after) {
            let candidate = after
                .trim_start_matches([',', ':', ';', ' '])
                .trim_start_matches("the ")
                .split([',', '.', ';', ':', '\n'])
                .next()
                .unwrap_or_default()
                .trim();
            if has_meaningful_prose(candidate, 1) {
                return Some(candidate.to_owned());
            }
        }
    }
    for marker in [
        "identity reference",
        "identity and wardrobe reference",
        "identity/wardrobe reference",
        "character identity reference",
        "character reference",
        "appearance reference",
        "face reference",
    ] {
        if let Some(index) = lower.find(marker) {
            let before = lower[..index].trim_end_matches(|character: char| {
                character.is_whitespace() || matches!(character, '.' | ',' | ':' | ';')
            });
            let candidate = before
                .split(|character: char| !character.is_alphanumeric() && character != '\'')
                .rfind(|token| !token.is_empty())
                .unwrap_or_default()
                .trim_end_matches("'s");
            if !matches!(candidate, "" | "a" | "an" | "the" | "this" | "is") {
                return Some(candidate.to_owned());
            }
        }
    }
    ["whenever ", "when "].iter().find_map(|marker| {
        let after = lower.split_once(marker)?.1;
        let candidate = after
            .split_once(" is ")?
            .0
            .trim_start_matches("the ")
            .trim();
        has_meaningful_prose(candidate, 1).then(|| candidate.to_owned())
    })
}

fn legacy_alias(fact: &str, subject: &str) -> Option<String> {
    let prefix = format!("{subject}:");
    fact.trim()
        .strip_prefix(&prefix)
        .and_then(|details| {
            details
                .split(|character: char| !character.is_alphanumeric())
                .find(|token| !token.is_empty())
        })
        .map(str::to_owned)
}

fn structured_alias(fact: &str, subject: &str) -> Option<String> {
    let fields = fact
        .split(';')
        .filter_map(|field| field.split_once(':'))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect::<Vec<_>>();
    fields
        .iter()
        .any(|(key, value)| *key == "subject" && *value == subject)
        .then(|| {
            fields
                .iter()
                .find(|(key, _)| *key == "name")
                .and_then(|(_, value)| {
                    value
                        .split(|character: char| !character.is_alphanumeric())
                        .find(|token| !token.is_empty())
                })
        })
        .flatten()
        .map(str::to_owned)
}

fn role_leading_alias(fact: &str, subject: &str) -> Option<String> {
    fact.trim()
        .strip_prefix(subject)
        .filter(|details| details.starts_with(char::is_whitespace))
        .and_then(|details| {
            let heading = details.split(':').next()?.trim();
            (!heading.contains(char::is_whitespace)
                && !matches!(
                    heading,
                    "" | "appearance" | "description" | "equipment" | "identity" | "wardrobe"
                ))
            .then(|| heading.to_owned())
        })
}

fn role_leading_prose_alias(fact: &str, subject: &str) -> Option<String> {
    fact.trim()
        .strip_prefix(subject)
        .filter(|details| details.starts_with(char::is_whitespace))
        .and_then(|details| {
            details
                .split(|character: char| !character.is_alphanumeric())
                .find(|token| !token.is_empty())
        })
        .filter(|alias| {
            !matches!(
                *alias,
                "appearance" | "description" | "equipment" | "identity" | "wardrobe"
            )
        })
        .map(str::to_owned)
}

fn prose_alias(fact: &str, subject: &str) -> Option<String> {
    fact.find(subject).and_then(|subject_index| {
        let before_subject = &fact[..subject_index];
        let introduction = before_subject.split(',').next()?.trim();
        let alias = introduction
            .split(|character: char| !character.is_alphanumeric())
            .find(|token| !token.is_empty())?;
        (!matches!(
            alias,
            "a" | "an" | "the" | "this" | "same" | "present" | "recurring"
        ) && before_subject.contains(','))
        .then(|| alias.to_owned())
    })
}

fn identity_subject_aliases(plan: &MoviePlan, subject: &str) -> Vec<String> {
    let mut aliases = vec![subject.to_owned()];
    for fact in &plan.continuity_bible {
        let lower = fact.to_ascii_lowercase();
        for alias in legacy_alias(&lower, subject)
            .into_iter()
            .chain(structured_alias(&lower, subject))
            .chain(role_leading_alias(&lower, subject))
            .chain(role_leading_prose_alias(&lower, subject))
            .chain(prose_alias(&lower, subject))
        {
            if !aliases.iter().any(|known| known == &alias) {
                aliases.push(alias);
            }
        }
    }
    aliases
}

fn continuity_handoff_matches(previous_out: &str, current_in: &str) -> bool {
    const GENERIC: &[&str] = &[
        "camera",
        "clip",
        "continue",
        "continues",
        "continuation",
        "frame",
        "handoff",
        "holds",
        "same",
        "scene",
        "shot",
        "still",
        "with",
    ];
    let anchors = |value: &str| {
        value
            .to_ascii_lowercase()
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|token| token.len() >= 4 && !GENERIC.contains(token))
            .map(str::to_owned)
            .collect::<HashSet<_>>()
    };
    let previous = anchors(previous_out);
    let current = anchors(current_in);
    previous.intersection(&current).take(2).count() >= 2
}

fn describes_child_age(clip: &PlannedClip) -> bool {
    let description =
        format!("{} {} {}", clip.title, clip.purpose, clip.prompt).to_ascii_lowercase();
    if [
        "childhood",
        "child version",
        "younger version",
        "young boy",
        "young girl",
    ]
    .iter()
    .any(|marker| description.contains(marker))
    {
        return true;
    }
    let tokens = description
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    tokens
        .windows(2)
        .any(|window| window[0] == "age" && window[1].parse::<u8>().is_ok_and(|age| age <= 15))
        || tokens.windows(3).any(|window| {
            window[1] == "year"
                && window[2] == "old"
                && window[0].parse::<u8>().is_ok_and(|age| age <= 15)
        })
}

fn description_supplies_child_identity(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    [
        "child",
        "childhood",
        "young boy",
        "young girl",
        "younger identity",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn clip_visibly_names_subject(clip: &PlannedClip, subjects: &[String]) -> bool {
    let description =
        format!("{} {} {}", clip.title, clip.purpose, clip.prompt).to_ascii_lowercase();
    let subject_named = subjects.iter().any(|subject| description.contains(subject));
    let subject_explicitly_absent = subjects.iter().any(|subject| {
        [format!("no {subject}"), format!("without {subject}")]
            .iter()
            .any(|marker| description.contains(marker))
    });
    let subject_explicitly_visible = subjects.iter().any(|subject| {
        [
            format!("{subject} is visible"),
            format!("{subject} visible"),
            format!("show the {subject}"),
            format!("shows the {subject}"),
            format!("{subject} appears"),
            format!("{subject} in frame"),
            format!("{subject} on camera"),
        ]
        .iter()
        .any(|marker| description.contains(marker))
    });
    let subject_pov_only = subjects.iter().any(|subject| {
        [
            format!("{subject} pov"),
            format!("{subject}'s pov"),
            format!("what the {subject} sees"),
            format!("from the {subject}'s perspective"),
            format!("{subject}'s subjective point of view"),
        ]
        .iter()
        .any(|marker| description.contains(marker))
    });
    subject_named
        && !subject_explicitly_absent
        && (!subject_pov_only || subject_explicitly_visible)
        && ![
            "offscreen",
            "off-screen",
            "absent from frame",
            "not visible",
            "behind the camera",
        ]
        .iter()
        .any(|marker| description.contains(marker))
}

fn clip_is_independent_subject_free(clip: &PlannedClip, subjects: &[String]) -> bool {
    let description =
        format!("{} {} {}", clip.title, clip.purpose, clip.prompt).to_ascii_lowercase();
    let explicitly_absent = [
        "subject-free",
        "protagonist-free",
        "animal-only",
        "environment-only",
    ]
    .iter()
    .any(|marker| description.contains(marker))
        || subjects.iter().any(|subject| {
            [
                format!("no {subject}"),
                format!("without {subject}"),
                format!("{subject} is absent"),
                format!("{subject} remains off-screen"),
                format!("{subject} remains offscreen"),
            ]
            .iter()
            .any(|marker| description.contains(marker))
        });
    explicitly_absent || !subjects.iter().any(|subject| description.contains(subject))
}

fn audio_reference_describes_speech(description: &str) -> bool {
    let lower = description.to_ascii_lowercase();
    [
        "voice",
        "spoken",
        "speech",
        "dialogue",
        "dialog",
        "narration",
        "narrator",
        "vocal",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn directs_unquoted_speech(direction: &str) -> bool {
    let direction = direction.to_ascii_lowercase();
    let mut affirmative = direction.clone();
    for negative in [
        "no speech, narration, or dialogue",
        "no dialogue, speech, or narration",
        "no dialogue or narration",
        "no speech or narration",
        "without dialogue or narration",
        "without speech or narration",
        "no spoken dialogue",
        "no spoken words",
        "without spoken dialogue",
        "without spoken words",
        "no-dialogue",
        "no-speech",
        "no-narration",
        "no dialogue",
        "no speech",
        "no narration",
        "without dialogue",
        "without speech",
        "without narration",
        "does not speak",
        "do not speak",
        "never speaks",
        "never speak",
    ] {
        affirmative = affirmative.replace(negative, " ");
    }

    let words = affirmative
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let directs_lexical_speech = words.iter().any(|word| {
        [
            "dialogue",
            "mutter",
            "mutters",
            "narrate",
            "narrated",
            "narrates",
            "narrating",
            "narration",
            "narrator",
            "saying",
            "says",
            "speak",
            "speaking",
            "speaks",
            "spoken",
            "utter",
            "uttering",
            "utters",
            "voice-over",
            "voiceover",
        ]
        .contains(word)
    }) || affirmative.contains("introduces himself")
        || affirmative.contains("introduces herself");
    if directs_lexical_speech {
        return true;
    }

    let explicitly_nonverbal = [
        "nonverbal",
        "non-verbal",
        "wordless",
        "no words",
        "without words",
    ]
    .iter()
    .any(|marker| direction.contains(marker));
    !explicitly_nonverbal && contains_non_environmental_vocalization(&affirmative)
}

fn contains_non_environmental_vocalization(direction: &str) -> bool {
    let words = direction
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    for (index, word) in words.iter().enumerate() {
        if ![
            "murmur", "murmurs", "mumble", "mumbles", "whisper", "whispers",
        ]
        .contains(word)
        {
            continue;
        }
        let nearby = &words[index.saturating_sub(3)..(index + 5).min(words.len())];
        if nearby.iter().any(|word| {
            [
                "actor",
                "boy",
                "character",
                "child",
                "dog",
                "girl",
                "man",
                "narrator",
                "person",
                "speaker",
                "subject",
                "woman",
            ]
            .contains(word)
        }) {
            return true;
        }
        if nearby.iter().any(|word| {
            [
                "air", "branch", "branches", "canopy", "dust", "foliage", "forest", "grass",
                "leaf", "leaves", "rain", "reeds", "sand", "stream", "tree", "trees", "water",
                "wind", "winds",
            ]
            .contains(word)
        }) {
            continue;
        }
        return true;
    }
    false
}

fn has_quoted_spoken_line(prompt: &str) -> bool {
    [('"', '"'), ('“', '”'), ('‘', '’')]
        .iter()
        .any(|(opening, closing)| {
            let Some(start) = prompt.find(*opening) else {
                return false;
            };
            let remainder = &prompt[start + opening.len_utf8()..];
            let Some(end) = remainder.find(*closing) else {
                return false;
            };
            remainder[..end]
                .split_whitespace()
                .filter(|word| word.chars().any(char::is_alphabetic))
                .count()
                >= 2
        })
}

fn distinctive_prompt_tokens(prompt: &str) -> HashSet<String> {
    const BOILERPLATE: &[&str] = &[
        "about",
        "after",
        "again",
        "ambient",
        "audio",
        "camera",
        "clip",
        "continue",
        "continues",
        "during",
        "frame",
        "handheld",
        "immersive",
        "natural",
        "scene",
        "shot",
        "sound",
        "sounds",
        "their",
        "through",
        "toward",
        "vlogger",
        "while",
        "with",
    ];
    prompt
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 4 && !BOILERPLATE.contains(token))
        .map(str::to_owned)
        .collect()
}

fn distinctive_prompt_similarity(left: &HashSet<String>, right: &HashSet<String>) -> f32 {
    let union = left.union(right).count();
    if union == 0 {
        return 0.0;
    }
    left.intersection(right).count() as f32 / union as f32
}

fn requested_duration_range(prompt: &str) -> Option<(f32, f32)> {
    let normalized = prompt.to_ascii_lowercase().replace(['-', '–', '—'], " to ");
    let tokens = normalized
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '.')
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    for window in tokens.windows(4) {
        let (Ok(first), Ok(second)) = (window[0].parse::<f32>(), window[2].parse::<f32>()) else {
            continue;
        };
        if window[1] == "to" && window[3].starts_with("minute") {
            return Some((first.min(second) * 60.0, first.max(second) * 60.0));
        }
    }
    for window in tokens.windows(2) {
        if let Ok(minutes) = window[0].parse::<f32>() {
            if window[1].starts_with("minute") {
                return Some((minutes * 54.0, minutes * 66.0));
            }
        }
    }
    None
}

fn has_meaningful_prose(value: &str, minimum_words: usize) -> bool {
    value
        .split_whitespace()
        .filter(|word| word.chars().any(char::is_alphabetic))
        .count()
        >= minimum_words
}

fn maximum_timecode_seconds(prompt: &str) -> Option<f32> {
    let characters = prompt.chars().collect::<Vec<_>>();
    let mut maximum: Option<f32> = None;
    let mut index = 0;
    while index < characters.len() {
        if !characters[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < characters.len()
            && (characters[index].is_ascii_digit()
                || characters[index] == '.'
                || characters[index] == ':')
        {
            index += 1;
        }
        let raw_token = characters[start..index].iter().collect::<String>();
        let token = raw_token.trim_end_matches(':');
        let seconds = if let Some((minutes, seconds)) = token.split_once(':') {
            match (minutes.parse::<f32>(), seconds.parse::<f32>()) {
                (Ok(minutes), Ok(seconds))
                    if minutes == 0.0
                        || characters[..start]
                            .iter()
                            .rev()
                            .find(|character| !character.is_whitespace())
                            .is_some_and(|character| ['[', '(', '-'].contains(character)) =>
                {
                    Some(minutes * 60.0 + seconds)
                }
                _ => None,
            }
        } else if characters
            .get(index)
            .is_some_and(|character| *character == 's')
            && seconds_token_is_timing_context(&characters, start, index + 1)
        {
            token.parse::<f32>().ok()
        } else {
            None
        };
        if let Some(seconds) = seconds {
            maximum = Some(maximum.map_or(seconds, |current| current.max(seconds)));
        }
    }
    maximum
}

fn seconds_token_is_timing_context(characters: &[char], start: usize, end: usize) -> bool {
    if start >= 2
        && ['-', '\u{2013}', '\u{2014}'].contains(&characters[start - 1])
        && characters[start - 2].is_alphabetic()
        && characters[start - 2] != 's'
    {
        // Do not interpret prose such as "mid-30s" as a 30-second shot marker.
        return false;
    }
    let previous = characters[..start]
        .iter()
        .rev()
        .find(|character| !character.is_whitespace())
        .copied();
    let next = characters[end..]
        .iter()
        .find(|character| !character.is_whitespace())
        .copied();
    if previous.is_some_and(|character| ['[', '-', '\u{2013}', '\u{2014}'].contains(&character))
        || next.is_some_and(|character| [']', '-', '\u{2013}', '\u{2014}'].contains(&character))
    {
        return true;
    }
    let context_start = start.saturating_sub(24);
    let prefix = characters[context_start..start]
        .iter()
        .collect::<String>()
        .to_ascii_lowercase();
    [
        "through", "until", "from", " to ", " at ", "beat", "shot", "hold",
    ]
    .iter()
    .any(|marker| prefix.contains(marker))
}

fn has_timed_structure(lower_prompt: &str) -> bool {
    [
        "timeline",
        "timecode",
        "shot 1",
        "shot one",
        "beat 1",
        "beat one",
        "timed beat",
    ]
    .iter()
    .any(|marker| lower_prompt.contains(marker))
        || maximum_timecode_seconds(lower_prompt).is_some()
}

#[allow(dead_code)]
fn review_failure_summary(
    native_issues: &[String],
    code_review: &MovieCodeReview,
    assessment: &MovieAssessment,
) -> String {
    let first_native = native_issues.first().map(String::as_str);
    let first_review = code_review
        .issues
        .first()
        .map(|issue| issue.finding.as_str());
    let first_assessment = assessment
        .blocking_issues
        .first()
        .map(|issue| issue.finding.as_str());
    first_native
        .or(first_assessment)
        .or(first_review)
        .unwrap_or(assessment.verdict.as_str())
        .chars()
        .take(500)
        .collect()
}

#[allow(dead_code)]
fn demands_unavailable_reference(issue: &ReviewIssue) -> bool {
    let text = format!("{} {}", issue.finding, issue.required_fix).to_ascii_lowercase();
    [
        "reference id",
        "asset id",
        "reference image",
        "image reference",
        "visual reference",
        "reference url",
        "provide a reference",
        "supply a reference",
        "attach a reference",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

#[allow(dead_code)]
fn misreads_audio_reference_as_editorial_track(issue: &ReviewIssue) -> bool {
    let text = format!("{} {}", issue.finding, issue.required_fix).to_ascii_lowercase();
    text.contains("audio")
        && [
            "crossfade",
            "pad with silence",
            "audio void",
            "duration mismatch",
            "source duration",
            "trim the audio",
            "loop the audio",
        ]
        .iter()
        .any(|marker| text.contains(marker))
}

#[allow(dead_code)]
fn movie_schema(max_clips: u32) -> Value {
    json!({"type":"json_schema","json_schema":{"name":"kestrel_movie_plan","strict":true,"schema":{
        "type":"object","additionalProperties":false,
        "properties":{
            "title":{"type":"string","minLength":1,"maxLength":160},
            "logline":{"type":"string","minLength":20,"maxLength":600},
            "audience":{"type":"string","minLength":3,"maxLength":300,"description":"Intended viewers and selling context in plain language, never a duration or number."},
            "creativeDirection":{"type":"string","minLength":40,"maxLength":2400},
            "continuityBible":{"type":"array","minItems":1,"maxItems":24,"description":"Reusable identity, wardrobe, prop, geography, screen-direction, lighting, and sound facts that must remain stable across clips; never IDs or placeholders.","items":{"type":"string","minLength":20,"maxLength":800}},
            "sourceCredits":{"type":"array","maxItems":24,"items":{"type":"string","maxLength":800}},
            "clips":{"type":"array","minItems":1,"maxItems":max_clips,"items":{"type":"object","additionalProperties":false,"properties":{
                "title":{"type":"string"},"purpose":{"type":"string"},"durationSeconds":{"type":"number","minimum":5,"maximum":15},"prompt":{"type":"string"},
                "continuityIn":{"type":"string"},"continuityOut":{"type":"string"},"transition":{"type":"string"},"usePreviousFrame":{"type":"boolean"},"sourceRefs":{"type":"array","description":"Textual source-credit IDs only; never producer image, video, or audio asset IDs.","items":{"type":"string"}},"referenceIds":{"type":"array","description":"Producer image, video, and audio asset IDs attached natively to this H3 clip.","items":{"type":"string"}}
            },"required":["title","purpose","durationSeconds","prompt","continuityIn","continuityOut","transition","usePreviousFrame","sourceRefs","referenceIds"]}}
        },"required":["title","logline","audience","creativeDirection","continuityBible","sourceCredits","clips"]
    }}})
}

fn movie_agent_request(
    model_id: &str,
    messages: &[Value],
    tools: &Value,
    settings: &MovieSettings,
    runtime_max_output_tokens: u32,
) -> Value {
    json!({
        "model": model_id,
        "messages": sanitize_chat_messages(messages),
        "tools": tools,
        "tool_choice": "required",
        "parallel_tool_calls": false,
        "stream": false,
        "temperature": settings.temperature,
        "top_p": settings.top_p,
        "top_k": settings.top_k,
        "max_tokens": settings.max_output_tokens.min(runtime_max_output_tokens),
        "thinking_budget_tokens": MOVIE_THINKING_BUDGET,
    })
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
    preview_available: bool,
}

struct ClipRenderContext<'a> {
    variant: Option<&'a str>,
    app: Option<&'a AppHandle>,
}

struct H3ReferenceInput<'a> {
    kind: &'a str,
    file: String,
    use_embedded_audio: bool,
}

fn bound_reference_prompt(references: &[H3ReferenceInput<'_>]) -> String {
    let mut bindings = Vec::new();
    let mut picture = 0usize;
    for _reference in references
        .iter()
        .filter(|reference| reference.kind == "image")
    {
        picture += 1;
        bindings.push(format!("Use <Picture {picture}> as a visual reference."));
    }
    let mut video = 0usize;
    let mut audio = 0usize;
    for reference in references
        .iter()
        .filter(|reference| reference.kind == "video")
    {
        if reference.use_embedded_audio {
            audio += 1;
            bindings.push(format!("Use <Audio {audio}> exactly as it is."));
        }
        video += 1;
        bindings.push(format!("Use <Video {video}> as a motion reference."));
    }
    for _reference in references
        .iter()
        .filter(|reference| reference.kind == "audio")
    {
        audio += 1;
        bindings.push(format!("Use <Audio {audio}> exactly as it is."));
    }
    bindings.join(" ")
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
        preview_available,
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
    if preview_available {
        graph[PREVIEW_NODE_ID] = preview_node("1", 12);
        graph["7"]["inputs"]["model"] = json!([PREVIEW_NODE_ID, 0]);
        graph["9"]["inputs"]["model"] = json!([PREVIEW_NODE_ID, 0]);
    }
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
            preview_available: true,
        });
        assert_eq!(graph["5"]["inputs"]["length"], 124);
        assert_eq!(graph["90"]["class_type"], "ModelPreviewOverrideKJ");
        assert_eq!(graph["90"]["inputs"]["preview_frames"], 12);
        assert_eq!(graph["7"]["inputs"]["model"], json!(["90", 0]));
        assert_eq!(graph["9"]["inputs"]["model"], json!(["90", 0]));
    }

    #[test]
    fn h3_graph_runs_without_the_optional_preview_node() {
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
            preview_available: false,
        });
        assert!(graph.get(PREVIEW_NODE_ID).is_none());
        assert_eq!(graph["7"]["inputs"]["model"], json!(["1", 0]));
        assert_eq!(graph["9"]["inputs"]["model"], json!(["1", 0]));
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
            preview_available: true,
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
                use_embedded_audio: false,
            },
            H3ReferenceInput {
                kind: "image",
                file: "kestrel/hero.png".into(),
                use_embedded_audio: false,
            },
            H3ReferenceInput {
                kind: "video",
                file: "kestrel/move.mp4".into(),
                use_embedded_audio: true,
            },
        ];
        let prompt = bound_reference_prompt(&references);
        assert!(prompt.contains("Use <Picture 1> as a visual reference."));
        assert!(prompt.contains("Use <Video 1> as a motion reference."));
        assert!(prompt.contains("Use <Audio 1> exactly as it is."));
        assert!(prompt.contains("Use <Audio 2> exactly as it is."));
        assert!(!prompt.contains("voice"));
        assert!(!prompt.contains("producer"));

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
            preview_available: true,
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
    fn movie_thinking_is_always_maximum() {
        let settings = MovieSettings {
            thinking_budget: 0,
            ..MovieSettings::default()
        }
        .validate(true)
        .unwrap();
        assert_eq!(settings.thinking_budget, MOVIE_THINKING_BUDGET);
        assert_eq!(
            MovieSettings::default().thinking_budget,
            MOVIE_THINKING_BUDGET
        );
    }

    #[test]
    fn legacy_edit_decisions_gain_safe_timeline_defaults() {
        let mut project: MovieProject = serde_json::from_value(json!({
            "schemaVersion": 4,
            "id": "e3e9e619-7e6a-4eed-a433-53c9e01ad99f",
            "prompt": "test", "title": "Legacy", "status": "complete",
            "phase": "complete", "detail": "ready",
            "createdAt": "2026-08-12T00:00:00Z", "updatedAt": "2026-08-12T00:00:00Z",
            "model": "test", "renderer": "test", "settings": {},
            "clips": [{
                "id": "clip-001", "index": 0, "title": "One", "prompt": "prompt",
                "durationSeconds": 5.0, "seed": 1, "status": "complete", "path": "one.mp4"
            }],
            "edit": { "clips": [{
                "clipId": "clip-001", "enabled": true, "order": 0,
                "trimStart": 0.0, "trimEnd": 0.0, "audioGain": 1.0
            }], "exportTitle": "Legacy" },
            "finalPath": "", "error": "", "producerReviewRequired": false,
            "producerApprovedAt": ""
        }))
        .unwrap();
        normalize_movie_project(&mut project);
        let decision = &project.edit.clips[0];
        assert_eq!(project.schema_version, SCHEMA_VERSION);
        assert_eq!(decision.speed, 1.0);
        assert_eq!(decision.id, "edit-clip-001-1");
        assert_eq!(project.edit.export_preset, "publish");
        assert_eq!(project.edit.target_lufs, -14.0);
        assert!(project.exports.is_empty());
    }

    #[test]
    fn timeline_validation_supports_repeated_sources_and_rejects_overlapping_fades() {
        let mut project: MovieProject = serde_json::from_value(json!({
            "schemaVersion": 5,
            "id": "e3e9e619-7e6a-4eed-a433-53c9e01ad99f",
            "prompt": "test", "title": "Timeline", "status": "complete",
            "phase": "complete", "detail": "ready",
            "createdAt": "2026-08-12T00:00:00Z", "updatedAt": "2026-08-12T00:00:00Z",
            "model": "test", "renderer": "test", "settings": {},
            "clips": [{
                "id": "clip-001", "index": 0, "title": "One", "prompt": "prompt",
                "durationSeconds": 5.0, "seed": 1, "status": "complete", "path": "one.mp4"
            }],
            "edit": { "clips": [], "exportTitle": "Timeline" },
            "finalPath": "", "error": "", "producerReviewRequired": false,
            "producerApprovedAt": ""
        }))
        .unwrap();
        normalize_movie_project(&mut project);
        let decision = |id: &str, order: u32| ClipEdit {
            id: id.into(),
            clip_id: "clip-001".into(),
            enabled: true,
            order,
            trim_start: 0.0,
            trim_end: 0.0,
            audio_gain: 1.0,
            source_version_id: String::new(),
            speed: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            audio_fade_in: 0.0,
            audio_fade_out: 0.0,
            label: String::new(),
            notes: String::new(),
        };
        let mut edit = MovieEdit {
            clips: vec![decision("second", 8), decision("first", 2)],
            export_title: "Timeline".into(),
            export_preset: "archive".into(),
            normalize_audio: true,
            target_lufs: -16.0,
            markers: vec![TimelineMarker {
                id: "opening-note".into(),
                time_seconds: 1.0,
                label: "Check the opening beat".into(),
                kind: "todo".into(),
                completed: false,
            }],
        };
        validate_movie_edit(&project, &mut edit).unwrap();
        assert_eq!(edit.clips[0].id, "first");
        assert_eq!(edit.clips[1].order, 1);

        edit.clips[0].fade_in = 3.0;
        edit.clips[0].fade_out = 3.0;
        assert!(validate_movie_edit(&project, &mut edit).is_err());
    }

    #[test]
    fn retiming_filters_and_export_names_are_bounded() {
        assert_eq!(atempo_filters(1.0), Vec::<f32>::new());
        assert_eq!(atempo_filters(0.25), vec![0.5, 0.5]);
        assert_eq!(atempo_filters(4.0), vec![2.0, 2.0]);
        assert_eq!(
            safe_export_stem("  My Offline Cut: 01  "),
            "my-offline-cut-01"
        );
        assert_eq!(safe_export_stem("🎬"), "kestrel-movie");
    }

    #[tokio::test]
    async fn project_mutations_serialize_only_matching_project_ids() {
        let root = tempfile::tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let first_id = uuid::Uuid::new_v4().to_string();
        let second_id = uuid::Uuid::new_v4().to_string();
        let first = studio.project_lock(&first_id).unwrap();
        let same_project = studio.project_lock(&first_id).unwrap();
        let other_project = studio.project_lock(&second_id).unwrap();
        assert!(Arc::ptr_eq(&first, &same_project));
        assert!(!Arc::ptr_eq(&first, &other_project));

        let _guard = first.lock().await;
        assert!(same_project.try_lock().is_err());
        assert!(other_project.try_lock().is_ok());
    }

    #[test]
    fn paused_productions_require_explicit_approval_before_rendering() {
        let root = tempfile::tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let mut project = studio
            .create(
                StartMovieRequest {
                    prompt: "A guarded producer review".into(),
                    settings: MovieSettings::default(),
                    references: vec![],
                    pause_after_plan: true,
                },
                false,
            )
            .unwrap();
        project.status = "awaiting-review".into();
        assert!(ensure_producer_render_approval(&project).is_err());

        project.status = "complete".into();
        assert!(ensure_producer_render_approval(&project).is_err());
        project.producer_approved_at = Utc::now().to_rfc3339();
        assert!(ensure_producer_render_approval(&project).is_ok());

        project.producer_review_required = false;
        project.producer_approved_at.clear();
        assert!(ensure_producer_render_approval(&project).is_ok());
    }

    #[tokio::test]
    async fn producer_can_checkpoint_and_approve_a_movie_without_starting_bonsai() {
        let root = tempfile::tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let project = studio
            .create_manual(
                StartMovieRequest {
                    prompt: String::new(),
                    settings: MovieSettings::default(),
                    references: vec![],
                    pause_after_plan: true,
                },
                false,
            )
            .unwrap();
        assert_eq!(project.status, "awaiting-review");
        assert_eq!(project.phase, "awaiting-producer");
        assert!(project.plan.as_ref().unwrap().clips.is_empty());
        assert!(project.detail.contains("Bonsai has not been started"));

        let blank_checkpoint = studio
            .save_producer_plan(
                &project.id,
                MoviePlan {
                    title: "Producer's private cut".into(),
                    logline: String::new(),
                    audience: String::new(),
                    creative_direction: String::new(),
                    continuity_bible: vec![],
                    source_credits: vec![],
                    quality_review: MovieQualityReview::default(),
                    clips: vec![],
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(blank_checkpoint.title, "Producer's private cut");
        assert!(blank_checkpoint.clips.is_empty());

        let detailed_direction = format!(
            "At 0:00-0:02, a locked wide camera frames the empty circus ring as soft morning light reveals silver mist and red canvas texture. At 0:02-0:05, the camera makes a slow controlled push toward a football resting on sawdust while warm practical lighting blooms around the ring and gentle crowd ambience, leather creaks, distant rigging sound, and a restrained orchestral score settle through the exact final frame. {}",
            "Maintain grounded live-action scale, stable geometry, natural motion, coherent shadows, precise screen direction, restrained film grain, clean spatial depth, readable foreground separation, consistent color, and an unbroken final hold. ".repeat(5)
        );
        assert!((MIN_H3_PROMPT_WORDS..=MAX_H3_PROMPT_WORDS)
            .contains(&detailed_direction.split_whitespace().count()));
        let ready = studio
            .save_producer_plan(
                &project.id,
                MoviePlan {
                    title: "Producer's private cut".into(),
                    logline: "A quiet circus reveal begins with a lone football.".into(),
                    audience: "Film buyers".into(),
                    creative_direction: "A tactile, restrained live-action selling film.".into(),
                    continuity_bible: vec![
                        "Morning mist remains silver beneath the red circus canvas.".into(),
                    ],
                    source_credits: vec![],
                    quality_review: MovieQualityReview::default(),
                    clips: vec![PlannedClip {
                        id: "producer-draft-id".into(),
                        title: "The empty ring".into(),
                        purpose: "Establish the circus world before the player arrives.".into(),
                        duration_seconds: 5.0,
                        prompt: detailed_direction,
                        continuity_in: "Independent opening on an empty misty ring.".into(),
                        continuity_out: "The football rests centered on the sawdust.".into(),
                        transition: "fade to black".into(),
                        use_previous_frame: false,
                        source_refs: vec![],
                        reference_ids: vec![],
                    }],
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(ready.plan.as_ref().unwrap().clips[0].id, "clip-001");
        let approved = studio
            .approve_producer_plan(&project.id, None)
            .await
            .unwrap();
        assert_eq!(approved.phase, "producer-approved");
        assert!(!approved.producer_approved_at.is_empty());
        assert_eq!(approved.plan.as_ref().unwrap().quality_review.score, 100);
        assert!(approved
            .plan
            .as_ref()
            .unwrap()
            .quality_review
            .verdict
            .contains("without an agent review"));
    }

    #[test]
    fn external_chat_plan_exchange_is_versioned_bounded_and_reference_safe() {
        let root = tempfile::tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let story = "A football player discovers a misty construction circus.";
        let mut project = studio
            .create_manual(
                StartMovieRequest {
                    prompt: story.into(),
                    settings: MovieSettings::default(),
                    references: vec![],
                    pause_after_plan: true,
                },
                false,
            )
            .unwrap();
        project.references.push(MovieReference {
            asset_id: "private-asset-id".into(),
            tag: "<Picture 1>".into(),
            audio_tag: String::new(),
            name: "player.png".into(),
            kind: "image".into(),
            mime_type: "image/png".into(),
            bytes: 10,
            duration_seconds: 0.0,
            width: 768,
            height: 1344,
            has_audio: false,
            path: "D:\\private\\player.png".into(),
            description: "Identity and wardrobe reference for the football player.".into(),
            use_embedded_audio: false,
            embedded_audio_description: String::new(),
            generation: None,
        });
        studio.save(&project).unwrap();

        let brief = studio.movie_plan_exchange_prompt(&project.id).unwrap();
        assert!(brief.contains(PLAN_EXCHANGE_FORMAT));
        assert!(brief.contains(story));
        assert!(brief.contains("reference-1"));
        assert!(!brief.contains("private-asset-id"));
        assert!(!brief.contains("D:\\private"));

        let response = format!(
            "Here is the result:\n```json\n{}\n```",
            json!({
                "format": PLAN_EXCHANGE_FORMAT,
                "version": PLAN_EXCHANGE_VERSION,
                "plan": {
                    "title": "The Circus Player",
                    "logline": "A player enters an impossible circus.",
                    "audience": "Film buyers",
                    "creativeDirection": "Misty live-action wonder.",
                    "continuityBible": ["The player retains the reference identity and red kit."],
                    "sourceCredits": [],
                    "clips": [{
                        "id": "external-1",
                        "title": "Arrival",
                        "purpose": "Introduce the player and circus.",
                        "durationSeconds": 5,
                        "prompt": "",
                        "continuityIn": "Independent morning entrance.",
                        "continuityOut": "Player stands beneath the circus arch.",
                        "transition": "hard cut",
                        "usePreviousFrame": false,
                        "sourceRefs": [],
                        "referenceIds": ["reference-1"]
                    }]
                }
            })
        );
        let plan = studio
            .parse_movie_plan_exchange(&project.id, &response)
            .unwrap();
        assert_eq!(plan.clips[0].id, "clip-001");
        assert_eq!(plan.clips[0].reference_ids, ["private-asset-id"]);

        let unknown = response.replace("reference-1", "reference-99");
        let error = studio
            .parse_movie_plan_exchange(&project.id, &unknown)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown reference handle 'reference-99'"));
    }

    #[tokio::test]
    #[ignore = "requires the installed local Bonsai model and several minutes"]
    async fn live_bonsai_ordinary_chat_returns_a_lint_clean_plan_exchange() {
        let root = tempfile::tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let project = studio
            .create_manual(
                StartMovieRequest {
                    prompt: "Create a 15-second surreal sales teaser. At misty dawn, an exhausted football player discovers a construction-site circus where cranes perform like trapeze artists. Keep it visually coherent, use no dialogue or narration, and end on the player smiling beneath a swinging work light.".into(),
                    settings: MovieSettings {
                        max_clips: 3,
                        clip_seconds: 8.0,
                        ..MovieSettings::default()
                    },
                    references: Vec::new(),
                    pause_after_plan: true,
                },
                false,
            )
            .unwrap();
        let exchange_prompt = studio.movie_plan_exchange_prompt(&project.id).unwrap();
        let runtime = Arc::new(RuntimeManager::new());
        let research = ResearchSettings::default();
        studio.release_comfy_memory().await;

        let result: Result<MoviePlan, String> = async {
            let lease = runtime
                .lease_research(&research)
                .await
                .map_err(|error| error.to_string())?;
            let body = json!({
                "model": lease.connection.model_id,
                "messages": [
                    {"role":"system","content":"You are the producer's local ordinary chat model. Follow the user's requested output format exactly. You have no tools and must return only the requested answer."},
                    {"role":"user","content":exchange_prompt}
                ],
                "temperature": 0.2,
                "top_p": 0.9,
                "top_k": 20,
                "max_tokens": 32_768,
                "stream": false
            });
            let client = Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3_600))
                .build()
                .map_err(|error| error.to_string())?;
            let response = authorized(
                client.post(format!("{}/chat/completions", lease.connection.endpoint)),
                &lease.connection,
            )
            .json(&body)
            .send()
            .await
            .map_err(|error| error.to_string())?;
            let status = response.status();
            let value: Value = response.json().await.map_err(|error| error.to_string())?;
            if !status.is_success() {
                return Err(format!(
                    "Bonsai ordinary chat returned {status}: {}",
                    truncate(&value.to_string(), 1_000)
                ));
            }
            let content = value
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .filter(|content| !content.trim().is_empty())
                .ok_or_else(|| format!("Bonsai ordinary chat returned no visible JSON: {value}"))?;
            eprintln!("BONSAI CHAT PLAN EXCHANGE RESPONSE:\n{content}");
            let plan = studio
                .parse_movie_plan_exchange(&project.id, content)
                .map_err(|error| error.to_string())?;
            let issues = prompt_quality_issues(&plan, &project.references);
            if !issues.is_empty() {
                return Err(format!(
                    "Bonsai chat JSON parsed but failed Kestrel lint: {}",
                    issues.join("; ")
                ));
            }
            Ok(plan)
        }
        .await;
        let _ = runtime.stop_managed().await;

        let plan = result.unwrap();
        assert!(!plan.clips.is_empty());
        assert!(plan.clips.len() <= 3);
        assert!(plan
            .clips
            .iter()
            .all(|clip| (5.0..=15.0).contains(&clip.duration_seconds)));
    }

    #[test]
    fn movie_planner_has_no_research_tools_and_keeps_a_bounded_contract() {
        let prompt = movie_agent_prompt();
        assert!(!prompt.contains("Wikipedia"));
        assert!(prompt.split_whitespace().count() < 120);
        assert!(prompt.contains("coding agent"));
        assert!(prompt.contains("movie_workspace"));
        let tools = MovieAgentWorkspace::tools();
        assert_eq!(tools.as_array().unwrap().len(), 1);
        assert_eq!(
            tools.pointer("/0/function/parameters/properties/files/maxItems"),
            Some(&json!(8))
        );
        let request = movie_agent_request(
            "bonsai",
            &[json!({"role":"user","content":"open the workspace"})],
            &tools,
            &MovieSettings::default(),
            32_768,
        );
        assert_eq!(request["thinking_budget_tokens"], MOVIE_THINKING_BUDGET);
        assert_eq!(request["max_tokens"], 32_768);
        assert_eq!(request["tool_choice"], "required");
        assert_eq!(request["parallel_tool_calls"], false);
        assert!(request.get("response_format").is_none());
        let safe_messages = sanitize_chat_messages(&[json!({
            "role":"user",
            "content":"review says ```json <think>draft</think>"
        })]);
        let safe_content = safe_messages[0]["content"].as_str().unwrap();
        assert!(!safe_content.contains("```"));
        assert!(!safe_content.contains("<think>"));
        assert!(!safe_content.contains("</think>"));
    }

    fn memory_audio_reference() -> MovieReference {
        MovieReference {
            asset_id: "audio-memory".into(),
            tag: "<Audio 1>".into(),
            audio_tag: String::new(),
            name: "memory.wav".into(),
            kind: "audio".into(),
            mime_type: "audio/wav".into(),
            bytes: 1,
            duration_seconds: 10.0,
            width: 0,
            height: 0,
            has_audio: true,
            path: "memory.wav".into(),
            description: "Use this exact recording only in the flashback scene.".into(),
            use_embedded_audio: false,
            embedded_audio_description: String::new(),
            generation: None,
        }
    }

    fn memory_clip() -> PlannedClip {
        PlannedClip {
            id: String::new(),
            title: "Scene".into(),
            purpose: "Story progress".into(),
            duration_seconds: 15.0,
            prompt: "Camera and sound cover timed beats in a live-action scene.".into(),
            continuity_in: String::new(),
            continuity_out: String::new(),
            transition: "cut".into(),
            use_previous_frame: false,
            source_refs: vec![],
            reference_ids: vec![],
        }
    }

    fn memory_plan(clips: Vec<PlannedClip>) -> MoviePlan {
        MoviePlan {
            title: "Test".into(),
            logline: "A memory shapes a present-day encounter.".into(),
            audience: "Film buyers".into(),
            creative_direction: "Live-action memory film".into(),
            continuity_bible: vec!["The same world continues across the cut.".into()],
            source_credits: vec![],
            quality_review: MovieQualityReview::default(),
            clips,
        }
    }

    #[test]
    fn runtime_gate_enforces_requested_duration() {
        let plan = memory_plan(vec![memory_clip()]);
        let issues =
            producer_intent_issues("Make the finished film about 2 to 3 minutes.", &plan, &[])
                .join(" ");
        assert!(issues.contains("120-180 seconds"));
    }

    #[test]
    fn flashback_audio_gate_rejects_present_day_placement() {
        let reference = memory_audio_reference();
        let mut clip = memory_clip();
        clip.reference_ids = vec![reference.asset_id.clone()];
        clip.title = "Present-day encounter".into();
        clip.purpose = "The vlogger watches the animal now".into();
        let issues = producer_intent_issues(
            "Make the finished film about 2 to 3 minutes.",
            &memory_plan(vec![clip]),
            std::slice::from_ref(&reference),
        )
        .join(" ");
        assert!(issues.contains("not explicitly a flashback"));
    }

    #[test]
    fn timecode_parser_accepts_explicit_ranges_and_rejects_prose_numbers() {
        assert_eq!(
            maximum_timecode_seconds("[00:00-00:05] action [00:05-00:20] hold"),
            Some(20.0)
        );
        assert_eq!(
            maximum_timecode_seconds("[0s-1.5s] action [1.5s-5s] hold"),
            Some(5.0)
        );
        assert_eq!(
            maximum_timecode_seconds("A courier (20s, focused) under a key light at 10:30."),
            None
        );
        assert_eq!(
            maximum_timecode_seconds(
                "A mid-30s African man waits. 0:00-0:03 wide; 0:09-0:12 hold."
            ),
            Some(12.0)
        );
        assert_eq!(
            maximum_timecode_seconds("0:00-0:08: action. 0:08-0:10: final hold."),
            Some(10.0)
        );
        assert_eq!(
            maximum_timecode_seconds("[0s-20s] overlong action"),
            Some(20.0)
        );
    }

    #[test]
    fn timed_structure_requires_real_timing_or_named_structure() {
        assert!(has_timed_structure(
            "0:00-0:03 wide; 0:03-0:06 medium; 0:06-0:12 hold"
        ));
        assert!(has_timed_structure("shot 1 establishes the forest"));
        assert!(!has_timed_structure(
            "At 0 to no cost, the courier wears 20s styling"
        ));
    }

    fn valid_long_form_memory_plan(reference: &MovieReference) -> MoviePlan {
        let mut plan = memory_plan((0..8).map(|_| memory_clip()).collect());
        plan.clips[2].title = "Childhood flashback".into();
        plan.clips[2].purpose = "A memory from the past".into();
        plan.clips[2].reference_ids = vec![reference.asset_id.clone()];
        plan
    }

    #[test]
    fn long_form_gate_requires_exact_previous_frame_continuation() {
        let reference = memory_audio_reference();
        let plan = valid_long_form_memory_plan(&reference);
        let issues = producer_intent_issues(
            "Make the finished film about 2-3 minutes.",
            &plan,
            std::slice::from_ref(&reference),
        )
        .join(" ");
        assert!(issues.contains("no exact previous-frame continuation"));
    }

    #[test]
    fn continuity_gate_requires_shared_handoff_anchors() {
        let reference = memory_audio_reference();
        let mut plan = valid_long_form_memory_plan(&reference);
        plan.clips[1].use_previous_frame = true;
        plan.clips[0].continuity_out =
            "The camera holds on the same subject and handoff pose.".into();
        plan.clips[1].continuity_in =
            "Continue the same subject, pose, light, and screen direction.".into();
        assert!(producer_intent_issues(
            "Make the finished film about 2-3 minutes.",
            &plan,
            &[reference],
        )
        .is_empty());
        assert!(continuity_handoff_matches(
            &plan.clips[0].continuity_out,
            &plan.clips[1].continuity_in
        ));
        plan.clips[1].continuity_in =
            "Kwame smiles in a present-day close-up after the memory ends.".into();
        assert!(prompt_quality_issues(&plan, &[])
            .join(" ")
            .contains("do not share at least two concrete handoff anchors"));
    }

    #[test]
    fn transition_gate_rejects_early_terminal_and_nonterminal_ending() {
        let mut plan = memory_plan((0..8).map(|_| memory_clip()).collect());
        plan.clips[0].transition = "end of film".into();
        let final_clip = plan.clips.len() - 1;
        plan.clips[final_clip].transition = "cut to next scene".into();
        let issues = prompt_quality_issues(&plan, &[]).join(" ");
        assert!(issues.contains("declares a terminal"));
        assert!(issues.contains("but no later scene exists"));
    }

    fn identity_reference() -> MovieReference {
        MovieReference {
            asset_id: "vlogger-picture".into(),
            tag: "<Picture 1>".into(),
            audio_tag: String::new(),
            name: "vlogger.png".into(),
            kind: "image".into(),
            mime_type: "image/png".into(),
            bytes: 1,
            duration_seconds: 0.0,
            width: 1024,
            height: 1024,
            has_audio: false,
            path: "vlogger.png".into(),
            description: "This is the vlogger's identity reference. Whenever he appears, preserve his face and wardrobe.".into(),
            use_embedded_audio: false,
            embedded_audio_description: String::new(),
            generation: None,
        }
    }

    fn identity_clip(title: &str, purpose: &str) -> PlannedClip {
        PlannedClip {
            id: String::new(),
            title: title.into(),
            purpose: purpose.into(),
            duration_seconds: 10.0,
            prompt: "A detailed timed live-action scene with camera, lighting, and sound.".into(),
            continuity_in: "Continue the same subject and pose.".into(),
            continuity_out: "Hold the subject in the same pose.".into(),
            transition: "cut".into(),
            use_previous_frame: false,
            source_refs: vec![],
            reference_ids: vec![],
        }
    }

    fn identity_plan() -> MoviePlan {
        MoviePlan {
            title: "Test".into(),
            logline: "A vlogger explores a forest.".into(),
            audience: "Film buyers".into(),
            creative_direction: "Live-action reference test".into(),
            continuity_bible: vec![
                "Vlogger: Kwame, with the same face and wardrobe throughout.".into(),
            ],
            source_credits: vec![],
            quality_review: MovieQualityReview::default(),
            clips: vec![
                identity_clip("Vlogger arrives", "Show the vlogger entering the forest"),
                identity_clip("Vlogger reacts", "Close-up of the vlogger's face"),
                identity_clip(
                    "Animal insert",
                    "Vlogger POV; the vlogger remains off-screen",
                ),
            ],
        }
    }

    #[test]
    fn identity_subject_extraction_finds_reference_role() {
        let reference = identity_reference();
        assert_eq!(
            identity_reference_subject(&reference.description).as_deref(),
            Some("vlogger")
        );
    }

    #[test]
    fn identity_gate_understands_producer_facing_reference_language() {
        let mut reference = identity_reference();
        reference.asset_id = "elias-picture".into();
        reference.description = "This is the immutable identity and wardrobe reference for Elias Vance, the adult football player. Whenever Elias is visibly present, preserve the same face, short dark hair, athletic build, navy number-one practice jersey, charcoal shorts, white socks, black football boots, and taped left forearm.".into();
        assert!(is_identity_reference_description(&reference.description));
        assert_eq!(
            identity_reference_subject(&reference.description).as_deref(),
            Some("elias vance")
        );
        let mut plan = identity_plan();
        plan.continuity_bible = vec![
            "Elias Vance is the football player in the navy number-one practice jersey.".into(),
        ];
        plan.clips = vec![
            identity_clip("Elias enters", "Elias Vance looks down at his boots"),
            identity_clip("Grand reveal", "Elias Vance watches the construction site"),
        ];
        plan.clips[0].reference_ids = vec![reference.asset_id.clone()];

        let issues = producer_intent_issues(
            "Make a two-minute film about Elias Vance.",
            &plan,
            std::slice::from_ref(&reference),
        )
        .join(" ");
        assert!(issues.contains("scene(s) 2"));
    }

    #[test]
    fn identity_alias_parser_supports_all_continuity_fact_forms() {
        let mut plan = identity_plan();
        assert_eq!(
            identity_subject_aliases(&plan, "vlogger"),
            vec!["vlogger", "kwame"]
        );
        plan.continuity_bible = vec![
            "description: The recurring presenter; name: Amina; subject: Vlogger; wardrobe: field jacket"
                .into(),
        ];
        assert_eq!(
            identity_subject_aliases(&plan, "vlogger"),
            vec!["vlogger", "amina"]
        );
        for (fact, expected) in [
            (
                "Tariq, a 34-year-old Somali vlogger with short natural hair, appears throughout.",
                "tariq",
            ),
            (
                "Vlogger Kofi: Black male, short natural hair, olive field overshirt.",
                "kofi",
            ),
            (
                "Vlogger Kwame Osei (29) is a Black male with short natural hair and a trimmed beard.",
                "kwame",
            ),
        ] {
            plan.continuity_bible = vec![fact.into()];
            assert_eq!(
                identity_subject_aliases(&plan, "vlogger"),
                vec!["vlogger", expected]
            );
        }
        plan.continuity_bible = vec!["Vlogger equipment: compact camera and stabilizer.".into()];
        assert_eq!(identity_subject_aliases(&plan, "vlogger"), vec!["vlogger"]);
    }

    #[test]
    fn identity_visibility_gate_handles_aliases_and_subject_free_scenes() {
        let plan = identity_plan();
        let aliases = identity_subject_aliases(&plan, "vlogger");
        assert!(clip_visibly_names_subject(
            &identity_clip("Kwame reacts", "Close-up of Kwame in the present day"),
            &aliases
        ));
        assert!(clip_visibly_names_subject(
            &identity_clip("Childhood Kwame", "Young Kwame remembers the jungle"),
            &aliases
        ));
        assert!(clip_visibly_names_subject(
            &identity_clip(
                "Vlogger raises camera",
                "The vlogger is visible before a POV transition"
            ),
            &aliases
        ));
        assert!(!clip_visibly_names_subject(
            &identity_clip(
                "Earth view subjective",
                "Subjective POV: what the vlogger sees from the moon"
            ),
            &aliases
        ));
        let absent_clip = identity_clip(
            "Jungle atmosphere",
            "Environment-only coverage with no vlogger",
        );
        assert!(!clip_visibly_names_subject(&absent_clip, &aliases));
        assert!(clip_is_independent_subject_free(&absent_clip, &aliases));
        let named_absent = identity_clip(
            "Animal insert",
            "Kwame remains off-screen while the animal crosses frame",
        );
        assert!(clip_is_independent_subject_free(&named_absent, &aliases));
    }

    #[test]
    fn identity_gate_requires_reference_on_independent_appearances() {
        let reference = identity_reference();
        let mut plan = identity_plan();
        plan.clips[0].reference_ids = vec![reference.asset_id.clone()];
        let issues =
            producer_intent_issues("Make a short film", &plan, std::slice::from_ref(&reference))
                .join(" ");
        assert!(issues.contains("scene(s) 2"));
        assert!(!issues.contains("scene(s) 3"));
        plan.clips[1].use_previous_frame = true;
        assert!(producer_intent_issues(
            "Make a short film",
            &plan,
            std::slice::from_ref(&reference)
        )
        .is_empty());
    }

    #[test]
    fn identity_age_gate_rejects_unsupported_transformations_and_adult_traits() {
        let reference = identity_reference();
        let mut plan = identity_plan();
        plan.clips = vec![identity_clip(
            "Childhood Kwame",
            "Young Kwame remembers his first jungle encounter",
        )];
        assert!(producer_intent_issues(
            "Make a short film",
            &plan,
            std::slice::from_ref(&reference)
        )
        .join(" ")
        .contains("scene(s) 1"));
        plan.clips[0].reference_ids = vec![reference.asset_id.clone()];
        assert!(producer_intent_issues(
            "Make a short film",
            &plan,
            std::slice::from_ref(&reference)
        )
        .join(" ")
        .contains("cannot guarantee this age transformation"));
        plan.clips[0].prompt =
            "A 10-year-old child with the same trimmed beard walks through the jungle.".into();
        assert!(
            prompt_quality_issues(&plan, std::slice::from_ref(&reference))
                .join(" ")
                .contains("child while retaining adult facial hair")
        );
        let mut child_reference = reference;
        child_reference.description = "This is the vlogger's identity reference for the child version. Whenever he appears, preserve his face and wardrobe.".into();
        plan.clips[0].prompt = "A child walks through the jungle.".into();
        assert!(producer_intent_issues(
            "Make a short film",
            &plan,
            std::slice::from_ref(&child_reference)
        )
        .is_empty());
    }

    #[test]
    fn identity_gate_rejects_reference_on_subject_free_scene() {
        let reference = identity_reference();
        let mut plan = identity_plan();
        plan.clips[0].reference_ids = vec![reference.asset_id.clone()];
        plan.clips[2].reference_ids = vec![reference.asset_id.clone()];
        let issues = producer_intent_issues("Make a short film", &plan, &[reference]).join(" ");
        assert!(issues.contains("scene(s) 3 do not visibly feature"));
    }

    #[test]
    fn spoken_audio_reference_requires_voice_assignment_and_exact_dialogue() {
        let reference = MovieReference {
            asset_id: "memory-voice".into(),
            tag: "<Audio 1>".into(),
            audio_tag: String::new(),
            name: "memory.wav".into(),
            kind: "audio".into(),
            mime_type: "audio/wav".into(),
            bytes: 1,
            duration_seconds: 8.0,
            width: 0,
            height: 0,
            has_audio: true,
            path: "memory.wav".into(),
            description: "A spoken voice reference for the flashback narrator.".into(),
            use_embedded_audio: false,
            embedded_audio_description: String::new(),
            generation: None,
        };
        let mut plan = MoviePlan {
            title: "Memory".into(),
            logline: "A remembered warning returns.".into(),
            audience: "Film buyers".into(),
            creative_direction: "Live action".into(),
            continuity_bible: vec!["The narrator remains consistent.".into()],
            source_credits: vec![],
            quality_review: MovieQualityReview::default(),
            clips: vec![PlannedClip {
                id: String::new(),
                title: "Flashback".into(),
                purpose: "The narrator remembers a warning.".into(),
                duration_seconds: 8.0,
                prompt: "A recorded spoken memory begins over the jungle ambience.".into(),
                continuity_in: "Dissolve into the memory.".into(),
                continuity_out: "Hold on the remembered clearing.".into(),
                transition: "Dissolve".into(),
                use_previous_frame: false,
                source_refs: vec![],
                reference_ids: vec!["memory-voice".into()],
            }],
        };
        assert!(
            producer_intent_issues("Make a film", &plan, std::slice::from_ref(&reference))
                .join(" ")
                .contains("state exact dialogue in quotation marks")
        );

        plan.clips[0].prompt = "Use the supplied voice reference as the narrator's vocal timbre. The narrator says exactly: \"Listen first, then look.\" Jungle ambience remains underneath.".into();
        assert!(producer_intent_issues("Make a film", &plan, &[reference]).is_empty());
    }

    #[test]
    fn long_form_identity_story_requires_real_subject_free_coverage() {
        let reference = MovieReference {
            asset_id: "lead-picture".into(),
            tag: "<Picture 1>".into(),
            audio_tag: String::new(),
            name: "lead.png".into(),
            kind: "image".into(),
            mime_type: "image/png".into(),
            bytes: 1,
            duration_seconds: 0.0,
            width: 1,
            height: 1,
            has_audio: false,
            path: "lead.png".into(),
            description: "The vlogger's identity reference.".into(),
            use_embedded_audio: false,
            embedded_audio_description: String::new(),
            generation: None,
        };
        let clip = PlannedClip {
            id: String::new(),
            title: "Vlogger journey".into(),
            purpose: "The vlogger walks through the forest.".into(),
            duration_seconds: 10.0,
            prompt: "The vlogger remains visible during this forest scene.".into(),
            continuity_in: "Independent cut.".into(),
            continuity_out: "The vlogger keeps walking.".into(),
            transition: "Cut".into(),
            use_previous_frame: false,
            source_refs: vec![],
            reference_ids: vec!["lead-picture".into()],
        };
        let mut plan = MoviePlan {
            title: "Journey".into(),
            logline: "A vlogger crosses a forest.".into(),
            audience: "Film buyers".into(),
            creative_direction: "Live action".into(),
            continuity_bible: vec!["Vlogger: Kwame, wearing the same field jacket.".into()],
            source_credits: vec![],
            quality_review: MovieQualityReview::default(),
            clips: vec![clip; 12],
        };
        let issues = producer_intent_issues(
            "Make a 2 to 3 minute film",
            &plan,
            std::slice::from_ref(&reference),
        )
        .join(" ");
        assert!(issues.contains("no independently cut subject-free coverage"));

        let insert = plan.clips.last_mut().unwrap();
        insert.title = "Animal-only insert".into();
        insert.purpose =
            "Environment-only wildlife coverage; the vlogger remains off-screen.".into();
        insert.prompt = "A forest animal crosses the clearing without the vlogger.".into();
        insert.reference_ids.clear();
        assert!(
            !producer_intent_issues("Make a 2 to 3 minute film", &plan, &[reference])
                .join(" ")
                .contains("no independently cut subject-free coverage")
        );
    }

    #[test]
    fn native_gate_rejects_duplicate_organized_scenes() {
        let clip = PlannedClip {
            id: String::new(),
            title: "Back to the Present".into(),
            purpose: "Return to the vlogger's present-day reaction.".into(),
            duration_seconds: 10.0,
            prompt: "A placeholder prompt for duplicate detection.".into(),
            continuity_in: "Independent cut".into(),
            continuity_out: "The vlogger holds still".into(),
            transition: "Hard cut".into(),
            use_previous_frame: false,
            source_refs: vec![],
            reference_ids: vec![],
        };
        let plan = MoviePlan {
            title: "Test".into(),
            logline: "A memory returns to the present.".into(),
            audience: "Film buyers".into(),
            creative_direction: "Live action".into(),
            continuity_bible: vec!["The vlogger remains visually consistent.".into()],
            source_credits: vec![],
            quality_review: MovieQualityReview::default(),
            clips: vec![clip.clone(), clip],
        };
        assert!(prompt_quality_issues(&plan, &[])
            .join(" ")
            .contains("Clips 1 and 2 duplicate"));
    }

    #[test]
    fn native_gate_rejects_near_duplicate_runtime_padding() {
        let repeated = "At twilight the traveler walks toward the forest edge, pauses beneath the indigo sky, looks back with quiet gratitude, breathes slowly, and resumes the journey while distant birds and wind fade through the trees.";
        let clip = |title: &str, purpose: &str| PlannedClip {
            id: String::new(),
            title: title.into(),
            purpose: purpose.into(),
            duration_seconds: 8.0,
            prompt: repeated.into(),
            continuity_in: "The walk continues.".into(),
            continuity_out: "The traveler keeps walking.".into(),
            transition: "Cut".into(),
            use_previous_frame: false,
            source_refs: vec![],
            reference_ids: vec![],
        };
        let plan = MoviePlan {
            title: "Test".into(),
            logline: "A traveler leaves the forest.".into(),
            audience: "Film buyers".into(),
            creative_direction: "Live action".into(),
            continuity_bible: vec!["Twilight deepens consistently.".into()],
            source_credits: vec![],
            quality_review: MovieQualityReview::default(),
            clips: vec![
                clip("Journey Home", "The traveler approaches the forest edge."),
                clip("Last Look", "The traveler reflects before leaving."),
            ],
        };
        assert!(prompt_quality_issues(&plan, &[])
            .join(" ")
            .contains("near-duplicate renderer direction"));
    }

    #[test]
    fn offline_plan_rejects_fabricated_source_credits() {
        let mut plan = MoviePlan {
            title: "Forest".into(),
            logline: "A traveler enters a forest.".into(),
            audience: "Film buyers".into(),
            creative_direction: "Live action".into(),
            continuity_bible: vec!["The forest remains consistent.".into()],
            source_credits: vec!["Inspired by Example Institute research".into()],
            quality_review: MovieQualityReview::default(),
            clips: vec![],
        };
        assert!(producer_intent_issues("Make a forest film", &plan, &[])
            .join(" ")
            .contains("invents source credits"));
        plan.source_credits.clear();
        assert!(producer_intent_issues("Make a forest film", &plan, &[]).is_empty());
    }

    #[test]
    fn native_prompt_gate_requires_official_example_level_detail() {
        let detailed_prompt = "Camera tracks the subject through each timed beat with live-action film lighting and textured production design while the audio carries ambience, sound effects, and score. ".repeat(10);
        let mut plan = MoviePlan {
            title: "Test".into(),
            logline: "Test".into(),
            audience: "Film buyers".into(),
            creative_direction: "Test".into(),
            continuity_bible: vec![
                "The subject keeps the same wardrobe, direction, and lighting across the cut."
                    .into(),
            ],
            source_credits: vec![],
            quality_review: MovieQualityReview::default(),
            clips: vec![PlannedClip {
                id: String::new(),
                title: "Shot".into(),
                purpose: "Test".into(),
                duration_seconds: 5.0,
                prompt: detailed_prompt,
                continuity_in: String::new(),
                continuity_out: String::new(),
                transition: "cut".into(),
                use_previous_frame: false,
                source_refs: vec![],
                reference_ids: vec![],
            }],
        };
        assert!(prompt_quality_issues(&plan, &[]).is_empty());
        let mut conflict = plan.clone();
        conflict.clips[0].use_previous_frame = true;
        conflict.clips[0].reference_ids = vec!["actor-reference".into()];
        let reference = MovieReference {
            asset_id: "actor-reference".into(),
            tag: "<Picture 1>".into(),
            audio_tag: String::new(),
            name: "actor.png".into(),
            kind: "image".into(),
            mime_type: "image/png".into(),
            bytes: 1,
            duration_seconds: 0.0,
            width: 1,
            height: 1,
            has_audio: false,
            path: "actor.png".into(),
            description: "The lead actor's identity".into(),
            use_embedded_audio: false,
            embedded_audio_description: String::new(),
            generation: None,
        };
        let conflict_issues = prompt_quality_issues(&conflict, &[reference]).join(" ");
        assert!(conflict_issues.contains("mutually exclusive fl2va and ref2va"));
        assert!(conflict_issues.contains("Minimally reorganize this boundary"));
        let mut false_continuation = plan.clone();
        false_continuation.clips[0].purpose =
            "A continuous shot from the previous scene follows the actor's reaction.".into();
        assert!(prompt_quality_issues(&false_continuation, &[])
            .join(" ")
            .contains("usePreviousFrame is false"));
        let mut unquoted_speech = plan.clone();
        unquoted_speech.clips[0].purpose = "The narrator speaks about the discovery.".into();
        assert!(prompt_quality_issues(&unquoted_speech, &[])
            .join(" ")
            .contains("speech or narration without exact quoted words"));
        unquoted_speech.clips[0]
            .prompt
            .push_str(" The narrator says exactly: \"Listen first, then look.\"");
        assert!(!prompt_quality_issues(&unquoted_speech, &[])
            .join(" ")
            .contains("speech or narration without exact quoted words"));
        let mut ambiguous_murmur = plan.clone();
        ambiguous_murmur.clips[0]
            .prompt
            .push_str(" No dialogue. A low murmur of awe is barely audible.");
        assert!(prompt_quality_issues(&ambiguous_murmur, &[])
            .join(" ")
            .contains("speech or narration without exact quoted words"));
        ambiguous_murmur.clips[0]
            .prompt
            .push_str(" The murmur is explicitly wordless and nonverbal.");
        assert!(!prompt_quality_issues(&ambiguous_murmur, &[])
            .join(" ")
            .contains("speech or narration without exact quoted words"));
        plan.clips[0].prompt = "A nice cinematic image with sound.".into();
        let issues = prompt_quality_issues(&plan, &[]).join(" ");
        assert!(issues.contains("120-450 words"));
        assert!(issues.contains("camera direction"));
        assert!(issues.contains("timed shot or beat structure"));
    }

    #[test]
    fn speech_gate_understands_negation_and_environmental_sound() {
        assert!(!directs_unquoted_speech(
            "No dialogue or narration. Environmental audio only: wind whispers through the leaves."
        ));
        assert!(!directs_unquoted_speech(
            "No dialogue or narration. The score fades into the soft, quiet whisper of the morning wind."
        ));
        assert!(!directs_unquoted_speech(
            "Silent/no-dialogue performance; environmental sound only."
        ));
        assert!(!directs_unquoted_speech(
            "No speech or audio other than environment."
        ));
        assert!(!directs_unquoted_speech(
            "The cat's expression is one of complete, unspoken awe. No dialogue or narration."
        ));
        assert!(!directs_unquoted_speech(
            "An utterly still visual narrative with no speech."
        ));
        assert!(directs_unquoted_speech(
            "No dialogue. A low murmur of awe is barely audible."
        ));
        assert!(!directs_unquoted_speech(
            "No dialogue. The murmur is explicitly wordless and nonverbal."
        ));
        assert!(directs_unquoted_speech(
            "The narrator speaks about the discovery."
        ));
        assert!(directs_unquoted_speech(
            "No dialogue. The dog whispers into the darkness."
        ));
    }

    #[test]
    fn native_prompt_gate_keeps_internal_asset_ids_out_of_renderer_prose() {
        let asset_id = "42eac5e4f66d9e70153e66f6628a3ad23d9099fea5bee1c47dc7f86f0478f8ec";
        let prompt = format!(
            "Camera tracks the subject through timed beats with live-action film lighting and textured production design while the audio carries ambience, sound effects, and score from asset {asset_id}. "
        )
        .repeat(9);
        let mut plan = MoviePlan {
            title: "Test".into(),
            logline: "Test".into(),
            audience: "Film buyers".into(),
            creative_direction: "Test".into(),
            continuity_bible: vec![
                "The archive keeps the same room, lighting, and camera axis throughout.".into(),
            ],
            source_credits: vec![],
            quality_review: MovieQualityReview::default(),
            clips: vec![PlannedClip {
                id: String::new(),
                title: "Shot".into(),
                purpose: "Test".into(),
                duration_seconds: 10.0,
                prompt,
                continuity_in: String::new(),
                continuity_out: String::new(),
                transition: "cut".into(),
                use_previous_frame: false,
                source_refs: vec![],
                reference_ids: vec![asset_id.into()],
            }],
        };
        let reference: MovieReference = serde_json::from_value(json!({
            "assetId": asset_id,
            "tag": "<Audio 1>",
            "name": "archive.wav",
            "kind": "audio",
            "mimeType": "audio/wav",
            "bytes": 1,
            "durationSeconds": 10.0,
            "width": 0,
            "height": 0,
            "hasAudio": true,
            "path": "archive.wav",
            "description": "flashback only"
        }))
        .unwrap();
        let issues = prompt_quality_issues(&plan, std::slice::from_ref(&reference)).join(" ");
        assert!(issues.contains("internal asset ID"));
        plan.clips[0].purpose = "The audio-1 spoken memory continues here.".into();
        let issues = prompt_quality_issues(&plan, &[reference]).join(" ");
        assert!(issues.contains("workspace reference ID audio-1"));
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
                    pause_after_plan: false,
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
    #[ignore = "requires the installed Bonsai model and may take several review passes"]
    async fn live_bonsai_movie_plan_clears_the_production_prompt_gate() {
        let root = tempfile::tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let research = ResearchSettings::default();
        let runtime = Arc::new(RuntimeManager::new());
        let cancel = CancellationToken::new();
        let result: Result<MovieProject, String> = async {
            studio.release_comfy_memory().await;
            let project = studio
                .create(
                    StartMovieRequest {
                        prompt: "Create exactly two five-second clips for a premium live-action trailer about a bicycle courier crossing a rain-soaked future city. The first is a continuous pursuit beat; the second is a hard-cut product-style reveal of the sealed package. Native city sound and music must be designed for both clips. No titles or logos.".into(),
                        settings: MovieSettings {
                            width: 864,
                            height: 480,
                            max_clips: 2,
                            ..MovieSettings::default()
                        },
                        references: vec![],
                        pause_after_plan: false,
                    },
                    false,
                )
                .map_err(|error| error.to_string())?;
            let lease = runtime
                .lease_research(&research)
                .await
                .map_err(|error| error.to_string())?;
            studio
                .plan(&project.id, &lease.connection, &research, &cancel, None)
                .await
                .map_err(|error| error.to_string())
        }
        .await;
        let _ = runtime.stop_managed().await;
        let _ = crate::services::stop_bonsai(&research.bonsai_root).await;
        let project = result.unwrap();
        let plan = project.plan.as_ref().unwrap();
        assert_eq!(plan.clips.len(), 2);
        assert!(plan.quality_review.score >= 90);
        assert!(plan.quality_review.attempts >= 2);
        eprintln!(
            "Accepted at {}/100 after {} attempt(s): {}",
            plan.quality_review.score, plan.quality_review.attempts, plan.quality_review.verdict
        );
        assert!(prompt_quality_issues(plan, &project.references).is_empty());
        assert!(project.sources.is_empty());
        for clip in &plan.clips {
            eprintln!("{}\n{}\n", clip.title, clip.prompt);
        }
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
                settings: MovieSettings { width: 864, height: 480, max_clips: 1, ..MovieSettings::default() },
                references: vec![],
                pause_after_plan: false,
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
        let probe = std::process::Command::new(media_program("ffprobe"))
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
        let picture_output = std::process::Command::new(media_program("ffmpeg"))
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
        let audio_output = std::process::Command::new(media_program("ffmpeg"))
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
                        prompt: "Make one five-second cinematic abstract sunrise shot. Use the attached picture for the exact burnt-orange color field and composition. Use the attached audio exactly as the clip's soundtrack underneath the image.".into(),
                        settings: MovieSettings {
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
                                description: "This exact five-second 220 Hz tone is the clip audio and belongs under the entire sunrise shot.".into(),
                                use_embedded_audio: false,
                                embedded_audio_description: String::new(),
                            },
                        ],
                        pause_after_plan: false,
                    },
                    false,
                )
                .map_err(|error| error.to_string())?;
            let lease = runtime
                .lease_research(&research)
                .await
                .map_err(|error| error.to_string())?;
            eprintln!("live reference movie: directing with Bonsai and producer manifest");
            let planned = match studio
                .plan(&project.id, &lease.connection, &research, &cancel, None)
                .await
            {
                Ok(planned) => planned,
                Err(error) => {
                    studio.fail(&project.id, &error, None);
                    return Err(error.to_string());
                }
            };
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
        let probe = std::process::Command::new(media_program("ffprobe"))
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
    #[ignore = "requires KESTREL_ACCEPTANCE_LIBRARY, KESTREL_ACCEPTANCE_PICTURE, KESTREL_ACCEPTANCE_AUDIO, installed Bonsai, MiniMax H3, ComfyUI, FFmpeg, and about two hours"]
    async fn live_long_africa_vlogger_reference_movie() {
        let library = std::env::var_os("KESTREL_ACCEPTANCE_LIBRARY")
            .map(PathBuf::from)
            .expect("KESTREL_ACCEPTANCE_LIBRARY must point to the durable Kestrel library");
        let picture = std::env::var_os("KESTREL_ACCEPTANCE_PICTURE")
            .map(PathBuf::from)
            .expect("KESTREL_ACCEPTANCE_PICTURE must point to the vlogger image");
        let audio = std::env::var_os("KESTREL_ACCEPTANCE_AUDIO")
            .map(PathBuf::from)
            .expect("KESTREL_ACCEPTANCE_AUDIO must point to the flashback recording");
        assert!(picture.is_file(), "missing picture: {}", picture.display());
        assert!(audio.is_file(), "missing audio: {}", audio.display());

        let studio = MovieStudio::new(&library).unwrap();
        let picture_asset = studio.import_reference_path(&picture).unwrap();
        let audio_asset = studio.import_reference_path(&audio).unwrap();
        let picture_id = picture_asset.id.clone();
        let audio_id = audio_asset.id.clone();
        let research = ResearchSettings::default();
        let runtime = Arc::new(RuntimeManager::new());
        let cancel = CancellationToken::new();
        let producer_prompt = "An African vlogger lowers his camera in a glowing, misty jungle, completely amazed by a beautiful wild animal. The audio is for the flashback scene and the reference image is the vlogger. Make the finished film about 2 to 3 minutes.";

        let result: Result<(MovieProject, MoviePlan), String> = async {
            eprintln!("AFRICA ACCEPTANCE: releasing ComfyUI memory before Bonsai planning");
            studio.release_comfy_memory().await;
            let project = studio
                .create(
                    StartMovieRequest {
                        prompt: producer_prompt.into(),
                        settings: MovieSettings {
                            width: 864,
                            height: 480,
                            clip_seconds: 10.0,
                            steps: 20,
                            max_clips: 18,
                            seed: 20_260_806,
                            ..MovieSettings::default()
                        },
                        references: vec![
                            ProducerReferenceRequest {
                                asset_id: picture_id.clone(),
                                description: "This is the vlogger's identity reference. Whenever he appears, preserve his face, short natural hair, trimmed beard, olive field overshirt, rust T-shirt, canvas backpack straps, and compact vlogging camera.".into(),
                                use_embedded_audio: false,
                                embedded_audio_description: String::new(),
                            },
                            ProducerReferenceRequest {
                                asset_id: audio_id.clone(),
                                description: "This spoken voice and vocal-timbre reference belongs only in the film's flashback scene. Use its speaker identity for short original dialogue stated exactly in the clip prompt. Do not assign it to a present-day scene or to any other clip.".into(),
                                use_embedded_audio: false,
                                embedded_audio_description: String::new(),
                            },
                        ],
                        pause_after_plan: false,
                    },
                    true,
                )
                .map_err(|error| error.to_string())?;
            eprintln!(
                "AFRICA ACCEPTANCE PROJECT: {}\nAFRICA ACCEPTANCE PATH: {}",
                project.id,
                studio.project_dir(&project.id).display()
            );
            let lease = runtime
                .lease_research(&research)
                .await
                .map_err(|error| error.to_string())?;
            eprintln!("AFRICA ACCEPTANCE: Bonsai is writing and reviewing the unattended plan");
            let planned = match studio
                .plan(&project.id, &lease.connection, &research, &cancel, None)
                .await
            {
                Ok(planned) => planned,
                Err(error) => {
                    studio.fail(&project.id, &error, None);
                    return Err(error.to_string());
                }
            };
            let plan = planned
                .plan
                .clone()
                .ok_or("Bonsai committed no movie plan")?;
            let planned_seconds = plan
                .clips
                .iter()
                .map(|clip| clip.duration_seconds)
                .sum::<f32>();
            let picture_clips = plan
                .clips
                .iter()
                .enumerate()
                .filter(|(_, clip)| clip.reference_ids.contains(&picture_id))
                .map(|(index, _)| index + 1)
                .collect::<Vec<_>>();
            let audio_clips = plan
                .clips
                .iter()
                .enumerate()
                .filter(|(_, clip)| clip.reference_ids.contains(&audio_id))
                .map(|(index, _)| index + 1)
                .collect::<Vec<_>>();
            let continuations = plan
                .clips
                .iter()
                .enumerate()
                .filter(|(_, clip)| clip.use_previous_frame)
                .map(|(index, _)| index + 1)
                .collect::<Vec<_>>();
            let vlogger_aliases = identity_subject_aliases(&plan, "vlogger");
            let subject_free_clips = plan
                .clips
                .iter()
                .enumerate()
                .filter(|(_, clip)| {
                    !clip.use_previous_frame
                        && !clip.reference_ids.contains(&picture_id)
                        && clip_is_independent_subject_free(clip, &vlogger_aliases)
                })
                .map(|(index, _)| index + 1)
                .collect::<Vec<_>>();
            eprintln!(
                "AFRICA ACCEPTANCE PLAN: {} clips, {:.1}s planned, quality {}/100 after {} attempt(s)\nPicture clips: {:?}\nAudio clips: {:?}\nPrevious-frame continuations: {:?}\nSubject-free clips: {:?}",
                plan.clips.len(),
                planned_seconds,
                plan.quality_review.score,
                plan.quality_review.attempts,
                picture_clips,
                audio_clips,
                continuations,
                subject_free_clips
            );
            for (index, clip) in plan.clips.iter().enumerate() {
                eprintln!(
                    "\n===== SCENE {}: {} ({:.1}s) =====\nPurpose: {}\nTransition: {}\nPrevious frame: {}\nReferences: {:?}\nContinuity in: {}\nContinuity out: {}\n{}",
                    index + 1,
                    clip.title,
                    clip.duration_seconds,
                    clip.purpose,
                    clip.transition,
                    clip.use_previous_frame,
                    clip.reference_ids,
                    clip.continuity_in,
                    clip.continuity_out,
                    clip.prompt
                );
            }
            let audio_scene_is_flashback = audio_clips.len() == 1 && {
                let clip = &plan.clips[audio_clips[0] - 1];
                let scene = format!("{} {} {}", clip.title, clip.purpose, clip.prompt)
                    .to_ascii_lowercase();
                ["flashback", "memory", "past"]
                    .iter()
                    .any(|marker| scene.contains(marker))
            };
            let preflight_issues = [
                (!(120.0..=180.0).contains(&planned_seconds)).then(|| {
                    format!("planned runtime is {planned_seconds:.1}s, not 120-180s")
                }),
                (audio_clips.len() != 1)
                    .then(|| format!("audio reference is assigned to {} scenes", audio_clips.len())),
                (!audio_scene_is_flashback)
                    .then(|| "audio-reference scene is not an explicit flashback".to_string()),
                continuations
                    .is_empty()
                    .then(|| "plan contains no exact previous-frame continuation".to_string()),
                subject_free_clips
                    .is_empty()
                    .then(|| "no independently cut protagonist-free scene exists".to_string()),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            write_json_atomic(
                &studio.project_dir(&project.id).join("acceptance-summary.json"),
                &json!({
                    "producerPrompt": producer_prompt,
                    "plannedSeconds": planned_seconds,
                    "clipCount": plan.clips.len(),
                    "qualityScore": plan.quality_review.score,
                    "qualityAttempts": plan.quality_review.attempts,
                    "pictureClips": picture_clips,
                    "audioClips": audio_clips,
                    "previousFrameContinuations": continuations,
                    "subjectFreeClips": subject_free_clips,
                    "planningGateIssues": prompt_quality_issues(&plan, &planned.references),
                }),
            )
            .map_err(|error| error.to_string())?;
            if !preflight_issues.is_empty() {
                return Err(format!(
                    "acceptance plan must not render: {}",
                    preflight_issues.join("; ")
                ));
            }
            drop(lease);
            runtime
                .stop_managed()
                .await
                .map_err(|error| error.to_string())?;
            crate::services::stop_bonsai(&research.bonsai_root)
                .await
                .map_err(|error| error.to_string())?;
            eprintln!("AFRICA ACCEPTANCE: planning complete; MiniMax H3 rendering begins");
            let rendered = studio
                .render(&project.id, &cancel, None)
                .await
                .map_err(|error| error.to_string())?;
            Ok((rendered, plan))
        }
        .await;
        let _ = runtime.stop_managed().await;
        let _ = crate::services::stop_bonsai(&research.bonsai_root).await;
        let (project, plan) = result.unwrap();
        let planned_seconds = plan
            .clips
            .iter()
            .map(|clip| clip.duration_seconds)
            .sum::<f32>();
        let audio_clips = plan
            .clips
            .iter()
            .filter(|clip| clip.reference_ids.contains(&audio_id))
            .collect::<Vec<_>>();
        assert_eq!(project.status, "complete");
        assert!(Path::new(&project.final_path).is_file());
        assert!(prompt_quality_issues(&plan, &project.references).is_empty());
        assert!(
            (120.0..=180.0).contains(&planned_seconds),
            "Bonsai planned {planned_seconds:.1}s instead of about 2-3 minutes"
        );
        assert_eq!(
            audio_clips.len(),
            1,
            "flashback audio must belong to exactly one scene"
        );
        let audio_scene = format!(
            "{} {} {}",
            audio_clips[0].title, audio_clips[0].purpose, audio_clips[0].prompt
        )
        .to_ascii_lowercase();
        assert!(
            audio_scene.contains("flashback")
                || audio_scene.contains("memory")
                || audio_scene.contains("past"),
            "the sole audio-reference scene is not identified as a flashback"
        );
        assert!(
            plan.clips.iter().any(|clip| clip.use_previous_frame),
            "Bonsai planned no continuous previous-frame handoff"
        );
        assert!(
            plan.clips.iter().any(|clip| !clip.use_previous_frame),
            "Bonsai planned no independent or non-continuous scene"
        );
        let probe = std::process::Command::new(media_program("ffprobe"))
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=nw=1:nk=1",
            ])
            .arg(&project.final_path)
            .output()
            .unwrap();
        assert!(probe.status.success());
        let rendered_seconds = String::from_utf8_lossy(&probe.stdout)
            .trim()
            .parse::<f64>()
            .unwrap();
        eprintln!(
            "AFRICA ACCEPTANCE COMPLETE: {:.2}s review cut\n{}",
            rendered_seconds, project.final_path
        );
        assert!((115.0..=195.0).contains(&rendered_seconds));
    }

    #[tokio::test]
    #[ignore = "requires KESTREL_ACCEPTANCE_LIBRARY, KESTREL_ACCEPTANCE_PICTURE, installed Bonsai, MiniMax H3, ComfyUI, FFmpeg, and several hours"]
    async fn live_long_moon_cat_reference_movie() {
        let library = std::env::var_os("KESTREL_ACCEPTANCE_LIBRARY")
            .map(PathBuf::from)
            .expect("KESTREL_ACCEPTANCE_LIBRARY must point to the durable Kestrel library");
        let picture = std::env::var_os("KESTREL_ACCEPTANCE_PICTURE")
            .map(PathBuf::from)
            .expect("KESTREL_ACCEPTANCE_PICTURE must point to the cat identity image");
        assert!(picture.is_file(), "missing picture: {}", picture.display());

        let studio = MovieStudio::new(&library).unwrap();
        let picture_asset = studio.import_reference_path(&picture).unwrap();
        let picture_id = picture_asset.id.clone();
        let research = ResearchSettings::default();
        let runtime = Arc::new(RuntimeManager::new());
        let cancel = CancellationToken::new();
        let producer_prompt = "A cat lowers his pawn in a glowing, misty moon, completely amazed by a beautiful view of earth. the reference image is the cat. Make the finished film about 2 to 3 minutes.";
        let existing_project_id = std::env::var("KESTREL_ACCEPTANCE_PROJECT_ID").ok();

        let result: Result<(MovieProject, MoviePlan), String> = async {
            eprintln!("MOON CAT ACCEPTANCE: releasing ComfyUI memory before Bonsai planning");
            studio.release_comfy_memory().await;
            let project = if let Some(id) = existing_project_id.as_deref() {
                studio
                    .begin_resume(id, None)
                    .map_err(|error| error.to_string())?
            } else {
                studio
                    .create(
                    StartMovieRequest {
                        prompt: producer_prompt.into(),
                        settings: MovieSettings {
                            width: 864,
                            height: 480,
                            clip_seconds: 10.0,
                            steps: 20,
                            max_clips: 18,
                            seed: 20_260_807,
                            ..MovieSettings::default()
                        },
                        references: vec![ProducerReferenceRequest {
                            asset_id: picture_id.clone(),
                            description: "This is the cat's identity reference.".into(),
                            use_embedded_audio: false,
                            embedded_audio_description: String::new(),
                        }],
                        pause_after_plan: false,
                    },
                    true,
                )
                    .map_err(|error| error.to_string())?
            };
            eprintln!(
                "MOON CAT ACCEPTANCE PROJECT: {}\nMOON CAT ACCEPTANCE PATH: {}",
                project.id,
                studio.project_dir(&project.id).display()
            );
            let lease = runtime
                .lease_research(&research)
                .await
                .map_err(|error| error.to_string())?;
            eprintln!("MOON CAT ACCEPTANCE: Bonsai is writing and reviewing the unattended plan");
            let planned = match studio
                .plan(&project.id, &lease.connection, &research, &cancel, None)
                .await
            {
                Ok(planned) => planned,
                Err(error) => {
                    studio.fail(&project.id, &error, None);
                    return Err(error.to_string());
                }
            };
            let plan = planned
                .plan
                .clone()
                .ok_or("Bonsai committed no movie plan")?;
            let planned_seconds = plan
                .clips
                .iter()
                .map(|clip| clip.duration_seconds)
                .sum::<f32>();
            let picture_clips = plan
                .clips
                .iter()
                .enumerate()
                .filter(|(_, clip)| clip.reference_ids.contains(&picture_id))
                .map(|(index, _)| index + 1)
                .collect::<Vec<_>>();
            let continuations = plan
                .clips
                .iter()
                .enumerate()
                .filter(|(_, clip)| clip.use_previous_frame)
                .map(|(index, _)| index + 1)
                .collect::<Vec<_>>();
            let cat_aliases = identity_subject_aliases(&plan, "cat");
            let subject_free_clips = plan
                .clips
                .iter()
                .enumerate()
                .filter(|(_, clip)| {
                    !clip.use_previous_frame
                        && !clip.reference_ids.contains(&picture_id)
                        && clip_is_independent_subject_free(clip, &cat_aliases)
                })
                .map(|(index, _)| index + 1)
                .collect::<Vec<_>>();
            let planning_gate_issues = prompt_quality_issues(&plan, &planned.references);
            eprintln!(
                "MOON CAT ACCEPTANCE PLAN: {} clips, {:.1}s planned, quality {}/100 after {} attempt(s)\nPicture clips: {:?}\nPrevious-frame continuations: {:?}\nSubject-free clips: {:?}",
                plan.clips.len(),
                planned_seconds,
                plan.quality_review.score,
                plan.quality_review.attempts,
                picture_clips,
                continuations,
                subject_free_clips
            );
            for (index, clip) in plan.clips.iter().enumerate() {
                eprintln!(
                    "\n===== SCENE {}: {} ({:.1}s) =====\nPurpose: {}\nTransition: {}\nPrevious frame: {}\nReferences: {:?}\nContinuity in: {}\nContinuity out: {}\n{}",
                    index + 1,
                    clip.title,
                    clip.duration_seconds,
                    clip.purpose,
                    clip.transition,
                    clip.use_previous_frame,
                    clip.reference_ids,
                    clip.continuity_in,
                    clip.continuity_out,
                    clip.prompt
                );
            }
            write_json_atomic(
                &studio.project_dir(&project.id).join("acceptance-summary.json"),
                &json!({
                    "producerPrompt": producer_prompt,
                    "plannedSeconds": planned_seconds,
                    "clipCount": plan.clips.len(),
                    "qualityScore": plan.quality_review.score,
                    "qualityAttempts": plan.quality_review.attempts,
                    "pictureClips": picture_clips,
                    "previousFrameContinuations": continuations,
                    "subjectFreeClips": subject_free_clips,
                    "planningGateIssues": planning_gate_issues,
                }),
            )
            .map_err(|error| error.to_string())?;
            let preflight_issues = [
                (!(120.0..=180.0).contains(&planned_seconds))
                    .then(|| format!("planned runtime is {planned_seconds:.1}s, not 120-180s")),
                picture_clips
                    .is_empty()
                    .then(|| "cat identity reference is assigned to no scene".to_string()),
                continuations
                    .is_empty()
                    .then(|| "plan contains no exact previous-frame continuation".to_string()),
                subject_free_clips
                    .is_empty()
                    .then(|| "no independently cut cat-free scene exists".to_string()),
                (!planning_gate_issues.is_empty())
                    .then(|| format!("planning gate still reports {} issue(s)", planning_gate_issues.len())),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            if !preflight_issues.is_empty() {
                return Err(format!(
                    "acceptance plan must not render: {}",
                    preflight_issues.join("; ")
                ));
            }
            drop(lease);
            runtime
                .stop_managed()
                .await
                .map_err(|error| error.to_string())?;
            crate::services::stop_bonsai(&research.bonsai_root)
                .await
                .map_err(|error| error.to_string())?;
            eprintln!("MOON CAT ACCEPTANCE: planning complete; MiniMax H3 rendering begins");
            let rendered = studio
                .render(&project.id, &cancel, None)
                .await
                .map_err(|error| error.to_string())?;
            Ok((rendered, plan))
        }
        .await;
        let _ = runtime.stop_managed().await;
        let _ = crate::services::stop_bonsai(&research.bonsai_root).await;
        let (project, plan) = result.unwrap();
        let planned_seconds = plan
            .clips
            .iter()
            .map(|clip| clip.duration_seconds)
            .sum::<f32>();
        assert_eq!(project.status, "complete");
        assert!(Path::new(&project.final_path).is_file());
        assert!(prompt_quality_issues(&plan, &project.references).is_empty());
        assert!((120.0..=180.0).contains(&planned_seconds));
        assert!(plan.clips.iter().any(|clip| clip.use_previous_frame));
        assert!(plan.clips.iter().any(|clip| !clip.use_previous_frame));
        eprintln!("MOON CAT ACCEPTANCE COMPLETE: {}", project.final_path);
    }

    #[tokio::test]
    #[ignore = "requires KESTREL_ACCEPTANCE_LIBRARY, KESTREL_ACCEPTANCE_PICTURE, installed Bonsai, MiniMax H3, ComfyUI, FFmpeg, and several hours"]
    async fn live_long_football_circus_reference_movie() {
        let library = std::env::var_os("KESTREL_ACCEPTANCE_LIBRARY")
            .map(PathBuf::from)
            .expect("KESTREL_ACCEPTANCE_LIBRARY must point to the durable Kestrel library");
        let picture = std::env::var_os("KESTREL_ACCEPTANCE_PICTURE")
            .map(PathBuf::from)
            .expect("KESTREL_ACCEPTANCE_PICTURE must point to the generated footballer image");
        assert!(picture.is_file(), "missing picture: {}", picture.display());

        let studio = MovieStudio::new(&library).unwrap();
        let picture_asset = studio.import_reference_path(&picture).unwrap();
        let picture_id = picture_asset.id.clone();
        let research = ResearchSettings::default();
        let runtime = Arc::new(RuntimeManager::new());
        let cancel = CancellationToken::new();
        let producer_prompt = "A football player looks at his feet in a shimmering, misty morning, completely amazed by a beautiful view of the construction-site circus. the reference image is the football player. Make the finished film about 2 to 3 minutes.";
        let existing_project_id = std::env::var("KESTREL_ACCEPTANCE_PROJECT_ID").ok();

        let result: Result<(MovieProject, MoviePlan), String> = async {
            eprintln!("FOOTBALL CIRCUS ACCEPTANCE: releasing ComfyUI memory before unattended Bonsai planning");
            studio.release_comfy_memory().await;
            let project = if let Some(id) = existing_project_id.as_deref() {
                studio
                    .begin_resume(id, None)
                    .map_err(|error| error.to_string())?
            } else {
                studio
                    .create(
                        StartMovieRequest {
                            prompt: producer_prompt.into(),
                            settings: MovieSettings {
                                width: 864,
                                height: 480,
                                clip_seconds: 10.0,
                                steps: 20,
                                max_clips: 18,
                                seed: 20_260_812,
                                ..MovieSettings::default()
                            },
                            references: vec![ProducerReferenceRequest {
                                asset_id: picture_id.clone(),
                                description: "This is the immutable identity and wardrobe reference for Elias Vance, the adult football player. Whenever Elias is visibly present, preserve the same face, short dark hair, athletic build, navy number-one practice jersey, charcoal shorts, white socks, black football boots, and taped left forearm. Use this reference only when Elias is actually visible; do not force it into circus inserts, environment views, object details, or other protagonist-free scenes.".into(),
                                use_embedded_audio: false,
                                embedded_audio_description: String::new(),
                            }],
                            pause_after_plan: false,
                        },
                        true,
                    )
                    .map_err(|error| error.to_string())?
            };
            eprintln!(
                "FOOTBALL CIRCUS PROJECT: {}\nFOOTBALL CIRCUS PATH: {}\nREFERENCE ASSET: {}\nNO IMPORTED AUDIO REFERENCE",
                project.id,
                studio.project_dir(&project.id).display(),
                picture_id
            );
            let lease = runtime
                .lease_research(&research)
                .await
                .map_err(|error| error.to_string())?;
            eprintln!("FOOTBALL CIRCUS ACCEPTANCE: Bonsai is writing, linting, and reviewing the plan without producer redirection");
            let planned = match studio
                .plan(&project.id, &lease.connection, &research, &cancel, None)
                .await
            {
                Ok(planned) => planned,
                Err(error) => {
                    studio.fail(&project.id, &error, None);
                    return Err(error.to_string());
                }
            };
            let plan = planned
                .plan
                .clone()
                .ok_or("Bonsai committed no movie plan")?;
            let planned_seconds = plan
                .clips
                .iter()
                .map(|clip| clip.duration_seconds)
                .sum::<f32>();
            let picture_clips = plan
                .clips
                .iter()
                .enumerate()
                .filter(|(_, clip)| clip.reference_ids.contains(&picture_id))
                .map(|(index, _)| index + 1)
                .collect::<Vec<_>>();
            let continuations = plan
                .clips
                .iter()
                .enumerate()
                .filter(|(_, clip)| clip.use_previous_frame)
                .map(|(index, _)| index + 1)
                .collect::<Vec<_>>();
            let aliases = identity_subject_aliases(&plan, "football player");
            let subject_free_clips = plan
                .clips
                .iter()
                .enumerate()
                .filter(|(_, clip)| {
                    !clip.use_previous_frame
                        && !clip.reference_ids.contains(&picture_id)
                        && clip_is_independent_subject_free(clip, &aliases)
                })
                .map(|(index, _)| index + 1)
                .collect::<Vec<_>>();
            let planning_gate_issues = prompt_quality_issues(&plan, &planned.references);
            eprintln!(
                "FOOTBALL CIRCUS PLAN: {} clips, {:.1}s planned, quality {}/100 after {} attempt(s)\nIdentity-reference clips: {:?}\nPrevious-frame continuations: {:?}\nIndependent protagonist-free clips: {:?}\nNative gate issues after planning: {}",
                plan.clips.len(),
                planned_seconds,
                plan.quality_review.score,
                plan.quality_review.attempts,
                picture_clips,
                continuations,
                subject_free_clips,
                planning_gate_issues.len()
            );
            for (index, clip) in plan.clips.iter().enumerate() {
                eprintln!(
                    "\n===== SCENE {}: {} ({:.1}s) =====\nPurpose: {}\nTransition: {}\nPrevious frame: {}\nReferences: {:?}\nContinuity in: {}\nContinuity out: {}\n{}",
                    index + 1,
                    clip.title,
                    clip.duration_seconds,
                    clip.purpose,
                    clip.transition,
                    clip.use_previous_frame,
                    clip.reference_ids,
                    clip.continuity_in,
                    clip.continuity_out,
                    clip.prompt
                );
            }
            let evaluation_issues = [
                (!(120.0..=180.0).contains(&planned_seconds))
                    .then(|| format!("planned runtime is {planned_seconds:.1}s, not 120-180s")),
                picture_clips
                    .is_empty()
                    .then(|| "footballer identity reference is assigned to no scene".to_string()),
                continuations
                    .is_empty()
                    .then(|| "plan contains no exact previous-frame continuation".to_string()),
                subject_free_clips
                    .is_empty()
                    .then(|| "no independently cut protagonist-free scene exists".to_string()),
                (!planning_gate_issues.is_empty())
                    .then(|| format!("native planning gate still reports {} issue(s)", planning_gate_issues.len())),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            write_json_atomic(
                &studio.project_dir(&project.id).join("acceptance-summary.json"),
                &json!({
                    "scenario": "quick evening idea to unattended morning sales-material review cut",
                    "producerPrompt": producer_prompt,
                    "generatedReferenceAssetId": picture_id,
                    "generatedReferencePath": picture,
                    "importedAudioReferences": 0,
                    "plannedSeconds": planned_seconds,
                    "clipCount": plan.clips.len(),
                    "qualityScore": plan.quality_review.score,
                    "qualityAttempts": plan.quality_review.attempts,
                    "pictureClips": picture_clips,
                    "previousFrameContinuations": continuations,
                    "subjectFreeClips": subject_free_clips,
                    "planningGateIssues": &planning_gate_issues,
                    "evaluationIssues": &evaluation_issues
                }),
            )
            .map_err(|error| error.to_string())?;
            if !evaluation_issues.is_empty() {
                return Err(format!(
                    "football circus acceptance gate failed: {}",
                    evaluation_issues.join("; ")
                ));
            }
            drop(lease);
            runtime
                .stop_managed()
                .await
                .map_err(|error| error.to_string())?;
            crate::services::stop_bonsai(&research.bonsai_root)
                .await
                .map_err(|error| error.to_string())?;
            eprintln!("FOOTBALL CIRCUS ACCEPTANCE: unattended plan accepted; MiniMax H3 rendering begins");
            let rendered = studio
                .render(&project.id, &cancel, None)
                .await
                .map_err(|error| error.to_string())?;
            Ok((rendered, plan))
        }
        .await;
        let _ = runtime.stop_managed().await;
        let _ = crate::services::stop_bonsai(&research.bonsai_root).await;
        let (project, plan) = result.unwrap();
        let planned_seconds = plan
            .clips
            .iter()
            .map(|clip| clip.duration_seconds)
            .sum::<f32>();
        assert_eq!(project.status, "complete");
        assert!(Path::new(&project.final_path).is_file());
        assert!(prompt_quality_issues(&plan, &project.references).is_empty());
        assert!((120.0..=180.0).contains(&planned_seconds));
        assert!(plan.clips.iter().any(|clip| clip.use_previous_frame));
        assert!(plan.clips.iter().any(|clip| !clip.use_previous_frame));
        assert!(plan
            .clips
            .iter()
            .any(|clip| clip.reference_ids.contains(&picture_id)));
        let aliases = identity_subject_aliases(&plan, "football player");
        assert!(plan.clips.iter().any(|clip| {
            !clip.use_previous_frame
                && !clip.reference_ids.contains(&picture_id)
                && clip_is_independent_subject_free(clip, &aliases)
        }));
        eprintln!(
            "FOOTBALL CIRCUS ACCEPTANCE COMPLETE: {}",
            project.final_path
        );
    }

    #[tokio::test]
    async fn default_review_cut_preserves_native_clip_duration_and_audio() {
        let ffmpeg_available = std::process::Command::new(media_program("ffmpeg"))
            .arg("-version")
            .output()
            .is_ok_and(|output| output.status.success());
        let ffprobe_available = std::process::Command::new(media_program("ffprobe"))
            .arg("-version")
            .output()
            .is_ok_and(|output| output.status.success());
        if !ffmpeg_available || !ffprobe_available {
            eprintln!("skipping native review-cut regression: FFmpeg and FFprobe are required");
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let studio = MovieStudio::new(temp.path()).unwrap();
        let mut project = studio
            .create(
                StartMovieRequest {
                    prompt: "Two deterministic test shots".into(),
                    settings: MovieSettings::default(),
                    references: vec![],
                    pause_after_plan: false,
                },
                false,
            )
            .unwrap();
        let raw = studio.project_dir(&project.id).join("raw");
        let generated = raw.join("generated.mp4");
        let second = raw.join("second.mp4");
        for (path, color, frequency) in [(&generated, "red", "440"), (&second, "blue", "660")] {
            let output = std::process::Command::new(media_program("ffmpeg"))
                .args([
                    "-y",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                ])
                .arg(format!("color=c={color}:s=64x64:r=24:d=1.125"))
                .args(["-f", "lavfi", "-i"])
                .arg(format!(
                    "sine=frequency={frequency}:sample_rate=32000:duration=1.125"
                ))
                .args([
                    "-shortest",
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                ])
                .arg(path)
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        project.clips = [generated, second]
            .into_iter()
            .enumerate()
            .map(|(index, path)| RenderedClip {
                id: format!("clip-{index}"),
                index: index as u32,
                title: format!("Clip {}", index + 1),
                prompt: "test".into(),
                duration_seconds: 1.0,
                seed: index as u64,
                status: "complete".into(),
                path: path.to_string_lossy().into_owned(),
                error: String::new(),
                versions: Vec::new(),
            })
            .collect();
        let assembled = studio.assemble_default(&project).await.unwrap();
        let probe = std::process::Command::new(media_program("ffprobe"))
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=nw=1:nk=1",
            ])
            .arg(&assembled)
            .output()
            .unwrap();
        assert!(probe.status.success());
        let duration = String::from_utf8_lossy(&probe.stdout)
            .trim()
            .parse::<f64>()
            .unwrap();
        assert!(
            (duration - 2.25).abs() < 0.08,
            "assembled duration was {duration}"
        );
        let streams = std::process::Command::new(media_program("ffprobe"))
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "csv=p=0",
            ])
            .arg(&assembled)
            .output()
            .unwrap();
        let streams = String::from_utf8_lossy(&streams.stdout);
        assert!(streams.contains("video"));
        assert!(streams.contains("audio"));

        project.edit = MovieEdit {
            clips: vec![
                ClipEdit {
                    id: "timeline-fast".into(),
                    clip_id: "clip-0".into(),
                    enabled: true,
                    order: 0,
                    trim_start: 0.1,
                    trim_end: 0.1,
                    audio_gain: 0.8,
                    source_version_id: String::new(),
                    speed: 2.0,
                    fade_in: 0.1,
                    fade_out: 0.1,
                    audio_fade_in: 0.1,
                    audio_fade_out: 0.1,
                    label: "Fast opening".into(),
                    notes: "Producer-approved timing.".into(),
                },
                ClipEdit {
                    id: "timeline-second".into(),
                    clip_id: "clip-1".into(),
                    enabled: true,
                    order: 1,
                    trim_start: 0.0,
                    trim_end: 0.0,
                    audio_gain: 1.0,
                    source_version_id: String::new(),
                    speed: 1.0,
                    fade_in: 0.0,
                    fade_out: 0.0,
                    audio_fade_in: 0.0,
                    audio_fade_out: 0.0,
                    label: String::new(),
                    notes: String::new(),
                },
            ],
            export_title: "Offline Timeline Regression".into(),
            export_preset: "review".into(),
            normalize_audio: true,
            target_lufs: -16.0,
            markers: Vec::new(),
        };
        studio.save(&project).unwrap();
        let edited = studio.render_edit(&project.id).await.unwrap();
        let export = edited.exports.last().unwrap();
        assert_eq!(export.title, "Offline Timeline Regression");
        assert_eq!(export.preset, "review");
        assert_eq!(export.clip_count, 2);
        assert!((export.duration_seconds - 1.4).abs() < 0.01);
        assert_eq!(export.sha256.len(), 64);
        assert!(Path::new(&export.path).is_file());
        assert!(Path::new(&export.path).with_extension("json").is_file());
    }
}
