//! Platform-neutral composition of existing, verified artifacts. No generation is implied.
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use crate::ProductionArtifact;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct MediaVisualSegment {
    pub source: ProductionArtifact,
    /// Explicit still hold or video excerpt duration. Video is never silently repeated.
    pub duration_seconds: f64,
    #[serde(default)]
    pub source_start_seconds: f64,
    /// Crossfade into this segment; zero is a cut. Must be zero on the first segment.
    #[serde(default)]
    pub transition_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct MediaAudioLayer {
    pub source: ProductionArtifact,
    pub start_seconds: f64,
    pub duration_seconds: f64,
    #[serde(default)]
    pub source_start_seconds: f64,
    pub gain: f64,
    #[serde(default)]
    pub fade_in_seconds: f64,
    #[serde(default)]
    pub fade_out_seconds: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MediaFrameShape { Landscape, Portrait, Square }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct MediaCompositionRequest {
    /// Caller-chosen job identity, also accepted by cancel_production_job.
    pub job_id: String,
    pub title: String,
    /// Total timeline duration, 0.1–3,600 seconds. Visual segments must cover it exactly.
    pub duration_seconds: f64,
    pub frame_shape: MediaFrameShape,
    /// Ordered stills/video, optional for audio-only delivery. At most 64 segments.
    pub visuals: Vec<MediaVisualSegment>,
    /// Independently timed audio, optional for silent video. At most 64 layers.
    pub audio: Vec<MediaAudioLayer>,
    pub output_directory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaCompositionExport {
    pub id: String,
    pub directory: String,
    pub media_path: String,
    pub native_path: String,
    pub manifest_path: String,
    pub duration_seconds: f64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MediaCompositionSummary {
    pub id: String,
    pub title: String,
    /// complete or incomplete; interrupted work is never started by discovery.
    pub status: String,
    pub duration_seconds: f64,
    pub native_path: Option<String>,
}
