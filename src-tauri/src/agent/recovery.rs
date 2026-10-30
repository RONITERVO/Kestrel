//! Reconcile an interrupted action by observation, never by executing it again. An uncertain
//! operation blocks further mutations until native evidence or an explicit human review settles it.
use super::*;
use tauri::Manager;

pub(super) fn is_mutation(name: &str, args: &Value) -> bool {
    match name {
        "write_file" | "create_directory" | "move_path" | "copy_file" | "run_program"
        | "open_path" => true,
        "app_call" => args
            .get("command")
            .and_then(Value::as_str)
            .and_then(|command| crate::app_control::policy(command).ok())
            .is_none_or(|(access, _)| access != "read"),
        _ => false,
    }
}

pub(super) fn require_reconciled(store: &WorkspaceStore, id: &str) -> Result<(), String> {
    let pending = store.task_memory(id)?.unsettled_actions;
    if let Some(sequence) = pending.first() {
        return Err(format!("An interrupted action has no recorded outcome (task_read sequence {sequence}). No new mutation was started. Use task_reconcile with that sequence to inspect native receipts/current files without replay. If its result is uncertain, ask the producer to review it in Working plan and memory; reading and discovery remain available."));
    }
    Ok(())
}

/// Capture bounded source identity before a file move/copy. This is native observation, not a
/// model-supplied claim. Large files/directories remain explicitly uncertain after interruption.
pub(super) fn source_identity(
    name: &str,
    args: &Value,
    access: Access,
    roots: &[String],
) -> Option<String> {
    if !matches!(name, "copy_file" | "move_path") {
        return None;
    }
    let path = allowed_existing(args.get("from")?.as_str()?, access, roots).ok()?;
    file_hash(&path).ok().flatten()
}

fn file_hash(path: &Path) -> Result<Option<String>, String> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    Ok(Some(text_sha256(&bytes)))
}

pub(super) fn reconcile(
    app: Option<&AppHandle>,
    store: &WorkspaceStore,
    id: &str,
    args: &Value,
    access: Access,
    roots: &[String],
) -> Result<ToolOutput, String> {
    let sequence = args
        .get("sequence")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or("task_reconcile requires a valid sequence")?;
    let action = store.unsettled_task_action(id, sequence)?;
    let data = action
        .data
        .as_ref()
        .ok_or("Interrupted action has no saved arguments; ask the producer to review it.")?;
    let name = data
        .get("tool")
        .and_then(Value::as_str)
        .or_else(|| action.title.strip_prefix("Using "))
        .unwrap_or("");
    let observation = if name == "app_call" {
        let app = app.ok_or("App receipts require the running desktop")?;
        let request_id = data
            .get("request_id")
            .and_then(Value::as_str)
            .ok_or("Missing native request ID")?;
        let request_id = format!("{id}-{request_id}");
        let state = app.state::<crate::AppState>();
        let path = state.control.path("actions", &request_id)?;
        if !path.exists() && !path.with_extension("json.backup").exists() {
            Some(
                json!({"outcome":"notSubmitted","requestId":request_id,"detail":"No native receipt exists. Native dispatch always saves a receipt before execution."}),
            )
        } else {
            let receipt = state.control.receipt(&request_id)?;
            if matches!(receipt.status.as_str(), "completed" | "failed") {
                Some(
                    json!({"outcome":"recorded","receipt":receipt,"detail":"The action's return is recorded; asynchronous generation must still be checked in its project/job."}),
                )
            } else {
                None
            }
        }
    } else {
        observe_file(name, data, access, roots)?
    };
    let Some(observation) = observation else {
        return Err(format!("Action {sequence} is still uncertain. No action was replayed. Inspect its file/project/job with read-only tools, explain what is known, and ask the producer to review this action in Working plan and memory before continuing."));
    };
    store.settle_task_action(id, sequence, "nativeObservation", observation.clone())?;
    Ok(ToolOutput {
        text: observation.to_string(),
        data: Some(observation),
        artifact: None,
    })
}

fn observe_file(
    name: &str,
    data: &Value,
    access: Access,
    roots: &[String],
) -> Result<Option<Value>, String> {
    let path = |key: &str| -> Result<PathBuf, String> {
        let raw = data
            .get(key)
            .and_then(Value::as_str)
            .ok_or("Interrupted action is missing a path")?;
        let candidate = allowed_new(raw, access, roots)?;
        // Follow existing links through the same policy before reading bytes.
        if candidate.exists() {
            allowed_existing(raw, access, roots)
        } else {
            Ok(candidate)
        }
    };
    let result = match name {
        "write_file" => {
            let target = path("path")?;
            let content = data
                .get("content")
                .and_then(Value::as_str)
                .ok_or("Missing saved write content")?;
            let current = file_hash(&target)?;
            if current.as_deref() == Some(text_sha256(content.as_bytes()).as_str()) {
                Some(json!({"outcome":"desiredStateObserved","path":target,"sha256":current}))
            } else if (!target.exists() && data.get("expectedSha256").is_none())
                || (current.is_some()
                    && current.as_deref() == data.get("expectedSha256").and_then(Value::as_str))
            {
                Some(json!({"outcome":"unchangedStateObserved","path":target,"sha256":current}))
            } else {
                None
            }
        }
        "create_directory" => {
            let target = path("path")?;
            if target.is_dir() {
                Some(json!({"outcome":"desiredStateObserved","path":target}))
            } else if !target.exists() {
                Some(json!({"outcome":"unchangedStateObserved","path":target}))
            } else {
                None
            }
        }
        "copy_file" | "move_path" => {
            let from = path("from")?;
            let to = path("to")?;
            let source_hash = data.get("sourceSha256").and_then(Value::as_str);
            let target_hash = file_hash(&to)?;
            if source_hash.is_some()
                && target_hash.as_deref() == source_hash
                && (name == "copy_file" || !from.exists())
            {
                Some(json!({"outcome":"desiredStateObserved","path":to,"sha256":target_hash}))
            } else if !to.exists()
                && source_hash.is_some()
                && file_hash(&from)?.as_deref() == source_hash
            {
                Some(json!({"outcome":"unchangedStateObserved","path":from,"sha256":source_hash}))
            } else {
                None
            }
        }
        _ => None,
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup(data: Value) -> (tempfile::TempDir, WorkspaceStore, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(dir.path()).unwrap();
        let run = store
            .create_task("local", "resume", Access::Workspace)
            .unwrap();
        store
            .add_task_event(ComputerTaskEvent {
                run_id: run.id.clone(),
                step: 1,
                kind: "tool_start".into(),
                title: "Using write_file".into(),
                detail: data.to_string(),
                data: Some(data),
                at: chrono::Utc::now().to_rfc3339(),
            })
            .unwrap();
        (dir, store, run.id)
    }
    #[test]
    fn crash_after_write_recovers_by_hash_without_overwriting_newer_work() {
        let target = tempfile::tempdir().unwrap();
        let file = target.path().join("draft.txt");
        let roots = vec![target.path().to_string_lossy().into_owned()];
        let (_dir, store, id) = setup(
            json!({"tool":"write_file","actionId":"write-1","mutation":true,"path":file,"content":"saved draft"}),
        );
        fs::write(&file, "saved draft").unwrap();
        assert!(require_reconciled(&store, &id).is_err());
        reconcile(
            None,
            &store,
            &id,
            &json!({"sequence":1}),
            Access::Workspace,
            &roots,
        )
        .unwrap();
        require_reconciled(&store, &id).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "saved draft");
        let (_dir, store, id) = setup(
            json!({"tool":"write_file","actionId":"write-2","mutation":true,"path":file,"content":"old proposed draft"}),
        );
        assert!(reconcile(
            None,
            &store,
            &id,
            &json!({"sequence":1}),
            Access::Workspace,
            &roots
        )
        .err()
        .unwrap()
        .contains("still uncertain"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "saved draft");
        assert!(require_reconciled(&store, &id).is_err());
    }
    #[test]
    fn observation_respects_workspace_and_reads_are_available_during_recovery() {
        assert!(!is_mutation(
            "app_call",
            &json!({"command":"get_image_project"})
        ));
        assert!(is_mutation(
            "app_call",
            &json!({"command":"create_image_project"})
        ));
        let approved = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let (_dir, store, id) = setup(
            json!({"tool":"write_file","mutation":true,"path":other.path().join("private.txt"),"content":"draft"}),
        );
        assert!(reconcile(
            None,
            &store,
            &id,
            &json!({"sequence":1}),
            Access::Workspace,
            &[approved.path().to_string_lossy().into_owned()]
        )
        .is_err());
        assert_eq!(store.task_memory(&id).unwrap().unsettled_actions, vec![1]);
    }
}
