//! Streaming, durable, tool-free local conversations.

use crate::{
    model::ModelInfo,
    models::{ChatStreamEvent, ControlSettings, StartChatRequest},
    runtime::{authorized, RuntimeManager},
    workspace::WorkspaceStore,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

const CHAT_SYSTEM_PROMPT: &str = "You are Kestrel, a capable fully offline assistant. Be clear, practical, and honest about uncertainty. Before committing to an answer, notice decision-critical ambiguity. Ask exactly one focused question per turn instead of guessing when different interpretations would materially change the target, scope, safety, irreversible action, required format, or success criteria. Choose the single most important ambiguity; never bundle several numbered questions. When useful, put two or three concise choices inside that one question and recommend one. Do not interrogate the user or ask about preferences that do not matter: take a safe, reversible default and state it. Never claim an action occurred in ordinary chat; computer actions require Computer Tasks.";

pub struct ChatStreamJob {
    pub app: Option<AppHandle>,
    pub runtime: Arc<RuntimeManager>,
    pub store: WorkspaceStore,
    pub request_id: String,
    pub session_id: String,
    pub request: StartChatRequest,
    pub models: Vec<ModelInfo>,
    pub settings: ControlSettings,
    pub cancel: CancellationToken,
}

impl ChatStreamJob {
    pub async fn run(self) -> Result<(), String> {
        let Self {
            app,
            runtime,
            store,
            request_id,
            session_id,
            request,
            models,
            settings,
            cancel,
        } = self;
        emit(app.as_ref(), &request_id, &session_id, "queued", None, None);
        let lease = tokio::select! {
            lease = runtime.lease_model(&request.model_id, &models, &settings, app.as_ref()) => {
                lease.map_err(|error| error.to_string())?
            }
            _ = cancel.cancelled() => {
                store.add_chat_message_with_status(
                    &session_id,
                    "assistant",
                    "Generation stopped before the first token.".into(),
                    None,
                    Some("interrupted".into()),
                )?;
                emit(app.as_ref(), &request_id, &session_id, "cancelled", None, None);
                return Ok(());
            }
        };
        emit(
            app.as_ref(),
            &request_id,
            &session_id,
            "started",
            None,
            None,
        );
        let session = store.get_chat(&session_id)?;
        let max_output_tokens = if settings.advanced_mode {
            request.max_output_tokens.max(1)
        } else {
            request
                .max_output_tokens
                .max(1)
                .min(settings.max_output_tokens.max(1))
        };
        let all_messages = session
            .messages
            .iter()
            .map(|message| json!({"role": message.role, "content": message.content}))
            .collect::<Vec<_>>();
        let prompt_chars = max_output_tokens
            .checked_add(1_024)
            .and_then(|reserved| settings.context_window.checked_sub(reserved))
            .unwrap_or(1_024)
            .saturating_mul(4) as usize;
        let history_budget = prompt_chars
            .max(4_096)
            .saturating_sub(CHAT_SYSTEM_PROMPT.len());
        let (history, omitted) = fit_recent_messages(&all_messages, history_budget.max(2_048));
        let included = history.len();
        let mut messages = Vec::with_capacity(history.len() + 1);
        messages.push(json!({"role":"system","content":CHAT_SYSTEM_PROMPT}));
        messages.extend(history);
        if omitted > 0 {
            emit(
            app.as_ref(),
            &request_id,
            &session_id,
            "context",
            Some(format!(
                "Using the newest {} messages; {} older messages remain saved but are outside this model turn.",
                included, omitted
            )),
            Some(json!({"included":included,"omitted":omitted})),
        );
        }
        let body = json!({
            "model": lease.connection.model_id,
            "messages": messages,
            "temperature": request.temperature,
            "top_p": request.top_p,
            "top_k": request.top_k,
            "max_tokens": max_output_tokens,
            "stream": true,
            "stream_options": {"include_usage": true}
        });
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(3_600))
            .build()
            .map_err(|error| error.to_string())?;
        let response = authorized(
            client.post(format!("{}/chat/completions", lease.connection.endpoint)),
            &lease.connection,
        )
        .json(&body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(format!(
                "local model returned {status}: {}",
                truncate(&detail, 600)
            ));
        }

        let mut bytes = response.bytes_stream();
        let mut buffer = Vec::<u8>::new();
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut completed = false;
        let mut finish_reason = None::<String>;
        loop {
            let next = tokio::select! {
                value = bytes.next() => value,
                _ = cancel.cancelled() => {
                    if !content.is_empty() || !reasoning.is_empty() {
                        store.add_chat_message_with_status(
                            &session_id,
                            "assistant",
                            content,
                            (!reasoning.is_empty()).then_some(reasoning),
                            Some("interrupted".into()),
                        )?;
                    } else {
                        store.add_chat_message_with_status(
                            &session_id,
                            "assistant",
                            "Generation stopped before the first token.".into(),
                            None,
                            Some("interrupted".into()),
                        )?;
                    }
                    emit(app.as_ref(), &request_id, &session_id, "cancelled", None, None);
                    return Ok(());
                }
            };
            let Some(chunk) = next else { break };
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    if !content.is_empty() || !reasoning.is_empty() {
                        store.add_chat_message_with_status(
                            &session_id,
                            "assistant",
                            content,
                            (!reasoning.is_empty()).then_some(reasoning),
                            Some("interrupted".into()),
                        )?;
                    }
                    return Err(error.to_string());
                }
            };
            buffer.extend_from_slice(&chunk);
            while let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
                let line = buffer.drain(..=end).collect::<Vec<_>>();
                let line = String::from_utf8_lossy(&line);
                let Some(data) = line.trim().strip_prefix("data: ") else {
                    continue;
                };
                if data == "[DONE]" {
                    completed = true;
                    continue;
                }
                let Ok(value) = serde_json::from_str::<Value>(data) else {
                    continue;
                };
                if let Some(token) = value
                    .pointer("/choices/0/delta/content")
                    .and_then(Value::as_str)
                {
                    content.push_str(token);
                    emit(
                        app.as_ref(),
                        &request_id,
                        &session_id,
                        "token",
                        Some(token.to_string()),
                        None,
                    );
                }
                if let Some(reason) = value
                    .pointer("/choices/0/finish_reason")
                    .and_then(Value::as_str)
                {
                    finish_reason = Some(reason.to_string());
                }
                if let Some(token) = value
                    .pointer("/choices/0/delta/reasoning_content")
                    .or_else(|| value.pointer("/choices/0/delta/reasoning"))
                    .and_then(Value::as_str)
                {
                    reasoning.push_str(token);
                    emit(
                        app.as_ref(),
                        &request_id,
                        &session_id,
                        "reasoning",
                        Some(token.to_string()),
                        None,
                    );
                }
                if value.get("usage").is_some() || value.get("timings").is_some() {
                    emit(
                        app.as_ref(),
                        &request_id,
                        &session_id,
                        "metrics",
                        None,
                        Some(value),
                    );
                }
            }
        }
        if !completed {
            if content.is_empty() && reasoning.is_empty() {
                return Err("local model stream ended before producing an answer".into());
            }
            store.add_chat_message_with_status(
                &session_id,
                "assistant",
                content,
                (!reasoning.is_empty()).then_some(reasoning),
                Some("interrupted".into()),
            )?;
            return Err("local model stream ended before confirming completion; the partial answer was saved".into());
        }
        let message_status =
            (finish_reason.as_deref() == Some("length")).then(|| "limited".to_string());
        store.add_chat_message_with_status(
            &session_id,
            "assistant",
            content,
            (!reasoning.is_empty()).then_some(reasoning),
            message_status,
        )?;
        emit(
            app.as_ref(),
            &request_id,
            &session_id,
            "done",
            None,
            finish_reason.map(|reason| json!({"finishReason":reason})),
        );
        Ok(())
    }
}

pub fn emit_error(app: Option<&AppHandle>, request_id: &str, session_id: &str, detail: String) {
    emit(app, request_id, session_id, "error", Some(detail), None);
}

pub fn emit_settled(app: Option<&AppHandle>, request_id: &str, session_id: &str) {
    emit(app, request_id, session_id, "settled", None, None);
}

fn emit(
    app: Option<&AppHandle>,
    request_id: &str,
    session_id: &str,
    kind: &str,
    content: Option<String>,
    data: Option<Value>,
) {
    if let Some(app) = app {
        let _ = app.emit(
            "chat-stream",
            ChatStreamEvent {
                request_id: request_id.to_string(),
                session_id: session_id.to_string(),
                kind: kind.to_string(),
                content,
                data,
                at: chrono::Utc::now().to_rfc3339(),
            },
        );
    }
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

fn fit_recent_messages(messages: &[Value], max_chars: usize) -> (Vec<Value>, usize) {
    let mut selected = Vec::new();
    let mut used = 0usize;
    for message in messages.iter().rev() {
        let length = message.to_string().chars().count();
        if !selected.is_empty() && used.saturating_add(length) > max_chars {
            break;
        }
        used = used.saturating_add(length);
        selected.push(message.clone());
    }
    selected.reverse();
    let omitted = messages.len().saturating_sub(selected.len());
    (selected, omitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_fitting_keeps_the_newest_complete_turns() {
        let messages = vec![
            json!({"role":"user","content":"old old old"}),
            json!({"role":"assistant","content":"middle middle middle"}),
            json!({"role":"user","content":"new"}),
        ];
        let newest_size = messages[2].to_string().len();
        let (selected, omitted) = fit_recent_messages(&messages, newest_size + 1);
        assert_eq!(selected, vec![messages[2].clone()]);
        assert_eq!(omitted, 2);
    }
}
