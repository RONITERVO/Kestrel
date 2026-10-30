//! Prompt admission is independent of model-authored summaries. Strata's local token-count
//! endpoint renders the same template without inference; unsupported servers use a conservative
//! estimate. The existing RuntimeManager lease encloses this request and the ensuing generation.
use super::*;
use crate::models::{ModelEngine, ThinkingLevel};
use crate::runtime::ModelConnection;

pub(super) async fn fit(
    client: &reqwest::Client,
    connection: &ModelConnection,
    messages: &mut Vec<Value>,
    tools: &[Value],
    context_window: u32,
    requested_output: u32,
    thinking: ThinkingLevel,
) -> Result<u32, String> {
    let minimum_output = requested_output.min(8192);
    loop {
        let count = count_strata(client, connection, messages, tools, thinking).await?;
        let tokens = count.unwrap_or_else(|| {
            let bytes = messages
                .iter()
                .chain(tools)
                .map(|v| v.to_string().len())
                .sum::<usize>();
            // Deliberately more conservative than four characters/token. The runtime still
            // validates its real limit; this is not advertised as an exact tokenizer count.
            (bytes.div_ceil(2) as u32).saturating_add(1024)
        });
        let available = context_window.saturating_sub(tokens).saturating_sub(1024);
        if available >= minimum_output {
            return Ok(requested_output.min(available));
        }
        if !drop_oldest_group(messages) {
            return Err(format!("The saved task's essential context needs {tokens} tokens of this model's {context_window}-token window, leaving too little space for its next answer. Full history remains saved; shorten the working checkpoint or attached context before continuing."));
        }
    }
}

fn drop_oldest_group(messages: &mut Vec<Value>) -> bool {
    if messages.len() <= 2 {
        return false;
    }
    let end = messages
        .iter()
        .enumerate()
        .skip(3)
        .find(|(_, message)| message.get("role").and_then(Value::as_str) == Some("assistant"))
        .map_or(messages.len(), |(index, _)| index);
    messages.drain(2..end);
    true
}

async fn count_strata(
    client: &reqwest::Client,
    connection: &ModelConnection,
    messages: &[Value],
    tools: &[Value],
    thinking: ThinkingLevel,
) -> Result<Option<u32>, String> {
    if connection.engine != ModelEngine::Strata {
        return Ok(None);
    }
    let Some(body) = count_request(messages, tools, thinking) else {
        return Ok(None);
    };
    let response = authorized(
        client.post(format!("{}/messages/count_tokens", connection.endpoint)),
        connection,
    )
    .timeout(Duration::from_secs(30))
    .json(&body)
    .send()
    .await
    .map_err(|e| format!("Local prompt measurement failed before generation: {e}"))?;
    if matches!(response.status().as_u16(), 404 | 405) {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!(
            "Strata could not measure this task prompt ({}). No action was started.",
            response.status()
        ));
    }
    let response: Value = response.json().await.map_err(|e| e.to_string())?;
    response
        .get("input_tokens")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .map(Some)
        .ok_or_else(|| "Strata's token-count response has no valid input_tokens".into())
}

/// Conversion follows the installed Strata frontend's Anthropic adapter. This is solely a
/// local tokenizer request, not a new cloud provider or a change to the generation protocol.
fn count_request(messages: &[Value], tools: &[Value], thinking: ThinkingLevel) -> Option<Value> {
    let mut system = String::new();
    let mut converted = Vec::new();
    for message in messages {
        let role = message.get("role")?.as_str()?;
        if role == "system" {
            system.push_str(message.get("content")?.as_str()?);
            continue;
        }
        if role == "tool" {
            converted.push(json!({"role":"user","content":[{"type":"tool_result","tool_use_id":message.get("tool_call_id"),"content":message.get("content")}]}));
            continue;
        }
        let mut parts = Vec::new();
        match message.get("content") {
            Some(Value::String(text)) if !text.is_empty() => {
                parts.push(json!({"type":"text","text":text}))
            }
            Some(Value::Array(items)) => {
                for item in items {
                    match item.get("type")?.as_str()? {
                        "text" => parts.push(item.clone()),
                        "image_url" => {
                            let url = item.pointer("/image_url/url")?.as_str()?;
                            let (header, data) =
                                url.strip_prefix("data:")?.split_once(";base64,")?;
                            parts.push(json!({"type":"image","source":{"type":"base64","media_type":header,"data":data}}));
                        }
                        _ => return None,
                    }
                }
            }
            Some(Value::Null) | None => {}
            Some(Value::String(_)) => {}
            _ => return None,
        }
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            for call in calls {
                let input: Value =
                    serde_json::from_str(call.pointer("/function/arguments")?.as_str()?).ok()?;
                parts.push(json!({"type":"tool_use","id":call.get("id"),"name":call.pointer("/function/name"),"input":input}));
            }
        }
        converted.push(json!({"role":role,"content":parts}));
    }
    let tools=tools.iter().map(|tool| {
        let function=tool.get("function")?;
        Some(json!({"name":function.get("name")?,"description":function.get("description")?,"input_schema":function.get("parameters")?}))
    }).collect::<Option<Vec<_>>>()?;
    Some(json!({"system":system,"messages":converted,"tools":tools,
        "thinking":if thinking.is_off() {json!({"type":"disabled"})} else {json!({"type":"enabled"})},
        "output_config":{"effort":thinking.as_template_effort()}}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn count_conversion_keeps_tool_pairs_schema_images_and_thinking_policy() {
        let messages = vec![
            json!({"role":"system","content":"policy"}),
            json!({"role":"user","content":[{"type":"text","text":"inspect"},{"type":"image_url","image_url":{"url":"data:image/png;base64,aGVsbG8="}}]}),
            json!({"role":"assistant","content":null,"tool_calls":[{"id":"a","function":{"name":"read_file","arguments":"{\"path\":\"D:/a\"}"}}]}),
            json!({"role":"tool","tool_call_id":"a","content":"result"}),
        ];
        let tools = vec![schema("read_file", "read", json!({"type":"object"}))];
        let body = count_request(&messages, &tools, ThinkingLevel::High).unwrap();
        assert_eq!(body["system"], "policy");
        assert_eq!(body["messages"][1]["content"][0]["input"]["path"], "D:/a");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "a");
        assert_eq!(
            body["messages"][0]["content"][1]["source"]["media_type"],
            "image/png"
        );
        assert_eq!(body["output_config"]["effort"], "high");
    }
    #[test]
    fn pressure_removes_a_whole_tool_batch_and_never_the_recovery_packet() {
        let mut messages = vec![
            json!({"role":"system"}),
            json!({"role":"user"}),
            json!({"role":"assistant"}),
            json!({"role":"tool"}),
            json!({"role":"tool"}),
            json!({"role":"assistant"}),
            json!({"role":"tool"}),
        ];
        assert!(drop_oldest_group(&mut messages));
        assert_eq!(messages.len(), 4);
        assert!(drop_oldest_group(&mut messages));
        assert_eq!(messages.len(), 2);
        assert!(!drop_oldest_group(&mut messages));
        assert_eq!(messages[1]["role"], "user");
    }

    #[tokio::test]
    #[ignore = "requires installed Strata; measures and generates four high-thinking replies through repeated forced context compaction"]
    async fn live_strata_admits_recovered_prompts_through_repeated_compaction() {
        let configured = std::env::var("KESTREL_LIVE_STRATA_ROOT").unwrap_or_default();
        let model = crate::strata::discover(&crate::strata::known_installs(&[configured.as_str()]))
            .into_iter()
            .next()
            .expect("no installed Strata model");
        let runtime = Arc::new(RuntimeManager::new());
        let settings = ControlSettings::default();
        let lease = runtime
            .lease_model(&model.id, std::slice::from_ref(&model), &settings, None)
            .await
            .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let tools = vec![schema(
            "read_file",
            "Read a bounded page",
            json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),
        )];
        let mut messages = vec![
            json!({"role":"system","content":"Follow the recovery checkpoint. Reply with its marker and current phase, without commentary or tools."}),
            json!({"role":"user","content":""}),
        ];
        for phase in 1..=4 {
            // Synthetic large historical results deliberately exceed the admission window. The
            // generation is real, uses high thinking, and keeps the production tool-choice mode.
            messages[1]["content"] = json!(format!("Native recovery checkpoint: the producer's marker is amber-owl-47; current phase is {phase}. Reply exactly amber-owl-47 phase {phase}."));
            for i in 0..12 {
                let id = format!("call-{phase}-{i}");
                messages.push(json!({"role":"assistant","content":null,"tool_calls":[{"id":id,"type":"function","function":{"name":"read_file","arguments":"{}"}}]}));
                messages.push(json!({"role":"tool","tool_call_id":id,"content":"Verified local result. ".repeat(700)}));
            }
            let before = count_strata(
                &client,
                &lease.connection,
                &messages,
                &tools,
                ThinkingLevel::High,
            )
            .await
            .unwrap()
            .expect("installed Strata must expose token measurement");
            let output = fit(
                &client,
                &lease.connection,
                &mut messages,
                &tools,
                8192,
                2048,
                ThinkingLevel::High,
            )
            .await
            .unwrap();
            let after = count_strata(
                &client,
                &lease.connection,
                &messages,
                &tools,
                ThinkingLevel::High,
            )
            .await
            .unwrap()
            .unwrap();
            assert!(after < before);
            assert!(after + output + 1024 <= 8192);
            let mut body = json!({"model":lease.connection.model_id,"messages":messages,"tools":tools,"tool_choice":"auto","max_tokens":output,"stream":false,"temperature":0,"reasoning_effort":"high","chat_template_kwargs":{"enable_thinking":true,"reasoning_effort":"high"}});
            apply_thinking_budget(
                &mut body,
                lease.connection.engine,
                ThinkingLevel::High,
                output,
            );
            let response = authorized(
                client.post(format!("{}/chat/completions", lease.connection.endpoint)),
                &lease.connection,
            )
            .json(&body)
            .send()
            .await
            .unwrap();
            let status = response.status();
            let result: Value = response.json().await.unwrap();
            assert!(status.is_success(), "{result}");
            let measured = result
                .pointer("/usage/prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap();
            assert!(
                measured + u64::from(output) < 8192,
                "actual generation exceeded its admission: {result}"
            );
            assert!(!completion_hit_limit(&result), "{result}");
            let reply = result.pointer("/choices/0/message").unwrap();
            assert!(
                reply
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.contains(&format!("amber-owl-47 phase {phase}"))),
                "recovery direction lost: {result}"
            );
            messages.push(memory::conversation_reply(reply));
            println!("STRATA_CONTEXT_ADMISSION phase={phase} before={before} after={after} actual={measured} output_cap={output}");
        }
        drop(lease);
        runtime.stop_managed().await.unwrap();
    }
}
