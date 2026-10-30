//! Bounded, inspectable computer tasks for the local model.
//!
//! Chat never calls this module. A task receives one explicit access mode, a fixed step budget,
//! native tool schemas, native path validation, recoverable overwrites, cancellation between every
//! action, and a durable event transcript.

use crate::{
    attachments::AttachmentStore,
    model::ModelInfo,
    models::{ComputerTaskAccess, ComputerTaskEvent, ComputerTaskRequest, ControlSettings},
    prompt_catalog::{self, PromptId},
    runtime::{authorized, RuntimeManager},
    workspace::WorkspaceStore,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tauri::{AppHandle, Manager};
use tokio::{process::Command, time::timeout};
use tokio_util::sync::CancellationToken;

type Access = ComputerTaskAccess;
mod context;
mod memory;
pub(crate) mod progress;
mod recovery;

struct ToolOutput {
    text: String,
    artifact: Option<PathBuf>,
    data: Option<Value>,
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    app: Option<AppHandle>,
    runtime: Arc<RuntimeManager>,
    store: WorkspaceStore,
    attachment_store: AttachmentStore,
    run_id: String,
    request: ComputerTaskRequest,
    models: Vec<ModelInfo>,
    settings: ControlSettings,
    cancel: CancellationToken,
    continuation: Option<String>,
) -> Result<(), String> {
    let mut settings = settings.for_model(&request.model_id);
    if let Some(model) = models.iter().find(|model| model.id == request.model_id) {
        settings.context_window = model.serving_context(settings.context_window);
    }
    let access = request.access;
    if access == Access::Full && !settings.allow_full_access_agent {
        return Err("Full computer access is locked in the runtime profile.".into());
    }
    if access == Access::Workspace && settings.agent_workspace_roots.is_empty() && app.is_none() {
        return Err(
            "Add at least one Computer Tasks workspace folder in the runtime profile.".into(),
        );
    }
    event(
        &app,
        &store,
        &run_id,
        0,
        "queued",
        "Waiting for the local model",
        "Research, chat, and computer tasks share one inference slot.",
        None,
    )?;
    let access_label = if access == Access::Workspace {
        "workspace-restricted file access"
    } else {
        "explicit full computer access"
    };
    let selected_model = models
        .iter()
        .find(|model| model.id == request.model_id)
        .cloned()
        .ok_or_else(|| "The selected model is no longer in the local catalog.".to_string())?;
    let objective = match continuation {
        Some(continuation) => prompt_catalog::render(
            PromptId::ComputerObjectiveContinuation,
            &[
                ("objective", &request.objective),
                ("continuation", &continuation),
            ],
        ),
        None => request.objective.clone(),
    };
    let preparation_store = attachment_store.clone();
    let preparation_ids = request.attachment_ids.clone();
    let preparation_objective = objective.clone();
    let preparation = tokio::task::spawn_blocking(move || {
        let attachments = preparation_store.resolve(&preparation_ids)?;
        let prepared = preparation_store.prepare_message(
            &preparation_objective,
            &attachments,
            &selected_model,
            120_000,
        )?;
        Ok::<_, String>((attachments, prepared))
    });
    let (context_attachments, prepared) = tokio::select! {
        result = preparation => result
            .map_err(|error| format!("Attachment preparation stopped unexpectedly: {error}"))??,
        _ = cancel.cancelled() => {
            event(&app, &store, &run_id, 0, "cancelled", "Stopped by you", "The task stopped while preparing local attachment context. No tool was started.", None)?;
            return Ok(());
        }
    };
    if cancel.is_cancelled() {
        event(
            &app,
            &store,
            &run_id,
            0,
            "cancelled",
            "Stopped by you",
            "The task stopped after preparing local attachment context. No tool was started.",
            None,
        )?;
        return Ok(());
    }
    let attachment_instruction = if context_attachments.is_empty() {
        String::new()
    } else {
        prompt_catalog::text(PromptId::ComputerAttachmentNotice)
    };
    let roots = if settings.agent_workspace_roots.is_empty() {
        if access == ComputerTaskAccess::Full {
            "full access explicitly enabled".into()
        } else {
            "No filesystem folders approved. Use native app-owned projects; ask the user to choose a workspace before importing or exporting external files.".into()
        }
    } else {
        settings.agent_workspace_roots.join("; ")
    };
    let mut system = prompt_catalog::render(
        PromptId::ComputerSystem,
        &[
            ("access_label", access_label),
            ("workspace_roots", &roots),
            ("attachment_instruction", &attachment_instruction),
        ],
    );
    if app.is_some() {
        system.push_str(include_str!("agent/producer.md"));
    }
    let mut messages = vec![
        json!({"role":"system","content":system}),
        json!({"role":"user","content":prepared.content}),
    ];
    if let Some(notice) = prepared.notice {
        event(
            &app,
            &store,
            &run_id,
            0,
            "context",
            "Attachment context prepared",
            &notice,
            None,
        )?;
    }
    let mut tools = tool_schemas(access, !context_attachments.is_empty());
    tools.extend(memory::schemas());
    if app.is_some() {
        tools.extend(crate::agent_app::schemas());
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3_600))
        .build()
        .map_err(|error| error.to_string())?;
    let max_steps = if settings.advanced_mode {
        request.max_steps.max(1)
    } else {
        request
            .max_steps
            .max(1)
            .min(settings.agent_max_steps.max(1))
    };
    let max_output_tokens = if settings.advanced_mode {
        request.max_output_tokens.max(1)
    } else {
        request
            .max_output_tokens
            .max(1)
            .min(settings.agent_max_output_tokens.max(1))
    };
    event(
        &app,
        &store,
        &run_id,
        0,
        "start",
        "Computer task started",
        &format!("{} · {}", request.model_id, access_label),
        None,
    )?;
    let thinking_level = request.thinking_level.unwrap_or(settings.thinking_level);
    let mut truncated_retries = 0_u8;
    let mut unavailable_retries = 0_u8;
    let base_objective = messages[1]["content"].clone();
    'steps: for step in 1..=max_steps {
        if cancel.is_cancelled() {
            event(
                &app,
                &store,
                &run_id,
                step,
                "cancelled",
                "Stopped by you",
                "No further tools will run.",
                None,
            )?;
            return Ok(());
        }
        let thinking_detail = if thinking_level.is_off() {
            "Planning the next visible action (Thinking off).".to_string()
        } else {
            format!(
                "Planning the next visible action ({} thinking).",
                thinking_level.as_str()
            )
        };
        let prompt_chars = crate::models::prompt_char_budget(
            settings.context_window,
            // A preliminary byte bound; the local tokenizer below admits the real combined
            // input/output request and fits its output allowance to the space actually left.
            2_048,
        )
        .unwrap_or(16_384);
        let direction_revision = store.task_direction_revision(&run_id)?;
        // Keep the prompt prefix stable between normal turns. Rewriting the first user message
        // after every event invalidates the local engine's KV cache and repeatedly prefills the
        // entire history. Fresh native recovery is needed on entry/direction/compaction, while
        // ordinary turns already contain their actual tool outcomes.
        if messages.len() == 2 {
            messages[1]["content"] =
                memory::with_context(&base_objective, &store.task_recovery_context(&run_id)?);
        }
        let tool_chars = tools
            .iter()
            .map(|value| value.to_string().len())
            .sum::<usize>();
        let previous_count = messages.len();
        messages = compact_messages(
            messages,
            prompt_chars.max(16_384).saturating_sub(tool_chars),
        )?;
        if messages.len() != previous_count {
            messages[1]["content"] =
                memory::with_context(&base_objective, &store.task_recovery_context(&run_id)?);
            messages = compact_messages(
                messages,
                prompt_chars.max(16_384).saturating_sub(tool_chars),
            )?;
        }
        // Media actions stop the language runtime while owning the workspace. Release their
        // caches before reloading it; otherwise Whisper and the LLM compete for host RAM/VRAM.
        // Do not poll or unload anything on ordinary planning turns with a ready runtime.
        if runtime.snapshot().await.phase != "ready" {
            if let Some(app) = &app {
                crate::release_all_comfy_memory(&app.state::<crate::AppState>()).await;
            }
        }
        let lease = tokio::select! {
            result = runtime.lease_model(&request.model_id, &models, &settings, app.as_ref()) => {
                result.map_err(|error| error.to_string())?
            }
            _ = cancel.cancelled() => {
                event(&app, &store, &run_id, 0, "cancelled", "Stopped by you", "No computer action was started.", None)?;
                return Ok(());
            }
        };
        if store.task_direction_revision(&run_id)? != direction_revision {
            messages.truncate(2);
            continue 'steps;
        }
        event(
            &app,
            &store,
            &run_id,
            step,
            "thinking",
            "Model is deciding",
            &thinking_detail,
            Some(json!({"thinkingLevel": thinking_level.as_str()})),
        )?;
        let step_output_tokens = loop {
            let previous_count = messages.len();
            let output = tokio::select! {
            result = context::fit(&client, &lease.connection, &mut messages, &tools, settings.context_window, max_output_tokens, thinking_level) => result?,
            _ = cancel.cancelled() => {
                event(&app,&store,&run_id,step,"cancelled","Stopped by you","No action started while checking context space.",None)?;
                return Ok(());
            }
            direction = store.wait_for_task_direction(&run_id,direction_revision) => {
                direction?;
                messages.truncate(2);
                continue 'steps;
            }
            };
            if messages.len() == previous_count {
                break output;
            }
            messages[1]["content"] =
                memory::with_context(&base_objective, &store.task_recovery_context(&run_id)?);
            // Measure the fresh recovery packet too; it may change the admission count.
        };
        let mut request_body = json!({
            "model": lease.connection.model_id,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto",
            "stream": false,
            "temperature": 0.2,
            "max_tokens": step_output_tokens
        });
        apply_thinking_budget(
            &mut request_body,
            lease.connection.engine,
            thinking_level,
            step_output_tokens,
        );
        if thinking_level.is_off() {
            request_body["thinking_budget_tokens"] = json!(0);
            request_body["chat_template_kwargs"] =
                json!({"enable_thinking": false, "reasoning": false});
            request_body["reasoning_effort"] = json!("off");
        } else {
            request_body["reasoning_effort"] = json!(thinking_level.as_str());
            request_body["chat_template_kwargs"] = json!({
                "reasoning_effort": thinking_level.as_template_effort(),
                "enable_thinking": true
            });
        }
        let response_request = async {
            let response = authorized(
                client.post(format!("{}/chat/completions", lease.connection.endpoint)),
                &lease.connection,
            )
            .json(&request_body)
            .send()
            .await?;
            let status = response.status();
            // Some servers send headers before finishing the JSON body. Human direction and
            // cancellation must also interrupt that body wait, not just connection setup.
            let body = response.json::<Value>().await?;
            Ok::<_, reqwest::Error>((status, body))
        };
        let (status, body) = tokio::select! {
            response = response_request => response.map_err(|error| format!("computer task model request failed: {error}"))?,
            direction = store.wait_for_task_direction(&run_id, direction_revision) => {
                direction?;
                messages.truncate(2);
                continue 'steps;
            }
            _ = cancel.cancelled() => {
                event(&app, &store, &run_id, step, "cancelled", "Stopped by you", "The pending model request was cancelled. No tool was started.", None)?;
                return Ok(());
            }
        };
        drop(lease); // App actions may acquire this same inference gate.
        if store.task_direction_revision(&run_id)? != direction_revision {
            messages.truncate(2);
            continue 'steps;
        }
        if !status.is_success() {
            if status == reqwest::StatusCode::SERVICE_UNAVAILABLE && unavailable_retries == 0 {
                unavailable_retries += 1;
                // This request only generated a proposal. No returned tool was dispatched, so
                // retrying inference cannot replay a mutation. Keep the normal step budget,
                // direction revision and cancellation checks; never loop on a broken engine.
                event(&app, &store, &run_id, step, "progress", "Recovering the local assistant",
                    "The local model could not finish its reply. Your work is saved. Retrying its next decision once; completed actions will not be repeated.",
                    Some(json!({"httpStatus":status.as_u16(),"retry":unavailable_retries,"error":truncate(&body.to_string(),800)})))?;
                messages.truncate(2);
                continue 'steps;
            }
            return Err(format!(
                "computer task model returned {status}: {}",
                truncate(&body.to_string(), 800)
            ));
        }
        let message = body
            .pointer("/choices/0/message")
            .cloned()
            .ok_or_else(|| "computer task model returned no message".to_string())?;
        if completion_hit_limit(&body) {
            truncated_retries += 1;
            // A truncated tool batch may contain incomplete arguments. Execute none of it,
            // and never turn a token-limit response into a completed task.
            if truncated_retries > 2 {
                return Err("The local model reached its response limit three times without completing an action. Completed work is preserved; review the transcript and resume with a smaller step or a larger output budget.".into());
            }
            event(&app, &store, &run_id, step, "progress", "Continuing after the model response limit", "The model ran out of response space. Its incomplete tool batch was not executed. Retrying with a smaller next step; this is not task completion.", Some(json!({"finishReason":"length","retry":truncated_retries})))?;
            if let Some(content) = message
                .get("content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                messages.push(json!({"role":"assistant","content":content}));
            }
            messages.push(json!({"role":"user","content":"Native runner feedback: your last response reached the token limit; none of its proposed tools were executed and the task is still unfinished. Continue the existing authorized task with ONE small concrete action. Do not repeat the plan or broad discovery. Reuse known artifact IDs. For large authored files, write a minimal complete version first or split the work into bounded actions. Finish only after checking the actual requested result; if blocked, use ask_user."}));
            continue;
        }
        truncated_retries = 0;
        if matches!(
            body.pointer("/choices/0/finish_reason")
                .and_then(Value::as_str),
            Some("content_filter" | "error")
        ) {
            return Err("The local model stopped without a successful completion. No action from that response was executed; inspect the model's response before resuming.".into());
        }
        if let Some(reasoning) = message
            .get("reasoning_content")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                message
                    .get("reasoning")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
            })
        {
            event(
                &app,
                &store,
                &run_id,
                step,
                "reasoning",
                "Reasoning",
                reasoning,
                None,
            )?;
        }
        let tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        messages.push(memory::conversation_reply(&message));
        if tool_calls.is_empty() {
            let answer = message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("The task ended without a summary.");
            let unsettled = store.task_memory(&run_id)?.unsettled_actions;
            let question = !unsettled.is_empty() || answer.trim_end().ends_with('?');
            let answer = if unsettled.is_empty() {
                answer.to_string()
            } else {
                format!("{answer}\n\nAn earlier action still has an uncertain outcome. Review it in Working plan and memory before more changes; this work is not marked complete.")
            };
            let response = ComputerTaskEvent {
                run_id: run_id.clone(),
                step,
                kind: if question { "question" } else { "done" }.into(),
                title: if question {
                    "Needs your input"
                } else {
                    "Completed"
                }
                .into(),
                detail: answer.clone(),
                data: if question {
                    Some(json!({"question":answer,"options":[]}))
                } else {
                    None
                },
                at: chrono::Utc::now().to_rfc3339(),
            };
            if !publish_if_current(&app, &store, response, direction_revision)? {
                messages.truncate(2);
                continue 'steps;
            }
            return Ok(());
        }
        if let Some(content) = message
            .get("content")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            event(
                &app, &store, &run_id, step, "message", "Kestrel", content, None,
            )?;
        }
        for call in tool_calls {
            if store.task_direction_revision(&run_id)? != direction_revision {
                messages.truncate(2);
                continue 'steps;
            }
            if cancel.is_cancelled() {
                event(
                    &app,
                    &store,
                    &run_id,
                    step,
                    "cancelled",
                    "Stopped by you",
                    "No further tools will run.",
                    None,
                )?;
                return Ok(());
            }
            let call_id = call.get("id").and_then(Value::as_str).unwrap_or("tool");
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let argument_text = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let arguments: Value = match serde_json::from_str(argument_text) {
                Ok(arguments) => arguments,
                Err(error) => {
                    let detail = format!("ERROR: invalid arguments for {name}: {error}");
                    event(
                        &app,
                        &store,
                        &run_id,
                        step,
                        "tool_error",
                        &format!("{name} finished"),
                        &detail,
                        Some(json!({"unstarted":true})),
                    )?;
                    messages.push(json!({"role":"tool","tool_call_id":call_id,"content":detail}));
                    continue;
                }
            };
            if !arguments.is_object() {
                let detail = "ERROR: tool arguments must be a JSON object; no action was started.";
                event(
                    &app,
                    &store,
                    &run_id,
                    step,
                    "tool_error",
                    "Invalid tool arguments",
                    detail,
                    Some(json!({"unstarted":true})),
                )?;
                messages.push(json!({"role":"tool","tool_call_id":call_id,"content":detail}));
                continue;
            }
            if name == "ask_user" {
                match parse_question(&arguments) {
                    Ok(question) => {
                        let response = ComputerTaskEvent {
                            run_id: run_id.clone(),
                            step,
                            kind: "question".into(),
                            title: "Needs your input".into(),
                            detail: question.question.clone(),
                            data: Some(
                                json!({"question":question.question,"options":question.options,"recommendedIndex":question.recommended_index}),
                            ),
                            at: chrono::Utc::now().to_rfc3339(),
                        };
                        if !publish_if_current(&app, &store, response, direction_revision)? {
                            messages.truncate(2);
                            continue 'steps;
                        }
                        return Ok(());
                    }
                    Err(error) => {
                        let detail = format!("ERROR: invalid clarification request: {error}");
                        event(
                            &app,
                            &store,
                            &run_id,
                            step,
                            "tool_error",
                            "ask_user finished",
                            &detail,
                            Some(json!({"unstarted":true})),
                        )?;
                        messages
                            .push(json!({"role":"tool","tool_call_id":call_id,"content":detail}));
                        continue;
                    }
                }
            }
            let mutation = recovery::is_mutation(name, &arguments);
            if mutation {
                if let Err(error) = recovery::require_reconciled(&store, &run_id) {
                    let detail = format!("ERROR: {error}");
                    event(
                        &app,
                        &store,
                        &run_id,
                        step,
                        "tool_error",
                        "Action needs review",
                        &detail,
                        Some(json!({"unstarted":true})),
                    )?;
                    messages.push(json!({"role":"tool","tool_call_id":call_id,"content":detail}));
                    continue;
                }
            }
            let action_id = uuid::Uuid::new_v4().to_string();
            let mut started = arguments.clone();
            if let Some(object) = started.as_object_mut() {
                object.insert("actionId".into(), json!(action_id));
                object.insert("tool".into(), json!(name));
                object.insert("mutation".into(), json!(mutation));
                object.remove("sourceSha256");
                if let Some(hash) = recovery::source_identity(
                    name,
                    &arguments,
                    access,
                    &settings.agent_workspace_roots,
                ) {
                    object.insert("sourceSha256".into(), json!(hash));
                }
            }
            let starting = ComputerTaskEvent {
                run_id: run_id.clone(),
                step,
                kind: "tool_start".into(),
                title: format!("Using {name}"),
                detail: argument_text.into(),
                data: Some(started),
                at: chrono::Utc::now().to_rfc3339(),
            };
            if store
                .add_task_event_if_current(starting.clone(), direction_revision)?
                .is_none()
            {
                messages.truncate(2);
                continue 'steps;
            }
            if let Some(app) = &app {
                let _ = crate::ipc_events::emit::<kestrel_app_core::events::ComputerTask>(
                    app, &starting,
                );
            }
            let result = if name == "task_reconcile" {
                recovery::reconcile(
                    app.as_ref(),
                    &store,
                    &run_id,
                    &arguments,
                    access,
                    &settings.agent_workspace_roots,
                )
            } else if name.starts_with("task_") {
                memory::execute(&store, &run_id, name, &arguments)
            } else if name.starts_with("app_") {
                match app.as_ref() {
                    Some(handle) => crate::agent_app::execute(
                        handle, &run_id, name, &arguments, &settings, access, &cancel,
                    )
                    .await
                    .map(|value| ToolOutput {
                        text: value.to_string(),
                        artifact: crate::agent_app::artifact(&arguments, &value),
                        data: Some(value),
                    }),
                    None => Err("App actions require the running Kestrel desktop.".into()),
                }
            } else {
                tokio::select! {
                    result = execute_tool(
                        name,
                        &arguments,
                        access,
                        &settings.agent_workspace_roots,
                        &attachment_store,
                        &request.attachment_ids,
                    ) => result,
                    _ = cancel.cancelled() => {
                        event(&app, &store, &run_id, step, "cancelled", "Stopped by you", "The active operation was cancelled and no further tools will run.", None)?;
                        return Ok(());
                    }
                }
            };
            let (output, failed) = match result {
                Ok(output) => (output, false),
                Err(error) => (
                    ToolOutput {
                        text: format!("ERROR: {error}"),
                        artifact: None,
                        data: None,
                    },
                    true,
                ),
            };
            let result_sequence = event(
                &app,
                &store,
                &run_id,
                step,
                if failed { "tool_error" } else { "tool_result" },
                &format!("{name} finished"),
                &output.text,
                Some(json!({"actionId":action_id,"result":output.data})),
            )?;
            if let Some(path) = output.artifact {
                event(
                    &app,
                    &store,
                    &run_id,
                    step,
                    "artifact",
                    "Artifact ready",
                    &path.to_string_lossy(),
                    Some(json!({"path": path.to_string_lossy()})),
                )?;
            }
            messages.push(json!({"role":"tool","tool_call_id":call_id,"content":memory::preview(&output.text, result_sequence)}));
        }
    }
    event(
        &app,
        &store,
        &run_id,
        max_steps,
        "limit",
        "Progress saved — ready to continue",
        "I reached this run's work limit and paused before another action. Send a message to continue or change direction; the saved work will stay in this conversation.",
        None,
    )?;
    Ok(())
}

pub fn emit_error(app: Option<&AppHandle>, store: &WorkspaceStore, run_id: &str, detail: String) {
    let owned = app.cloned();
    if let Err(error) = event(
        &owned,
        store,
        run_id,
        0,
        "error",
        "Computer task stopped",
        &detail,
        None,
    ) {
        eprintln!("Task {run_id} stopped; its final error could not be saved: {error}");
        if let Some(app) = app {
            let value = ComputerTaskEvent {
                run_id: run_id.into(), step: 0, kind: "error".into(),
                title: "Stopped because task history could not be saved".into(),
                detail: format!("{detail}\nStorage error: {error}. Check free space and permissions before continuing. This final notice is not saved."),
                data: None, at: chrono::Utc::now().to_rfc3339(),
            };
            let _ = crate::ipc_events::emit::<kestrel_app_core::events::ComputerTask>(app, &value);
        }
    }
}

fn publish_if_current(
    app: &Option<AppHandle>,
    store: &WorkspaceStore,
    value: ComputerTaskEvent,
    direction_revision: u32,
) -> Result<bool, String> {
    if store
        .add_task_event_if_current(value.clone(), direction_revision)?
        .is_none()
    {
        return Ok(false);
    }
    if let Some(app) = app {
        let _ = crate::ipc_events::emit::<kestrel_app_core::events::ComputerTask>(app, &value);
    }
    Ok(true)
}

fn completion_hit_limit(body: &Value) -> bool {
    body.pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        == Some("length")
}

fn apply_thinking_budget(
    body: &mut Value,
    engine: crate::models::ModelEngine,
    level: crate::models::ThinkingLevel,
    max_output: u32,
) {
    // Strata calls this reasoning_budget_tokens; thinking_budget_tokens is ignored there.
    // Keep room for a real answer/tool call even at high effort.
    let budget = level.budget_tokens(max_output.saturating_sub(512).max(1));
    let key = if engine == crate::models::ModelEngine::Strata {
        "reasoning_budget_tokens"
    } else {
        "thinking_budget_tokens"
    };
    body[key] = json!(budget);
}

#[allow(clippy::too_many_arguments)]
fn event(
    app: &Option<AppHandle>,
    store: &WorkspaceStore,
    run_id: &str,
    step: u32,
    kind: &str,
    title: &str,
    detail: &str,
    data: Option<Value>,
) -> Result<usize, String> {
    let value = ComputerTaskEvent {
        run_id: run_id.to_string(),
        step,
        kind: kind.to_string(),
        title: title.to_string(),
        detail: detail.to_string(),
        data,
        at: chrono::Utc::now().to_rfc3339(),
    };
    let sequence = store.add_task_event(value.clone())?.events.len();
    if let Some(app) = app {
        let _ = crate::ipc_events::emit::<kestrel_app_core::events::ComputerTask>(app, &value);
    }
    Ok(sequence)
}

fn tool_schemas(access: Access, has_attachments: bool) -> Vec<Value> {
    let mut tools = vec![
        schema(
            "ask_user",
            &prompt_catalog::text(PromptId::ToolAskUser),
            json!({"type":"object","properties":{"question":{"type":"string"},"options":{"type":"array","minItems":2,"maxItems":4,"items":{"type":"string"}},"recommended_index":{"type":"integer","minimum":0,"maximum":3}},"required":["question"]}),
        ),
        schema(
            "list_directory",
            &prompt_catalog::text(PromptId::ToolListDirectory),
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        ),
        schema(
            "read_file",
            "Read a local text file in bounded character pages. Returns content, nextOffset and sha256. Inspect existing content before changing it; pass its sha256 as expectedSha256 to write_file. Supports text files up to 64 MiB without filling model context.",
            json!({"type":"object","properties":{"path":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":16000}},"required":["path"]}),
        ),
        schema(
            "write_file",
            "Save exact text to an approved path with a recovery copy. To overwrite an existing file, first read it and pass its exact sha256 as expectedSha256; intervening changes are rejected. Omit expectedSha256 only for a new file. Never overwrite another person's changes without inspecting and preserving their work.",
            json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"},"expectedSha256":{"type":"string"}},"required":["path","content"]}),
        ),
        schema(
            "create_directory",
            &prompt_catalog::text(PromptId::ToolCreateDirectory),
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        ),
        schema(
            "move_path",
            &prompt_catalog::text(PromptId::ToolMovePath),
            json!({"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"}},"required":["from","to"]}),
        ),
        schema(
            "copy_file",
            &prompt_catalog::text(PromptId::ToolCopyFile),
            json!({"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"}},"required":["from","to"]}),
        ),
    ];
    if has_attachments {
        tools.push(schema("read_attachment", &prompt_catalog::text(PromptId::ToolReadAttachment), json!({"type":"object","properties":{"attachment_id":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100000}},"required":["attachment_id"]})));
    }
    if access == Access::Full {
        tools.extend([
            schema("run_program", &prompt_catalog::text(PromptId::ToolRunProgram), json!({"type":"object","properties":{"program":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"cwd":{"type":"string"},"timeout_seconds":{"type":"integer","minimum":1,"maximum":600}},"required":["program","args","cwd"]})),
            schema("list_processes", &prompt_catalog::text(PromptId::ToolListProcesses), json!({"type":"object","properties":{}})),
            schema("open_path", &prompt_catalog::text(PromptId::ToolOpenPath), json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]})),
        ]);
    }
    tools
}

struct ClarifyingQuestion {
    question: String,
    options: Vec<String>,
    recommended_index: Option<usize>,
}

fn parse_question(arguments: &Value) -> Result<ClarifyingQuestion, String> {
    let question = arguments
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "ask_user requires a non-empty question".to_string())?;
    if question.chars().count() > 600 {
        return Err("ask_user question exceeds 600 characters".into());
    }
    let options = arguments
        .get("options")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .take(4)
                .map(|value| value.chars().take(240).collect::<String>())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let recommended_index = arguments
        .get("recommended_index")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .filter(|index| *index < options.len());
    Ok(ClarifyingQuestion {
        question: question.to_string(),
        options,
        recommended_index,
    })
}

fn schema(name: &str, description: &str, parameters: Value) -> Value {
    json!({"type":"function","function":{"name":name,"description":description,"parameters":parameters}})
}

async fn execute_tool(
    name: &str,
    arguments: &Value,
    access: Access,
    roots: &[String],
    attachments: &AttachmentStore,
    attachment_ids: &[String],
) -> Result<ToolOutput, String> {
    let string = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("missing string argument: {key}"))
    };
    match name {
        "read_attachment" => {
            let id = string("attachment_id")?;
            if !attachment_ids.iter().any(|known| known == id) {
                return Err("That attachment is not part of this durable task.".into());
            }
            let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(20_000)
                .clamp(1, 100_000) as usize;
            let text = attachments.read_extracted(id, offset, limit)?;
            Ok(ToolOutput {
                text,
                artifact: None,
                data: Some(json!({"attachmentId":id,"offset":offset,"limit":limit})),
            })
        }
        "list_directory" => {
            let path = allowed_existing(string("path")?, access, roots)?;
            if !path.is_dir() {
                return Err(format!("not a directory: {}", path.display()));
            }
            let mut rows = Vec::new();
            for entry in fs::read_dir(&path)
                .map_err(|error| error.to_string())?
                .take(500)
            {
                let entry = entry.map_err(|error| error.to_string())?;
                let metadata = entry.metadata().map_err(|error| error.to_string())?;
                rows.push(format!(
                    "{}\t{}\t{} bytes",
                    if metadata.is_dir() { "DIR" } else { "FILE" },
                    entry.file_name().to_string_lossy(),
                    metadata.len()
                ));
            }
            rows.sort();
            Ok(output(rows.join("\n")))
        }
        "read_file" => {
            let path = allowed_existing(string("path")?, access, roots)?;
            let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
            if !metadata.is_file() || metadata.len() > 64 * 1_048_576 {
                return Err("read_file accepts a text file up to 64 MiB in bounded pages".into());
            }
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            use sha2::Digest;
            let sha256 = hex::encode(sha2::Sha256::digest(&bytes));
            let text = std::str::from_utf8(&bytes).map_err(|_| "read_file requires UTF-8 text")?;
            let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0);
            let total = text.chars().count() as u64;
            if offset > total {
                return Err("Read offset is beyond the end of the file".into());
            }
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(12_000)
                .clamp(1, 16_000);
            let content: String = text
                .chars()
                .skip(offset as usize)
                .take(limit as usize)
                .collect();
            let next = offset + content.chars().count() as u64;
            let data = json!({"path":path,"sha256":sha256,"text":content,"totalChars":total,"offset":offset,"nextOffset":(next<total).then_some(next)});
            Ok(ToolOutput {
                text: data.to_string(),
                data: Some(data),
                artifact: None,
            })
        }
        "write_file" => {
            let content = string("content")?;
            if content.len() > 5 * 1_048_576 {
                return Err("write_file content exceeds 5 MiB".into());
            }
            let path = allowed_new(string("path")?, access, roots)?;
            verify_write_version(
                &path,
                arguments.get("expectedSha256").and_then(Value::as_str),
            )?;
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            let backup = if path.is_file() {
                let backup = recovery_path(&path);
                fs::copy(&path, &backup).map_err(|error| error.to_string())?;
                Some(backup)
            } else {
                None
            };
            let temporary = path.with_extension(format!(
                "{}.{}.kestrel-tmp",
                path.extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("file"),
                uuid::Uuid::new_v4()
            ));
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)
                .map_err(|error| error.to_string())?;
            let staged = file
                .write_all(content.as_bytes())
                .and_then(|()| file.sync_all());
            drop(file);
            if let Err(error) = staged {
                let _ = fs::remove_file(&temporary);
                return Err(error.to_string());
            }
            if let Err(error) = verify_write_version(
                &path,
                arguments.get("expectedSha256").and_then(Value::as_str),
            ) {
                let _ = fs::remove_file(&temporary);
                return Err(error);
            }
            if let Err(error) = fs::rename(&temporary, &path) {
                let _ = fs::remove_file(&temporary);
                if !path.exists() {
                    if let Some(backup) = &backup {
                        let _ = fs::copy(backup, &path);
                    }
                }
                return Err(error.to_string());
            }
            Ok(ToolOutput {
                text: format!(
                    "Wrote {} bytes to {}{}",
                    content.len(),
                    path.display(),
                    backup
                        .as_ref()
                        .map(|value| format!("; recovery copy: {}", value.display()))
                        .unwrap_or_default()
                ),
                artifact: Some(path.clone()),
                data: Some(
                    json!({"path":path,"backup":backup,"sha256":text_sha256(content.as_bytes())}),
                ),
            })
        }
        "create_directory" => {
            let path = allowed_new(string("path")?, access, roots)?;
            fs::create_dir_all(&path).map_err(|error| error.to_string())?;
            Ok(output(format!("Created {}", path.display())))
        }
        "move_path" => {
            let from = allowed_existing(string("from")?, access, roots)?;
            let to = allowed_new(string("to")?, access, roots)?;
            if to.exists() {
                return Err(format!("destination already exists: {}", to.display()));
            }
            fs::rename(&from, &to).map_err(|error| error.to_string())?;
            Ok(ToolOutput {
                text: format!("Moved {} to {}", from.display(), to.display()),
                artifact: to.is_file().then_some(to),
                data: None,
            })
        }
        "copy_file" => {
            let from = allowed_existing(string("from")?, access, roots)?;
            let to = allowed_new(string("to")?, access, roots)?;
            if !from.is_file() || to.exists() {
                return Err("copy_file requires a file and a new destination".into());
            }
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            let bytes = fs::copy(&from, &to).map_err(|error| error.to_string())?;
            Ok(ToolOutput {
                text: format!("Copied {bytes} bytes to {}", to.display()),
                artifact: Some(to.clone()),
                data: Some(json!({"path":to})),
            })
        }
        "run_program" if access == Access::Full => {
            let program = string("program")?;
            let cwd = allowed_existing(string("cwd")?, access, roots)?;
            let args = arguments
                .get("args")
                .and_then(Value::as_array)
                .ok_or_else(|| "missing argument array: args".to_string())?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .ok_or_else(|| "program args must all be strings".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let seconds = arguments
                .get("timeout_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(120)
                .clamp(1, 600);
            let mut command = Command::new(program);
            command
                .args(args)
                .current_dir(cwd)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let result = timeout(Duration::from_secs(seconds), command.output())
                .await
                .map_err(|_| format!("program exceeded {seconds} seconds"))?
                .map_err(|error| error.to_string())?;
            Ok(output(format!(
                "exit: {}\nstdout:\n{}\nstderr:\n{}",
                result.status,
                truncate(&String::from_utf8_lossy(&result.stdout), 16_000),
                truncate(&String::from_utf8_lossy(&result.stderr), 16_000)
            )))
        }
        "list_processes" if access == Access::Full => {
            let mut command = Command::new("tasklist.exe");
            command
                .arg("/FO")
                .arg("CSV")
                .arg("/NH")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            #[cfg(windows)]
            command.creation_flags(0x08000000);
            let result = command.output().await.map_err(|error| error.to_string())?;
            Ok(output(truncate(
                &String::from_utf8_lossy(&result.stdout),
                24_000,
            )))
        }
        "open_path" if access == Access::Full => {
            let path = allowed_existing(string("path")?, access, roots)?;
            Command::new("explorer.exe")
                .arg(&path)
                .spawn()
                .map_err(|error| error.to_string())?;
            Ok(output(format!("Opened {}", path.display())))
        }
        _ => Err(format!("tool is unavailable in this access mode: {name}")),
    }
}

fn output(text: impl Into<String>) -> ToolOutput {
    ToolOutput {
        text: text.into(),
        artifact: None,
        data: None,
    }
}

pub(crate) fn allowed_existing(
    value: &str,
    access: Access,
    roots: &[String],
) -> Result<PathBuf, String> {
    let path = absolute(value)?;
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    ensure_access(&canonical, access, roots)?;
    Ok(canonical)
}

pub(crate) fn allowed_new(
    value: &str,
    access: Access,
    roots: &[String],
) -> Result<PathBuf, String> {
    let path = absolute(value)?;
    let parent = path
        .parent()
        .ok_or_else(|| "path has no parent".to_string())?;
    let existing_parent = nearest_existing(parent)?;
    let canonical_parent = existing_parent
        .canonicalize()
        .map_err(|error| error.to_string())?;
    ensure_access(&canonical_parent, access, roots)?;
    if fs::symlink_metadata(&path).is_ok() {
        let canonical = path.canonicalize().map_err(|e| {
            format!(
                "Cannot resolve existing destination {}: {e}",
                path.display()
            )
        })?;
        ensure_access(&canonical, access, roots)?;
    }
    Ok(path)
}

fn absolute(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err("tool paths must be absolute".into());
    }
    if value.contains('*') || value.contains('?') {
        return Err("wildcards are not accepted in tool paths".into());
    }
    Ok(path)
}

fn nearest_existing(path: &Path) -> Result<PathBuf, String> {
    let mut current = path;
    loop {
        if current.exists() {
            return Ok(current.to_path_buf());
        }
        current = current
            .parent()
            .ok_or_else(|| "no existing parent directory".to_string())?;
    }
}

fn ensure_access(path: &Path, access: Access, roots: &[String]) -> Result<(), String> {
    if access == Access::Full {
        return Ok(());
    }
    for root in roots {
        if let Ok(canonical) = Path::new(root).canonicalize() {
            if path.starts_with(canonical) {
                return Ok(());
            }
        }
    }
    Err(format!(
        "path is outside the approved workspace: {}",
        path.display()
    ))
}

fn recovery_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("file");
    path.with_file_name(format!(
        "{name}.kestrel-backup-{}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S"),
        uuid::Uuid::new_v4()
    ))
}

fn text_sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(bytes))
}

fn verify_write_version(path: &Path, expected: Option<&str>) -> Result<(), String> {
    if path.exists() {
        let expected = expected.ok_or("This file already exists. Read it first, preserve existing work and pass its sha256 as expectedSha256 before replacing it.")?;
        if fs::metadata(path).map_err(|e| e.to_string())?.len() > 64 * 1_048_576 {
            return Err("This file exceeds the 64 MiB safe text overwrite limit.".into());
        }
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        if text_sha256(&bytes) != expected {
            return Err("The file changed after it was inspected. Read it again and preserve the newer changes before saving.".into());
        }
    } else if expected.is_some() {
        return Err("The inspected file no longer exists. Re-inspect the destination before creating a replacement.".into());
    }
    Ok(())
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

/// Durable records carry old history. This only selects whole recent assistant/tool groups;
/// it never promotes tool text into system instructions or recursively summarizes summaries.
fn compact_messages(messages: Vec<Value>, max_chars: usize) -> Result<Vec<Value>, String> {
    let mut iter = messages.into_iter();
    let mut output: Vec<Value> = iter.by_ref().take(2).collect();
    let mut used = output.iter().map(|v| v.to_string().len()).sum::<usize>();
    if used > max_chars {
        return Err("The task's fixed context exceeds this model's input budget. Its records are saved; lower the output reservation or use a larger supported context before continuing.".into());
    }
    let mut groups: Vec<Vec<Value>> = Vec::new();
    for message in iter {
        if message.get("role").and_then(Value::as_str) == Some("assistant") || groups.is_empty() {
            groups.push(Vec::new());
        }
        groups
            .last_mut()
            .expect("a group was created")
            .push(message);
    }
    let mut kept = Vec::new();
    for group in groups.into_iter().rev() {
        let size = group.iter().map(|v| v.to_string().len()).sum::<usize>();
        if used.saturating_add(size) > max_chars {
            break;
        }
        used += size;
        kept.push(group);
    }
    output.extend(kept.into_iter().rev().flatten());
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_limit_is_not_success_even_when_it_contains_summary_or_tools() {
        assert!(completion_hit_limit(
            &json!({"choices":[{"finish_reason":"length","message":{"content":"I will assemble it now","tool_calls":[{"function":{"name":"write_file","arguments":"{partial"}}]}}]})
        ));
        assert!(!completion_hit_limit(
            &json!({"choices":[{"finish_reason":"stop","message":{"content":"The saved output was verified."}}]})
        ));
        assert!(!completion_hit_limit(
            &json!({"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[]}}]})
        ));
    }

    #[test]
    fn strata_receives_its_real_budget_key_and_reserves_output_space() {
        use crate::models::{ModelEngine, ThinkingLevel};
        let mut strata = json!({});
        apply_thinking_budget(&mut strata, ModelEngine::Strata, ThinkingLevel::Low, 8192);
        assert_eq!(strata["reasoning_budget_tokens"], 2048);
        assert!(strata.get("thinking_budget_tokens").is_none());
        apply_thinking_budget(&mut strata, ModelEngine::Strata, ThinkingLevel::Max, 8192);
        assert_eq!(strata["reasoning_budget_tokens"], 7680);
        let mut llama = json!({});
        apply_thinking_budget(&mut llama, ModelEngine::LlamaCpp, ThinkingLevel::Off, 8192);
        assert_eq!(llama["thinking_budget_tokens"], 0);
    }

    #[test]
    fn workspace_paths_cannot_escape_an_approved_root() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("workspace");
        fs::create_dir_all(&root).unwrap();
        let roots = vec![root.to_string_lossy().into_owned()];
        assert!(allowed_new(
            &root.join("result.svg").to_string_lossy(),
            Access::Workspace,
            &roots
        )
        .is_ok());
        assert!(allowed_new(
            &directory.path().join("outside.svg").to_string_lossy(),
            Access::Workspace,
            &roots
        )
        .is_err());
    }

    #[test]
    fn tool_paths_must_be_absolute_and_cannot_use_globs() {
        assert!(absolute("relative.txt").is_err());
        assert!(absolute(r"C:\\Users\\*\\file.txt").is_err());
    }

    #[test]
    fn repeated_compaction_is_bounded_and_keeps_policy_objective_and_whole_groups() {
        let mut messages = vec![
            json!({"role":"system","content":"policy and recovery packet"}),
            json!({"role":"user","content":"original objective"}),
        ];
        for i in 0..100 {
            messages.push(json!({"role":"assistant","tool_calls":[{"id":i.to_string()}]}));
            messages.push(
                json!({"role":"tool","tool_call_id":i.to_string(),"content":"x".repeat(100)}),
            );
            messages = compact_messages(messages, 600).unwrap();
            assert!(messages.iter().map(|v| v.to_string().len()).sum::<usize>() <= 600);
            assert_eq!(messages[0]["content"], "policy and recovery packet");
            assert_eq!(messages[1]["content"], "original objective");
            assert_eq!(messages.last().unwrap()["tool_call_id"], i.to_string());
            for pair in messages[2..].chunks(2) {
                assert_eq!(pair.len(), 2);
                assert_eq!(pair[0]["tool_calls"][0]["id"], pair[1]["tool_call_id"]);
            }
        }
        assert!(compact_messages(messages, 10).is_err());
    }

    #[test]
    fn clarification_parser_bounds_options_and_recommendation() {
        let question = parse_question(&json!({
            "question":"Which destination should I use?",
            "options":["Safe folder","Existing folder","Third","Fourth","Ignored"],
            "recommended_index":1
        }))
        .unwrap();
        assert_eq!(question.options.len(), 4);
        assert_eq!(question.recommended_index, Some(1));
        let question = parse_question(&json!({
            "question":"Which destination should I use?",
            "options":["Safe folder","Existing folder"],
            "recommended_index":2
        }))
        .unwrap();
        assert_eq!(question.recommended_index, None);
        assert!(parse_question(&json!({"question":"  "})).is_err());
    }

    #[tokio::test]
    async fn write_tool_is_atomic_auditable_and_recoverable() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("workspace");
        fs::create_dir_all(&root).unwrap();
        let path = root.join("cat.svg");
        fs::write(&path, "old version").unwrap();
        let other_staging = path.with_extension("svg.kestrel-tmp");
        fs::write(&other_staging, "another writer's unfinished file").unwrap();
        let attachment_store = AttachmentStore::new(directory.path()).unwrap();
        let result = execute_tool(
            "write_file",
            &json!({"path":path,"content":"<svg><path d=\"M0 0\"/></svg>","expectedSha256":text_sha256(b"old version")}),
            Access::Workspace,
            &[root.to_string_lossy().into_owned()],
            &attachment_store,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "<svg><path d=\"M0 0\"/></svg>"
        );
        assert_eq!(result.artifact.as_deref(), Some(path.as_path()));
        let backup = result
            .data
            .as_ref()
            .and_then(|value| value.get("backup"))
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(fs::read_to_string(backup).unwrap(), "old version");
        let second = execute_tool(
            "write_file",
            &json!({"path":path,"content":"third version","expectedSha256":result.data.as_ref().unwrap()["sha256"]}),
            Access::Workspace,
            &[root.to_string_lossy().into_owned()],
            &attachment_store,
            &[],
        ).await.unwrap();
        let second_backup = second.data.as_ref().unwrap()["backup"].as_str().unwrap();
        assert_ne!(backup, second_backup);
        assert_eq!(fs::read_to_string(backup).unwrap(), "old version");
        assert_eq!(
            fs::read_to_string(second_backup).unwrap(),
            "<svg><path d=\"M0 0\"/></svg>"
        );
        assert_eq!(
            fs::read_to_string(other_staging).unwrap(),
            "another writer's unfinished file"
        );
    }

    #[tokio::test]
    async fn inspected_file_cannot_overwrite_another_writers_new_changes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shared.txt");
        fs::write(&path, "human version").unwrap();
        let attachments = AttachmentStore::new(directory.path()).unwrap();
        let roots = vec![directory.path().to_string_lossy().into_owned()];
        let read = execute_tool(
            "read_file",
            &json!({"path":path}),
            Access::Workspace,
            &roots,
            &attachments,
            &[],
        )
        .await
        .unwrap();
        let sha = read.data.unwrap()["sha256"].clone();
        fs::write(&path, "new changes from someone else").unwrap();
        let stale = execute_tool(
            "write_file",
            &json!({"path":path,"content":"agent proposal","expectedSha256":sha}),
            Access::Workspace,
            &roots,
            &attachments,
            &[],
        )
        .await;
        assert!(stale.err().unwrap().contains("changed"));
        let blind = execute_tool(
            "write_file",
            &json!({"path":path,"content":"agent proposal"}),
            Access::Workspace,
            &roots,
            &attachments,
            &[],
        )
        .await;
        assert!(blind.err().unwrap().contains("Read it first"));
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "new changes from someone else"
        );
    }

    #[tokio::test]
    async fn file_pages_handle_large_unicode_documents_with_stable_hashes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.txt");
        fs::write(&path, "🦉ä".repeat(300_000)).unwrap();
        let attachments = AttachmentStore::new(directory.path()).unwrap();
        let roots = vec![directory.path().to_string_lossy().into_owned()];
        let first = execute_tool(
            "read_file",
            &json!({"path":path,"limit":9}),
            Access::Workspace,
            &roots,
            &attachments,
            &[],
        )
        .await
        .unwrap()
        .data
        .unwrap();
        let next = execute_tool(
            "read_file",
            &json!({"path":path,"offset":first["nextOffset"],"limit":9}),
            Access::Workspace,
            &roots,
            &attachments,
            &[],
        )
        .await
        .unwrap()
        .data
        .unwrap();
        assert_eq!(first["totalChars"], 600_000);
        assert_eq!(first["nextOffset"], 9);
        assert_eq!(first["sha256"], next["sha256"]);
        assert_eq!(next["text"], "ä🦉ä🦉ä🦉ä🦉ä");
    }

    #[test]
    fn failed_event_persistence_prevents_starting_work() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let run = store
            .create_task("local", "persist before action", Access::Workspace)
            .unwrap();
        fs::write(
            directory.path().join("workspace/task-journal"),
            "not a directory",
        )
        .unwrap();
        assert!(event(
            &None,
            &store,
            &run.id,
            1,
            "tool_start",
            "write_file",
            "args",
            None
        )
        .is_err());
    }

    #[tokio::test]
    async fn attachment_tool_reads_only_declared_task_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("evidence.txt");
        fs::write(&source, "bounded local evidence").unwrap();
        let attachment_store = AttachmentStore::new(directory.path()).unwrap();
        let attachment = attachment_store.import_path(&source).unwrap();
        let arguments = json!({"attachment_id":attachment.id,"offset":8,"limit":5});
        let denied = execute_tool(
            "read_attachment",
            &arguments,
            Access::Workspace,
            &[],
            &attachment_store,
            &[],
        )
        .await;
        assert!(denied.is_err());
        let allowed = execute_tool(
            "read_attachment",
            &arguments,
            Access::Workspace,
            &[],
            &attachment_store,
            &[attachment.id],
        )
        .await
        .unwrap();
        assert_eq!(allowed.text, "local");
    }
}
