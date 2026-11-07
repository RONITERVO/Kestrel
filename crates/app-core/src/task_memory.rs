//! Durable producer memory is independent of the selected model and delivery transport.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskCheckpoint {
    /// Agent-authored working summary, never a replacement for original user instructions.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub plan: Vec<TaskPlanItem>,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub next_steps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskPlanItem {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub status: TaskPlanStatus,
    /// One-based native journal sequence numbers supporting the claimed result.
    #[serde(default)]
    pub evidence: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum TaskPlanStatus {
    #[default]
    #[serde(alias = "Pending", alias = "todo")]
    Pending,
    #[serde(alias = "InProgress", alias = "in_progress", alias = "active", alias = "running")]
    InProgress,
    #[serde(alias = "Done", alias = "completed", alias = "complete")]
    Done,
    #[serde(alias = "Blocked", alias = "waiting")]
    Blocked,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskCheckpointUpdate {
    /// Compare-and-swap revision obtained from get_task_memory or task_memory.
    #[serde(default)]
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
