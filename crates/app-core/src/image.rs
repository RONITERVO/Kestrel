// Rust-owned wire and durable data. Native services retain execution authority.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ImageSettings {
    pub width: u32,
    pub height: u32,
    pub preset: String,
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
    pub seed: u64,
    #[serde(default = "default_batch_size")]
    pub batch_size: u32,
    pub comfy_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
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
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ImageSummary {
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
pub struct CreateImageProjectRequest {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub idea: String,
    #[serde(default)]
    pub comfy_root: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ImageGenerationEvent {
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

pub fn default_art_style() -> String {
    "Editorial illustration with purposeful shape language and finished detail.".into()
}

pub const fn default_batch_size() -> u32 {
    1
}

pub fn default_style_mode() -> String {
    "photo".into()
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

/// Producer-editable fields. Saved takes, receipts, status and timestamps stay native-owned.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ImageProjectEdit {
    pub id: String,
    pub title: String,
    pub idea: String,
    pub high_level_description: String,
    pub style: ImageStyle,
    pub background: String,
    pub elements: Vec<ImageElement>,
    pub settings: ImageSettings,
    pub active_take_id: String,
}

impl From<ImageProject> for ImageProjectEdit {
    fn from(project: ImageProject) -> Self {
        Self {
            id: project.id,
            title: project.title,
            idea: project.idea,
            high_level_description: project.high_level_description,
            style: project.style,
            background: project.background,
            elements: project.elements,
            settings: project.settings,
            active_take_id: project.active_take_id,
        }
    }
}
