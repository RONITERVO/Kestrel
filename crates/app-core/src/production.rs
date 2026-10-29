//! Shared artifact identities and format-neutral delivery bundles.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub enum ProductionArtifact {
    ImageTake { project_id: String, take_id: String },
    MusicTake { project_id: String, take_id: String },
    MovieClip { project_id: String, clip_id: String, version_id: Option<String> },
    MovieExport { project_id: String },
    /// Use the generated/...opus cache reference returned by synthesize_local_speech, with
    /// the exact spoken text. An exported M4A is an imported Attachment instead.
    Speech { relative_path: String, text: String },
    /// Reuse a completed native composition, independently of any exported copy.
    Composition { id: String },
    /// Imported local files: images, audio, documents, code or future model assets.
    Attachment { id: String },
    /// One immutable published research file, including its evidence or provenance.
    Research { report_id: String, file: ResearchArtifactFile },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ResearchArtifactFile { Html, Report, Sources, Provenance }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ProductionImageFit { Contain, Cover }

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub enum ProductionArtifactFormat {
    /// Copy exact source bytes, preserving the source extension.
    Original,
    /// Explicit image conversion; also useful for optional thumbnail choices.
    JpegImage { width: u32, height: u32, fit: ProductionImageFit },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ProductionBundleArtifact {
    pub source: ProductionArtifact,
    /// Safe relative filename, optionally with subfolders. Never an absolute path.
    pub filename: String,
    /// Producer-defined description such as front view, cover option, narration, or evidence.
    pub role: String,
    pub format: ProductionArtifactFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ProductionTextFile {
    pub filename: String,
    /// Exact producer-authored text, metadata, document, or application source; never executed.
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ProductionBundleRequest {
    /// Choose any fresh 1–128 character identifier using letters, digits, hyphens or underscores.
    /// It is a cancellation identity, not an existing artifact ID or a credential.
    pub job_id: String,
    pub title: String,
    pub description: String,
    pub artifacts: Vec<ProductionBundleArtifact>,
    pub files: Vec<ProductionTextFile>,
    /// New child folder is created here. No publish/upload step is implied.
    pub output_directory: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProductionBundle {
    pub id: String,
    pub directory: String,
    pub manifest_path: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProductionBundleSummary {
    pub id: String,
    pub title: String,
    /// State of the native bundle; delivery can separately fail after native completion.
    pub status: String,
    pub native_directory: String,
}
