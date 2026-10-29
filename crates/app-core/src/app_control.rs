//! Agent-neutral producer protocol. Authority and execution remain native.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct AppActionRequest {
    /// Unique for an intentional action; reuse unchanged to retrieve a prior receipt.
    pub request_id: String,
    pub command: String,
    #[serde(default = "empty_args")]
    pub args: Value,
}
fn empty_args() -> Value {
    serde_json::json!({})
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppActionReceipt {
    pub request: AppActionRequest,
    /// running, completed, failed, or interrupted. Completed means the command returned;
    /// background generation must still be inspected through its native project/job state.
    pub status: String,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppActionDescription {
    pub command: String,
    pub access: String,
    pub guidance: String,
    pub arguments: Value,
    /// Native successful result type, including non-text scalar/structured results.
    pub result_type: String,
    /// Rust-generated TypeScript declarations, including transitive dependencies.
    pub types: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppJobObservation {
    pub id: String,
    pub event: String,
    pub status: String,
    pub text: String,
    pub latest: Value,
    #[serde(default)]
    pub provenance: Value,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ProducerWorkflowStep {
    pub id: String,
    pub command: String,
    /// {"$from":"earlier-step-id","pointer":"/id"} substitutes a prior result.
    #[serde(default = "empty_args")]
    pub args: Value,
    /// Optional native observation to poll after this command (for example get_image_project).
    /// On success this observation becomes the step's result for subsequent $from references.
    pub wait: Option<ProducerWorkflowWait>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ProducerWorkflowWait {
    pub command: String,
    pub args: Value,
    pub pointer: String,
    pub equals: Value,
    /// Explicit bounded time budget, 1–43,200 seconds.
    pub timeout_seconds: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ProducerWorkflowPlan {
    pub title: String,
    pub steps: Vec<ProducerWorkflowStep>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProducerWorkflowRun {
    pub id: String,
    pub plan: ProducerWorkflowPlan,
    /// A plan created by a local task retains its workspace boundary after restart.
    #[serde(default)]
    pub workspace_scope: Option<Vec<String>>,
    pub status: String,
    pub next_step: usize,
    pub attempt: u32,
    pub results: std::collections::BTreeMap<String, Value>,
    pub error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProducerControlSession {
    pub pipe: String,
    pub token: String,
    pub protocol_version: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProducerTaskStatus {
    pub id: String,
    pub status: String,
    pub step: u32,
    pub title: String,
    pub detail: String,
    pub question: Option<Value>,
    pub artifact_paths: Vec<String>,
    pub event_count: usize,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProducerEnvironment {
    pub selected_model: Option<crate::ModelInfo>,
    pub library_directory: String,
    pub model_download_directory: String,
    pub workspace_directories: Vec<String>,
    pub tools: Vec<ProducerToolPath>,
    /// Effective native defaults, keyed by generation capability. Future providers add entries.
    pub generation_defaults: std::collections::BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProducerToolPath {
    pub name: String,
    pub path: String,
    /// Filesystem presence only; use Setup's snapshot for component readiness.
    pub path_exists: bool,
}
