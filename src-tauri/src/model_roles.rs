//! Durable model-role compatibility and local protocol qualification.
//!
//! Loading a GGUF proves only that llama.cpp can read it. Kestrel records a separate, recoverable
//! qualification receipt after a producer explicitly asks a model to demonstrate structured tool
//! use. Receipts are bound to the path-independent model identity, engine bytes, runtime profile,
//! and Studio protocol revision so an upgrade cannot silently inherit an obsolete result.

use crate::model::ModelInfo;
use crate::models::ControlSettings;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub const STUDIO_PROTOCOL_REVISION: &str = "kestrel-studio-model-role-v2";
const STORE_SCHEMA_VERSION: u32 = 1;
const MAX_STORE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelQualificationReceipt {
    pub model_id: String,
    pub model_name: String,
    pub protocol_revision: String,
    pub engine_sha256: String,
    pub context_window: u32,
    pub max_output_tokens: u32,
    pub passed: bool,
    pub checks: Vec<String>,
    pub detail: String,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelCompatibility {
    pub model_id: String,
    pub model_name: String,
    pub tier: String,
    pub studio_ready: bool,
    pub requires_qualification: bool,
    pub detail: String,
    pub protocol_revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<ModelQualificationReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct QualificationLedger {
    schema_version: u32,
    receipts: Vec<ModelQualificationReceipt>,
}

#[derive(Debug, Clone)]
pub struct ModelQualificationStore {
    path: PathBuf,
    receipts: Arc<Mutex<Vec<ModelQualificationReceipt>>>,
}

impl ModelQualificationStore {
    pub fn new(library_root: &Path) -> Result<Self, String> {
        let path = library_root.join("model-qualifications.json");
        let receipts = read_ledger(&path)?;
        Ok(Self {
            path,
            receipts: Arc::new(Mutex::new(receipts)),
        })
    }

    pub fn record(&self, receipt: ModelQualificationReceipt) -> Result<(), String> {
        let mut receipts = self
            .receipts
            .lock()
            .map_err(|_| "model qualification store is unavailable".to_string())?;
        receipts.retain(|known| known.model_id != receipt.model_id);
        receipts.push(receipt);
        receipts.sort_by(|left, right| left.model_name.cmp(&right.model_name));
        write_ledger(&self.path, &receipts)
    }

    pub fn receipt(&self, model_id: &str) -> Result<Option<ModelQualificationReceipt>, String> {
        Ok(self
            .receipts
            .lock()
            .map_err(|_| "model qualification store is unavailable".to_string())?
            .iter()
            .find(|receipt| receipt.model_id == model_id)
            .cloned())
    }

    pub fn assess(
        &self,
        model: &ModelInfo,
        settings: &ControlSettings,
    ) -> Result<ModelCompatibility, String> {
        let receipt = self.receipt(&model.id)?;
        let current_engine = engine_identity(Path::new(&settings.engine_path))?;
        assess_model(model, settings, receipt, &current_engine)
    }

    pub fn assess_all(
        &self,
        models: &[ModelInfo],
        settings: &ControlSettings,
    ) -> Result<Vec<ModelCompatibility>, String> {
        let current_engine = engine_identity(Path::new(&settings.engine_path))?;
        models
            .iter()
            .map(|model| {
                let receipt = self.receipt(&model.id)?;
                assess_model(model, settings, receipt, &current_engine)
            })
            .collect()
    }
}

pub fn qualification_receipt(
    model: &ModelInfo,
    settings: &ControlSettings,
    passed: bool,
    checks: Vec<String>,
    detail: String,
) -> Result<ModelQualificationReceipt, String> {
    Ok(ModelQualificationReceipt {
        model_id: model.id.clone(),
        model_name: model.name.clone(),
        protocol_revision: STUDIO_PROTOCOL_REVISION.into(),
        engine_sha256: engine_identity(Path::new(&settings.engine_path))?,
        context_window: settings.context_window,
        max_output_tokens: settings.max_output_tokens,
        passed,
        checks,
        detail,
        checked_at: Utc::now().to_rfc3339(),
    })
}

pub fn is_bonsai(model: &ModelInfo) -> bool {
    format!("{} {}", model.name, model.path)
        .to_ascii_lowercase()
        .contains("bonsai")
}

fn assess_model(
    model: &ModelInfo,
    settings: &ControlSettings,
    receipt: Option<ModelQualificationReceipt>,
    current_engine: &str,
) -> Result<ModelCompatibility, String> {
    let mut tier = "unverified";
    let mut studio_ready = false;
    let mut requires_qualification = true;
    let detail;
    if !model.chat_template {
        tier = "incompatible";
        requires_qualification = false;
        detail = "This GGUF does not advertise an embedded chat template, so Kestrel cannot construct reliable Studio turns.".into();
    } else if model.context_length.is_some_and(|context| context < 32_768) {
        tier = "limited-context";
        requires_qualification = false;
        detail = format!(
            "This model advertises only {} tokens of context; durable Studio planning requires at least 32,768.",
            model.context_length.unwrap_or_default()
        );
    } else if is_bonsai(model) {
        tier = "release-validated";
        studio_ready = true;
        requires_qualification = false;
        detail = "Bundled Kestrel baseline with full Studio acceptance coverage.".into();
    } else if let Some(known) = receipt.as_ref().filter(|known| known.passed) {
        if known.protocol_revision == STUDIO_PROTOCOL_REVISION
            && known.engine_sha256 == current_engine
            && known.context_window == settings.context_window
            && known.max_output_tokens == settings.max_output_tokens
        {
            tier = "protocol-ready";
            studio_ready = true;
            requires_qualification = false;
            detail = "This exact model and runtime profile passed Kestrel's local structured-tool protocol check.".into();
        } else {
            detail = "The model passed an older or different runtime profile. Run the local Studio check again before unattended planning.".into();
        }
    } else if let Some(known) = receipt.as_ref() {
        detail = format!(
            "The latest local Studio protocol check failed: {}",
            known.detail
        );
    } else {
        detail = "The model can be loaded for chat, but its structured Studio-agent behavior has not been checked locally.".into();
    }
    Ok(ModelCompatibility {
        model_id: model.id.clone(),
        model_name: model.name.clone(),
        tier: tier.into(),
        studio_ready,
        requires_qualification,
        detail,
        protocol_revision: STUDIO_PROTOCOL_REVISION.into(),
        receipt,
    })
}

fn engine_identity(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| {
        format!(
            "could not fingerprint the configured llama.cpp engine {}: {error}",
            path.display()
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn read_ledger(path: &Path) -> Result<Vec<ModelQualificationReceipt>, String> {
    enum ReadError {
        Oversized(String),
        Corrupt,
    }

    let backup = path.with_extension("json.backup");
    let read = |candidate: &Path| -> Result<QualificationLedger, ReadError> {
        if fs::metadata(candidate)
            .map_err(|_| ReadError::Corrupt)?
            .len()
            > MAX_STORE_BYTES
        {
            return Err(ReadError::Oversized(
                "model qualification store exceeds 2 MiB".into(),
            ));
        }
        serde_json::from_slice(&fs::read(candidate).map_err(|_| ReadError::Corrupt)?)
            .map_err(|_| ReadError::Corrupt)
    };
    let ledger = if path.is_file() {
        match read(path) {
            Ok(value) => Some(value),
            Err(ReadError::Oversized(error)) => return Err(error),
            Err(ReadError::Corrupt) if backup.is_file() => match read(&backup) {
                Ok(value) => {
                    fs::copy(&backup, path).map_err(|error| error.to_string())?;
                    Some(value)
                }
                Err(ReadError::Oversized(error)) => return Err(error),
                Err(ReadError::Corrupt) => {
                    quarantine(path);
                    quarantine(&backup);
                    None
                }
            },
            Err(ReadError::Corrupt) => {
                quarantine(path);
                None
            }
        }
    } else if backup.is_file() {
        match read(&backup) {
            Ok(value) => {
                fs::copy(&backup, path).map_err(|error| error.to_string())?;
                Some(value)
            }
            Err(ReadError::Oversized(error)) => return Err(error),
            Err(ReadError::Corrupt) => {
                quarantine(&backup);
                None
            }
        }
    } else {
        None
    };
    match ledger {
        Some(ledger) if ledger.schema_version == STORE_SCHEMA_VERSION => Ok(ledger.receipts),
        Some(_) => {
            quarantine(path);
            quarantine(&backup);
            Ok(Vec::new())
        }
        None => Ok(Vec::new()),
    }
}

fn quarantine(path: &Path) {
    if !path.is_file() {
        return;
    }
    let suffix = format!("corrupt-{}", Utc::now().timestamp_millis());
    let target = path.with_file_name(format!(
        "{}.{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("model-qualifications.json"),
        suffix
    ));
    let _ = fs::rename(path, target);
}

fn write_ledger(path: &Path, receipts: &[ModelQualificationReceipt]) -> Result<(), String> {
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.backup");
    let bytes = serde_json::to_vec_pretty(&QualificationLedger {
        schema_version: STORE_SCHEMA_VERSION,
        receipts: receipts.to_vec(),
    })
    .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err("model qualification store exceeds 2 MiB".into());
    }
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
    if !backup.is_file() {
        fs::copy(path, &backup).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(root: &Path, name: &str, chat_template: bool) -> ModelInfo {
        ModelInfo {
            id: format!("id-{name}"),
            name: name.into(),
            path: root
                .join(format!("{name}.gguf"))
                .to_string_lossy()
                .into_owned(),
            source: "test".into(),
            bytes: 4,
            architecture: Some("qwen".into()),
            context_length: Some(131_072),
            chat_template,
            quantization: Some("Q4_K_M".into()),
            mmproj_path: None,
            supports_vision: false,
            supports_audio: false,
            recommendation: String::new(),
        }
    }

    fn settings(root: &Path) -> ControlSettings {
        let engine = root.join("llama-server.exe");
        fs::write(&engine, b"engine").unwrap();
        ControlSettings {
            engine_path: engine.to_string_lossy().into_owned(),
            ..ControlSettings::default()
        }
    }

    #[test]
    fn bonsai_is_release_validated_but_generic_models_require_a_matching_receipt() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let engine = engine_identity(Path::new(&settings.engine_path)).unwrap();
        let bonsai = model(temp.path(), "Ternary Bonsai 27B", true);
        assert_eq!(
            assess_model(&bonsai, &settings, None, &engine)
                .unwrap()
                .tier,
            "release-validated"
        );
        let generic = model(temp.path(), "Qwen", true);
        assert_eq!(
            assess_model(&generic, &settings, None, &engine)
                .unwrap()
                .tier,
            "unverified"
        );
        let receipt = qualification_receipt(
            &generic,
            &settings,
            true,
            vec!["tool-call".into()],
            "passed".into(),
        )
        .unwrap();
        assert_eq!(
            assess_model(&generic, &settings, Some(receipt), &engine)
                .unwrap()
                .tier,
            "protocol-ready"
        );
    }

    #[test]
    fn receipt_is_invalidated_by_engine_or_runtime_profile_changes() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = settings(temp.path());
        let generic = model(temp.path(), "Qwen", true);
        let receipt =
            qualification_receipt(&generic, &settings, true, vec![], "passed".into()).unwrap();
        settings.context_window += 1;
        let engine = engine_identity(Path::new(&settings.engine_path)).unwrap();
        assert_eq!(
            assess_model(&generic, &settings, Some(receipt), &engine)
                .unwrap()
                .tier,
            "unverified"
        );
    }

    #[test]
    fn store_recovers_its_backup() {
        let temp = tempfile::tempdir().unwrap();
        let store = ModelQualificationStore::new(temp.path()).unwrap();
        let settings = settings(temp.path());
        let receipt = qualification_receipt(
            &model(temp.path(), "Qwen", true),
            &settings,
            true,
            vec![],
            "passed".into(),
        )
        .unwrap();
        store.record(receipt.clone()).unwrap();
        fs::copy(&store.path, store.path.with_extension("json.backup")).unwrap();
        fs::write(&store.path, b"invalid").unwrap();
        let recovered = ModelQualificationStore::new(temp.path()).unwrap();
        assert_eq!(recovered.receipt(&receipt.model_id).unwrap(), Some(receipt));
    }

    #[test]
    fn corrupt_rebuildable_store_never_blocks_startup() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model-qualifications.json");
        fs::write(&path, b"invalid").unwrap();
        fs::write(path.with_extension("json.backup"), b"also invalid").unwrap();

        let recovered = ModelQualificationStore::new(temp.path()).unwrap();

        assert!(recovered.receipts.lock().unwrap().is_empty());
        assert!(!path.is_file());
    }

    #[test]
    fn oversized_store_is_reported_without_quarantine() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model-qualifications.json");
        fs::write(&path, vec![b' '; MAX_STORE_BYTES as usize + 1]).unwrap();

        let error = match ModelQualificationStore::new(temp.path()) {
            Ok(_) => panic!("oversized qualification store should be rejected"),
            Err(error) => error,
        };

        assert!(error.contains("exceeds 2 MiB"));
        assert!(path.is_file());
        assert!(fs::read_dir(temp.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("corrupt-")
        }));
    }
}
