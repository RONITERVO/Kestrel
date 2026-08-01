//! Durable, fully local context attachments.
//!
//! Files selected by the user are copied into a content-addressed object store before a model
//! sees them. Images and audio use llama.cpp's native multimodal content blocks when the selected
//! projector advertises that modality. Documents are converted to bounded plain text locally;
//! arbitrary binaries remain durable and identifiable without pretending the model read them.

use crate::model::ModelInfo;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use quick_xml::{events::Event, Reader};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_NATIVE_MEDIA_BYTES: u64 = 32 * 1024 * 1024;
const MAX_NATIVE_MEDIA_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_MEDIA_CACHE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EXTRACTED_CHARS: usize = 2_000_000;
const MAX_PLAIN_TEXT_BYTES: usize = MAX_EXTRACTED_CHARS * 4 + 4;
const MAX_XML_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
const READ_CHUNK: usize = 64 * 1024;

#[derive(Clone)]
struct CachedMedia {
    content: Arc<Value>,
    encoded_bytes: u64,
}

#[derive(Default)]
pub(crate) struct MediaCache {
    entries: HashMap<String, CachedMedia>,
    order: VecDeque<String>,
    encoded_bytes: u64,
}

impl MediaCache {
    fn get(&mut self, id: &str) -> Option<Arc<Value>> {
        let content = Arc::clone(&self.entries.get(id)?.content);
        self.order.retain(|known| known != id);
        self.order.push_back(id.to_string());
        Some(content)
    }

    fn insert(&mut self, id: String, encoded_bytes: u64, content: Arc<Value>) {
        if encoded_bytes > MAX_MEDIA_CACHE_BYTES {
            return;
        }
        if let Some(replaced) = self.entries.remove(&id) {
            self.encoded_bytes = self.encoded_bytes.saturating_sub(replaced.encoded_bytes);
            self.order.retain(|known| known != &id);
        }
        while self.encoded_bytes.saturating_add(encoded_bytes) > MAX_MEDIA_CACHE_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.encoded_bytes = self.encoded_bytes.saturating_sub(removed.encoded_bytes);
            }
        }
        self.encoded_bytes = self.encoded_bytes.saturating_add(encoded_bytes);
        self.order.push_back(id.clone());
        self.entries.insert(
            id,
            CachedMedia {
                content,
                encoded_bytes,
            },
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContextAttachment {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub mime_type: String,
    pub bytes: u64,
    pub sha256: String,
    pub stored_path: String,
    pub extracted_chars: usize,
    pub context_mode: String,
    pub note: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct PreparedAttachments {
    pub content: Value,
    pub notice: Option<String>,
}

#[derive(Clone)]
pub struct AttachmentStore {
    root: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl AttachmentStore {
    pub fn new(workspace_root: &Path) -> Result<Self, String> {
        let root = workspace_root.join("attachments");
        for child in ["objects", "text", "meta"] {
            fs::create_dir_all(root.join(child)).map_err(|error| error.to_string())?;
        }
        Ok(Self {
            root,
            write_lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn import_path(&self, source: &Path) -> Result<ContextAttachment, String> {
        let source = source
            .canonicalize()
            .map_err(|error| format!("Cannot open {}: {error}", source.display()))?;
        let metadata = source
            .metadata()
            .map_err(|error| format!("Cannot inspect {}: {error}", source.display()))?;
        if !metadata.is_file() {
            return Err(format!(
                "Only regular files can be attached: {}",
                source.display()
            ));
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(format!(
                "{} is larger than Kestrel's 128 MiB per-file safety limit.",
                display_name(&source)
            ));
        }

        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| "attachment store lock is unavailable".to_string())?;
        let temporary = self
            .root
            .join("objects")
            .join(format!("import-{}.tmp", uuid::Uuid::new_v4()));
        let (sha256, bytes) = copy_and_hash(&source, &temporary)?;
        let meta_path = self.root.join("meta").join(format!("{sha256}.json"));
        if meta_path.is_file() {
            fs::remove_file(&temporary).map_err(|error| error.to_string())?;
            return self.get(&sha256);
        }
        let extension = safe_extension(&source);
        let object_name = if extension.is_empty() {
            sha256.clone()
        } else {
            format!("{sha256}.{extension}")
        };
        let object_path = self.root.join("objects").join(object_name);
        if object_path.is_file() {
            fs::remove_file(&temporary).map_err(|error| error.to_string())?;
        } else {
            fs::rename(&temporary, &object_path).map_err(|error| error.to_string())?;
        }

        let (mut kind, mut mime_type) = classify(&source);
        if matches!(kind.as_str(), "image" | "audio")
            && !media_signature_matches(&object_path, &mime_type)
        {
            kind = "binary".into();
            mime_type = "application/octet-stream".into();
        }
        let extraction = extract_text(&object_path, &kind, &extension);
        let (extracted_chars, context_mode, note) = match extraction {
            Ok(Some(text)) => {
                let text = truncate_chars(&text, MAX_EXTRACTED_CHARS);
                let count = text.chars().count();
                atomic_write(
                    &self.root.join("text").join(format!("{sha256}.txt")),
                    text.as_bytes(),
                )?;
                let mode = if kind == "pdf" || kind == "document" {
                    "extracted_text"
                } else {
                    "text"
                };
                (
                    count,
                    mode.to_string(),
                    format!("{count} characters are available as local model context."),
                )
            }
            Ok(None) if matches!(kind.as_str(), "image" | "audio") => (
                0,
                "native_media".into(),
                "Sent through the selected model's native multimodal input when supported.".into(),
            ),
            Ok(None) => (
                0,
                "metadata_only".into(),
                "Stored locally. This format has no safe built-in text extractor.".into(),
            ),
            Err(error) => (
                0,
                "metadata_only".into(),
                format!("Stored locally, but extraction failed: {error}"),
            ),
        };
        let attachment = ContextAttachment {
            id: sha256.clone(),
            name: display_name(&source),
            kind,
            mime_type,
            bytes,
            sha256,
            stored_path: object_path.to_string_lossy().into_owned(),
            extracted_chars,
            context_mode,
            note,
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        atomic_json(&meta_path, &attachment)?;
        Ok(attachment)
    }

    pub fn get(&self, id: &str) -> Result<ContextAttachment, String> {
        validate_id(id)?;
        let mut attachment: ContextAttachment =
            read_json(&self.root.join("meta").join(format!("{id}.json")))?;
        if attachment.id != id || attachment.sha256 != id {
            return Err("attachment metadata does not match its content address".into());
        }
        let stored_name = Path::new(&attachment.stored_path)
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| *value == id || value.starts_with(&format!("{id}.")))
            .ok_or_else(|| "attachment metadata contains an invalid object name".to_string())?;
        let current_path = self.root.join("objects").join(stored_name);
        if !current_path.is_file() {
            return Err("The durable attachment object is missing.".into());
        }
        // Persisted records remain portable if the user later moves Kestrel's workspace.
        attachment.stored_path = current_path.to_string_lossy().into_owned();
        Ok(attachment)
    }

    pub fn resolve(&self, ids: &[String]) -> Result<Vec<ContextAttachment>, String> {
        if ids.len() > 12 {
            return Err("A single message can contain at most 12 attachments.".into());
        }
        let mut total = 0u64;
        let mut values = Vec::with_capacity(ids.len());
        for id in ids {
            if values
                .iter()
                .any(|known: &ContextAttachment| &known.id == id)
            {
                continue;
            }
            let value = self.get(id)?;
            total = total.saturating_add(value.bytes);
            if total > 256 * 1024 * 1024 {
                return Err("Attachments in one message cannot exceed 256 MiB in total.".into());
            }
            values.push(value);
        }
        Ok(values)
    }

    pub fn prepare_message(
        &self,
        text: &str,
        attachments: &[ContextAttachment],
        model: &ModelInfo,
        max_text_chars: usize,
    ) -> Result<PreparedAttachments, String> {
        self.prepare_message_cached(
            text,
            attachments,
            model,
            max_text_chars,
            &mut MediaCache::default(),
        )
    }

    pub(crate) fn prepare_message_cached(
        &self,
        text: &str,
        attachments: &[ContextAttachment],
        model: &ModelInfo,
        max_text_chars: usize,
        media_cache: &mut MediaCache,
    ) -> Result<PreparedAttachments, String> {
        if attachments.is_empty() {
            return Ok(PreparedAttachments {
                content: Value::String(text.to_string()),
                notice: None,
            });
        }
        let mut manifest = String::from("\n\nLocal attachments (durable copies):\n");
        let mut remaining = max_text_chars;
        let mut media = Vec::new();
        let mut native_media_bytes = 0u64;
        let mut degraded = Vec::new();
        for attachment in attachments {
            manifest.push_str(&format!(
                "- {} [{}; {}; {} bytes; sha256 {}]\n",
                attachment.name,
                attachment.kind,
                attachment.mime_type,
                attachment.bytes,
                attachment.sha256
            ));
            if matches!(attachment.kind.as_str(), "image" | "audio") {
                let supported = if attachment.kind == "image" {
                    model.supports_vision
                } else {
                    model.supports_audio
                };
                if !supported {
                    degraded.push(if attachment.kind == "image" {
                        format!(
                            "{} was not sent visually because this model has no vision projector.",
                            attachment.name
                        )
                    } else {
                        format!(
                            "{} was not sent as audio because this model/projector does not advertise audio input.",
                            attachment.name
                        )
                    });
                } else if attachment.bytes > MAX_NATIVE_MEDIA_BYTES {
                    degraded.push(format!(
                        "{} is stored but was not sent as {} because native media is limited to 32 MiB per file.",
                        attachment.name,
                        if attachment.kind == "image" { "an image" } else { "audio" }
                    ));
                } else if native_media_bytes.saturating_add(attachment.bytes)
                    > MAX_NATIVE_MEDIA_TOTAL_BYTES
                {
                    degraded.push(format!(
                        "{} is stored but was not sent because native media is limited to 64 MiB per message.",
                        attachment.name
                    ));
                } else {
                    media.push(self.native_media_part_cached(attachment, media_cache)?);
                    native_media_bytes = native_media_bytes.saturating_add(attachment.bytes);
                }
                continue;
            }
            match attachment.kind.as_str() {
                _ if attachment.extracted_chars > 0 => {
                    let available = remaining.min(attachment.extracted_chars);
                    let mut included = 0;
                    if available > 0 {
                        let extracted = self.read_extracted(&attachment.id, 0, available)?;
                        included = extracted.chars().count();
                        manifest.push_str(&format!(
                            "\n--- BEGIN ATTACHMENT {} ---\n{}\n--- END ATTACHMENT {} ---\n",
                            attachment.name, extracted, attachment.name
                        ));
                        remaining = remaining.saturating_sub(included);
                    }
                    if included < attachment.extracted_chars {
                        degraded.push(format!(
                            "{} was truncated to fit this turn; its full local extraction remains stored.",
                            attachment.name
                        ));
                    }
                }
                _ => degraded.push(format!(
                    "{} is attached as metadata only; its binary contents were not interpreted.",
                    attachment.name
                )),
            }
        }
        if !degraded.is_empty() {
            manifest.push_str("\nContext limitations:\n");
            for item in &degraded {
                manifest.push_str(&format!("- {item}\n"));
            }
        }
        let combined = format!("{text}{manifest}");
        let mut parts = vec![json!({"type":"text","text":combined})];
        parts.extend(media);
        Ok(PreparedAttachments {
            content: Value::Array(parts),
            notice: (!degraded.is_empty()).then(|| degraded.join(" ")),
        })
    }

    pub fn read_extracted(&self, id: &str, offset: usize, limit: usize) -> Result<String, String> {
        validate_id(id)?;
        let limit = limit.clamp(1, 100_000);
        let path = self.root.join("text").join(format!("{id}.txt"));
        let text = fs::read_to_string(path)
            .map_err(|_| "This attachment has no extracted text.".to_string())?;
        Ok(text.chars().skip(offset).take(limit).collect())
    }

    pub fn open(&self, id: &str) -> Result<(), String> {
        let attachment = self.get(id)?;
        let path = PathBuf::from(attachment.stored_path);
        if !path.is_file() {
            return Err("The durable attachment object is missing.".into());
        }
        #[cfg(windows)]
        {
            std::process::Command::new("explorer.exe")
                .arg(&path)
                .spawn()
                .map_err(|error| error.to_string())?;
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            return Err("Opening attachments is currently supported on Windows.".into());
        }
        Ok(())
    }

    fn native_media_part(&self, attachment: &ContextAttachment) -> Result<Value, String> {
        if attachment.bytes > MAX_NATIVE_MEDIA_BYTES {
            return Err(format!(
                "{} is stored, but native model media input is limited to 32 MiB per file.",
                attachment.name
            ));
        }
        // Durable chat/task records may outlive an application-data move. Resolve the object from
        // its content address instead of trusting the absolute path embedded in an old record.
        let stored = self.get(&attachment.id)?;
        let bytes = fs::read(&stored.stored_path).map_err(|error| error.to_string())?;
        let encoded = STANDARD.encode(bytes);
        if attachment.kind == "image" {
            Ok(
                json!({"type":"image_url","image_url":{"url":format!("data:{};base64,{encoded}", stored.mime_type)}}),
            )
        } else {
            let format = audio_format(&stored.name, &stored.mime_type);
            Ok(json!({"type":"input_audio","input_audio":{"data":encoded,"format":format}}))
        }
    }

    fn native_media_part_cached(
        &self,
        attachment: &ContextAttachment,
        cache: &mut MediaCache,
    ) -> Result<Value, String> {
        if let Some(content) = cache.get(&attachment.id) {
            return Ok((*content).clone());
        }
        let content = Arc::new(self.native_media_part(attachment)?);
        let encoded_bytes = (attachment.bytes.saturating_add(2) / 3)
            .saturating_mul(4)
            .saturating_add(1_024);
        cache.insert(attachment.id.clone(), encoded_bytes, Arc::clone(&content));
        Ok((*content).clone())
    }
}

fn copy_and_hash(source: &Path, destination: &Path) -> Result<(String, u64), String> {
    let result = (|| {
        let mut input = File::open(source).map_err(|error| error.to_string())?;
        let mut output = File::create(destination).map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; READ_CHUNK];
        let mut total = 0u64;
        loop {
            let count = input.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            total = total.saturating_add(count as u64);
            if total > MAX_FILE_BYTES {
                return Err(
                    "The selected file grew beyond the 128 MiB safety limit while copying.".into(),
                );
            }
            hasher.update(&buffer[..count]);
            output
                .write_all(&buffer[..count])
                .map_err(|error| error.to_string())?;
        }
        output.sync_all().map_err(|error| error.to_string())?;
        Ok((hex::encode(hasher.finalize()), total))
    })();
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}

fn extract_text(path: &Path, kind: &str, extension: &str) -> Result<Option<String>, String> {
    match kind {
        "text" => read_plain_text(path).map(Some),
        "pdf" => match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pdf_extract::extract_text(path)
        })) {
            Ok(result) => result.map(Some).map_err(|error| error.to_string()),
            Err(payload) => {
                let detail = payload
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                    .unwrap_or("unknown PDF parser panic");
                Err(format!("PDF extraction stopped unexpectedly: {detail}"))
            }
        },
        "document" => extract_open_xml(path, extension).map(Some),
        _ => Ok(None),
    }
}

fn read_plain_text(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(MAX_PLAIN_TEXT_BYTES.min(READ_CHUNK));
    Read::by_ref(&mut file)
        .take((MAX_PLAIN_TEXT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    let truncated = bytes.len() > MAX_PLAIN_TEXT_BYTES;
    bytes.truncate(MAX_PLAIN_TEXT_BYTES);
    if bytes.starts_with(&[0xff, 0xfe]) {
        let units = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16(&units).map_err(|error| error.to_string());
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        let units = bytes[2..]
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        return String::from_utf16(&units).map_err(|error| error.to_string());
    }
    if bytes.iter().take(8_192).any(|byte| *byte == 0) {
        return Err("The file contains binary data and was not treated as text.".into());
    }
    match std::str::from_utf8(&bytes) {
        Ok(value) => Ok(value.to_string()),
        Err(error) if truncated && error.error_len().is_none() => {
            Ok(String::from_utf8_lossy(&bytes[..error.valid_up_to()]).into_owned())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn extract_open_xml(path: &Path, extension: &str) -> Result<String, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    let mut names = (0..archive.len())
        .filter_map(|index| {
            archive
                .by_index(index)
                .ok()
                .map(|entry| entry.name().to_string())
        })
        .filter(|name| open_xml_entry(extension, name))
        .collect::<Vec<_>>();
    names.sort();
    let mut output = String::new();
    for name in names {
        let mut entry = archive.by_name(&name).map_err(|error| error.to_string())?;
        if entry.size() > MAX_XML_ENTRY_BYTES {
            continue;
        }
        let mut xml = String::new();
        entry
            .by_ref()
            .take(MAX_XML_ENTRY_BYTES + 1)
            .read_to_string(&mut xml)
            .map_err(|error| error.to_string())?;
        if xml.len() as u64 > MAX_XML_ENTRY_BYTES {
            continue;
        }
        let text = xml_text(&xml)?;
        if !text.trim().is_empty() {
            output.push_str(&format!("\n[{name}]\n{text}\n"));
        }
        if output.chars().count() >= MAX_EXTRACTED_CHARS {
            break;
        }
    }
    if output.trim().is_empty() {
        Err("No readable document text was found.".into())
    } else {
        Ok(output)
    }
}

fn open_xml_entry(extension: &str, name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    match extension {
        "docx" => {
            lower == "word/document.xml"
                || lower.starts_with("word/header")
                || lower.starts_with("word/footer")
        }
        "pptx" => lower.starts_with("ppt/slides/slide") && lower.ends_with(".xml"),
        "xlsx" => {
            lower == "xl/sharedstrings.xml"
                || (lower.starts_with("xl/worksheets/sheet") && lower.ends_with(".xml"))
        }
        _ => false,
    }
}

fn xml_text(xml: &str) -> Result<String, String> {
    let mut reader = Reader::from_str(xml);
    let mut output = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Text(value)) => {
                let decoded = value.decode().map_err(|error| error.to_string())?;
                output.push_str(&decoded);
            }
            Ok(Event::CData(value)) => {
                output.push_str(&value.decode().map_err(|error| error.to_string())?);
            }
            Ok(Event::GeneralRef(value)) => {
                let reference = value.decode().map_err(|error| error.to_string())?;
                let encoded = format!("&{reference};");
                output.push_str(
                    &quick_xml::escape::unescape(&encoded).map_err(|error| error.to_string())?,
                );
            }
            Ok(Event::End(_))
                if output
                    .chars()
                    .last()
                    .is_some_and(|value| !value.is_whitespace()) =>
            {
                output.push(' ');
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(error.to_string()),
            _ => {}
        }
    }
    Ok(output.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn classify(path: &Path) -> (String, String) {
    let extension = safe_extension(path);
    let (kind, mime) = match extension.as_str() {
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "webp" => ("image", "image/webp"),
        "gif" => ("image", "image/gif"),
        "bmp" => ("image", "image/bmp"),
        "wav" => ("audio", "audio/wav"),
        "mp3" => ("audio", "audio/mpeg"),
        "flac" => ("audio", "audio/flac"),
        "ogg" | "oga" => ("audio", "audio/ogg"),
        "m4a" => ("audio", "audio/mp4"),
        "pdf" => ("pdf", "application/pdf"),
        "docx" => (
            "document",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        "pptx" => (
            "document",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        "xlsx" => (
            "document",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        value if is_text_extension(value) => ("text", "text/plain"),
        _ => ("binary", "application/octet-stream"),
    };
    (kind.into(), mime.into())
}

fn media_signature_matches(path: &Path, mime: &str) -> bool {
    let mut header = [0u8; 16];
    let mut count = 0;
    if let Ok(mut file) = File::open(path) {
        while count < header.len() {
            match file.read(&mut header[count..]) {
                Ok(0) | Err(_) => break,
                Ok(read) => count += read,
            }
        }
    }
    let bytes = &header[..count];
    match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/bmp" => bytes.starts_with(b"BM"),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
        "audio/wav" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE"),
        "audio/mpeg" => {
            bytes.starts_with(b"ID3")
                || bytes
                    .get(..2)
                    .is_some_and(|pair| pair[0] == 0xff && pair[1] & 0xe0 == 0xe0)
        }
        "audio/flac" => bytes.starts_with(b"fLaC"),
        "audio/ogg" => bytes.starts_with(b"OggS"),
        "audio/mp4" => bytes.get(4..8) == Some(b"ftyp"),
        _ => false,
    }
}

fn is_text_extension(extension: &str) -> bool {
    matches!(
        extension,
        "txt"
            | "md"
            | "markdown"
            | "csv"
            | "tsv"
            | "json"
            | "jsonl"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "html"
            | "htm"
            | "css"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "rs"
            | "py"
            | "java"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cs"
            | "go"
            | "rb"
            | "php"
            | "swift"
            | "kt"
            | "kts"
            | "sql"
            | "sh"
            | "ps1"
            | "bat"
            | "cmd"
            | "ini"
            | "cfg"
            | "conf"
            | "log"
            | "svg"
    )
}

fn audio_format(name: &str, mime: &str) -> &'static str {
    let extension = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    match extension.to_ascii_lowercase().as_str() {
        "mp3" => "mp3",
        "flac" => "flac",
        "ogg" | "oga" => "ogg",
        "m4a" => "m4a",
        _ if mime == "audio/mpeg" => "mp3",
        _ => "wav",
    }
}

fn safe_extension(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>()
        .to_ascii_lowercase()
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("attachment")
        .chars()
        .take(240)
        .collect()
}

fn truncate_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.len() == 64 && id.bytes().all(|value| value.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("invalid attachment id".into())
    }
}

fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    atomic_write(path, &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    if path.is_file() {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_deduplicates_and_extracts_text() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("notes.md");
        fs::write(&source, "offline evidence").unwrap();
        let store = AttachmentStore::new(directory.path()).unwrap();
        let first = store.import_path(&source).unwrap();
        let second = store.import_path(&source).unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.context_mode, "text");
        assert_eq!(store.read_extracted(&first.id, 8, 8).unwrap(), "evidence");
    }

    #[test]
    fn durable_objects_rebase_when_the_workspace_moves() {
        let source_directory = tempfile::tempdir().unwrap();
        let source = source_directory.path().join("portable.txt");
        fs::write(&source, "portable context").unwrap();
        let original_parent = tempfile::tempdir().unwrap();
        let original_root = original_parent.path().join("workspace");
        let original = AttachmentStore::new(&original_root).unwrap();
        let attachment = original.import_path(&source).unwrap();

        let moved_parent = tempfile::tempdir().unwrap();
        let moved_root = moved_parent.path().join("workspace");
        fs::rename(&original_root, &moved_root).unwrap();
        let moved = AttachmentStore::new(&moved_root).unwrap();
        let restored = moved.get(&attachment.id).unwrap();

        assert!(Path::new(&restored.stored_path).starts_with(&moved_root));
        assert_eq!(
            moved.read_extracted(&restored.id, 0, 100).unwrap(),
            "portable context"
        );
    }

    #[test]
    fn arbitrary_binary_is_kept_without_false_extraction() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("archive.bin");
        fs::write(&source, [0, 1, 2, 3]).unwrap();
        let store = AttachmentStore::new(directory.path()).unwrap();
        let attachment = store.import_path(&source).unwrap();
        assert_eq!(attachment.context_mode, "metadata_only");
        assert_eq!(attachment.kind, "binary");
    }

    #[test]
    fn rejects_untrusted_ids() {
        let directory = tempfile::tempdir().unwrap();
        let store = AttachmentStore::new(directory.path()).unwrap();
        assert!(store.get("../secret").is_err());
    }

    #[test]
    fn native_media_is_capability_gated_without_losing_the_attachment() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("reference.png");
        fs::write(&source, b"\x89PNG\r\n\x1a\nlocal-image-bytes").unwrap();
        let store = AttachmentStore::new(directory.path()).unwrap();
        let attachment = store.import_path(&source).unwrap();
        let mut model = ModelInfo {
            id: "vision-model".into(),
            name: "Vision model".into(),
            path: "model.gguf".into(),
            source: "test".into(),
            bytes: 1,
            architecture: None,
            context_length: None,
            chat_template: true,
            quantization: None,
            mmproj_path: Some("mmproj.gguf".into()),
            supports_vision: true,
            supports_audio: false,
            recommendation: "test".into(),
        };
        let prepared = store
            .prepare_message(
                "Inspect it.",
                std::slice::from_ref(&attachment),
                &model,
                1_000,
            )
            .unwrap();
        assert_eq!(prepared.content[1]["type"], "image_url");

        model.supports_vision = false;
        let prepared = store
            .prepare_message("Inspect it.", &[attachment], &model, 1_000)
            .unwrap();
        assert_eq!(prepared.content.as_array().unwrap().len(), 1);
        assert!(prepared.notice.unwrap().contains("no vision projector"));
    }

    #[test]
    fn native_media_has_an_aggregate_message_budget() {
        let directory = tempfile::tempdir().unwrap();
        let store = AttachmentStore::new(directory.path()).unwrap();
        let mut attachments = Vec::new();
        for index in 0..3 {
            let source = directory.path().join(format!("reference-{index}.png"));
            fs::write(
                &source,
                [b"\x89PNG\r\n\x1a\n".as_slice(), &[index]].concat(),
            )
            .unwrap();
            let mut attachment = store.import_path(&source).unwrap();
            attachment.bytes = MAX_NATIVE_MEDIA_BYTES;
            attachments.push(attachment);
        }
        let model = ModelInfo {
            id: "vision-model".into(),
            name: "Vision model".into(),
            path: "model.gguf".into(),
            source: "test".into(),
            bytes: 1,
            architecture: None,
            context_length: None,
            chat_template: true,
            quantization: None,
            mmproj_path: Some("mmproj.gguf".into()),
            supports_vision: true,
            supports_audio: false,
            recommendation: "test".into(),
        };

        let prepared = store
            .prepare_message("Inspect them.", &attachments, &model, 1_000)
            .unwrap();
        assert_eq!(prepared.content.as_array().unwrap().len(), 3);
        assert!(prepared.notice.unwrap().contains("64 MiB per message"));
    }

    #[test]
    fn media_cache_evicts_old_encoded_values_at_its_byte_cap() {
        let mut cache = MediaCache::default();
        cache.insert(
            "first".into(),
            40 * 1024 * 1024,
            Arc::new(json!({"data":"first"})),
        );
        cache.insert(
            "second".into(),
            40 * 1024 * 1024,
            Arc::new(json!({"data":"second"})),
        );

        assert!(cache.get("first").is_none());
        assert!(cache.get("second").is_some());
        assert!(cache.encoded_bytes <= MAX_MEDIA_CACHE_BYTES);
    }

    #[test]
    fn extraction_notice_uses_the_characters_actually_included() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("long.txt");
        fs::write(&source, "x".repeat(150_000)).unwrap();
        let store = AttachmentStore::new(directory.path()).unwrap();
        let attachment = store.import_path(&source).unwrap();
        let model = ModelInfo {
            id: "text-model".into(),
            name: "Text model".into(),
            path: "model.gguf".into(),
            source: "test".into(),
            bytes: 1,
            architecture: None,
            context_length: None,
            chat_template: true,
            quantization: None,
            mmproj_path: None,
            supports_vision: false,
            supports_audio: false,
            recommendation: "test".into(),
        };

        let prepared = store
            .prepare_message("Read it.", &[attachment], &model, 200_000)
            .unwrap();
        assert!(prepared.notice.unwrap().contains("truncated"));
    }

    #[test]
    fn pdf_text_is_extracted_into_durable_context() {
        use lopdf::{dictionary, Document, Object, Stream};

        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("evidence.pdf");
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content_id = document.add_object(Stream::new(
            dictionary! {},
            b"BT /F1 18 Tf 72 720 Td (Offline PDF evidence) Tj ET".to_vec(),
        ));
        let page_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        });
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        document.compress();
        document.save(&source).unwrap();

        let store = AttachmentStore::new(directory.path()).unwrap();
        let attachment = store.import_path(&source).unwrap();
        assert_eq!(attachment.kind, "pdf");
        assert_eq!(attachment.context_mode, "extracted_text");
        assert!(store
            .read_extracted(&attachment.id, 0, 1_000)
            .unwrap()
            .contains("Offline PDF evidence"));
    }

    #[test]
    fn docx_xml_entities_are_preserved_as_unescaped_text() {
        use zip::{write::SimpleFileOptions, ZipWriter};

        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("evidence.docx");
        let mut archive = ZipWriter::new(File::create(&source).unwrap());
        archive
            .start_file("word/document.xml", SimpleFileOptions::default())
            .unwrap();
        archive
            .write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
                    <w:document xmlns:w="urn:test"><w:body><w:p><w:r><w:t>Research &amp; evidence &#x2014; offline</w:t></w:r></w:p></w:body></w:document>"#,
            )
            .unwrap();
        archive.finish().unwrap();

        let store = AttachmentStore::new(directory.path()).unwrap();
        let attachment = store.import_path(&source).unwrap();
        assert_eq!(attachment.context_mode, "extracted_text");
        let extracted = store.read_extracted(&attachment.id, 0, 1_000).unwrap();
        assert!(
            extracted.contains("Research & evidence — offline"),
            "{extracted:?}"
        );
    }
}
