use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MovieEditorPlacement {
    Masters,
    ReplaceRange,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MovieEditorJobStatus {
    Preparing,
    Ready,
    Writing,
    Rendering,
    Complete,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieEditorRangeRequest {
    pub project_id: String,
    pub expected_edit_hash: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub duration_seconds: f32,
    pub direction: String,
    pub placement: MovieEditorPlacement,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieEditorGenerateRequest {
    pub project_id: String,
    pub job_id: String,
    /// Empty means use the producer's supplied render prompt without inference.
    pub model_id: String,
    pub render_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieEditorEndpoint {
    pub edit_id: String,
    pub clip_id: String,
    pub version_id: String,
    pub source_path: String,
    pub source_sha256: String,
    pub source_seconds: f64,
    pub image_path: String,
    pub image_sha256: String,
    pub scene_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieEditorJob {
    pub id: String,
    pub project_id: String,
    pub status: MovieEditorJobStatus,
    pub detail: String,
    pub created_at: String,
    pub updated_at: String,
    pub edit_hash: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub duration_seconds: f32,
    pub placement: MovieEditorPlacement,
    pub direction: String,
    pub render_prompt: String,
    pub model_id: String,
    pub seed: u64,
    pub first: MovieEditorEndpoint,
    pub last: MovieEditorEndpoint,
    pub clip_id: String,
    pub result_path: String,
    pub result_sha256: String,
    #[serde(default)]
    pub raw_path: String,
    #[serde(default)]
    pub raw_sha256: String,
    pub placed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieEditorState {
    pub edit_hash: String,
    pub jobs: Vec<MovieEditorJob>,
}
