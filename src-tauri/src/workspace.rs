//! Durable local conversations and computer-task transcripts.
//!
//! Files in this module are user data, not a cache. Each update is written through a temporary
//! file with a recovery copy so a crash cannot silently erase a long conversation or task audit.

use crate::attachments::ContextAttachment;
use crate::models::{
    ChatMessage, ChatSession, ChatSessionSummary, ComputerTaskAccess, ComputerTaskEvent,
    ComputerTaskRun, ComputerTaskSummary, SpeechRecordingAttachment,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct WorkspaceStore {
    root: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

struct ChatMessageDraft {
    role: String,
    content: String,
    reasoning: Option<String>,
    status: Option<String>,
    attachments: Vec<ContextAttachment>,
    recording: Option<SpeechRecordingAttachment>,
}

impl WorkspaceStore {
    pub fn new(library_root: &Path) -> Result<Self, String> {
        let root = library_root.join("workspace");
        fs::create_dir_all(root.join("chats")).map_err(|error| error.to_string())?;
        fs::create_dir_all(root.join("tasks")).map_err(|error| error.to_string())?;
        fs::create_dir_all(root.join("chat-index")).map_err(|error| error.to_string())?;
        fs::create_dir_all(root.join("task-index")).map_err(|error| error.to_string())?;
        let store = Self {
            root,
            write_lock: Arc::new(Mutex::new(())),
        };
        store.ensure_indices()?;
        store.recover_interrupted_tasks()?;
        Ok(store)
    }

    pub fn list_chats(&self) -> Result<Vec<ChatSessionSummary>, String> {
        let mut sessions =
            read_json_directory::<ChatSessionSummary>(&self.root.join("chat-index"))?;
        sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(sessions)
    }

    pub fn create_chat(&self, model_id: &str, first_message: &str) -> Result<ChatSession, String> {
        let now = chrono::Utc::now().to_rfc3339();
        let session = ChatSession {
            id: uuid::Uuid::new_v4().to_string(),
            title: title_from(first_message),
            model_id: model_id.to_string(),
            created_at: now.clone(),
            updated_at: now,
            messages: Vec::new(),
        };
        self.save_chat(&session)?;
        Ok(session)
    }

    pub fn get_chat(&self, id: &str) -> Result<ChatSession, String> {
        read_recoverable(&self.chat_path(id)?)
    }

    #[cfg(test)]
    pub fn add_chat_message(
        &self,
        id: &str,
        role: &str,
        content: String,
        reasoning: Option<String>,
    ) -> Result<ChatSession, String> {
        self.add_chat_message_with_status(id, role, content, reasoning, None)
    }

    pub fn add_chat_message_with_status(
        &self,
        id: &str,
        role: &str,
        content: String,
        reasoning: Option<String>,
        status: Option<String>,
    ) -> Result<ChatSession, String> {
        self.append_chat_message(
            id,
            ChatMessageDraft {
                role: role.to_string(),
                content,
                reasoning,
                status,
                attachments: Vec::new(),
                recording: None,
            },
        )
    }

    pub fn add_user_message_with_attachments(
        &self,
        id: &str,
        content: String,
        attachments: Vec<ContextAttachment>,
        recording: Option<SpeechRecordingAttachment>,
    ) -> Result<ChatSession, String> {
        self.append_chat_message(
            id,
            ChatMessageDraft {
                role: "user".to_string(),
                content,
                reasoning: None,
                status: None,
                attachments,
                recording,
            },
        )
    }

    fn append_chat_message(
        &self,
        id: &str,
        draft: ChatMessageDraft,
    ) -> Result<ChatSession, String> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| "conversation store lock is unavailable".to_string())?;
        let mut session: ChatSession = read_recoverable(&self.chat_path(id)?)?;
        let now = chrono::Utc::now().to_rfc3339();
        session.messages.push(ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: draft.role,
            content: draft.content,
            reasoning: draft.reasoning,
            status: draft.status,
            attachments: draft.attachments,
            recording: draft.recording,
            created_at: now.clone(),
        });
        session.updated_at = now;
        atomic_json(&self.chat_path(id)?, &session)?;
        self.write_chat_summary(&session)?;
        Ok(session)
    }

    pub fn save_chat(&self, session: &ChatSession) -> Result<(), String> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| "conversation store lock is unavailable".to_string())?;
        atomic_json(&self.chat_path(&session.id)?, session)?;
        self.write_chat_summary(session)
    }

    pub fn delete_chat(&self, id: &str) -> Result<(), String> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| "conversation store lock is unavailable".to_string())?;
        let path = self.chat_path(id)?;
        if path.is_file() {
            let backup = path.with_extension("json.backup");
            if backup.is_file() {
                fs::remove_file(backup).map_err(|error| error.to_string())?;
            }
            let archived = self.root.join("chats").join(format!(
                "{id}.archived-{}-{}.json",
                chrono::Utc::now().timestamp(),
                uuid::Uuid::new_v4()
            ));
            fs::rename(&path, archived).map_err(|error| error.to_string())?;
        }
        let index = self.chat_index_path(id)?;
        if index.is_file() {
            fs::remove_file(index).map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn create_task(
        &self,
        model_id: &str,
        objective: &str,
        access: ComputerTaskAccess,
    ) -> Result<ComputerTaskRun, String> {
        self.create_task_with_attachments(model_id, objective, access, Vec::new())
    }

    pub fn create_task_with_attachments(
        &self,
        model_id: &str,
        objective: &str,
        access: ComputerTaskAccess,
        attachments: Vec<ContextAttachment>,
    ) -> Result<ComputerTaskRun, String> {
        let now = chrono::Utc::now().to_rfc3339();
        let run = ComputerTaskRun {
            id: uuid::Uuid::new_v4().to_string(),
            objective: objective.trim().to_string(),
            model_id: model_id.to_string(),
            access,
            status: "starting".into(),
            created_at: now.clone(),
            updated_at: now,
            events: Vec::new(),
            artifacts: Vec::new(),
            attachments,
        };
        self.save_task(&run)?;
        Ok(run)
    }

    pub fn list_tasks(&self) -> Result<Vec<ComputerTaskSummary>, String> {
        let mut tasks = read_json_directory::<ComputerTaskSummary>(&self.root.join("task-index"))?;
        tasks.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(tasks)
    }

    pub fn get_task(&self, id: &str) -> Result<ComputerTaskRun, String> {
        read_recoverable(&self.task_path(id)?)
    }

    pub fn add_task_event(&self, event: ComputerTaskEvent) -> Result<ComputerTaskRun, String> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| "task store lock is unavailable".to_string())?;
        let mut run: ComputerTaskRun = read_recoverable(&self.task_path(&event.run_id)?)?;
        run.updated_at = event.at.clone();
        match event.kind.as_str() {
            "done" => run.status = "completed".into(),
            "error" | "limit" => run.status = "failed".into(),
            "cancelled" => run.status = "cancelled".into(),
            "question" => run.status = "waiting".into(),
            _ => run.status = "running".into(),
        }
        if event.kind == "artifact" {
            if let Some(path) = event
                .data
                .as_ref()
                .and_then(|data| data.get("path"))
                .and_then(serde_json::Value::as_str)
            {
                if !run.artifacts.iter().any(|known| known == path) {
                    run.artifacts.push(path.to_string());
                }
            }
        }
        run.events.push(event);
        atomic_json(&self.task_path(&run.id)?, &run)?;
        self.write_task_summary(&run)?;
        Ok(run)
    }

    fn save_task(&self, run: &ComputerTaskRun) -> Result<(), String> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| "task store lock is unavailable".to_string())?;
        atomic_json(&self.task_path(&run.id)?, run)?;
        self.write_task_summary(run)
    }

    fn chat_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.root.join("chats").join(format!("{id}.json")))
    }

    fn task_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.root.join("tasks").join(format!("{id}.json")))
    }

    fn chat_index_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.root.join("chat-index").join(format!("{id}.json")))
    }

    fn task_index_path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.root.join("task-index").join(format!("{id}.json")))
    }

    fn write_chat_summary(&self, session: &ChatSession) -> Result<(), String> {
        atomic_json(
            &self.chat_index_path(&session.id)?,
            &ChatSessionSummary {
                id: session.id.clone(),
                title: session.title.clone(),
                model_id: session.model_id.clone(),
                updated_at: session.updated_at.clone(),
                message_count: session.messages.len(),
            },
        )
    }

    fn write_task_summary(&self, run: &ComputerTaskRun) -> Result<(), String> {
        atomic_json(
            &self.task_index_path(&run.id)?,
            &ComputerTaskSummary {
                id: run.id.clone(),
                objective: run.objective.clone(),
                model_id: run.model_id.clone(),
                access: run.access,
                status: run.status.clone(),
                updated_at: run.updated_at.clone(),
                event_count: run.events.len(),
                artifact_count: run.artifacts.len(),
            },
        )
    }

    fn ensure_indices(&self) -> Result<(), String> {
        for path in json_paths(&self.root.join("chats"))? {
            let Some(id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if uuid::Uuid::parse_str(id).is_ok() && !self.chat_index_path(id)?.is_file() {
                if let Ok(session) = read_recoverable::<ChatSession>(&path) {
                    self.write_chat_summary(&session)?;
                }
            }
        }
        for path in json_paths(&self.root.join("tasks"))? {
            let Some(id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if uuid::Uuid::parse_str(id).is_ok() && !self.task_index_path(id)?.is_file() {
                if let Ok(run) = read_recoverable::<ComputerTaskRun>(&path) {
                    self.write_task_summary(&run)?;
                }
            }
        }
        Ok(())
    }

    fn recover_interrupted_tasks(&self) -> Result<(), String> {
        for summary in read_json_directory::<ComputerTaskSummary>(&self.root.join("task-index"))? {
            if matches!(summary.status.as_str(), "starting" | "running") {
                let Ok(mut run) = self.get_task(&summary.id) else {
                    continue;
                };
                let now = chrono::Utc::now().to_rfc3339();
                run.status = "interrupted".into();
                run.updated_at = now.clone();
                run.events.push(ComputerTaskEvent {
                    run_id: run.id.clone(),
                    step: 0,
                    kind: "interrupted".into(),
                    title: "Kestrel restarted".into(),
                    detail: "The previous process ended before this task recorded completion. No action was resumed automatically.".into(),
                    data: None,
                    at: now,
                });
                atomic_json(&self.task_path(&run.id)?, &run)?;
                self.write_task_summary(&run)?;
            }
        }
        Ok(())
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id)
        .map(|_| ())
        .map_err(|_| "invalid local record id".to_string())
}

fn title_from(message: &str) -> String {
    let title = message
        .split_whitespace()
        .take(10)
        .collect::<Vec<_>>()
        .join(" ");
    if title.chars().count() > 72 {
        format!("{}…", title.chars().take(71).collect::<String>())
    } else if title.is_empty() {
        "New conversation".into()
    } else {
        title
    }
}

fn atomic_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.backup");
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    if path.is_file() {
        fs::copy(path, &backup).map_err(|error| error.to_string())?;
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.is_file() {
            let _ = fs::copy(&backup, path);
        }
        return Err(error.to_string());
    }
    Ok(())
}

fn read_recoverable<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let primary = match fs::read(path) {
        Ok(primary) => primary,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return read_backup_and_restore(path).map_err(|backup_error| {
                if path.with_extension("json.backup").is_file() {
                    backup_error
                } else {
                    error.to_string()
                }
            });
        }
        Err(error) => return Err(error.to_string()),
    };
    match serde_json::from_slice(&primary) {
        Ok(value) => Ok(value),
        Err(primary_error) => read_backup_and_restore(path).map_err(|_| primary_error.to_string()),
    }
}

fn read_backup_and_restore<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let backup = path.with_extension("json.backup");
    let bytes = fs::read(&backup).map_err(|error| error.to_string())?;
    let value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    fs::copy(backup, path).map_err(|error| error.to_string())?;
    Ok(value)
}

fn read_json_directory<T: serde::de::DeserializeOwned>(directory: &Path) -> Result<Vec<T>, String> {
    let mut values = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("json")
            && !path.to_string_lossy().contains(".archived-")
        {
            if let Ok(value) = read_recoverable(&path) {
                values.push(value);
            }
        }
    }
    Ok(values)
}

fn json_paths(directory: &Path) -> Result<Vec<PathBuf>, String> {
    Ok(fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversations_survive_reopen_and_recover() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let session = store
            .create_chat("model", "A durable local conversation")
            .unwrap();
        store
            .add_chat_message(&session.id, "user", "hello".into(), None)
            .unwrap();
        assert_eq!(store.list_chats().unwrap()[0].message_count, 1);
        let path = directory
            .path()
            .join("workspace")
            .join("chats")
            .join(format!("{}.json", session.id));
        fs::write(&path, b"broken").unwrap();
        // Atomic saves retain one previous generation, so recovery returns the pre-message session.
        assert_eq!(store.get_chat(&session.id).unwrap().messages.len(), 0);
    }

    #[test]
    fn missing_primary_is_restored_from_recovery_copy() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let session = store.create_chat("model", "recover me").unwrap();
        store
            .add_chat_message(&session.id, "user", "hello".into(), None)
            .unwrap();
        let path = directory
            .path()
            .join("workspace")
            .join("chats")
            .join(format!("{}.json", session.id));
        fs::remove_file(&path).unwrap();
        let recovered = store.get_chat(&session.id).unwrap();
        assert_eq!(recovered.id, session.id);
        assert_eq!(recovered.title, "recover me");
        assert!(path.is_file());
    }

    #[test]
    fn deleting_chat_removes_index_and_retains_unique_archive() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let session = store.create_chat("model", "archive me").unwrap();
        store
            .add_chat_message(&session.id, "user", "keep this".into(), None)
            .unwrap();
        store.delete_chat(&session.id).unwrap();
        assert!(store.list_chats().unwrap().is_empty());

        let chat_dir = directory.path().join("workspace").join("chats");
        let archives = json_paths(&chat_dir)
            .unwrap()
            .into_iter()
            .filter(|path| path.to_string_lossy().contains(".archived-"))
            .collect::<Vec<_>>();
        assert_eq!(archives.len(), 1);
        let archived: ChatSession = read_recoverable(&archives[0]).unwrap();
        assert_eq!(archived.id, session.id);
        assert_eq!(archived.messages[0].content, "keep this");
        let stem = archives[0]
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap();
        let suffix = stem
            .strip_prefix(&format!("{}.archived-", session.id))
            .unwrap();
        let (_, archive_id) = suffix.split_once('-').unwrap();
        assert!(uuid::Uuid::parse_str(archive_id).is_ok());
    }

    #[test]
    fn invalid_ids_cannot_escape_the_store() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        assert!(store.get_chat("..\\settings").is_err());
    }

    #[test]
    fn incomplete_tasks_are_marked_interrupted_instead_of_resumed() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let run = store
            .create_task("model", "make a file", ComputerTaskAccess::Workspace)
            .unwrap();
        drop(store);
        let reopened = WorkspaceStore::new(directory.path()).unwrap();
        let recovered = reopened.get_task(&run.id).unwrap();
        assert_eq!(recovered.status, "interrupted");
        assert_eq!(reopened.list_tasks().unwrap()[0].status, "interrupted");
    }

    #[test]
    fn clarification_questions_pause_a_task_durably() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let run = store
            .create_task("model", "organize files", ComputerTaskAccess::Workspace)
            .unwrap();
        let paused = store
            .add_task_event(ComputerTaskEvent {
                run_id: run.id.clone(),
                step: 1,
                kind: "question".into(),
                title: "Needs your input".into(),
                detail: "Which folder should be changed?".into(),
                data: Some(serde_json::json!({"options":["A","B"],"recommendedIndex":0})),
                at: chrono::Utc::now().to_rfc3339(),
            })
            .unwrap();
        assert_eq!(paused.status, "waiting");
        drop(store);
        let reopened = WorkspaceStore::new(directory.path()).unwrap();
        assert_eq!(reopened.get_task(&run.id).unwrap().status, "waiting");
    }

    #[test]
    fn interrupted_chat_status_survives_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let session = store.create_chat("model", "explain this").unwrap();
        store
            .add_chat_message_with_status(
                &session.id,
                "assistant",
                "partial".into(),
                None,
                Some("interrupted".into()),
            )
            .unwrap();
        drop(store);
        let reopened = WorkspaceStore::new(directory.path()).unwrap();
        assert_eq!(
            reopened.get_chat(&session.id).unwrap().messages[0]
                .status
                .as_deref(),
            Some("interrupted")
        );
    }

    #[test]
    fn attachment_references_survive_chat_and_task_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(directory.path()).unwrap();
        let attachment = ContextAttachment {
            id: "a".repeat(64),
            name: "evidence.pdf".into(),
            kind: "pdf".into(),
            mime_type: "application/pdf".into(),
            bytes: 42,
            sha256: "a".repeat(64),
            stored_path: "objects/evidence.pdf".into(),
            extracted_chars: 120,
            context_mode: "extracted_text".into(),
            note: "local".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let session = store.create_chat("model", "inspect evidence").unwrap();
        store
            .add_user_message_with_attachments(
                &session.id,
                "inspect".into(),
                vec![attachment.clone()],
                None,
            )
            .unwrap();
        let task = store
            .create_task_with_attachments(
                "model",
                "inspect",
                ComputerTaskAccess::Workspace,
                vec![attachment.clone()],
            )
            .unwrap();
        drop(store);

        let reopened = WorkspaceStore::new(directory.path()).unwrap();
        assert_eq!(
            reopened.get_chat(&session.id).unwrap().messages[0].attachments,
            vec![attachment.clone()]
        );
        assert_eq!(
            reopened.get_task(&task.id).unwrap().attachments,
            vec![attachment]
        );
    }
}
