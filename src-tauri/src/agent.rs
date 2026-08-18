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
use tauri::{AppHandle, Emitter};
use tokio::{process::Command, time::timeout};
use tokio_util::sync::CancellationToken;

type Access = ComputerTaskAccess;

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
    let settings = settings.for_model(&request.model_id);
    let access = request.access;
    if access == Access::Full && !settings.allow_full_access_agent {
        return Err("Full computer access is locked in the runtime profile.".into());
    }
    if access == Access::Workspace && settings.agent_workspace_roots.is_empty() {
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
    );
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
            event(&app, &store, &run_id, 0, "cancelled", "Stopped by you", "The task stopped while preparing local attachment context. No tool was started.", None);
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
        );
        return Ok(());
    }
    let attachment_instruction = if context_attachments.is_empty() {
        String::new()
    } else {
        prompt_catalog::text(PromptId::ComputerAttachmentNotice)
    };
    let roots = if settings.agent_workspace_roots.is_empty() {
        "full access explicitly enabled".into()
    } else {
        settings.agent_workspace_roots.join("; ")
    };
    let system = prompt_catalog::render(
        PromptId::ComputerSystem,
        &[
            ("access_label", access_label),
            ("workspace_roots", &roots),
            ("attachment_instruction", &attachment_instruction),
        ],
    );
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
        );
    }
    let tools = tool_schemas(access, !context_attachments.is_empty());
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
    let lease = tokio::select! {
        result = runtime.lease_model(&request.model_id, &models, &settings, app.as_ref()) => {
            result.map_err(|error| error.to_string())?
        }
        _ = cancel.cancelled() => {
            event(&app, &store, &run_id, 0, "cancelled", "Stopped by you", "No computer action was started.", None);
            return Ok(());
        }
    };
    event(
        &app,
        &store,
        &run_id,
        0,
        "start",
        "Computer task started",
        &format!("{} · {}", lease.connection.model_label, access_label),
        None,
    );
    for step in 1..=max_steps {
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
            );
            return Ok(());
        }
        event(
            &app,
            &store,
            &run_id,
            step,
            "thinking",
            "Model is deciding",
            "Planning the next visible action.",
            None,
        );
        let prompt_chars = max_output_tokens
            .checked_add(2_048)
            .and_then(|reserved| settings.context_window.checked_sub(reserved))
            .unwrap_or(4_096)
            .saturating_mul(4) as usize;
        messages = compact_messages(messages, prompt_chars.max(16_384));
        let response_request = authorized(
            client.post(format!("{}/chat/completions", lease.connection.endpoint)),
            &lease.connection,
        )
        .json(&json!({
            "model": lease.connection.model_id,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto",
            "stream": false,
            "temperature": 0.2,
            "max_tokens": max_output_tokens
        }))
        .send();
        let response = tokio::select! {
            response = response_request => response.map_err(|error| format!("computer task model request failed: {error}"))?,
            _ = cancel.cancelled() => {
                event(&app, &store, &run_id, step, "cancelled", "Stopped by you", "The pending model request was cancelled. No tool was started.", None);
                return Ok(());
            }
        };
        let status = response.status();
        let body: Value = response.json().await.map_err(|error| error.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "computer task model returned {status}: {}",
                truncate(&body.to_string(), 800)
            ));
        }
        let message = body
            .pointer("/choices/0/message")
            .cloned()
            .ok_or_else(|| "computer task model returned no message".to_string())?;
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
            );
        }
        let tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        messages.push(message.clone());
        if tool_calls.is_empty() {
            let answer = message
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("The task ended without a summary.");
            if answer.trim_end().ends_with('?') {
                event(
                    &app,
                    &store,
                    &run_id,
                    step,
                    "question",
                    "Needs your input",
                    answer,
                    Some(json!({"question": answer, "options": []})),
                );
                return Ok(());
            }
            event(
                &app,
                &store,
                &run_id,
                step,
                "done",
                "Completed",
                answer,
                Some(body),
            );
            return Ok(());
        }
        for call in tool_calls {
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
                );
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
                        None,
                    );
                    messages.push(json!({"role":"tool","tool_call_id":call_id,"content":detail}));
                    continue;
                }
            };
            if name == "ask_user" {
                match parse_question(&arguments) {
                    Ok(question) => {
                        event(
                            &app,
                            &store,
                            &run_id,
                            step,
                            "question",
                            "Needs your input",
                            &question.question,
                            Some(json!({
                                "question": question.question,
                                "options": question.options,
                                "recommendedIndex": question.recommended_index
                            })),
                        );
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
                            None,
                        );
                        messages
                            .push(json!({"role":"tool","tool_call_id":call_id,"content":detail}));
                        continue;
                    }
                }
            }
            event(
                &app,
                &store,
                &run_id,
                step,
                "tool_start",
                &format!("Using {name}"),
                argument_text,
                Some(arguments.clone()),
            );
            let result = tokio::select! {
                result = execute_tool(
                    name,
                    &arguments,
                    access,
                    &settings.agent_workspace_roots,
                    &attachment_store,
                    &request.attachment_ids,
                ) => result,
                _ = cancel.cancelled() => {
                    event(&app, &store, &run_id, step, "cancelled", "Stopped by you", "The active operation was cancelled and no further tools will run.", None);
                    return Ok(());
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
            event(
                &app,
                &store,
                &run_id,
                step,
                if failed { "tool_error" } else { "tool_result" },
                &format!("{name} finished"),
                &truncate(&output.text, 8_000),
                output.data,
            );
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
                );
            }
            messages.push(json!({"role":"tool","tool_call_id":call_id,"content":output.text}));
        }
    }
    event(
        &app,
        &store,
        &run_id,
        max_steps,
        "limit",
        "Step limit reached",
        "The task stopped before another action. Increase the limit only after reviewing the transcript.",
        None,
    );
    Ok(())
}

pub fn emit_error(app: Option<&AppHandle>, store: &WorkspaceStore, run_id: &str, detail: String) {
    let owned = app.cloned();
    event(
        &owned,
        store,
        run_id,
        0,
        "error",
        "Computer task stopped",
        &detail,
        None,
    );
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
) {
    let value = ComputerTaskEvent {
        run_id: run_id.to_string(),
        step,
        kind: kind.to_string(),
        title: title.to_string(),
        detail: detail.to_string(),
        data,
        at: chrono::Utc::now().to_rfc3339(),
    };
    if let Err(error) = store.add_task_event(value.clone()) {
        eprintln!(
            "failed to persist computer task event run={run_id} step={step} kind={kind}: {error}"
        );
    }
    if let Some(app) = app {
        let _ = app.emit("computer-task-event", value);
    }
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
            &prompt_catalog::text(PromptId::ToolReadFile),
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        ),
        schema(
            "write_file",
            &prompt_catalog::text(PromptId::ToolWriteFile),
            json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
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
            if !metadata.is_file() || metadata.len() > 1_048_576 {
                return Err("read_file accepts a text file up to 1 MiB".into());
            }
            Ok(output(
                fs::read_to_string(path).map_err(|error| error.to_string())?,
            ))
        }
        "write_file" => {
            let content = string("content")?;
            if content.len() > 5 * 1_048_576 {
                return Err("write_file content exceeds 5 MiB".into());
            }
            let path = allowed_new(string("path")?, access, roots)?;
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
                "{}.kestrel-tmp",
                path.extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("file")
            ));
            fs::write(&temporary, content.as_bytes()).map_err(|error| error.to_string())?;
            if path.exists() {
                if let Err(error) = fs::remove_file(&path) {
                    let _ = fs::remove_file(&temporary);
                    return Err(error.to_string());
                }
            }
            if let Err(error) = fs::rename(&temporary, &path) {
                let _ = fs::remove_file(&temporary);
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
                data: Some(json!({"path":path,"backup":backup})),
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

fn allowed_existing(value: &str, access: Access, roots: &[String]) -> Result<PathBuf, String> {
    let path = absolute(value)?;
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    ensure_access(&canonical, access, roots)?;
    Ok(canonical)
}

fn allowed_new(value: &str, access: Access, roots: &[String]) -> Result<PathBuf, String> {
    let path = absolute(value)?;
    let parent = path
        .parent()
        .ok_or_else(|| "path has no parent".to_string())?;
    let existing_parent = nearest_existing(parent)?;
    let canonical_parent = existing_parent
        .canonicalize()
        .map_err(|error| error.to_string())?;
    ensure_access(&canonical_parent, access, roots)?;
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
        "{name}.kestrel-backup-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    ))
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn compact_messages(messages: Vec<Value>, max_chars: usize) -> Vec<Value> {
    let total = messages
        .iter()
        .map(|value| value.to_string().len())
        .sum::<usize>();
    if total <= max_chars || messages.len() <= 4 {
        return messages;
    }
    let prefix = messages.iter().take(2).cloned().collect::<Vec<_>>();
    let mut groups = Vec::<Vec<Value>>::new();
    for message in messages.into_iter().skip(2) {
        let starts_group = message.get("role").and_then(Value::as_str) == Some("assistant");
        if starts_group || groups.is_empty() {
            groups.push(Vec::new());
        }
        if let Some(group) = groups.last_mut() {
            group.push(message);
        }
    }
    let prefix_chars = prefix
        .iter()
        .map(|value| value.to_string().len())
        .sum::<usize>();
    let recent_budget = max_chars.saturating_sub(prefix_chars).saturating_mul(3) / 4;
    let mut kept = Vec::<Vec<Value>>::new();
    let mut used = 0usize;
    while let Some(group) = groups.pop() {
        let size = group
            .iter()
            .map(|value| value.to_string().len())
            .sum::<usize>();
        if !kept.is_empty() && used.saturating_add(size) > recent_budget {
            groups.push(group);
            break;
        }
        used = used.saturating_add(size);
        kept.push(group);
    }
    kept.reverse();
    if groups.is_empty() {
        let mut output = prefix;
        output.extend(kept.into_iter().flatten());
        return output;
    }
    let mut ledger = Vec::new();
    for group in &groups {
        for message in group {
            if message.get("role").and_then(Value::as_str) == Some("tool") {
                let content = message
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                ledger.push(truncate(content, 500));
            }
        }
    }
    let memory = prompt_catalog::render(
        PromptId::ComputerCompaction,
        &[
            ("omitted_count", &groups.len().to_string()),
            ("tool_results", &ledger.join("\n---\n")),
        ],
    );
    let mut output = prefix;
    if let Some(system) = output.first_mut() {
        let original = system
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default();
        system["content"] = Value::String(format!("{original}\n\n{memory}"));
    }
    output.extend(kept.into_iter().flatten());
    output
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn compact_memory_keeps_objective_and_recent_tool_group() {
        let messages = vec![
            json!({"role":"system","content":"policy"}),
            json!({"role":"user","content":"objective"}),
            json!({"role":"assistant","tool_calls":[{"id":"old"}]}),
            json!({"role":"tool","tool_call_id":"old","content":"old result that should enter the ledger"}),
            json!({"role":"assistant","tool_calls":[{"id":"new"}]}),
            json!({"role":"tool","tool_call_id":"new","content":"new result"}),
        ];
        let compacted = compact_messages(messages, 180);
        assert!(compacted[0]["content"]
            .as_str()
            .is_some_and(|value| value.starts_with("policy\n\n")));
        assert_eq!(compacted[1]["content"], "objective");
        assert!(compacted
            .iter()
            .any(|message| message["content"] == "new result"));
        assert!(compacted.iter().any(|message| message["content"]
            .as_str()
            .is_some_and(|value| value.contains("COMPACT SHARED MEMORY"))));
        assert_eq!(
            compacted
                .iter()
                .filter(|message| message["role"] == "system")
                .count(),
            1
        );
        assert_eq!(compacted[0]["role"], "system");
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
        let attachment_store = AttachmentStore::new(directory.path()).unwrap();
        let result = execute_tool(
            "write_file",
            &json!({"path":path,"content":"<svg><path d=\"M0 0\"/></svg>"}),
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
