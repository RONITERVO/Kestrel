//! Producer-directed Bonsai planning orchestration.
//!
//! This module is the lifecycle owner for context sessions, producer controls, workspace tool
//! dispatch, checkpoints, and independent-review repair rounds. It deliberately does not define
//! model wire formats or workspace mutation semantics.

use super::agent_protocol::{self, AgentTranscript, AssistantTurn};
use super::movie_agent::{MovieAgentWorkspace, WorkspaceToolRequest, WorkspaceToolResult};
use super::planning::PlanningControl;
use super::{
    check_cancel, has_meaningful_prose, prompts, IndependentReviewRequest, MoviePlan, MovieProject,
    MovieSettings, MovieStudio, ProducerFeedbackRecord, StudioError, MAX_MOVIE_AGENT_SESSIONS,
    MOVIE_AGENT_SESSION_STEPS,
};
use crate::{models::ResearchSettings, runtime::ModelConnection};
use serde_json::{json, Value};
use std::{fs, path::Path, time::Duration};
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
    pub connection: &'a ModelConnection,
    pub research: &'a ResearchSettings,
    pub cancel: &'a CancellationToken,
}

pub(super) struct MovieAgentProgress<'a> {
    pub project: &'a mut MovieProject,
    pub app: Option<&'a AppHandle>,
}

#[derive(Debug, Default)]
struct AgentCursor {
    session: u32,
    absolute_step: u32,
    independent_review_round: u32,
}

impl AgentCursor {
    fn new() -> Self {
        Self {
            session: 1,
            ..Self::default()
        }
    }

    fn next_step(&mut self) -> u32 {
        self.absolute_step = self.absolute_step.saturating_add(1);
        self.absolute_step
    }

    fn next_session(&mut self) {
        self.session = self.session.saturating_add(1);
    }

    fn position(&self) -> (u32, u32) {
        (self.session, self.absolute_step)
    }
}

enum ReviewDisposition {
    Accepted(MoviePlan),
    Repair,
}

/// Runs the durable Bonsai planning state machine.
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
    let mut cursor = AgentCursor::new();

    'sessions: loop {
        check_cancel(request.cancel)?;
        ensure_session_budget(&cursor)?;
        archive_previous_transcript(&workspace, &transcript_path, cursor.session)?;
        let instruction = if cursor.session == 1 && !resuming_existing_workspace {
            prompts::INITIAL_INSTRUCTION
        } else {
            prompts::RESUME_INSTRUCTION
        };
        let mut transcript = AgentTranscript::begin(
            transcript_path.clone(),
            cursor.absolute_step,
            prompts::MOVIE_AGENT_SYSTEM,
            instruction,
        )?;
        let mut no_tool_streak = 0_u32;

        for _ in 0..MOVIE_AGENT_SESSION_STEPS {
            check_cancel(request.cancel)?;
            let control = apply_producer_controls(
                studio,
                project,
                &workspace,
                &mut transcript,
                &cursor,
                app,
            )?;
            if control.checkpoint_requested {
                return Ok(MovieAgentOutcome::Checkpointed);
            }

            let step = cursor.next_step();
            announce_turn(studio, project, &cursor, app)?;
            let request_messages =
                transcript.request_messages(workspace.authoritative_story_memory()?);
            let stream_request = StreamRequest {
                connection: request.connection,
                messages: &request_messages,
                tools: &tools,
                settings: request.settings,
                research: request.research,
                cancel: request.cancel,
                project_id: &project.id,
                position: cursor.position(),
                app,
            };
            let response = match complete_agent_stream(studio, stream_request).await {
                Ok(response) => response,
                Err(StudioError::Cancelled) => return Err(StudioError::Cancelled),
                Err(error) => {
                    transcript.push(
                        json!({"role":"user","content":prompts::response_checkpoint(&error.to_string())}),
                        step,
                    )?;
                    project.detail = format!(
                        "Bonsai response failed safely at agent step {step}; Kestrel is checkpointing and resuming in a fresh context."
                    );
                    studio.persist_emit(project, app)?;
                    cursor.next_session();
                    restart_delay(request.cancel).await?;
                    continue 'sessions;
                }
            };

            let turn = AssistantTurn::from_response(&response)?;
            transcript.push(turn.history_message(), step)?;
            if !turn.has_tool_calls() {
                no_tool_streak = no_tool_streak.saturating_add(1);
                transcript.push(
                    json!({"role":"user","content":prompts::CONTINUE_WITH_TOOLS}),
                    step,
                )?;
                if no_tool_streak >= 3 {
                    project.detail = format!(
                        "Bonsai stopped using its workspace for three turns at agent step {step}; Kestrel is checkpointing and resuming in a fresh context."
                    );
                    studio.persist_emit(project, app)?;
                    cursor.next_session();
                    restart_delay(request.cancel).await?;
                    continue 'sessions;
                }
                continue;
            }

            no_tool_streak = 0;
            for call in turn.tool_calls() {
                check_cancel(request.cancel)?;
                let (call_id, result) =
                    execute_workspace_call(studio, &mut workspace, call, project, &cursor, app);
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
                        &mut cursor,
                        plan,
                        SubmissionReview {
                            prompt: request.prompt,
                            connection: request.connection,
                            settings: request.settings,
                            research: request.research,
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
        cursor.next_session();
    }
}

fn ensure_session_budget(cursor: &AgentCursor) -> Result<(), StudioError> {
    if cursor.session > MAX_MOVIE_AGENT_SESSIONS {
        return Err(StudioError::Planning(format!(
            "Bonsai did not submit a valid movie after {MAX_MOVIE_AGENT_SESSIONS} context sessions; the durable workspace is intact for a later retry"
        )));
    }
    Ok(())
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
    cursor: &AgentCursor,
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
                cursor.absolute_step,
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
            "Bonsai received the producer's latest direction and is revising the durable plan."
                .into();
        studio.persist_emit(project, app)?;
    }
    Ok(control)
}

fn announce_turn(
    studio: &MovieStudio,
    project: &mut MovieProject,
    cursor: &AgentCursor,
    app: Option<&AppHandle>,
) -> Result<(), StudioError> {
    project.phase = "agent-workspace".into();
    project.detail = format!(
        "Bonsai is editing and checking the durable movie codebase (agent step {}, context session {}).",
        cursor.absolute_step, cursor.session
    );
    studio.persist_emit(project, app)?;
    studio.emit_planning(
        &project.id,
        "turn-start",
        "planning",
        format!("Bonsai is planning turn {}.", cursor.absolute_step),
        cursor.position(),
        app,
    );
    Ok(())
}

fn execute_workspace_call(
    studio: &MovieStudio,
    workspace: &mut MovieAgentWorkspace,
    call: &Value,
    project: &MovieProject,
    cursor: &AgentCursor,
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
                    "activity",
                    &request.action,
                    producer_activity(&request),
                    cursor.position(),
                    app,
                );
                let result = workspace.execute(request);
                studio.emit_planning(
                    &project.id,
                    "tool-result",
                    "native-check",
                    producer_tool_result(&result.message),
                    cursor.position(),
                    app,
                );
                result
            })
            .unwrap_or_else(|error| WorkspaceToolResult {
                message: format!("ERROR: invalid movie_workspace arguments: {error}"),
                submitted: None,
            }),
        None => WorkspaceToolResult {
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
    connection: &'a ModelConnection,
    settings: &'a MovieSettings,
    research: &'a ResearchSettings,
    cancel: &'a CancellationToken,
    app: Option<&'a AppHandle>,
}

async fn review_submission(
    studio: &MovieStudio,
    project: &mut MovieProject,
    transcript: &mut AgentTranscript,
    cursor: &mut AgentCursor,
    mut plan: MoviePlan,
    request: SubmissionReview<'_>,
) -> Result<ReviewDisposition, StudioError> {
    project.phase = "agent-submitted".into();
    project.detail = format!(
        "Bonsai submitted a checked {}-scene plan. A fresh-context reviewer is comparing every scene with the producer brief.",
        plan.clips.len()
    );
    studio.persist_emit(project, request.app)?;
    let review = tokio::select! {
        result = studio.independently_review_movie_plan(IndependentReviewRequest {
            project_id: &project.id,
            prompt: request.prompt,
            references: &project.references,
            plan: &plan,
            connection: request.connection,
            settings: request.settings,
            research: request.research,
        }) => result?,
        _ = request.cancel.cancelled() => return Err(StudioError::Cancelled),
    };
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
        plan.quality_review.verdict = "Bonsai completed the durable workspace build, two clean native checks, a whole-codebase self-review, and a separate fresh-context fidelity review against the exact producer brief and references.".into();
        project.detail = format!(
            "Bonsai's {}-scene plan passed native lint, self-review, and an independent whole-film review.",
            plan.clips.len()
        );
        studio.persist_emit(project, request.app)?;
        return Ok(ReviewDisposition::Accepted(plan));
    }

    cursor.independent_review_round = cursor.independent_review_round.saturating_add(1);
    if cursor.independent_review_round >= 3 {
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
        cursor.absolute_step,
    )?;
    project.phase = "agent-workspace".into();
    project.detail = format!(
        "The independent reviewer found {} blocking issue(s). Bonsai is repairing the durable plan before H3 can render.",
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
    research: &'a ResearchSettings,
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
            runtime_max_output_tokens: request.research.max_output_tokens,
            cancel: request.cancel,
            audit_path: &audit_path,
            fallback_tool_call_prefix: &fallback_tool_call_prefix,
        },
        |event| match event {
            agent_protocol::StreamEvent::Content(token) => studio.emit_planning(
                request.project_id,
                "token",
                "model-text",
                token,
                request.position,
                request.app,
            ),
            agent_protocol::StreamEvent::ReasoningStarted => studio.emit_planning(
                request.project_id,
                "reasoning",
                "thinking",
                "Bonsai is reasoning locally before its next production action.",
                request.position,
                request.app,
            ),
            agent_protocol::StreamEvent::ToolArgumentsStarted => studio.emit_planning(
                request.project_id,
                "activity",
                "planning",
                "Bonsai is streaming its next structured production action.",
                request.position,
                request.app,
            ),
            agent_protocol::StreamEvent::ToolArguments(fragment) => studio.emit_planning(
                request.project_id,
                "advanced-token",
                "tool-arguments",
                fragment,
                request.position,
                request.app,
            ),
        },
    )
    .await?;
    studio.emit_planning(
        request.project_id,
        "turn-complete",
        "planning",
        "Bonsai completed the model turn and is applying its production action.",
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
    match request.action.as_str() {
        "list" => "Bonsai is reviewing the durable production workspace.".into(),
        "read" | "read_many" => {
            if request.path.is_empty() {
                "Bonsai is reviewing the brief and current scene work.".into()
            } else {
                format!("Bonsai is reviewing {}.", request.path)
            }
        }
        "write" | "write_batch" => "Bonsai is updating the screenplay and scene directions.".into(),
        "delete" => "Bonsai is removing an obsolete draft scene.".into(),
        "check" => "Bonsai is running native H3 lint and the full production review.".into(),
        "submit" => "Bonsai is submitting the checked plan for producer review.".into(),
        _ => "Bonsai is applying a production action.".into(),
    }
}

fn producer_tool_result(message: &str) -> String {
    if message.starts_with("CHECK PASS") {
        "Native checks passed. Bonsai is completing the required whole-film review.".into()
    } else if message.starts_with("CHECK FAIL") {
        let issues = message.lines().filter(|line| line.starts_with('-')).count();
        format!(
            "Native review found {} issue{}; Bonsai is repairing the affected scenes.",
            issues.max(1),
            if issues == 1 { "" } else { "s" }
        )
    } else if message.starts_with("SUBMITTED") {
        "The checked plan was submitted successfully.".into()
    } else if message.starts_with("ERROR") || message.starts_with("SUBMIT BLOCKED") {
        "The production action was rejected safely; Bonsai received the exact diagnostic and will repair it.".into()
    } else {
        "The durable workspace accepted Bonsai's production action.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_arguments_accept_string_and_object_encodings() {
        let string = json!(r#"{"action":"list"}"#);
        let object = json!({"action":"list"});
        assert_eq!(parse_workspace_request(&string).unwrap().action, "list");
        assert_eq!(parse_workspace_request(&object).unwrap().action, "list");
    }

    #[test]
    fn cursor_keeps_session_and_absolute_step_separate() {
        let mut cursor = AgentCursor::new();
        assert_eq!(cursor.next_step(), 1);
        cursor.next_session();
        assert_eq!(cursor.position(), (2, 1));
        assert_eq!(cursor.next_step(), 2);
    }

    #[test]
    fn producer_status_copy_is_stable_and_nontechnical() {
        let request = parse_workspace_request(&json!({"action":"check"})).unwrap();
        assert!(producer_activity(&request).contains("native H3 lint"));
        assert!(producer_tool_result("CHECK FAIL\n- issue").contains("1 issue"));
    }
}
