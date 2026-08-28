use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use thiserror::Error;

const LIBRARY_REVISION: u32 = 1;
const DEFAULT_VOICE_ID: &str = "voice-default";
const MAX_REFERENCE_BYTES: usize = 32 * 1024 * 1024;
const MIN_REFERENCE_SECONDS: f64 = 3.0;
const MAX_REFERENCE_SECONDS: f64 = 45.0;

#[derive(Debug, Error)]
pub enum VoiceLibraryError {
    #[error("voice library file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("voice library JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("voice library request is invalid: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceProfile {
    pub id: String,
    pub name: String,
    pub language: String,
    pub tags: Vec<String>,
    pub source: String,
    pub consent_confirmed: bool,
    pub performance: String,
    pub reference_relative_path: Option<String>,
    pub reference_sha256: Option<String>,
    pub reference_seconds: Option<f64>,
    pub original_file_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl VoiceProfile {
    pub fn built_in() -> Self {
        Self {
            id: DEFAULT_VOICE_ID.into(),
            name: "Chatterbox Default".into(),
            language: "Auto".into(),
            tags: vec!["Built in".into(), "Neutral".into()],
            source: "built-in".into(),
            consent_confirmed: true,
            performance: "natural".into(),
            reference_relative_path: None,
            reference_sha256: None,
            reference_seconds: None,
            original_file_name: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    pub fn is_built_in(&self) -> bool {
        self.id == DEFAULT_VOICE_ID
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceLibrarySnapshot {
    pub profiles: Vec<VoiceProfile>,
    pub default_profile_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVoiceProfileRequest {
    pub name: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub source: String,
    pub consent_confirmed: bool,
    #[serde(default = "default_performance")]
    pub performance: String,
    pub audio_base64: String,
    pub mime_type: String,
    pub original_file_name: String,
    pub duration_seconds: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateVoiceProfileRequest {
    pub id: String,
    pub name: String,
    pub language: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub consent_confirmed: bool,
    pub performance: String,
}

#[derive(Debug, Clone)]
pub struct VoiceConditioning {
    pub profile_id: String,
    pub reference_path: Option<PathBuf>,
    pub reference_sha256: Option<String>,
    pub performance: String,
}

impl VoiceConditioning {
    pub fn fingerprint(&self) -> &str {
        self.reference_sha256
            .as_deref()
            .unwrap_or("built-in-default")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct StoredVoiceLibrary {
    revision: u32,
    default_profile_id: String,
    profiles: Vec<VoiceProfile>,
}

impl Default for StoredVoiceLibrary {
    fn default() -> Self {
        Self {
            revision: LIBRARY_REVISION,
            default_profile_id: DEFAULT_VOICE_ID.into(),
            profiles: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct VoiceLibrary {
    speech_root: PathBuf,
    library_path: PathBuf,
    objects_root: PathBuf,
}

impl VoiceLibrary {
    pub fn new(library_root: &Path) -> Result<Self, VoiceLibraryError> {
        let speech_root = library_root.join("speech-cache");
        let voices_root = speech_root.join("voices");
        let objects_root = voices_root.join("objects");
        fs::create_dir_all(&objects_root)?;
        Ok(Self {
            speech_root,
            library_path: voices_root.join("library.json"),
            objects_root,
        })
    }

    pub fn snapshot(&self) -> Result<VoiceLibrarySnapshot, VoiceLibraryError> {
        let stored = self.load()?;
        let mut profiles = vec![VoiceProfile::built_in()];
        profiles.extend(stored.profiles);
        profiles.sort_by(|left, right| {
            right
                .is_built_in()
                .cmp(&left.is_built_in())
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
        });
        let default_profile_id = if profiles
            .iter()
            .any(|profile| profile.id == stored.default_profile_id)
        {
            stored.default_profile_id
        } else {
            DEFAULT_VOICE_ID.into()
        };
        Ok(VoiceLibrarySnapshot {
            profiles,
            default_profile_id,
        })
    }

    pub fn resolve(&self, profile_id: &str) -> Result<VoiceConditioning, VoiceLibraryError> {
        if profile_id == DEFAULT_VOICE_ID {
            return Ok(VoiceConditioning {
                profile_id: DEFAULT_VOICE_ID.into(),
                reference_path: None,
                reference_sha256: None,
                performance: "natural".into(),
            });
        }
        let stored = self.load()?;
        let profile = stored
            .profiles
            .into_iter()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| {
                VoiceLibraryError::Invalid("the selected voice no longer exists".into())
            })?;
        let relative = profile.reference_relative_path.as_deref().ok_or_else(|| {
            VoiceLibraryError::Invalid("the selected voice has no reference recording".into())
        })?;
        let path = self.speech_root.join(relative);
        let canonical_root = fs::canonicalize(&self.speech_root)?;
        let canonical_path = fs::canonicalize(&path).map_err(|_| {
            VoiceLibraryError::Invalid("the selected voice reference is missing".into())
        })?;
        if !canonical_path.starts_with(canonical_root) || !canonical_path.is_file() {
            return Err(VoiceLibraryError::Invalid(
                "the selected voice reference is outside Kestrel's private voice library".into(),
            ));
        }
        let actual_sha = sha256_file(&canonical_path)?;
        if profile.reference_sha256.as_deref() != Some(actual_sha.as_str()) {
            return Err(VoiceLibraryError::Invalid(
                "the selected voice reference failed its integrity check".into(),
            ));
        }
        Ok(VoiceConditioning {
            profile_id: profile.id,
            reference_path: Some(canonical_path),
            reference_sha256: Some(actual_sha),
            performance: profile.performance,
        })
    }

    pub fn create(
        &self,
        request: CreateVoiceProfileRequest,
    ) -> Result<VoiceLibrarySnapshot, VoiceLibraryError> {
        let name = validate_name(&request.name)?;
        let language = validate_language(&request.language)?;
        let tags = validate_tags(request.tags)?;
        let performance = validate_performance(&request.performance)?;
        if !request.consent_confirmed {
            return Err(VoiceLibraryError::Invalid(
                "confirm that you own the recording or have permission to create this voice".into(),
            ));
        }
        if request.source != "recorded" && request.source != "imported" {
            return Err(VoiceLibraryError::Invalid(
                "voice source must be recorded or imported".into(),
            ));
        }
        if !request.duration_seconds.is_finite()
            || !(MIN_REFERENCE_SECONDS..=MAX_REFERENCE_SECONDS).contains(&request.duration_seconds)
        {
            return Err(VoiceLibraryError::Invalid(format!(
                "voice references must be {MIN_REFERENCE_SECONDS:.0}-{MAX_REFERENCE_SECONDS:.0} seconds long (received {:.1}s)",
                request.duration_seconds
            )));
        }
        if request.audio_base64.len() > MAX_REFERENCE_BYTES * 4 / 3 + 16 {
            return Err(VoiceLibraryError::Invalid(
                "voice reference exceeds the 32 MiB local safety limit".into(),
            ));
        }
        let bytes = STANDARD
            .decode(request.audio_base64.as_bytes())
            .map_err(|_| {
                VoiceLibraryError::Invalid("voice reference is not valid base64 audio".into())
            })?;
        if bytes.is_empty() || bytes.len() > MAX_REFERENCE_BYTES {
            return Err(VoiceLibraryError::Invalid(
                "voice reference is empty or exceeds the 32 MiB local safety limit".into(),
            ));
        }
        let extension = detect_audio_extension(&bytes, &request.mime_type).ok_or_else(|| {
            VoiceLibraryError::Invalid(
                "use a WAV, FLAC, MP3, Ogg/Opus, WebM, or M4A voice recording".into(),
            )
        })?;
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let object_path = self.objects_root.join(format!("{sha256}.{extension}"));
        if !object_path.is_file() {
            write_bytes_atomic(&object_path, &bytes)?;
        }
        let relative_path = object_path
            .strip_prefix(&self.speech_root)
            .map_err(|_| VoiceLibraryError::Invalid("voice object escaped its library".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        let now = Utc::now().to_rfc3339();
        let profile = VoiceProfile {
            id: format!("voice-{}", uuid::Uuid::new_v4().simple()),
            name,
            language,
            tags,
            source: request.source,
            consent_confirmed: true,
            performance,
            reference_relative_path: Some(relative_path),
            reference_sha256: Some(sha256),
            reference_seconds: Some(request.duration_seconds),
            original_file_name: Some(safe_original_name(&request.original_file_name)),
            created_at: now.clone(),
            updated_at: now,
        };
        let mut stored = self.load()?;
        stored.profiles.push(profile);
        self.save(&stored)?;
        self.snapshot()
    }

    pub fn update(
        &self,
        request: UpdateVoiceProfileRequest,
    ) -> Result<VoiceLibrarySnapshot, VoiceLibraryError> {
        if request.id == DEFAULT_VOICE_ID {
            return Err(VoiceLibraryError::Invalid(
                "the built-in Chatterbox voice cannot be edited".into(),
            ));
        }
        let name = validate_name(&request.name)?;
        let language = validate_language(&request.language)?;
        let tags = validate_tags(request.tags)?;
        let performance = validate_performance(&request.performance)?;
        if !request.consent_confirmed {
            return Err(VoiceLibraryError::Invalid(
                "custom voices must retain their rights confirmation".into(),
            ));
        }
        let mut stored = self.load()?;
        let profile = stored
            .profiles
            .iter_mut()
            .find(|profile| profile.id == request.id)
            .ok_or_else(|| VoiceLibraryError::Invalid("voice profile was not found".into()))?;
        profile.name = name;
        profile.language = language;
        profile.tags = tags;
        profile.performance = performance;
        profile.updated_at = Utc::now().to_rfc3339();
        self.save(&stored)?;
        self.snapshot()
    }

    pub fn set_default(&self, profile_id: &str) -> Result<VoiceLibrarySnapshot, VoiceLibraryError> {
        let mut stored = self.load()?;
        if profile_id != DEFAULT_VOICE_ID
            && !stored
                .profiles
                .iter()
                .any(|profile| profile.id == profile_id)
        {
            return Err(VoiceLibraryError::Invalid(
                "voice profile was not found".into(),
            ));
        }
        stored.default_profile_id = profile_id.into();
        self.save(&stored)?;
        self.snapshot()
    }

    pub fn delete(&self, profile_id: &str) -> Result<VoiceLibrarySnapshot, VoiceLibraryError> {
        if profile_id == DEFAULT_VOICE_ID {
            return Err(VoiceLibraryError::Invalid(
                "the built-in Chatterbox voice cannot be deleted".into(),
            ));
        }
        let mut stored = self.load()?;
        let index = stored
            .profiles
            .iter()
            .position(|profile| profile.id == profile_id)
            .ok_or_else(|| VoiceLibraryError::Invalid("voice profile was not found".into()))?;
        let removed = stored.profiles.remove(index);
        if stored.default_profile_id == profile_id {
            stored.default_profile_id = DEFAULT_VOICE_ID.into();
        }
        self.save(&stored)?;
        if let (Some(relative), Some(hash)) =
            (removed.reference_relative_path, removed.reference_sha256)
        {
            let still_used = stored
                .profiles
                .iter()
                .any(|profile| profile.reference_sha256.as_deref() == Some(hash.as_str()));
            if !still_used {
                let target = self.speech_root.join(relative);
                if let (Ok(target), Ok(objects)) = (
                    fs::canonicalize(target),
                    fs::canonicalize(&self.objects_root),
                ) {
                    if target.starts_with(objects) && target.is_file() {
                        let _ = fs::remove_file(target);
                    }
                }
            }
        }
        self.snapshot()
    }

    fn load(&self) -> Result<StoredVoiceLibrary, VoiceLibraryError> {
        let Some(bytes) = read_recoverable(&self.library_path)? else {
            return Ok(StoredVoiceLibrary::default());
        };
        let stored: StoredVoiceLibrary = match serde_json::from_slice(&bytes) {
            Ok(stored) => stored,
            Err(primary_error) => {
                let backup = recovery_path(&self.library_path);
                let backup_bytes = fs::read(&backup).map_err(|_| primary_error)?;
                let stored = serde_json::from_slice(&backup_bytes)?;
                fs::copy(backup, &self.library_path)?;
                stored
            }
        };
        if stored.revision > LIBRARY_REVISION {
            return Err(VoiceLibraryError::Invalid(format!(
                "voice library revision {} is newer than this Kestrel release",
                stored.revision
            )));
        }
        Ok(stored)
    }

    fn save(&self, stored: &StoredVoiceLibrary) -> Result<(), VoiceLibraryError> {
        write_bytes_recoverable(&self.library_path, &serde_json::to_vec_pretty(stored)?)
    }
}

fn default_language() -> String {
    "Auto".into()
}

fn default_performance() -> String {
    "natural".into()
}

fn validate_name(value: &str) -> Result<String, VoiceLibraryError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 80 || value.chars().any(char::is_control) {
        return Err(VoiceLibraryError::Invalid(
            "voice name must contain 1-80 printable characters".into(),
        ));
    }
    Ok(value.into())
}

fn validate_language(value: &str) -> Result<String, VoiceLibraryError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 40 || value.chars().any(char::is_control) {
        return Err(VoiceLibraryError::Invalid(
            "voice language must contain 1-40 printable characters".into(),
        ));
    }
    Ok(value.into())
}

fn validate_tags(values: Vec<String>) -> Result<Vec<String>, VoiceLibraryError> {
    if values.len() > 8 {
        return Err(VoiceLibraryError::Invalid(
            "a voice can have at most eight tags".into(),
        ));
    }
    let mut tags = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || value.chars().count() > 32 || value.chars().any(char::is_control) {
            return Err(VoiceLibraryError::Invalid(
                "voice tags must contain 1-32 printable characters".into(),
            ));
        }
        if !tags
            .iter()
            .any(|tag: &String| tag.eq_ignore_ascii_case(value))
        {
            tags.push(value.into());
        }
    }
    Ok(tags)
}

fn validate_performance(value: &str) -> Result<String, VoiceLibraryError> {
    match value {
        "restrained" | "natural" | "expressive" | "dramatic" => Ok(value.into()),
        _ => Err(VoiceLibraryError::Invalid(
            "voice performance must be restrained, natural, expressive, or dramatic".into(),
        )),
    }
}

fn safe_original_name(value: &str) -> String {
    Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("voice-reference")
        .chars()
        .filter(|character| !character.is_control())
        .take(160)
        .collect()
}

fn detect_audio_extension(bytes: &[u8], _mime_type: &str) -> Option<&'static str> {
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        return Some("wav");
    }
    if bytes.starts_with(b"fLaC") {
        return Some("flac");
    }
    if bytes.starts_with(b"OggS") {
        return Some("ogg");
    }
    if bytes.starts_with(b"ID3")
        || bytes
            .get(..2)
            .is_some_and(|prefix| prefix[0] == 0xff && prefix[1] & 0xe0 == 0xe0)
    {
        return Some("mp3");
    }
    if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Some("webm");
    }
    if bytes.get(4..8) == Some(b"ftyp") {
        return Some("m4a");
    }
    None
}

fn sha256_file(path: &Path) -> Result<String, VoiceLibraryError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

fn recovery_path(path: &Path) -> PathBuf {
    path.with_extension("json.recovery")
}

fn read_recoverable(path: &Path) -> Result<Option<Vec<u8>>, VoiceLibraryError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::read(recovery_path(path)) {
                Ok(bytes) => {
                    let _ = fs::copy(recovery_path(path), path);
                    Ok(Some(bytes))
                }
                Err(recovery) if recovery.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(recovery) => Err(recovery.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), VoiceLibraryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!(
        "{}.tmp-{}",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("data"),
        uuid::Uuid::new_v4().simple()
    ));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        if path.is_file() {
            return Ok(());
        }
        return Err(error.into());
    }
    Ok(())
}

fn write_bytes_recoverable(path: &Path, bytes: &[u8]) -> Result<(), VoiceLibraryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let backup = recovery_path(path);
    if path.is_file() {
        fs::copy(path, &backup)?;
    }
    let temporary = path.with_extension(format!("json.tmp-{}", uuid::Uuid::new_v4().simple()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if path.is_file() {
        fs::remove_file(path)?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        if backup.is_file() {
            let _ = fs::copy(&backup, path);
        }
        return Err(error.into());
    }
    fs::copy(path, backup)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav_bytes() -> Vec<u8> {
        let mut bytes = b"RIFF\x24\0\0\0WAVEfmt ".to_vec();
        bytes.extend_from_slice(&[0; 48]);
        bytes
    }

    fn request(name: &str) -> CreateVoiceProfileRequest {
        CreateVoiceProfileRequest {
            name: name.into(),
            language: "English".into(),
            tags: vec!["Warm".into()],
            source: "imported".into(),
            consent_confirmed: true,
            performance: "natural".into(),
            audio_base64: STANDARD.encode(wav_bytes()),
            mime_type: "audio/wav".into(),
            original_file_name: "narrator.wav".into(),
            duration_seconds: 12.0,
        }
    }

    #[test]
    fn creates_content_addressed_voice_and_restores_default() {
        let directory = tempfile::tempdir().unwrap();
        let library = VoiceLibrary::new(directory.path()).unwrap();
        let created = library.create(request("Evening Narrator")).unwrap();
        assert_eq!(created.profiles.len(), 2);
        let custom = created
            .profiles
            .iter()
            .find(|profile| !profile.is_built_in())
            .unwrap();
        let resolved = library.resolve(&custom.id).unwrap();
        assert!(resolved.reference_path.unwrap().is_file());
        let selected = library.set_default(&custom.id).unwrap();
        assert_eq!(selected.default_profile_id, custom.id);
        let after_delete = library.delete(&custom.id).unwrap();
        assert_eq!(after_delete.default_profile_id, DEFAULT_VOICE_ID);
        assert_eq!(after_delete.profiles.len(), 1);
    }

    #[test]
    fn rejects_unconfirmed_or_disguised_references() {
        let directory = tempfile::tempdir().unwrap();
        let library = VoiceLibrary::new(directory.path()).unwrap();
        let mut unconfirmed = request("No consent");
        unconfirmed.consent_confirmed = false;
        assert!(library.create(unconfirmed).is_err());
        let mut disguised = request("Not audio");
        disguised.audio_base64 = STANDARD.encode(b"not actually audio");
        disguised.mime_type = "application/octet-stream".into();
        assert!(library.create(disguised).is_err());
    }

    #[test]
    fn integrity_failure_blocks_conditioning() {
        let directory = tempfile::tempdir().unwrap();
        let library = VoiceLibrary::new(directory.path()).unwrap();
        let snapshot = library.create(request("Narrator")).unwrap();
        let custom = snapshot
            .profiles
            .iter()
            .find(|profile| !profile.is_built_in())
            .unwrap();
        let path = library.resolve(&custom.id).unwrap().reference_path.unwrap();
        fs::write(path, b"changed").unwrap();
        assert!(library.resolve(&custom.id).is_err());
    }

    #[test]
    fn rejects_out_of_range_durations_with_explicit_received_length() {
        let directory = tempfile::tempdir().unwrap();
        let library = VoiceLibrary::new(directory.path()).unwrap();
        let mut too_short = request("Too Short");
        too_short.duration_seconds = 2.5;
        let short_err = library.create(too_short).unwrap_err().to_string();
        assert!(short_err.contains("voice references must be 3-45 seconds long (received 2.5s)"));

        let mut too_long = request("Too Long");
        too_long.duration_seconds = 63.4;
        let long_err = library.create(too_long).unwrap_err().to_string();
        assert!(long_err.contains("voice references must be 3-45 seconds long (received 63.4s)"));
    }
}
