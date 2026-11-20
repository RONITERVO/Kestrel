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

/// Strip a single leading and trailing newline (`\r\n`, `\n`, or `\r`) from an XML parameter body.
fn trim_xml_param_edges(raw: &str) -> &str {
    let trimmed_start = raw
        .strip_prefix("\r\n")
        .or_else(|| raw.strip_prefix('\n'))
        .or_else(|| raw.strip_prefix('\r'))
        .unwrap_or(raw);
    trimmed_start
        .strip_suffix("\r\n")
        .or_else(|| trimmed_start.strip_suffix('\n'))
        .or_else(|| trimmed_start.strip_suffix('\r'))
        .unwrap_or(trimmed_start)
}

/// Find byte ranges `[start, end)` covered by paired triple-backtick markdown code fences.
fn code_fence_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut open_start: Option<usize> = None;
    let mut offset = 0usize;
    for segment in text.split_inclusive('\n') {
        let trimmed = segment.trim_start();
        if trimmed.starts_with("```") {
            if let Some(start) = open_start.take() {
                ranges.push((start, offset + segment.len()));
            } else {
                open_start = Some(offset);
            }
        }
        offset += segment.len();
    }
    ranges
}

fn inside_ranges(ranges: &[(usize, usize)], pos: usize) -> bool {
    ranges
        .iter()
        .any(|(start, end)| pos >= *start && pos < *end)
}

fn find_outside_fences(text: &str, needle: &str, from: usize, fences: &[(usize, usize)]) -> Option<usize> {
    let mut search_from = from;
    while let Some(rel) = text.get(search_from..)?.find(needle) {
        let abs = search_from + rel;
        if !inside_ranges(fences, abs) {
            return Some(abs);
        }
        search_from = abs + needle.len();
    }
    None
}

fn declared_tool_names(tools: &[Value]) -> Vec<&str> {
    tools
        .iter()
        .filter_map(|tool| {
            tool.pointer("/function/name")
                .and_then(Value::as_str)
                .or_else(|| tool.get("name").and_then(Value::as_str))
        })
        .collect()
}

fn property_schema_type<'a>(tools: &'a [Value], tool_name: &str, key: &str) -> Option<&'a str> {
    tools.iter().find_map(|tool| {
        let name = tool
            .pointer("/function/name")
            .and_then(Value::as_str)
            .or_else(|| tool.get("name").and_then(Value::as_str))?;
        if name != tool_name {
            return None;
        }
        let props = tool
            .pointer("/function/parameters/properties")
            .or_else(|| tool.pointer("/parameters/properties"))?;
        props.pointer(&format!("/{key}/type")).and_then(Value::as_str)
    })
}

fn is_valid_tool_ident(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn extract_tool_header<'a>(
    block: &'a str,
    known_tools: &[&str],
) -> Option<(String, &'a str)> {
    let trimmed = block.trim_start();
    if let Some(after_fn) = trimmed.strip_prefix("<function=") {
        let close = after_fn.find(['>', '\r', '\n', '<'])?;
        let name = after_fn[..close].trim();
        if !is_valid_tool_ident(name)
            || (!known_tools.is_empty() && !known_tools.contains(&name))
        {
            return None;
        }
        let mut rest = &after_fn[close..];
        rest = rest.strip_prefix('>').unwrap_or(rest);
        if let Some(stripped) = rest.trim_start().strip_prefix("</parameter>") {
            rest = stripped;
        }
        return Some((name.to_string(), rest));
    }

    let first_param = trimmed.find("<parameter=").unwrap_or(trimmed.len());
    let header = &trimmed[..first_param];
    if let Some(eq_idx) = header.rfind('=') {
        let after_eq = &header[eq_idx + 1..];
        let name_end = after_eq
            .find(|ch: char| ch.is_whitespace() || matches!(ch, '>' | '<'))
            .unwrap_or(after_eq.len());
        let name = after_eq[..name_end].trim();
        if is_valid_tool_ident(name)
            && (known_tools.is_empty() || known_tools.contains(&name))
        {
            return Some((name.to_string(), &trimmed[first_param..]));
        }
    }

    if let Some(after_param) = trimmed.strip_prefix("<parameter=") {
        if let Some(gt) = after_param.find('>') {
            let candidate = after_param[..gt].trim();
            if is_valid_tool_ident(candidate)
                && !known_tools.is_empty()
                && known_tools.contains(&candidate)
            {
                let after_gt = &after_param[gt + 1..];
                if let Some(close_idx) = after_gt.find("</parameter>") {
                    if after_gt[..close_idx].trim().is_empty() {
                        let rest = &after_gt[close_idx + "</parameter>".len()..];
                        return Some((candidate.to_string(), rest));
                    }
                }
            }
        }
    }

    None
}

fn find_param_close(rest: &str) -> Option<usize> {
    let close_tag = "</parameter>";
    let mut from = 0usize;
    while let Some(rel) = rest.get(from..)?.find(close_tag) {
        let abs = from + rel;
        let after = rest[abs + close_tag.len()..].trim_start();
        if after.is_empty()
            || after.starts_with("<parameter=")
            || after.starts_with("</function>")
            || after.starts_with("</tool_call>")
            || after.starts_with("<|im_")
        {
            return Some(abs);
        }
        from = abs + close_tag.len();
    }
    None
}

fn decode_param_value(raw: &str, key: &str, schema_type: Option<&str>) -> Value {
    if key == "edits" {
        if let Ok(parsed @ Value::Array(_)) = serde_json::from_str::<Value>(raw.trim()) {
            return parsed;
        }
        return Value::String(raw.to_string());
    }
    match schema_type {
        Some("string") => Value::String(raw.to_string()),
        Some("integer" | "number" | "boolean" | "array" | "object") => {
            serde_json::from_str::<Value>(raw.trim())
                .unwrap_or_else(|_| Value::String(raw.to_string()))
        }
        _ => Value::String(raw.to_string()),
    }
}

fn parse_xml_tool_block(block: &str, tools: &[Value]) -> Option<(String, Value)> {
    let known_tools = declared_tool_names(tools);
    let (tool_name, mut cursor) = extract_tool_header(block, &known_tools)?;

    let mut args = serde_json::Map::new();
    let mut finds: Vec<String> = Vec::new();
    let mut replaces: Vec<String> = Vec::new();
    let mut edits_items: Option<Vec<Value>> = None;

    loop {
        let trimmed = cursor.trim_start();
        if trimmed.is_empty()
            || trimmed.starts_with("</function>")
            || trimmed.starts_with("</tool_call>")
        {
            break;
        }
        let Some(param_idx) = trimmed.find("<parameter=") else {
            break;
        };
        let after_eq = &trimmed[param_idx + "<parameter=".len()..];
        let gt_idx = after_eq.find('>')?;
        let key = after_eq[..gt_idx].trim();
        if key.is_empty() || key.contains(['<', '\n', '\r']) {
            return None;
        }
        let body_start = &after_eq[gt_idx + 1..];
        let close_idx = find_param_close(body_start)?;
        let raw_val = trim_xml_param_edges(&body_start[..close_idx]);
        if tool_name == "edit_file" && key == "find" {
            finds.push(raw_val.to_string());
        } else if tool_name == "edit_file" && key == "replace" {
            replaces.push(raw_val.to_string());
        } else {
            let schema_type = property_schema_type(tools, &tool_name, key);
            let val = decode_param_value(raw_val, key, schema_type);
            if tool_name == "edit_file" && key == "edits" {
                if let Value::Array(arr) = val {
                    edits_items = Some(arr);
                } else {
                    args.insert(key.to_string(), val);
                }
            } else {
                args.insert(key.to_string(), val);
            }
        }
        cursor = &body_start[close_idx + "</parameter>".len()..];
    }

    if tool_name == "edit_file" {
        if let Some(first_find) = finds.first() {
            args.insert("find".into(), Value::String(first_find.clone()));
        }
        if let Some(first_replace) = replaces.first() {
            args.insert("replace".into(), Value::String(first_replace.clone()));
        }
        let mut combined_edits: Vec<Value> = Vec::new();
        if finds.len() > 1 && finds.len() == replaces.len() {
            for (find, replace) in finds.iter().zip(replaces.iter()).skip(1) {
                combined_edits.push(json!({ "find": find, "replace": replace }));
            }
        }
        if let Some(existing) = edits_items {
            combined_edits.extend(existing);
        }
        if !combined_edits.is_empty() {
            args.insert("edits".into(), Value::Array(combined_edits));
        }
    }

    Some((tool_name, Value::Object(args)))
}

fn find_xml_tool_spans(text: &str, fences: &[(usize, usize)]) -> Vec<(usize, usize, usize, usize)> {
    let mut spans = Vec::new();
    let mut pos = 0usize;
    while let Some(open_tc) = find_outside_fences(text, "<tool_call>", pos, fences) {
        let inner_start = open_tc + "<tool_call>".len();
        let next_tc = find_outside_fences(text, "<tool_call>", inner_start, fences);
        if let Some(close_tc) = find_outside_fences(text, "</tool_call>", inner_start, fences) {
            if next_tc.is_none_or(|next| close_tc < next) {
                spans.push((open_tc, close_tc + "</tool_call>".len(), inner_start, close_tc));
                pos = close_tc + "</tool_call>".len();
                continue;
            }
        }
        if let Some(close_fn) = find_outside_fences(text, "</function>", inner_start, fences) {
            if next_tc.is_none_or(|next| close_fn < next) {
                let end = close_fn + "</function>".len();
                spans.push((open_tc, end, inner_start, end));
                pos = end;
                continue;
            }
        }
        pos = inner_start;
    }

    // Fallback: standalone `<function=...>...</function>` blocks outside `<tool_call>` spans and code fences.
    let mut covered = fences.to_vec();
    for &(s, e, _, _) in &spans {
        covered.push((s, e));
    }
    let mut fn_pos = 0usize;
    while let Some(open_fn) = find_outside_fences(text, "<function=", fn_pos, &covered) {
        if let Some(close_fn) = find_outside_fences(text, "</function>", open_fn, &covered) {
            let end = close_fn + "</function>".len();
            spans.push((open_fn, end, open_fn, end));
            covered.push((open_fn, end));
            fn_pos = end;
        } else {
            break;
        }
    }

    spans.sort_by_key(|span| span.0);
    spans
}

/// Recover XML-encoded tool calls that Strata/Qwen emitted inside `message["content"]`
/// instead of structured `message["tool_calls"]`.
pub fn recover_xml_tool_calls(message: &mut Value, tools: &[Value]) {
    if message
        .get("tool_calls")
        .and_then(Value::as_array)
        .is_some_and(|calls| !calls.is_empty())
    {
        return;
    }
    let Some(content) = message.get("content").and_then(Value::as_str).map(str::to_string) else {
        return;
    };
    if !(content.contains("<tool_call>")
        || content.contains("<function=")
        || content.contains("<|im_start|>functioncall"))
    {
        return;
    }

    let fences = code_fence_ranges(&content);
    let spans = find_xml_tool_spans(&content, &fences);
    if spans.is_empty() {
        return;
    }

    let mut recovered = Vec::new();
    let mut stripped_ranges = Vec::new();
    for (outer_start, outer_end, inner_start, inner_end) in spans {
        let block = &content[inner_start..inner_end];
        if let Some((name, args)) = parse_xml_tool_block(block, tools) {
            let id = format!("call_recovered_{}", recovered.len() + 1);
            recovered.push(json!({
                "id": id,
                "type": "function",
                "function": {
                    "name": name,
                    "arguments": args.to_string(),
                }
            }));
            stripped_ranges.push((outer_start, outer_end));
        }
    }

    if recovered.is_empty() {
        return;
    }

    let mut remaining = String::with_capacity(content.len());
    let mut cursor = 0usize;
    for (start, end) in stripped_ranges {
        if start > cursor {
            remaining.push_str(&content[cursor..start]);
        }
        cursor = end;
    }
    if cursor < content.len() {
        remaining.push_str(&content[cursor..]);
    }

    message["content"] = Value::String(remaining.trim().to_string());
    message["tool_calls"] = Value::Array(recovered);
}

/// Detect unparsed or truncated XML tool call markup leaked into assistant text outside code fences.
pub fn has_unparsed_tool_markup(text: &str) -> bool {
    let fences = code_fence_ranges(text);
    for marker in [
        "<tool_call>",
        "</tool_call>",
        "<|im_start|>functioncall",
        "<function=",
    ] {
        if find_outside_fences(text, marker, 0, &fences).is_some() {
            return true;
        }
    }
    if let Some(param_pos) = find_outside_fences(text, "<parameter=", 0, &fences) {
        if find_outside_fences(text, "</parameter>", param_pos, &fences).is_some()
            || find_outside_fences(text, "</function>", param_pos, &fences).is_some()
        {
            return true;
        }
    }
    false
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

    fn edit_file_test_tools() -> Vec<Value> {
        vec![json!({
            "type": "function",
            "function": {
                "name": "edit_file",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string"},
                        "expectedSha256": {"type": "string"},
                        "find": {"type": "string"},
                        "replace": {"type": "string"}
                    }
                }
            }
        })]
    }

    #[test]
    fn recovers_standard_xml_tool_call_and_preserves_multiline_text() {
        let tools = edit_file_test_tools();
        let mut msg = json!({
            "role": "assistant",
            "content": "Let me patch that.\n<tool_call>\n<function=edit_file>\n<parameter=path>\nC:\\work\\a.html\n</parameter>\n<parameter=expectedSha256>\nabcdef0123456789\n</parameter>\n<parameter=find>\n  line 1\n  line 2\n</parameter>\n<parameter=replace>\n  updated 1\n  updated 2\n</parameter>\n</function>\n</tool_call>",
            "tool_calls": []
        });
        recover_xml_tool_calls(&mut msg, &tools);
        let calls = msg["tool_calls"].as_array().expect("recovered tool calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "edit_file");
        let args: Value =
            serde_json::from_str(calls[0]["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["path"], r"C:\work\a.html");
        assert_eq!(args["expectedSha256"], "abcdef0123456789");
        assert_eq!(args["find"], "  line 1\n  line 2");
        assert_eq!(args["replace"], "  updated 1\n  updated 2");
        assert_eq!(msg["content"], "Let me patch that.");
        assert!(!has_unparsed_tool_markup(msg["content"].as_str().unwrap()));
    }

    #[test]
    fn recovers_mangled_iq3s_abyss_ping_tool_call_with_hybrid_find_replace_and_edits() {
        let tools = edit_file_test_tools();
        let leaked = concat!(
            "<tool_call>\n",
            "<|im_start|>functioncall<arg_key>=edit_file\n",
            "</parameter>\n",
            "<parameter=find>\n",
            "<div id=\"titlebar\"><b>ABYSS PING</b> sonar descent &middot; sector 7 <span id=\"tbDepth\">0 m</span></div>\n",
            "</parameter>\n",
            "<parameter=replace>\n",
            "<div id=\"titlebar\"><b>ABYSS PING</b> sonar descent &middot; <span id=\"tbSector\">SECTOR 1 / SILT FLATS</span> <span id=\"tbDepth\">0 m</span></div>\n",
            "</parameter>\n",
            "<parameter=edits>\n",
            "[{\"find\": \"  #meterFill.hot{background:linear-gradient(90deg,#7a2530,#ff5f6d)}\", \"replace\": \"  #meterFill.hot{background:linear-gradient(90deg,#7a2530,#ff5f6d)}\\n  #noiseMeter{width:190px;height:5px}\"}, ",
            "{\"find\": \"<span id=\\\"hPing\\\">READY</span>\", \"replace\": \"<span id=\\\"hPing\\\">READY</span><span id=\\\"hNoise\\\">SILENT</span>\"}, ",
            "{\"find\": \"dinner bell.\", \"replace\": \"dinner bell. cut throttle and drift.\"}, ",
            "{\"find\": \"pause\", \"replace\": \"decoy + pause\"}, ",
            "{\"find\": \"<span>pause</span>\", \"replace\": \"<span>decoy</span><span>pause</span>\"}, ",
            "{\"find\": \"@media (max-width:760px){#hud{font-size:10px}}\", \"replace\": \"@media (max-width:760px){#hud{font-size:9.5px}}\"}]\n",
            "</parameter>\n",
            "<parameter=path>\n",
            "\\\\?\\C:\\Users\\ronit\\OneDrive\\Tiedostot\\My Games\\Abyss Ping\\abyss-ping.html\n",
            "</parameter>\n",
            "<parameter=expectedSha256>\n",
            "6ac5b1f18896c90a8a840921b0e85cefdcef923b3d7a841dbab46b8a7c16562a\n",
            "</parameter>\n",
            "</function>\n",
            "</tool_call>"
        );
        let mut msg = json!({
            "role": "assistant",
            "content": leaked,
            "tool_calls": []
        });
        recover_xml_tool_calls(&mut msg, &tools);
        let calls = msg["tool_calls"].as_array().expect("recovered tool calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["function"]["name"], "edit_file");
        let args: Value =
            serde_json::from_str(calls[0]["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(
            args["path"],
            r"\\?\C:\Users\ronit\OneDrive\Tiedostot\My Games\Abyss Ping\abyss-ping.html"
        );
        assert_eq!(
            args["expectedSha256"],
            "6ac5b1f18896c90a8a840921b0e85cefdcef923b3d7a841dbab46b8a7c16562a"
        );
        assert_eq!(
            args["find"],
            "<div id=\"titlebar\"><b>ABYSS PING</b> sonar descent &middot; sector 7 <span id=\"tbDepth\">0 m</span></div>"
        );
        assert_eq!(args["edits"].as_array().unwrap().len(), 6);
        assert_eq!(msg["content"], "");
    }

    #[test]
    fn unparsed_tool_markup_ignores_code_fences_but_flags_truncated_calls() {
        assert!(!has_unparsed_tool_markup(
            "Here is an example:\n```xml\n<tool_call><function=read_file></function></tool_call>\n```\nAll done."
        ));
        assert!(has_unparsed_tool_markup(
            "<tool_call>\n<function=edit_file>\n<parameter=path>\nC:\\work\\a.txt"
        ));
    }
}
