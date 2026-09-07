// Rust-owned wire and durable data. Native services retain execution authority.
use crate::VoiceProfile;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SpeechModel {
    pub id: String,
    pub name: String,
    pub provider: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSnapshot {
    pub narration_available: bool,
    pub transcription_available: bool,
    pub comfy_ready: bool,
    pub voices: Vec<SpeechModel>,
    pub transcribers: Vec<SpeechModel>,
    pub voice_profiles: Vec<VoiceProfile>,
    pub default_voice_profile_id: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SpeechSynthesisRequest {
    pub job_id: String,
    pub source_kind: String,
    pub source_id: String,
    pub passage_id: String,
    pub text: String,
    pub model_id: String,
    #[serde(default = "default_voice_profile_id")]
    pub voice_profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SpeechAlignmentRequest {
    pub job_id: String,
    pub source_kind: String,
    pub source_id: String,
    pub passage_id: String,
    pub text: String,
    pub relative_path: String,
    pub voice_model_id: String,
    #[serde(default = "default_voice_profile_id")]
    pub voice_profile_id: String,
    pub alignment_model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTiming {
    pub value: String,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SpeechClip {
    pub job_id: String,
    pub passage_id: String,
    pub relative_path: String,
    pub model_id: String,
    pub voice_profile_id: String,
    pub cache_hit: bool,
    pub segments: Vec<SpeechTiming>,
    pub words: Vec<SpeechTiming>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTranscriptionRequest {
    pub job_id: String,
    pub source_kind: String,
    pub source_id: String,
    pub recording_id: String,
    pub audio_base64: String,
    pub mime_type: String,
    pub model_id: String,
    #[serde(default = "default_transcription_language")]
    pub language: String,
    #[serde(default)]
    pub prompt: String,
    pub final_pass: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SpeechTranscription {
    pub job_id: String,
    pub recording_id: String,
    pub text: String,
    pub segments: Vec<SpeechTiming>,
    pub words: Vec<SpeechTiming>,
    pub audio_relative_path: Option<String>,
    pub final_pass: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SpeechProgress {
    pub job_id: String,
    pub passage_id: String,
    pub stage: String,
    pub detail: String,
}

pub fn default_transcription_language() -> String {
    "auto".into()
}

pub fn default_voice_profile_id() -> String {
    "voice-default".into()
}
