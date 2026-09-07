use serde::Serialize;
use ts_rs::TS;

/// Transient progress; the owning native service controls the operation.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OperationProgress {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub phase: Option<String>,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub at: Option<String>,
}
