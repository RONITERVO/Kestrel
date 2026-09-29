//! Structured model replies across engines.
//!
//! llama.cpp turns an OpenAI `response_format` JSON schema into a decoding grammar, so the reply
//! cannot leave the schema. Strata ignores that field. For Strata the same schema travels as an
//! explicit instruction in the final user turn, and Kestrel's native parsers remain the only
//! authority: a reply that does not parse or validate is rejected exactly as before, never
//! repaired by guessing.

use crate::model::ModelEngine;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, value::RawValue, Value};

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

/// A JSON schema whose property order is part of the requested format.
///
/// Kestrel's `serde_json` keeps object keys sorted, while llama.cpp builds its grammar in the
/// order the properties arrive, so a `Value` schema would make the model write keys
/// alphabetically. An ordered schema therefore stays literal JSON text and reaches the engine
/// exactly as written.
pub struct OrderedSchema {
    pub name: &'static str,
    pub text: &'static str,
}

/// A request ready to send, plus the same request as a `Value` for receipts. The receipt's
/// `response_format` is the parsed schema, so only its key order differs from the wire bytes.
pub struct PreparedRequest {
    pub bytes: Vec<u8>,
    pub receipt: Value,
}

/// Serialize a request that must follow an ordered schema, in the form the engine honours.
pub fn prepare_ordered(
    mut request: Value,
    schema: &OrderedSchema,
    engine: ModelEngine,
) -> Result<PreparedRequest, serde_json::Error> {
    if let Some(fields) = request.as_object_mut() {
        fields.remove("response_format");
    }
    if !engine.enforces_json_schema() {
        append_instruction(&mut request, &schema_instruction(schema.text));
        return Ok(PreparedRequest {
            bytes: serde_json::to_vec(&request)?,
            receipt: request,
        });
    }
    #[derive(Serialize)]
    struct WithFormat<'a> {
        #[serde(flatten)]
        request: &'a Value,
        response_format: &'a RawValue,
    }
    let format = RawValue::from_string(format!(
        r#"{{"type":"json_schema","json_schema":{{"name":{},"strict":true,"schema":{}}}}}"#,
        serde_json::to_string(schema.name)?,
        schema.text
    ))?;
    let bytes = serde_json::to_vec(&WithFormat {
        request: &request,
        response_format: &format,
    })?;
    request["response_format"] = serde_json::from_str(format.get())?;
    Ok(PreparedRequest {
        bytes,
        receipt: request,
    })
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

    const ORDERED: OrderedSchema = OrderedSchema {
        name: "ordered",
        text: r#"{"type":"object","properties":{"zeta":{"type":"string"},"alpha":{"type":"string"}},"required":["zeta","alpha"]}"#,
    };

    fn position(haystack: &str, needle: &str) -> usize {
        haystack.find(needle).unwrap_or(usize::MAX)
    }

    #[test]
    fn llama_cpp_receives_an_ordered_schema_exactly_as_written() {
        let request = json!({"model":"m","messages":[{"role":"user","content":"Design."}],"response_format":{"stale":true}});
        let prepared = prepare_ordered(request, &ORDERED, ModelEngine::LlamaCpp).unwrap();
        let wire = String::from_utf8(prepared.bytes.clone()).unwrap();
        assert!(
            position(&wire, r#""zeta""#) < position(&wire, r#""alpha""#),
            "{wire}"
        );
        assert!(!wire.contains("stale"));
        let parsed: Value = serde_json::from_slice(&prepared.bytes).unwrap();
        assert_eq!(parsed["model"], "m");
        assert_eq!(parsed["messages"][0]["content"], "Design.");
        assert_eq!(parsed["response_format"]["json_schema"]["name"], "ordered");
        assert_eq!(parsed["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            prepared.receipt["response_format"],
            parsed["response_format"]
        );
    }

    #[test]
    fn strata_reads_an_ordered_schema_in_the_prompt_as_written() {
        let request = json!({"model":"m","messages":[{"role":"user","content":"Design."}]});
        let prepared = prepare_ordered(request, &ORDERED, ModelEngine::Strata).unwrap();
        let parsed: Value = serde_json::from_slice(&prepared.bytes).unwrap();
        assert!(parsed.get("response_format").is_none());
        let content = parsed["messages"][0]["content"].as_str().unwrap();
        assert!(content.starts_with("Design.\n\nReply with exactly one JSON object"));
        assert!(content.ends_with(ORDERED.text));
        assert_eq!(prepared.receipt, parsed);
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
