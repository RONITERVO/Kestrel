// Rust-owned wire and durable data. Native services retain execution authority.
use crate::MusicMidiDocument;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicSettings {
    pub max_duration_seconds: f64,
    pub steps: u32,
    pub cfg_scale: f64,
    pub top_k: u32,
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
    pub seed: u64,
    pub tiled_decode: bool,
    pub model_variant: String,
    pub comfy_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicSection {
    pub id: String,
    pub tag: String,
    pub name: String,
    pub bars: u32,
    pub lyrics: String,
    pub direction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiSettings {
    pub executable_path: String,
    pub model_path: String,
    pub instruments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
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
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicSummary {
    pub id: String,
    pub title: String,
    pub status: String,
    pub updated_at: String,
    pub take_count: usize,
    pub active_take_path: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiRequest {
    pub project_id: String,
    pub take_id: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SaveMusicMidiDocumentRequest {
    pub project_id: String,
    pub take_id: String,
    pub document: MusicMidiDocument,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiSaveResult {
    pub project: MusicProject,
    pub document: MusicMidiDocument,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicLyricWord {
    pub value: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
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
    #[serde(default)]
    pub translation_language: String,
    #[serde(default)]
    pub translation_model_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub segments: Vec<MusicLyricSegment>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicLyricsRequest {
    pub project_id: String,
    pub take_id: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TranscribeMusicLyricsRequest {
    pub project_id: String,
    pub take_id: String,
    pub job_id: String,
    pub model_id: String,
    #[serde(default = "default_lyrics_language")]
    pub language: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DraftLyricsFromAudioRangeRequest {
    pub project_id: String,
    pub take_id: String,
    pub model_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DraftLyricsFromAudioRangeResult {
    pub transcription: String,
    pub model_id: String,
    pub model_name: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TranslateMusicLyricsRequest {
    pub project_id: String,
    pub take_id: String,
    pub model_id: String,
    pub target_language: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TranslateMusicLyricsResult {
    pub translations: Vec<String>,
    pub model_id: String,
    pub model_name: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SaveMusicLyricsDocumentRequest {
    pub project_id: String,
    pub take_id: String,
    pub document: MusicLyricsDocument,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicLyricsSaveResult {
    pub project: MusicProject,
    pub document: MusicLyricsDocument,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicGenerationEvent {
    pub project_id: String,
    pub take_id: String,
    pub kind: String,
    pub phase: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub step: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub total: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub eta_seconds: Option<u64>,
    pub at: String,
}

pub fn default_lyrics_language() -> String {
    "auto".into()
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

/// Producer-editable fields. Saved takes, receipts, status and timestamps stay native-owned.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MusicProjectEdit {
    pub id: String,
    pub title: String,
    pub idea: String,
    pub caption: String,
    pub instrumental: bool,
    pub sections: Vec<MusicSection>,
    pub settings: MusicSettings,
    pub midi: MusicMidiSettings,
    pub active_take_id: String,
}

impl From<MusicProject> for MusicProjectEdit {
    fn from(project: MusicProject) -> Self {
        Self {
            id: project.id,
            title: project.title,
            idea: project.idea,
            caption: project.caption,
            instrumental: project.instrumental,
            sections: project.sections,
            settings: project.settings,
            midi: project.midi,
            active_take_id: project.active_take_id,
        }
    }
}
