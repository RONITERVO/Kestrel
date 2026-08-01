//! Portable, deliberately non-secret setup profiles.
//!
//! Profiles carry model identities and tuning, not weights, chat/research content, credentials,
//! developer paths, or a Full Access unlock. Import merges those safe fields into local settings
//! and lets native engine discovery repair machine-specific executable paths.

use crate::{
    config::{ControlSettingsStore, SettingsStore},
    model::ModelInfo,
    models::{ControlSettings, ProfileTransfer, ResearchSettings},
    runtime,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
use thiserror::Error;

const PROFILE_SCHEMA_VERSION: u32 = 1;
const MAX_PROFILE_BYTES: u64 = 1024 * 1024;
const MAX_CONTEXT_WINDOW: u32 = 1_048_576;
const MAX_OUTPUT_TOKENS: u32 = 262_144;
const MAX_THREADS: u32 = 256;
const MAX_AGENT_STEPS: u32 = 1_000;
const MAX_RESEARCH_LANES: u32 = 256;
const MAX_RESULTS_PER_LANE: u32 = 1_000;
const MAX_SOURCE_TARGET: u32 = 10_000;
const MAX_TOOL_TURNS: u32 = 1_000;
const MAX_SOURCE_CHARS: u32 = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("profile file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("profile JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("unsupported Kestrel profile version: {0}")]
    UnsupportedVersion(u32),
    #[error("profile must be a JSON file no larger than 1 MiB")]
    InvalidFile,
    #[error("profile settings are invalid: {0}")]
    Settings(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableProfile {
    schema_version: u32,
    created_at: String,
    app_version: String,
    research: ResearchSettings,
    control: PortableControl,
    models: Vec<PortableModel>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableControl {
    advanced_mode: bool,
    engine_hint: String,
    extra_model_roots: Vec<String>,
    selected_model_id: Option<String>,
    context_window: u32,
    max_output_tokens: u32,
    threads: u32,
    agent_max_steps: u32,
    agent_max_output_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PortableModel {
    id: String,
    name: String,
    bytes: u64,
}

pub struct ImportedProfile {
    pub research: ResearchSettings,
    pub control: ControlSettings,
}

pub fn export(
    library_root: &Path,
    research: &ResearchSettings,
    control: &ControlSettings,
    models: &[ModelInfo],
) -> Result<ProfileTransfer, ProfileError> {
    let directory = library_root.join("setup-profiles");
    fs::create_dir_all(&directory)?;
    let unique = uuid::Uuid::new_v4().simple().to_string();
    let path = directory.join(format!(
        "kestrel-profile-{}-{}.json",
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        &unique[..8]
    ));
    let mut portable_research = research.clone();
    portable_research.bonsai_root = portable_path(&portable_research.bonsai_root);
    let pinned_model = control
        .selected_model_id
        .as_deref()
        .and_then(|id| models.iter().find(|model| model.id == id))
        .or_else(|| {
            models.iter().find(|model| {
                format!("{} {}", model.name, model.path)
                    .to_lowercase()
                    .contains("bonsai")
            })
        });
    let profile = PortableProfile {
        schema_version: PROFILE_SCHEMA_VERSION,
        created_at: Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        research: portable_research,
        control: PortableControl {
            advanced_mode: control.advanced_mode,
            engine_hint: portable_path(&control.engine_path),
            extra_model_roots: control
                .extra_model_roots
                .iter()
                .map(|path| portable_path(path))
                .collect(),
            selected_model_id: pinned_model.map(|model| model.id.clone()),
            context_window: control.context_window,
            max_output_tokens: control.max_output_tokens,
            threads: control.threads,
            agent_max_steps: control.agent_max_steps,
            agent_max_output_tokens: control.agent_max_output_tokens,
        },
        models: pinned_model
            .into_iter()
            .map(|model| PortableModel {
                id: model.id.clone(),
                name: model.name.clone(),
                bytes: model.bytes,
            })
            .collect(),
    };
    write_new(&path, &serde_json::to_vec_pretty(&profile)?)?;
    Ok(ProfileTransfer {
        path: path.to_string_lossy().into_owned(),
        message: "Portable setup exported without weights, secrets, chats, research, developer paths, or Full Access authority.".into(),
    })
}

pub fn import(
    source: &Path,
    research_store: &SettingsStore,
    control_store: &ControlSettingsStore,
) -> Result<ImportedProfile, ProfileError> {
    let metadata = fs::metadata(source)?;
    if !metadata.is_file()
        || metadata.len() > MAX_PROFILE_BYTES
        || !source
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("json"))
    {
        return Err(ProfileError::InvalidFile);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(source)?
        .take(MAX_PROFILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(ProfileError::InvalidFile);
    }
    let profile: PortableProfile = serde_json::from_slice(&bytes)?;
    if profile.schema_version != PROFILE_SCHEMA_VERSION {
        return Err(ProfileError::UnsupportedVersion(profile.schema_version));
    }
    validate_tuning(&profile)?;

    let current_research = research_store
        .load()
        .map_err(|error| ProfileError::Settings(error.to_string()))?;
    let imported_bonsai = localize_path(&profile.research.bonsai_root);
    let mut research = profile.research;
    research.bonsai_root = if valid_bonsai_root(&imported_bonsai) {
        imported_bonsai
    } else {
        current_research.bonsai_root.clone()
    };

    let mut control = control_store
        .load()
        .map_err(|error| ProfileError::Settings(error.to_string()))?;
    control.advanced_mode = profile.control.advanced_mode;
    control.extra_model_roots = profile
        .control
        .extra_model_roots
        .iter()
        .map(|path| localize_path(path))
        .filter(|path| Path::new(path).is_dir())
        .collect();
    control.selected_model_id = profile.control.selected_model_id;
    control.context_window = profile.control.context_window;
    control.max_output_tokens = profile.control.max_output_tokens;
    control.threads = profile.control.threads;
    control.agent_max_steps = profile.control.agent_max_steps;
    control.agent_max_output_tokens = profile.control.agent_max_output_tokens;
    control.allow_full_access_agent = false;

    let engine_hint = localize_path(&profile.control.engine_hint);
    let candidates = runtime::detect_engines(&engine_hint, &research.bonsai_root);
    if let Some(candidate) = candidates.first() {
        control.engine_path.clone_from(&candidate.path);
    }

    research_store
        .save(&research)
        .map_err(|error| ProfileError::Settings(error.to_string()))?;
    if let Err(error) = control_store.save(&control) {
        let rollback = research_store.save(&current_research);
        return Err(ProfileError::Settings(match rollback {
            Ok(()) => format!("{error}; research settings were restored"),
            Err(rollback_error) => {
                format!("{error}; research settings rollback also failed: {rollback_error}")
            }
        }));
    }
    Ok(ImportedProfile { research, control })
}

fn valid_bonsai_root(value: &str) -> bool {
    let root = Path::new(value);
    root.join("settings.json").is_file()
        || root.join("runtime").join("llama-server.exe").is_file()
        || root.join("models").is_dir()
}

fn validate_tuning(profile: &PortableProfile) -> Result<(), ProfileError> {
    for (value, limit, field) in [
        (
            profile.research.context_window,
            MAX_CONTEXT_WINDOW,
            "research.contextWindow",
        ),
        (
            profile.research.max_output_tokens,
            MAX_OUTPUT_TOKENS,
            "research.maxOutputTokens",
        ),
        (
            profile.research.research_lanes,
            MAX_RESEARCH_LANES,
            "research.researchLanes",
        ),
        (
            profile.research.results_per_lane,
            MAX_RESULTS_PER_LANE,
            "research.resultsPerLane",
        ),
        (
            profile.research.source_target,
            MAX_SOURCE_TARGET,
            "research.sourceTarget",
        ),
        (
            profile.research.tool_turns,
            MAX_TOOL_TURNS,
            "research.toolTurns",
        ),
        (
            profile.research.thinking_budget,
            MAX_OUTPUT_TOKENS,
            "research.thinkingBudget",
        ),
        (
            profile.research.max_source_chars,
            MAX_SOURCE_CHARS,
            "research.maxSourceChars",
        ),
        (
            profile.control.context_window,
            MAX_CONTEXT_WINDOW,
            "control.contextWindow",
        ),
        (
            profile.control.max_output_tokens,
            MAX_OUTPUT_TOKENS,
            "control.maxOutputTokens",
        ),
        (profile.control.threads, MAX_THREADS, "control.threads"),
        (
            profile.control.agent_max_steps,
            MAX_AGENT_STEPS,
            "control.agentMaxSteps",
        ),
        (
            profile.control.agent_max_output_tokens,
            MAX_OUTPUT_TOKENS,
            "control.agentMaxOutputTokens",
        ),
    ] {
        if value == 0 || value > limit {
            return Err(ProfileError::Settings(format!(
                "{field} must be between 1 and {limit}, found {value}"
            )));
        }
    }
    Ok(())
}

fn portable_path(value: &str) -> String {
    if let Some(base) = directories::BaseDirs::new() {
        if let Ok(relative) = Path::new(value).strip_prefix(base.home_dir()) {
            return PathBuf::from("{HOME}")
                .join(relative)
                .to_string_lossy()
                .into_owned();
        }
    }
    value.into()
}

fn localize_path(value: &str) -> String {
    if let Some(relative) = value.strip_prefix("{HOME}") {
        if let Some(base) = directories::BaseDirs::new() {
            return base
                .home_dir()
                .join(relative.trim_start_matches(['\\', '/']))
                .to_string_lossy()
                .into_owned();
        }
    }
    value.into()
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_profile_never_restores_full_access_or_developer_paths() {
        let directory = tempfile::tempdir().unwrap();
        let research_store = SettingsStore::new(directory.path());
        let control_store = ControlSettingsStore::new(directory.path());
        let research = ResearchSettings {
            bonsai_root: directory.path().to_string_lossy().into_owned(),
            ..ResearchSettings::default()
        };
        research_store.save(&research).unwrap();
        let control = ControlSettings {
            project_root: r"C:\private\repository".into(),
            allow_full_access_agent: true,
            ..ControlSettings::default()
        };
        control_store.save(&control).unwrap();
        let transfer = export(directory.path(), &research, &control, &[]).unwrap();

        let imported = import(Path::new(&transfer.path), &research_store, &control_store).unwrap();

        assert!(!imported.control.allow_full_access_agent);
        assert_eq!(imported.control.project_root, r"C:\private\repository");
        let exported = fs::read_to_string(transfer.path).unwrap();
        assert!(!exported.contains("private\\repository"));
        assert!(!exported.contains("allowFullAccessAgent"));
    }

    #[test]
    fn rejects_unknown_or_oversized_profile_formats() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.txt");
        fs::write(&path, b"{}").unwrap();
        let result = import(
            &path,
            &SettingsStore::new(directory.path()),
            &ControlSettingsStore::new(directory.path()),
        );
        assert!(matches!(result, Err(ProfileError::InvalidFile)));
    }

    #[test]
    fn imported_engine_hint_cannot_select_an_unrelated_program() {
        let directory = tempfile::tempdir().unwrap();
        let research_store = SettingsStore::new(directory.path());
        let control_store = ControlSettingsStore::new(directory.path());
        let research = ResearchSettings::default();
        let safe = ControlSettings::default();
        research_store.save(&research).unwrap();
        control_store.save(&safe).unwrap();
        let unrelated = directory.path().join("program.exe");
        fs::write(&unrelated, b"unrelated").unwrap();
        let exported = ControlSettings {
            engine_path: unrelated.to_string_lossy().into_owned(),
            ..safe.clone()
        };
        let transfer = export(directory.path(), &research, &exported, &[]).unwrap();

        let imported = import(Path::new(&transfer.path), &research_store, &control_store).unwrap();

        assert_ne!(
            imported.control.engine_path,
            unrelated.to_string_lossy().into_owned()
        );
    }

    #[test]
    fn export_pins_bonsai_when_no_model_was_explicitly_selected() {
        let directory = tempfile::tempdir().unwrap();
        let model = ModelInfo {
            id: "bonsai-signature".into(),
            name: "Ternary Bonsai 27B".into(),
            path: r"D:\LocalAI\Bonsai27B\models\bonsai.gguf".into(),
            source: "LocalAI".into(),
            bytes: 42,
            architecture: None,
            context_length: None,
            chat_template: true,
            quantization: None,
            mmproj_path: None,
            recommendation: "Bonsai".into(),
        };
        let transfer = export(
            directory.path(),
            &ResearchSettings::default(),
            &ControlSettings::default(),
            &[model],
        )
        .unwrap();
        let profile: PortableProfile =
            serde_json::from_slice(&fs::read(transfer.path).unwrap()).unwrap();

        assert_eq!(
            profile.control.selected_model_id.as_deref(),
            Some("bonsai-signature")
        );
        assert_eq!(profile.models.len(), 1);
    }

    #[test]
    fn import_rejects_oversized_tuning_before_persistence() {
        let directory = tempfile::tempdir().unwrap();
        let research_store = SettingsStore::new(directory.path());
        let control_store = ControlSettingsStore::new(directory.path());
        let research = ResearchSettings::default();
        let control = ControlSettings::default();
        research_store.save(&research).unwrap();
        control_store.save(&control).unwrap();
        let transfer = export(directory.path(), &research, &control, &[]).unwrap();
        let mut profile: PortableProfile =
            serde_json::from_slice(&fs::read(&transfer.path).unwrap()).unwrap();
        profile.control.threads = u32::MAX;
        fs::write(&transfer.path, serde_json::to_vec_pretty(&profile).unwrap()).unwrap();

        assert!(matches!(
            import(Path::new(&transfer.path), &research_store, &control_store),
            Err(ProfileError::Settings(_))
        ));
        assert_eq!(control_store.load().unwrap().threads, control.threads);
    }
}
