use super::{write_json_atomic, StudioError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, path::Path};

const MAX_DIRECTIONS: usize = 32;
const MAX_DIRECTION_CHARS: usize = 16_000;
const MAX_ADVANCED_DOCUMENT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerDirection {
    pub id: String,
    pub created_at: String,
    pub text: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlanningControl {
    #[serde(default)]
    pub checkpoint_requested: bool,
    #[serde(default)]
    pub pending_directions: Vec<ProducerDirection>,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoviePlanningEvent {
    pub project_id: String,
    pub sequence: u64,
    pub kind: String,
    pub stage: String,
    pub text: String,
    pub session: u32,
    pub step: u32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptDocument {
    pub id: String,
    pub title: String,
    pub category: String,
    pub content: String,
}

impl PromptDocument {
    pub(super) fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        category: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            category: category.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoviePlanningSnapshot {
    pub project_id: String,
    pub checkpoint_requested: bool,
    pub pending_directions: Vec<ProducerDirection>,
    pub prompt_documents: Vec<PromptDocument>,
    pub tool_schema: Value,
    pub last_request: Value,
    pub transcript: Value,
    pub current_text: String,
}

pub(super) fn load_control(path: &Path) -> Result<PlanningControl, StudioError> {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(value) => Ok(value),
            Err(primary_error) => {
                let recovery = control_recovery_path(path);
                fs::read(&recovery)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                    .ok_or_else(|| {
                        StudioError::Invalid(format!(
                            "planning controls are damaged and no recovery copy is readable: {primary_error}"
                        ))
                    })
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(PlanningControl::default())
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn save_control(path: &Path, control: &mut PlanningControl) -> Result<(), StudioError> {
    control.updated_at = Utc::now().to_rfc3339();
    if path.is_file() {
        fs::copy(path, control_recovery_path(path))?;
    }
    write_json_atomic(path, control)
}

pub(super) fn add_direction(
    path: &Path,
    text: &str,
) -> Result<(PlanningControl, ProducerDirection), StudioError> {
    let text = text.trim();
    let count = text.chars().count();
    if !(3..=MAX_DIRECTION_CHARS).contains(&count) {
        return Err(StudioError::Invalid(
            "producer direction must contain 3 to 16,000 characters".into(),
        ));
    }
    let mut control = load_control(path)?;
    if control.pending_directions.len() >= MAX_DIRECTIONS {
        return Err(StudioError::Invalid(
            "32 producer directions are already waiting; let Bonsai consume them before adding more"
                .into(),
        ));
    }
    let direction = ProducerDirection {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: Utc::now().to_rfc3339(),
        text: text.into(),
    };
    control.pending_directions.push(direction.clone());
    save_control(path, &mut control)?;
    Ok((control, direction))
}

pub(super) fn request_checkpoint(path: &Path) -> Result<PlanningControl, StudioError> {
    let mut control = load_control(path)?;
    control.checkpoint_requested = true;
    save_control(path, &mut control)?;
    Ok(control)
}

pub(super) fn take_pending(path: &Path) -> Result<PlanningControl, StudioError> {
    let mut control = load_control(path)?;
    let taken = control.clone();
    if !control.checkpoint_requested && control.pending_directions.is_empty() {
        return Ok(taken);
    }
    control.pending_directions.clear();
    control.checkpoint_requested = false;
    save_control(path, &mut control)?;
    Ok(taken)
}

fn control_recovery_path(path: &Path) -> std::path::PathBuf {
    path.with_extension("previous.json")
}

pub(super) fn read_advanced_json(path: &Path) -> Result<Value, StudioError> {
    let metadata = match fs::metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({"messages": []}))
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_ADVANCED_DOCUMENT_BYTES {
        return Ok(json!({
            "unavailable": true,
            "reason": "The exact transcript is larger than the 8 MiB UI inspection limit. Open the durable agent-workspace file from the project folder."
        }));
    }
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(StudioError::from)
}

pub(super) fn read_prompt_document(
    path: &Path,
    id: &str,
    title: &str,
    category: &str,
) -> Result<Option<PromptDocument>, StudioError> {
    let metadata = match fs::metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_ADVANCED_DOCUMENT_BYTES {
        return Ok(Some(PromptDocument::new(
            id,
            title,
            category,
            "This document exceeds the 8 MiB UI inspection limit. Open it from the durable project folder.",
        )));
    }
    let content = fs::read_to_string(path).map_err(|error| {
        StudioError::Invalid(format!("cannot inspect {}: {error}", path.display()))
    })?;
    Ok(Some(PromptDocument::new(id, title, category, content)))
}

pub(super) fn latest_assistant_text(transcript: &Value) -> String {
    transcript
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            messages.iter().rev().find_map(|message| {
                (message.get("role").and_then(Value::as_str) == Some("assistant"))
                    .then(|| message.get("content").and_then(Value::as_str))
                    .flatten()
            })
        })
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_control() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "kestrel-planning-control-{}.json",
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn direction_and_checkpoint_controls_are_durable_and_consumed_once() {
        let path = temporary_control();
        let (_, direction) =
            add_direction(&path, "Keep the ending quiet and observational.").unwrap();
        request_checkpoint(&path).unwrap();

        let restored = load_control(&path).unwrap();
        assert!(restored.checkpoint_requested);
        assert_eq!(restored.pending_directions[0].id, direction.id);

        let taken = take_pending(&path).unwrap();
        assert!(taken.checkpoint_requested);
        assert_eq!(taken.pending_directions.len(), 1);
        let cleared = load_control(&path).unwrap();
        assert!(!cleared.checkpoint_requested);
        assert!(cleared.pending_directions.is_empty());
        fs::remove_file(&path).unwrap();
        fs::remove_file(control_recovery_path(&path)).unwrap();
    }

    #[test]
    fn latest_text_uses_the_last_assistant_message() {
        let transcript = json!({"messages":[
            {"role":"assistant","content":"first"},
            {"role":"tool","content":"check output"},
            {"role":"assistant","content":"current producer-facing text"}
        ]});
        assert_eq!(
            latest_assistant_text(&transcript),
            "current producer-facing text"
        );
    }

    #[test]
    fn direction_length_is_bounded() {
        let path = temporary_control();
        assert!(add_direction(&path, "no").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn corrupt_control_recovers_the_previous_durable_copy() {
        let path = temporary_control();
        add_direction(&path, "Keep the camera at the child's eye level.").unwrap();
        request_checkpoint(&path).unwrap();
        fs::write(&path, b"{broken").unwrap();
        let recovered = load_control(&path).unwrap();
        assert_eq!(recovered.pending_directions.len(), 1);
        fs::remove_file(&path).unwrap();
        fs::remove_file(control_recovery_path(&path)).unwrap();
    }
}
