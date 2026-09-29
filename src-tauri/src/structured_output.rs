//! Structured model replies across engines.
//!
//! llama.cpp turns an OpenAI `response_format` JSON schema into a decoding grammar, so the reply
//! cannot leave the schema. Strata ignores that field. For Strata the same schema travels as an
//! explicit instruction in the final user turn, and Kestrel's native parsers remain the only
//! authority: a reply that does not parse or validate is rejected exactly as before, never
//! repaired by guessing.

use crate::model::ModelEngine;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

/// Attach a `{"type":"json_schema", ...}` response format in the form the engine honours.
pub fn apply(request: &mut Value, format: Value, engine: ModelEngine) {
    if engine.enforces_json_schema() {
        request["response_format"] = format;
        return;
    }
    let schema = format
        .pointer("/json_schema/schema")
        .cloned()
        .unwrap_or(format);
    append_instruction(
        request,
        &schema_instruction(&serde_json::to_string(&schema).unwrap_or_default()),
    );
}

fn schema_instruction(schema: &str) -> String {
    format!(
        "Reply with exactly one JSON object that validates against the JSON Schema below. Output only that JSON object: no Markdown fence and no text before or after it.\n\nJSON Schema:\n{schema}"
    )
}

fn append_instruction(request: &mut Value, instruction: &str) {
    let Some(messages) = request.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    if let Some(last) = messages
        .last_mut()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
    {
        match last.get_mut("content") {
            Some(Value::String(text)) => {
                text.push_str("\n\n");
                text.push_str(instruction);
                return;
            }
            Some(Value::Array(parts)) => {
                parts.push(json!({"type": "text", "text": instruction}));
                return;
            }
            _ => {}
        }
    }
    messages.push(json!({"role": "user", "content": instruction}));
}

/// The outermost JSON object in a reply, tolerating a Markdown fence or a sentence around it.
pub fn json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

/// Parse a reply as `T`, first as a whole and then as its outermost JSON object.
pub fn parse<T: DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_str(text.trim())
        .ok()
        .or_else(|| json_object(text).and_then(|object| serde_json::from_str(object).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format() -> Value {
        json!({"type":"json_schema","json_schema":{"name":"plan","strict":true,"schema":{
            "type":"object","properties":{"lanes":{"type":"array"}},"required":["lanes"]
        }}})
    }

    #[test]
    fn llama_cpp_receives_the_grammar_unchanged() {
        let mut request = json!({"messages":[{"role":"user","content":"Plan."}]});
        apply(&mut request, format(), ModelEngine::LlamaCpp);
        assert_eq!(request["response_format"], format());
        assert_eq!(request["messages"][0]["content"], "Plan.");
    }

    #[test]
    fn strata_receives_the_schema_in_the_final_user_turn() {
        let mut request = json!({"messages":[
            {"role":"system","content":"You plan."},
            {"role":"user","content":"Plan."}
        ]});
        apply(&mut request, format(), ModelEngine::Strata);
        assert!(request.get("response_format").is_none());
        let content = request["messages"][1]["content"].as_str().unwrap();
        assert!(content.starts_with("Plan.\n\nReply with exactly one JSON object"));
        assert!(content.contains(r#""required":["lanes"]"#));
        assert_eq!(request["messages"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn strata_gets_a_new_user_turn_after_an_assistant_turn() {
        let mut request = json!({"messages":[
            {"role":"user","content":"Plan."},
            {"role":"assistant","content":"{}"}
        ]});
        apply(&mut request, format(), ModelEngine::Strata);
        let messages = request["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2]["role"], "user");
    }

    #[test]
    fn replies_parse_whole_or_from_their_outermost_object() {
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct Plan {
            lanes: Vec<String>,
        }
        let expected = Plan {
            lanes: vec!["a".into()],
        };
        assert_eq!(
            parse::<Plan>(r#"{"lanes":["a"]}"#),
            Some(Plan {
                lanes: vec!["a".into()]
            })
        );
        assert_eq!(
            parse::<Plan>("```json\n{\"lanes\":[\"a\"]}\n```"),
            Some(expected)
        );
        assert_eq!(parse::<Plan>("no object here"), None);
        assert_eq!(json_object("} {"), None);
    }
}
