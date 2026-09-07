// Rust-owned wire and durable data. Native services retain execution authority.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiDocument {
    pub schema_version: u32,
    pub take_id: String,
    pub source_sha256: String,
    pub revision: u32,
    pub ticks_per_quarter: u16,
    pub duration_ticks: u64,
    pub duration_seconds: f64,
    pub tempos: Vec<MusicMidiTempo>,
    pub time_signatures: Vec<MusicMidiTimeSignature>,
    pub tracks: Vec<MusicMidiTrack>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiTempo {
    pub tick: u64,
    pub microseconds_per_quarter: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiTimeSignature {
    pub tick: u64,
    pub numerator: u8,
    pub denominator: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiTrack {
    pub id: String,
    pub name: String,
    pub channel: u8,
    pub program: u8,
    pub muted: bool,
    pub notes: Vec<MusicMidiNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiNote {
    pub id: String,
    pub pitch: u8,
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub velocity: u8,
    pub channel: u8,
}
