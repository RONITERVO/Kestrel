use crate::models::ResearchSettings;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("research settings file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("research settings JSON error: {0}")]
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

impl SettingsStore {
    pub fn new(library_root: &Path) -> Self {
        Self {
            path: library_root.join("research-settings.json"),
        }
    }

    pub fn load(&self) -> Result<ResearchSettings, ConfigError> {
        if !self.path.is_file() {
            return Ok(ResearchSettings::default());
        }
        let settings = serde_json::from_slice(&fs::read(&self.path)?)?;
        validate(&settings)?;
        Ok(settings)
    }

    pub fn save(&self, settings: &ResearchSettings) -> Result<(), ConfigError> {
        validate(settings)?;
        let temporary = self.path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(settings)?)?;
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        fs::rename(temporary, &self.path)?;
        Ok(())
    }
}

pub fn apply_bonsai_runtime(settings: &ResearchSettings) -> Result<PathBuf, ConfigError> {
    validate(settings)?;
    let path = Path::new(&settings.bonsai_root).join("settings.json");
    if !path.is_file() {
        return Err(ConfigError::MissingBonsaiSettings(
            path.display().to_string(),
        ));
    }
    let mut value: Value = serde_json::from_slice(&fs::read(&path)?)?;
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
        fs::write(
            directory.path().join("settings.json"),
            serde_json::to_vec_pretty(&original).unwrap(),
        )
        .unwrap();
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
        let backup: Value = serde_json::from_slice(
            &fs::read(directory.path().join("settings.json.kestrel-backup")).unwrap(),
        )
        .unwrap();
        assert_eq!(applied["ContextWindow"], 196_608);
        assert_eq!(applied["MainMaxOutputTokens"], 65_536);
        assert_eq!(applied["Temperature"], 0.6);
        assert_eq!(backup, original);
    }
}
