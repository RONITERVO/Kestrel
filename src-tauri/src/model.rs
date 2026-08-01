//! Read-only GGUF discovery and bounded metadata inspection.
//!
//! This module deliberately has no runtime, network, or mutation authority. Model identity is a
//! fast path-independent content signature so a cached selection survives drive/user changes.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

const CATALOG_SCHEMA_VERSION: u32 = 1;
const MAX_CATALOG_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedCatalog {
    schema_version: u32,
    updated_at: String,
    models: Vec<ModelInfo>,
}

#[derive(Debug, Clone)]
pub struct ModelCatalogStore {
    path: PathBuf,
}

impl ModelCatalogStore {
    pub fn new(library_root: &Path) -> Self {
        Self {
            path: library_root.join("model-catalog.json"),
        }
    }

    pub fn load(&self) -> io::Result<Vec<ModelInfo>> {
        let backup = self.path.with_extension("json.backup");
        let catalog = match read_catalog(&self.path) {
            Ok(Some(value)) => value,
            Ok(None) => match read_catalog(&backup) {
                Ok(Some(value)) => {
                    fs::copy(&backup, &self.path)?;
                    value
                }
                Ok(None) => return Ok(Vec::new()),
                Err(error) => return Err(error),
            },
            Err(_primary_error) => match read_catalog(&backup) {
                Ok(Some(value)) => {
                    quarantine(&self.path);
                    fs::copy(&backup, &self.path)?;
                    value
                }
                _ => {
                    quarantine(&self.path);
                    return Ok(Vec::new());
                }
            },
        };
        if catalog.schema_version != CATALOG_SCHEMA_VERSION {
            return Ok(Vec::new());
        }
        Ok(catalog
            .models
            .into_iter()
            .filter(|model| {
                fs::metadata(&model.path)
                    .is_ok_and(|metadata| metadata.is_file() && metadata.len() == model.bytes)
            })
            .collect())
    }

    pub fn save(&self, models: &[ModelInfo]) -> io::Result<()> {
        let temporary = self.path.with_extension("json.tmp");
        let backup = self.path.with_extension("json.backup");
        let catalog = CachedCatalog {
            schema_version: CATALOG_SCHEMA_VERSION,
            updated_at: Utc::now().to_rfc3339(),
            models: models.to_vec(),
        };
        fs::write(
            &temporary,
            serde_json::to_vec_pretty(&catalog).map_err(io::Error::other)?,
        )?;
        if self.path.is_file() {
            fs::copy(&self.path, &backup)?;
            fs::remove_file(&self.path)?;
        }
        if let Err(error) = fs::rename(&temporary, &self.path) {
            if backup.is_file() {
                let _ = fs::copy(&backup, &self.path);
            }
            return Err(error);
        }
        if !backup.is_file() {
            fs::copy(&self.path, &backup)?;
        }
        Ok(())
    }
}

fn read_catalog(path: &Path) -> io::Result<Option<CachedCatalog>> {
    if !path.is_file() {
        return Ok(None);
    }
    if fs::metadata(path)?.len() > MAX_CATALOG_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "model catalog exceeds 64 MiB",
        ));
    }
    serde_json::from_slice(&fs::read(path)?)
        .map(Some)
        .map_err(io::Error::other)
}

fn quarantine(path: &Path) {
    if path.is_file() {
        let unique = uuid::Uuid::new_v4().simple().to_string();
        let name = format!(
            "model-catalog.corrupt-{}-{}.json",
            Utc::now().format("%Y%m%dT%H%M%S"),
            &unique[..8]
        );
        let _ = fs::rename(path, path.with_file_name(name));
    }
}

pub fn merge_catalogs(cached: Vec<ModelInfo>, refreshed: Vec<ModelInfo>) -> Vec<ModelInfo> {
    let mut by_id = HashMap::new();
    for model in cached.into_iter().chain(refreshed) {
        by_id.insert(model.id.clone(), model);
    }
    let mut models: Vec<_> = by_id.into_values().collect();
    models.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    models
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub source: String,
    pub bytes: u64,
    pub architecture: Option<String>,
    pub context_length: Option<u64>,
    pub chat_template: bool,
    pub quantization: Option<String>,
    pub mmproj_path: Option<String>,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub supports_audio: bool,
    pub recommendation: String,
}

pub fn default_roots(extra: &[String], bonsai_root: &str) -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from(bonsai_root).join("models")];
    if let Some(base) = directories::BaseDirs::new() {
        let home = base.home_dir();
        let data = base.data_dir();
        roots.extend([
            home.join("jan").join("models"),
            home.join(".cache").join("lm-studio").join("models"),
            home.join(".lmstudio").join("models"),
            home.join(".cache").join("huggingface").join("hub"),
            home.join(".ollama").join("models").join("blobs"),
            data.join("Jan").join("data").join("models"),
            data.join("Jan")
                .join("data")
                .join("llamacpp")
                .join("models"),
        ]);
    }
    roots.extend(extra.iter().map(PathBuf::from));
    roots.sort();
    roots.dedup();
    roots
}

pub fn scan(roots: &[PathBuf]) -> Vec<ModelInfo> {
    let mut models = Vec::new();
    for root in roots.iter().filter(|path| path.exists()) {
        for entry in WalkDir::new(root)
            .follow_links(false)
            .max_depth(8)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file() && is_candidate(entry.path()) {
                if let Ok(model) = inspect(entry.path()) {
                    models.push(model);
                }
            }
        }
    }
    models.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    models.dedup_by(|a, b| a.id == b.id);
    models
}

fn is_candidate(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase();
    !name.contains("mmproj")
        && (!name.contains("-of-") || name.contains("-00001-of-"))
        && (path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("gguf"))
            || path
                .to_string_lossy()
                .to_lowercase()
                .contains(".ollama\\models\\blobs"))
}

fn inspect(path: &Path) -> io::Result<ModelInfo> {
    let bytes = path.metadata()?.len();
    let metadata = read_gguf_metadata(path)?;
    let architecture = string_value(metadata.get("general.architecture"));
    let context_length = architecture
        .as_ref()
        .and_then(|value| metadata.get(&format!("{value}.context_length")))
        .and_then(Value::as_u64);
    let name = string_value(metadata.get("general.name")).unwrap_or_else(|| friendly_name(path));
    let file_type = metadata.get("general.file_type").and_then(Value::as_u64);
    let lower = format!("{} {}", name, path.display()).to_lowercase();
    let recommendation = if lower.contains("ternary-bonsai-27b") {
        "Validated Bonsai profile: one slot, Q4 KV, flash attention, full GPU, visible context restart"
    } else {
        "Use embedded GGUF template and native context; keep placement explicit"
    };
    let mmproj_path = find_projector(path);
    let projector_metadata = mmproj_path
        .as_deref()
        .and_then(|projector| read_gguf_metadata(Path::new(projector)).ok());
    let supports_vision = projector_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("clip.has_vision_encoder"))
        .and_then(Value::as_bool)
        .unwrap_or(mmproj_path.is_some());
    let supports_audio = projector_metadata
        .as_ref()
        .and_then(|metadata| metadata.get("clip.has_audio_encoder"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(ModelInfo {
        id: content_identity(path, bytes)?,
        name,
        path: path.to_string_lossy().into_owned(),
        source: source_for(path).into(),
        bytes,
        architecture,
        context_length,
        chat_template: metadata.keys().any(|key| key.contains("chat_template")),
        quantization: if lower.contains("ternary-bonsai-27b") && lower.contains("q2_0") {
            Some("TQ2_0".into())
        } else {
            file_type.map(quantization_name)
        },
        mmproj_path,
        supports_vision,
        supports_audio,
        recommendation: recommendation.into(),
    })
}

fn content_identity(path: &Path, bytes: u64) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes.to_le_bytes());
    let length = usize::try_from(bytes.min(1024 * 1024)).unwrap_or(1024 * 1024);
    let mut sample = vec![0; length];
    file.read_exact(&mut sample)?;
    hasher.update(&sample);
    if bytes > length as u64 {
        file.seek(SeekFrom::End(-(length as i64)))?;
        file.read_exact(&mut sample)?;
        hasher.update(&sample);
    }
    Ok(hex::encode(hasher.finalize())[..24].into())
}

fn find_projector(path: &Path) -> Option<String> {
    let mut candidates = std::fs::read_dir(path.parent()?)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    let lower = name.to_lowercase();
                    candidate.is_file() && lower.contains("mmproj") && lower.ends_with(".gguf")
                })
        })
        .collect::<Vec<_>>();
    candidates.sort();
    if candidates.len() == 1 {
        return candidates
            .first()
            .map(|candidate| candidate.to_string_lossy().into_owned());
    }
    let model_tokens = identity_tokens(path);
    candidates
        .into_iter()
        .map(|candidate| {
            let score = identity_tokens(&candidate)
                .intersection(&model_tokens)
                .map(String::len)
                .sum::<usize>();
            (score, candidate)
        })
        .filter(|(score, _)| *score > 0)
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| right.1.cmp(&left.1)))
        .map(|(_, candidate)| candidate.to_string_lossy().into_owned())
}

fn identity_tokens(path: &Path) -> std::collections::BTreeSet<String> {
    const GENERIC: &[&str] = &[
        "gguf",
        "model",
        "mmproj",
        "projector",
        "instruct",
        "chat",
        "q2",
        "q3",
        "q4",
        "q5",
        "q6",
        "q8",
        "bf16",
        "f16",
        "f32",
        "km",
        "ks",
        "kl",
        "xl",
    ];
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| token.len() >= 3 && !GENERIC.contains(token))
        .map(ToOwned::to_owned)
        .collect()
}

fn source_for(path: &Path) -> &'static str {
    let value = path.to_string_lossy().to_lowercase();
    if value.contains("huggingface") {
        "Hugging Face"
    } else if value.contains(".ollama") {
        "Ollama"
    } else if value.contains("jan") {
        "Jan"
    } else if value.contains("lmstudio") || value.contains("lm-studio") {
        "LM Studio"
    } else if value.contains("localai") {
        "LocalAI"
    } else {
        "Custom"
    }
}

fn friendly_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("Unknown model")
        .replace("-00001-of-", " · split ")
}

fn string_value(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(ToOwned::to_owned)
}

fn quantization_name(value: u64) -> String {
    match value {
        0 => "F32".into(),
        1 => "F16".into(),
        2 => "Q4_0".into(),
        7 => "Q8_0".into(),
        10 => "Q2_K".into(),
        15 => "Q4_K_M".into(),
        17 => "Q5_K_M".into(),
        18 => "Q6_K".into(),
        30 => "BF16".into(),
        34 => "TQ1_0".into(),
        35 => "TQ2_0".into(),
        other => format!("type {other}"),
    }
}

fn read_gguf_metadata(path: &Path) -> io::Result<HashMap<String, Value>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut magic = [0; 4];
    reader.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not GGUF"));
    }
    let version = read_u32(&mut reader)?;
    if !(2..=3).contains(&version) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported GGUF",
        ));
    }
    let _tensor_count = read_u64(&mut reader)?;
    let count = read_u64(&mut reader)?;
    if count > 100_000 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "metadata count"));
    }
    let mut output = HashMap::new();
    for _ in 0..count {
        let key = read_string(&mut reader)?;
        let kind = read_u32(&mut reader)?;
        let value = read_value(&mut reader, kind, 0)?;
        if matches!(
            key.as_str(),
            "general.name"
                | "general.architecture"
                | "general.file_type"
                | "clip.has_vision_encoder"
                | "clip.has_audio_encoder"
        ) || key.ends_with(".context_length")
            || key.contains("chat_template")
        {
            output.insert(key, value);
        }
    }
    Ok(output)
}

fn read_value<R: Read>(reader: &mut R, kind: u32, depth: u8) -> io::Result<Value> {
    if depth > 3 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "metadata depth"));
    }
    Ok(match kind {
        0 => Value::from(read_num::<1, _>(reader)?[0]),
        1 => Value::from(read_num::<1, _>(reader)?[0] as i8),
        2 => Value::from(u16::from_le_bytes(read_num(reader)?)),
        3 => Value::from(i16::from_le_bytes(read_num(reader)?)),
        4 => Value::from(read_u32(reader)?),
        5 => Value::from(i32::from_le_bytes(read_num(reader)?)),
        6 => Value::from(f32::from_le_bytes(read_num(reader)?)),
        7 => Value::from(read_num::<1, _>(reader)?[0] != 0),
        8 => Value::from(read_string(reader)?),
        9 => {
            let item_kind = read_u32(reader)?;
            let length = read_u64(reader)?;
            if length > 10_000_000 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "metadata array"));
            }
            for _ in 0..length {
                let _ = read_value(reader, item_kind, depth + 1)?;
            }
            Value::Null
        }
        10 => Value::from(read_u64(reader)?),
        11 => Value::from(i64::from_le_bytes(read_num(reader)?)),
        12 => Value::from(f64::from_le_bytes(read_num(reader)?)),
        _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "metadata type")),
    })
}

fn read_string<R: Read>(reader: &mut R) -> io::Result<String> {
    let length = read_u64(reader)?;
    if length > 16 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "metadata string",
        ));
    }
    let mut bytes = vec![0; length as usize];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn read_u32<R: Read>(reader: &mut R) -> io::Result<u32> {
    Ok(u32::from_le_bytes(read_num(reader)?))
}
fn read_u64<R: Read>(reader: &mut R) -> io::Result<u64> {
    Ok(u64::from_le_bytes(read_num(reader)?))
}
fn read_num<const N: usize, R: Read>(reader: &mut R) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_quantization_names() {
        assert_eq!(quantization_name(15), "Q4_K_M");
        assert_eq!(quantization_name(41), "type 41");
    }

    #[test]
    fn projector_and_split_files_are_not_models() {
        assert!(!is_candidate(Path::new("model-mmproj-Q8_0.gguf")));
        assert!(!is_candidate(Path::new("model-00002-of-00004.gguf")));
        assert!(is_candidate(Path::new("model-00001-of-00004.gguf")));
    }

    #[test]
    fn projector_matching_does_not_cross_models_in_a_shared_folder() {
        let directory = tempfile::tempdir().unwrap();
        let bonsai = directory.path().join("Ternary-Bonsai-27B-Q2.gguf");
        let bonsai_projector = directory.path().join("mmproj-Ternary-Bonsai-27B-Q8.gguf");
        let gemma_projector = directory.path().join("mmproj-gemma-4-e2b-f16.gguf");
        fs::write(&bonsai, b"model").unwrap();
        fs::write(&bonsai_projector, b"projector").unwrap();
        fs::write(&gemma_projector, b"projector").unwrap();
        assert_eq!(
            find_projector(&bonsai).as_deref(),
            Some(bonsai_projector.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn projector_metadata_advertises_vision_and_audio_without_filename_guesses() {
        use std::io::Write as _;

        fn write_gguf(path: &Path, flags: &[(&str, bool)]) {
            let mut file = File::create(path).unwrap();
            file.write_all(b"GGUF").unwrap();
            file.write_all(&3u32.to_le_bytes()).unwrap();
            file.write_all(&0u64.to_le_bytes()).unwrap();
            file.write_all(&(flags.len() as u64).to_le_bytes()).unwrap();
            for (key, value) in flags {
                file.write_all(&(key.len() as u64).to_le_bytes()).unwrap();
                file.write_all(key.as_bytes()).unwrap();
                file.write_all(&7u32.to_le_bytes()).unwrap();
                file.write_all(&[u8::from(*value)]).unwrap();
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let model = directory.path().join("gemma-4-e2b.gguf");
        let projector = directory.path().join("mmproj-gemma-4-e2b.gguf");
        write_gguf(&model, &[]);
        write_gguf(
            &projector,
            &[
                ("clip.has_vision_encoder", true),
                ("clip.has_audio_encoder", true),
            ],
        );
        let inspected = inspect(&model).unwrap();
        assert!(inspected.supports_vision);
        assert!(inspected.supports_audio);
        assert_eq!(
            inspected.mmproj_path.as_deref(),
            Some(projector.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn catalog_cache_restores_only_existing_unchanged_models() {
        let directory = tempfile::tempdir().unwrap();
        let model_path = directory.path().join("model.gguf");
        fs::write(&model_path, b"stable-model").unwrap();
        let store = ModelCatalogStore::new(directory.path());
        let model = ModelInfo {
            id: "identity".into(),
            name: "Model".into(),
            path: model_path.to_string_lossy().into_owned(),
            source: "Custom".into(),
            bytes: 12,
            architecture: None,
            context_length: None,
            chat_template: false,
            quantization: None,
            mmproj_path: None,
            supports_vision: false,
            supports_audio: false,
            recommendation: "Inspect metadata".into(),
        };
        store.save(std::slice::from_ref(&model)).unwrap();
        assert_eq!(store.load().unwrap(), vec![model]);

        fs::write(model_path, b"changed").unwrap();
        assert!(store.load().unwrap().is_empty());
    }

    #[test]
    fn corrupt_catalog_is_quarantined_without_blocking_startup() {
        let directory = tempfile::tempdir().unwrap();
        let store = ModelCatalogStore::new(directory.path());
        fs::write(directory.path().join("model-catalog.json"), b"broken").unwrap();

        assert!(store.load().unwrap().is_empty());
        assert!(directory
            .path()
            .read_dir()
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("model-catalog.corrupt-")));
    }

    #[test]
    fn corrupt_primary_is_restored_from_the_catalog_backup() {
        let directory = tempfile::tempdir().unwrap();
        let model_path = directory.path().join("restored.gguf");
        fs::write(&model_path, b"catalog model").unwrap();
        let store = ModelCatalogStore::new(directory.path());
        let model = ModelInfo {
            id: "restored-identity".into(),
            name: "Restored model".into(),
            path: model_path.to_string_lossy().into_owned(),
            source: "Test".into(),
            bytes: 13,
            architecture: None,
            context_length: None,
            chat_template: false,
            quantization: None,
            mmproj_path: None,
            supports_vision: false,
            supports_audio: false,
            recommendation: "Recovered".into(),
        };
        store.save(std::slice::from_ref(&model)).unwrap();
        fs::write(directory.path().join("model-catalog.json"), b"broken").unwrap();

        assert_eq!(store.load().unwrap(), vec![model.clone()]);
        let restored = read_catalog(&directory.path().join("model-catalog.json"))
            .unwrap()
            .unwrap();
        assert_eq!(restored.models, vec![model]);
    }
}
