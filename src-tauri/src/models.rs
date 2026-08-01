use serde::{Deserialize, Serialize};

use crate::attachments::ContextAttachment;
use crate::model::ModelInfo;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceStatus {
    pub bonsai: String,
    pub wikipedia: String,
    pub model: String,
    pub archive: String,
    pub offline_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub status: ServiceStatus,
    pub reports: Vec<ReportSummary>,
    pub library_root: String,
    pub settings: ResearchSettings,
    pub control: ControlSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ControlSettings {
    pub advanced_mode: bool,
    pub engine_path: String,
    pub extra_model_roots: Vec<String>,
    pub selected_model_id: Option<String>,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub threads: u32,
    pub project_root: String,
    pub agent_workspace_roots: Vec<String>,
    pub allow_full_access_agent: bool,
    pub agent_max_steps: u32,
    pub agent_max_output_tokens: u32,
}

impl Default for ControlSettings {
    fn default() -> Self {
        let project_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
            .to_string_lossy()
            .into_owned();
        let mut workspace_roots = Vec::new();
        if let Some(user) = directories::UserDirs::new() {
            for path in [user.desktop_dir(), user.document_dir(), user.download_dir()]
                .into_iter()
                .flatten()
            {
                workspace_roots.push(path.to_string_lossy().into_owned());
            }
        }
        Self {
            advanced_mode: false,
            engine_path: r"D:\LocalAI\Bonsai27B\runtime\llama-server.exe".into(),
            extra_model_roots: Vec::new(),
            selected_model_id: None,
            context_window: 32_768,
            max_output_tokens: 8_192,
            threads: std::thread::available_parallelism()
                .map(|value| value.get() as u32)
                .unwrap_or(4),
            project_root,
            agent_workspace_roots: workspace_roots,
            allow_full_access_agent: false,
            agent_max_steps: 30,
            agent_max_output_tokens: 8_192,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedRuntimeSnapshot {
    pub phase: String,
    pub mode: String,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub endpoint: Option<String>,
    pub pid: Option<u32>,
    pub context_window: u32,
    pub launch_args: Vec<String>,
    pub detail: String,
    pub inference_busy: bool,
}

impl Default for ManagedRuntimeSnapshot {
    fn default() -> Self {
        Self {
            phase: "stopped".into(),
            mode: "none".into(),
            model_id: None,
            model_name: None,
            endpoint: None,
            pid: None,
            context_window: 0,
            launch_args: Vec::new(),
            detail: "No local model runtime is attached.".into(),
            inference_busy: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeveloperStatus {
    pub codex_available: bool,
    pub codex_authenticated: bool,
    pub codex_version: Option<String>,
    pub project_root: String,
    pub git_repository: bool,
    pub worktree_clean: bool,
    pub running: bool,
    pub last_report: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlSnapshot {
    pub settings: ControlSettings,
    pub models: Vec<ModelInfo>,
    pub engine_candidates: Vec<EngineCandidate>,
    pub runtime: ManagedRuntimeSnapshot,
    pub gpu: Option<GpuSnapshot>,
    pub developer: DeveloperStatus,
    pub runtime_logs: Vec<RuntimeLog>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineCandidate {
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileTransfer {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeLog {
    pub stream: String,
    pub line: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// Present when an assistant turn is partial or stopped at the configured output limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ContextAttachment>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub model_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSessionSummary {
    pub id: String,
    pub title: String,
    pub model_id: String,
    pub updated_at: String,
    pub message_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartChatRequest {
    pub session_id: Option<String>,
    pub model_id: String,
    pub message: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStart {
    pub request_id: String,
    pub session: ChatSession,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamEvent {
    pub request_id: String,
    pub session_id: String,
    pub kind: String,
    pub content: Option<String>,
    pub data: Option<serde_json::Value>,
    pub at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerTaskRequest {
    pub model_id: String,
    pub objective: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    pub access: ComputerTaskAccess,
    #[serde(default)]
    pub approval_mode: ComputerTaskApprovalMode,
    pub max_steps: u32,
    pub max_output_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeComputerTaskRequest {
    pub run_id: String,
    pub answer: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveComputerTaskApprovalRequest {
    pub run_id: String,
    pub approval_id: String,
    pub decision: ComputerTaskApprovalDecision,
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ComputerTaskAccess {
    Workspace,
    Full,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ComputerTaskApprovalMode {
    Manual,
    #[default]
    Automatic,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ComputerTaskApprovalDecision {
    Approve,
    Reject,
}

/// One exact, validated native action waiting for a human decision. This is embedded in the
/// durable task transcript so an app restart cannot broaden or silently forget the request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingComputerAction {
    pub approval_id: String,
    #[serde(default)]
    pub step: u32,
    pub tool: String,
    pub arguments: serde_json::Value,
    pub summary: String,
    pub reason: String,
    pub risk: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerTaskEvent {
    pub run_id: String,
    pub step: u32,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub data: Option<serde_json::Value>,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerTaskRun {
    pub id: String,
    pub objective: String,
    pub model_id: String,
    pub access: ComputerTaskAccess,
    #[serde(default)]
    pub approval_mode: ComputerTaskApprovalMode,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub events: Vec<ComputerTaskEvent>,
    pub artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ContextAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerTaskSummary {
    pub id: String,
    pub objective: String,
    pub model_id: String,
    pub access: ComputerTaskAccess,
    #[serde(default)]
    pub approval_mode: ComputerTaskApprovalMode,
    pub status: String,
    pub updated_at: String,
    pub event_count: usize,
    pub artifact_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeveloperRepairRequest {
    pub issue: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeveloperRepairReport {
    pub success: bool,
    pub summary: String,
    pub diagnostics_before: String,
    pub diagnostics_after: String,
    pub report_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSettings {
    pub advanced_mode: bool,
    pub bonsai_root: String,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub research_lanes: u32,
    pub results_per_lane: u32,
    pub source_target: u32,
    pub tool_turns: u32,
    pub thinking_budget: u32,
    pub max_source_chars: u32,
}

impl Default for ResearchSettings {
    fn default() -> Self {
        Self {
            advanced_mode: false,
            bonsai_root: r"D:\LocalAI\Bonsai27B".into(),
            context_window: 98_304,
            max_output_tokens: 32_768,
            research_lanes: 6,
            results_per_lane: 6,
            source_target: 12,
            tool_turns: 24,
            thinking_budget: 4_096,
            max_source_chars: 20_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuSnapshot {
    pub name: String,
    pub total_mib: u64,
    pub used_mib: u64,
    pub free_mib: u64,
    pub utilization_percent: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub parallel_slots: u32,
    pub kv_cache: String,
    pub model_vram_mib: u64,
    pub model_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemSnapshot {
    pub status: ServiceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu: Option<GpuSnapshot>,
    pub runtime: RuntimeSnapshot,
    pub settings: ResearchSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub kind: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    pub reference: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub title: String,
    pub explanation: String,
    #[serde(default)]
    pub citations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchSection {
    pub id: String,
    pub heading: String,
    pub summary: String,
    #[serde(default)]
    pub body: Vec<String>,
    #[serde(default)]
    pub citations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineItem {
    pub label: String,
    pub date: String,
    pub description: String,
    #[serde(default)]
    pub citations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    pub term: String,
    pub meaning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReport {
    pub id: String,
    pub title: String,
    pub dek: String,
    pub query: String,
    pub answer: String,
    pub created_at: String,
    pub updated_at: String,
    pub edition: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub improvement: String,
    pub model: String,
    pub archive_snapshot: String,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub sections: Vec<ResearchSection>,
    #[serde(default)]
    pub timeline: Vec<TimelineItem>,
    #[serde(default)]
    pub terms: Vec<Term>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub sources: Vec<Source>,
    pub html_path: String,
    pub word_count: u32,
    pub reading_minutes: u32,
    #[serde(default = "default_profile")]
    pub research_profile: String,
    #[serde(default)]
    pub context_window: u32,
    #[serde(default)]
    pub output_budget: u32,
    #[serde(default)]
    pub research_lanes: u32,
}

fn default_profile() -> String {
    "standard".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportSummary {
    pub id: String,
    pub title: String,
    pub query: String,
    pub dek: String,
    pub updated_at: String,
    pub edition: u32,
    pub source_count: u32,
    pub reading_minutes: u32,
}

impl From<&ResearchReport> for ReportSummary {
    fn from(report: &ResearchReport) -> Self {
        Self {
            id: report.id.clone(),
            title: report.title.clone(),
            query: report.query.clone(),
            dek: report.dek.clone(),
            updated_at: report.updated_at.clone(),
            edition: report.edition,
            source_count: report.sources.len() as u32,
            reading_minutes: report.reading_minutes,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunResearchRequest {
    pub query: String,
    pub depth: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchProgress {
    pub job_id: String,
    pub stage: String,
    pub title: String,
    pub detail: String,
    pub current: u32,
    pub total: u32,
    pub elapsed_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchDraft {
    pub title: String,
    pub dek: String,
    pub answer: String,
    pub improvement: String,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub sections: Vec<ResearchSection>,
    #[serde(default)]
    pub timeline: Vec<TimelineItem>,
    #[serde(default)]
    pub terms: Vec<Term>,
    #[serde(default)]
    pub open_questions: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computer_task_access_rejects_unknown_values() {
        let request = serde_json::from_value::<ComputerTaskRequest>(serde_json::json!({
            "modelId": "model",
            "objective": "inspect files",
            "access": "network",
            "maxSteps": 1,
            "maxOutputTokens": 1
        }));
        assert!(request.is_err());
    }

    #[test]
    fn computer_task_approval_defaults_to_automatic_and_rejects_unknown_values() {
        let request = serde_json::from_value::<ComputerTaskRequest>(serde_json::json!({
            "modelId": "model",
            "objective": "inspect files",
            "access": "workspace",
            "maxSteps": 1,
            "maxOutputTokens": 1
        }))
        .unwrap();
        assert_eq!(request.approval_mode, ComputerTaskApprovalMode::Automatic);
        let invalid = serde_json::from_value::<ComputerTaskRequest>(serde_json::json!({
            "modelId": "model",
            "objective": "inspect files",
            "access": "workspace",
            "approvalMode": "trust-everything",
            "maxSteps": 1,
            "maxOutputTokens": 1
        }));
        assert!(invalid.is_err());
    }
}
