// Rust-owned wire and durable data. Native services retain execution authority.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProfile {
    pub id: String,
    pub name: String,
    pub language: String,
    pub tags: Vec<String>,
    pub source: String,
    pub consent_confirmed: bool,
    pub performance: String,
    pub reference_relative_path: Option<String>,
    pub reference_sha256: Option<String>,
    pub reference_seconds: Option<f64>,
    pub original_file_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct VoiceLibrarySnapshot {
    pub profiles: Vec<VoiceProfile>,
    pub default_profile_id: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CreateVoiceProfileRequest {
    pub name: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source: String,
    pub consent_confirmed: bool,
    #[serde(default = "default_performance")]
    pub performance: String,
    pub audio_base64: String,
    pub mime_type: String,
    pub original_file_name: String,
    pub duration_seconds: f64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct UpdateVoiceProfileRequest {
    pub id: String,
    pub name: String,
    pub language: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub consent_confirmed: bool,
    pub performance: String,
}

pub fn default_language() -> String {
    "Auto".into()
}

pub fn default_performance() -> String {
    "natural".into()
}

impl VoiceProfile {
    pub fn built_in() -> Self {
        Self {
            id: DEFAULT_VOICE_ID.into(),
            name: "Chatterbox Default".into(),
            language: "Auto".into(),
            tags: vec!["Built in".into(), "Neutral".into()],
            source: "built-in".into(),
            consent_confirmed: true,
            performance: "natural".into(),
            reference_relative_path: None,
            reference_sha256: None,
            reference_seconds: None,
            original_file_name: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    pub fn is_built_in(&self) -> bool {
        self.id == DEFAULT_VOICE_ID
    }
}

pub const DEFAULT_VOICE_ID: &str = "voice-default";
