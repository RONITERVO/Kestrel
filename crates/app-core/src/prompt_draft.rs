// Rust-owned wire and durable data. Native services retain execution authority.
use crate::ThinkingLevel;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum PromptDraftTarget {
    Story,
    ImageAsset,
    ImageComposition,
    ReferenceDescription,
    MusicCaption,
    MusicLyrics,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum PromptDraftMode {
    Develop,
    Continue,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PromptDraftRequest {
    pub request_id: String,
    pub model_id: String,
    pub target: PromptDraftTarget,
    pub mode: PromptDraftMode,
    #[serde(default)]
    pub story_text: String,
    #[serde(default)]
    pub existing_text: String,
    #[serde(default)]
    pub asset_name: String,
    #[serde(default)]
    pub asset_kind: String,
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PromptDraftReceipt {
    pub target: PromptDraftTarget,
    pub mode: PromptDraftMode,
    pub model_id: String,
    pub messages: Vec<Value>,
    pub temperature: f64,
    pub top_p: f64,
    pub top_k: u32,
    pub max_tokens: u32,
    pub exact_request: Value,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PromptDraftEvent {
    pub request_id: String,
    pub kind: String,
    pub content: Option<String>,
    pub model_name: Option<String>,
    pub thinking_level: Option<ThinkingLevel>,
    pub receipt: Option<PromptDraftReceipt>,
    pub at: String,
}
