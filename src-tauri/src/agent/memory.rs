//! Task-scoped memory tools: no arbitrary file access, no cross-task writes, no inference.
use super::*;

pub(super) fn schemas() -> Vec<Value> {
    let strings = json!({"type":"array","items":{"type":"string"}});
    vec![
        schema("task_reconcile", "Inspect the native receipt/current file for an interrupted action without executing it again. Supply its sequence from unsettledActions. Native observations may settle the uncertainty; otherwise ask the producer to review the action in Working plan and memory. Never retry with a new ID to bypass this gate.",json!({"type":"object","properties":{"sequence":{"type":"integer","minimum":1}},"required":["sequence"]})),
        schema("task_memory", "Call {} to read durable checkpoint, revision, user-direction sequences, and unsettled actions. To update, pass expectedRevision and checkpoint:{summary,plan,decisions,references,nextSteps}. Save before long work, asking, or finishing; never invent completion evidence.", json!({"type":"object","properties":{
            "expectedRevision":{"type":"integer","minimum":0},
            "checkpoint":{"type":"object","properties":{
                "summary":{"type":"string"},"decisions":strings,"references":strings,"nextSteps":strings,
                "plan":{"type":"array","items":{"type":"object","properties":{
                    "id":{"type":"string"},"description":{"type":"string"},
                    "status":{"type":"string","enum":["pending","inProgress","done","blocked"]},
                    "evidence":{"type":"array","description":"Use [] for pending work; done items require tool_result/artifact sequence IDs (not 0).","items":{"type":"integer","minimum":1}}
                },"required":["id","description","status","evidence"]}}
            },"required":["summary","plan","decisions","references","nextSteps"]}
        }})),
        schema("task_history", "Search durable events in this task (user_input, tool_start, tool_result, tool_error, artifact, checkpoint). Paginate with nextAfter; inspect exact records with task_read.", json!({"type":"object","properties":{"after":{"type":"integer","minimum":0},"kind":{"type":"string"},"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":30}}})),
        schema("task_read", "Read exact saved payload in character pages by sequence (0 = original objective) with optional JSON pointer (e.g. /result/sources) and offset/limit.", json!({"type":"object","properties":{"sequence":{"type":"integer","minimum":0},"pointer":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":16000}},"required":["sequence"]})),
    ]
}

pub(super) fn execute(
    store: &WorkspaceStore,
    run_id: &str,
    name: &str,
    args: &Value,
) -> Result<ToolOutput, String> {
    let value = match name {
        "task_memory" => {
            let is_write = args.get("checkpoint").is_some()
                || args.get("expectedRevision").is_some()
                || args.get("summary").is_some()
                || args.get("plan").is_some();
            let snapshot = if is_write {
                let normalized = normalize_checkpoint_args(args, store, run_id)?;
                store.update_task_checkpoint(
                    run_id,
                    serde_json::from_value(normalized)
                        .map_err(|e| format!("Invalid working checkpoint: {e}"))?,
                )?
            } else {
                store.task_memory(run_id)?
            };
            serde_json::to_value(snapshot)
        }
        "task_history" => serde_json::to_value(
            store.task_history(
                run_id,
                serde_json::from_value(args.clone())
                    .map_err(|e| format!("Invalid history query: {e}"))?,
            )?,
        ),
        "task_read" => {
            let sequence = args
                .get("sequence")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("task_read requires a valid sequence")?;
            let offset = args
                .get("offset")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(8000);
            serde_json::to_value(store.read_task_payload(
                run_id,
                sequence,
                args.get("pointer").and_then(Value::as_str),
                offset,
                limit,
            )?)
        }
        _ => return Err("Unknown task memory tool".into()),
    }
    .map_err(|e| e.to_string())?;
    Ok(ToolOutput {
        text: value.to_string(),
        artifact: None,
        data: Some(value),
    })
}

/// Keep bounded previews in the live conversation; the entire result is already in the journal.
pub(super) fn preview(text: &str, sequence: usize) -> String {
    // Receipts echo their original arguments for durable audit. Repeating a long script or
    // narration before its result can hide the actual outcome from the bounded live preview.
    // The unmodified receipt remains available at the native record sequence.
    let compact = crate::workspace::task_result_preview(text);
    let text = compact.as_str();
    if text.chars().count() <= 12_000 {
        return format!("{text}\n[Native result saved as task_read sequence {sequence}. Use this sequence if citing evidence in task_memory.]");
    }
    format!("{}\n[Preview truncated. Full native result is task_read sequence {sequence}; use paged reads or task_history. Do not treat this preview as the entire result.]",text.chars().take(12_000).collect::<String>())
}

pub(super) fn with_context(original: &Value, context: &str) -> Value {
    // Retrieved tool/source text stays below system policy; never append it to system messages.
    let mut remaining = 8000;
    let mut text_preview = |text: &str| {
        let mut end = text.len().min(remaining);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        remaining -= end;
        if end < text.len() {
            format!("{}\n[Initial context preview. Retrieve the original request with task_read sequence 0 and attachment text with read_attachment before acting on omitted detail.]", &text[..end])
        } else {
            text.into()
        }
    };
    if let Some(parts) = original.as_array() {
        let mut parts = parts.clone();
        for part in &mut parts {
            if part.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    part["text"] = json!(text_preview(text));
                }
            }
        }
        parts.push(json!({"type":"text","text":context}));
        json!(parts)
    } else {
        json!(format!(
            "{}\n\n{context}",
            text_preview(original.as_str().unwrap_or_default())
        ))
    }
}

/// Reasoning is saved as a native event for inspection. It is not working memory and must not
/// consume the next planning prompt (nor carry provider-specific stream/usage metadata with it).
pub(super) fn conversation_reply(message: &Value) -> Value {
    let mut reply = json!({"role":"assistant","content":message.get("content").cloned().unwrap_or(Value::Null)});
    if let Some(calls) = message.get("tool_calls") {
        reply["tool_calls"] = calls.clone();
    }
    reply
}

/// Recover common local-model shape drift on checkpoint writes (double-nested `checkpoint`,
/// top-level fields without wrapper, omitted `expectedRevision`, missing empty arrays, unknown
/// keys, and `in_progress` snake-case) without weakening evidence or revision validation.
fn normalize_checkpoint_args(
    args: &Value,
    store: &WorkspaceStore,
    run_id: &str,
) -> Result<Value, String> {
    let top = args.as_object().cloned().unwrap_or_default();
    let mut expected_revision = top.get("expectedRevision").filter(|v| !v.is_null()).cloned();

    let mut inner = match top.get("checkpoint") {
        Some(Value::Object(cp)) => {
            if expected_revision.is_none() {
                expected_revision = cp.get("expectedRevision").filter(|v| !v.is_null()).cloned();
            }
            if let Some(Value::Object(nested)) = cp.get("checkpoint") {
                if !cp.contains_key("summary") && !cp.contains_key("plan") {
                    nested.clone()
                } else {
                    cp.clone()
                }
            } else {
                cp.clone()
            }
        }
        _ => top,
    };

    if expected_revision.is_none() {
        let current = store.task_memory(run_id)?;
        expected_revision = Some(json!(current.revision));
    }

    let mut clean_cp = serde_json::Map::new();
    clean_cp.insert(
        "summary".into(),
        inner.remove("summary").unwrap_or_else(|| json!("")),
    );
    for key in ["decisions", "references", "nextSteps"] {
        clean_cp.insert(
            key.into(),
            match inner.remove(key) {
                Some(Value::Array(arr)) => Value::Array(arr),
                _ => Value::Array(Vec::new()),
            },
        );
    }
    let raw_plan = match inner.remove("plan") {
        Some(Value::Array(arr)) => arr,
        _ => Vec::new(),
    };
    let task_events = if raw_plan.iter().any(|item| {
        item.get("status").and_then(Value::as_str) == Some("done")
            && item
                .get("evidence")
                .and_then(Value::as_array)
                .is_some_and(|arr| !arr.is_empty())
    }) {
        store.get_task(run_id).ok().map(|run| run.events)
    } else {
        None
    };
    let clean_plan = raw_plan
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let mut obj = match item {
                Value::Object(map) => map,
                _ => return None,
            };
            let id = match obj.get("id").and_then(Value::as_str) {
                Some(s) if !s.trim().is_empty() => s.to_string(),
                _ => format!("step-{}", index + 1),
            };
            let description = obj.remove("description").unwrap_or_else(|| json!(""));
            let mut status = match obj.get("status").and_then(Value::as_str) {
                Some("in_progress" | "in-progress") => "inProgress".to_string(),
                Some(s) => s.to_string(),
                None => "pending".to_string(),
            };
            let mut evidence = match obj.remove("evidence") {
                Some(Value::Array(arr)) => arr,
                _ => Vec::new(),
            };
            if status == "done" && !evidence.is_empty() {
                if let Some(events) = task_events.as_deref() {
                    let seqs: Option<Vec<u32>> = evidence
                        .iter()
                        .map(|val| val.as_u64().and_then(|n| u32::try_from(n).ok()))
                        .collect();
                    if let Some(seqs) = seqs {
                        let all_exist = seqs
                            .iter()
                            .all(|&s| s >= 1 && (s as usize) <= events.len());
                        let has_valid = seqs.iter().any(|&s| {
                            let idx = (s - 1) as usize;
                            let kind = events[idx].kind.as_str();
                            matches!(kind, "tool_result" | "artifact")
                                || (kind == "tool_start"
                                    && events.get(idx + 1).is_some_and(|next| {
                                        matches!(next.kind.as_str(), "tool_result" | "artifact")
                                    }))
                        });
                        let has_discussion = seqs.iter().any(|&s| {
                            let idx = (s - 1) as usize;
                            matches!(
                                events[idx].kind.as_str(),
                                "question" | "user_input" | "checkpoint"
                            )
                        });
                        if all_exist && !has_valid && has_discussion {
                            status = "inProgress".into();
                            evidence.clear();
                        }
                    }
                }
            }
            Some(json!({
                "id": id,
                "description": description,
                "status": status,
                "evidence": evidence,
            }))
        })
        .collect();
    clean_cp.insert("plan".into(), Value::Array(clean_plan));

    let mut out = serde_json::Map::new();
    if let Some(rev) = expected_revision {
        out.insert("expectedRevision".into(), rev);
    }
    out.insert("checkpoint".into(), Value::Object(clean_cp));
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_reasoning_is_not_replayed_and_native_results_remain_retrievable() {
        let message = json!({"role":"assistant","content":"Checking the saved file.","reasoning_content":"r".repeat(100_000),"tool_calls":[{"id":"a","function":{"name":"read_file","arguments":"{}"}}],"usage":{"output_tokens":32000}});
        let reply = conversation_reply(&message);
        assert!(reply.get("reasoning_content").is_none());
        assert!(reply.get("usage").is_none());
        assert_eq!(reply["tool_calls"], message["tool_calls"]);
        let result = preview(&"ä🦉".repeat(10000), 42);
        assert!(result.contains("sequence 42"));
        assert!(result.chars().count() < 12300);
    }

    #[test]
    fn receipt_preview_prioritizes_outcome_over_echoed_input() {
        let receipt = json!({"request":{"command":"synthesize_local_speech","requestId":"voice-1","args":{"text":"long input".repeat(4000)}},"status":"completed","result":{"narrationCheck":{"passed":false},"relativePath":"clip.opus"},"error":null});
        let result = preview(&receipt.to_string(), 42);
        assert!(result.contains("\"passed\":false"));
        assert!(result.contains("clip.opus"));
        assert!(!result.contains("long input"));
        assert!(result.contains("sequence 42"));
        assert!(receipt["request"]["args"].is_object());
    }

    #[test]
    fn task_memory_normalizes_drift_without_relaxing_revision_or_evidence_gates() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(dir.path()).unwrap();
        let task = store
            .create_task("local", "normalize", ComputerTaskAccess::Workspace)
            .unwrap();

        let initial = execute(&store, &task.id, "task_memory", &json!({})).unwrap();
        assert_eq!(initial.data.unwrap()["revision"], 0);

        let nested = execute(
            &store,
            &task.id,
            "task_memory",
            &json!({
                "checkpoint": {
                    "checkpoint": {
                        "summary": "Build Aurora Watch",
                        "plan": [
                            {"description": "Create portraits", "status": "in_progress", "extra_note": "ignored"}
                        ],
                        "plan_note": "extra key dropped"
                    }
                }
            }),
        )
        .unwrap();
        let nested_data = nested.data.unwrap();
        assert_eq!(nested_data["revision"], 1);
        assert_eq!(nested_data["checkpoint"]["plan"][0]["id"], "step-1");
        assert_eq!(nested_data["checkpoint"]["plan"][0]["status"], "inProgress");

        let flat = execute(
            &store,
            &task.id,
            "task_memory",
            &json!({
                "summary": "Flat top-level checkpoint",
                "plan": []
            }),
        )
        .unwrap();
        assert_eq!(flat.data.unwrap()["revision"], 2);

        let stale = execute(
            &store,
            &task.id,
            "task_memory",
            &json!({
                "expectedRevision": 1,
                "checkpoint": {"summary": "Stale revision"}
            }),
        );
        assert!(stale.is_err());

        let ungrounded_done = execute(
            &store,
            &task.id,
            "task_memory",
            &json!({
                "checkpoint": {
                    "summary": "Claimed done without evidence",
                    "plan": [{"id": "s1", "description": "Finish", "status": "done", "evidence": []}]
                }
            }),
        );
        assert!(ungrounded_done.is_err());

        let q_seq = store
            .add_task_event_if_current(
                ComputerTaskEvent {
                    run_id: task.id.clone(),
                    step: 1,
                    kind: "question".into(),
                    title: "Scope clarification".into(),
                    detail: "Fresh or reuse?".into(),
                    data: None,
                    at: "2026-11-14T00:00:00Z".into(),
                },
                0,
            )
            .unwrap()
            .unwrap() as u32;
        let downgraded = execute(
            &store,
            &task.id,
            "task_memory",
            &json!({
                "checkpoint": {
                    "summary": "Scope clarified via ask_user",
                    "plan": [{"id": "scope", "description": "Clarify scope", "status": "done", "evidence": [q_seq]}]
                }
            }),
        )
        .unwrap();
        let downgraded_data = downgraded.data.unwrap();
        assert_eq!(downgraded_data["revision"], 3);
        assert_eq!(downgraded_data["checkpoint"]["plan"][0]["status"], "inProgress");
        assert_eq!(downgraded_data["checkpoint"]["plan"][0]["evidence"], json!([]));
    }
}
