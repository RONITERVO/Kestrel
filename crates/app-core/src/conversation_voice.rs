//! Live speech is an input/output adapter. It never selects or leases a language model.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ConversationVoiceSnapshot {
    pub installed: bool,
    pub running: bool,
    pub process_id: Option<u32>,
    pub languages: Vec<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSpeakRequest {
    pub job_id: String,
    pub conversation_id: String,
    pub response_id: String,
    pub text: String,
    pub language: String,
    pub voice_profile_id: String,
}

/// PCM is mono signed 16-bit little endian at sample_rate. Sequence starts at zero per job.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ConversationAudio {
    pub job_id: String,
    pub conversation_id: String,
    pub response_id: String,
    pub sequence: u32,
    pub sample_rate: u32,
    pub pcm_base64: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTranscribeRequest {
    pub job_id: String,
    /// Mono signed 16-bit little endian, 16 kHz; at most 30 seconds.
    pub pcm_base64: String,
    /// "auto" or a supported ISO 639-1 language.
    pub language: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ConversationTranscript {
    pub job_id: String,
    pub text: String,
    pub language: String,
    pub words: Vec<crate::speech::SpeechTiming>,
}

/// Provisional presentation text only; tool JSON and reasoning never enter this event.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TaskResponseDelta {
    pub run_id: String,
    pub response_id: String,
    pub step: u32,
    pub kind: String,
    pub text: String,
}
