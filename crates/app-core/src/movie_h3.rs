// Rust-owned wire and durable contracts for Kestrel's MiniMax H3 Movie Studio.
// Native services retain all compilation, media compositing, and ComfyUI execution authority.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieH3Defaults {
    pub fps: u32,
    pub frame_step: u32,
    pub frame_offset: u32,
    pub min_frames: u32,
    pub max_frames: u32,
    pub default_scene_width: u32,
    pub default_scene_height: u32,
    pub draft_width: u32,
    pub draft_height: u32,
    pub detail_width: u32,
    pub detail_height: u32,
    pub upscale_width: u32,
    pub upscale_height: u32,
    pub soundtrack_width: u32,
    pub soundtrack_height: u32,
    pub shift_video: f32,
    pub shift_audio: f32,
    pub default_sampler: String,
    pub default_scheduler: String,
    pub max_picture_refs: usize,
    pub max_video_refs: usize,
    pub max_audio_refs: usize,
    pub max_guides: usize,
    pub take_placement_policy: String,
}

impl Default for MovieH3Defaults {
    fn default() -> Self {
        Self {
            fps: 24,
            frame_step: 17,
            frame_offset: 5,
            min_frames: 5,
            max_frames: 362,
            default_scene_width: 768,
            default_scene_height: 448,
            draft_width: 768,
            draft_height: 448,
            detail_width: 1344,
            detail_height: 768,
            upscale_width: 1920,
            upscale_height: 1088,
            soundtrack_width: 64,
            soundtrack_height: 32,
            shift_video: 12.0,
            shift_audio: 6.0,
            default_sampler: "er_sde".into(),
            default_scheduler: "sgm_uniform".into(),
            max_picture_refs: 9,
            max_video_refs: 3,
            max_audio_refs: 3,
            max_guides: 12,
            take_placement_policy: "authored_v1".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3PromptFormat {
    #[default]
    #[serde(alias = "h3_full_reference", alias = "fullReference", alias = "structured")]
    H3FullReference,
    #[serde(alias = "h3_base", alias = "base", alias = "prose")]
    H3Base,
    #[serde(alias = "character_swap")]
    CharacterSwap,
    #[serde(alias = "head_swap")]
    HeadSwap,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3TaskMode {
    #[default]
    #[serde(alias = "Standard", alias = "default", alias = "video_and_audio")]
    Standard,
    #[serde(alias = "keep_existing_audio", alias = "freeze_audio", alias = "lip_sync")]
    KeepExistingAudio,
    #[serde(alias = "audio_over_existing_video", alias = "freeze_video", alias = "foley")]
    AudioOverExistingVideo,
    #[serde(alias = "soundtrack_pass", alias = "soundtrack")]
    SoundtrackPass,
    #[serde(alias = "character_swap")]
    CharacterSwap,
    #[serde(alias = "head_swap")]
    HeadSwap,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3PassPreset {
    #[default]
    #[serde(alias = "draft_02_mp", alias = "draft", alias = "0.2mp", alias = "768x448", alias = "low", alias = "fast")]
    Draft02Mp,
    #[serde(alias = "detail_1_mp", alias = "detail", alias = "1mp", alias = "medium")]
    Detail1Mp,
    #[serde(alias = "upscale_1080p", alias = "upscale", alias = "1080p", alias = "high")]
    Upscale1080p,
    #[serde(alias = "Custom")]
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3ModelChoice {
    #[default]
    #[serde(alias = "Auto")]
    Auto,
    #[serde(alias = "Ref2VA", alias = "ref2v")]
    Ref2va,
    #[serde(alias = "FL2VA", alias = "fl2v")]
    Fl2va,
    #[serde(alias = "ref2va_cond_with_fl2va_model")]
    Ref2vaCondWithFl2vaModel,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3TakePlacementMode {
    #[default]
    #[serde(alias = "Trimmed", alias = "trim", alias = "default")]
    Trimmed,
    #[serde(alias = "Untrimmed", alias = "full")]
    Untrimmed,
    #[serde(alias = "asset_only", alias = "none")]
    AssetOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3SceneKind {
    #[default]
    #[serde(alias = "Standard", alias = "main", alias = "video")]
    Standard,
    #[serde(alias = "Audition", alias = "test")]
    Audition,
    #[serde(alias = "Soundtrack", alias = "audio", alias = "score")]
    Soundtrack,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3EntityKind {
    #[default]
    #[serde(alias = "Character", alias = "person", alias = "actor", alias = "subject")]
    Character,
    #[serde(alias = "Location", alias = "scene", alias = "background", alias = "environment", alias = "setting")]
    Location,
    #[serde(alias = "Prop", alias = "object", alias = "item")]
    Prop,
    #[serde(alias = "Outfit", alias = "clothing", alias = "wardrobe", alias = "costume")]
    Outfit,
    #[serde(alias = "Voice", alias = "speaker", alias = "vocal")]
    Voice,
    #[serde(alias = "Sound", alias = "audio", alias = "sfx", alias = "music")]
    Sound,
    #[serde(alias = "Other", alias = "general")]
    Other,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3ReferencePopulation {
    #[default]
    #[serde(alias = "Pictures", alias = "picture", alias = "image", alias = "images")]
    Pictures,
    #[serde(alias = "Videos", alias = "video")]
    Videos,
    #[serde(alias = "standalone_audios", alias = "standaloneAudio", alias = "audio", alias = "audios", alias = "StandaloneAudios")]
    StandaloneAudios,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3ReferenceRole {
    #[default]
    #[serde(alias = "Subject", alias = "character", alias = "identity", alias = "face")]
    Subject,
    #[serde(alias = "Outfit", alias = "wardrobe", alias = "clothing")]
    Outfit,
    #[serde(alias = "Scene", alias = "location", alias = "background", alias = "environment")]
    Scene,
    #[serde(alias = "Motion", alias = "pose", alias = "action")]
    Motion,
    #[serde(alias = "Camera", alias = "cinematography")]
    Camera,
    #[serde(alias = "Voice", alias = "speaker", alias = "speech", alias = "vocal")]
    Voice,
    #[serde(alias = "Sound", alias = "sfx", alias = "music", alias = "ambience", alias = "audio")]
    Sound,
    #[serde(alias = "General", alias = "prop", alias = "object", alias = "other")]
    General,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3RetentionLevel {
    #[default]
    #[serde(alias = "fully_preserved", alias = "strict", alias = "high")]
    FullyPreserved,
    #[serde(alias = "partially_preserved", alias = "mostlyPreserved", alias = "mostly_preserved", alias = "medium")]
    PartiallyPreserved,
    #[serde(alias = "attribute_transfer", alias = "transfer")]
    AttributeTransfer,
    #[serde(alias = "weak_reference", alias = "looselyInspired", alias = "loosely_inspired", alias = "weak", alias = "low")]
    WeakReference,
    #[serde(alias = "standard")]
    Reference,
    #[serde(alias = "fully_copy", alias = "copy", alias = "exact")]
    FullyCopy,
}

impl H3RetentionLevel {
    pub const fn as_h3_token(self) -> &'static str {
        match self {
            Self::FullyPreserved => "fully_preserved",
            Self::PartiallyPreserved => "partially_preserved",
            Self::AttributeTransfer => "attribute_transfer",
            Self::WeakReference => "weak_reference",
            Self::Reference => "reference",
            Self::FullyCopy => "fully_copy",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3VisualIntent {
    #[default]
    #[serde(
        alias = "Preserve",
        alias = "subject",
        alias = "Subject",
        alias = "character",
        alias = "identity",
        alias = "face",
        alias = "appearance",
        alias = "keep",
        alias = "strict",
        alias = "exact"
    )]
    Preserve,
    #[serde(
        alias = "Adapt",
        alias = "style",
        alias = "outfit",
        alias = "wardrobe",
        alias = "partial"
    )]
    Adapt,
    #[serde(
        alias = "Transfer",
        alias = "motion",
        alias = "pose",
        alias = "camera",
        alias = "action"
    )]
    Transfer,
    #[serde(
        alias = "Context",
        alias = "scene",
        alias = "location",
        alias = "background",
        alias = "environment",
        alias = "prop",
        alias = "general"
    )]
    Context,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3AudioIntent {
    #[default]
    #[serde(
        alias = "VoiceIdentity",
        alias = "voice_identity",
        alias = "voice",
        alias = "Voice",
        alias = "speaker",
        alias = "vocal",
        alias = "speech",
        alias = "dialogue"
    )]
    VoiceIdentity,
    #[serde(
        alias = "SoundTexture",
        alias = "sound_texture",
        alias = "sound",
        alias = "Sound",
        alias = "sfx",
        alias = "music",
        alias = "ambience",
        alias = "audio"
    )]
    SoundTexture,
    #[serde(alias = "UseEmbedded", alias = "use_embedded", alias = "embedded")]
    UseEmbedded,
    #[serde(
        alias = "IgnoreEmbedded",
        alias = "ignore_embedded",
        alias = "ignore",
        alias = "mute",
        alias = "none"
    )]
    IgnoreEmbedded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3ReferenceVerdictKind {
    Winner,
    Superseded,
    BelowThreshold,
    Outside,
    CapExceeded,
    Muted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3VocalEventType {
    #[default]
    #[serde(alias = "Dialogue", alias = "speech", alias = "speaking", alias = "talk")]
    Dialogue,
    #[serde(alias = "Narration", alias = "voiceover", alias = "vo")]
    Narration,
    #[serde(alias = "Singing", alias = "song", alias = "vocal")]
    Singing,
    #[serde(alias = "Shout", alias = "yell", alias = "scream")]
    Shout,
    #[serde(alias = "Whisper", alias = "murmur")]
    Whisper,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3ClipSourceKind {
    #[default]
    Take,
    ReferenceAsset,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3CropRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3ReferenceMember {
    #[serde(default)]
    pub id: String,
    #[serde(alias = "asset_id")]
    pub asset_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub population: H3ReferencePopulation,
    #[serde(default)]
    pub role: H3ReferenceRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub crop: Option<H3CropRect>,
    #[serde(default, alias = "source_start_sec")]
    pub source_start_sec: f64,
    #[serde(default, alias = "source_end_sec")]
    pub source_end_sec: f64,
    #[serde(default, alias = "use_embedded_audio")]
    pub use_embedded_audio: bool,
    #[serde(default, alias = "description_override", alias = "description")]
    pub description_override: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3ReferenceEntity {
    #[serde(default)]
    pub id: String,
    /// Canonical handle token without leading `@` (e.g. `"Cowboy"`, written `@Cowboy` in prompts).
    #[serde(alias = "name")]
    pub handle: String,
    #[serde(default, alias = "display_name")]
    pub display_name: String,
    #[serde(default)]
    pub kind: H3EntityKind,
    #[serde(default, alias = "subject_description", alias = "description")]
    pub subject_description: String,
    #[serde(default, alias = "retention_level", alias = "retention")]
    pub retention_level: H3RetentionLevel,
    #[serde(default, alias = "default_visual_intent")]
    pub default_visual_intent: H3VisualIntent,
    #[serde(default, alias = "default_audio_intent")]
    pub default_audio_intent: H3AudioIntent,
    #[serde(default)]
    pub members: Vec<H3ReferenceMember>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct H3WindowConfig {
    #[serde(default)]
    pub in_sec: f64,
    #[serde(default = "default_out_sec")]
    pub out_sec: f64,
    #[serde(default)]
    pub pre_context_sec: f64,
    #[serde(default)]
    pub post_context_sec: f64,
    #[serde(default)]
    pub mask_start_offset_sec: f64,
    #[serde(default)]
    pub mask_end_offset_sec: f64,
    #[serde(default = "default_overlap_ratio")]
    pub min_overlap_ratio: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub spatial_mask_rect: Option<H3CropRect>,
}

fn default_out_sec() -> f64 {
    5.0
}

fn default_scene_duration_sec() -> f64 {
    15.0
}

fn default_overlap_ratio() -> f64 {
    0.5
}

impl Default for H3WindowConfig {
    fn default() -> Self {
        Self {
            in_sec: 0.0,
            out_sec: 5.0,
            pre_context_sec: 0.0,
            post_context_sec: 0.0,
            mask_start_offset_sec: 0.0,
            mask_end_offset_sec: 0.0,
            min_overlap_ratio: default_overlap_ratio(),
            spatial_mask_rect: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3LoraSpec {
    pub file_name: String,
    pub strength_model: f32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

fn default_unit_f32() -> f32 {
    1.0
}

fn default_shift_video() -> f32 {
    12.0
}

fn default_shift_audio() -> f32 {
    6.0
}

fn default_sampler() -> String {
    "er_sde".into()
}

fn default_scheduler() -> String {
    "sgm_uniform".into()
}

fn default_ref_size() -> String {
    "max".into()
}

fn default_steps_h3() -> u32 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3GenerationSettings {
    #[serde(default)]
    pub pass_preset: H3PassPreset,
    #[serde(default)]
    pub task_mode: H3TaskMode,
    #[serde(default)]
    pub prompt_format: H3PromptFormat,
    #[serde(default)]
    pub model_choice: H3ModelChoice,
    #[serde(default)]
    pub placement_mode: H3TakePlacementMode,
    #[serde(default = "default_custom_width")]
    pub custom_width: u32,
    #[serde(default = "default_custom_height")]
    pub custom_height: u32,
    #[serde(default = "default_steps_h3")]
    pub steps: u32,
    #[serde(default = "default_unit_f32")]
    pub cfg: f32,
    #[serde(default = "default_unit_f32")]
    pub denoise: f32,
    #[serde(default = "default_sampler")]
    pub sampler_name: String,
    #[serde(default = "default_scheduler")]
    pub scheduler: String,
    #[serde(default = "default_shift_video")]
    pub shift_video: f32,
    #[serde(default = "default_shift_audio")]
    pub shift_audio: f32,
    #[serde(default)]
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
    pub seed: u64,
    #[serde(default = "default_true")]
    pub randomize_seed: bool,
    #[serde(default = "default_ref_size")]
    pub ref_image_size: String,
    #[serde(default = "default_true")]
    pub color_drift_correction: bool,
    #[serde(default)]
    pub loras: Vec<H3LoraSpec>,
}

fn default_custom_width() -> u32 {
    768
}

fn default_custom_height() -> u32 {
    448
}

impl Default for H3GenerationSettings {
    fn default() -> Self {
        Self {
            pass_preset: H3PassPreset::Draft02Mp,
            task_mode: H3TaskMode::Standard,
            prompt_format: H3PromptFormat::H3FullReference,
            model_choice: H3ModelChoice::Auto,
            placement_mode: H3TakePlacementMode::Trimmed,
            custom_width: 768,
            custom_height: 448,
            steps: 10,
            cfg: 1.0,
            denoise: 1.0,
            sampler_name: default_sampler(),
            scheduler: default_scheduler(),
            shift_video: 12.0,
            shift_audio: 6.0,
            seed: 0,
            randomize_seed: true,
            ref_image_size: default_ref_size(),
            color_drift_correction: true,
            loras: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3VideoClip {
    pub id: String,
    #[serde(default)]
    pub source_kind: H3ClipSourceKind,
    /// `job_id` when `source_kind == Take`, or `asset_id` when `source_kind == ReferenceAsset`.
    pub source_id: String,
    #[serde(default)]
    pub label: String,
    pub timeline_start_sec: f64,
    pub duration_sec: f64,
    #[serde(default)]
    pub source_in_sec: f64,
    #[serde(default)]
    pub source_duration_sec: f64,
    #[serde(default = "default_unit_f32")]
    pub opacity: f32,
    #[serde(default = "default_unit_f32")]
    pub audio_gain: f32,
    #[serde(default = "default_true")]
    pub include_embedded_audio: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3VideoLane {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub clips: Vec<H3VideoClip>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3AudioClip {
    pub id: String,
    #[serde(default)]
    pub source_kind: H3ClipSourceKind,
    pub source_id: String,
    #[serde(default)]
    pub label: String,
    pub timeline_start_sec: f64,
    pub duration_sec: f64,
    #[serde(default)]
    pub source_in_sec: f64,
    #[serde(default)]
    pub source_duration_sec: f64,
    #[serde(default = "default_unit_f32")]
    pub gain: f32,
    #[serde(default)]
    pub fade_in_sec: f64,
    #[serde(default)]
    pub fade_out_sec: f64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3AudioLane {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub clips: Vec<H3AudioClip>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3DriverClip {
    pub id: String,
    pub asset_id: String,
    #[serde(default)]
    pub label: String,
    pub timeline_start_sec: f64,
    pub duration_sec: f64,
    #[serde(default)]
    pub source_in_sec: f64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3DriverLane {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub clips: Vec<H3DriverClip>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3StagedReferenceItem {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub member_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub asset_id: Option<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub role_override: Option<H3ReferenceRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub retention_override: Option<H3RetentionLevel>,
    #[serde(default)]
    pub description_override: String,
    pub timeline_start_sec: f64,
    pub duration_sec: f64,
    #[serde(default = "default_true")]
    pub use_visual: bool,
    #[serde(default = "default_true")]
    pub use_audio: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3ReferenceLane {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub items: Vec<H3StagedReferenceItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3GuideItem {
    pub id: String,
    pub asset_id: String,
    #[serde(default)]
    pub label: String,
    pub timeline_sec: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub end_sec: Option<f64>,
    #[serde(default = "default_true")]
    pub use_image: bool,
    #[serde(default)]
    pub use_audio: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3VocalEvent {
    #[serde(default)]
    pub id: String,
    #[serde(default, alias = "speaker_handle", alias = "speaker", alias = "handle")]
    pub speaker_handle: String,
    #[serde(default, alias = "event_type", alias = "kind", alias = "type")]
    pub event_type: H3VocalEventType,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub delivery: String,
    #[serde(default, alias = "line", alias = "dialogue")]
    pub text: String,
}

fn default_language() -> String {
    "English".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3SwapPromptFields {
    #[serde(default, alias = "target_subject")]
    pub target_subject: String,
    #[serde(default, alias = "replacement_handle")]
    pub replacement_handle: String,
    #[serde(default = "default_true", alias = "keep_pose_and_lighting")]
    pub keep_pose_and_lighting: bool,
    #[serde(default, alias = "extra_instruction")]
    pub extra_instruction: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3GlobalPrompt {
    #[serde(default, alias = "summary_lead", alias = "summary")]
    pub summary_lead: String,
    #[serde(default, alias = "detailed_lead", alias = "style")]
    pub detailed_lead: String,
    #[serde(default, alias = "overall_soundscape", alias = "soundscape")]
    pub overall_soundscape: String,
    #[serde(default, alias = "non_diegetic_music", alias = "music")]
    pub non_diegetic_music: String,
    #[serde(default, alias = "negative_prompt", alias = "negative")]
    pub negative_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3PromptSection {
    #[serde(default)]
    pub id: String,
    #[serde(default, alias = "title", alias = "name")]
    pub label: String,
    #[serde(
        default,
        alias = "timeline_start_sec",
        alias = "startSec",
        alias = "start_sec",
        alias = "inSec",
        alias = "in_sec"
    )]
    pub timeline_start_sec: f64,
    #[serde(
        default = "default_out_sec",
        alias = "duration_sec",
        alias = "duration",
        alias = "durationSeconds"
    )]
    pub duration_sec: f64,
    #[serde(default, alias = "summary_beat", alias = "summary", alias = "purpose")]
    pub summary_beat: String,
    #[serde(
        default,
        alias = "detailed_action",
        alias = "action",
        alias = "prompt",
        alias = "description"
    )]
    pub detailed_action: String,
    #[serde(default, alias = "camera_and_lighting", alias = "camera", alias = "lighting")]
    pub camera_and_lighting: String,
    #[serde(default, alias = "soundscape_override", alias = "soundscape", alias = "sfx")]
    pub soundscape_override: String,
    #[serde(default, alias = "music_override", alias = "music")]
    pub music_override: String,
    #[serde(default, alias = "vocal_events", alias = "vocals", alias = "dialogue")]
    pub vocal_events: Vec<H3VocalEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "swap_fields")]
    #[ts(optional)]
    pub swap_fields: Option<H3SwapPromptFields>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3Scene {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: H3SceneKind,
    #[serde(default = "default_custom_width")]
    pub width: u32,
    #[serde(default = "default_custom_height")]
    pub height: u32,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default = "default_scene_duration_sec")]
    pub duration_sec: f64,
    #[serde(default)]
    pub window: H3WindowConfig,
    #[serde(default)]
    pub generation: H3GenerationSettings,
    #[serde(default)]
    pub global_prompt: H3GlobalPrompt,
    #[serde(default)]
    pub prompt_sections: Vec<H3PromptSection>,
    #[serde(default)]
    pub video_lanes: Vec<H3VideoLane>,
    #[serde(default)]
    pub audio_lanes: Vec<H3AudioLane>,
    #[serde(default)]
    pub driver_lanes: Vec<H3DriverLane>,
    #[serde(default)]
    pub reference_lanes: Vec<H3ReferenceLane>,
    #[serde(default)]
    pub guides: Vec<H3GuideItem>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn default_fps() -> u32 {
    24
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3BoundOrdinalSlot {
    pub ordinal: u32,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub subject_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub speaker_code: Option<String>,
    pub population: H3ReferencePopulation,
    pub role: H3ReferenceRole,
    pub retention: H3RetentionLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub member_id: Option<String>,
    pub asset_id: String,
    pub label: String,
    pub description: String,
    pub has_embedded_audio: bool,
    pub use_embedded_audio: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub crop: Option<H3CropRect>,
    pub source_start_sec: f64,
    pub source_end_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3SubjectBinding {
    pub subject_index: u32,
    pub subject_tag: String,
    pub speaker_code: String,
    pub handle: String,
    pub display_name: String,
    pub entity_id: String,
    pub visual_tags: Vec<String>,
    pub audio_tags: Vec<String>,
    pub subject_description: String,
    pub retention: H3RetentionLevel,
    pub shot_indices: Vec<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3OrdinalManifest {
    pub pictures: Vec<H3BoundOrdinalSlot>,
    pub videos: Vec<H3BoundOrdinalSlot>,
    pub audios: Vec<H3BoundOrdinalSlot>,
    pub subjects: Vec<H3SubjectBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3ReferenceItemVerdict {
    pub item_id: String,
    pub lane_id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub handle: Option<String>,
    pub verdict: H3ReferenceVerdictKind,
    pub overlap_ratio: f64,
    pub assigned_tags: Vec<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3ResolvedGuide {
    pub guide_id: String,
    pub asset_id: String,
    pub label: String,
    pub frame_idx: i32,
    pub timeline_sec: f64,
    pub use_image: bool,
    pub use_audio: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3TakePlacementGeometry {
    pub policy: String,
    pub fps: u32,
    pub render_start_sec: f64,
    pub render_end_sec: f64,
    pub selection_in_sec: f64,
    pub selection_out_sec: f64,
    pub mask_start_frame: u32,
    pub mask_end_frame: u32,
    pub unpadded_frames: u32,
    pub padded_frames: u32,
    pub frame_count_padding: u32,
    pub video_latent_tokens: u32,
    pub video_mask_token_lo: u32,
    pub video_mask_token_hi: u32,
    pub audio_latent_tokens: u32,
    pub audio_mask_token_lo: u32,
    pub audio_mask_token_hi: u32,
    pub trimmed_timeline_start_sec: f64,
    pub trimmed_duration_sec: f64,
    pub trimmed_source_in_sec: f64,
    pub untrimmed_timeline_start_sec: f64,
    pub untrimmed_duration_sec: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3AdvisorySeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3CompileAdvisory {
    pub severity: H3AdvisorySeverity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3CompiledPromptChannels {
    pub subject_definitions: String,
    pub summary: String,
    pub retention_analysis: String,
    pub detailed_description: String,
    pub overall_soundscape: String,
    pub non_diegetic_music: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3WindowCompileResult {
    pub scene_id: String,
    pub scene_name: String,
    pub resolved_width: u32,
    pub resolved_height: u32,
    pub resolved_model: String,
    pub resolved_steps: u32,
    pub resolved_denoise: f32,
    pub uses_source_av_latent: bool,
    pub uses_video_noise_mask: bool,
    pub uses_audio_noise_mask: bool,
    pub resolved_loras: Vec<H3LoraSpec>,
    pub prompt_format: H3PromptFormat,
    pub compiled_prompt: String,
    pub negative_prompt: String,
    pub channels: H3CompiledPromptChannels,
    pub ordinals: H3OrdinalManifest,
    pub verdicts: Vec<H3ReferenceItemVerdict>,
    pub guides: Vec<H3ResolvedGuide>,
    pub geometry: H3TakePlacementGeometry,
    pub advisories: Vec<H3CompileAdvisory>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3ColorDriftReport {
    pub applied: bool,
    pub matched_context_frames: u32,
    pub max_bias_abs: f32,
    pub max_matrix_deviation: f32,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3TakeQualityReport {
    pub verified: bool,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub frame_count: u32,
    pub duration_sec: f64,
    pub has_audio: bool,
    pub audio_peak_dbfs: f32,
    pub audio_rms_dbfs: f32,
    pub audio_is_silent: bool,
    pub contact_sheet_paths: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum H3RenderJobStatus {
    Queued,
    Preparing,
    Rendering,
    PostProcessing,
    Complete,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3RenderQueueJob {
    pub id: String,
    pub project_id: String,
    pub scene_id: String,
    pub scene_name: String,
    pub label: String,
    pub status: H3RenderJobStatus,
    pub pass_preset: H3PassPreset,
    pub task_mode: H3TaskMode,
    pub placement_mode: H3TakePlacementMode,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub denoise: f32,
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
    pub seed: u64,
    pub compiled_prompt: String,
    pub resolved_model: String,
    pub geometry: H3TakePlacementGeometry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub output_clip_id: Option<String>,
    #[serde(default)]
    pub take_media_path: String,
    #[serde(default)]
    pub take_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color_drift: Option<H3ColorDriftReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub quality: Option<H3TakeQualityReport>,
    #[serde(default)]
    pub error: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3DiscoveredLora {
    pub file_name: String,
    pub bytes: u64,
    pub recommended_role: String,
    pub default_strength: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3EnvironmentSnapshot {
    pub ready: bool,
    pub comfy_root: String,
    pub has_ref2va: bool,
    pub has_fl2va: bool,
    pub has_qwen3vl: bool,
    pub has_video_vae: bool,
    pub has_audio_vae: bool,
    pub has_taeh3_preview: bool,
    pub has_ref2v_turbo_lora: bool,
    pub has_fl2v_turbo_lora: bool,
    pub discovered_loras: Vec<H3DiscoveredLora>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3TimeSpan {
    pub start_sec: f64,
    pub end_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3EntityDigest {
    pub id: String,
    pub handle: String,
    pub kind: H3EntityKind,
    pub picture_count: usize,
    pub video_count: usize,
    pub audio_count: usize,
    pub has_description: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3SceneDigest {
    pub id: String,
    pub name: String,
    pub kind: H3SceneKind,
    pub width: u32,
    pub height: u32,
    pub duration_sec: f64,
    pub window_in_sec: f64,
    pub window_out_sec: f64,
    pub pass_preset: H3PassPreset,
    pub task_mode: H3TaskMode,
    pub video_clip_count: usize,
    pub audio_clip_count: usize,
    pub staged_reference_count: usize,
    pub guide_count: usize,
    pub prompt_section_count: usize,
    pub covered_video_spans: Vec<H3TimeSpan>,
    pub uncovered_video_gaps: Vec<H3TimeSpan>,
    pub advisory_error_count: usize,
    pub advisory_warning_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3JobDigest {
    pub id: String,
    pub scene_id: String,
    pub label: String,
    pub status: H3RenderJobStatus,
    pub pass_preset: H3PassPreset,
    pub duration_sec: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub audio_peak_dbfs: Option<f32>,
    pub verified: bool,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3ProductionDigest {
    pub project_id: String,
    pub title: String,
    pub status: String,
    pub phase: String,
    pub active_scene_id: String,
    pub story_excerpt: String,
    pub reference_asset_count: usize,
    pub entities: Vec<H3EntityDigest>,
    pub scenes: Vec<H3SceneDigest>,
    pub recent_jobs: Vec<H3JobDigest>,
    pub export_count: usize,
    pub final_export_ready: bool,
    pub next_actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3TakeInspection {
    pub job_id: String,
    pub project_id: String,
    pub scene_id: String,
    pub label: String,
    pub status: H3RenderJobStatus,
    pub pass_preset: H3PassPreset,
    pub task_mode: H3TaskMode,
    pub resolved_model: String,
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
    pub seed: u64,
    pub compiled_prompt: String,
    pub take_media_path: String,
    pub take_sha256: String,
    pub geometry: H3TakePlacementGeometry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color_drift: Option<H3ColorDriftReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub quality: Option<H3TakeQualityReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub provenance_summary: Option<serde_json::Value>,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateMovieH3ProjectRequest {
    pub title: String,
    #[serde(default)]
    pub story_markdown: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub duration_sec: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveMovieH3StoryRequest {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub title: Option<String>,
    pub story_markdown: String,
    #[serde(default)]
    pub instruction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpsertMovieH3EntityRequest {
    pub project_id: String,
    pub entity: H3ReferenceEntity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeleteMovieH3EntityRequest {
    pub project_id: String,
    pub entity_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateMovieH3SceneRequest {
    pub project_id: String,
    pub name: String,
    #[serde(default)]
    pub kind: H3SceneKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub duration_sec: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveMovieH3SceneRequest {
    pub project_id: String,
    pub scene: H3Scene,
    #[serde(default = "default_true")]
    pub set_active: bool,
    #[serde(default)]
    pub replace_empty_collections: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StageMovieH3ReferenceRequest {
    #[serde(alias = "project_id")]
    pub project_id: String,
    #[serde(default, alias = "scene_id")]
    pub scene_id: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "entity_id",
        alias = "handle"
    )]
    #[ts(optional = nullable)]
    pub entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "member_id")]
    #[ts(optional = nullable)]
    pub member_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "asset_id")]
    #[ts(optional = nullable)]
    pub asset_id: Option<String>,
    #[serde(default, alias = "start_sec", alias = "timelineStartSec", alias = "inSec")]
    pub start_sec: f64,
    #[serde(
        default = "default_scene_duration_sec",
        alias = "end_sec",
        alias = "outSec"
    )]
    pub end_sec: f64,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "role_override")]
    #[ts(optional = nullable)]
    pub role_override: Option<H3ReferenceRole>,
    #[serde(default = "default_true", alias = "use_visual")]
    pub use_visual: bool,
    #[serde(default = "default_true", alias = "use_audio")]
    pub use_audio: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "lane_id")]
    #[ts(optional = nullable)]
    pub lane_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3StageReferenceReceipt {
    pub project_id: String,
    pub scene_id: String,
    pub lane_id: String,
    pub lane_name: String,
    pub item_id: String,
    pub label: String,
    pub start_sec: f64,
    pub end_sec: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct UpsertMovieH3PromptSectionRequest {
    #[serde(alias = "project_id")]
    pub project_id: String,
    #[serde(default, alias = "scene_id")]
    pub scene_id: String,
    #[serde(alias = "shot")]
    pub section: H3PromptSection,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "global_prompt")]
    #[ts(optional = nullable)]
    pub global_prompt: Option<H3GlobalPrompt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3UpsertPromptSectionReceipt {
    pub project_id: String,
    pub scene_id: String,
    pub section_id: String,
    pub shot_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CaptureMovieH3FrameRequest {
    #[serde(alias = "project_id")]
    pub project_id: String,
    #[serde(default, alias = "scene_id")]
    pub scene_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "source_job_id")]
    #[ts(optional = nullable)]
    pub source_job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "source_asset_id")]
    #[ts(optional = nullable)]
    pub source_asset_id: Option<String>,
    #[serde(default, alias = "time_sec")]
    pub time_sec: f64,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "add_guide_at_sec")]
    #[ts(optional = nullable)]
    pub add_guide_at_sec: Option<f64>,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CompileMovieH3WindowRequest {
    #[serde(alias = "project_id")]
    pub project_id: String,
    #[serde(default, alias = "scene_id")]
    pub scene_id: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "window_override",
        alias = "window"
    )]
    #[ts(optional = nullable)]
    pub window_override: Option<H3WindowConfig>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "generation_override",
        alias = "generation",
        alias = "settings"
    )]
    #[ts(optional = nullable)]
    pub generation_override: Option<H3GenerationSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3QueuedWindowSpec {
    #[serde(default, alias = "scene_id")]
    pub scene_id: String,
    #[serde(default)]
    pub label: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "window_override",
        alias = "window"
    )]
    #[ts(optional = nullable)]
    pub window_override: Option<H3WindowConfig>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "generation_override",
        alias = "generation",
        alias = "settings"
    )]
    #[ts(optional = nullable)]
    pub generation_override: Option<H3GenerationSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3StartGenerationRequest {
    #[serde(alias = "project_id")]
    pub project_id: String,
    #[serde(default)]
    pub windows: Vec<H3QueuedWindowSpec>,
    /// When true, expands each requested window into Sonder's 3-pass ladder:
    /// Pass 1 (`Draft02Mp`, `denoise = 1.0`) -> Pass 2 (`Detail1Mp`, `denoise = 0.75`) -> Pass 3 (`Upscale1080p`, `denoise = 0.40`),
    /// placing each pass onto the timeline before the next pass refines it.
    #[serde(default, alias = "expand_quality_ladder")]
    pub expand_quality_ladder: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct H3GenerationStartReceipt {
    pub project_id: String,
    pub job_ids: Vec<String>,
    pub queued_count: usize,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PlaceMovieH3TakeRequest {
    #[serde(alias = "project_id")]
    pub project_id: String,
    #[serde(default, alias = "job_id")]
    pub job_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "asset_id")]
    #[ts(optional = nullable)]
    pub asset_id: Option<String>,
    #[serde(default, alias = "target_scene_id", alias = "sceneId", alias = "scene_id")]
    pub target_scene_id: String,
    #[serde(default, alias = "placement_mode")]
    pub placement_mode: H3TakePlacementMode,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "target_video_lane_id"
    )]
    #[ts(optional = nullable)]
    pub target_video_lane_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "target_audio_lane_id"
    )]
    #[ts(optional = nullable)]
    pub target_audio_lane_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "target_driver_lane_id"
    )]
    #[ts(optional = nullable)]
    pub target_driver_lane_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "timeline_start_override_sec",
        alias = "startSec"
    )]
    #[ts(optional = nullable)]
    pub timeline_start_override_sec: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "duration_override_sec",
        alias = "durationSec"
    )]
    #[ts(optional = nullable)]
    pub duration_override_sec: Option<f64>,
    /// Mix gain for the placed clip (0.0–2.0). Audio clips use it as clip gain; video clips use
    /// it for their embedded audio. Omit for the defaults (1.0, or 0.65 for score lane A2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub gain: Option<f32>,
    /// Audio fade-in seconds for a placed audio clip; ignored for video clips.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "fade_in_sec")]
    #[ts(optional = nullable)]
    pub fade_in_sec: Option<f64>,
    /// Audio fade-out seconds for a placed audio clip; ignored for video clips.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "fade_out_sec")]
    #[ts(optional = nullable)]
    pub fade_out_sec: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RenderMovieExportRequest {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub scene_id: Option<String>,
    #[serde(default = "default_publish_preset")]
    pub preset: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub peak_headroom_db: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional = nullable)]
    pub destination_path: Option<String>,
}

fn default_publish_preset() -> String {
    "publish".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieExportReceipt {
    pub project_id: String,
    pub scene_id: String,
    pub export: crate::MovieExport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub destination_copy_path: Option<String>,
}
