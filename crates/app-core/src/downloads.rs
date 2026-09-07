// Rust-owned wire and durable data. Native services retain execution authority.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadRequest {
    pub url: String,
    #[serde(default)]
    pub expected_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadCandidate {
    pub file_path: String,
    pub file_name: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: Option<String>,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadInspection {
    pub repository: String,
    pub revision: String,
    pub candidates: Vec<ModelDownloadCandidate>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadRecord {
    pub id: String,
    pub status: String,
    pub source_url: String,
    pub repository: String,
    pub revision: String,
    pub file_name: String,
    pub destination_path: String,
    pub partial_path: String,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub bytes_per_second: u64,
    pub eta_seconds: Option<u64>,
    pub expected_sha256: Option<String>,
    pub actual_sha256: Option<String>,
    pub source_etag: Option<String>,
    pub checksum_source: String,
    pub retry_count: u32,
    pub created_at: String,
    pub updated_at: String,
    pub detail: String,
    pub error: Option<String>,
}
