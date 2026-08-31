//! Rust-owned application contracts shared across Kestrel features.
//!
//! These types describe durable data and the native IPC boundary. TypeScript bindings are
//! generated from this crate; the desktop application must not maintain handwritten mirrors.

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ContextAttachment {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub mime_type: String,
    pub bytes: u64,
    pub sha256: String,
    pub stored_path: String,
    pub extracted_chars: usize,
    pub context_mode: String,
    pub note: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub source: String,
    pub bytes: u64,
    #[ts(optional)]
    pub architecture: Option<String>,
    #[ts(optional)]
    pub context_length: Option<u64>,
    pub chat_template: bool,
    #[ts(optional)]
    pub quantization: Option<String>,
    #[ts(optional)]
    pub mmproj_path: Option<String>,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub supports_audio: bool,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetupComponent {
    pub id: String,
    pub label: String,
    pub status: String,
    pub detail: String,
    pub path: String,
    pub download_bytes: u64,
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetupModelAsset {
    pub id: String,
    pub component: String,
    pub label: String,
    pub file_name: String,
    pub bytes: u64,
    pub recognized: bool,
    pub installed_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetupSnapshot {
    pub ready: bool,
    pub install_root: String,
    pub available_bytes: u64,
    #[ts(optional)]
    pub gpu_name: Option<String>,
    pub gpu_memory_bytes: u64,
    pub components: Vec<SetupComponent>,
    pub model_assets: Vec<SetupModelAsset>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetupLocations {
    pub install_root: String,
    pub bonsai_root: String,
    pub engine_path: String,
    pub wikipedia_zim_path: String,
    pub kiwix_server_path: String,
    pub comfy_root: String,
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetupInstallRequest {
    pub component: String,
    pub install_root: String,
    #[serde(default = "compact_wikipedia")]
    pub wikipedia_edition: String,
    #[serde(default)]
    pub accept_ideogram_non_commercial_license: bool,
    #[serde(default)]
    pub whisper_checkpoint_path: String,
    #[serde(default)]
    pub muscriptor_checkpoint_path: String,
    #[serde(default)]
    pub accept_muscriptor_non_commercial_license: bool,
    #[serde(default)]
    pub existing_model_paths: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SetupProgress {
    pub component: String,
    pub stage: String,
    pub detail: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_second: u64,
}

fn compact_wikipedia() -> String {
    "compact".into()
}

/// A proven, empirically validated hardware configuration for a local model and VRAM tier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProvenHardwareProfile {
    pub id: String,
    pub model_pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub quantization_pattern: Option<String>,
    pub display_name: String,
    pub min_vram_mib: u32,
    #[ts(optional)]
    pub max_vram_mib: Option<u32>,
    pub recommended_context_window: u32,
    pub recommended_max_output_tokens: u32,
    pub recommended_thinking_level: ThinkingLevel,
    pub recommended_threads: u32,
    pub description: String,
    pub proven_speed_notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ServiceStatus {
    #[ts(type = "\"ready\" | \"starting\" | \"stopped\" | \"unavailable\"")]
    pub model_runtime: String,
    #[ts(type = "\"ready\" | \"starting\" | \"stopped\" | \"unavailable\"")]
    pub wikipedia: String,
    pub model: String,
    pub archive: String,
    pub offline_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppSnapshot {
    pub status: ServiceStatus,
    pub reports: Vec<ReportSummary>,
    pub library_root: String,
    pub settings: ResearchSettings,
    pub control: ControlSnapshot,
    pub setup: SetupSnapshot,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ThinkingLevel {
    Off,
    Low,
    Medium,
    #[default]
    High,
    Max,
}

impl ThinkingLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "max",
        }
    }

    /// Jinja chat templates (e.g. Qwen 3.8) strictly enforce ('xhigh', 'high', 'medium', 'low')
    /// and raise an exception on 'max'. This helper returns the Jinja template-compatible reasoning effort string.
    pub fn as_template_effort(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Max => "xhigh",
        }
    }

    pub fn is_off(self) -> bool {
        matches!(self, Self::Off)
    }

    pub fn from_budget(budget: u32) -> Self {
        if budget == 0 {
            Self::Off
        } else if budget <= 2_048 {
            Self::Low
        } else if budget <= 8_192 {
            Self::Medium
        } else if budget <= 20_000 {
            Self::High
        } else {
            Self::Max
        }
    }

    #[allow(dead_code)]
    pub fn budget_tokens(self, max_output_tokens: u32) -> u32 {
        match self {
            Self::Off => 0,
            Self::Low => 2_048.min(max_output_tokens),
            Self::Medium => 8_192.min(max_output_tokens),
            Self::High => 16_384.min(max_output_tokens),
            Self::Max => max_output_tokens,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ControlSettings {
    pub advanced_mode: bool,
    pub engine_path: String,
    pub extra_model_roots: Vec<String>,
    #[ts(optional)]
    pub selected_model_id: Option<String>,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub threads: u32,
    pub thinking_level: ThinkingLevel,
    pub model_overrides: Vec<ModelRuntimeOverride>,
    pub project_root: String,
    pub agent_workspace_roots: Vec<String>,
    pub allow_full_access_agent: bool,
    pub agent_max_steps: u32,
    pub agent_max_output_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ModelRuntimeOverride {
    pub model_id: String,
    #[ts(optional)]
    pub context_window: Option<u32>,
    #[ts(optional)]
    pub max_output_tokens: Option<u32>,
    #[ts(optional)]
    pub threads: Option<u32>,
    #[ts(optional)]
    pub thinking_level: Option<ThinkingLevel>,
}

impl ControlSettings {
    /// Resolve app-wide defaults for one model. Feature-specific overrides are applied after this.
    pub fn for_model(&self, model_id: &str) -> Self {
        let mut effective = self.clone();
        if let Some(model) = self
            .model_overrides
            .iter()
            .find(|candidate| candidate.model_id == model_id)
        {
            if let Some(value) = model.context_window {
                effective.context_window = value;
            }
            if let Some(value) = model.max_output_tokens {
                effective.max_output_tokens = value;
            }
            if let Some(value) = model.threads {
                effective.threads = value;
            }
            if let Some(value) = model.thinking_level {
                effective.thinking_level = value;
            }
        }
        effective
    }
}

impl Default for ControlSettings {
    fn default() -> Self {
        let project_root = directories::UserDirs::new()
            .and_then(|dirs| dirs.document_dir().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| std::path::PathBuf::from("."))
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
            engine_path: default_bonsai_root()
                .join("runtime")
                .join("llama-server.exe")
                .to_string_lossy()
                .into_owned(),
            extra_model_roots: Vec::new(),
            selected_model_id: None,
            context_window: 32_768,
            max_output_tokens: 8_192,
            threads: std::thread::available_parallelism()
                .map(|value| value.get() as u32)
                .unwrap_or(4),
            thinking_level: ThinkingLevel::High,
            model_overrides: Vec::new(),
            project_root,
            agent_workspace_roots: workspace_roots,
            allow_full_access_agent: false,
            agent_max_steps: 30,
            agent_max_output_tokens: 8_192,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ManagedRuntimeSnapshot {
    pub phase: String,
    pub mode: String,
    #[ts(optional)]
    pub model_id: Option<String>,
    #[ts(optional)]
    pub model_name: Option<String>,
    #[ts(optional)]
    pub endpoint: Option<String>,
    #[ts(optional)]
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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeveloperStatus {
    pub codex_available: bool,
    pub codex_authenticated: bool,
    #[ts(optional)]
    pub codex_version: Option<String>,
    pub project_root: String,
    pub git_repository: bool,
    pub worktree_clean: bool,
    pub running: bool,
    #[ts(optional)]
    pub last_report: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ControlSnapshot {
    pub settings: ControlSettings,
    pub models: Vec<ModelInfo>,
    pub engine_candidates: Vec<EngineCandidate>,
    pub runtime: ManagedRuntimeSnapshot,
    #[ts(optional)]
    pub gpu: Option<GpuSnapshot>,
    pub developer: DeveloperStatus,
    pub runtime_logs: Vec<RuntimeLog>,
    #[serde(default)]
    pub proven_hardware_profiles: Vec<ProvenHardwareProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EngineCandidate {
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProfileTransfer {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RuntimeLog {
    pub stream: String,
    pub line: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SpeechTimingRecord {
    pub value: String,
    pub start: f64,
    pub end: f64,
}

pub const MAX_TRANSCRIPT_TIMINGS: usize = 100_000;

fn deserialize_speech_timings<'de, D>(deserializer: D) -> Result<Vec<SpeechTimingRecord>, D::Error>
where
    D: Deserializer<'de>,
{
    let timings = Vec::<SpeechTimingRecord>::deserialize(deserializer)?;
    if timings.len() > MAX_TRANSCRIPT_TIMINGS {
        return Err(D::Error::custom(format!(
            "voice recording exceeds the {MAX_TRANSCRIPT_TIMINGS} word-timing limit"
        )));
    }
    if !timings.iter().all(|timing| {
        timing.value.len() <= 4_096
            && timing.start.is_finite()
            && timing.end.is_finite()
            && timing.start >= 0.0
            && timing.end >= timing.start
            && timing.end <= 24.0 * 60.0 * 60.0
    }) {
        return Err(D::Error::custom(
            "voice recording contains an invalid word timing",
        ));
    }
    Ok(timings)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SpeechRecordingAttachment {
    pub audio_relative_path: String,
    #[serde(default, deserialize_with = "deserialize_speech_timings")]
    pub words: Vec<SpeechTimingRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub reasoning: Option<String>,
    /// Present when an assistant turn is partial or stopped at the configured output limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ContextAttachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recording: Option<SpeechRecordingAttachment>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub model_id: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatSessionSummary {
    pub id: String,
    pub title: String,
    pub model_id: String,
    pub updated_at: String,
    pub message_count: usize,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StartChatRequest {
    #[ts(optional)]
    pub session_id: Option<String>,
    pub model_id: String,
    pub message: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    #[serde(default)]
    #[ts(optional)]
    pub recording: Option<SpeechRecordingAttachment>,
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: u32,
    pub max_output_tokens: u32,
    #[serde(default)]
    #[ts(optional)]
    pub thinking_level: Option<ThinkingLevel>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatStart {
    pub request_id: String,
    pub session: ChatSession,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChatStreamEvent {
    pub request_id: String,
    pub session_id: String,
    #[ts(
        type = "\"queued\" | \"started\" | \"context\" | \"token\" | \"reasoning\" | \"metrics\" | \"done\" | \"cancelled\" | \"error\" | \"settled\""
    )]
    pub kind: String,
    #[ts(optional)]
    pub content: Option<String>,
    #[ts(optional, type = "Record<string, unknown>")]
    pub data: Option<serde_json::Value>,
    pub at: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ComputerTaskRequest {
    pub model_id: String,
    pub objective: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    pub access: ComputerTaskAccess,
    pub max_steps: u32,
    pub max_output_tokens: u32,
    #[serde(default)]
    #[ts(optional)]
    pub thinking_level: Option<ThinkingLevel>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResumeComputerTaskRequest {
    pub run_id: String,
    pub answer: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum ComputerTaskAccess {
    Workspace,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ComputerTaskEvent {
    pub run_id: String,
    pub step: u32,
    pub kind: String,
    pub title: String,
    pub detail: String,
    #[ts(
        optional,
        type = "{ path?: string; question?: string; options?: Array<string>; recommendedIndex?: number; [key: string]: unknown }"
    )]
    pub data: Option<serde_json::Value>,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ComputerTaskRun {
    pub id: String,
    pub objective: String,
    pub model_id: String,
    pub access: ComputerTaskAccess,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub events: Vec<ComputerTaskEvent>,
    pub artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ContextAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ComputerTaskSummary {
    pub id: String,
    pub objective: String,
    pub model_id: String,
    pub access: ComputerTaskAccess,
    pub status: String,
    pub updated_at: String,
    pub event_count: usize,
    pub artifact_count: usize,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeveloperRepairRequest {
    pub issue: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeveloperRepairReport {
    pub success: bool,
    pub summary: String,
    pub diagnostics_before: String,
    pub diagnostics_after: String,
    pub report_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ResearchSettings {
    pub advanced_mode: bool,
    pub bonsai_root: String,
    pub install_root: String,
    pub wikipedia_zim_path: String,
    pub kiwix_server_path: String,
    pub wikipedia_book: String,
    pub wikipedia_snapshot: String,
    pub comfy_root: String,
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
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
        let install_root = default_install_root();
        let bonsai_root = default_bonsai_root();
        let wikipedia_zim = install_root
            .join("Wikipedia")
            .join("wikipedia_en_all_mini_2026-06.zim");
        let kiwix_server = install_root
            .join("Wikipedia")
            .join("tools")
            .join("kiwix-serve.exe");
        let comfy_root = install_root
            .join("ComfyUI_windows_portable")
            .join("ComfyUI");
        let wikipedia_book = wikipedia_zim
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("wikipedia_en_all_mini_2026-06")
            .to_string();
        let wikipedia_snapshot = wikipedia_book
            .rsplit_once('_')
            .map(|(_, value)| value)
            .unwrap_or("2026-06")
            .to_string();
        Self {
            advanced_mode: false,
            bonsai_root: bonsai_root.to_string_lossy().into_owned(),
            install_root: install_root.to_string_lossy().into_owned(),
            wikipedia_zim_path: wikipedia_zim.to_string_lossy().into_owned(),
            kiwix_server_path: kiwix_server.to_string_lossy().into_owned(),
            wikipedia_book,
            wikipedia_snapshot,
            comfy_root: comfy_root.to_string_lossy().into_owned(),
            ffmpeg_path: String::new(),
            ffprobe_path: String::new(),
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

fn default_install_root() -> std::path::PathBuf {
    directories::UserDirs::new()
        .map(|dirs| dirs.home_dir().join("Kestrel AI"))
        .unwrap_or_else(|| std::path::PathBuf::from("Kestrel AI"))
}

fn default_bonsai_root() -> std::path::PathBuf {
    default_install_root().join("Bonsai")
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GpuSnapshot {
    pub name: String,
    pub total_mib: u64,
    pub used_mib: u64,
    pub free_mib: u64,
    pub utilization_percent: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RuntimeSnapshot {
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub parallel_slots: u32,
    pub kv_cache: String,
    pub model_vram_mib: u64,
    pub model_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SystemSnapshot {
    pub status: ServiceStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub gpu: Option<GpuSnapshot>,
    pub runtime: RuntimeSnapshot,
    pub settings: ResearchSettings,
    pub control: ControlSettings,
    pub models: Vec<ModelInfo>,
    pub managed_runtime: ManagedRuntimeSnapshot,
    #[serde(default)]
    pub proven_hardware_profiles: Vec<ProvenHardwareProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Source {
    pub id: String,
    pub kind: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub section: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub snapshot: Option<String>,
    pub reference: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Finding {
    pub title: String,
    pub explanation: String,
    #[serde(default)]
    pub citations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResearchSection {
    pub id: String,
    pub heading: String,
    pub summary: String,
    #[serde(default)]
    pub body: Vec<String>,
    #[serde(default)]
    pub citations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TimelineItem {
    pub label: String,
    pub date: String,
    pub description: String,
    #[serde(default)]
    pub citations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Term {
    pub term: String,
    pub meaning: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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
    #[ts(optional)]
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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RunResearchRequest {
    pub query: String,
    #[ts(type = "\"focused\" | \"thorough\" | \"expedition\"")]
    pub depth: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResearchProgress {
    pub job_id: String,
    #[ts(
        type = "\"preparing\" | \"library\" | \"searching\" | \"reading\" | \"synthesizing\" | \"publishing\" | \"complete\" | \"cancelled\" | \"failed\""
    )]
    pub stage: String,
    pub title: String,
    pub detail: String,
    pub current: u32,
    pub total: u32,
    pub elapsed_seconds: u64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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

/// The two intentionally separate local-model conversations in Movie Studio.
///
/// Story conversations own prose revisions. Scene conversations own H3 prompt drafting. Neither
/// conversation receives filesystem tools, rendering authority, or producer reference choices.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MovieStudioConversationKind {
    Story,
    Scenes,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MovieStudioMessageRole {
    Producer,
    Collaborator,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MovieStoryRevisionOrigin {
    Producer,
    Collaborator,
    Imported,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MovieStudioConversationMode {
    Continue,
    Fresh,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MovieSceneFrameSourceKind {
    PreviousScene,
    ReferenceImage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieSceneFrameSource {
    pub kind: MovieSceneFrameSourceKind,
    #[ts(optional)]
    pub asset_id: Option<String>,
}

/// Producer-owned reference selection for one scene.
///
/// The local model never receives or writes this structure. Kestrel binds these selections to the
/// native H3 graph and adds the required renderer text immediately before rendering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieSceneReferenceSelection {
    pub asset_id: String,
    pub use_visual: bool,
    pub use_audio: bool,
    #[serde(default)]
    pub guidance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieSceneDraft {
    pub id: String,
    pub revision: u32,
    pub title: String,
    pub purpose: String,
    pub duration_seconds: f32,
    pub h3_prompt: String,
    pub continuity_in: String,
    pub continuity_out: String,
    pub transition: String,
    #[ts(optional)]
    pub first_frame: Option<MovieSceneFrameSource>,
    #[ts(optional)]
    pub last_frame: Option<MovieSceneFrameSource>,
    #[serde(default)]
    pub references: Vec<MovieSceneReferenceSelection>,
    pub story_revision_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieStoryRevision {
    pub id: String,
    pub number: u32,
    #[ts(optional)]
    pub parent_revision_id: Option<String>,
    pub created_at: String,
    pub origin: MovieStoryRevisionOrigin,
    pub instruction: String,
    pub markdown: String,
    #[ts(optional)]
    pub accepted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieStudioMessage {
    pub id: String,
    pub created_at: String,
    pub role: MovieStudioMessageRole,
    pub markdown: String,
    #[ts(optional)]
    pub story_revision_id: Option<String>,
    #[serde(default)]
    pub selected_scene_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieStudioConversationSummary {
    pub id: String,
    pub kind: MovieStudioConversationKind,
    pub created_at: String,
    pub updated_at: String,
    pub story_revision_id: String,
    pub title: String,
    pub summary: String,
    pub message_count: usize,
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieStudioConversation {
    pub id: String,
    pub kind: MovieStudioConversationKind,
    pub created_at: String,
    pub updated_at: String,
    pub story_revision_id: String,
    pub title: String,
    pub summary: String,
    pub archived: bool,
    #[serde(default)]
    pub messages: Vec<MovieStudioMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieProducerWorkspace {
    pub schema_version: u32,
    pub project_id: String,
    pub created_at: String,
    pub updated_at: String,
    #[ts(optional)]
    pub active_story_revision_id: Option<String>,
    #[ts(optional)]
    pub accepted_story_revision_id: Option<String>,
    #[ts(optional)]
    pub active_story_conversation_id: Option<String>,
    #[ts(optional)]
    pub active_scene_conversation_id: Option<String>,
    #[serde(default)]
    pub story_revisions: Vec<MovieStoryRevision>,
    #[serde(default)]
    pub conversations: Vec<MovieStudioConversationSummary>,
    #[serde(default)]
    pub scenes: Vec<MovieSceneDraft>,
    pub scene_revision: u64,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieStudioChatRequest {
    pub request_id: String,
    pub project_id: String,
    pub kind: MovieStudioConversationKind,
    pub mode: MovieStudioConversationMode,
    #[ts(optional)]
    pub conversation_id: Option<String>,
    pub model_id: String,
    pub instruction: String,
    #[ts(optional)]
    pub story_revision_id: Option<String>,
    #[serde(default)]
    pub selected_scene_ids: Vec<String>,
    #[ts(optional)]
    pub thinking_level: Option<ThinkingLevel>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieStudioChatEvent {
    pub request_id: String,
    pub project_id: String,
    pub conversation_id: String,
    pub kind: MovieStudioConversationKind,
    #[ts(
        type = "\"queued\" | \"started\" | \"token\" | \"reasoning\" | \"complete\" | \"cancelled\" | \"error\" | \"settled\""
    )]
    pub event: String,
    #[ts(optional)]
    pub content: Option<String>,
    #[ts(optional)]
    pub model_name: Option<String>,
    #[ts(optional)]
    pub thinking_level: Option<ThinkingLevel>,
    #[ts(optional)]
    pub story_revision: Option<MovieStoryRevision>,
    #[serde(default)]
    pub changed_scene_ids: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveMovieStoryRevisionRequest {
    pub project_id: String,
    #[ts(optional)]
    pub parent_revision_id: Option<String>,
    pub markdown: String,
    #[serde(default)]
    pub instruction: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AcceptMovieStoryRevisionRequest {
    pub project_id: String,
    pub revision_id: String,
    pub conversation_mode: MovieStudioConversationMode,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SaveMovieScenesRequest {
    pub project_id: String,
    pub expected_revision: u64,
    pub scenes: Vec<MovieSceneDraft>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResetMovieStudioConversationRequest {
    pub project_id: String,
    pub conversation_id: String,
    pub keep_summary: bool,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SummarizeMovieStudioConversationRequest {
    pub project_id: String,
    pub conversation_id: String,
    pub model_id: String,
    #[ts(optional)]
    pub thinking_level: Option<ThinkingLevel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieProducerProjectSettings {
    pub width: u32,
    pub height: u32,
    pub clip_seconds: f32,
    pub steps: u32,
    pub max_clips: u32,
    pub seed: u64,
    pub thinking_budget: u32,
    pub max_output_tokens: u32,
    #[ts(optional)]
    pub context_window: Option<u32>,
    pub comfy_root: String,
    pub ref_image_size: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MovieProducerReferenceRequest {
    pub asset_id: String,
    pub description: String,
    pub include_embedded_audio: bool,
    pub embedded_audio_description: String,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CreateMovieProducerProjectRequest {
    pub starting_material: String,
    pub collaborator_model_id: String,
    #[ts(optional)]
    pub thinking_level: Option<ThinkingLevel>,
    pub settings: MovieProducerProjectSettings,
    #[serde(default)]
    pub references: Vec<MovieProducerReferenceRequest>,
}

#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AttachMovieProducerReferencesRequest {
    pub project_id: String,
    pub references: Vec<MovieProducerReferenceRequest>,
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
    fn voice_recording_rejects_oversized_or_invalid_word_timings() {
        let words = (0..=MAX_TRANSCRIPT_TIMINGS)
            .map(|_| serde_json::json!({"value":"word","start":0.0,"end":0.1}))
            .collect::<Vec<_>>();
        assert!(
            serde_json::from_value::<SpeechRecordingAttachment>(serde_json::json!({
                "audioRelativePath": "recordings/chat/voice.webm",
                "words": words,
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<SpeechRecordingAttachment>(serde_json::json!({
                "audioRelativePath": "recordings/chat/voice.webm",
                "words": [{"value":"word","start":2.0,"end":1.0}],
            }))
            .is_err()
        );
    }
}
