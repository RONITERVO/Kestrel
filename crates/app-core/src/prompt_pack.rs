use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use ts_rs::TS;

/// Durable user-editable prompt pack. Raw-text IPC preserves invalid drafts for editing;
/// the native catalog validates format, keys and placeholders before applying a pack.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PromptPack {
    pub format: String,
    pub version: u32,
    pub prompts: BTreeMap<String, String>,
}
