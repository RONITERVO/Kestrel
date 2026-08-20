//! Studio Director planning protocol and durable conversation primitives.
//!
//! Model request construction, SSE assembly, typed tool-only retries, explicit redacted views,
//! and lossless transcript persistence live here so every Studio model call follows the same
//! bounded contract.

use super::{
    model_stream::{reasoning_delta, OpenAiSseDecoder, OpenAiStreamEvent},
    write_json_atomic, MovieSettings, StudioError,
};
use crate::prompt_catalog::{render, PromptId};
use crate::runtime::{authorized, ModelConnection};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

/// Durable conversation state for one Studio Director context session.
///
/// Mutations are persisted through this type so orchestration code cannot accidentally update the
/// in-memory prompt without updating the producer-inspectable transcript.
pub(super) struct AgentTranscript {
    path: PathBuf,
    messages: Vec<Value>,
    step: u32,
}

impl AgentTranscript {
    pub(super) fn begin(
        path: PathBuf,
        step: u32,
        system_prompt: &str,
        instruction: &str,
    ) -> Result<Self, StudioError> {
        let transcript = Self {
            path,
            messages: vec![
                json!({"role":"system","content":system_prompt}),
                json!({"role":"user","content":instruction}),
            ],
            step,
        };
        transcript.persist()?;
        Ok(transcript)
    }

    /// Continue a producer-visible session without discarding any completed turns.
    pub(super) fn resume(path: PathBuf, instruction: &str) -> Result<Self, StudioError> {
        const MAX_TRANSCRIPT_BYTES: u64 = 2 * 1024 * 1024;
        let metadata = fs::metadata(&path)?;
        if metadata.len() > MAX_TRANSCRIPT_BYTES {
            return Err(StudioError::Invalid(
                "the saved agent transcript exceeds the 2 MiB resume limit".into(),
            ));
        }
        let value: Value = serde_json::from_slice(&fs::read(&path)?)?;
        let step = value
            .get("step")
            .and_then(Value::as_u64)
            .unwrap_or_default() as u32;
        let mut messages = value
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                StudioError::Invalid("the saved agent transcript has no messages".into())
            })?;
        if messages.is_empty() || messages.len() > 2_048 {
            return Err(StudioError::Invalid(
                "the saved agent transcript has an invalid message count".into(),
            ));
        }
        messages.push(json!({"role":"user","content":instruction}));
        let transcript = Self {
            path,
            messages,
            step,
        };
        transcript.persist()?;
        Ok(transcript)
    }

    pub(super) fn request_messages(&self, authoritative_memory: String) -> Vec<Value> {
        let mut messages = self
            .messages
            .iter()
            .cloned()
            .map(compact_superseded_context_for_request)
            .collect::<Vec<_>>();
        messages.push(json!({"role":"user","content":authoritative_memory}));
        messages
    }

    pub(super) fn push(&mut self, message: Value, step: u32) -> Result<(), StudioError> {
        self.step = step;
        self.messages.push(message);
        self.persist()
    }

    pub(super) fn extend<I>(&mut self, messages: I, step: u32) -> Result<(), StudioError>
    where
        I: IntoIterator<Item = Value>,
    {
        self.step = step;
        self.messages.extend(messages);
        self.persist()
    }

    pub(super) fn persist(&self) -> Result<(), StudioError> {
        write_json_atomic(
            &self.path,
            &json!({
                "updatedAt": Utc::now().to_rfc3339(),
                "step": self.step,
                "messages": self.messages,
            }),
        )
    }
}

fn compact_superseded_context_for_request(mut message: Value) -> Value {
    let is_tool = message.get("role").and_then(Value::as_str) == Some("tool");
    let is_legacy_context = message
        .get("content")
        .and_then(Value::as_str)
        .is_some_and(|content| content.starts_with("CONTEXT: "));
    if is_tool && is_legacy_context {
        message["content"] = json!("The complete current authoritative context is supplied as the final user message of this turn. This older duplicate is omitted from model input but remains unchanged in the durable transcript.");
    }
    message
}

#[derive(Debug)]
pub(super) struct AssistantTurn {
    history_message: Value,
    tool_calls: Vec<Value>,
}

impl AssistantTurn {
    pub(super) fn from_response(response: &Value) -> Result<Self, StudioError> {
        let message = response_message(response)?;
        let tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut history_message = message;
        if let Some(object) = history_message.as_object_mut() {
            object.remove("reasoning");
            object.remove("reasoning_content");
        }
        Ok(Self {
            history_message,
            tool_calls,
        })
    }

    pub(super) fn history_message(&self) -> Value {
        self.history_message.clone()
    }

    pub(super) fn tool_calls(&self) -> &[Value] {
        &self.tool_calls
    }

    pub(super) fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

pub(super) fn sanitize_chat_messages(messages: &[Value]) -> Vec<Value> {
    messages
        .iter()
        .cloned()
        .map(|mut message| {
            if let Some(content) = message.get_mut("content") {
                if let Some(text) = content.as_str() {
                    *content = Value::String(
                        text.replace("```", "''' ")
                            .replace("<think>", "[reasoning omitted]")
                            .replace("</think>", "[end reasoning]"),
                    );
                }
            }
            message
        })
        .collect()
}

pub(super) fn redacted_transcript_view(transcript: &Value) -> Value {
    let mut view = transcript.clone();
    if let Some(messages) = transcript.get("messages").and_then(Value::as_array) {
        view["messages"] = Value::Array(sanitize_chat_messages(messages));
    }
    view
}

pub(super) fn response_message(response: &Value) -> Result<Value, StudioError> {
    response
        .pointer("/choices/0/message")
        .cloned()
        .ok_or_else(|| {
            StudioError::Planning(format!(
                "missing model message: {}",
                super::truncate(&response.to_string(), 500)
            ))
        })
}

pub(super) fn movie_agent_request(
    model_id: &str,
    messages: &[Value],
    tools: &Value,
    settings: &MovieSettings,
    runtime_max_output_tokens: u32,
) -> Value {
    let mut req = json!({
        "model": model_id,
        "messages": messages,
        "tools": tools,
        "tool_choice": "required",
        "parallel_tool_calls": false,
        "stream": false,
        "temperature": settings.temperature,
        "top_p": settings.top_p,
        "top_k": settings.top_k,
        "max_tokens": settings.max_output_tokens.min(runtime_max_output_tokens),
        "thinking_budget_tokens": settings.thinking_budget,
    });
    if settings.thinking_budget == 0 {
        req["chat_template_kwargs"] = json!({"enable_thinking": false, "reasoning": false});
        req["reasoning_effort"] = json!("off");
    } else {
        let level = crate::models::ThinkingLevel::from_budget(settings.thinking_budget);
        req["reasoning_effort"] = json!(level.as_str());
        req["chat_template_kwargs"] = json!({
            "reasoning_effort": level.as_template_effort(),
            "enable_thinking": true
        });
    }
    req
}

pub(super) struct StreamCompletionRequest<'a> {
    pub connection: &'a ModelConnection,
    pub messages: &'a [Value],
    pub tools: &'a Value,
    pub settings: &'a MovieSettings,
    pub runtime_max_output_tokens: u32,
    pub cancel: &'a CancellationToken,
    pub audit_path: Option<&'a Path>,
    pub fallback_tool_call_prefix: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum StreamEvent {
    Content(String),
    Reasoning(String),
    ToolArgumentsStarted,
    ToolArguments(String),
    AttemptStarted {
        attempt: u8,
        maximum: u8,
    },
    SubmissionInvalid(String),
    Terminal {
        status: &'static str,
        detail: String,
        completion_marker_seen: bool,
        finish_reason: Option<String>,
    },
}

#[derive(Default)]
struct StreamedToolCall {
    id: String,
    name: String,
    arguments: String,
    activity_announced: bool,
}

/// Streams one tool-only model turn while assembling an OpenAI-compatible response object.
pub(super) async fn complete_stream(
    client: &Client,
    request: StreamCompletionRequest<'_>,
    mut on_event: impl FnMut(StreamEvent),
) -> Result<Value, StudioError> {
    let mut body = movie_agent_request(
        &request.connection.model_id,
        request.messages,
        request.tools,
        request.settings,
        request.runtime_max_output_tokens,
    );
    body["stream"] = json!(true);
    body["stream_options"] = json!({"include_usage": true});
    if let Some(path) = request.audit_path {
        write_json_atomic(path, &body)?;
    }
    let response = match authorized(
        client.post(format!("{}/chat/completions", request.connection.endpoint)),
        request.connection,
    )
    .json(&body)
    .send()
    .await
    {
        Ok(response) => response,
        Err(error) => {
            on_event(StreamEvent::Terminal {
                status: "failed",
                detail: format!(
                    "The model request could not start or lost its connection: {error}"
                ),
                completion_marker_seen: false,
                finish_reason: None,
            });
            return Err(error.into());
        }
    };
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        on_event(StreamEvent::Terminal {
            status: "failed",
            detail: format!("Model HTTP request failed with status {status}"),
            completion_marker_seen: false,
            finish_reason: None,
        });
        return Err(StudioError::Planning(format!(
            "movie agent HTTP {status}: {}",
            super::truncate(&text, 500)
        )));
    }

    let mut stream = response.bytes_stream();
    let mut decoder = OpenAiSseDecoder::default();
    let mut content = String::new();
    let mut tool_calls = Vec::<StreamedToolCall>::new();
    let mut finish_reason = None::<String>;
    let mut accept_events = |events: Vec<OpenAiStreamEvent>| {
        for event in events {
            let OpenAiStreamEvent::Message(value) = event else {
                continue;
            };
            if let Some(reason) = value
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
            {
                finish_reason = Some(reason.to_string());
            }
            if let Some(token) = value
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
            {
                content.push_str(token);
                on_event(StreamEvent::Content(token.to_string()));
            }
            if let Some(token) = reasoning_delta(&value) {
                on_event(StreamEvent::Reasoning(token.to_string()));
            }
            if let Some(deltas) = value
                .pointer("/choices/0/delta/tool_calls")
                .and_then(Value::as_array)
            {
                collect_tool_deltas(&mut tool_calls, deltas, &mut on_event);
            }
        }
    };
    loop {
        let next = tokio::select! {
            value = stream.next() => value,
            _ = request.cancel.cancelled() => {
                on_event(StreamEvent::Terminal {
                    status: "cancelled",
                    detail: "Producer stopped this model turn; every token received before the checkpoint is retained".into(),
                    completion_marker_seen: false,
                    finish_reason: None,
                });
                return Err(StudioError::Cancelled);
            },
        };
        let Some(chunk) = next else { break };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                on_event(StreamEvent::Terminal {
                    status: "failed",
                    detail: format!("The model response connection failed: {error}"),
                    completion_marker_seen: false,
                    finish_reason: finish_reason.clone(),
                });
                return Err(error.into());
            }
        };
        let events = match decoder.push(&chunk) {
            Ok(events) => events,
            Err(error) => {
                let detail = format!("movie agent stream error: {error}");
                on_event(StreamEvent::Terminal {
                    status: "failed",
                    detail: detail.clone(),
                    completion_marker_seen: stream_error_saw_completion_marker(&error),
                    finish_reason: finish_reason.clone(),
                });
                return Err(StudioError::Planning(detail));
            }
        };
        accept_events(events);
    }
    let final_events = match decoder.finish() {
        Ok(events) => events,
        Err(error) => {
            let detail = format!("movie agent stream error: {error}");
            on_event(StreamEvent::Terminal {
                status: "failed",
                detail: detail.clone(),
                completion_marker_seen: stream_error_saw_completion_marker(&error),
                finish_reason: finish_reason.clone(),
            });
            return Err(StudioError::Planning(detail));
        }
    };
    accept_events(final_events);
    if finish_reason_indicates_truncation(finish_reason.as_deref()) {
        on_event(StreamEvent::Terminal {
            status: "truncated",
            detail: "The local model reached its generation or context limit; its incomplete output is retained but cannot be accepted as a typed action".into(),
            completion_marker_seen: true,
            finish_reason: finish_reason.clone(),
        });
        return Err(StudioError::Planning(
            "the local model reached its generation or context limit while streaming a Studio tool call; the incomplete call was discarded and the durable workspace can resume in a clean context"
                .into(),
        ));
    }
    on_event(StreamEvent::Terminal {
        status: "complete",
        detail: "The model stream supplied its completion marker".into(),
        completion_marker_seen: true,
        finish_reason: finish_reason.clone(),
    });
    let tool_calls = tool_calls
        .into_iter()
        .enumerate()
        .filter(|(_, call)| !call.name.is_empty() || !call.arguments.is_empty())
        .map(|(index, call)| {
            json!({
                "id": if call.id.is_empty() {
                    format!("{}-{index}", request.fallback_tool_call_prefix)
                } else {
                    call.id
                },
                "type": "function",
                "function": {"name": call.name, "arguments": call.arguments},
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({"choices":[{"message":{
        "role":"assistant",
        "content":if content.is_empty() { Value::Null } else { Value::String(content) },
        "tool_calls":tool_calls,
    }}]}))
}

fn stream_error_saw_completion_marker(error: &str) -> bool {
    error.contains("after its completion marker")
        || error.contains("more than one completion marker")
}

fn finish_reason_indicates_truncation(reason: Option<&str>) -> bool {
    matches!(reason, Some("length" | "max_tokens"))
}

fn collect_tool_deltas(
    tool_calls: &mut Vec<StreamedToolCall>,
    deltas: &[Value],
    on_event: &mut impl FnMut(StreamEvent),
) {
    for delta in deltas {
        let index = delta
            .get("index")
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize;
        while tool_calls.len() <= index {
            tool_calls.push(StreamedToolCall::default());
        }
        let call = &mut tool_calls[index];
        if let Some(fragment) = delta.get("id").and_then(Value::as_str) {
            call.id.push_str(fragment);
        }
        if let Some(fragment) = delta.pointer("/function/name").and_then(Value::as_str) {
            call.name.push_str(fragment);
        }
        if let Some(fragment) = delta.pointer("/function/arguments").and_then(Value::as_str) {
            if !call.activity_announced {
                call.activity_announced = true;
                on_event(StreamEvent::ToolArgumentsStarted);
            }
            call.arguments.push_str(fragment);
            on_event(StreamEvent::ToolArguments(fragment.to_string()));
        }
    }
}

pub(super) struct ToolSubmissionRequest<'a> {
    pub connection: &'a ModelConnection,
    pub initial_messages: &'a [Value],
    pub tool_name: &'a str,
    pub tool_description: &'a str,
    pub response_format: Value,
    pub settings: &'a MovieSettings,
    pub runtime_max_output_tokens: u32,
    pub label: &'a str,
    pub audit_path: Option<&'a Path>,
    pub cancel: Option<&'a CancellationToken>,
    pub on_event: Option<&'a mut (dyn FnMut(StreamEvent) + Send)>,
}

/// Completes a typed, tool-only model exchange with bounded corrective retries.
///
/// The exact request is optionally persisted before every attempt. Invalid model output is fed
/// back into the same conversation, while private reasoning fields never enter durable history.
pub(super) async fn complete_tool_submission<T: DeserializeOwned>(
    client: &Client,
    request: ToolSubmissionRequest<'_>,
) -> Result<T, StudioError> {
    let ToolSubmissionRequest {
        connection,
        initial_messages,
        tool_name,
        tool_description,
        response_format,
        settings,
        runtime_max_output_tokens,
        label,
        audit_path,
        cancel,
        mut on_event,
    } = request;
    let schema = response_format
        .pointer("/json_schema/schema")
        .cloned()
        .unwrap_or(response_format);
    let tools = json!([{
        "type":"function",
        "function":{
            "name":tool_name,
            "description":tool_description,
            "parameters":schema,
        }
    }]);
    let mut messages = initial_messages.to_vec();
    let mut last_error = String::new();
    let local_cancel = CancellationToken::new();
    let cancel = cancel.unwrap_or(&local_cancel);
    for attempt in 0..3 {
        if let Some(handler) = on_event.as_deref_mut() {
            handler(StreamEvent::AttemptStarted {
                attempt: attempt + 1,
                maximum: 3,
            });
        }
        let fallback_tool_call_prefix = format!("studio-submission-{attempt}");
        let response = complete_stream(
            client,
            StreamCompletionRequest {
                connection,
                messages: &messages,
                tools: &tools,
                settings,
                runtime_max_output_tokens,
                cancel,
                audit_path,
                fallback_tool_call_prefix: &fallback_tool_call_prefix,
            },
            |event| {
                if let Some(handler) = on_event.as_deref_mut() {
                    handler(event);
                }
            },
        )
        .await;
        let response = match response {
            Ok(response) => response,
            Err(StudioError::Cancelled) => return Err(StudioError::Cancelled),
            Err(error) => {
                last_error = error.to_string();
                if let Some(handler) = on_event.as_deref_mut() {
                    handler(StreamEvent::SubmissionInvalid(format!(
                        "{label} attempt {} could not be accepted: {last_error}",
                        attempt + 1
                    )));
                }
                // The model's partial response is deliberately absent from history. A transport
                // failure is not semantic feedback and adding a correction prompt makes small
                // models repeat or explain an output they never see. Retry the same complete,
                // fresh-context request; only schema/parse failures below enter corrective history.
                continue;
            }
        };
        let message = response_message(&response)?;
        let tool_call = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .and_then(|calls| {
                calls.iter().find(|call| {
                    call.pointer("/function/name").and_then(Value::as_str) == Some(tool_name)
                })
            });
        if let Some(call) = tool_call {
            let arguments = call
                .pointer("/function/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let parsed = if let Some(text) = arguments.as_str() {
                serde_json::from_str::<T>(text)
            } else {
                serde_json::from_value::<T>(arguments)
            };
            match parsed {
                Ok(value) => return Ok(value),
                Err(error) => last_error = error.to_string(),
            }
        } else {
            last_error = format!("The Studio Director did not call {tool_name}");
        }
        if let Some(handler) = on_event.as_deref_mut() {
            handler(StreamEvent::SubmissionInvalid(format!(
                "{label} attempt {} could not be accepted: {last_error}",
                attempt + 1
            )));
        }
        let mut history_message = message;
        if let Some(object) = history_message.as_object_mut() {
            object.remove("reasoning");
            object.remove("reasoning_content");
        }
        messages.push(history_message);
        messages.push(json!({"role":"user","content":render(PromptId::StudioSubmissionCorrection, &[("label", label), ("error", &last_error), ("tool_name", tool_name)])}));
    }
    Err(StudioError::Planning(format!(
        "{} remained invalid after three attempts: {last_error}",
        label
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct TestReview {
        summary: String,
        issues: Vec<String>,
    }

    async fn read_http_request(stream: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 8_192];
        let header_end = loop {
            let read = stream.read(&mut buffer).await.unwrap();
            assert!(read > 0, "request ended before its headers");
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(position) = bytes.windows(4).position(|value| value == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or_default();
        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut buffer).await.unwrap();
            assert!(read > 0, "request ended before its body");
            bytes.extend_from_slice(&buffer[..read]);
        }
        String::from_utf8(bytes[header_end..header_end + content_length].to_vec()).unwrap()
    }

    async fn write_sse_response(stream: &mut tokio::net::TcpStream, body: &str) {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    }

    #[test]
    fn assistant_turn_removes_private_reasoning_but_preserves_tool_calls() {
        let turn = AssistantTurn::from_response(&json!({"choices":[{"message":{
            "role":"assistant",
            "content":"working",
            "reasoning":"private",
            "tool_calls":[{"id":"call-1","function":{"name":"movie_workspace","arguments":"{}"}}]
        }}]}))
        .unwrap();

        assert!(turn.has_tool_calls());
        assert_eq!(turn.tool_calls().len(), 1);
        assert!(turn.history_message().get("reasoning").is_none());
    }

    #[test]
    fn transcript_mutations_are_immediately_durable_and_lossless() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent-transcript.json");
        let mut transcript = AgentTranscript::begin(path.clone(), 0, "system", "start").unwrap();
        transcript
            .push(
                json!({"role":"assistant","content":"```json\n<think>hidden</think>"}),
                1,
            )
            .unwrap();

        let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(value["step"], 1);
        let serialized = value.to_string();
        assert!(serialized.contains("```"));
        assert!(serialized.contains("<think>hidden</think>"));
    }

    #[test]
    fn legacy_context_tool_output_is_compacted_only_in_the_model_request() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("transcript.json");
        let mut transcript =
            AgentTranscript::begin(path.clone(), 0, "system", "instruction").unwrap();
        let durable_context = format!("CONTEXT: {}", "x".repeat(20_000));
        transcript
            .push(
                json!({"role":"tool","tool_call_id":"call-1","content":durable_context}),
                1,
            )
            .unwrap();

        let request = transcript.request_messages("CURRENT AUTHORITATIVE MEMORY".into());
        assert!(request[2]["content"].as_str().unwrap().len() < 512);
        assert_eq!(
            request.last().unwrap()["content"],
            "CURRENT AUTHORITATIVE MEMORY"
        );

        let durable: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            durable["messages"][2]["content"].as_str().unwrap().len(),
            20_009
        );
    }

    #[test]
    fn transcript_resume_keeps_prior_turns_and_adds_a_visible_resume_instruction() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agent-transcript.json");
        let mut transcript = AgentTranscript::begin(path.clone(), 0, "system", "start").unwrap();
        transcript
            .push(json!({"role":"assistant","content":"saved candidate"}), 4)
            .unwrap();

        AgentTranscript::resume(path.clone(), "resume from checkpoint").unwrap();

        let value: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(value["step"], 4);
        assert_eq!(value["messages"].as_array().unwrap().len(), 4);
        assert_eq!(value["messages"][2]["content"], "saved candidate");
        assert_eq!(value["messages"][3]["content"], "resume from checkpoint");
    }

    #[test]
    fn sanitized_view_redacts_without_mutating_source_messages() {
        let messages = vec![json!({
            "role":"user",
            "content":"```json\n<think>private marker</think>"
        })];

        let redacted = sanitize_chat_messages(&messages);

        assert_eq!(
            messages[0]["content"],
            "```json\n<think>private marker</think>"
        );
        assert!(!redacted[0]["content"].as_str().unwrap().contains("```"));
        assert!(!redacted[0]["content"].as_str().unwrap().contains("<think>"));

        let request = movie_agent_request(
            "bonsai",
            &messages,
            &json!([]),
            &MovieSettings::default(),
            32_768,
        );
        assert_eq!(request["messages"], json!(messages));

        let transcript = json!({"step": 1, "messages": messages});
        let view = redacted_transcript_view(&transcript);
        assert!(transcript["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("<think>"));
        assert!(!view["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("<think>"));
    }

    #[test]
    fn streamed_tool_fragments_are_assembled_and_reported_incrementally() {
        let mut calls = Vec::new();
        let mut events = Vec::new();
        collect_tool_deltas(
            &mut calls,
            &[json!({
                "index": 0,
                "id": "call-",
                "function": {"name": "movie_", "arguments": "{\"action\":"}
            })],
            &mut |event| events.push(event),
        );
        collect_tool_deltas(
            &mut calls,
            &[json!({
                "index": 0,
                "id": "1",
                "function": {"name": "workspace", "arguments": "\"list\"}"}
            })],
            &mut |event| events.push(event),
        );

        assert_eq!(calls[0].id, "call-1");
        assert_eq!(calls[0].name, "movie_workspace");
        assert_eq!(calls[0].arguments, r#"{"action":"list"}"#);
        assert_eq!(
            events,
            vec![
                StreamEvent::ToolArgumentsStarted,
                StreamEvent::ToolArguments("{\"action\":".into()),
                StreamEvent::ToolArguments("\"list\"}".into()),
            ]
        );
    }

    #[test]
    fn output_limit_finish_reasons_discard_partial_tool_calls() {
        assert!(finish_reason_indicates_truncation(Some("length")));
        assert!(finish_reason_indicates_truncation(Some("max_tokens")));
        assert!(!finish_reason_indicates_truncation(Some("tool_calls")));
        assert!(!finish_reason_indicates_truncation(Some("stop")));
        assert!(!finish_reason_indicates_truncation(None));
    }

    #[test]
    fn completion_marker_state_is_derived_from_decoder_errors_without_guessing() {
        assert!(!stream_error_saw_completion_marker(
            "the model stream ended before its completion marker; received output remains retained for inspection"
        ));
        assert!(stream_error_saw_completion_marker(
            "the model stream sent data after its completion marker"
        ));
        assert!(stream_error_saw_completion_marker(
            "the model stream sent more than one completion marker"
        ));
    }

    #[tokio::test]
    async fn tool_submission_retries_a_dropped_stream_with_fresh_messages() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut request_bodies = Vec::new();
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                request_bodies.push(read_http_request(&mut stream).await);
                let arguments = if attempt == 0 {
                    "{\"summary\":\"partial"
                } else {
                    "{\"summary\":\"The candidate preserves both endpoints.\",\"issues\":[]}"
                };
                let event = json!({"choices":[{"delta":{"tool_calls":[{
                    "index":0,"id":"review-call","function":{
                        "name":"submit_review","arguments":arguments
                    }
                }]}}]});
                let body = if attempt == 0 {
                    format!("data: {event}\n\n")
                } else {
                    format!(
                        "data: {event}\n\ndata: {}\n\ndata: [DONE]\n\n",
                        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]})
                    )
                };
                write_sse_response(&mut stream, &body).await;
            }
            request_bodies
        });
        let connection = ModelConnection {
            endpoint: format!("http://{address}/v1"),
            api_key: None,
            model_id: "test-model".into(),
            model_label: "Test model".into(),
        };
        let messages = vec![json!({"role":"user","content":"Review this candidate."})];
        let schema = json!({"type":"json_schema","json_schema":{"schema":{
            "type":"object","additionalProperties":false,
            "properties":{"summary":{"type":"string"},"issues":{"type":"array","items":{"type":"string"}}},
            "required":["summary","issues"]
        }}});
        let result: TestReview = complete_tool_submission(
            &Client::builder().no_proxy().build().unwrap(),
            ToolSubmissionRequest {
                connection: &connection,
                initial_messages: &messages,
                tool_name: "submit_review",
                tool_description: "Submit the review.",
                response_format: schema,
                settings: &MovieSettings::default(),
                runtime_max_output_tokens: 4_096,
                label: "test review",
                audit_path: None,
                cancel: None,
                on_event: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            result,
            TestReview {
                summary: "The candidate preserves both endpoints.".into(),
                issues: vec![],
            }
        );
        let request_bodies = server.await.unwrap();
        let first: Value = serde_json::from_str(&request_bodies[0]).unwrap();
        let second: Value = serde_json::from_str(&request_bodies[1]).unwrap();
        assert_eq!(first["messages"], second["messages"]);
        assert_eq!(second["messages"], json!(messages));
    }
}
