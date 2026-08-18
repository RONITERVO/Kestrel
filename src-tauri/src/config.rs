use crate::models::{ControlSettings, ResearchSettings};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{collections::HashSet, fs};
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
    #[error("each model override needs a unique model ID and positive values")]
    InvalidModelOverride,
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
        if settings.context_window == 0
            || settings.max_output_tokens == 0
            || settings.threads == 0
            || settings.agent_max_steps == 0
            || settings.agent_max_output_tokens == 0
        {
            return Err(ConfigError::InvalidAdvancedValue);
        }
        let mut model_ids = HashSet::new();
        if settings.model_overrides.iter().any(|model| {
            model.model_id.trim().is_empty()
                || !model_ids.insert(model.model_id.as_str())
                || [
                    model.context_window,
                    model.max_output_tokens,
                    model.threads,
                ]
                .into_iter()
                .flatten()
                .any(|value| value == 0)
        }) {
            return Err(ConfigError::InvalidModelOverride);
        }
        let mut stored = settings.clone();
        if !stored.advanced_mode {
            stored.context_window = stored.context_window.min(98_304);
            stored.max_output_tokens = stored.max_output_tokens.min(32_768);
            stored.agent_max_steps = stored.agent_max_steps.min(50);
            stored.agent_max_output_tokens = stored.agent_max_output_tokens.min(32_768);
            for model in &mut stored.model_overrides {
                model.context_window = model.context_window.map(|value| value.min(98_304));
                model.max_output_tokens =
                    model.max_output_tokens.map(|value| value.min(32_768));
            }
        }
        atomic_json_write(&self.path, &serde_json::to_vec_pretty(&stored)?)
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
    settings.allow_full_access_agent = value
        .get("allow_full_access_agent")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    settings.agent_workspace_roots = value
        .get("agent_workspace_roots")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(Into::into)
                .collect()
        })
        .filter(|values: &Vec<String>| !values.is_empty())
        .unwrap_or(settings.agent_workspace_roots);
    settings.agent_max_output_tokens = value
        .get("agent_max_tokens")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(settings.agent_max_output_tokens);
    if let Some(level) = value.get("thinking_level").or_else(|| value.get("thinkingLevel")) {
        if let Ok(parsed) = serde_json::from_value::<crate::models::ThinkingLevel>(level.clone()) {
            settings.thinking_level = parsed;
        }
    }
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
        let mut stored = settings.clone();
        if !stored.advanced_mode {
            let defaults = ResearchSettings::default();
            stored.context_window = stored.context_window.min(defaults.context_window);
            stored.max_output_tokens = stored.max_output_tokens.min(defaults.max_output_tokens);
            stored.research_lanes = stored.research_lanes.min(defaults.research_lanes);
            stored.results_per_lane = stored.results_per_lane.min(defaults.results_per_lane);
            stored.source_target = stored.source_target.min(defaults.source_target);
            stored.tool_turns = stored.tool_turns.min(defaults.tool_turns);
            stored.thinking_budget = stored.thinking_budget.min(defaults.thinking_budget);
            stored.max_source_chars = stored.max_source_chars.min(defaults.max_source_chars);
        }
        atomic_json_write(&self.path, &serde_json::to_vec_pretty(&stored)?)
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
    fn legacy_research_settings_gain_safe_defaults_for_new_setup_fields() {
        let legacy = serde_json::json!({
            "advancedMode": true,
            "bonsaiRoot": r"D:\LocalAI\Bonsai27B",
            "contextWindow": 98_304,
            "maxOutputTokens": 32_768,
            "researchLanes": 7,
            "resultsPerLane": 6,
            "sourceTarget": 12,
            "toolTurns": 24,
            "thinkingBudget": 4_096,
            "maxSourceChars": 20_000
        });

        let settings: ResearchSettings = serde_json::from_value(legacy).unwrap();

        assert_eq!(settings.bonsai_root, r"D:\LocalAI\Bonsai27B");
        assert!(!settings.install_root.is_empty());
        assert!(!settings.wikipedia_zim_path.is_empty());
        assert!(!settings.kiwix_server_path.is_empty());
        assert!(!settings.comfy_root.is_empty());
        assert_eq!(settings.research_lanes, 7);
    }

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
    fn standard_control_profile_keeps_tested_internal_ceiling() {
        let directory = tempfile::tempdir().unwrap();
        let store = ControlSettingsStore::new(directory.path());
        let settings = ControlSettings {
            advanced_mode: false,
            context_window: 500_000,
            max_output_tokens: 100_000,
            agent_max_steps: 500,
            agent_max_output_tokens: 100_000,
            ..ControlSettings::default()
        };
        store.save(&settings).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.context_window, 98_304);
        assert_eq!(loaded.max_output_tokens, 32_768);
        assert_eq!(loaded.agent_max_steps, 50);
        assert_eq!(loaded.agent_max_output_tokens, 32_768);
    }

    #[test]
    fn advanced_control_profile_preserves_supplied_values() {
        let directory = tempfile::tempdir().unwrap();
        let store = ControlSettingsStore::new(directory.path());
        let settings = ControlSettings {
            advanced_mode: true,
            context_window: 196_608,
            max_output_tokens: 65_536,
            agent_max_steps: 100,
            agent_max_output_tokens: 65_536,
            ..ControlSettings::default()
        };
        store.save(&settings).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(loaded.context_window, settings.context_window);
        assert_eq!(loaded.max_output_tokens, settings.max_output_tokens);
        assert_eq!(loaded.agent_max_steps, settings.agent_max_steps);
        assert_eq!(
            loaded.agent_max_output_tokens,
            settings.agent_max_output_tokens
        );
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
    fn per_model_runtime_values_override_global_defaults() {
        let settings = ControlSettings {
            context_window: 32_768,
            max_output_tokens: 8_192,
            thinking_level: crate::models::ThinkingLevel::High,
            model_overrides: vec![crate::models::ModelRuntimeOverride {
                model_id: "model-a".into(),
                context_window: Some(65_536),
                max_output_tokens: Some(16_384),
                threads: Some(6),
                thinking_level: Some(crate::models::ThinkingLevel::Off),
            }],
            ..ControlSettings::default()
        };

        let effective = settings.for_model("model-a");
        assert_eq!(effective.context_window, 65_536);
        assert_eq!(effective.max_output_tokens, 16_384);
        assert_eq!(effective.threads, 6);
        assert_eq!(effective.thinking_level, crate::models::ThinkingLevel::Off);
        assert_eq!(settings.for_model("model-b").context_window, 32_768);
        assert_eq!(settings.for_model("model-b").thinking_level, crate::models::ThinkingLevel::High);
    }
}
