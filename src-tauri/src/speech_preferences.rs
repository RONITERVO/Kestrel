use crate::config::{atomic_json_write, without_utf8_bom};
use kestrel_app_core::{
    LegacySpeechPreferences, ResearchSpeechPreferences, SpeechPreferences, VadSettings,
};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_PREFERENCES_BYTES: u64 = 64 * 1024;

fn read_preferences(path: &Path) -> Result<SpeechPreferences, String> {
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_PREFERENCES_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_PREFERENCES_BYTES {
        return Err("Speech preferences exceed the 64 KiB limit.".into());
    }
    let mut preferences: SpeechPreferences =
        serde_json::from_slice(without_utf8_bom(&bytes)).map_err(|error| error.to_string())?;
    preferences.research.validate()?;
    preferences.vad = preferences.vad.normalize();
    Ok(preferences)
}

pub struct SpeechPreferencesStore {
    path: PathBuf,
    gate: Mutex<()>,
}

impl SpeechPreferencesStore {
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join("speech-preferences.json"),
            gate: Mutex::new(()),
        }
    }

    fn load(&self) -> Result<Option<SpeechPreferences>, String> {
        let backup = self.path.with_extension("json.backup");
        if !self.path.exists() && !backup.exists() {
            return Ok(None);
        }
        match read_preferences(&self.path) {
            Ok(preferences) => Ok(Some(preferences)),
            Err(primary_error) => match read_preferences(&backup) {
                Ok(preferences) => {
                    fs::copy(&backup, &self.path).map_err(|error| {
                        format!("Could not restore speech preferences: {error}")
                    })?;
                    Ok(Some(preferences))
                }
                Err(_) => Err(format!(
                    "Could not read speech preferences or their recovery copy: {primary_error}"
                )),
            },
        }
    }

    fn write(&self, preferences: &SpeechPreferences) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(preferences).map_err(|e| e.to_string())?;
        atomic_json_write(&self.path, &bytes).map_err(|error| format!("Could not save speech preferences. Check write access to the research library: {error}"))
    }

    pub fn get(
        &self,
        legacy: Option<LegacySpeechPreferences>,
    ) -> Result<SpeechPreferences, String> {
        let _guard = self.gate.lock().map_err(|e| e.to_string())?;
        if let Some(saved) = self.load()? {
            return Ok(saved);
        }
        let mut preferences = SpeechPreferences::default();
        if let Some(legacy) = legacy {
            if let Some(json) = legacy.vad_json.filter(|s| s.len() <= 8192) {
                if let Ok(vad) = serde_json::from_str::<VadSettings>(&json) {
                    preferences.vad = vad.normalize();
                }
            }
            let research = &mut preferences.research;
            research.model_id = legacy
                .model_id
                .filter(|s| s.len() <= 256)
                .unwrap_or_default();
            research.voice_profile_id = legacy
                .voice_profile_id
                .filter(|s| s.len() <= 256)
                .unwrap_or_default();
            if let Some(rate) = legacy
                .rate
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|r| r.is_finite() && (0.8..=1.5).contains(r))
            {
                research.rate = rate;
            }
            research.scope = match legacy.scope.as_deref() {
                Some("summary") => kestrel_app_core::ResearchSpeechScope::Summary,
                Some("all") => kestrel_app_core::ResearchSpeechScope::All,
                _ => kestrel_app_core::ResearchSpeechScope::Article,
            };
        }
        self.write(&preferences)?;
        Ok(preferences)
    }

    pub fn save_vad(&self, vad: VadSettings) -> Result<SpeechPreferences, String> {
        let _guard = self.gate.lock().map_err(|e| e.to_string())?;
        let mut preferences = self.load()?.unwrap_or_default();
        preferences.vad = vad.normalize();
        self.write(&preferences)?;
        Ok(preferences)
    }

    pub fn save_research(
        &self,
        research: ResearchSpeechPreferences,
    ) -> Result<SpeechPreferences, String> {
        research.validate()?;
        let _guard = self.gate.lock().map_err(|e| e.to_string())?;
        let mut preferences = self.load()?.unwrap_or_default();
        preferences.research = research;
        self.write(&preferences)?;
        Ok(preferences)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_once_preserves_other_preferences_and_recovers_backup() {
        let root = tempfile::tempdir().unwrap();
        let store = SpeechPreferencesStore::new(root.path());
        let initial = store
            .get(Some(LegacySpeechPreferences {
                vad_json: Some(r#"{"silenceTimeoutSec":3.5}"#.into()),
                rate: Some("1.2".into()),
                scope: Some("all".into()),
                ..Default::default()
            }))
            .unwrap();
        assert_eq!(initial.vad.silence_timeout_sec, 3.5);
        let next = store
            .save_vad(VadSettings {
                speech_threshold_db: 100.0,
                ..initial.vad
            })
            .unwrap();
        assert_eq!(next.vad.speech_threshold_db, -20.0);
        assert_eq!(next.research.rate, 1.2);
        assert_eq!(
            store.get(Some(LegacySpeechPreferences::default())).unwrap(),
            next
        );
        // Recovery copy contains the previous complete preferences, never partial JSON.
        std::fs::write(&store.path, b"broken").unwrap();
        let recovered = store.get(None).unwrap();
        assert_eq!(recovered.research.rate, 1.2);
        assert_eq!(recovered.vad.silence_timeout_sec, 3.5);
    }

    #[test]
    fn oversized_preferences_recover_without_resetting_user_values() {
        let root = tempfile::tempdir().unwrap();
        let store = SpeechPreferencesStore::new(root.path());
        let initial = store
            .get(Some(LegacySpeechPreferences {
                rate: Some("1.3".into()),
                ..Default::default()
            }))
            .unwrap();
        fs::write(&store.path, vec![b' '; MAX_PREFERENCES_BYTES as usize + 1]).unwrap();
        assert_eq!(store.get(None).unwrap(), initial);
        fs::write(&store.path, b"broken").unwrap();
        fs::write(store.path.with_extension("json.backup"), b"broken").unwrap();
        assert!(store.get(None).unwrap_err().contains("recovery copy"));
        assert_eq!(fs::read(&store.path).unwrap(), b"broken");
    }
}
