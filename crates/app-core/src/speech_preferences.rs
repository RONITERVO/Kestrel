use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct VadSettings {
    pub enabled: bool,
    pub silence_timeout_sec: f64,
    pub speech_threshold_db: f64,
    pub min_speech_duration_ms: f64,
    pub initial_grace_timeout_sec: f64,
}

impl Default for VadSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            silence_timeout_sec: 2.0,
            speech_threshold_db: -42.0,
            min_speech_duration_ms: 400.0,
            initial_grace_timeout_sec: 15.0,
        }
    }
}

impl VadSettings {
    pub fn normalize(mut self) -> Self {
        fn bound(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                fallback
            }
        }
        self.silence_timeout_sec = bound(self.silence_timeout_sec, 0.5, 10.0, 2.0);
        self.speech_threshold_db = bound(self.speech_threshold_db, -60.0, -20.0, -42.0).round();
        self.min_speech_duration_ms =
            bound(self.min_speech_duration_ms, 100.0, 2000.0, 400.0).round();
        self.initial_grace_timeout_sec = bound(self.initial_grace_timeout_sec, 3.0, 60.0, 15.0);
        self
    }
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ResearchSpeechScope {
    Summary,
    #[default]
    Article,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct ResearchSpeechPreferences {
    pub model_id: String,
    pub voice_profile_id: String,
    pub rate: f64,
    pub scope: ResearchSpeechScope,
}

impl Default for ResearchSpeechPreferences {
    fn default() -> Self {
        Self {
            model_id: String::new(),
            voice_profile_id: String::new(),
            rate: 1.0,
            scope: ResearchSpeechScope::default(),
        }
    }
}

impl ResearchSpeechPreferences {
    pub fn validate(&self) -> Result<(), String> {
        if !self.rate.is_finite() || !(0.8..=1.5).contains(&self.rate) {
            return Err("Narration speed must be between 0.8 and 1.5.".into());
        }
        if self.model_id.len() > 256 || self.voice_profile_id.len() > 256 {
            return Err("Speech model and voice identifiers must be at most 256 bytes.".into());
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct SpeechPreferences {
    pub vad: VadSettings,
    pub research: ResearchSpeechPreferences,
}

/// Read-only import from older WebView preferences. Native storage wins once present.
#[derive(Debug, Default, Deserialize, TS)]
#[serde(default, rename_all = "camelCase")]
#[ts(export)]
pub struct LegacySpeechPreferences {
    pub vad_json: Option<String>,
    pub model_id: Option<String>,
    pub voice_profile_id: Option<String>,
    pub rate: Option<String>,
    pub scope: Option<String>,
}
