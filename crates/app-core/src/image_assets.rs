// Rust-owned wire and durable data. Native services retain execution authority.
use crate::MovieReferenceAsset;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MovieImageAssetRequest {
    pub request_id: String,
    pub prompt: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    #[serde(default)]
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
    pub seed: u64,
    pub comfy_root: String,
    #[serde(default = "default_stabilize")]
    pub stabilize: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImageProvenance {
    pub generation_id: String,
    pub workflow: String,
    pub workflow_source: String,
    pub workflow_revision: String,
    pub prompt: String,
    pub rendered_prompt: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
    pub seed: u64,
    pub requested_length: u32,
    pub resolved_frame_count: u32,
    pub frame_index: u32,
    pub sampler: String,
    pub scheduler: String,
    pub diffusion_model: String,
    pub text_encoder: String,
    pub vae: String,
    pub comfy_prompt_id: String,
    pub created_at: String,
    #[serde(default)]
    pub exact_graph: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MovieImageAssetCandidate {
    pub frame_index: u32,
    pub asset: MovieReferenceAsset,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MovieImageAssetGeneration {
    pub id: String,
    pub status: String,
    pub stage: String,
    pub detail: String,
    pub prompt: String,
    pub rendered_prompt: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    #[serde(with = "crate::seed_serde")]
    #[ts(type = "number | string")]
    pub seed: u64,
    pub stabilize: bool,
    pub workflow: String,
    pub workflow_source: String,
    pub workflow_revision: String,
    #[serde(default = "legacy_preview_provenance")]
    pub preview_node_revision: String,
    #[serde(default = "legacy_preview_provenance")]
    pub preview_decoder_revision: String,
    #[serde(default = "legacy_preview_provenance")]
    pub preview_decoder_sha256: String,
    pub requested_length: u32,
    pub resolved_frame_count: u32,
    pub candidate_start: u32,
    pub candidate_count: u32,
    pub comfy_prompt_id: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub completed_at: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub candidates: Vec<MovieImageAssetCandidate>,
    #[serde(default)]
    pub exact_graph: Value,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MovieImageAssetEvent {
    pub request_id: String,
    pub kind: String,
    pub stage: String,
    pub detail: String,
    pub progress: u8,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub generation: Option<MovieImageAssetGeneration>,
}

pub fn default_stabilize() -> bool {
    true
}

pub fn legacy_preview_provenance() -> String {
    "unavailable (legacy generation)".into()
}
