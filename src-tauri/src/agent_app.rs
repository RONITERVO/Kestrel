//! Computer Tasks' app tools. The model chooses producer actions; native policy owns authority.
use crate::{agent, app_control, AppState};
use kestrel_app_core::{AppActionRequest, ComputerTaskAccess, ControlSettings};
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

pub fn schemas() -> Vec<Value> {
    let schema = |name: &str, description: &str, parameters: Value| json!({"type":"function","function":{"name":name,"description":description,"parameters":parameters}});
    vec![
        schema("app_actions", "List native producer action names and access by name fragment, e.g. music, image, movie, draft, research. Use app_describe for a selected action's guidance and exact formats. App projects are managed through these actions, never by editing internal files.", json!({"type":"object","properties":{"filter":{"type":"string"}}})),
        schema("app_describe", "Read the exact Rust-owned argument types, access policy and guidance for an action before invoking it.", json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]})),
        schema("app_call", "Perform one native producer action with typed arguments. Use a unique request_id per intentional action; retry unchanged with the same ID. You are the producer: write text yourself using save actions OR ask the app's local model via start_studio_prompt_draft and inspect get_app_job before saving. Generation acceptance is not completion. Never claim a render exists without inspecting its completed native take.", json!({"type":"object","properties":{"request_id":{"type":"string"},"command":{"type":"string"},"args":{"type":"object"}},"required":["request_id","command","args"]})),
    ]
}

pub fn artifact(arguments: &Value, receipt: &Value) -> Option<std::path::PathBuf> {
    let command = arguments.get("command")?.as_str()?;
    let result = receipt.get("result")?;
    let path = match command {
        "run_research" | "get_report" => result.get("htmlPath")?.as_str()?,
        "export_narration" | "export_music_lyrics" => {
            result.get("files")?.as_array()?.first()?.as_str()?
        }
        "export_production_bundle" => result.get("manifestPath")?.as_str()?,
        "export_media_composition" => result.get("mediaPath")?.as_str()?,
        "get_movie" => result.get("finalPath")?.as_str()?,
        "get_music_project" | "get_image_project" => {
            let active = result.get("activeTakeId")?.as_str()?;
            result
                .get("takes")?
                .as_array()?
                .iter()
                .find(|take| {
                    take.get("id").and_then(Value::as_str) == Some(active)
                        && take.get("status").and_then(Value::as_str) == Some("complete")
                })?
                .get("path")?
                .as_str()?
        }
        _ => return None,
    };
    let path = std::path::PathBuf::from(path);
    path.is_file().then_some(path)
}

pub async fn execute(
    app: &AppHandle,
    run_id: &str,
    name: &str,
    args: &Value,
    settings: &ControlSettings,
    access: ComputerTaskAccess,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    let command = || {
        args.get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| "command is required".to_string())
    };
    match name {
        "app_actions" => Ok(json!(app_control::catalog(
            args.get("filter").and_then(Value::as_str)
        )
        .iter()
        .map(|name| {
            let (access, _) =
                app_control::policy(name).unwrap_or_else(|error| ("unavailable".into(), error));
            json!({"command":name,"access":access})
        })
        .collect::<Vec<_>>())),
        "app_describe" => {
            serde_json::to_value(app_control::describe(command()?)?).map_err(|e| e.to_string())
        }
        "app_call" => {
            let command = command()?;
            if matches!(
                command,
                "start_computer_task"
                    | "start_producer_task"
                    | "resume_computer_task"
                    | "send_producer_message"
                    | "acknowledge_task_action"
                    | "stop_computer_task"
                    | "start_chat_stream"
                    | "start_local_model"
                    | "stop_local_model"
                    | "release_ai_memory"
            ) {
                return Err("A Computer Task cannot recursively start tasks/chat or change the producer's selected runtime. Use Studio drafting actions for local collaboration.".into());
            }
            let arguments = args.get("args").cloned().ok_or("args is required")?;
            let configured = app
                .state::<AppState>()
                .research_settings
                .load()
                .map_err(|e| e.to_string())?;
            let trusted = [
                configured.comfy_root,
                configured.muscriptor_executable_path,
                configured.muscriptor_model_path,
            ];
            check_paths(&arguments, settings, access, &trusted)?;
            let request = AppActionRequest {
                request_id: format!(
                    "{run_id}-{}",
                    args.get("request_id")
                        .and_then(Value::as_str)
                        .ok_or("request_id is required")?
                ),
                command: command.into(),
                args: arguments,
            };
            let state = app.state::<AppState>();
            *state
                .control
                .task_progress
                .lock()
                .map_err(|e| e.to_string())? = None;
            // The planning lease is already dropped. The task keeps its separate ownership gate,
            // while the ordinary command takes the normal work lock and inference lease itself.
            state.work_active.store(false, Ordering::Release);
            // Saved plans keep workspace scope even when the current task has Full Access;
            // a later unattended resume never inherits a one-off full-access opt-in.
            let operation = app_control::TASK_WORKSPACE.scope(
                settings.agent_workspace_roots.clone(),
                app_control::TASK_ACTION.scope(true, app_control::execute(app, request)),
            );
            tokio::pin!(operation);
            let result = tokio::select! {
                result = &mut operation => result,
                _ = cancel.cancelled() => {
                    // Cancellation never drops a mutation midway through saving its receipt.
                    // The task owns the workspace, so these are its child jobs.
                    for registry in [&state.jobs, &state.interactive_jobs, &state.movie_jobs, &state.music_jobs, &state.image_generation_jobs, &state.speech_jobs] {
                        if let Ok(jobs) = registry.lock() { for token in jobs.values() { token.cancel(); } }
                    }
                    operation.await
                }
            };
            if command == "start_producer_workflow" {
                let id = args
                    .pointer("/args/id")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                loop {
                    let active = state
                        .control
                        .workflows
                        .lock()
                        .map_err(|e| e.to_string())?
                        .get(id)
                        .cloned();
                    let Some(active) = active else { break };
                    if cancel.is_cancelled() {
                        active.cancel();
                        for registry in [
                            &state.jobs,
                            &state.interactive_jobs,
                            &state.movie_jobs,
                            &state.music_jobs,
                            &state.image_generation_jobs,
                            &state.speech_jobs,
                        ] {
                            if let Ok(jobs) = registry.lock() {
                                for token in jobs.values() {
                                    token.cancel();
                                }
                            }
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
            let mut stopped = false;
            while state.work_active.load(Ordering::Acquire) {
                if cancel.is_cancelled() && !stopped {
                    stopped = true;
                    cancel_owned_job(command, args.get("args").unwrap_or(&Value::Null), &state)?;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            state.work_active.store(true, Ordering::Release);
            serde_json::to_value(result?).map_err(|e| e.to_string())
        }
        _ => Err(format!("Unknown app tool: {name}")),
    }
}

fn cancel_owned_job(command: &str, args: &Value, state: &AppState) -> Result<(), String> {
    let id = args
        .get("id")
        .or_else(|| args.pointer("/request/requestId"))
        .and_then(Value::as_str);
    if let Some(id) = id {
        let registry = match command {
            "start_image_generation" => &state.image_generation_jobs,
            "start_music_generation" => &state.music_jobs,
            "render_movie_scenes" | "start_movie_editor_generation" => &state.movie_jobs,
            _ => &state.interactive_jobs,
        };
        if let Some(token) = registry.lock().map_err(|e| e.to_string())?.get(id) {
            token.cancel();
        }
    }
    Ok(())
}

pub(crate) fn check_paths(
    value: &Value,
    settings: &ControlSettings,
    access: ComputerTaskAccess,
    trusted: &[String],
) -> Result<(), String> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                // Speech exports use opaque app-owned cache references. Their native resolver
                // rejects escapes; treating these as filesystem paths would break parity.
                if key == "relativePath" {
                    continue;
                }
                if key == "paths" {
                    if let Some(paths) = child.as_array() {
                        for path in paths {
                            agent::allowed_existing(
                                path.as_str().ok_or("paths must contain strings")?,
                                access,
                                &settings.agent_workspace_roots,
                            )?;
                        }
                    }
                } else if (key.to_ascii_lowercase().ends_with("path")
                    || key.to_ascii_lowercase().ends_with("directory")
                    || key.to_ascii_lowercase().ends_with("root"))
                    && child.as_str().is_some_and(|s| !s.is_empty())
                {
                    let path = child.as_str().unwrap();
                    let installed_tool_field = matches!(
                        key.as_str(),
                        "comfyRoot"
                            | "muscriptorExecutablePath"
                            | "muscriptorModelPath"
                            | "executablePath"
                            | "modelPath"
                    );
                    if installed_tool_field
                        && std::path::Path::new(path).canonicalize().is_ok_and(|path| {
                            trusted.iter().any(|configured| {
                                std::path::Path::new(configured)
                                    .canonicalize()
                                    .is_ok_and(|configured| path == configured)
                            })
                        })
                    {
                        continue;
                    }
                    if std::path::Path::new(path).exists() {
                        agent::allowed_existing(path, access, &settings.agent_workspace_roots)?;
                    } else {
                        agent::allowed_new(path, access, &settings.agent_workspace_roots)?;
                    }
                } else {
                    check_paths(child, settings, access, trusted)?;
                }
            }
        }
        Value::Array(array) => {
            for child in array {
                check_paths(child, settings, access, trusted)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspected_research_and_audio_exports_are_visible_task_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let report = dir.path().join("report.html");
        let audio = dir.path().join("narration.m4a");
        std::fs::write(&report, "local report").unwrap();
        std::fs::write(&audio, "local audio").unwrap();
        assert_eq!(
            artifact(
                &json!({"command":"run_research"}),
                &json!({"result":{"htmlPath":report}})
            ),
            Some(report)
        );
        assert_eq!(
            artifact(
                &json!({"command":"export_narration"}),
                &json!({"result":{"files":[audio]}})
            ),
            Some(audio)
        );
    }
    #[test]
    fn app_tools_cannot_use_paths_to_escape_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let settings = ControlSettings {
            agent_workspace_roots: vec![temp.path().to_string_lossy().into_owned()],
            ..ControlSettings::default()
        };
        assert!(check_paths(
            &json!({"paths":["relative.txt"]}),
            &settings,
            ComputerTaskAccess::Workspace,
            &[]
        )
        .is_err());
        assert!(check_paths(
            &json!({"request":{"outputPath":"relative.txt"}}),
            &settings,
            ComputerTaskAccess::Workspace,
            &[]
        )
        .is_err());
    }
}
