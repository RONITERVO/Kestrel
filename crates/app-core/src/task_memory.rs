//! Durable producer memory is independent of the selected model and delivery transport.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct TaskCheckpoint {
    /// Agent-authored working summary, never a replacement for original user instructions.
    pub summary: String,
    pub plan: Vec<TaskPlanItem>,
    pub decisions: Vec<String>,
    pub references: Vec<String>,
    pub next_steps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct TaskPlanItem {
    pub id: String,
    pub description: String,
    pub status: TaskPlanStatus,
    /// One-based native journal sequence numbers supporting the claimed result.
    pub evidence: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TaskPlanStatus {
    Pending,
    InProgress,
    Done,
    Blocked,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct TaskCheckpointUpdate {
    /// Compare-and-swap revision obtained from get_task_memory or task_memory.
    pub expected_revision: u32,
    pub checkpoint: TaskCheckpoint,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskMemorySnapshot {
    pub task_id: String,
    pub revision: u32,
    pub checkpoint: Option<TaskCheckpoint>,
    pub event_count: u32,
    pub direction_sequences: Vec<u32>,
    /// Started actions without a recorded result. Inspect these before further mutation.
    pub unsettled_actions: Vec<u32>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct TaskHistoryQuery {
    #[serde(default)]
    pub after: u32,
    pub kind: Option<String>,
    pub query: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskHistoryEntry {
    pub sequence: u32,
    pub kind: String,
    pub title: String,
    pub preview: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskHistoryPage {
    pub entries: Vec<TaskHistoryEntry>,
    pub next_after: Option<u32>,
    pub event_count: u32,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskRecordPage {
    pub sequence: u32,
    pub text: String,
    pub next_offset: Option<u32>,
    pub total_chars: u32,
}
