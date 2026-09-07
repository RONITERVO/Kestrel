use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum ExternalCollaborationTarget {
    MovieBrief,
    MovieImageDescription,
    MovieReferenceDescription,
    ImageDesign,
    MusicDescription,
    MusicLyrics,
    MovieGenerationDirection,
}

/// Clipboard interchange for a producer-reviewed draft, never an executable model instruction.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExternalCollaborationResponse {
    pub format: String,
    pub version: u32,
    pub target: ExternalCollaborationTarget,
    pub result: serde_json::Value,
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExternalCollaborationFormat {
    pub format: &'static str,
    pub version: u32,
    pub maximum_bytes: u32,
}

pub const EXTERNAL_COLLABORATION: ExternalCollaborationFormat = ExternalCollaborationFormat {
    format: "kestrel.external-collaboration.response",
    version: 1,
    maximum_bytes: 2 * 1024 * 1024,
};
