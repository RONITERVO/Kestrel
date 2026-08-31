//! Durable offline movie Studio facade.
//!
//! Maintainers should read `studio/README.md` before changing model-assisted flows or persistence
//! boundaries. Child modules own producer state, rendering, media, and editing concerns.

use crate::models::{ControlSettings, ResearchSettings};
use chrono::Utc;
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
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use thiserror::Error;
use tokio::{process::Child, sync::Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;

mod image_assets;
mod image_studio;
mod live_preview;
mod model_stream;
mod music;
mod music_lyrics_model;
mod music_midi;
mod producer;
mod producer_chat;
mod prompt_draft;
pub use image_assets::{
    emit_image_asset_error, GeneratedImageProvenance, MovieImageAssetGeneration,
    MovieImageAssetRequest,
};
pub use image_studio::{CreateImageProjectRequest, ImageProject, ImageStudio, ImageSummary};
pub use live_preview::MovieRenderState;
use live_preview::{
    emit_preview_unavailable, preview_node, LivePreviewRegistry, LivePreviewSession, PreviewTarget,
    PREVIEW_NODE_ID,
};
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
pub use producer_chat::{
    emit_error as emit_studio_chat_error, emit_settled as emit_studio_chat_settled,
    summarize_conversation as summarize_studio_conversation, MovieStudioChatJob,
};
pub(crate) use prompt_draft::{
    emit_error as emit_prompt_draft_error, emit_settled as emit_prompt_draft_settled,
    validate_request as validate_prompt_draft_request,
};
pub use prompt_draft::{PromptDraftJob, PromptDraftRequest};

const SCHEMA_VERSION: u32 = 7;
const COMFY_BASE: &str = "http://127.0.0.1:8188";
pub(super) const MUSIC_COMFY_BASE: &str = "http://127.0.0.1:8189";
const MAX_REFERENCE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_AUDIO_BYTES: u64 = 256 * 1024 * 1024;
const HASH_BUFFER_BYTES: usize = 64 * 1024;
pub(super) const MAX_MOVIE_PROMPT_BYTES: usize = 64 * 1024;
const MAX_REFERENCE_SECONDS: f64 = 15.1;
const MOVIE_THINKING_BUDGET: u32 = 32_768;
const COMFY_RENDER_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const MIN_TIMELINE_SOURCE_SECONDS: f32 = 0.1;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimePolicyTier {
    maximum_context_window: u32,
    maximum_max_output_tokens: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimePolicyLimitsFile {
    minimum_context_window: u32,
    minimum_max_output_tokens: u32,
    standard: RuntimePolicyTier,
    advanced: RuntimePolicyTier,
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimePolicyLimits {
    pub minimum_context_window: u32,
    pub minimum_max_output_tokens: u32,
    pub maximum_context_window: u32,
    pub maximum_max_output_tokens: u32,
}

pub(crate) fn runtime_policy_limits(advanced: bool) -> RuntimePolicyLimits {
    static LIMITS: OnceLock<RuntimePolicyLimitsFile> = OnceLock::new();
    let limits = LIMITS.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../apps/desktop/src/features/control/runtimePolicyLimits.json"
        ))
        .expect("the shared runtime policy limits must be valid JSON")
    });
    let tier = if advanced {
        limits.advanced
    } else {
        limits.standard
    };
    RuntimePolicyLimits {
        minimum_context_window: limits.minimum_context_window,
        minimum_max_output_tokens: limits.minimum_max_output_tokens,
        maximum_context_window: tier.maximum_context_window,
        maximum_max_output_tokens: tier.maximum_max_output_tokens,
    }
}

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
        "ogg" | "oga" => "audio/ogg",
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
    #[error("MiniMax H3 render failed: {0}")]
    Render(String),
    #[error("studio operation was stopped")]
    Cancelled,
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
    /// Zero in a legacy project means inherit the selected model's System/per-model context.
    #[serde(default)]
    pub context_window: u32,
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
            context_window: 0,
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
        let runtime_limits = runtime_policy_limits(advanced);
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
        self.thinking_budget = self.thinking_budget.min(MOVIE_THINKING_BUDGET);
        self.max_output_tokens = self.max_output_tokens.clamp(
            runtime_limits.minimum_max_output_tokens,
            runtime_limits.maximum_max_output_tokens,
        );
        if self.context_window > 0 {
            self.context_window = self.context_window.clamp(
                runtime_limits.minimum_context_window,
                runtime_limits.maximum_context_window,
            );
        }
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

    /// Apply the project layer after System defaults and per-model policy.
    pub(crate) fn runtime_settings_for(
        &self,
        base: &ControlSettings,
        model_id: &str,
    ) -> ControlSettings {
        let mut effective = base.for_model(model_id);
        if self.context_window > 0 {
            effective.context_window = self.context_window;
        }
        effective.max_output_tokens = self.max_output_tokens;
        effective.model_overrides.clear();
        effective
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// Producer-selected image used by H3's first-frame conditioning path.
    #[serde(default)]
    pub first_frame_reference_id: String,
    /// Producer-selected image used by H3's last-frame conditioning path.
    #[serde(default)]
    pub last_frame_reference_id: String,
    /// Producer-owned per-scene native bindings. Empty means a legacy plan using reference_ids.
    #[serde(default)]
    pub reference_selections: Vec<crate::models::MovieSceneReferenceSelection>,
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
    /// Preserved verbatim when an older project is opened; no current workflow reads it.
    #[serde(default, rename = "modelRoles", skip_serializing_if = "Value::is_null")]
    pub legacy_model_roles: Value,
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
    /// Compatibility payloads are retained so saving a legacy project is non-destructive.
    #[serde(
        default,
        rename = "producerFeedback",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub legacy_producer_feedback: Vec<Value>,
    #[serde(
        default,
        rename = "copilotHistory",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub legacy_copilot_history: Vec<Value>,
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

    pub(crate) fn create_producer_base(
        &self,
        prompt: String,
        settings: MovieSettings,
        references: Vec<ProducerReferenceRequest>,
        collaborator_name: &str,
        advanced: bool,
    ) -> Result<MovieProject, StudioError> {
        if prompt.trim().chars().count() < 3 || prompt.len() > MAX_MOVIE_PROMPT_BYTES {
            return Err(StudioError::Invalid(
                "starting material must contain 3 characters to 64 KiB".into(),
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
            status: "awaiting-review".into(),
            phase: "story-draft".into(),
            detail: "The starting material is safe. The story collaborator is ready to write one complete Markdown sketch.".into(),
            created_at: now.clone(),
            updated_at: now,
            model: format!("Producer Studio collaborator · {collaborator_name}"),
            legacy_model_roles: Value::Null,
            renderer: "MiniMax H3 / ComfyUI native".into(),
            settings,
            references,
            plan: None,
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
            producer_review_required: true,
            producer_approved_at: String::new(),
            legacy_producer_feedback: Vec::new(),
            legacy_copilot_history: Vec::new(),
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
            if target.is_file() {
                if hash_reference(&target)? != asset.id {
                    return Err(StudioError::Invalid(format!(
                        "The preserved project copy of {} failed its integrity check; it was not overwritten.",
                        asset.name
                    )));
                }
            } else {
                if target.exists() {
                    return Err(StudioError::Invalid(format!(
                        "The project reference destination for {} is not a regular file.",
                        asset.name
                    )));
                }
                if fs::hard_link(&asset.path, &target).is_err() {
                    fs::copy(&asset.path, &target)?;
                    if hash_reference(&target)? != asset.id {
                        let _ = fs::remove_file(&target);
                        return Err(StudioError::Invalid(format!(
                            "The copied project reference {} failed its integrity check.",
                            asset.name
                        )));
                    }
                }
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

    pub async fn release_comfy_memory(&self) {
        let _ = self
            .http
            .post(format!("{COMFY_BASE}/free"))
            .json(&json!({"unload_models":true,"free_memory":true}))
            .timeout(Duration::from_secs(30))
            .send()
            .await;
    }

    pub async fn render(
        &self,
        id: &str,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let mut project = self.get(id)?;
        ensure_producer_render_approval(&project)?;
        self.live_previews.clear_movie(id);
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
            return Ok(());
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
        let legacy_reference_ids = planned.reference_ids.iter().collect::<HashSet<_>>();
        let selected_references = project
            .references
            .iter()
            .filter_map(|reference| {
                let selection = planned
                    .reference_selections
                    .iter()
                    .find(|selection| selection.asset_id == reference.asset_id);
                let selected = if planned.reference_selections.is_empty() {
                    legacy_reference_ids.contains(&reference.asset_id)
                } else {
                    selection.is_some_and(|selection| selection.use_visual || selection.use_audio)
                };
                selected.then_some((reference, selection))
            })
            .collect::<Vec<_>>();
        let mut graph_references = Vec::with_capacity(selected_references.len());
        for (reference, selection) in &selected_references {
            if selection.is_some_and(|selection| {
                reference.kind == "video" && selection.use_audio && !selection.use_visual
            }) {
                return Err(StudioError::Render(
                    "H3 cannot bind embedded video audio without also binding that video's motion; update the producer reference selection"
                        .into(),
                ));
            }
            let relative = stage_movie_reference(project, reference).await?;
            graph_references.push(H3ReferenceInput {
                kind: reference.kind.as_str(),
                file: relative,
                use_embedded_audio: selection
                    .map(|selection| selection.use_audio)
                    .unwrap_or(reference.use_embedded_audio),
                description: &reference.description,
                guidance: selection
                    .map(|selection| selection.guidance.as_str())
                    .unwrap_or_default(),
            });
        }
        let explicit_first =
            stage_frame_reference(project, &planned.first_frame_reference_id, "first-frame")
                .await?;
        let explicit_last =
            stage_frame_reference(project, &planned.last_frame_reference_id, "last-frame").await?;
        if !graph_references.is_empty() && (explicit_first.is_some() || explicit_last.is_some()) {
            return Err(StudioError::Render(
                "H3 first/last-frame conditioning cannot be combined with native references in one scene"
                    .into(),
            ));
        }
        let continuity_input = if explicit_first.is_some() {
            explicit_first
        } else if graph_references.is_empty() && index > 0 && planned.use_previous_frame {
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
            last_frame: explicit_last.as_deref(),
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
            LivePreviewSession::connect(app, &client_id, preview_target, &self.live_previews).await
        } else {
            emit_preview_unavailable(app, &self.live_previews, preview_target);
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
                    let media = find_output_media(entry, "videos").ok_or_else(|| {
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
        if item.trim_start + item.trim_end > source_duration - MIN_TIMELINE_SOURCE_SECONDS {
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

fn check_cancel(cancel: &CancellationToken) -> Result<(), StudioError> {
    if cancel.is_cancelled() {
        Err(StudioError::Cancelled)
    } else {
        Ok(())
    }
}

async fn stage_movie_reference(
    project: &MovieProject,
    reference: &MovieReference,
) -> Result<String, StudioError> {
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
    Ok(relative)
}

fn ensure_producer_render_approval(project: &MovieProject) -> Result<(), StudioError> {
    if project.producer_approved_at.trim().is_empty() {
        return Err(StudioError::Invalid(
            "save and explicitly approve the producer-owned scene cards before rendering".into(),
        ));
    }
    Ok(())
}

async fn stage_frame_reference(
    project: &MovieProject,
    asset_id: &str,
    label: &str,
) -> Result<Option<String>, StudioError> {
    if asset_id.is_empty() {
        return Ok(None);
    }
    let reference = project
        .references
        .iter()
        .find(|reference| reference.asset_id == asset_id)
        .ok_or_else(|| {
            StudioError::Render(format!(
                "the producer-selected {label} image is no longer in this project"
            ))
        })?;
    if reference.kind != "image" {
        return Err(StudioError::Render(format!(
            "the producer-selected {label} input must be an image"
        )));
    }
    stage_movie_reference(project, reference).await.map(Some)
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
    last_frame: Option<&'a str>,
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
    description: &'a str,
    guidance: &'a str,
}

fn bound_reference_prompt(references: &[H3ReferenceInput<'_>]) -> String {
    let mut bindings = Vec::new();
    let mut picture = 0usize;
    for reference in references
        .iter()
        .filter(|reference| reference.kind == "image")
    {
        picture += 1;
        bindings.push(format!(
            "Use <Picture {picture}> as a visual reference for {}{}.",
            reference.description.trim(),
            reference_guidance_suffix(reference.guidance)
        ));
    }
    let mut video = 0usize;
    let mut audio = 0usize;
    for reference in references
        .iter()
        .filter(|reference| reference.kind == "video")
    {
        if reference.use_embedded_audio {
            audio += 1;
            bindings.push(format!(
                "Use <Audio {audio}> exactly as it is; do not replace, reinterpret, remix, shorten, extend, or regenerate it{}.",
                reference_guidance_suffix(reference.guidance)
            ));
        }
        video += 1;
        bindings.push(format!(
            "Use <Video {video}> as a motion reference for {}{}.",
            reference.description.trim(),
            reference_guidance_suffix(reference.guidance)
        ));
    }
    for reference in references
        .iter()
        .filter(|reference| reference.kind == "audio")
    {
        audio += 1;
        bindings.push(format!(
            "Use <Audio {audio}> exactly as it is; do not replace, reinterpret, remix, shorten, extend, or regenerate it{}.",
            reference_guidance_suffix(reference.guidance)
        ));
    }
    bindings.join(" ")
}

fn reference_guidance_suffix(guidance: &str) -> String {
    let guidance = guidance.trim();
    if guidance.is_empty() {
        String::new()
    } else {
        format!(". Producer placement: {guidance}")
    }
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
        last_frame,
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
        if let Some(image) = last_frame {
            graph["16"] = json!({"class_type":"LoadImage","inputs":{"image":image}});
            graph["5"]["inputs"]["last_frame"] = json!(["16", 0]);
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

pub(super) fn hash_file(path: &Path) -> Result<(u64, String), StudioError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn h3_frame_graph_uses_both_producer_selected_endpoints() {
        let graph = h3_graph(H3GraphRequest {
            prompt: "A timed scene.",
            width: 1344,
            height: 768,
            seconds: 5.0,
            steps: 20,
            seed: 42,
            prefix: "kestrel/test",
            first_frame: Some("first.png"),
            last_frame: Some("last.png"),
            references: &[],
            ref_image_size: "match",
            preview_available: false,
        });
        assert_eq!(
            graph["1"]["inputs"]["unet_name"],
            "minimax_h3_fl2va_pruned_int8_convrot.safetensors"
        );
        assert_eq!(graph["15"]["inputs"]["image"], "first.png");
        assert_eq!(graph["16"]["inputs"]["image"], "last.png");
        assert_eq!(graph["5"]["inputs"]["first_frame"], json!(["15", 0]));
        assert_eq!(graph["5"]["inputs"]["last_frame"], json!(["16", 0]));
    }

    #[test]
    fn h3_reference_graph_binds_exact_audio_and_native_inputs() {
        let references = vec![
            H3ReferenceInput {
                kind: "image",
                file: "identity.png".into(),
                use_embedded_audio: false,
                description: "Mara's face and raincoat",
                guidance: "keep the scar on her left cheek",
            },
            H3ReferenceInput {
                kind: "audio",
                file: "station-id.wav".into(),
                use_embedded_audio: false,
                description: "station identification",
                guidance: "begin at the first frame",
            },
        ];
        let prompt = bound_reference_prompt(&references);
        assert!(prompt.contains("Use <Picture 1> as a visual reference"));
        assert!(prompt.contains("Use <Audio 1> exactly as it is"));
        assert!(prompt
            .contains("do not replace, reinterpret, remix, shorten, extend, or regenerate it"));
        assert!(prompt.contains("Producer placement: begin at the first frame"));

        let graph = h3_graph(H3GraphRequest {
            prompt: &prompt,
            width: 1344,
            height: 768,
            seconds: 5.0,
            steps: 20,
            seed: 42,
            prefix: "kestrel/test",
            first_frame: None,
            last_frame: None,
            references: &references,
            ref_image_size: "match",
            preview_available: false,
        });
        assert_eq!(
            graph["1"]["inputs"]["unet_name"],
            "minimax_h3_ref2va_pruned_int8_convrot.safetensors"
        );
        assert_eq!(
            graph["5"]["inputs"]["ref_images.ref_image_0"],
            json!(["15", 0])
        );
        assert_eq!(
            graph["5"]["inputs"]["ref_audios.ref_audio_0"],
            json!(["16", 0])
        );
    }

    #[test]
    fn corrupt_project_json_recovers_from_the_last_complete_copy() {
        let root = tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let mut project = studio
            .create_producer_base(
                "A keeper waits beside a fog signal.".into(),
                MovieSettings::default(),
                Vec::new(),
                "test collaborator",
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
            .create_producer_base(
                "A keeper waits beside a fog signal.".into(),
                MovieSettings::default(),
                Vec::new(),
                "test collaborator",
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
