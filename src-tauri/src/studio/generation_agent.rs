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
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, path::PathBuf};
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

const GENERATION_AGENT_STEPS_PER_SESSION: u32 = 24;
const GENERATION_CONTEXT_BYTES: usize = 384 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieGenerationAgentRequest {
    pub request_id: String,
    pub project_id: String,
    pub task: MovieGenerationTask,
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MovieGenerationTask {
    ShotVersion {
        clip_id: String,
        direction: String,
    },
    Transition {
        position: MovieTransitionPosition,
        #[serde(default)]
        first_anchor: Option<MovieFrameAnchor>,
        #[serde(default)]
        last_anchor: Option<MovieFrameAnchor>,
        direction: String,
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
        clip_id: String,
        clip: PlannedClip,
        #[serde(default)]
        checklist: Vec<String>,
    },
    Transition {
        motion_prompt: String,
        duration_seconds: f32,
        camera_motion: String,
        subject_motion: String,
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
}

impl GenerationWorkspace {
    fn open(
        root: PathBuf,
        project: MovieProject,
        task: MovieGenerationTask,
    ) -> Result<Self, StudioError> {
        fs::create_dir_all(&root)?;
        let task_path = root.join("task.json");
        if task_path.is_file() {
            if fs::metadata(&task_path)?.len() > 64 * 1024 {
                return Err(StudioError::Invalid(
                    "the saved generative-edit task exceeds the 64 KiB resume limit".into(),
                ));
            }
            let bytes = fs::read(&task_path)?;
            let saved: Value = serde_json::from_slice(&bytes)?;
            let requested = serde_json::to_value(&task)?;
            if saved != requested {
                return Err(StudioError::Invalid(
                    "this checkpoint belongs to a different generative-edit task; start a new request instead"
                        .into(),
                ));
            }
        }
        let context = generation_context(&project, &task)?;
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
        json!([{"type":"function","function":{
            "name":"generation_workspace",
            "description":"Read the immutable production context, write one typed generative-edit candidate, run two clean native checks, and submit it for an independent review.",
            "parameters":{
                "type":"object","additionalProperties":false,
                "properties":{
                    "action":{"type":"string","enum":["read_context","read_candidate","write_candidate","check","submit"]},
                    "candidate":generation_candidate_schema(&self.task)
                },
                "required":["action"]
            }
        }}])
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
            GenerationAction::ReadContext => Ok(GenerationToolOutcome::Continue(format!(
                "CONTEXT: {}",
                self.authoritative_memory()?
            ))),
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
    let transcript_path = workspace_root.join("transcript.json");
    let resume_saved_transcript = transcript_path.is_file();
    let mut workspace = GenerationWorkspace::open(
        workspace_root.clone(),
        project.clone(),
        request.task.clone(),
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
            let lease = tokio::select! {
                result = model_runtime.runtime.lease_model(
                    model_runtime.director_model_id,
                    model_runtime.models,
                    model_runtime.settings,
                    app,
                ) => result.map_err(|error| StudioError::Planning(error.to_string()))?,
                _ = cancel.cancelled() => return Err(StudioError::Cancelled),
            };
            let mut settings = project.settings.clone();
            let director_settings = model_runtime
                .settings
                .for_model(model_runtime.director_model_id);
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
                let (message, submitted) = match result {
                    GenerationToolOutcome::Continue(message) => (message, None),
                    GenerationToolOutcome::Submitted(candidate) => (
                        "Candidate submitted for independent review.".into(),
                        Some(candidate),
                    ),
                };
                emit(app, request, "activity", "director", &message);
                transcript.push(
                    json!({"role":"tool","tool_call_id":call_id,"content":message}),
                    step,
                )?;
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

fn execute_call(workspace: &mut GenerationWorkspace, call: &Value) -> GenerationToolOutcome {
    if call.pointer("/function/name").and_then(Value::as_str) != Some("generation_workspace") {
        return GenerationToolOutcome::Continue("ERROR: unknown tool".into());
    }
    let Some(arguments) = call.pointer("/function/arguments") else {
        return GenerationToolOutcome::Continue("ERROR: missing tool arguments".into());
    };
    let request = if let Some(text) = arguments.as_str() {
        serde_json::from_str(text)
    } else {
        serde_json::from_value(arguments.clone())
    };
    match request {
        Ok(request) => workspace.execute(request),
        Err(error) => GenerationToolOutcome::Continue(format!(
            "ERROR: invalid generation_workspace arguments: {error}"
        )),
    }
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
    } = context;
    emit(
        app,
        request,
        "turn-start",
        "reviewer",
        "Fresh-context review started",
    );
    let lease = tokio::select! {
        result = model_runtime.runtime.lease_model(
            model_runtime.reviewer_model_id,
            model_runtime.models,
            model_runtime.settings,
            app,
        ) => result.map_err(|error| StudioError::Planning(error.to_string()))?,
        _ = cancel.cancelled() => return Err(StudioError::Cancelled),
    };
    let messages = vec![
        json!({"role":"system","content":super::prompts::generation_reviewer_system()}),
        json!({"role":"user","content":serde_json::to_string(&json!({
            "producerBrief":project.prompt,
            "task":request.task,
            "candidate":candidate,
            "plan":project.plan,
            "references":reference_manifest(&project.references),
        }))?}),
    ];
    let reviewer_settings = model_runtime
        .settings
        .for_model(model_runtime.reviewer_model_id);
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
        let mut workspace = GenerationWorkspace::open(root.path().into(), project, task).unwrap();
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
        )
        .unwrap();
        let error = match GenerationWorkspace::open(
            root.path().into(),
            project,
            MovieGenerationTask::ShotVersion {
                clip_id: "clip-002".into(),
                direction: "Change a different shot under the same checkpoint.".into(),
            },
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
