// Rust-owned wire and durable data. Native services retain execution authority.
use crate::GpuSnapshot;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GpuMemoryProcess {
    pub pid: u32,
    pub name: String,
    pub executable_path: String,
    pub memory_mib: u64,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct VramCleanupPreview {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gpu: Option<GpuSnapshot>,
    pub candidates: Vec<GpuMemoryProcess>,
    pub exclusions: Vec<GpuMemoryExclusion>,
    pub candidate_memory_mib: u64,
    pub protected_process_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GpuMemoryExclusion {
    pub process: GpuMemoryProcess,
    pub reason: String,
    pub can_include: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct GpuCleanupFailure {
    pub process: GpuMemoryProcess,
    pub detail: String,
    pub can_force_close: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub powershell_command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct VramCleanupResult {
    pub attempted: Vec<GpuMemoryProcess>,
    pub terminated: Vec<GpuMemoryProcess>,
    pub failed: Vec<GpuCleanupFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub before_gpu: Option<GpuSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub after_gpu: Option<GpuSnapshot>,
    pub freed_mib: u64,
    pub message: String,
}
