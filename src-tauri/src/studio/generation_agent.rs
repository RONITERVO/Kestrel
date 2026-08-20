//! Durable agent flow for producer-directed generative movie edits.
//!
//! Planning and generative repair share the same model protocol, lifecycle limits, single
//! inference lease, typed workspace discipline, native checks, and fresh-context review. H3 is
//! never started here: the agent produces a checked candidate and the producer explicitly chooses
//! whether to render or place it in the storyline.

use super::agent_lifecycle::{AgentLifecycle, ReviewDecision, TurnDecision};
use super::agent_protocol::{self, AgentTranscript, AssistantTurn};
use super::{
    check_cancel, has_meaningful_prose, prepare_producer_plan, prompt_quality_issues,
    reference_manifest, write_json_atomic, MovieFrameAnchor, MovieModelRuntime, MovieProject,
    MovieStudio, MovieTransitionPosition, PlannedClip, StudioError,
};
use crate::models::ThinkingLevel;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

const GENERATION_AGENT_STEPS_PER_SESSION: u32 = 24;
const GENERATION_CONTEXT_BYTES: usize = 384 * 1024;
const MAX_ANCHOR_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
const FRAME_ANALYSIS_OUTPUT_TOKENS: u32 = 4_096;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieGenerationAgentRequest {
    pub request_id: String,
    pub project_id: String,
    pub task: MovieGenerationTask,
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    /// Optional local vision model used only to observe exact transition endpoint PNGs.
    /// The Director and reviewer still use their project-pinned roles.
    #[serde(default)]
    pub frame_analyst_model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MovieGenerationTask {
    ShotVersion {
        #[serde(rename = "clipId", alias = "clip_id")]
        clip_id: String,
        direction: String,
    },
    Transition {
        position: MovieTransitionPosition,
        #[serde(default, rename = "firstAnchor", alias = "first_anchor")]
        first_anchor: Option<MovieFrameAnchor>,
        #[serde(default, rename = "lastAnchor", alias = "last_anchor")]
        last_anchor: Option<MovieFrameAnchor>,
        direction: String,
        #[serde(rename = "durationSeconds", alias = "duration_seconds")]
        duration_seconds: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieGenerationProposal {
    pub summary: String,
    pub review_summary: String,
    pub candidate: MovieGenerationCandidate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MovieGenerationCandidate {
    ShotVersion {
        #[serde(rename = "clipId", alias = "clip_id")]
        clip_id: String,
        clip: PlannedClip,
        #[serde(default)]
        checklist: Vec<String>,
    },
    Transition {
        #[serde(rename = "motionPrompt", alias = "motion_prompt")]
        motion_prompt: String,
        #[serde(rename = "durationSeconds", alias = "duration_seconds")]
        duration_seconds: f32,
        #[serde(rename = "cameraMotion", alias = "camera_motion")]
        camera_motion: String,
        #[serde(rename = "subjectMotion", alias = "subject_motion")]
        subject_motion: String,
        #[serde(rename = "transitionNotes", alias = "transition_notes")]
        transition_notes: String,
        #[serde(default)]
        checklist: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieGenerationAgentEvent {
    pub request_id: String,
    pub project_id: String,
    pub kind: String,
    pub model_role: String,
    pub content: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerationReview {
    approved: bool,
    summary: String,
    #[serde(default)]
    issues: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct GenerationFrameAnalysis {
    schema_version: u32,
    model_id: String,
    model_name: String,
    frames: Vec<GenerationFrameObservation>,
    created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct GenerationFrameObservation {
    role: String,
    anchor: MovieFrameAnchor,
    image_path: String,
    image_sha256: String,
    description: String,
    visible_action: String,
    composition: String,
    #[serde(default)]
    continuity_facts: Vec<String>,
    #[serde(default)]
    uncertainties: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameAnalysisSubmission {
    frames: Vec<FrameObservationSubmission>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameObservationSubmission {
    role: String,
    description: String,
    visible_action: String,
    composition: String,
    #[serde(default)]
    continuity_facts: Vec<String>,
    #[serde(default)]
    uncertainties: Vec<String>,
}

struct CapturedAnalysisFrame {
    role: &'static str,
    anchor: MovieFrameAnchor,
    path: PathBuf,
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct GenerationWorkspaceState {
    revision: u64,
    checked_revision: Option<u64>,
    clean_checks: u32,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GenerationAction {
    ReadContext,
    ReadCandidate,
    WriteCandidate,
    Check,
    Submit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerationToolRequest {
    action: GenerationAction,
    #[serde(default)]
    candidate: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyGenerationToolRequest {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WriteGenerationCandidateRequest {
    candidate: Value,
}

struct GenerationWorkspace {
    root: PathBuf,
    project: MovieProject,
    task: MovieGenerationTask,
    context: Value,
    candidate: Option<MovieGenerationProposal>,
    state: GenerationWorkspaceState,
}

enum GenerationToolOutcome {
    Continue(String),
    MalformedArguments(String),
    Submitted(Box<MovieGenerationProposal>),
}

struct GenerationReviewContext<'a> {
    studio: &'a MovieStudio,
    request: &'a MovieGenerationAgentRequest,
    project: &'a MovieProject,
    model_runtime: MovieModelRuntime<'a>,
    cancel: &'a CancellationToken,
    app: Option<&'a AppHandle>,
    workspace_root: &'a std::path::Path,
    exact_frame_analysis: Option<Value>,
}

impl GenerationWorkspace {
    fn open(
        root: PathBuf,
        project: MovieProject,
        task: MovieGenerationTask,
        frame_analysis: Option<GenerationFrameAnalysis>,
    ) -> Result<Self, StudioError> {
        fs::create_dir_all(&root)?;
        let task_path = root.join("task.json");
        validate_checkpoint_task(&root, &task)?;
        let context = generation_context(&project, &task, frame_analysis.as_ref())?;
        let candidate_path = root.join("candidate.json");
        let candidate = fs::read(&candidate_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        let state = fs::read(root.join("state.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        write_json_atomic(&root.join("context.json"), &context)?;
        write_json_atomic(&task_path, &task)?;
        let workspace = Self {
            root,
            project,
            task,
            context,
            candidate,
            state,
        };
        workspace.persist_state()?;
        Ok(workspace)
    }

    fn tools(&self) -> Value {
        let candidate = generation_candidate_schema(&self.task);
        json!([
            simple_generation_tool(
                "generation_read_context",
                "Acknowledge the complete authoritative production context already attached to this turn. Call with an empty object."
            ),
            simple_generation_tool(
                "generation_read_candidate",
                "Read the current durable generative-edit candidate and native check state. Call with an empty object."
            ),
            {"type":"function","function":{
                "name":"generation_write_candidate",
                "description":"Write one complete typed generative-edit candidate. This is the only tool that accepts candidate data.",
                "parameters":{"type":"object","additionalProperties":false,"properties":{
                    "candidate":candidate
                },"required":["candidate"]}
            }},
            simple_generation_tool(
                "generation_check",
                "Run one native check on the unchanged durable candidate. Call with an empty object."
            ),
            simple_generation_tool(
                "generation_submit",
                "Submit the already durable candidate for fresh-context review after two clean checks. Call with an empty object; never resend the candidate."
            )
        ])
    }

    fn authoritative_memory(&self) -> Result<String, StudioError> {
        let value = json!({
            "notice":"Authoritative current production context. Re-read this on every turn; never continue a different story or cut.",
            "task":self.task,
            "context":self.context,
            "currentCandidate":self.candidate,
            "nativeState":self.state,
        });
        let text = serde_json::to_string(&value)?;
        if text.len() > GENERATION_CONTEXT_BYTES {
            return Err(StudioError::Invalid(
                "the generative-edit context exceeds the 384 KiB agent limit; shorten producer notes before retrying"
                    .into(),
            ));
        }
        Ok(text)
    }

    fn execute(&mut self, request: GenerationToolRequest) -> GenerationToolOutcome {
        match self.execute_checked(request) {
            Ok(outcome) => outcome,
            Err(error) => GenerationToolOutcome::Continue(format!("ERROR: {error}")),
        }
    }

    fn execute_checked(
        &mut self,
        request: GenerationToolRequest,
    ) -> Result<GenerationToolOutcome, StudioError> {
        match request.action {
            GenerationAction::ReadContext => Ok(GenerationToolOutcome::Continue(
                "Context acknowledged. The complete authoritative current production context is already attached as the final user message on every Director turn; use that copy rather than asking Kestrel to duplicate it in the transcript."
                    .into(),
            )),
            GenerationAction::ReadCandidate => Ok(GenerationToolOutcome::Continue(format!(
                "CANDIDATE: {}",
                serde_json::to_string(&self.candidate)?
            ))),
            GenerationAction::WriteCandidate => {
                let value = request.candidate.ok_or_else(|| {
                    StudioError::Invalid("write_candidate requires candidate".into())
                })?;
                let candidate: MovieGenerationProposal = serde_json::from_value(value)
                    .map_err(|error| StudioError::Invalid(format!("invalid candidate: {error}")))?;
                validate_generation_candidate(&self.project, &self.task, &candidate)?;
                self.state.revision = self.state.revision.saturating_add(1);
                self.state.checked_revision = None;
                self.state.clean_checks = 0;
                write_json_atomic(&self.root.join("candidate.json"), &candidate)?;
                self.candidate = Some(candidate);
                self.persist_state()?;
                Ok(GenerationToolOutcome::Continue(
                    "Candidate saved. Run check twice without another write before submit.".into(),
                ))
            }
            GenerationAction::Check => {
                let candidate = self.candidate.as_ref().ok_or_else(|| {
                    StudioError::Invalid("write a candidate before running native checks".into())
                })?;
                validate_generation_candidate(&self.project, &self.task, candidate)?;
                if self.state.checked_revision == Some(self.state.revision) {
                    self.state.clean_checks = self.state.clean_checks.saturating_add(1).min(2);
                } else {
                    self.state.checked_revision = Some(self.state.revision);
                    self.state.clean_checks = 1;
                }
                self.persist_state()?;
                let message = if self.state.clean_checks == 1 {
                    "First clean native check passed. Re-read the complete context and run check once more without changing the candidate."
                } else {
                    "Second clean native check passed. Submit the unchanged candidate for fresh-context review."
                };
                Ok(GenerationToolOutcome::Continue(message.into()))
            }
            GenerationAction::Submit => {
                let candidate = self.candidate.as_ref().ok_or_else(|| {
                    StudioError::Invalid("write a candidate before submit".into())
                })?;
                validate_generation_candidate(&self.project, &self.task, candidate)?;
                if self.state.checked_revision != Some(self.state.revision)
                    || self.state.clean_checks < 2
                {
                    return Err(StudioError::Invalid(
                        "submit is blocked until the unchanged candidate passes two clean native checks"
                            .into(),
                    ));
                }
                Ok(GenerationToolOutcome::Submitted(Box::new(
                    candidate.clone(),
                )))
            }
        }
    }

    fn record_review_rejection(&mut self, review: &GenerationReview) -> Result<(), StudioError> {
        write_json_atomic(&self.root.join("review.json"), review)?;
        self.state.checked_revision = None;
        self.state.clean_checks = 0;
        self.persist_state()
    }

    fn persist_result(&self, proposal: &MovieGenerationProposal) -> Result<(), StudioError> {
        write_json_atomic(&self.root.join("result.json"), proposal)
    }

    fn persist_state(&self) -> Result<(), StudioError> {
        write_json_atomic(&self.root.join("state.json"), &self.state)
    }
}

pub(super) async fn run(
    studio: &MovieStudio,
    request: &MovieGenerationAgentRequest,
    model_runtime: MovieModelRuntime<'_>,
    cancel: &CancellationToken,
    app: Option<&AppHandle>,
) -> Result<MovieGenerationProposal, StudioError> {
    check_cancel(cancel)?;
    validate_generation_request(request)?;
    let project = studio.get(&request.project_id)?;
    let workspace_root = studio
        .project_dir(&project.id)
        .join("agent-workspace")
        .join("generative-edits")
        .join(&request.request_id);
    fs::create_dir_all(&workspace_root)?;
    validate_checkpoint_task(&workspace_root, &request.task)?;
    let frame_analysis = prepare_frame_analysis(
        studio,
        request,
        &project,
        model_runtime,
        cancel,
        app,
        &workspace_root,
    )
    .await?;
    let transcript_path = workspace_root.join("transcript.json");
    let resume_saved_transcript = transcript_path.is_file();
    let mut workspace = GenerationWorkspace::open(
        workspace_root.clone(),
        project.clone(),
        request.task.clone(),
        frame_analysis,
    )?;
    let result_path = workspace_root.join("result.json");
    if result_path.is_file() && fs::metadata(&result_path)?.len() <= 256 * 1024 {
        if let Ok(result) =
            serde_json::from_slice::<MovieGenerationProposal>(&fs::read(&result_path)?)
        {
            if validate_generation_candidate(&project, &request.task, &result).is_ok() {
                emit(app, request, "complete", "reviewer", &result.review_summary);
                return Ok(result);
            }
        }
    }
    let tools = workspace.tools();
    let mut lifecycle = AgentLifecycle::new();

    'sessions: loop {
        lifecycle.ensure_session_budget()?;
        if lifecycle.session() > 1 && transcript_path.is_file() {
            fs::copy(
                &transcript_path,
                workspace_root.join(format!(
                    "transcript-session-{:03}.json",
                    lifecycle.session() - 1
                )),
            )?;
        }
        let system_prompt = super::prompts::generation_agent_system();
        let session_instruction = if lifecycle.session() == 1 {
            super::prompts::generation_initial()
        } else {
            super::prompts::generation_resume()
        };
        let mut transcript = if lifecycle.session() == 1 && resume_saved_transcript {
            AgentTranscript::resume(
                transcript_path.clone(),
                &super::prompts::generation_resume(),
            )?
        } else {
            AgentTranscript::begin(
                transcript_path.clone(),
                lifecycle.absolute_step(),
                &system_prompt,
                &session_instruction,
            )?
        };
        for _ in 0..GENERATION_AGENT_STEPS_PER_SESSION {
            check_cancel(cancel)?;
            let step = lifecycle.begin_step();
            emit(
                app,
                request,
                "turn-start",
                "director",
                &format!("Generative Director turn {step}"),
            );
            let messages = transcript.request_messages(workspace.authoritative_memory()?);
            let director_settings = project.settings.runtime_settings_for(
                model_runtime.settings,
                model_runtime.director_model_id,
            );
            let lease = tokio::select! {
                result = model_runtime.runtime.lease_model(
                    model_runtime.director_model_id,
                    model_runtime.models,
                    &director_settings,
                    app,
                ) => result.map_err(|error| StudioError::Planning(error.to_string()))?,
                _ = cancel.cancelled() => return Err(StudioError::Cancelled),
            };
            let mut settings = project.settings.clone();
            settings.thinking_budget = request
                .thinking_level
                .unwrap_or(director_settings.thinking_level)
                .budget_tokens(32_768);
            let response = agent_protocol::complete_stream(
                &studio.http,
                agent_protocol::StreamCompletionRequest {
                    connection: &lease.connection,
                    messages: &messages,
                    tools: &tools,
                    settings: &settings,
                    runtime_max_output_tokens: director_settings.max_output_tokens,
                    cancel,
                    audit_path: Some(&workspace_root.join("last-request.json")),
                    fallback_tool_call_prefix: &format!("generation-tool-{step}"),
                },
                |event| emit_stream_event(app, request, "director", event),
            )
            .await;
            drop(lease);
            let response = match response {
                Ok(response) => response,
                Err(StudioError::Cancelled) => return Err(StudioError::Cancelled),
                Err(error) => {
                    transcript.push(
                        json!({"role":"user","content":format!("The model turn failed safely: {error}. Resume from the durable workspace.")}),
                        step,
                    )?;
                    lifecycle.restart_session();
                    continue 'sessions;
                }
            };
            let turn = AssistantTurn::from_response(&response)?;
            transcript.push(turn.history_message(), step)?;
            if !turn.has_tool_calls() {
                transcript.push(
                    json!({"role":"user","content":super::prompts::generation_continue()}),
                    step,
                )?;
                if lifecycle.record_model_turn(false) == TurnDecision::RestartSession {
                    lifecycle.restart_session();
                    continue 'sessions;
                }
                continue;
            }
            lifecycle.record_model_turn(true);
            for call in turn.tool_calls() {
                let call_id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("generation-tool")
                    .to_owned();
                let result = execute_call(&mut workspace, call);
                let (message, submitted, restart_after_tool_error) = match result {
                    GenerationToolOutcome::Continue(message) => (message, None, false),
                    GenerationToolOutcome::MalformedArguments(message) => {
                        (message, None, true)
                    }
                    GenerationToolOutcome::Submitted(candidate) => (
                        "Candidate submitted for independent review.".into(),
                        Some(candidate),
                        false,
                    ),
                };
                emit(app, request, "activity", "director", &message);
                transcript.push(
                    json!({"role":"tool","tool_call_id":call_id,"content":message}),
                    step,
                )?;
                if restart_after_tool_error {
                    emit(
                        app,
                        request,
                        "activity",
                        "director",
                        "The incomplete tool call was discarded. Restarting from the durable candidate in a clean context session",
                    );
                    lifecycle.restart_session();
                    continue 'sessions;
                }
                let Some(mut candidate) = submitted else {
                    continue;
                };
                let review = review_candidate(
                    &candidate,
                    GenerationReviewContext {
                        studio,
                        request,
                        project: &project,
                        model_runtime,
                        cancel,
                        app,
                        workspace_root: &workspace_root,
                        exact_frame_analysis: workspace
                            .context
                            .get("exactFrameAnalysis")
                            .filter(|value| !value.is_null())
                            .cloned(),
                    },
                )
                .await?;
                if review.approved && review.issues.is_empty() {
                    candidate.review_summary = review.summary;
                    workspace.persist_result(&candidate)?;
                    emit(
                        app,
                        request,
                        "complete",
                        "reviewer",
                        &candidate.review_summary,
                    );
                    return Ok(*candidate);
                }
                workspace.record_review_rejection(&review)?;
                if lifecycle.record_review_rejection() == ReviewDecision::Exhausted {
                    return Err(StudioError::Planning(format!(
                        "the fresh-context generative reviewer still found {} blocking issue(s) after three repair rounds; the durable candidate and review are preserved",
                        review.issues.len()
                    )));
                }
                transcript.push(
                    json!({"role":"user","content":super::prompts::generation_review_rejected(&serde_json::to_string(&review)?)}),
                    step,
                )?;
            }
        }
        lifecycle.restart_session();
    }
}

fn validate_checkpoint_task(root: &Path, task: &MovieGenerationTask) -> Result<(), StudioError> {
    let task_path = root.join("task.json");
    if !task_path.is_file() {
        return Ok(());
    }
    if fs::metadata(&task_path)?.len() > 64 * 1024 {
        return Err(StudioError::Invalid(
            "the saved generative-edit task exceeds the 64 KiB resume limit".into(),
        ));
    }
    let bytes = fs::read(&task_path)?;
    let saved: Value = serde_json::from_slice(&bytes)?;
    let requested = serde_json::to_value(task)?;
    if saved != requested {
        return Err(StudioError::Invalid(
            "this checkpoint belongs to a different generative-edit task; start a new request instead"
                .into(),
        ));
    }
    Ok(())
}

fn execute_call(workspace: &mut GenerationWorkspace, call: &Value) -> GenerationToolOutcome {
    let Some(name) = call.pointer("/function/name").and_then(Value::as_str) else {
        return GenerationToolOutcome::Continue("ERROR: unknown tool".into());
    };
    let Some(arguments) = call.pointer("/function/arguments") else {
        return GenerationToolOutcome::Continue("ERROR: missing tool arguments".into());
    };
    let request = match name {
        "generation_read_context" => {
            parse_generation_arguments::<EmptyGenerationToolRequest>(arguments).map(|_| {
                GenerationToolRequest {
                    action: GenerationAction::ReadContext,
                    candidate: None,
                }
            })
        }
        "generation_read_candidate" => {
            parse_generation_arguments::<EmptyGenerationToolRequest>(arguments).map(|_| {
                GenerationToolRequest {
                    action: GenerationAction::ReadCandidate,
                    candidate: None,
                }
            })
        }
        "generation_write_candidate" => {
            parse_generation_arguments::<WriteGenerationCandidateRequest>(arguments).map(
                |request| GenerationToolRequest {
                    action: GenerationAction::WriteCandidate,
                    candidate: Some(request.candidate),
                },
            )
        }
        "generation_check" => {
            parse_generation_arguments::<EmptyGenerationToolRequest>(arguments).map(|_| {
                GenerationToolRequest {
                    action: GenerationAction::Check,
                    candidate: None,
                }
            })
        }
        "generation_submit" => {
            parse_generation_arguments::<EmptyGenerationToolRequest>(arguments).map(|_| {
                GenerationToolRequest {
                    action: GenerationAction::Submit,
                    candidate: None,
                }
            })
        }
        // Read old in-flight calls safely, but never advertise this overloaded protocol again.
        "generation_workspace" => parse_generation_arguments(arguments),
        _ => return GenerationToolOutcome::Continue("ERROR: unknown tool".into()),
    };
    match request {
        Ok(request) => workspace.execute(request),
        Err(error) => GenerationToolOutcome::MalformedArguments(format!(
            "ERROR: incomplete or invalid {name} arguments: {error}. The call was discarded; retry one complete schema-valid tool call in the clean checkpoint session."
        )),
    }
}

fn parse_generation_arguments<T: DeserializeOwned>(arguments: &Value) -> Result<T, serde_json::Error> {
    if let Some(text) = arguments.as_str() {
        serde_json::from_str(text)
    } else {
        serde_json::from_value(arguments.clone())
    }
}

fn simple_generation_tool(name: &str, description: &str) -> Value {
    json!({"type":"function","function":{
        "name":name,
        "description":description,
        "parameters":{"type":"object","additionalProperties":false,"properties":{}}
    }})
}

async fn review_candidate(
    candidate: &MovieGenerationProposal,
    context: GenerationReviewContext<'_>,
) -> Result<GenerationReview, StudioError> {
    let GenerationReviewContext {
        studio,
        request,
        project,
        model_runtime,
        cancel,
        app,
        workspace_root,
        exact_frame_analysis,
    } = context;
    emit(
        app,
        request,
        "turn-start",
        "reviewer",
        "Fresh-context review started",
    );
    let reviewer_settings = project.settings.runtime_settings_for(
        model_runtime.settings,
        model_runtime.reviewer_model_id,
    );
    let lease = tokio::select! {
        result = model_runtime.runtime.lease_model(
            model_runtime.reviewer_model_id,
            model_runtime.models,
            &reviewer_settings,
            app,
        ) => result.map_err(|error| StudioError::Planning(error.to_string()))?,
        _ = cancel.cancelled() => return Err(StudioError::Cancelled),
    };
    let messages = generation_review_messages(
        project,
        &request.task,
        candidate,
        exact_frame_analysis.as_ref(),
    )?;
    let mut movie_settings = project.settings.clone();
    movie_settings.temperature = 0.1;
    movie_settings.top_p = 0.9;
    movie_settings.top_k = 20;
    movie_settings.thinking_budget = request
        .thinking_level
        .unwrap_or(reviewer_settings.thinking_level)
        .budget_tokens(32_768);
    let mut on_event = |event| emit_stream_event(app, request, "reviewer", event);
    let review = agent_protocol::complete_tool_submission(
        &studio.http,
        agent_protocol::ToolSubmissionRequest {
            connection: &lease.connection,
            initial_messages: &messages,
            tool_name: "submit_generation_review",
            tool_description: "Submit the independent review of the generative-edit candidate.",
            response_format: generation_review_schema(),
            settings: &movie_settings,
            runtime_max_output_tokens: reviewer_settings.max_output_tokens,
            label: "generative edit review",
            audit_path: Some(&workspace_root.join("review-last-request.json")),
            cancel: Some(cancel),
            on_event: Some(&mut on_event),
        },
    )
    .await?;
    drop(lease);
    write_json_atomic(&workspace_root.join("review.json"), &review)?;
    Ok(review)
}

fn generation_review_messages(
    project: &MovieProject,
    task: &MovieGenerationTask,
    candidate: &MovieGenerationProposal,
    exact_frame_analysis: Option<&Value>,
) -> Result<Vec<Value>, StudioError> {
    Ok(vec![
        json!({"role":"system","content":super::prompts::generation_reviewer_system()}),
        json!({"role":"user","content":serde_json::to_string(&json!({
            "producerBrief":project.prompt,
            "task":task,
            "candidate":candidate,
            "exactFrameAnalysis":exact_frame_analysis,
            "plan":project.plan,
            "references":reference_manifest(&project.references),
        }))?}),
    ])
}

async fn prepare_frame_analysis(
    studio: &MovieStudio,
    request: &MovieGenerationAgentRequest,
    project: &MovieProject,
    model_runtime: MovieModelRuntime<'_>,
    cancel: &CancellationToken,
    app: Option<&AppHandle>,
    workspace_root: &Path,
) -> Result<Option<GenerationFrameAnalysis>, StudioError> {
    let anchors = task_frame_anchors(&request.task);
    if anchors.is_empty() {
        return Ok(None);
    }
    let Some(model_id) = request
        .frame_analyst_model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        emit(
            app,
            request,
            "activity",
            "frameAnalyst",
            "No local vision model is selected; the Director will use exact timecodes and storyline metadata only",
        );
        return Ok(None);
    };
    let model = model_runtime
        .models
        .iter()
        .find(|candidate| candidate.id == model_id)
        .ok_or_else(|| {
            StudioError::Invalid(
                "the selected frame-understanding model is no longer in the local model catalog"
                    .into(),
            )
        })?;
    if !model.supports_vision || model.mmproj_path.is_none() {
        return Err(StudioError::Invalid(format!(
            "{} cannot inspect endpoint frames because it has no verified local vision projector; choose a model marked vision or write the H3 direction yourself",
            model.name
        )));
    }

    fs::create_dir_all(workspace_root)?;
    let analysis_path = workspace_root.join("frame-analysis.json");
    if analysis_path.is_file() {
        if fs::metadata(&analysis_path)?.len() > 256 * 1024 {
            return Err(StudioError::Invalid(
                "the saved endpoint frame analysis exceeds the 256 KiB resume limit".into(),
            ));
        }
        let analysis: GenerationFrameAnalysis = serde_json::from_slice(&fs::read(&analysis_path)?)?;
        validate_frame_analysis(&analysis, &anchors)?;
        emit(
            app,
            request,
            "activity",
            "frameAnalyst",
            &format!(
                "Reusing the durable exact-frame observations from {}",
                analysis.model_name
            ),
        );
        emit(
            app,
            request,
            "token",
            "frameAnalyst",
            &frame_analysis_visible_summary(&analysis),
        );
        return Ok(Some(analysis));
    }

    check_cancel(cancel)?;
    emit(
        app,
        request,
        "turn-start",
        "frameAnalyst",
        &format!("{} is inspecting the exact endpoint PNGs", model.name),
    );
    let frames = capture_analysis_frames(studio, project, workspace_root, &anchors).await?;
    let messages = frame_analysis_messages(&request.task, &frames)?;
    let schema = frame_analysis_schema();
    let model_settings = project
        .settings
        .runtime_settings_for(model_runtime.settings, model_id);
    let mut movie_settings = project.settings.clone();
    movie_settings.temperature = 0.1;
    movie_settings.top_p = 0.9;
    movie_settings.top_k = 20;
    movie_settings.max_output_tokens = FRAME_ANALYSIS_OUTPUT_TOKENS;
    movie_settings.thinking_budget = request
        .thinking_level
        .unwrap_or(model_settings.thinking_level)
        .budget_tokens(FRAME_ANALYSIS_OUTPUT_TOKENS);
    write_json_atomic(
        &workspace_root.join("frame-analysis-request.json"),
        &json!({
            "modelId":model.id,
            "modelName":model.name,
            "systemPrompt":super::prompts::generation_frame_analyst_system(),
            "task":request.task,
            "images":frames.iter().map(|frame| json!({
                "role":frame.role,
                "anchor":frame.anchor,
                "path":frame.path,
                "sha256":frame.sha256,
                "bytes":frame.bytes,
                "mimeType":"image/png",
            })).collect::<Vec<_>>(),
            "responseSchema":schema,
            "settings":{
                "temperature":movie_settings.temperature,
                "topP":movie_settings.top_p,
                "topK":movie_settings.top_k,
                "maxOutputTokens":movie_settings.max_output_tokens,
                "thinkingBudget":movie_settings.thinking_budget,
            },
            "note":"The model received the exact bytes of each durable PNG listed above as local data:image/png content blocks. Paths and SHA-256 values make those bytes inspectable without duplicating base64 in this receipt."
        }),
    )?;

    let lease = match tokio::select! {
        result = model_runtime.runtime.lease_model(
            model_id,
            model_runtime.models,
            &model_settings,
            app,
        ) => result.map_err(|error| StudioError::Planning(error.to_string())),
        _ = cancel.cancelled() => Err(StudioError::Cancelled),
    } {
        Ok(lease) => lease,
        Err(error) => {
            persist_frame_analysis_error(workspace_root, model, &error)?;
            return Err(error);
        }
    };
    let mut on_event = |event| emit_stream_event(app, request, "frameAnalyst", event);
    let submission = agent_protocol::complete_tool_submission::<FrameAnalysisSubmission>(
        &studio.http,
        agent_protocol::ToolSubmissionRequest {
            connection: &lease.connection,
            initial_messages: &messages,
            tool_name: "submit_frame_analysis",
            tool_description:
                "Describe only the directly visible state of each exact endpoint frame.",
            response_format: schema,
            settings: &movie_settings,
            runtime_max_output_tokens: model_settings.max_output_tokens,
            label: "endpoint frame analysis",
            audit_path: None,
            cancel: Some(cancel),
            on_event: Some(&mut on_event),
        },
    )
    .await;
    drop(lease);
    let submission = match submission {
        Ok(value) => value,
        Err(error) => {
            persist_frame_analysis_error(workspace_root, model, &error)?;
            return Err(error);
        }
    };
    let analysis = assemble_frame_analysis(model, &frames, submission)?;
    write_json_atomic(&analysis_path, &analysis)?;
    let _ = fs::remove_file(workspace_root.join("frame-analysis-error.json"));
    emit(
        app,
        request,
        "token",
        "frameAnalyst",
        &frame_analysis_visible_summary(&analysis),
    );
    emit(
        app,
        request,
        "activity",
        "frameAnalyst",
        "Exact endpoint observations are durable and available to the Director and reviewer",
    );
    Ok(Some(analysis))
}

fn frame_analysis_visible_summary(analysis: &GenerationFrameAnalysis) -> String {
    let mut summary = format!("Exact endpoint observations — {}\n", analysis.model_name);
    for frame in &analysis.frames {
        summary.push_str(&format!(
            "\n{} frame at {:.3}s\n{}\nVisible state: {}\nComposition: {}\n",
            frame.role.to_ascii_uppercase(),
            frame.anchor.time_seconds,
            frame.description,
            frame.visible_action,
            frame.composition,
        ));
        if !frame.continuity_facts.is_empty() {
            summary.push_str("Continuity facts:\n");
            for fact in &frame.continuity_facts {
                summary.push_str(&format!("- {fact}\n"));
            }
        }
        if !frame.uncertainties.is_empty() {
            summary.push_str("Uncertain from this still:\n");
            for uncertainty in &frame.uncertainties {
                summary.push_str(&format!("- {uncertainty}\n"));
            }
        }
    }
    summary
}

fn task_frame_anchors(task: &MovieGenerationTask) -> Vec<(&'static str, MovieFrameAnchor)> {
    match task {
        MovieGenerationTask::Transition {
            first_anchor,
            last_anchor,
            ..
        } => first_anchor
            .iter()
            .cloned()
            .map(|anchor| ("first", anchor))
            .chain(last_anchor.iter().cloned().map(|anchor| ("last", anchor)))
            .collect(),
        MovieGenerationTask::ShotVersion { .. } => Vec::new(),
    }
}

async fn capture_analysis_frames(
    studio: &MovieStudio,
    project: &MovieProject,
    workspace_root: &Path,
    anchors: &[(&'static str, MovieFrameAnchor)],
) -> Result<Vec<CapturedAnalysisFrame>, StudioError> {
    let directory = workspace_root.join("endpoint-frames");
    fs::create_dir_all(&directory)?;
    let project_dir = studio.project_dir(&project.id);
    let mut frames = Vec::with_capacity(anchors.len());
    for (role, anchor) in anchors {
        let source = super::resolve_frame_anchor(&project_dir, project, anchor)?;
        let path = directory.join(format!("{role}.png"));
        super::extract_exact_frame(&source.path, anchor.time_seconds, &path).await?;
        let metadata = fs::metadata(&path)?;
        if metadata.len() == 0 || metadata.len() > MAX_ANCHOR_IMAGE_BYTES {
            return Err(StudioError::Invalid(format!(
                "the exact {role} endpoint PNG is empty or exceeds the 16 MiB local vision limit"
            )));
        }
        let (_, sha256) = super::hash_file(&path)?;
        frames.push(CapturedAnalysisFrame {
            role,
            anchor: anchor.clone(),
            path,
            sha256,
            bytes: metadata.len(),
        });
    }
    Ok(frames)
}

fn frame_analysis_messages(
    task: &MovieGenerationTask,
    frames: &[CapturedAnalysisFrame],
) -> Result<Vec<Value>, StudioError> {
    let mut content = vec![json!({
        "type":"text",
        "text":format!(
            "Analyze the exact endpoint frames for this generative edit. The role label immediately before each image identifies it. Task metadata is context only; pixels are authoritative. Task: {}",
            serde_json::to_string(task)?
        )
    })];
    for frame in frames {
        content.push(json!({
            "type":"text",
            "text":format!(
                "{} ENDPOINT — exact source time {:.6} seconds — {}",
                frame.role.to_ascii_uppercase(),
                frame.anchor.time_seconds,
                frame.anchor.label.as_deref().unwrap_or("unlabelled endpoint")
            )
        }));
        let bytes = fs::read(&frame.path)?;
        content.push(json!({
            "type":"image_url",
            "image_url":{"url":format!("data:image/png;base64,{}", STANDARD.encode(bytes))}
        }));
    }
    Ok(vec![
        json!({"role":"system","content":super::prompts::generation_frame_analyst_system()}),
        json!({"role":"user","content":content}),
    ])
}

fn assemble_frame_analysis(
    model: &crate::model::ModelInfo,
    frames: &[CapturedAnalysisFrame],
    submission: FrameAnalysisSubmission,
) -> Result<GenerationFrameAnalysis, StudioError> {
    if submission.frames.len() != frames.len() {
        return Err(StudioError::Invalid(format!(
            "frame analyst returned {} observations for {} endpoint images",
            submission.frames.len(),
            frames.len()
        )));
    }
    let mut observations = Vec::with_capacity(frames.len());
    for frame in frames {
        let submitted = submission
            .frames
            .iter()
            .find(|candidate| candidate.role == frame.role)
            .ok_or_else(|| {
                StudioError::Invalid(format!(
                    "frame analyst omitted the exact {} endpoint",
                    frame.role
                ))
            })?;
        validate_frame_observation_submission(submitted)?;
        observations.push(GenerationFrameObservation {
            role: frame.role.into(),
            anchor: frame.anchor.clone(),
            image_path: frame.path.to_string_lossy().into_owned(),
            image_sha256: frame.sha256.clone(),
            description: submitted.description.trim().into(),
            visible_action: submitted.visible_action.trim().into(),
            composition: submitted.composition.trim().into(),
            continuity_facts: submitted
                .continuity_facts
                .iter()
                .map(|value| value.trim().to_string())
                .collect(),
            uncertainties: submitted
                .uncertainties
                .iter()
                .map(|value| value.trim().to_string())
                .collect(),
        });
    }
    let analysis = GenerationFrameAnalysis {
        schema_version: 1,
        model_id: model.id.clone(),
        model_name: model.name.clone(),
        frames: observations,
        created_at: Utc::now().to_rfc3339(),
    };
    validate_frame_analysis(
        &analysis,
        &frames
            .iter()
            .map(|frame| (frame.role, frame.anchor.clone()))
            .collect::<Vec<_>>(),
    )?;
    Ok(analysis)
}

fn validate_frame_observation_submission(
    observation: &FrameObservationSubmission,
) -> Result<(), StudioError> {
    if !matches!(observation.role.as_str(), "first" | "last")
        || !bounded_frame_text(&observation.description, 20, 8_000)
        || !bounded_frame_text(&observation.visible_action, 3, 2_000)
        || !bounded_frame_text(&observation.composition, 3, 2_000)
        || observation.continuity_facts.len() > 16
        || observation.uncertainties.len() > 12
        || observation
            .continuity_facts
            .iter()
            .chain(&observation.uncertainties)
            .any(|value| !bounded_frame_text(value, 1, 500))
    {
        return Err(StudioError::Invalid(
            "frame analyst returned an incomplete or oversized endpoint observation".into(),
        ));
    }
    Ok(())
}

fn bounded_frame_text(value: &str, minimum_chars: usize, maximum_chars: usize) -> bool {
    let length = value.trim().chars().count();
    (minimum_chars..=maximum_chars).contains(&length)
}

fn validate_frame_analysis(
    analysis: &GenerationFrameAnalysis,
    anchors: &[(&'static str, MovieFrameAnchor)],
) -> Result<(), StudioError> {
    if analysis.schema_version != 1 || analysis.frames.len() != anchors.len() {
        return Err(StudioError::Invalid(
            "the saved endpoint analysis does not match this generative edit".into(),
        ));
    }
    for (role, anchor) in anchors {
        let observation = analysis
            .frames
            .iter()
            .find(|frame| frame.role == *role)
            .ok_or_else(|| {
                StudioError::Invalid(format!(
                    "the saved endpoint analysis is missing the {role} frame"
                ))
            })?;
        if observation.anchor != *anchor
            || !bounded_frame_text(&observation.description, 20, 8_000)
            || !bounded_frame_text(&observation.visible_action, 3, 2_000)
            || !bounded_frame_text(&observation.composition, 3, 2_000)
            || !Path::new(&observation.image_path).is_file()
        {
            return Err(StudioError::Invalid(format!(
                "the saved {role} endpoint analysis is stale or incomplete"
            )));
        }
        let (_, current_sha256) = super::hash_file(Path::new(&observation.image_path))?;
        if current_sha256 != observation.image_sha256 {
            return Err(StudioError::Invalid(format!(
                "the saved {role} endpoint PNG no longer matches its durable analysis receipt"
            )));
        }
    }
    Ok(())
}

fn persist_frame_analysis_error(
    workspace_root: &Path,
    model: &crate::model::ModelInfo,
    error: &StudioError,
) -> Result<(), StudioError> {
    write_json_atomic(
        &workspace_root.join("frame-analysis-error.json"),
        &json!({
            "modelId":model.id,
            "modelName":model.name,
            "error":error.to_string(),
            "at":Utc::now().to_rfc3339(),
            "resume":"The exact endpoint PNGs and request manifest are preserved. Resume Director to retry the frame-analysis stage."
        }),
    )
}

fn frame_analysis_schema() -> Value {
    json!({"type":"json_schema","json_schema":{"name":"kestrel_frame_analysis","strict":true,"schema":{
        "type":"object","additionalProperties":false,"properties":{
            "frames":{"type":"array","minItems":1,"maxItems":2,"items":{
                "type":"object","additionalProperties":false,"properties":{
                    "role":{"type":"string","enum":["first","last"]},
                    "description":{"type":"string","minLength":20,"maxLength":8000},
                    "visibleAction":{"type":"string","minLength":3,"maxLength":2000},
                    "composition":{"type":"string","minLength":3,"maxLength":2000},
                    "continuityFacts":{"type":"array","maxItems":16,"items":{"type":"string","minLength":1,"maxLength":500}},
                    "uncertainties":{"type":"array","maxItems":12,"items":{"type":"string","minLength":1,"maxLength":500}}
                },"required":["role","description","visibleAction","composition","continuityFacts","uncertainties"]
            }}
        },"required":["frames"]
    }}})
}

fn validate_generation_request(request: &MovieGenerationAgentRequest) -> Result<(), StudioError> {
    super::validate_id(&request.request_id)?;
    super::validate_id(&request.project_id)?;
    match &request.task {
        MovieGenerationTask::ShotVersion { clip_id, direction } => {
            if clip_id.trim().is_empty() || !has_meaningful_prose(direction, 3) {
                return Err(StudioError::Invalid(
                    "shot-version assistance needs a selected shot and a producer direction".into(),
                ));
            }
        }
        MovieGenerationTask::Transition {
            position,
            first_anchor,
            last_anchor,
            direction,
            duration_seconds,
        } => {
            validate_transition_anchors(*position, first_anchor.as_ref(), last_anchor.as_ref())?;
            if !has_meaningful_prose(direction, 3) {
                return Err(StudioError::Invalid(
                    "transition assistance needs a producer direction".into(),
                ));
            }
            if !duration_seconds.is_finite() || !(1.0..=15.0).contains(duration_seconds) {
                return Err(StudioError::Invalid(
                    "transition duration must be between 1 and 15 seconds".into(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_anchor(anchor: &MovieFrameAnchor) -> Result<(), StudioError> {
    if anchor.edit_id.trim().is_empty()
        || !anchor.time_seconds.is_finite()
        || anchor.time_seconds < 0.0
    {
        return Err(StudioError::Invalid(
            "frame anchors require a valid storyline edit and non-negative time".into(),
        ));
    }
    Ok(())
}

fn validate_transition_anchors(
    position: MovieTransitionPosition,
    first: Option<&MovieFrameAnchor>,
    last: Option<&MovieFrameAnchor>,
) -> Result<(), StudioError> {
    if let Some(anchor) = first {
        validate_anchor(anchor)?;
    }
    if let Some(anchor) = last {
        validate_anchor(anchor)?;
    }
    let valid = match position {
        MovieTransitionPosition::Before => first.is_none() && last.is_some(),
        MovieTransitionPosition::Between => first.is_some() && last.is_some(),
        MovieTransitionPosition::After => first.is_some() && last.is_none(),
    };
    if !valid {
        return Err(StudioError::Invalid(
            "before needs only a story end frame, between needs both frames, and after needs only a story start frame"
                .into(),
        ));
    }
    Ok(())
}

fn validate_generation_candidate(
    project: &MovieProject,
    task: &MovieGenerationTask,
    proposal: &MovieGenerationProposal,
) -> Result<(), StudioError> {
    if !has_meaning_proposal_text(&proposal.summary) {
        return Err(StudioError::Invalid(
            "candidate summary needs clear producer-facing prose".into(),
        ));
    }
    match (task, &proposal.candidate) {
        (
            MovieGenerationTask::ShotVersion { clip_id, .. },
            MovieGenerationCandidate::ShotVersion {
                clip_id: candidate_id,
                clip,
                ..
            },
        ) => {
            if clip_id != candidate_id || clip.id != *clip_id {
                return Err(StudioError::Invalid(
                    "shot candidate must preserve the selected clip id".into(),
                ));
            }
            let mut plan = project.plan.clone().ok_or_else(|| {
                StudioError::Invalid("shot assistance requires an approved movie plan".into())
            })?;
            let index = plan
                .clips
                .iter()
                .position(|planned| planned.id == *clip_id)
                .ok_or_else(|| {
                    StudioError::Invalid("selected shot is absent from the plan".into())
                })?;
            plan.clips[index] = clip.clone();
            prepare_producer_plan(project, &mut plan)?;
            let issues = prompt_quality_issues(&plan, &project.references);
            if !issues.is_empty() {
                return Err(StudioError::Invalid(issues.join(" ")));
            }
        }
        (
            MovieGenerationTask::Transition { .. },
            MovieGenerationCandidate::Transition {
                motion_prompt,
                duration_seconds,
                camera_motion,
                subject_motion,
                transition_notes,
                ..
            },
        ) => {
            let words = motion_prompt.split_whitespace().count();
            if !(120..=450).contains(&words) {
                return Err(StudioError::Invalid(format!(
                    "transition renderer direction has {words} words; H3 directions require 120-450"
                )));
            }
            if !duration_seconds.is_finite() || !(1.0..=15.0).contains(duration_seconds) {
                return Err(StudioError::Invalid(
                    "transition candidate duration must be between 1 and 15 seconds".into(),
                ));
            }
            if !has_meaningful_prose(camera_motion, 3)
                || !has_meaningful_prose(subject_motion, 3)
                || !has_meaningful_prose(transition_notes, 3)
            {
                return Err(StudioError::Invalid(
                    "transition candidate needs explicit camera, subject, and continuity direction"
                        .into(),
                ));
            }
            if super::contains_reference_tag(motion_prompt)
                || project
                    .references
                    .iter()
                    .any(|reference| motion_prompt.contains(&reference.asset_id))
            {
                return Err(StudioError::Invalid(
                    "transition renderer prose cannot contain internal reference tags or asset ids"
                        .into(),
                ));
            }
        }
        _ => {
            return Err(StudioError::Invalid(
                "candidate kind does not match the requested generative edit".into(),
            ));
        }
    }
    Ok(())
}

fn has_meaning_proposal_text(value: &str) -> bool {
    value.len() <= 4_000 && has_meaningful_prose(value, 3)
}

fn generation_context(
    project: &MovieProject,
    task: &MovieGenerationTask,
    frame_analysis: Option<&GenerationFrameAnalysis>,
) -> Result<Value, StudioError> {
    let selected = match task {
        MovieGenerationTask::ShotVersion { clip_id, .. } => json!({
            "plannedClip":project.plan.as_ref().and_then(|plan| plan.clips.iter().find(|clip| clip.id == *clip_id)),
            "renderedClip":project.clips.iter().find(|clip| clip.id == *clip_id),
        }),
        MovieGenerationTask::Transition {
            position,
            first_anchor,
            last_anchor,
            ..
        } => json!({
            "position":position,
            "first":first_anchor.as_ref().map(|anchor| anchor_context(project, anchor)).transpose()?,
            "last":last_anchor.as_ref().map(|anchor| anchor_context(project, anchor)).transpose()?,
        }),
    };
    Ok(json!({
        "producerBrief":project.prompt,
        "task":task,
        "selected":selected,
        "exactFrameAnalysis":frame_analysis,
        "plan":project.plan,
        "currentStoryline":project.edit,
        "references":reference_manifest(&project.references),
        "settings":{"width":project.settings.width,"height":project.settings.height,"steps":project.settings.steps},
    }))
}

fn anchor_context(project: &MovieProject, anchor: &MovieFrameAnchor) -> Result<Value, StudioError> {
    let edit = project
        .edit
        .clips
        .iter()
        .find(|edit| edit.id == anchor.edit_id)
        .ok_or_else(|| {
            StudioError::Invalid("frame anchor edit is absent from the storyline".into())
        })?;
    let clip = project
        .clips
        .iter()
        .find(|clip| clip.id == edit.clip_id)
        .ok_or_else(|| StudioError::Invalid("frame anchor source is absent from masters".into()))?;
    let duration_seconds = if edit.source_version_id.is_empty() {
        clip.duration_seconds
    } else {
        clip.versions
            .iter()
            .find(|version| version.id == edit.source_version_id)
            .ok_or_else(|| StudioError::Invalid("frame anchor preserved version is absent".into()))?
            .duration_seconds
    };
    if anchor.time_seconds > f64::from(duration_seconds) + 0.001 {
        return Err(StudioError::Invalid(
            "frame anchor time exceeds its source duration".into(),
        ));
    }
    Ok(json!({"anchor":anchor,"edit":edit,"clip":clip}))
}

fn generation_candidate_schema(task: &MovieGenerationTask) -> Value {
    let checklist = json!({"type":"array","maxItems":12,"items":{"type":"string","minLength":3,"maxLength":400}});
    match task {
        MovieGenerationTask::ShotVersion { .. } => json!({
            "type":"object","additionalProperties":false,
            "properties":{
                "summary":{"type":"string","minLength":10,"maxLength":4000},
                "reviewSummary":{"type":"string","maxLength":4000},
                "candidate":{"type":"object","additionalProperties":false,"properties":{
                    "kind":{"const":"shotVersion"},"clipId":{"type":"string"},
                    "clip":planned_clip_schema(),"checklist":checklist
                },"required":["kind","clipId","clip","checklist"]}
            },"required":["summary","reviewSummary","candidate"]
        }),
        MovieGenerationTask::Transition { .. } => json!({
            "type":"object","additionalProperties":false,
            "properties":{
                "summary":{"type":"string","minLength":10,"maxLength":4000},
                "reviewSummary":{"type":"string","maxLength":4000},
                "candidate":{"type":"object","additionalProperties":false,"properties":{
                    "kind":{"const":"transition"},
                    "motionPrompt":{"type":"string","minLength":300,"maxLength":65536},
                    "durationSeconds":{"type":"number","minimum":1,"maximum":15},
                    "cameraMotion":{"type":"string","minLength":10,"maxLength":4000},
                    "subjectMotion":{"type":"string","minLength":10,"maxLength":4000},
                    "transitionNotes":{"type":"string","minLength":10,"maxLength":4000},
                    "checklist":checklist
                },"required":["kind","motionPrompt","durationSeconds","cameraMotion","subjectMotion","transitionNotes","checklist"]}
            },"required":["summary","reviewSummary","candidate"]
        }),
    }
}

fn planned_clip_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"properties":{
        "id":{"type":"string"},"title":{"type":"string"},"purpose":{"type":"string"},
        "durationSeconds":{"type":"number","minimum":5,"maximum":15},"prompt":{"type":"string"},
        "continuityIn":{"type":"string"},"continuityOut":{"type":"string"},"transition":{"type":"string"},
        "usePreviousFrame":{"type":"boolean"},"sourceRefs":{"type":"array","items":{"type":"string"}},
        "referenceIds":{"type":"array","items":{"type":"string"}}
    },"required":["id","title","purpose","durationSeconds","prompt","continuityIn","continuityOut","transition","usePreviousFrame","sourceRefs","referenceIds"]})
}

fn generation_review_schema() -> Value {
    json!({"type":"json_schema","json_schema":{"name":"kestrel_generation_review","strict":true,"schema":{
        "type":"object","additionalProperties":false,"properties":{
            "approved":{"type":"boolean"},"summary":{"type":"string","minLength":10,"maxLength":4000},
            "issues":{"type":"array","maxItems":24,"items":{"type":"string","minLength":3,"maxLength":2000}}
        },"required":["approved","summary","issues"]
    }}})
}

fn emit_stream_event(
    app: Option<&AppHandle>,
    request: &MovieGenerationAgentRequest,
    model_role: &str,
    event: agent_protocol::StreamEvent,
) {
    match event {
        agent_protocol::StreamEvent::Content(content) => {
            emit(app, request, "token", model_role, &content)
        }
        agent_protocol::StreamEvent::Reasoning(content) => {
            emit(app, request, "reasoning", model_role, &content)
        }
        agent_protocol::StreamEvent::ToolArgumentsStarted => emit(
            app,
            request,
            "activity",
            model_role,
            "Streaming a typed generative workspace action",
        ),
        agent_protocol::StreamEvent::ToolArguments(content) => {
            emit(app, request, "advanced-token", model_role, &content)
        }
    }
}

fn emit(
    app: Option<&AppHandle>,
    request: &MovieGenerationAgentRequest,
    kind: &str,
    model_role: &str,
    content: &str,
) {
    let Some(app) = app else { return };
    let _ = app.emit(
        "movie-generation-agent",
        MovieGenerationAgentEvent {
            request_id: request.request_id.clone(),
            project_id: request.project_id.clone(),
            kind: kind.into(),
            model_role: model_role.into(),
            content: content.into(),
            at: Utc::now().to_rfc3339(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::{ClipEdit, MovieSettings, MovieTransitionPlacement};

    #[test]
    fn frontend_camel_case_generation_contract_round_trips_and_keeps_legacy_tasks_readable() {
        let request: MovieGenerationAgentRequest = serde_json::from_value(json!({
            "requestId":"request-123",
            "projectId":"project-123",
            "task":{
                "kind":"transition",
                "position":"between",
                "firstAnchor":{"editId":"edit-1","timeSeconds":2.0},
                "lastAnchor":{"editId":"edit-1","timeSeconds":5.0},
                "direction":"Replace the failed stone lift with a clean physical action.",
                "durationSeconds":6.0
            },
            "thinkingLevel":"high",
            "frameAnalystModelId":"vision-1"
        }))
        .unwrap();
        let MovieGenerationTask::Transition {
            first_anchor,
            last_anchor,
            duration_seconds,
            ..
        } = &request.task
        else {
            panic!("expected transition task");
        };
        assert_eq!(first_anchor.as_ref().unwrap().edit_id, "edit-1");
        assert_eq!(last_anchor.as_ref().unwrap().time_seconds, 5.0);
        assert_eq!(*duration_seconds, 6.0);

        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(encoded.pointer("/task/durationSeconds"), Some(&json!(6.0)));
        assert!(encoded.pointer("/task/duration_seconds").is_none());
        assert!(encoded.pointer("/task/firstAnchor").is_some());
        assert_eq!(encoded["frameAnalystModelId"], "vision-1");

        let shot_task: MovieGenerationTask = serde_json::from_value(json!({
            "kind":"shotVersion",
            "clipId":"clip-7",
            "direction":"Keep the framing but repair the failed action."
        }))
        .unwrap();
        assert!(matches!(
            shot_task,
            MovieGenerationTask::ShotVersion { clip_id, .. } if clip_id == "clip-7"
        ));

        let candidate: MovieGenerationCandidate = serde_json::from_value(json!({
            "kind":"transition",
            "motionPrompt":"A complete H3 motion direction.",
            "durationSeconds":6.0,
            "cameraMotion":"The camera holds its axis.",
            "subjectMotion":"The subject completes the lift.",
            "transitionNotes":"Match both endpoint frames.",
            "checklist":[]
        }))
        .unwrap();
        let encoded_candidate = serde_json::to_value(candidate).unwrap();
        assert!(encoded_candidate.get("motionPrompt").is_some());
        assert!(encoded_candidate.get("transitionNotes").is_some());

        let legacy: MovieGenerationTask = serde_json::from_value(json!({
            "kind":"transition",
            "position":"after",
            "first_anchor":{"editId":"edit-1","timeSeconds":5.0},
            "direction":"Continue the story after the preserved ending.",
            "duration_seconds":5.0
        }))
        .unwrap();
        assert!(matches!(
            legacy,
            MovieGenerationTask::Transition {
                duration_seconds: 5.0,
                ..
            }
        ));
    }

    #[test]
    fn exact_frame_analysis_is_bound_to_each_anchor_and_enters_authoritative_context() {
        let root = tempfile::tempdir().unwrap();
        let first_path = root.path().join("first.png");
        let last_path = root.path().join("last.png");
        fs::write(&first_path, b"first exact frame").unwrap();
        fs::write(&last_path, b"last exact frame").unwrap();
        let (_, first_sha256) = super::super::hash_file(&first_path).unwrap();
        let (_, last_sha256) = super::super::hash_file(&last_path).unwrap();
        let first_anchor = MovieFrameAnchor {
            edit_id: "edit-1".into(),
            time_seconds: 4.0,
            label: Some("Run out".into()),
        };
        let last_anchor = MovieFrameAnchor {
            edit_id: "edit-2".into(),
            time_seconds: 1.0,
            label: Some("Wide in".into()),
        };
        let frames = vec![
            CapturedAnalysisFrame {
                role: "first",
                anchor: first_anchor.clone(),
                path: first_path,
                sha256: first_sha256,
                bytes: 17,
            },
            CapturedAnalysisFrame {
                role: "last",
                anchor: last_anchor.clone(),
                path: last_path,
                sha256: last_sha256.clone(),
                bytes: 16,
            },
        ];
        let submission = FrameAnalysisSubmission {
            frames: vec![
                frame_submission(
                    "first",
                    "A football player leans over a dark stone with both hands touching it.",
                ),
                frame_submission(
                    "last",
                    "The same player is upright while the stone remains on the workbench.",
                ),
            ],
        };
        let model = vision_model();
        let analysis = assemble_frame_analysis(&model, &frames, submission).unwrap();
        assert_eq!(analysis.frames[0].anchor, first_anchor);
        assert_eq!(analysis.frames[1].image_sha256, last_sha256);

        let task = MovieGenerationTask::Transition {
            position: MovieTransitionPosition::Between,
            first_anchor: Some(first_anchor),
            last_anchor: Some(last_anchor),
            direction: "Show the player try and fail to lift the stone.".into(),
            duration_seconds: 5.0,
        };
        let context = generation_context(&test_project(), &task, Some(&analysis)).unwrap();
        assert_eq!(
            context["exactFrameAnalysis"]["frames"][0]["description"],
            "A football player leans over a dark stone with both hands touching it."
        );

        let analysis_value = serde_json::to_value(&analysis).unwrap();
        let review_messages = generation_review_messages(
            &test_project(),
            &task,
            &transition_proposal(),
            Some(&analysis_value),
        )
        .unwrap();
        let reviewer_context: Value =
            serde_json::from_str(review_messages[1]["content"].as_str().unwrap()).unwrap();
        assert_eq!(
            reviewer_context["exactFrameAnalysis"]["frames"][1]["imageSha256"],
            last_sha256
        );
    }

    #[test]
    fn multimodal_frame_request_labels_images_separately_and_uses_exact_png_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("first.png");
        fs::write(&path, b"png bytes").unwrap();
        let task = MovieGenerationTask::Transition {
            position: MovieTransitionPosition::After,
            first_anchor: Some(MovieFrameAnchor {
                edit_id: "edit-1".into(),
                time_seconds: 4.5,
                label: Some("Story end".into()),
            }),
            last_anchor: None,
            direction: "Continue after the exact ending frame.".into(),
            duration_seconds: 5.0,
        };
        let messages = frame_analysis_messages(
            &task,
            &[CapturedAnalysisFrame {
                role: "first",
                anchor: task_frame_anchors(&task)[0].1.clone(),
                path,
                sha256: "sha".into(),
                bytes: 9,
            }],
        )
        .unwrap();
        let content = messages[1]["content"].as_array().unwrap();
        assert!(content[1]["text"]
            .as_str()
            .unwrap()
            .contains("FIRST ENDPOINT"));
        assert_eq!(content[2]["type"], "image_url");
        assert!(content[2]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,"));
    }

    #[test]
    fn exact_frame_observations_have_a_producer_readable_stream_summary() {
        let analysis = GenerationFrameAnalysis {
            schema_version: 1,
            model_id: "vision-1".into(),
            model_name: "Local Vision".into(),
            frames: vec![GenerationFrameObservation {
                role: "first".into(),
                anchor: MovieFrameAnchor {
                    edit_id: "edit-1".into(),
                    time_seconds: 3.25,
                    label: Some("Out".into()),
                },
                image_path: "first.png".into(),
                image_sha256: "sha".into(),
                description: "A sea lion leans over a stone beside the window.".into(),
                visible_action: "Its front flippers rest beside the stone.".into(),
                composition: "A side-on medium shot keeps the animal and stone visible.".into(),
                continuity_facts: vec!["The stone is still on the table.".into()],
                uncertainties: vec!["The still does not establish whether it tried to lift it.".into()],
            }],
            created_at: "2026-08-20T00:00:00Z".into(),
        };
        let summary = frame_analysis_visible_summary(&analysis);
        assert!(summary.contains("Exact endpoint observations — Local Vision"));
        assert!(summary.contains("FIRST frame at 3.250s"));
        assert!(summary.contains("The stone is still on the table."));
        assert!(summary.contains("The still does not establish"));
    }

    #[test]
    fn workspace_tool_requires_two_clean_checks_after_the_last_write() {
        let root = tempfile::tempdir().unwrap();
        let project = test_project();
        let task = MovieGenerationTask::Transition {
            position: MovieTransitionPosition::Between,
            first_anchor: Some(MovieFrameAnchor {
                edit_id: "edit-1".into(),
                time_seconds: 4.9,
                label: None,
            }),
            last_anchor: Some(MovieFrameAnchor {
                edit_id: "edit-2".into(),
                time_seconds: 0.0,
                label: None,
            }),
            direction: "Move the running player into the quiet wide shot.".into(),
            duration_seconds: 5.0,
        };
        let mut workspace =
            GenerationWorkspace::open(root.path().into(), project, task, None).unwrap();
        let proposal = transition_proposal();
        assert!(matches!(
            workspace.execute(GenerationToolRequest {
                action: GenerationAction::WriteCandidate,
                candidate: Some(serde_json::to_value(proposal).unwrap()),
            }),
            GenerationToolOutcome::Continue(_)
        ));
        assert!(matches!(
            workspace.execute(GenerationToolRequest {
                action: GenerationAction::Submit,
                candidate: None,
            }),
            GenerationToolOutcome::Continue(message) if message.starts_with("ERROR:")
        ));
        for _ in 0..2 {
            workspace.execute(GenerationToolRequest {
                action: GenerationAction::Check,
                candidate: None,
            });
        }
        assert!(matches!(
            workspace.execute(GenerationToolRequest {
                action: GenerationAction::Submit,
                candidate: None,
            }),
            GenerationToolOutcome::Submitted(_)
        ));
    }

    #[test]
    fn action_tools_use_empty_objects_and_read_context_is_not_duplicated() {
        let root = tempfile::tempdir().unwrap();
        let project = test_project();
        let task = MovieGenerationTask::ShotVersion {
            clip_id: "clip-001".into(),
            direction: "Preserve the ending while tightening the action.".into(),
        };
        let mut workspace =
            GenerationWorkspace::open(root.path().into(), project, task, None).unwrap();
        let tools = workspace.tools();
        let tools = tools
            .as_array()
            .unwrap();
        let submit = tools
            .iter()
            .find(|tool| tool.pointer("/function/name") == Some(&json!("generation_submit")))
            .unwrap();
        assert_eq!(submit["function"]["parameters"]["properties"], json!({}));
        assert!(submit["function"]["parameters"].get("oneOf").is_none());

        let read_call = json!({
            "function":{"name":"generation_read_context","arguments":"{}"}
        });
        let GenerationToolOutcome::Continue(message) = execute_call(&mut workspace, &read_call)
        else {
            panic!("generation_read_context did not return its acknowledgement");
        };
        assert!(message.len() < 512);
        assert!(!message.contains("producerBrief"));

        let submit_call = json!({
            "function":{"name":"generation_submit","arguments":"{}"}
        });
        let GenerationToolOutcome::Continue(message) = execute_call(&mut workspace, &submit_call)
        else {
            panic!("submit without a candidate should fail natively, not at JSON decoding");
        };
        assert!(message.contains("write a candidate"));
    }

    #[test]
    fn truncated_candidate_write_is_checkpointed_for_a_clean_session_retry() {
        let root = tempfile::tempdir().unwrap();
        let project = test_project();
        let task = MovieGenerationTask::ShotVersion {
            clip_id: "clip-001".into(),
            direction: "Preserve the ending while tightening the action.".into(),
        };
        let mut workspace =
            GenerationWorkspace::open(root.path().into(), project, task, None).unwrap();
        let call = json!({
            "function": {
                "name": "generation_write_candidate",
                "arguments": "{\"candidate\":{\"summary\":\"unfinished"
            }
        });
        assert!(matches!(
            execute_call(&mut workspace, &call),
            GenerationToolOutcome::MalformedArguments(message)
                if message.contains("clean checkpoint session")
        ));
    }

    #[test]
    fn task_kind_cannot_be_swapped_by_model_output() {
        let project = test_project();
        let task = MovieGenerationTask::ShotVersion {
            clip_id: "clip-001".into(),
            direction: "Make the camera settle sooner and preserve the ending.".into(),
        };
        assert!(validate_generation_candidate(&project, &task, &transition_proposal()).is_err());
    }

    #[test]
    fn a_checkpoint_request_id_cannot_be_reused_for_a_different_task() {
        let root = tempfile::tempdir().unwrap();
        let project = test_project();
        GenerationWorkspace::open(
            root.path().into(),
            project.clone(),
            MovieGenerationTask::ShotVersion {
                clip_id: "clip-001".into(),
                direction: "Preserve the player and settle the camera sooner.".into(),
            },
            None,
        )
        .unwrap();
        let error = match GenerationWorkspace::open(
            root.path().into(),
            project,
            MovieGenerationTask::ShotVersion {
                clip_id: "clip-002".into(),
                direction: "Change a different shot under the same checkpoint.".into(),
            },
            None,
        ) {
            Ok(_) => panic!("a checkpoint accepted a different task"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("different generative-edit task"));
    }

    #[test]
    fn replacement_transition_uses_exact_storyline_edit_ids_and_keeps_edge_fragments() {
        let mut project = test_project();
        let first = MovieFrameAnchor {
            edit_id: "edit-1".into(),
            time_seconds: 3.0,
            label: None,
        };
        let last = MovieFrameAnchor {
            edit_id: "edit-2".into(),
            time_seconds: 1.0,
            label: None,
        };
        let transition = ClipEdit {
            id: "edit-transition".into(),
            clip_id: "transition".into(),
            enabled: true,
            order: 0,
            trim_start: 0.0,
            trim_end: 0.0,
            audio_gain: 1.0,
            source_version_id: String::new(),
            speed: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            audio_fade_in: 0.0,
            audio_fade_out: 0.0,
            label: String::new(),
            notes: String::new(),
        };
        super::super::validate_transition_placement(
            &project,
            MovieTransitionPosition::Between,
            Some(&first),
            Some(&last),
            MovieTransitionPlacement::ReplaceRange,
        )
        .unwrap();
        super::super::replace_storyline_range_with_transition(
            &mut project,
            &first,
            &last,
            transition,
        )
        .unwrap();
        assert_eq!(project.edit.clips.len(), 3);
        assert_eq!(project.edit.clips[0].id, "edit-1");
        assert_eq!(project.edit.clips[0].trim_end, 2.0);
        assert_eq!(project.edit.clips[1].id, "edit-transition");
        assert_eq!(project.edit.clips[2].id, "edit-2");
        assert_eq!(project.edit.clips[2].trim_start, 1.0);
    }

    #[test]
    fn replacement_transition_rejects_backward_storyline_ranges() {
        let project = test_project();
        let error = super::super::validate_transition_placement(
            &project,
            MovieTransitionPosition::Between,
            Some(&MovieFrameAnchor {
                edit_id: "edit-2".into(),
                time_seconds: 1.0,
                label: None,
            }),
            Some(&MovieFrameAnchor {
                edit_id: "edit-1".into(),
                time_seconds: 3.0,
                label: None,
            }),
            MovieTransitionPlacement::ReplaceRange,
        )
        .unwrap_err();
        assert!(error.to_string().contains("run forward"));
    }

    #[test]
    fn edge_transitions_accept_only_the_story_facing_endpoint_and_placement() {
        let project = test_project();
        let first = MovieFrameAnchor {
            edit_id: "edit-1".into(),
            time_seconds: 0.0,
            label: None,
        };
        let last = MovieFrameAnchor {
            edit_id: "edit-2".into(),
            time_seconds: 4.9,
            label: None,
        };
        super::super::validate_transition_placement(
            &project,
            MovieTransitionPosition::Before,
            None,
            Some(&first),
            MovieTransitionPlacement::InsertBeforeRight,
        )
        .unwrap();
        super::super::validate_transition_placement(
            &project,
            MovieTransitionPosition::After,
            Some(&last),
            None,
            MovieTransitionPlacement::InsertAfterLeft,
        )
        .unwrap();
        assert!(super::super::validate_transition_placement(
            &project,
            MovieTransitionPosition::Before,
            Some(&first),
            None,
            MovieTransitionPlacement::InsertAfterLeft,
        )
        .is_err());
    }

    #[test]
    fn placed_transition_moves_the_real_cut_to_the_nudged_endpoint_frames() {
        let mut project = test_project();
        let first = MovieFrameAnchor {
            edit_id: "edit-1".into(),
            time_seconds: 4.0,
            label: None,
        };
        let last = MovieFrameAnchor {
            edit_id: "edit-2".into(),
            time_seconds: 1.0,
            label: None,
        };
        super::super::validate_transition_placement(
            &project,
            MovieTransitionPosition::Between,
            Some(&first),
            Some(&last),
            MovieTransitionPlacement::InsertAfterLeft,
        )
        .unwrap();
        super::super::place_transition_edit(
            &mut project,
            Some(&first),
            Some(&last),
            MovieTransitionPlacement::InsertAfterLeft,
            ClipEdit {
                id: "edit-transition".into(),
                clip_id: "transition".into(),
                enabled: true,
                order: 0,
                trim_start: 0.0,
                trim_end: 0.0,
                audio_gain: 1.0,
                source_version_id: String::new(),
                speed: 1.0,
                fade_in: 0.0,
                fade_out: 0.0,
                audio_fade_in: 0.0,
                audio_fade_out: 0.0,
                label: String::new(),
                notes: String::new(),
            },
        )
        .unwrap();
        assert_eq!(project.edit.clips[0].trim_end, 1.0);
        assert_eq!(project.edit.clips[1].id, "edit-transition");
        assert_eq!(project.edit.clips[2].trim_start, 1.0);
    }

    fn transition_proposal() -> MovieGenerationProposal {
        MovieGenerationProposal {
            summary: "A controlled physical transition between the selected endpoints.".into(),
            review_summary: String::new(),
            candidate: MovieGenerationCandidate::Transition {
                motion_prompt: (0..130)
                    .map(|index| format!("motion{index}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                duration_seconds: 5.0,
                camera_motion: "The camera tracks forward with a restrained lateral pan.".into(),
                subject_motion: "The player decelerates and turns toward the landing composition."
                    .into(),
                transition_notes:
                    "Morning mist and field ambience remain continuous across the move.".into(),
                checklist: vec!["Both endpoints preserved".into()],
            },
        }
    }

    fn frame_submission(role: &str, description: &str) -> FrameObservationSubmission {
        FrameObservationSubmission {
            role: role.into(),
            description: description.into(),
            visible_action: "The subject is stationary in this single frame.".into(),
            composition: "Medium eye-level framing with the workbench centered.".into(),
            continuity_facts: vec!["The stone remains in contact with the bench.".into()],
            uncertainties: vec!["A still frame does not establish the preceding motion.".into()],
        }
    }

    fn vision_model() -> crate::model::ModelInfo {
        crate::model::ModelInfo {
            id: "vision-1".into(),
            name: "Local Vision".into(),
            path: "model.gguf".into(),
            source: "test".into(),
            bytes: 1,
            architecture: Some("qwen".into()),
            context_length: Some(32_768),
            chat_template: true,
            quantization: Some("Q4".into()),
            mmproj_path: Some("mmproj.gguf".into()),
            supports_vision: true,
            supports_audio: false,
            recommendation: String::new(),
        }
    }

    fn test_project() -> MovieProject {
        serde_json::from_value(json!({
            "schemaVersion":7,"id":uuid::Uuid::new_v4().to_string(),"title":"Test",
            "prompt":"A player crosses a field.","status":"complete","phase":"complete","detail":"Ready",
            "createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z",
            "model":"Local","renderer":"H3","settings":MovieSettings::default(),
            "modelRoles":{"director":{},"reviewer":{}},
            "plan":{"title":"Test","logline":"A player crosses.","audience":"Producers","creativeDirection":"Natural light.","continuityBible":[],"sourceCredits":[],"qualityReview":{"attempts":1,"score":100,"verdict":"ready"},"clips":[
                {"id":"clip-001","title":"Run","purpose":"Movement","durationSeconds":5.0,"prompt":"x ".repeat(130),"continuityIn":"Player enters.","continuityOut":"Player exits.","transition":"Cut","usePreviousFrame":false,"sourceRefs":[],"referenceIds":[]},
                {"id":"clip-002","title":"Wide","purpose":"Landing","durationSeconds":5.0,"prompt":"x ".repeat(130),"continuityIn":"Player arrives.","continuityOut":"Hold.","transition":"Cut","usePreviousFrame":false,"sourceRefs":[],"referenceIds":[]}
            ]},
            "clips":[
                {"id":"clip-001","index":0,"title":"Run","prompt":"x","durationSeconds":5.0,"seed":1,"status":"complete","path":"a.mp4","error":"","versions":[]},
                {"id":"clip-002","index":1,"title":"Wide","prompt":"x","durationSeconds":5.0,"seed":2,"status":"complete","path":"b.mp4","error":"","versions":[]}
            ],
            "references":[],"exports":[],"sources":[],
            "edit":{"clips":[
                {"id":"edit-1","clipId":"clip-001","enabled":true,"order":0,"trimStart":0.0,"trimEnd":0.0,"audioGain":1.0,"sourceVersionId":"","speed":1.0,"fadeIn":0.0,"fadeOut":0.0,"audioFadeIn":0.0,"audioFadeOut":0.0,"label":"","notes":""},
                {"id":"edit-2","clipId":"clip-002","enabled":true,"order":1,"trimStart":0.0,"trimEnd":0.0,"audioGain":1.0,"sourceVersionId":"","speed":1.0,"fadeIn":0.0,"fadeOut":0.0,"audioFadeIn":0.0,"audioFadeOut":0.0,"label":"","notes":""}
            ],"exportTitle":"Test","exportPreset":"publish","normalizeAudio":false,"targetLufs":-14.0,"markers":[]},
            "finalPath":"","error":"","producerReviewRequired":false,"producerApprovedAt":"","producerFeedback":[],"copilotHistory":[]
        })).unwrap()
    }
}
