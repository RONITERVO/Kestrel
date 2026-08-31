//! Shared OpenAI-compatible server-sent-event decoding for Studio model features.
//!
//! Network ownership and producer-facing events remain with each feature. This module owns the
//! wire-level contract so story chat, scene chat, and prompt drafting cannot
//! silently diverge in how they handle fragmented UTF-8, completion markers, or malformed data.

use serde_json::Value;

const MAX_SSE_EVENT_BYTES: usize = 1024 * 1024;

/// Returns the model's explicit reasoning channel without trying to infer private thought from
/// ordinary answer text. OpenAI-compatible local servers use both field names in practice.
pub(super) fn reasoning_delta(value: &Value) -> Option<&str> {
    value
        .pointer("/choices/0/delta/reasoning_content")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .or_else(|| {
            value
                .pointer("/choices/0/delta/reasoning")
                .and_then(Value::as_str)
                .filter(|token| !token.is_empty())
        })
}

#[derive(Debug, PartialEq)]
pub(super) enum OpenAiStreamEvent {
    Message(Value),
    Done,
}

#[derive(Debug, Default)]
pub(super) struct OpenAiSseDecoder {
    buffer: Vec<u8>,
    completed: bool,
}

impl OpenAiSseDecoder {
    pub(super) fn push(&mut self, chunk: &[u8]) -> Result<Vec<OpenAiStreamEvent>, String> {
        if self.completed && !chunk.iter().all(u8::is_ascii_whitespace) {
            return Err("the model stream sent data after its completion marker".into());
        }
        let mut unterminated_line_bytes = self.buffer.len();
        for byte in chunk {
            if *byte == b'\n' {
                unterminated_line_bytes = 0;
            } else {
                unterminated_line_bytes = unterminated_line_bytes.saturating_add(1);
                if unterminated_line_bytes > MAX_SSE_EVENT_BYTES {
                    return Err(format!(
                        "the model stream event exceeds the {MAX_SSE_EVENT_BYTES}-byte limit"
                    ));
                }
            }
        }
        self.buffer.extend_from_slice(chunk);
        self.decode_complete_lines()
    }

    pub(super) fn finish(&mut self) -> Result<Vec<OpenAiStreamEvent>, String> {
        let mut events = self.decode_complete_lines()?;
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            if !line.iter().all(u8::is_ascii_whitespace) {
                if let Some(event) = self.decode_line(&line)? {
                    events.push(event);
                }
            }
        }
        if !self.completed {
            return Err(
                "the model stream ended before its completion marker; received output remains retained for inspection"
                    .into(),
            );
        }
        Ok(events)
    }

    fn decode_complete_lines(&mut self) -> Result<Vec<OpenAiStreamEvent>, String> {
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=end).collect::<Vec<_>>();
            if let Some(event) = self.decode_line(&line)? {
                events.push(event);
            }
        }
        Ok(events)
    }

    fn decode_line(&mut self, line: &[u8]) -> Result<Option<OpenAiStreamEvent>, String> {
        let line = std::str::from_utf8(line)
            .map_err(|_| "the model stream contained invalid UTF-8".to_string())?
            .trim();
        if line.is_empty() || line.starts_with(':') {
            return Ok(None);
        }
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(None);
        };
        let data = data.trim();
        if data == "[DONE]" {
            if self.completed {
                return Err("the model stream sent more than one completion marker".into());
            }
            self.completed = true;
            return Ok(Some(OpenAiStreamEvent::Done));
        }
        if self.completed {
            return Err("the model stream sent an event after its completion marker".into());
        }
        let value = serde_json::from_str(data)
            .map_err(|error| format!("the model stream contained malformed JSON: {error}"))?;
        Ok(Some(OpenAiStreamEvent::Message(value)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decoder_preserves_fragmented_utf8_and_multiple_events() {
        let wire = "data: {\"choices\":[{\"delta\":{\"content\":\"Hyvää\"}}]}\r\n\r\ndata: {\"usage\":{\"tokens\":2}}\n\ndata: [DONE]\n";
        let bytes = wire.as_bytes();
        let split = wire.find('ä').unwrap() + 1;
        let mut decoder = OpenAiSseDecoder::default();
        let mut events = decoder.push(&bytes[..split]).unwrap();
        events.extend(decoder.push(&bytes[split..split + 2]).unwrap());
        events.extend(decoder.push(&bytes[split + 2..]).unwrap());
        events.extend(decoder.finish().unwrap());

        assert_eq!(events.len(), 3);
        assert_eq!(
            events[0],
            OpenAiStreamEvent::Message(json!({"choices":[{"delta":{"content":"Hyvää"}}]}))
        );
        assert_eq!(
            events[1],
            OpenAiStreamEvent::Message(json!({"usage":{"tokens":2}}))
        );
        assert_eq!(events[2], OpenAiStreamEvent::Done);
    }

    #[test]
    fn decoder_accepts_comments_and_a_final_marker_without_newline() {
        let mut decoder = OpenAiSseDecoder::default();
        assert!(decoder.push(b": keepalive\n\n").unwrap().is_empty());
        assert!(decoder.push(b"data: [DONE]").unwrap().is_empty());
        assert_eq!(decoder.finish().unwrap(), vec![OpenAiStreamEvent::Done]);
    }

    #[test]
    fn decoder_rejects_malformed_json_instead_of_silently_losing_tokens() {
        let mut decoder = OpenAiSseDecoder::default();
        let error = decoder.push(b"data: {not-json}\n").unwrap_err();
        assert!(error.contains("malformed JSON"));
    }

    #[test]
    fn decoder_rejects_streams_without_a_completion_marker() {
        let mut decoder = OpenAiSseDecoder::default();
        decoder
            .push(b"data: {\"choices\":[{\"delta\":{}}]}\n")
            .unwrap();
        let error = decoder.finish().unwrap_err();
        assert!(error.contains("before its completion marker"));
    }

    #[test]
    fn decoder_rejects_events_after_completion() {
        let mut decoder = OpenAiSseDecoder::default();
        decoder.push(b"data: [DONE]\n").unwrap();
        let error = decoder.push(b"data: {\"choices\":[]}\n").unwrap_err();
        assert!(error.contains("after its completion marker"));
    }

    #[test]
    fn decoder_rejects_an_oversized_unterminated_event_before_buffering_it() {
        let mut decoder = OpenAiSseDecoder::default();
        let oversized = vec![b'x'; MAX_SSE_EVENT_BYTES + 1];
        let error = decoder.push(&oversized).unwrap_err();
        assert!(error.contains("event exceeds"));
        assert!(decoder.buffer.is_empty());
    }

    #[test]
    fn reasoning_delta_accepts_both_explicit_local_server_fields_only() {
        assert_eq!(
            reasoning_delta(&json!({"choices":[{"delta":{"reasoning_content":"first"}}]})),
            Some("first")
        );
        assert_eq!(
            reasoning_delta(&json!({"choices":[{"delta":{"reasoning":"second"}}]})),
            Some("second")
        );
        assert_eq!(
            reasoning_delta(
                &json!({"choices":[{"delta":{"reasoning_content":"","reasoning":"fallback"}}]})
            ),
            Some("fallback")
        );
        assert_eq!(
            reasoning_delta(
                &json!({"choices":[{"delta":{"content":"<think>not a channel</think>"}}]})
            ),
            None
        );
    }
}
