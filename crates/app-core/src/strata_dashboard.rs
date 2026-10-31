//! Read-only observations of the currently owned Strata runtime. Never includes credentials.
use crate::ManagedRuntimeSnapshot;
use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StrataDashboardSnapshot {
    pub runtime: ManagedRuntimeSnapshot,
    pub available: bool,
    pub detail: String,
    pub activity: Option<StrataDashboardActivity>,
    pub hardware: Option<StrataDashboardHardware>,
    pub requests: Vec<StrataDashboardRequest>,
    pub request_capture: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StrataDashboardActivity {
    pub model: String,
    pub context_window: u32,
    pub images: bool,
    pub phase: String,
    pub queued: u32,
    pub prompt_tokens: Option<f64>,
    pub prompt_read: Option<f64>,
    pub prompt_total: Option<f64>,
    pub generated_tokens: Option<f64>,
    pub max_output_tokens: Option<f64>,
    pub elapsed_seconds: Option<f64>,
    pub tokens_per_second: Option<f64>,
    pub prefill_tokens_per_second: Option<f64>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StrataDashboardHardware {
    pub gpu_name: String,
    pub gpu_percent: Option<f64>,
    pub gpu_used_mib: Option<f64>,
    pub gpu_total_mib: Option<f64>,
    pub gpu_temperature: Option<f64>,
    pub gpu_watts: Option<f64>,
    pub cpu_percent: Option<f64>,
    pub ram_used_gib: Option<f64>,
    pub ram_total_gib: Option<f64>,
    pub gpu_history: Vec<Option<f64>>,
    pub cpu_history: Vec<Option<f64>>,
    pub ram_percent_history: Vec<Option<f64>>,
    pub output_speed_history: Vec<Option<f64>>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StrataDashboardRequest {
    pub id: String,
    pub path: String,
    pub state: String,
    pub started_at: Option<f64>,
    pub wall_seconds: Option<f64>,
    pub queue_seconds: Option<f64>,
    pub load_seconds: Option<f64>,
    pub first_token_seconds: Option<f64>,
    pub prompt_tokens: Option<f64>,
    pub reused_tokens: Option<f64>,
    pub output_tokens: Option<f64>,
    pub prefill_tokens_per_second: Option<f64>,
    pub output_tokens_per_second: Option<f64>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StrataDashboardRequestDetail {
    pub request: StrataDashboardRequest,
    pub input: String,
    pub output: String,
    pub reasoning: String,
    pub input_truncated: bool,
    pub output_truncated: bool,
    pub reasoning_truncated: bool,
    pub error: Option<String>,
}
