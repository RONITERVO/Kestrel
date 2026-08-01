use crate::models::{ControlSettings, ResearchSettings};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("research settings file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("local settings JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("advanced values must be positive integers")]
    InvalidAdvancedValue,
    #[error("Bonsai root does not contain settings.json: {0}")]
    MissingBonsaiSettings(String),
}

#[derive(Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

#[derive(Clone)]
pub struct ControlSettingsStore {
    path: PathBuf,
}

impl ControlSettingsStore {
    pub fn new(library_root: &Path) -> Self {
        Self {
            path: library_root.join("control-settings.json"),
        }
    }

    pub fn load(&self) -> Result<ControlSettings, ConfigError> {
        if let Some(settings) = recoverable_json(&self.path)? {
            return Ok(settings);
        }
        Ok(legacy_control_settings().unwrap_or_default())
    }

    pub fn save(&self, settings: &ControlSettings) -> Result<(), ConfigError> {
        if settings.context_window == 0 || settings.max_output_tokens == 0 || settings.threads == 0
        {
            return Err(ConfigError::InvalidAdvancedValue);
        }
        atomic_json_write(&self.path, &serde_json::to_vec_pretty(settings)?)
    }
}

fn legacy_control_settings() -> Option<ControlSettings> {
    let base = directories::BaseDirs::new()?;
    let path = base
        .config_dir()
        .join("app.kestrel.local")
        .join("settings.json");
    let bytes = fs::read(path).ok()?;
    let value: Value = serde_json::from_slice(without_utf8_bom(&bytes)).ok()?;
    let mut settings = ControlSettings::default();
    settings.advanced_mode = value
        .get("advanced_mode")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    settings.engine_path = value
        .get("llama_server_path")
        .and_then(Value::as_str)
        .unwrap_or(&settings.engine_path)
        .into();
    settings.extra_model_roots = value
        .get("extra_model_roots")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(Into::into)
                .collect()
        })
        .unwrap_or_default();
    settings.selected_model_id = value
        .get("startup_model_id")
        .and_then(Value::as_str)
        .map(Into::into);
    let context = value
        .get("context_size")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if context > 0 {
        settings.context_window = u32::try_from(context).unwrap_or(u32::MAX);
    }
    settings.max_output_tokens = value
        .get("agent_max_tokens")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(settings.max_output_tokens);
    settings.threads = value
        .get("threads")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(settings.threads);
    Some(settings)
}

impl SettingsStore {
    pub fn new(library_root: &Path) -> Self {
        Self {
            path: library_root.join("research-settings.json"),
        }
    }

    pub fn load(&self) -> Result<ResearchSettings, ConfigError> {
        let settings = recoverable_json(&self.path)?.unwrap_or_default();
        validate(&settings)?;
        Ok(settings)
    }

    pub fn save(&self, settings: &ResearchSettings) -> Result<(), ConfigError> {
        validate(settings)?;
        atomic_json_write(&self.path, &serde_json::to_vec_pretty(settings)?)
    }
}

fn atomic_json_write(path: &Path, contents: &[u8]) -> Result<(), ConfigError> {
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.backup");
    fs::write(&temporary, contents)?;
    if path.is_file() {
        fs::copy(path, &backup)?;
        fs::remove_file(path)?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.is_file() {
            let _ = fs::copy(&backup, path);
        }
        return Err(error.into());
    }
    if !backup.is_file() {
        fs::copy(path, &backup)?;
    }
    Ok(())
}

fn recoverable_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, ConfigError> {
    let backup = path.with_extension("json.backup");
    if path.is_file() {
        let primary = fs::read(path)?;
        match serde_json::from_slice(without_utf8_bom(&primary)) {
            Ok(value) => return Ok(Some(value)),
            Err(primary_error) => {
                if backup.is_file() {
                    if let Ok(value) = serde_json::from_slice(without_utf8_bom(&fs::read(&backup)?))
                    {
                        fs::copy(&backup, path)?;
                        return Ok(Some(value));
                    }
                }
                return Err(primary_error.into());
            }
        }
    }
    if backup.is_file() {
        let bytes = fs::read(&backup)?;
        let value = serde_json::from_slice(without_utf8_bom(&bytes))?;
        fs::copy(&backup, path)?;
        return Ok(Some(value));
    }
    Ok(None)
}

pub(crate) fn without_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

pub fn apply_bonsai_runtime(settings: &ResearchSettings) -> Result<PathBuf, ConfigError> {
    validate(settings)?;
    let path = Path::new(&settings.bonsai_root).join("settings.json");
    if !path.is_file() {
        return Err(ConfigError::MissingBonsaiSettings(
            path.display().to_string(),
        ));
    }
    let bytes = fs::read(&path)?;
    let mut value: Value = serde_json::from_slice(without_utf8_bom(&bytes))?;
    value["AdvancedMode"] = Value::Bool(true);
    value["ContextWindow"] = Value::from(settings.context_window);
    value["MainMaxOutputTokens"] = Value::from(settings.max_output_tokens);
    let temporary = path.with_extension("json.kestrel.tmp");
    let backup = path.with_extension("json.kestrel-backup");
    fs::write(&temporary, serde_json::to_vec_pretty(&value)?)?;
    fs::copy(&path, &backup)?;
    fs::remove_file(&path)?;
    if let Err(error) = fs::rename(&temporary, &path) {
        let _ = fs::copy(&backup, &path);
        return Err(error.into());
    }
    Ok(path)
}

pub fn validate(settings: &ResearchSettings) -> Result<(), ConfigError> {
    if settings.advanced_mode
        && [
            settings.context_window,
            settings.max_output_tokens,
            settings.research_lanes,
            settings.results_per_lane,
            settings.source_target,
            settings.tool_turns,
            settings.thinking_budget,
            settings.max_source_chars,
        ]
        .contains(&0)
    {
        return Err(ConfigError::InvalidAdvancedValue);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_uncapped_advanced_values() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(directory.path());
        let settings = ResearchSettings {
            advanced_mode: true,
            context_window: 196_608,
            max_output_tokens: 65_536,
            research_lanes: 40,
            ..ResearchSettings::default()
        };
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap(), settings);
    }

    #[test]
    fn restores_settings_from_the_recovery_copy() {
        let directory = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(directory.path());
        let first = ResearchSettings::default();
        let second = ResearchSettings {
            context_window: 65_536,
            ..first.clone()
        };
        store.save(&first).unwrap();
        store.save(&second).unwrap();
        fs::write(directory.path().join("research-settings.json"), b"broken").unwrap();
        assert_eq!(store.load().unwrap(), first);
        assert!(serde_json::from_slice::<ResearchSettings>(
            &fs::read(directory.path().join("research-settings.json")).unwrap()
        )
        .is_ok());
    }

    #[test]
    fn rejects_zero_only_when_advanced_is_enabled() {
        let safe = ResearchSettings {
            research_lanes: 0,
            ..ResearchSettings::default()
        };
        assert!(validate(&safe).is_ok());
        assert!(validate(&ResearchSettings {
            advanced_mode: true,
            ..safe
        })
        .is_err());
    }

    #[test]
    fn applies_runtime_values_and_preserves_a_recovery_copy() {
        let directory = tempfile::tempdir().unwrap();
        let original = serde_json::json!({
            "AdvancedMode": false,
            "ContextWindow": 4_096,
            "MainMaxOutputTokens": 1_024,
            "Temperature": 0.6
        });
        let mut powershell_json = vec![0xEF, 0xBB, 0xBF];
        powershell_json.extend(serde_json::to_vec_pretty(&original).unwrap());
        fs::write(directory.path().join("settings.json"), powershell_json).unwrap();
        let settings = ResearchSettings {
            advanced_mode: true,
            bonsai_root: directory.path().to_string_lossy().into_owned(),
            context_window: 196_608,
            max_output_tokens: 65_536,
            ..ResearchSettings::default()
        };

        apply_bonsai_runtime(&settings).unwrap();

        let applied: Value =
            serde_json::from_slice(&fs::read(directory.path().join("settings.json")).unwrap())
                .unwrap();
        let backup_bytes = fs::read(directory.path().join("settings.json.kestrel-backup")).unwrap();
        let backup: Value = serde_json::from_slice(without_utf8_bom(&backup_bytes)).unwrap();
        assert_eq!(applied["ContextWindow"], 196_608);
        assert_eq!(applied["MainMaxOutputTokens"], 65_536);
        assert_eq!(applied["Temperature"], 0.6);
        assert_eq!(backup, original);
    }

    #[test]
    #[ignore = "writes the user's installed Bonsai settings with the validated profile"]
    fn live_applies_profile_to_powershell_bom_settings() {
        let settings = ResearchSettings {
            advanced_mode: true,
            ..ResearchSettings::default()
        };
        let path = apply_bonsai_runtime(&settings).expect("installed Bonsai settings should apply");
        let bytes = fs::read(&path).unwrap();
        assert!(!bytes.starts_with(&[0xEF, 0xBB, 0xBF]));
        let applied: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(applied["ContextWindow"], 98_304);
        assert_eq!(applied["MainMaxOutputTokens"], 32_768);
        assert!(path.with_extension("json.kestrel-backup").is_file());
    }
}
