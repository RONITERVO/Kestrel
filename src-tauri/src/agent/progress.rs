//! Customer-facing status is a projection of native events. Tool arguments and reasoning belong
//! in technical history, not the progress line of a producer's conversation.
use crate::models::{ComputerTaskEvent, ComputerTaskRun, ProducerTaskStatus};

pub(crate) fn snapshot(
    run: &ComputerTaskRun,
    progress: Option<(String, String, String)>,
) -> ProducerTaskStatus {
    let last = run.events.last();
    let mut status = ProducerTaskStatus {
        id: run.id.clone(),
        status: run.status.clone(),
        step: last.map_or(0, |e| e.step),
        title: last.map_or_else(|| "Starting your request".into(), |e| e.title.clone()),
        detail: last.map_or_else(String::new, |e| e.detail.chars().take(1800).collect()),
        question: if run.status == "waiting" {
            run.events
                .iter()
                .rev()
                .find(|e| e.kind == "question")
                .and_then(|e| e.data.clone())
        } else {
            None
        },
        artifact_paths: run.artifacts.iter().rev().take(24).cloned().collect(),
        event_count: run.events.len(),
        updated_at: run.updated_at.clone(),
    };
    if !matches!(run.status.as_str(), "starting" | "running") {
        return status;
    }
    let intent = run
        .events
        .iter()
        .rev()
        .take_while(|e| e.kind != "user_input")
        .find(|e| e.kind == "message")
        .map(|e| e.detail.chars().take(600).collect::<String>());
    if let Some(event) = last {
        let (title,detail)=match event.kind.as_str() {
            "queued"|"start"|"preparing"=>("Preparing your local assistant".into(),"Loading or waiting for the selected local model. Your request is saved.".into()),
            "thinking"|"reasoning"=>("Planning the next step".into(), intent.clone().unwrap_or_else(||{
                run.events.iter().rev().find(|e|e.kind=="tool_start")
                    .map(|e|format!("Last activity: {}. The local assistant is deciding what to do next.",tool_label(e).0.to_lowercase()))
                    .unwrap_or_else(||"The local assistant is working on your request.".into())
            })),
            "tool_start"=>tool_label(event),
            "tool_result"=>("Step finished".into(),intent.clone().unwrap_or_else(||"The result is saved. The local assistant will check the next step.".into())),
            "tool_error"=>("Checking a tool error".into(),"The attempted step reported a problem. Its details are saved while the local assistant reviews what to do next.".into()),
            "checkpoint"|"memory"=>("Working plan saved".into(),"Keeping decisions and next steps for later continuation.".into()),
            _=>(status.title.clone(),status.detail.clone()),
        };
        status.title = title;
        status.detail = detail;
        if let Some((title, detail, at)) = progress.filter(|(_, _, at)| {
            chrono::DateTime::parse_from_rfc3339(at)
                .ok()
                .zip(chrono::DateTime::parse_from_rfc3339(&event.at).ok())
                .is_some_and(|(new, old)| new >= old)
        }) {
            if matches!(
                event.kind.as_str(),
                "tool_start"
                    | "tool_result"
                    | "tool_error"
                    | "queued"
                    | "start"
                    | "preparing"
                    | "thinking"
            ) {
                status.title = match title.as_str() {
                    "starting" => "Loading your local assistant".into(),
                    "ready" => "Local assistant ready".into(),
                    _ => title,
                };
                status.detail = detail;
                status.updated_at = at;
            }
        }
    }
    status
}

fn tool_label(event: &ComputerTaskEvent) -> (String, String) {
    let name = event
        .data
        .as_ref()
        .and_then(|d| d.get("tool"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| event.title.strip_prefix("Using "))
        .unwrap_or("");
    let (title, detail) = match name {
        "task_memory" => (
            "Updating the working plan",
            "Saving decisions and next steps so this work can continue later.",
        ),
        "task_history" | "task_read" => (
            "Recovering earlier work",
            "Checking saved messages and results before continuing.",
        ),
        "task_reconcile" => (
            "Checking an interrupted action",
            "Inspecting what happened without repeating the action.",
        ),
        "app_actions" | "app_describe" => (
            "Checking available tools",
            "Finding the local actions needed for your request.",
        ),
        "app_call" => (
            "Working with local tools",
            "Checking or updating the project through Kestrel's local tools.",
        ),
        "read_file" | "read_attachment" | "list_directory" => (
            "Reading local material",
            "Checking the relevant files and saved work.",
        ),
        "write_file" => (
            "Saving your work",
            "Writing the result to your approved workspace.",
        ),
        "create_directory" => (
            "Preparing the output folder",
            "Creating the folder for your files.",
        ),
        "copy_file" | "move_path" => (
            "Organizing output files",
            "Placing files in their chosen location.",
        ),
        _ => (
            "Working on your request",
            "The local assistant is carrying out the next action.",
        ),
    };
    (title.into(), detail.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{models::ComputerTaskAccess, workspace::WorkspaceStore};
    #[test]
    fn active_status_hides_arguments_and_old_runtime_progress_but_keeps_terminal_answers() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(temp.path()).unwrap();
        let task = store
            .create_task("local", "Make a guide", ComputerTaskAccess::Workspace)
            .unwrap();
        let event = ComputerTaskEvent {
            run_id: task.id.clone(),
            step: 2,
            kind: "tool_start".into(),
            title: "Using task_memory".into(),
            detail: "{opaque checkpoint JSON}".into(),
            data: None,
            at: "2026-10-30T10:00:02Z".into(),
        };
        let run = store.add_task_event(event.clone()).unwrap();
        let old = Some((
            "rendering".into(),
            "old job".into(),
            "2026-10-30T10:00:01Z".into(),
        ));
        let status = snapshot(&run, old);
        assert_eq!(status.title, "Updating the working plan");
        assert!(!status.detail.contains("JSON"));
        let mut thinking = event.clone();
        thinking.kind = "thinking".into();
        let run = store.add_task_event(thinking).unwrap();
        let status = snapshot(
            &run,
            Some((
                "starting".into(),
                "Loading the local model".into(),
                "2026-10-30T10:00:03Z".into(),
            )),
        );
        assert_eq!(status.title, "Loading your local assistant");
        let mut done = event;
        done.kind = "done".into();
        done.detail = "Your guide is ready.".into();
        let run = store.add_task_event(done).unwrap();
        assert_eq!(snapshot(&run, None).detail, "Your guide is ready.");
    }
}
