//! Bounded local-model assistance for the visual lyric editor.
//!
//! These helpers may suggest text, but they never mutate a music project. The Tauri command owns
//! the workspace gate and `RuntimeManager` owns the inference lease; `music.rs` remains the only
//! owner of durable lyric revisions.

use super::music::{
    clean_lyric_line, parse_lyrical_translations, DraftLyricsFromAudioRangeRequest,
    DraftLyricsFromAudioRangeResult, TranslateMusicLyricsRequest, TranslateMusicLyricsResult,
};
use crate::{
    model::ModelInfo,
    runtime::{authorized, ModelConnection},
};
use base64::Engine as _;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::Duration;

const MAX_AUDIO_SLICE_BYTES: usize = 16 * 1024 * 1024;
const MAX_COMPLETION_BYTES: usize = 256 * 1024;
const MAX_DRAFT_BYTES: usize = 4 * 1024;
const MAX_TRANSLATION_LINES: usize = 64;
const MAX_TRANSLATION_LINE_BYTES: usize = 4 * 1024;
const MAX_TRANSLATION_TEXT_BYTES: usize = 64 * 1024;

pub(crate) async fn draft_from_audio(
    request: &DraftLyricsFromAudioRangeRequest,
    model: &ModelInfo,
    connection: &ModelConnection,
    wav_bytes: &[u8],
) -> Result<DraftLyricsFromAudioRangeResult, String> {
    if !model.supports_audio {
        return Err(format!(
            "{} cannot listen to audio. Choose a catalog model with native audio support.",
            model.name
        ));
    }
    if wav_bytes.is_empty() || wav_bytes.len() > MAX_AUDIO_SLICE_BYTES {
        return Err("The prepared audio excerpt is empty or exceeds the 16 MiB local-model limit. Choose a range of 30 seconds or less.".into());
    }
    let audio = base64::engine::general_purpose::STANDARD.encode(wav_bytes);
    let messages = vec![json!({
        "role": "user",
        "content": [
            {
                "type": "text",
                "text": format!(
                    "Listen to this {:.2}s–{:.2}s excerpt from a song. Transcribe only the exact sung words. Return plain lyric text with no heading, commentary, timestamp, or quotation marks.",
                    request.start_seconds, request.end_seconds
                )
            },
            {
                "type": "input_audio",
                "input_audio": { "data": audio, "format": "wav" }
            }
        ]
    })];
    let raw = completion_text(
        connection,
        json!({
            "model": connection.model_id,
            "messages": messages,
            "max_tokens": 256,
            "temperature": 0.1
        }),
        "Audio lyric suggestion",
    )
    .await?;
    let transcription = parse_audio_draft(&raw);
    if transcription.is_empty() {
        return Err(format!(
            "{} finished without returning any sung words. The existing prompt is unchanged.",
            model.name
        ));
    }
    Ok(DraftLyricsFromAudioRangeResult {
        transcription,
        model_id: model.id.clone(),
        model_name: model.name.clone(),
    })
}

pub(crate) async fn translate(
    request: &TranslateMusicLyricsRequest,
    model: &ModelInfo,
    connection: &ModelConnection,
) -> Result<TranslateMusicLyricsResult, String> {
    validate_translation_request(request)?;
    if request.lines.is_empty() {
        return Ok(TranslateMusicLyricsResult {
            translations: Vec::new(),
            model_id: model.id.clone(),
            model_name: model.name.clone(),
        });
    }
    let numbered = request
        .lines
        .iter()
        .enumerate()
        .map(|(index, line)| format!("{}. {}", index + 1, line.trim()))
        .collect::<Vec<_>>()
        .join("\n");
    let messages = vec![
        json!({
            "role": "system",
            "content": "Translate song lyrics naturally while preserving line-by-line meaning and alignment. Return only numbered translated lines. Keep every input number exactly once. Do not add headings, notes, explanations, or metadata."
        }),
        json!({
            "role": "user",
            "content": format!(
                "Target language: {}\n\n{}\n\nReturn exactly {} numbered translated lines.",
                request.target_language.trim(),
                numbered,
                request.lines.len()
            )
        }),
    ];
    let raw = completion_text(
        connection,
        json!({
            "model": connection.model_id,
            "messages": messages,
            "max_tokens": (request.lines.len() * 64).clamp(256, 4096),
            "temperature": 0.2
        }),
        "Lyric translation",
    )
    .await?;
    let translations = parse_lyrical_translations(&raw, request.lines.len());
    let translated = translations
        .iter()
        .filter(|line| !line.trim().is_empty())
        .count();
    let translated_bytes = translations
        .iter()
        .map(|line| line.len())
        .fold(0_usize, usize::saturating_add);
    if translated != request.lines.len()
        || translations
            .iter()
            .any(|line| line.len() > MAX_TRANSLATION_LINE_BYTES)
        || translated_bytes > MAX_TRANSLATION_TEXT_BYTES
    {
        return Err(format!(
            "{} did not return a complete bounded set of {} aligned lyric lines (received {translated}). No partial translation was applied; retry or translate one cue at a time.",
            model.name,
            request.lines.len()
        ));
    }
    Ok(TranslateMusicLyricsResult {
        translations,
        model_id: model.id.clone(),
        model_name: model.name.clone(),
    })
}

pub(crate) fn validate_translation_request(
    request: &TranslateMusicLyricsRequest,
) -> Result<(), String> {
    let target = request.target_language.trim();
    if target.is_empty()
        || target.len() > 64
        || !target
            .chars()
            .all(|character| character.is_alphabetic() || matches!(character, ' ' | '-'))
    {
        return Err(
            "Choose a target language using letters, spaces, or hyphens (64 bytes maximum).".into(),
        );
    }
    if request.lines.len() > MAX_TRANSLATION_LINES {
        return Err(format!(
            "Translate at most {MAX_TRANSLATION_LINES} lyric cues in one local-model pass."
        ));
    }
    let mut total = 0_usize;
    for line in &request.lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.len() > MAX_TRANSLATION_LINE_BYTES {
            return Err("Every translated cue must contain 1–4096 UTF-8 bytes.".into());
        }
        total = total.saturating_add(trimmed.len());
    }
    if total > MAX_TRANSLATION_TEXT_BYTES {
        return Err("The selected lyric cues exceed the 64 KiB translation boundary.".into());
    }
    Ok(())
}

async fn completion_text(
    connection: &ModelConnection,
    body: Value,
    operation: &str,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|error| format!("Could not prepare the local {operation} request: {error}"))?;
    let response = authorized(
        client.post(format!("{}/chat/completions", connection.endpoint)),
        connection,
    )
    .json(&body)
    .send()
    .await
    .map_err(|error| format!("{operation} request failed: {error}"))?;
    let status = response.status();
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("{operation} response failed: {error}"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_COMPLETION_BYTES {
            return Err(format!(
                "{operation} returned more than the 256 KiB local safety limit."
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        return Err(format!(
            "{operation} returned HTTP {status}: {}",
            utf8_prefix(&String::from_utf8_lossy(&bytes), 600)
        ));
    }
    let completion: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{operation} returned invalid JSON: {error}"))?;
    let content = completion
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let reasoning = completion
        .pointer("/choices/0/message/reasoning_content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let text = if content.is_empty() {
        reasoning
    } else {
        content
    };
    if text.is_empty() {
        return Err(format!("{operation} returned no text."));
    }
    Ok(text.to_string())
}

fn parse_audio_draft(raw: &str) -> String {
    let mut output = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("```") {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let without_label = ["transcribed lyrics:", "transcription:", "lyrics:"]
            .iter()
            .find_map(|label| lower.starts_with(label).then(|| &line[label.len()..]))
            .unwrap_or(line);
        let cleaned = clean_lyric_line(without_label);
        if !cleaned.is_empty() {
            output.push(cleaned);
        }
    }
    utf8_prefix(&output.join(" "), MAX_DRAFT_BYTES)
        .trim()
        .to_string()
}

fn utf8_prefix(value: &str, maximum: usize) -> &str {
    if value.len() <= maximum {
        return value;
    }
    let mut boundary = maximum;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_draft_removes_model_wrappers_and_stays_bounded() {
        assert_eq!(
            parse_audio_draft("```text\nTranscription: \"Stay here\"\nthrough the rain\n```"),
            "Stay here through the rain"
        );
        let long = "歌".repeat(2_000);
        let parsed = parse_audio_draft(&long);
        assert!(parsed.len() <= MAX_DRAFT_BYTES);
        assert!(parsed.is_char_boundary(parsed.len()));
    }

    #[test]
    fn translation_request_is_explicitly_bounded() {
        let request = TranslateMusicLyricsRequest {
            project_id: "project".into(),
            take_id: "take".into(),
            model_id: "model".into(),
            target_language: "Brazilian Portuguese".into(),
            lines: vec!["Stay here".into()],
        };
        assert!(validate_translation_request(&request).is_ok());
        let mut too_many = request;
        too_many.lines = vec!["line".into(); MAX_TRANSLATION_LINES + 1];
        assert!(validate_translation_request(&too_many).is_err());
    }
}
