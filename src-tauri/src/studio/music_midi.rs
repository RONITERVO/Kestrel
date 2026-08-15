//! Validated Standard MIDI File parsing and producer-editable piano-roll documents.
//!
//! MuScriptor output is immutable source material. This module converts it into a bounded typed
//! document and writes producer revisions as new MIDI files; model output is never overwritten.

use super::StudioError;
use midly::{
    num::{u15, u24, u28, u4, u7},
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::Write,
    path::Path,
};

const MIDI_DOCUMENT_SCHEMA_VERSION: u32 = 1;
const MAX_MIDI_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MIDI_TRACKS: usize = 128;
const MAX_MIDI_NOTES: usize = 250_000;
const MAX_MIDI_TICKS: u64 = 100_000_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiTempo {
    pub tick: u64,
    pub microseconds_per_quarter: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiTimeSignature {
    pub tick: u64,
    pub numerator: u8,
    pub denominator: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiTrack {
    pub id: String,
    pub name: String,
    pub channel: u8,
    pub program: u8,
    pub muted: bool,
    pub notes: Vec<MusicMidiNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MusicMidiNote {
    pub id: String,
    pub pitch: u8,
    pub start_tick: u64,
    pub duration_ticks: u64,
    pub velocity: u8,
    pub channel: u8,
}

#[derive(Default)]
struct TrackBuilder {
    channel: u8,
    program: u8,
    notes: Vec<(u8, u64, u64, u8)>,
    open_notes: HashMap<u8, Vec<(u64, u8)>>,
}

pub fn parse_midi_document(
    path: &Path,
    take_id: &str,
    source_sha256: &str,
    revision: u32,
) -> Result<MusicMidiDocument, StudioError> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_MIDI_BYTES {
        return Err(StudioError::Invalid(
            "the MIDI file is empty or exceeds the 64 MiB editor limit".into(),
        ));
    }
    let bytes = fs::read(path)?;
    let smf = Smf::parse(&bytes)
        .map_err(|error| StudioError::Invalid(format!("the MIDI file is malformed: {error}")))?;
    let ticks_per_quarter =
        match smf.header.timing {
            Timing::Metrical(value) => value.as_int(),
            Timing::Timecode(_, _) => return Err(StudioError::Invalid(
                "SMPTE-time MIDI is preserved but cannot be edited in the beat-based piano roll"
                    .into(),
            )),
        };
    let mut tempo_map = BTreeMap::new();
    tempo_map.insert(0_u64, 500_000_u32);
    let mut signature_map = BTreeMap::new();
    signature_map.insert(0_u64, (4_u8, 4_u8));
    let mut tracks = Vec::new();
    let mut duration_ticks = 0_u64;

    for (source_index, source_track) in smf.tracks.iter().enumerate() {
        let mut tick = 0_u64;
        let mut source_name = format!("Track {}", source_index + 1);
        let mut builders: BTreeMap<u8, TrackBuilder> = BTreeMap::new();
        for event in source_track {
            tick = tick.saturating_add(u64::from(event.delta.as_int()));
            if tick > MAX_MIDI_TICKS {
                return Err(StudioError::Invalid(
                    "the MIDI timeline exceeds the supported editor duration".into(),
                ));
            }
            duration_ticks = duration_ticks.max(tick);
            match event.kind {
                TrackEventKind::Meta(MetaMessage::TrackName(name)) => {
                    let value = String::from_utf8_lossy(name).trim().to_string();
                    if !value.is_empty() {
                        source_name = value;
                    }
                }
                TrackEventKind::Meta(MetaMessage::Tempo(value)) => {
                    tempo_map.insert(tick, value.as_int());
                }
                TrackEventKind::Meta(MetaMessage::TimeSignature(
                    numerator,
                    denominator_power,
                    _,
                    _,
                )) => {
                    if denominator_power <= 6 {
                        signature_map.insert(tick, (numerator.max(1), 1_u8 << denominator_power));
                    }
                }
                TrackEventKind::Midi { channel, message } => {
                    let channel = channel.as_int();
                    let builder = builders.entry(channel).or_insert_with(|| TrackBuilder {
                        channel,
                        ..TrackBuilder::default()
                    });
                    match message {
                        MidiMessage::ProgramChange { program } => {
                            builder.program = program.as_int();
                        }
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            builder
                                .open_notes
                                .entry(key.as_int())
                                .or_default()
                                .push((tick, vel.as_int()));
                        }
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            if let Some(open) = builder.open_notes.get_mut(&key.as_int()) {
                                if let Some((start, velocity)) = open.pop() {
                                    let duration = tick.saturating_sub(start).max(1);
                                    builder
                                        .notes
                                        .push((key.as_int(), start, duration, velocity));
                                    duration_ticks =
                                        duration_ticks.max(start.saturating_add(duration));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        for builder in builders.values_mut() {
            for (pitch, open) in &builder.open_notes {
                for (start, velocity) in open {
                    let duration = tick.saturating_sub(*start).max(1);
                    builder.notes.push((*pitch, *start, duration, *velocity));
                    duration_ticks = duration_ticks.max(start.saturating_add(duration));
                }
            }
        }
        for (channel, mut builder) in builders {
            if builder.notes.is_empty() {
                continue;
            }
            builder.notes.sort_by_key(|note| (note.1, note.0, note.2));
            let track_number = tracks.len() + 1;
            let notes = builder
                .notes
                .into_iter()
                .enumerate()
                .map(
                    |(index, (pitch, start_tick, duration_ticks, velocity))| MusicMidiNote {
                        id: format!("track-{track_number:03}-note-{:06}", index + 1),
                        pitch,
                        start_tick,
                        duration_ticks,
                        velocity: velocity.max(1),
                        channel: builder.channel,
                    },
                )
                .collect();
            tracks.push(MusicMidiTrack {
                id: format!("source-{:03}-channel-{:02}", source_index + 1, channel + 1),
                name: if builders_name_needs_channel(&source_name, &tracks) {
                    format!("{source_name} · Ch {}", channel + 1)
                } else {
                    source_name.clone()
                },
                channel,
                program: builder.program,
                muted: false,
                notes,
            });
        }
    }

    let tempos = tempo_map
        .into_iter()
        .map(|(tick, microseconds_per_quarter)| MusicMidiTempo {
            tick,
            microseconds_per_quarter,
        })
        .collect::<Vec<_>>();
    let time_signatures = signature_map
        .into_iter()
        .map(|(tick, (numerator, denominator))| MusicMidiTimeSignature {
            tick,
            numerator,
            denominator,
        })
        .collect::<Vec<_>>();
    let document = MusicMidiDocument {
        schema_version: MIDI_DOCUMENT_SCHEMA_VERSION,
        take_id: take_id.into(),
        source_sha256: source_sha256.into(),
        revision,
        ticks_per_quarter,
        duration_ticks,
        duration_seconds: ticks_to_seconds(duration_ticks, ticks_per_quarter, &tempos),
        tempos,
        time_signatures,
        tracks,
    };
    validate_midi_document(&document)?;
    Ok(document)
}

fn builders_name_needs_channel(name: &str, tracks: &[MusicMidiTrack]) -> bool {
    tracks
        .iter()
        .any(|track| track.name == name || track.name.starts_with(&format!("{name} · Ch")))
}

pub fn validate_midi_document(document: &MusicMidiDocument) -> Result<(), StudioError> {
    if document.schema_version != MIDI_DOCUMENT_SCHEMA_VERSION {
        return Err(StudioError::Invalid(
            "unsupported MIDI editor document version".into(),
        ));
    }
    if !(1..=32_767).contains(&document.ticks_per_quarter) {
        return Err(StudioError::Invalid(
            "MIDI ticks per quarter must be between 1 and 32767".into(),
        ));
    }
    if document.source_sha256.len() != 64
        || !document
            .source_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(StudioError::Invalid(
            "MIDI source provenance does not contain a valid SHA-256 digest".into(),
        ));
    }
    if document.duration_ticks > MAX_MIDI_TICKS || !document.duration_seconds.is_finite() {
        return Err(StudioError::Invalid(
            "MIDI duration is outside the editor limits".into(),
        ));
    }
    if document.tracks.len() > MAX_MIDI_TRACKS {
        return Err(StudioError::Invalid(
            "MIDI projects are limited to 128 tracks".into(),
        ));
    }
    let mut track_ids = HashSet::new();
    let mut note_ids = HashSet::new();
    let mut note_count = 0_usize;
    for track in &document.tracks {
        if track.id.is_empty() || !track_ids.insert(track.id.as_str()) {
            return Err(StudioError::Invalid(
                "MIDI track IDs must be non-empty and unique".into(),
            ));
        }
        if track.name.len() > 256 || track.channel > 15 || track.program > 127 {
            return Err(StudioError::Invalid(
                "a MIDI track has invalid metadata".into(),
            ));
        }
        note_count = note_count.saturating_add(track.notes.len());
        for note in &track.notes {
            let end = note.start_tick.saturating_add(note.duration_ticks);
            if note.id.is_empty()
                || !note_ids.insert(note.id.as_str())
                || note.pitch > 127
                || note.velocity == 0
                || note.velocity > 127
                || note.channel > 15
                || note.duration_ticks == 0
                || end > MAX_MIDI_TICKS
            {
                return Err(StudioError::Invalid(
                    "a MIDI note has invalid timing or performance data".into(),
                ));
            }
        }
    }
    if note_count > MAX_MIDI_NOTES {
        return Err(StudioError::Invalid(
            "MIDI projects are limited to 250000 notes".into(),
        ));
    }
    if document.tempos.is_empty()
        || document.tempos.len() > 4096
        || document.tempos.iter().any(|tempo| {
            tempo.tick > MAX_MIDI_TICKS
                || !(10_000..=60_000_000).contains(&tempo.microseconds_per_quarter)
        })
    {
        return Err(StudioError::Invalid("the MIDI tempo map is invalid".into()));
    }
    if document.time_signatures.is_empty()
        || document.time_signatures.len() > 4096
        || document.time_signatures.iter().any(|signature| {
            signature.tick > MAX_MIDI_TICKS
                || signature.numerator == 0
                || !matches!(signature.denominator, 1 | 2 | 4 | 8 | 16 | 32 | 64)
        })
    {
        return Err(StudioError::Invalid(
            "the MIDI time-signature map is invalid".into(),
        ));
    }
    Ok(())
}

pub fn normalize_midi_document(
    mut document: MusicMidiDocument,
) -> Result<MusicMidiDocument, StudioError> {
    for track in &mut document.tracks {
        track.name = track.name.trim().to_string();
        if track.name.is_empty() {
            track.name = format!("Channel {}", track.channel + 1);
        }
        track
            .notes
            .sort_by_key(|note| (note.start_tick, note.pitch, note.duration_ticks));
    }
    let mut tempos = BTreeMap::new();
    for tempo in std::mem::take(&mut document.tempos) {
        tempos.insert(tempo.tick, tempo);
    }
    document.tempos = tempos.into_values().collect();
    let mut time_signatures = BTreeMap::new();
    for signature in std::mem::take(&mut document.time_signatures) {
        time_signatures.insert(signature.tick, signature);
    }
    document.time_signatures = time_signatures.into_values().collect();
    document.duration_ticks = document
        .tracks
        .iter()
        .flat_map(|track| track.notes.iter())
        .map(|note| note.start_tick.saturating_add(note.duration_ticks))
        .chain(document.tempos.iter().map(|tempo| tempo.tick))
        .chain(
            document
                .time_signatures
                .iter()
                .map(|signature| signature.tick),
        )
        .max()
        .unwrap_or(0);
    document.duration_seconds = ticks_to_seconds(
        document.duration_ticks,
        document.ticks_per_quarter,
        &document.tempos,
    );
    validate_midi_document(&document)?;
    Ok(document)
}

pub fn encode_midi_document(document: &MusicMidiDocument) -> Result<Vec<u8>, StudioError> {
    validate_midi_document(document)?;
    let mut tracks: Vec<Vec<TrackEvent<'_>>> = Vec::new();
    let mut conductor_events = Vec::new();
    for tempo in &document.tempos {
        conductor_events.push((
            tempo.tick,
            1_u8,
            TrackEventKind::Meta(MetaMessage::Tempo(u24::new(tempo.microseconds_per_quarter))),
        ));
    }
    for signature in &document.time_signatures {
        conductor_events.push((
            signature.tick,
            2_u8,
            TrackEventKind::Meta(MetaMessage::TimeSignature(
                signature.numerator,
                signature.denominator.ilog2() as u8,
                24,
                8,
            )),
        ));
    }
    tracks.push(absolute_events_to_track(conductor_events)?);

    for track in document.tracks.iter().filter(|track| !track.muted) {
        let mut events = vec![(
            0,
            0,
            TrackEventKind::Meta(MetaMessage::TrackName(track.name.as_bytes())),
        )];
        events.push((
            0,
            3,
            TrackEventKind::Midi {
                channel: u4::new(track.channel),
                message: MidiMessage::ProgramChange {
                    program: u7::new(track.program),
                },
            },
        ));
        for note in &track.notes {
            events.push((
                note.start_tick,
                5,
                TrackEventKind::Midi {
                    channel: u4::new(note.channel),
                    message: MidiMessage::NoteOn {
                        key: u7::new(note.pitch),
                        vel: u7::new(note.velocity),
                    },
                },
            ));
            events.push((
                note.start_tick.saturating_add(note.duration_ticks),
                4,
                TrackEventKind::Midi {
                    channel: u4::new(note.channel),
                    message: MidiMessage::NoteOff {
                        key: u7::new(note.pitch),
                        vel: u7::new(0),
                    },
                },
            ));
        }
        tracks.push(absolute_events_to_track(events)?);
    }
    let header = Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(document.ticks_per_quarter)),
    );
    let smf = Smf { header, tracks };
    let mut bytes = Vec::new();
    smf.write_std(&mut bytes).map_err(|error| {
        StudioError::Invalid(format!("could not encode MIDI revision: {error}"))
    })?;
    Ok(bytes)
}

fn absolute_events_to_track<'a>(
    mut events: Vec<(u64, u8, TrackEventKind<'a>)>,
) -> Result<Vec<TrackEvent<'a>>, StudioError> {
    events.sort_by_key(|event| (event.0, event.1));
    let mut previous = 0_u64;
    let mut track = Vec::with_capacity(events.len() + 1);
    for (tick, _, kind) in events {
        let delta = tick.saturating_sub(previous);
        if delta > (1_u64 << 28) - 1 {
            return Err(StudioError::Invalid(
                "a MIDI event delta exceeds the file format limit".into(),
            ));
        }
        track.push(TrackEvent {
            delta: u28::new(delta as u32),
            kind,
        });
        previous = tick;
    }
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    Ok(track)
}

pub fn write_midi_document(path: &Path, document: &MusicMidiDocument) -> Result<(), StudioError> {
    let bytes = encode_midi_document(document)?;
    write_bytes_recoverable(path, &bytes)
}

pub fn write_bytes_recoverable(path: &Path, bytes: &[u8]) -> Result<(), StudioError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("mid.tmp");
    let backup = path.with_extension("mid.bak");
    {
        let mut output = fs::File::create(&temporary)?;
        output.write_all(bytes)?;
        output.sync_all()?;
    }
    if !path.exists() {
        fs::rename(temporary, path)?;
        return Ok(());
    }
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    fs::rename(path, &backup)?;
    match fs::rename(&temporary, path) {
        Ok(()) => {
            let _ = fs::remove_file(backup);
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(backup, path);
            let _ = fs::remove_file(temporary);
            Err(error.into())
        }
    }
}

fn ticks_to_seconds(tick: u64, ticks_per_quarter: u16, tempos: &[MusicMidiTempo]) -> f64 {
    let mut elapsed = 0_f64;
    let mut previous_tick = 0_u64;
    let mut current_tempo = 500_000_u32;
    for tempo in tempos {
        if tempo.tick > tick {
            break;
        }
        elapsed += (tempo.tick.saturating_sub(previous_tick)) as f64 * f64::from(current_tempo)
            / f64::from(ticks_per_quarter)
            / 1_000_000.0;
        previous_tick = tempo.tick;
        current_tempo = tempo.microseconds_per_quarter;
    }
    elapsed
        + (tick.saturating_sub(previous_tick)) as f64 * f64::from(current_tempo)
            / f64::from(ticks_per_quarter)
            / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn document() -> MusicMidiDocument {
        MusicMidiDocument {
            schema_version: 1,
            take_id: "take".into(),
            source_sha256: "a".repeat(64),
            revision: 0,
            ticks_per_quarter: 480,
            duration_ticks: 960,
            duration_seconds: 1.0,
            tempos: vec![MusicMidiTempo {
                tick: 0,
                microseconds_per_quarter: 500_000,
            }],
            time_signatures: vec![MusicMidiTimeSignature {
                tick: 0,
                numerator: 4,
                denominator: 4,
            }],
            tracks: vec![MusicMidiTrack {
                id: "track-1".into(),
                name: "Piano".into(),
                channel: 0,
                program: 0,
                muted: false,
                notes: vec![MusicMidiNote {
                    id: "note-1".into(),
                    pitch: 60,
                    start_tick: 0,
                    duration_ticks: 480,
                    velocity: 96,
                    channel: 0,
                }],
            }],
        }
    }

    #[test]
    fn typed_document_round_trips_through_standard_midi() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("edit.mid");
        let original = document();
        write_midi_document(&path, &original).unwrap();
        let parsed = parse_midi_document(&path, "take", &"a".repeat(64), 0).unwrap();
        assert_eq!(parsed.ticks_per_quarter, 480);
        assert_eq!(parsed.tracks.len(), 1);
        assert_eq!(parsed.tracks[0].name, "Piano");
        assert_eq!(parsed.tracks[0].program, 0);
        assert_eq!(parsed.tracks[0].notes[0].pitch, 60);
        assert_eq!(parsed.tracks[0].notes[0].duration_ticks, 480);
    }

    #[test]
    fn muted_tracks_are_omitted_from_the_editable_export_not_the_document() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("muted.mid");
        let mut original = document();
        original.tracks[0].muted = true;
        write_midi_document(&path, &original).unwrap();
        let parsed = parse_midi_document(&path, "take", &"a".repeat(64), 1).unwrap();
        assert!(parsed.tracks.is_empty());
        assert_eq!(original.tracks.len(), 1);
    }

    #[test]
    fn invalid_note_data_is_rejected_before_writing() {
        let mut invalid = document();
        invalid.tracks[0].notes[0].velocity = 0;
        assert!(validate_midi_document(&invalid).is_err());
    }
}
