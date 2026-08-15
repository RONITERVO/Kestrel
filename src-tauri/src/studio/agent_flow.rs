//! Producer-directed local-model planning orchestration.
//!
//! This module is the lifecycle owner for context sessions, producer controls, workspace tool
//! dispatch, checkpoints, and independent-review repair rounds. It deliberately does not define
//! model wire formats or workspace mutation semantics.

use super::agent_lifecycle::{AgentLifecycle, ReviewDecision, TurnDecision};
use super::agent_protocol::{self, AgentTranscript, AssistantTurn};
use super::movie_agent::{
    MovieAgentWorkspace, WorkspaceAction, WorkspaceOutcome, WorkspaceToolRequest,
    WorkspaceToolResult,
};
use super::planning::{PlanningControl, PlanningEventKind, PlanningStage};
use super::{
    check_cancel, has_meaningful_prose, prompts, IndependentReviewRequest, MoviePlan, MovieProject,
    MovieSettings, MovieStudio, ProducerFeedbackRecord, StudioError, MOVIE_AGENT_SESSION_STEPS,
};
use crate::{
    model::ModelInfo,
    models::ControlSettings,
    runtime::{ModelConnection, RuntimeManager},
};
use serde_json::{json, Value};
use std::{fs, path::Path, sync::Arc, time::Duration};
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

pub(super) enum MovieAgentOutcome {
    Submitted(MoviePlan),
    Checkpointed,
}

pub(super) struct MovieAgentRequest<'a> {
    pub prompt: &'a str,
    pub manifest: &'a str,
    pub seed: Option<&'a MoviePlan>,
    pub producer_feedback: Option<&'a str>,
    pub settings: &'a MovieSettings,
    pub runtime: &'a Arc<RuntimeManager>,
    pub models: &'a [ModelInfo],
    pub runtime_settings: &'a ControlSettings,
    pub director_model_id: &'a str,
    pub reviewer_model_id: &'a str,
    pub cancel: &'a CancellationToken,
}

pub(super) struct MovieAgentProgress<'a> {
    pub project: &'a mut MovieProject,
    pub app: Option<&'a AppHandle>,
}

enum ReviewDisposition {
    Accepted(MoviePlan),
    Repair,
}

/// Runs the durable local-model planning state machine.
///
/// The runner owns orchestration only. Workspace mutation remains typed in `movie_agent`, model
/// wire-format handling lives in `agent_protocol`, and `MovieStudio` retains project persistence
/// and producer-visible event boundaries.
pub(super) async fn run(
    studio: &MovieStudio,
    request: MovieAgentRequest<'_>,
    progress: MovieAgentProgress<'_>,
) -> Result<MovieAgentOutcome, StudioError> {
    let MovieAgentProgress { project, app } = progress;
    let workspace_root = studio.project_dir(&project.id).join("agent-workspace");
    let resuming_existing_workspace = workspace_root.join("movie.json").is_file();
    let transcript_path = workspace_root.join("agent-transcript.json");
    let mut workspace = MovieAgentWorkspace::open(
        workspace_root,
        request.prompt,
        request.manifest,
        request.settings,
        &project.references,
        request.seed,
        request.producer_feedback,
    )?;
    let tools = MovieAgentWorkspace::tools();
    let mut lifecycle = AgentLifecycle::new();

    'sessions: loop {
        check_cancel(request.cancel)?;
        lifecycle.ensure_session_budget()?;
        archive_previous_transcript(&workspace, &transcript_path, lifecycle.session())?;
        let instruction = if lifecycle.session() == 1 && !resuming_existing_workspace {
            prompts::INITIAL_INSTRUCTION
        } else {
            prompts::RESUME_INSTRUCTION
        };
        let mut transcript = AgentTranscript::begin(
            transcript_path.clone(),
            lifecycle.absolute_step(),
            prompts::MOVIE_AGENT_SYSTEM,
            instruction,
        )?;
        for _ in 0..MOVIE_AGENT_SESSION_STEPS {
            check_cancel(request.cancel)?;
            let control = apply_producer_controls(
                studio,
                project,
                &workspace,
                &mut transcript,
                &lifecycle,
                app,
            )?;
            if control.checkpoint_requested {
                return Ok(MovieAgentOutcome::Checkpointed);
            }

            let step = lifecycle.begin_step();
            announce_turn(studio, project, &lifecycle, app)?;
            let request_messages =
                transcript.request_messages(workspace.authoritative_story_memory()?);
            if request.director_model_id != request.reviewer_model_id
                && request.runtime.snapshot().await.model_id.as_deref()
                    == Some(request.reviewer_model_id)
            {
                studio.emit_planning(
                    &project.id,
                    PlanningEventKind::Activity,
                    PlanningStage::Planning,
                    "Kestrel is switching from the independent Reviewer to the Director. The local model reload can pause briefly while GPU memory changes hands.",
                    lifecycle.position(),
                    app,
                );
            }
            let lease = tokio::select! {
                result = request.runtime.lease_model(
                    request.director_model_id,
                    request.models,
                    request.runtime_settings,
                    app,
                ) => result.map_err(|error| StudioError::Planning(error.to_string()))?,
                _ = request.cancel.cancelled() => return Err(StudioError::Cancelled),
            };
            let stream_request = StreamRequest {
                connection: &lease.connection,
                messages: &request_messages,
                tools: &tools,
                settings: request.settings,
                runtime_max_output_tokens: request.runtime_settings.max_output_tokens,
                cancel: request.cancel,
                project_id: &project.id,
                position: lifecycle.position(),
                app,
            };
            let response_result = complete_agent_stream(studio, stream_request).await;
            drop(lease);
            let response = match response_result {
                Ok(response) => response,
                Err(StudioError::Cancelled) => return Err(StudioError::Cancelled),
                Err(error) => {
                    transcript.push(
                        json!({"role":"user","content":prompts::response_checkpoint(&error.to_string())}),
                        step,
                    )?;
                    project.detail = format!(
                        "The Director response failed safely at agent step {step}; Kestrel is checkpointing and resuming in a fresh context."
                    );
                    studio.persist_emit(project, app)?;
                    lifecycle.restart_session();
                    restart_delay(request.cancel).await?;
                    continue 'sessions;
                }
            };

            let turn = AssistantTurn::from_response(&response)?;
            transcript.push(turn.history_message(), step)?;
            if !turn.has_tool_calls() {
                transcript.push(
                    json!({"role":"user","content":prompts::CONTINUE_WITH_TOOLS}),
                    step,
                )?;
                if lifecycle.record_model_turn(false) == TurnDecision::RestartSession {
                    project.detail = format!(
                        "The Director stopped using its workspace for three turns at agent step {step}; Kestrel is checkpointing and resuming in a fresh context."
                    );
                    studio.persist_emit(project, app)?;
                    lifecycle.restart_session();
                    restart_delay(request.cancel).await?;
                    continue 'sessions;
                }
                continue;
            }

            lifecycle.record_model_turn(true);
            for call in turn.tool_calls() {
                check_cancel(request.cancel)?;
                let (call_id, result) =
                    execute_workspace_call(studio, &mut workspace, call, project, &lifecycle, app);
                let submitted = result.submitted;
                transcript.push(
                    json!({
                        "role":"tool",
                        "tool_call_id":call_id,
                        "content":result.message,
                    }),
                    step,
                )?;
                if let Some(plan) = submitted {
                    match review_submission(
                        studio,
                        project,
                        &mut transcript,
                        &mut lifecycle,
                        plan,
                        SubmissionReview {
                            prompt: request.prompt,
                            settings: request.settings,
                            runtime: request.runtime,
                            models: request.models,
                            runtime_settings: request.runtime_settings,
                            director_model_id: request.director_model_id,
                            reviewer_model_id: request.reviewer_model_id,
                            cancel: request.cancel,
                            app,
                        },
                    )
                    .await?
                    {
                        ReviewDisposition::Accepted(plan) => {
                            return Ok(MovieAgentOutcome::Submitted(plan));
                        }
                        ReviewDisposition::Repair => {}
                    }
                }
            }
        }
        lifecycle.restart_session();
    }
}

fn archive_previous_transcript(
    workspace: &MovieAgentWorkspace,
    transcript_path: &Path,
    session: u32,
) -> Result<(), StudioError> {
    if session > 1 && transcript_path.is_file() {
        fs::copy(
            transcript_path,
            workspace
                .root()
                .join(format!("agent-transcript-session-{:03}.json", session - 1)),
        )?;
    }
    Ok(())
}

fn apply_producer_controls(
    studio: &MovieStudio,
    project: &mut MovieProject,
    workspace: &MovieAgentWorkspace,
    transcript: &mut AgentTranscript,
    lifecycle: &AgentLifecycle,
    app: Option<&AppHandle>,
) -> Result<PlanningControl, StudioError> {
    let control = studio.consume_planning_control(&project.id, |control| {
        if control.pending_directions.is_empty() && !control.checkpoint_requested {
            return Ok(());
        }
        if !control.pending_directions.is_empty() {
            workspace.record_producer_directions(&control.pending_directions)?;
            transcript.extend(
                control.pending_directions.iter().map(|direction| {
                    json!({
                        "role":"user",
                        "content":prompts::producer_direction(&direction.text),
                    })
                }),
                lifecycle.absolute_step(),
            )?;
        } else {
            transcript.persist()?;
        }
        Ok(())
    })?;
    if !control.pending_directions.is_empty() {
        project
            .producer_feedback
            .extend(
                control
                    .pending_directions
                    .iter()
                    .map(|direction| ProducerFeedbackRecord {
                        created_at: direction.created_at.clone(),
                        scope: "live-planning".into(),
                        clip_id: String::new(),
                        feedback: direction.text.clone(),
                    }),
            );
        project.detail =
            "The Director received the producer's latest direction and is revising the durable plan."
                .into();
        studio.persist_emit(project, app)?;
    }
    Ok(control)
}

fn announce_turn(
    studio: &MovieStudio,
    project: &mut MovieProject,
    lifecycle: &AgentLifecycle,
    app: Option<&AppHandle>,
) -> Result<(), StudioError> {
    project.phase = "agent-workspace".into();
    project.detail = format!(
        "The Director is editing and checking the durable movie codebase (agent step {}, context session {}).",
        lifecycle.absolute_step(),
        lifecycle.session()
    );
    studio.persist_emit(project, app)?;
    studio.emit_planning(
        &project.id,
        PlanningEventKind::TurnStart,
        PlanningStage::Planning,
        format!(
            "The Director is planning turn {}.",
            lifecycle.absolute_step()
        ),
        lifecycle.position(),
        app,
    );
    Ok(())
}

fn execute_workspace_call(
    studio: &MovieStudio,
    workspace: &mut MovieAgentWorkspace,
    call: &Value,
    project: &MovieProject,
    lifecycle: &AgentLifecycle,
    app: Option<&AppHandle>,
) -> (String, WorkspaceToolResult) {
    let call_id = call
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("movie-tool")
        .to_string();
    let name = call
        .pointer("/function/name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if name != "movie_workspace" {
        return (
            call_id,
            WorkspaceToolResult {
                outcome: WorkspaceOutcome::Rejected,
                message: format!("ERROR: unknown tool {name}"),
                submitted: None,
            },
        );
    }
    let result = match call.pointer("/function/arguments") {
        Some(arguments) => parse_workspace_request(arguments)
            .map(|request| {
                studio.emit_planning(
                    &project.id,
                    PlanningEventKind::Activity,
                    request.action.planning_stage(),
                    producer_activity(&request),
                    lifecycle.position(),
                    app,
                );
                let result = workspace.execute(request);
                studio.emit_planning(
                    &project.id,
                    PlanningEventKind::ToolResult,
                    PlanningStage::NativeCheck,
                    producer_tool_result(&result),
                    lifecycle.position(),
                    app,
                );
                result
            })
            .unwrap_or_else(|error| WorkspaceToolResult {
                outcome: WorkspaceOutcome::Rejected,
                message: format!("ERROR: invalid movie_workspace arguments: {error}"),
                submitted: None,
            }),
        None => WorkspaceToolResult {
            outcome: WorkspaceOutcome::Rejected,
            message: "ERROR: invalid movie_workspace arguments: missing arguments".into(),
            submitted: None,
        },
    };
    (call_id, result)
}

fn parse_workspace_request(arguments: &Value) -> Result<WorkspaceToolRequest, serde_json::Error> {
    if let Some(text) = arguments.as_str() {
        serde_json::from_str(text)
    } else {
        serde_json::from_value(arguments.clone())
    }
}

struct SubmissionReview<'a> {
    prompt: &'a str,
    settings: &'a MovieSettings,
    runtime: &'a Arc<RuntimeManager>,
    models: &'a [ModelInfo],
    runtime_settings: &'a ControlSettings,
    director_model_id: &'a str,
    reviewer_model_id: &'a str,
    cancel: &'a CancellationToken,
    app: Option<&'a AppHandle>,
}

async fn review_submission(
    studio: &MovieStudio,
    project: &mut MovieProject,
    transcript: &mut AgentTranscript,
    lifecycle: &mut AgentLifecycle,
    mut plan: MoviePlan,
    request: SubmissionReview<'_>,
) -> Result<ReviewDisposition, StudioError> {
    project.phase = "agent-submitted".into();
    project.detail = format!(
        "The Director submitted a checked {}-scene plan. A fresh-context reviewer is comparing every scene with the producer brief.",
        plan.clips.len()
    );
    studio.persist_emit(project, request.app)?;
    if request.director_model_id != request.reviewer_model_id
        && request.runtime.snapshot().await.model_id.as_deref() == Some(request.director_model_id)
    {
        studio.emit_planning(
            &project.id,
            PlanningEventKind::Activity,
            PlanningStage::Planning,
            "Kestrel is switching from the Director to the independent Reviewer. The local model reload can pause briefly while GPU memory changes hands.",
            lifecycle.position(),
            request.app,
        );
    }
    let lease = tokio::select! {
        result = request.runtime.lease_model(
            request.reviewer_model_id,
            request.models,
            request.runtime_settings,
            request.app,
        ) => result.map_err(|error| StudioError::Planning(error.to_string()))?,
        _ = request.cancel.cancelled() => return Err(StudioError::Cancelled),
    };
    studio.emit_planning(
        &project.id,
        PlanningEventKind::TurnStart,
        PlanningStage::Thinking,
        "The independent Reviewer is comparing the complete plan with the producer brief.",
        lifecycle.position(),
        request.app,
    );
    let review_result = tokio::select! {
        result = studio.independently_review_movie_plan(IndependentReviewRequest {
            project_id: &project.id,
            prompt: request.prompt,
            references: &project.references,
            plan: &plan,
            connection: &lease.connection,
            settings: request.settings,
            runtime_max_output_tokens: request.runtime_settings.max_output_tokens,
            cancel: request.cancel,
            app: request.app,
            position: lifecycle.position(),
        }) => result,
        _ = request.cancel.cancelled() => Err(StudioError::Cancelled),
    };
    drop(lease);
    studio.emit_planning(
        &project.id,
        PlanningEventKind::TurnComplete,
        PlanningStage::Planning,
        "The independent Reviewer completed its model turn.",
        lifecycle.position(),
        request.app,
    );
    let review = review_result?;
    let blocking = review
        .issues
        .into_iter()
        .filter(|issue| {
            issue.clip_number as usize <= plan.clips.len()
                && has_meaningful_prose(&issue.finding, 3)
                && has_meaningful_prose(&issue.required_fix, 3)
        })
        .collect::<Vec<_>>();
    if blocking.is_empty() {
        plan.quality_review.verdict = "The Kestrel Director completed the durable workspace build, two clean native checks, a whole-codebase self-review, and a separate fresh-context fidelity review against the exact producer brief and references.".into();
        project.detail = format!(
            "The Director's {}-scene plan passed native lint, self-review, and an independent whole-film review.",
            plan.clips.len()
        );
        studio.persist_emit(project, request.app)?;
        return Ok(ReviewDisposition::Accepted(plan));
    }

    if lifecycle.record_review_rejection() == ReviewDecision::Exhausted {
        return Err(StudioError::Planning(format!(
            "the independent whole-film reviewer still found {} blocking issue(s) after three repair rounds; the durable workspace and review input are preserved",
            blocking.len()
        )));
    }
    let review_feedback = json!({
        "summary": review.summary,
        "blockingIssues": blocking,
    });
    transcript.push(
        json!({
            "role":"user",
            "content":format!(
                "Independent whole-film review rejected the submitted plan. Treat these findings as blocking, re-read the fresh authoritative story memory, patch only the affected movie/scene files, then repeat both clean checks and submit again:\n{}",
                review_feedback
            ),
        }),
        lifecycle.absolute_step(),
    )?;
    project.phase = "agent-workspace".into();
    project.detail = format!(
        "The independent reviewer found {} blocking issue(s). The Director is repairing the durable plan before H3 can render.",
        review_feedback["blockingIssues"]
            .as_array()
            .map_or(0, Vec::len)
    );
    studio.persist_emit(project, request.app)?;
    Ok(ReviewDisposition::Repair)
}

struct StreamRequest<'a> {
    connection: &'a ModelConnection,
    messages: &'a [Value],
    tools: &'a Value,
    settings: &'a MovieSettings,
    runtime_max_output_tokens: u32,
    cancel: &'a CancellationToken,
    project_id: &'a str,
    position: (u32, u32),
    app: Option<&'a AppHandle>,
}

async fn complete_agent_stream(
    studio: &MovieStudio,
    request: StreamRequest<'_>,
) -> Result<Value, StudioError> {
    let audit_path = studio
        .project_dir(request.project_id)
        .join("agent-workspace")
        .join("agent-last-request.json");
    let fallback_tool_call_prefix = format!("movie-tool-{}", request.position.1);
    let response = agent_protocol::complete_stream(
        &studio.http,
        agent_protocol::StreamCompletionRequest {
            connection: request.connection,
            messages: request.messages,
            tools: request.tools,
            settings: request.settings,
            runtime_max_output_tokens: request.runtime_max_output_tokens,
            cancel: request.cancel,
            audit_path: Some(&audit_path),
            fallback_tool_call_prefix: &fallback_tool_call_prefix,
        },
        |event| match event {
            agent_protocol::StreamEvent::Content(token) => studio.emit_planning(
                request.project_id,
                PlanningEventKind::Token,
                PlanningStage::ModelText,
                token,
                request.position,
                request.app,
            ),
            agent_protocol::StreamEvent::Reasoning(token) => studio.emit_planning(
                request.project_id,
                PlanningEventKind::Reasoning,
                PlanningStage::Thinking,
                token,
                request.position,
                request.app,
            ),
            agent_protocol::StreamEvent::ToolArgumentsStarted => studio.emit_planning(
                request.project_id,
                PlanningEventKind::Activity,
                PlanningStage::Planning,
                "The Director is streaming its next structured production action.",
                request.position,
                request.app,
            ),
            agent_protocol::StreamEvent::ToolArguments(fragment) => studio.emit_planning(
                request.project_id,
                PlanningEventKind::AdvancedToken,
                PlanningStage::ToolArguments,
                fragment,
                request.position,
                request.app,
            ),
        },
    )
    .await?;
    studio.emit_planning(
        request.project_id,
        PlanningEventKind::TurnComplete,
        PlanningStage::Planning,
        "The Director completed the model turn and is applying its production action.",
        request.position,
        request.app,
    );
    Ok(response)
}
async fn restart_delay(cancel: &CancellationToken) -> Result<(), StudioError> {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(2)) => Ok(()),
        _ = cancel.cancelled() => Err(StudioError::Cancelled),
    }
}

fn producer_activity(request: &WorkspaceToolRequest) -> String {
    match request.action {
        WorkspaceAction::List => {
            "The Director is reviewing the durable production workspace.".into()
        }
        WorkspaceAction::Read | WorkspaceAction::ReadMany => {
            if request.path.is_empty() {
                "The Director is reviewing the brief and current scene work.".into()
            } else {
                format!("The Director is reviewing {}.", request.path)
            }
        }
        WorkspaceAction::Write | WorkspaceAction::WriteBatch => {
            "The Director is updating the screenplay and scene directions.".into()
        }
        WorkspaceAction::Delete => "The Director is removing an obsolete draft scene.".into(),
        WorkspaceAction::Check => {
            "The Director is running native H3 lint and the full production review.".into()
        }
        WorkspaceAction::Submit => {
            "The Director is submitting the checked plan for producer review.".into()
        }
    }
}

fn producer_tool_result(result: &WorkspaceToolResult) -> String {
    match result.outcome {
        WorkspaceOutcome::CheckPassed => {
            "Native checks passed. The Director is completing the required whole-film review.".into()
        }
        WorkspaceOutcome::CheckFailed { issue_count } => format!(
            "Native review found {} issue{}; the Director is repairing the affected scenes.",
            issue_count.max(1),
            if issue_count == 1 { "" } else { "s" }
        ),
        WorkspaceOutcome::Submitted => "The checked plan was submitted successfully.".into(),
        WorkspaceOutcome::SubmissionBlocked | WorkspaceOutcome::Rejected => {
            "The production action was rejected safely; the Director received the exact diagnostic and will repair it.".into()
        }
        WorkspaceOutcome::Observed | WorkspaceOutcome::Mutated => {
            "The durable workspace accepted the Director's production action.".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_arguments_accept_string_and_object_encodings() {
        let string = json!(r#"{"action":"list"}"#);
        let object = json!({"action":"list"});
        assert_eq!(
            parse_workspace_request(&string).unwrap().action,
            WorkspaceAction::List
        );
        assert_eq!(
            parse_workspace_request(&object).unwrap().action,
            WorkspaceAction::List
        );
    }

    #[test]
    fn producer_status_copy_is_stable_and_nontechnical() {
        let request = parse_workspace_request(&json!({"action":"check"})).unwrap();
        assert!(producer_activity(&request).contains("native H3 lint"));
        let result = WorkspaceToolResult {
            outcome: WorkspaceOutcome::CheckFailed { issue_count: 1 },
            message: "copy may change without affecting control flow".into(),
            submitted: None,
        };
        assert!(producer_tool_result(&result).contains("1 issue"));
    }
}
