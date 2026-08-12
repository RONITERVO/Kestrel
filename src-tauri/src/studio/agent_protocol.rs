//! Bonsai planning protocol and durable conversation primitives.
//!
//! Model request construction, SSE assembly, typed tool-only retries, explicit redacted views,
//! and lossless transcript persistence live here so every Studio model call follows the same
//! bounded contract.

use super::{
    model_stream::{OpenAiSseDecoder, OpenAiStreamEvent},
    write_json_atomic, MovieSettings, StudioError, MOVIE_THINKING_BUDGET,
};
use crate::runtime::{authorized, ModelConnection};
use chrono::Utc;
use futures_util::StreamExt;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

/// Durable conversation state for one Bonsai context session.
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

    pub(super) fn request_messages(&self, authoritative_memory: String) -> Vec<Value> {
        let mut messages = self.messages.clone();
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
    json!({
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
        "thinking_budget_tokens": MOVIE_THINKING_BUDGET,
    })
}

pub(super) async fn complete(
    client: &Client,
    connection: &ModelConnection,
    messages: &[Value],
    tools: &Value,
    settings: &MovieSettings,
    runtime_max_output_tokens: u32,
) -> Result<Value, StudioError> {
    let body = movie_agent_request(
        &connection.model_id,
        messages,
        tools,
        settings,
        runtime_max_output_tokens,
    );
    let response = authorized(
        client.post(format!("{}/chat/completions", connection.endpoint)),
        connection,
    )
    .json(&body)
    .send()
    .await?;
    let status = response.status();
    let text = response.text().await?;
    if !status.is_success() {
        return Err(StudioError::Planning(format!(
            "movie agent HTTP {status}: {}",
            super::truncate(&text, 500)
        )));
    }
    serde_json::from_str(&text).map_err(StudioError::from)
}

pub(super) struct StreamCompletionRequest<'a> {
    pub connection: &'a ModelConnection,
    pub messages: &'a [Value],
    pub tools: &'a Value,
    pub settings: &'a MovieSettings,
    pub runtime_max_output_tokens: u32,
    pub cancel: &'a CancellationToken,
    pub audit_path: &'a Path,
    pub fallback_tool_call_prefix: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum StreamEvent {
    Content(String),
    ReasoningStarted,
    ToolArgumentsStarted,
    ToolArguments(String),
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
    write_json_atomic(request.audit_path, &body)?;
    let response = authorized(
        client.post(format!("{}/chat/completions", request.connection.endpoint)),
        request.connection,
    )
    .json(&body)
    .send()
    .await?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(StudioError::Planning(format!(
            "movie agent HTTP {status}: {}",
            super::truncate(&text, 500)
        )));
    }

    let mut stream = response.bytes_stream();
    let mut decoder = OpenAiSseDecoder::default();
    let mut content = String::new();
    let mut tool_calls = Vec::<StreamedToolCall>::new();
    let mut reasoning_announced = false;
    let mut accept_events = |events: Vec<OpenAiStreamEvent>| {
        for event in events {
            let OpenAiStreamEvent::Message(value) = event else {
                continue;
            };
            if let Some(token) = value
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
            {
                content.push_str(token);
                on_event(StreamEvent::Content(token.to_string()));
            }
            if !reasoning_announced
                && value
                    .pointer("/choices/0/delta/reasoning_content")
                    .or_else(|| value.pointer("/choices/0/delta/reasoning"))
                    .and_then(Value::as_str)
                    .is_some()
            {
                reasoning_announced = true;
                on_event(StreamEvent::ReasoningStarted);
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
            _ = request.cancel.cancelled() => return Err(StudioError::Cancelled),
        };
        let Some(chunk) = next else { break };
        let events = decoder
            .push(&chunk?)
            .map_err(|error| StudioError::Planning(format!("movie agent stream error: {error}")))?;
        accept_events(events);
    }
    let final_events = decoder
        .finish()
        .map_err(|error| StudioError::Planning(format!("movie agent stream error: {error}")))?;
    accept_events(final_events);
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
}

/// Completes a typed, tool-only model exchange with bounded corrective retries.
///
/// The exact request is optionally persisted before every attempt. Invalid model output is fed
/// back into the same conversation, while private reasoning fields never enter durable history.
pub(super) async fn complete_tool_submission<T: DeserializeOwned>(
    client: &Client,
    request: ToolSubmissionRequest<'_>,
) -> Result<T, StudioError> {
    let schema = request
        .response_format
        .pointer("/json_schema/schema")
        .cloned()
        .unwrap_or(request.response_format);
    let tools = json!([{
        "type":"function",
        "function":{
            "name":request.tool_name,
            "description":request.tool_description,
            "parameters":schema,
        }
    }]);
    let mut messages = request.initial_messages.to_vec();
    let mut last_error = String::new();
    for _ in 0..3 {
        if let Some(path) = request.audit_path {
            write_json_atomic(
                path,
                &movie_agent_request(
                    &request.connection.model_id,
                    &messages,
                    &tools,
                    request.settings,
                    request.runtime_max_output_tokens,
                ),
            )?;
        }
        let response = complete(
            client,
            request.connection,
            &messages,
            &tools,
            request.settings,
            request.runtime_max_output_tokens,
        )
        .await?;
        let message = response_message(&response)?;
        let tool_call = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .and_then(|calls| {
                calls.iter().find(|call| {
                    call.pointer("/function/name").and_then(Value::as_str)
                        == Some(request.tool_name)
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
            last_error = format!("Bonsai did not call {}", request.tool_name);
        }
        let mut history_message = message;
        if let Some(object) = history_message.as_object_mut() {
            object.remove("reasoning");
            object.remove("reasoning_content");
        }
        messages.push(history_message);
        messages.push(json!({"role":"user","content":format!(
            "The {} submission failed validation: {last_error}. Correct it and call {}; do not answer in prose.",
            request.label, request.tool_name
        )}));
    }
    Err(StudioError::Planning(format!(
        "{} remained invalid after three attempts: {last_error}",
        request.label
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
