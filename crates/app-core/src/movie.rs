// Rust-owned wire and durable data. Native services retain execution authority.
use crate::{GeneratedImageProvenance, ResearchSettings};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ProducerReferenceRequest {
    pub asset_id: String,
    pub description: String,
    #[serde(default)]
    pub use_embedded_audio: bool,
    #[serde(default)]
    pub embedded_audio_description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
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
    #[ts(optional)]
    pub generation: Option<GeneratedImageProvenance>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
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
    #[ts(optional)]
    pub generation: Option<GeneratedImageProvenance>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MovieReferenceImport {
    pub references: Vec<MovieReferenceAsset>,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
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
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
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

#[derive(Debug, Default, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MovieQualityReview {
    pub attempts: u32,
    pub score: u32,
    pub verdict: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
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
    pub reference_selections: Vec<crate::MovieSceneReferenceSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MovieSource {
    pub id: String,
    pub title: String,
    pub reference: String,
    pub snapshot: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RenderedClip {
    pub id: String,
    pub index: u32,
    pub title: String,
    pub prompt: String,
    pub duration_seconds: f32,
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
    pub seed: u64,
    pub status: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub versions: Vec<ClipVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ClipVersion {
    pub id: String,
    pub created_at: String,
    pub title: String,
    pub prompt: String,
    pub duration_seconds: f32,
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
    pub seed: u64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
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

pub fn default_clip_seconds() -> f32 {
    5.0
}

pub fn default_comfy_root() -> String {
    ResearchSettings::default().comfy_root
}

pub fn default_export_preset() -> String {
    "publish".into()
}

pub fn default_export_title() -> String {
    "Kestrel Movie".into()
}

pub fn default_gain() -> f32 {
    1.0
}

pub fn default_height() -> u32 {
    448
}

pub fn default_marker_kind() -> String {
    "marker".into()
}

pub fn default_max_clips() -> u32 {
    MAX_MOVIE_SCENES
}

pub fn default_output() -> u32 {
    32_768
}

pub fn default_ref_image_size() -> String {
    "match".into()
}

pub fn default_speed() -> f32 {
    1.0
}

pub fn default_steps() -> u32 {
    20
}

pub fn default_target_lufs() -> f32 {
    -14.0
}

pub fn default_temperature() -> f32 {
    0.7
}

pub fn default_thinking() -> u32 {
    MOVIE_THINKING_BUDGET
}

pub fn default_top_k() -> u32 {
    20
}

pub fn default_top_p() -> f32 {
    0.95
}

pub fn default_width() -> u32 {
    768
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

pub const MOVIE_THINKING_BUDGET: u32 = 32_768;

pub const MAX_MOVIE_SCENES: u32 = 4_096;
