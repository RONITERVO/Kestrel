//! Immutable event files overlay the original task JSON. The original file is never rewritten
//! during migration; journal sequence numbers continue after its legacy events.
use super::*;
use std::io::Write;

impl WorkspaceStore {
    fn task_journal_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.root.join("task-journal").join(id))
    }

    pub(super) fn replay_task_journal(&self, run: &mut ComputerTaskRun) -> Result<(), String> {
        let directory = self.task_journal_path(&run.id)?;
        if !directory.exists() {
            return Ok(());
        }
        let mut paths = Vec::new();
        for entry in fs::read_dir(directory).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let sequence = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.parse::<usize>().ok())
                    .ok_or_else(|| format!("Invalid task journal filename: {}", path.display()))?;
                paths.push((sequence, path));
            }
        }
        paths.sort_by_key(|(sequence, _)| *sequence);
        for (sequence, path) in paths {
            if sequence != run.events.len() + 1 {
                return Err(format!("Task {} has a missing or duplicate journal record before {sequence}. Its history is preserved; repair the journal before continuing.", run.id));
            }
            let event: ComputerTaskEvent = serde_json::from_slice(
                &fs::read(&path)
                    .map_err(|e| format!("Cannot read task journal {}: {e}", path.display()))?,
            )
            .map_err(|e| {
                format!(
                    "Task journal {} is damaged: {e}. No history was discarded.",
                    path.display()
                )
            })?;
            if event.run_id != run.id {
                return Err(format!(
                    "Task journal record {sequence} belongs to another task."
                ));
            }
            apply_event(run, event);
        }
        Ok(())
    }

    /// Caller holds the shared workspace write lock. The journal commit precedes the disposable
    /// index. Never roll back a committed event just because the index could not be written.
    pub(super) fn append_task_event_locked(
        &self,
        event: ComputerTaskEvent,
    ) -> Result<ComputerTaskRun, String> {
        let mut run = self.get_task(&event.run_id)?;
        let directory = self.task_journal_path(&run.id)?;
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = directory.join(format!("{:010}.json", run.events.len() + 1));
        let staging = directory.join(format!("{}.tmp", uuid::Uuid::new_v4()));
        let bytes = serde_json::to_vec(&event).map_err(|e| e.to_string())?;
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&staging)
                .map_err(|e| e.to_string())?;
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|e| e.to_string())?;
            drop(file);
            if path.exists() {
                return Err("Task journal changed concurrently; no record was overwritten.".into());
            }
            fs::rename(&staging, &path).map_err(|e| e.to_string())
        })();
        if let Err(error) = result {
            let _ = fs::remove_file(staging);
            return Err(format!(
                "Cannot save task history: {error}. No further action may start."
            ));
        }
        let new_direction = event.kind == "user_input";
        apply_event(&mut run, event);
        if new_direction {
            self.direction_changed.notify_one();
        }
        // A stale summary is repaired at startup or by the next successful append.
        if let Err(error) = self.write_task_summary(&run) {
            eprintln!("task index is stale for {}: {error}", run.id);
        }
        Ok(run)
    }
}

fn apply_event(run: &mut ComputerTaskRun, event: ComputerTaskEvent) {
    run.updated_at = event.at.clone();
    match event.kind.as_str() {
        "done" => run.status = "completed".into(),
        "error" => run.status = "failed".into(),
        "limit" => run.status = "paused".into(),
        "cancelled" => run.status = "cancelled".into(),
        "interrupted" => run.status = "interrupted".into(),
        "question" => run.status = "waiting".into(),
        "user_input" | "checkpoint" | "memory" | "reconciliation" => {}
        _ => run.status = "running".into(),
    }
    if event.kind == "artifact" {
        if let Some(path) = event
            .data
            .as_ref()
            .and_then(|d| d.get("path"))
            .and_then(serde_json::Value::as_str)
        {
            if !run.artifacts.iter().any(|known| known == path) {
                run.artifacts.push(path.to_string());
            }
        }
    }
    run.events.push(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn second_writer_cannot_recover_an_active_task_and_clone_keeps_ownership() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let task = store
            .create_task("local", "Keep working", ComputerTaskAccess::Workspace)
            .unwrap();
        let cloned = store.clone();
        drop(store);
        assert!(WorkspaceStore::new(directory.path())
            .err()
            .unwrap()
            .contains("already open"));
        assert_eq!(cloned.get_task(&task.id).unwrap().status, "starting");
        drop(cloned);
        let reopened = WorkspaceStore::new(directory.path()).unwrap();
        assert_eq!(reopened.get_task(&task.id).unwrap().status, "interrupted");
    }
    fn event(id: &str, kind: &str, detail: &str) -> ComputerTaskEvent {
        ComputerTaskEvent {
            run_id: id.into(),
            step: 1,
            kind: kind.into(),
            title: kind.into(),
            detail: detail.into(),
            data: None,
            at: chrono::Utc::now().to_rfc3339(),
        }
    }
    #[test]
    fn immutable_journal_recovers_with_missing_index_and_keeps_full_results() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let run = store
            .create_task("local", "Keep my work", ComputerTaskAccess::Workspace)
            .unwrap();
        let base = fs::read(store.task_path(&run.id).unwrap()).unwrap();
        let content = "complete result 🦉".repeat(10_000);
        store
            .add_task_event(event(&run.id, "tool_result", &content))
            .unwrap();
        store
            .add_task_event(event(&run.id, "done", "ready"))
            .unwrap();
        assert_eq!(fs::read(store.task_path(&run.id).unwrap()).unwrap(), base);
        fs::remove_file(store.task_index_path(&run.id).unwrap()).unwrap();
        drop(store);
        let reopened = WorkspaceStore::new(directory.path()).unwrap();
        let restored = reopened.get_task(&run.id).unwrap();
        assert_eq!(restored.events[0].detail, content);
        assert_eq!(restored.status, "completed");
        assert_eq!(reopened.list_tasks().unwrap()[0].event_count, 2);
    }
    #[test]
    fn legacy_events_keep_their_sequence_and_a_gap_stops_recovery() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let mut run = store
            .create_task("local", "legacy", ComputerTaskAccess::Workspace)
            .unwrap();
        run.events
            .push(event(&run.id, "user_input", "An early correction"));
        store.save_task(&run).unwrap();
        store
            .add_task_event(event(&run.id, "done", "complete"))
            .unwrap();
        assert_eq!(
            store.get_task(&run.id).unwrap().events[0].detail,
            "An early correction"
        );
        let journal = store.task_journal_path(&run.id).unwrap();
        fs::rename(
            journal.join("0000000002.json"),
            journal.join("0000000003.json"),
        )
        .unwrap();
        assert!(store
            .get_task(&run.id)
            .unwrap_err()
            .contains("missing or duplicate"));
    }
    #[test]
    fn damaged_record_is_not_silently_skipped_or_replaced_by_backup() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let run = store
            .create_task("local", "damage", ComputerTaskAccess::Workspace)
            .unwrap();
        store
            .add_task_event(event(&run.id, "done", "complete"))
            .unwrap();
        fs::write(
            store
                .task_journal_path(&run.id)
                .unwrap()
                .join("0000000001.json"),
            "broken",
        )
        .unwrap();
        assert!(store.get_task(&run.id).unwrap_err().contains("damaged"));
        drop(store);
        assert!(WorkspaceStore::new(directory.path()).is_err());
    }

    #[test]
    fn accepted_follow_up_recovers_as_interrupted_without_losing_completed_work() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let run = store
            .create_task("local", "Make a card", ComputerTaskAccess::Workspace)
            .unwrap();
        store
            .add_task_event(event(&run.id, "done", "The first card is ready"))
            .unwrap();
        store
            .add_task_event(event(&run.id, "user_input", "Change the wording"))
            .unwrap();
        store
            .add_task_event(event(&run.id, "queued", "Message saved"))
            .unwrap();
        drop(store);

        let reopened = WorkspaceStore::new(directory.path()).unwrap();
        let recovered = reopened.get_task(&run.id).unwrap();
        assert_eq!(recovered.status, "interrupted");
        assert_eq!(recovered.events.len(), 4);
        assert_eq!(recovered.events[0].detail, "The first card is ready");
        assert_eq!(recovered.events[1].detail, "Change the wording");
        assert_eq!(recovered.events[3].kind, "interrupted");
        assert_eq!(reopened.task_direction_revision(&run.id).unwrap(), 2);
    }
}
