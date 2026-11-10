//! Assemble a streamed proposal before the runner is allowed to inspect or execute its tools.
use futures_util::StreamExt;
use serde_json::{json, Value};

const MAX_RESPONSE: usize = 8 * 1024 * 1024;

pub struct Presentation {
    app: Option<tauri::AppHandle>,
    event: kestrel_app_core::TaskResponseDelta,
    finished: bool,
}

impl Presentation {
    pub fn new(app: Option<tauri::AppHandle>, run_id: &str, step: u32) -> Self {
        let mut value = Self {
            app,
            event: kestrel_app_core::TaskResponseDelta {
                run_id: run_id.into(),
                response_id: uuid::Uuid::new_v4().to_string(),
                step,
                kind: String::new(),
                text: String::new(),
            },
            finished: false,
        };
        value.publish("started", "");
        value
    }

    pub fn publish(&mut self, kind: &str, text: &str) {
        self.event.kind = kind.into();
        self.event.text = text.into();
        if let Some(app) = &self.app {
            let _ = crate::ipc_events::emit::<kestrel_app_core::events::TaskResponseDelta>(
                app,
                &self.event,
            );
        }
    }

    pub fn finish(&mut self) {
        self.publish("done", "");
        self.finished = true;
    }
}

impl Drop for Presentation {
    fn drop(&mut self) {
        if !self.finished {
            self.publish("cancelled", "");
        }
    }
}

#[derive(Default)]
struct Proposal {
    content: String,
    reasoning: String,
    calls: Vec<Value>,
    finish: Option<String>,
    bytes: usize,
    done: bool,
}

impl Proposal {
    fn accept(&mut self, data: &str, visible: &mut impl FnMut(&str)) -> Result<(), String> {
        if data == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        if self.done {
            return Err("The task stream sent data after completion.".into());
        }
        self.bytes += data.len();
        if self.bytes > MAX_RESPONSE {
            return Err(
                "The task response exceeded the 8 MiB safety limit; no proposed tool was executed."
                    .into(),
            );
        }
        let value: Value =
            serde_json::from_str(data).map_err(|e| format!("Invalid task stream: {e}"))?;
        if value.get("error").is_some() {
            return Err(format!("The local task stream failed: {}", value["error"]));
        }
        let Some(choice) = value.pointer("/choices/0") else {
            return Ok(());
        };
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.finish = Some(reason.into());
        }
        let delta = &choice["delta"];
        if let Some(text) = delta["content"].as_str() {
            self.content.push_str(text);
            visible(text);
        }
        if let Some(text) = delta["reasoning_content"]
            .as_str()
            .or_else(|| delta["reasoning"].as_str())
        {
            self.reasoning.push_str(text);
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for call in calls {
                let index =
                    call["index"].as_u64().filter(|index| *index < 128).ok_or(
                        "Invalid tool index in task stream; no proposed tool was executed.",
                    )? as usize;
                while self.calls.len() <= index {
                    self.calls.push(
                        json!({"id":"","type":"function","function":{"name":"","arguments":""}}),
                    );
                }
                for (path, fragment) in [
                    ("/id", &call["id"]),
                    ("/function/name", &call["function"]["name"]),
                    ("/function/arguments", &call["function"]["arguments"]),
                ] {
                    if let Some(fragment) = fragment.as_str() {
                        let target = self.calls[index]
                            .pointer_mut(path)
                            .expect("fixed tool shape");
                        *target = Value::String(format!(
                            "{}{fragment}",
                            target.as_str().unwrap_or_default()
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<Value, String> {
        if !self.done || self.finish.is_none() {
            return Err("The task stream ended without confirmed completion. No proposed tool was executed; continue to retry the decision.".into());
        }
        if !matches!(
            self.finish.as_deref(),
            Some("stop" | "tool_calls" | "length" | "content_filter" | "error")
        ) {
            return Err(
                "The task stream returned an unknown finish reason; no proposed tool was executed."
                    .into(),
            );
        }
        Ok(json!({"choices":[{"finish_reason":self.finish,"message":{
            "role":"assistant","content":self.content,"reasoning_content":self.reasoning,"tool_calls":self.calls
        }}]}))
    }
}

pub async fn read(
    response: reqwest::Response,
    mut visible: impl FnMut(&str),
) -> Result<Value, String> {
    // Older OpenAI-compatible servers may ignore stream=true. Retain their bounded JSON path.
    if !response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"))
    {
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            if bytes.len() + chunk.len() > MAX_RESPONSE {
                return Err("Task response exceeded 8 MiB.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        return serde_json::from_slice(&bytes).map_err(|e| format!("Invalid task reply: {e}"));
    }
    let mut stream = response.bytes_stream();
    let mut pending = Vec::new();
    let mut proposal = Proposal::default();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Task stream interrupted: {e}"))?;
        if pending.len() + chunk.len() > MAX_RESPONSE {
            return Err("Task stream frame exceeded 8 MiB.".into());
        }
        pending.extend_from_slice(&chunk);
        while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
            let line =
                String::from_utf8(pending.drain(..=end).collect()).map_err(|e| e.to_string())?;
            if let Some(data) = line.trim_end().strip_prefix("data:") {
                proposal.accept(data.trim_start(), &mut visible)?;
            }
        }
        if proposal.done {
            break;
        }
    }
    proposal.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_only_visible_words_and_assembles_tools_without_executing_them() {
        let mut p = Proposal::default();
        let mut said = String::new();
        let mut visible = |text: &str| said.push_str(text);
        p.accept(r#"{"choices":[{"delta":{"reasoning_content":"private","content":"I will check.","tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}"#, &mut visible).unwrap();
        p.accept(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"C:/work/a\"}"}}]},"finish_reason":"tool_calls"}]}"#, &mut visible).unwrap();
        p.accept("[DONE]", &mut visible).unwrap();
        let value = p.finish().unwrap();
        assert_eq!(said, "I will check.");
        assert_eq!(
            value["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
            r#"{"path":"C:/work/a"}"#
        );
    }

    #[test]
    fn interrupted_or_unbounded_tool_proposals_never_become_complete() {
        let mut p = Proposal::default();
        p.accept(
            r#"{"choices":[{"delta":{"content":"Working"}}]}"#,
            &mut |_| {},
        )
        .unwrap();
        assert!(p.finish().is_err());
        let mut p = Proposal::default();
        assert!(p
            .accept(
                r#"{"choices":[{"delta":{"tool_calls":[{"index":999999}]}}]}"#,
                &mut |_| {}
            )
            .is_err());
    }
}
