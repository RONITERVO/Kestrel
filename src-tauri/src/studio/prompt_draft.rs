use crate::{
    model::ModelInfo,
    models::{ControlSettings, ThinkingLevel},
    prompt_catalog::{self, PromptId},
    runtime::{authorized, RuntimeManager},
};
use chrono::Utc;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

use super::{
    model_stream::{reasoning_delta, OpenAiSseDecoder, OpenAiStreamEvent},
    MAX_MOVIE_PROMPT_BYTES,
};

// Prompt drafting is an explicit producer action and may need a long private reasoning pass
// before any visible prose arrives. Use its tested ceiling when the configured runtime allows it,
// while respecting the runtime's explicit output limit.
const PROMPT_COLLABORATOR_MAX_TOKENS: u32 = 32_768;
const PROMPT_COLLABORATOR_THINKING_BUDGET: u32 = 24_576;
const PROMPT_COLLABORATOR_VISIBLE_OUTPUT_TOKENS: u32 = 8_192;
const MAX_REFERENCE_DESCRIPTION_BYTES: usize = 4_000;

#[derive(Debug, PartialEq, Eq)]
struct PromptInferenceAllowance {
    thinking_budget_tokens: u32,
    visible_output_tokens: u32,
}

impl PromptInferenceAllowance {
    fn generation_limit(&self) -> u32 {
        self.thinking_budget_tokens
            .saturating_add(self.visible_output_tokens)
    }
}

fn prompt_inference_allowance(settings: &ControlSettings) -> PromptInferenceAllowance {
    let configured_limit = PROMPT_COLLABORATOR_MAX_TOKENS.min(settings.max_output_tokens);
    if settings.thinking_level.is_off() {
        return PromptInferenceAllowance {
            thinking_budget_tokens: 0,
            visible_output_tokens: PROMPT_COLLABORATOR_VISIBLE_OUTPUT_TOKENS.min(configured_limit),
        };
    }
    let thinking_budget =
        PROMPT_COLLABORATOR_THINKING_BUDGET.min(configured_limit.saturating_mul(3) / 4);
    let visible_output_allowance = PROMPT_COLLABORATOR_VISIBLE_OUTPUT_TOKENS
        .min(configured_limit.saturating_sub(thinking_budget));
    PromptInferenceAllowance {
        thinking_budget_tokens: thinking_budget,
        visible_output_tokens: visible_output_allowance,
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PromptDraftTarget {
    Story,
    ImageAsset,
    ImageComposition,
    ReferenceDescription,
    MusicCaption,
    MusicLyrics,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PromptDraftMode {
    Develop,
    Continue,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptDraftRequest {
    pub request_id: String,
    pub model_id: String,
    pub target: PromptDraftTarget,
    pub mode: PromptDraftMode,
    #[serde(default)]
    pub story_text: String,
    #[serde(default)]
    pub existing_text: String,
    #[serde(default)]
    pub asset_name: String,
    #[serde(default)]
    pub asset_kind: String,
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptDraftReceipt {
    pub target: PromptDraftTarget,
    pub mode: PromptDraftMode,
    pub model_id: String,
    pub messages: Vec<Value>,
    pub temperature: f64,
    pub top_p: f64,
    pub top_k: u32,
    pub max_tokens: u32,
    pub exact_request: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptDraftEvent {
    pub request_id: String,
    pub kind: String,
    pub content: Option<String>,
    pub model_name: Option<String>,
    pub thinking_level: Option<ThinkingLevel>,
    pub receipt: Option<PromptDraftReceipt>,
    pub at: String,
}

pub struct PromptDraftJob {
    pub app: AppHandle,
    pub runtime: Arc<RuntimeManager>,
    pub models: Vec<ModelInfo>,
    pub settings: ControlSettings,
    pub request: PromptDraftRequest,
    pub cancel: CancellationToken,
}

impl PromptDraftJob {
    pub async fn run(self) -> Result<(), String> {
        let Self {
            app,
            runtime,
            models,
            settings,
            request,
            cancel,
        } = self;
        validate_request(&request, &models)?;
        let mut settings = settings.for_model(&request.model_id);
        let maximum_context = if settings.advanced_mode {
            1_048_576
        } else {
            98_304
        };
        let maximum_output = if settings.advanced_mode {
            262_144
        } else {
            32_768
        };
        if let Some(value) = request.context_window {
            if !(4_096..=maximum_context).contains(&value) {
                return Err(format!(
                    "Collaborator context must be between 4,096 and {maximum_context} tokens."
                ));
            }
            settings.context_window = value;
        }
        if let Some(value) = request.max_output_tokens {
            if !(1_024..=maximum_output).contains(&value) {
                return Err(format!(
                    "Collaborator output must be between 1,024 and {maximum_output} tokens."
                ));
            }
            settings.max_output_tokens = value;
        }
        settings.model_overrides.clear();
        if let Some(level) = request.thinking_level {
            settings.thinking_level = level;
        }
        let effective_thinking_level = settings.thinking_level;
        let model = models
            .iter()
            .find(|model| model.id == request.model_id)
            .expect("validated prompt collaborator model");
        emit(
            &app,
            &request.request_id,
            "queued",
            None,
            Some(&model.name),
            Some(effective_thinking_level),
            None,
        );
        let lease = tokio::select! {
            result = runtime.lease_model(&request.model_id, &models, &settings, Some(&app)) => {
                result.map_err(|error| error.to_string())?
            }
            _ = cancel.cancelled() => {
                emit(&app, &request.request_id, "cancelled", None, Some(&model.name), Some(effective_thinking_level), None);
                return Ok(());
            }
        };
        let existing = request.existing_text.trim_end();
        let messages = build_messages(&request);
        let remaining_bytes = output_byte_limit(&request, existing);
        let allowance = prompt_inference_allowance(&settings);
        let max_tokens = allowance.generation_limit();
        let thinking_budget_tokens = allowance.thinking_budget_tokens;
        let (temperature, top_p, top_k) = sampling(request.target);
        let mut body = json!({
            "model": lease.connection.model_id,
            "messages": messages,
            "temperature": temperature,
            "top_p": top_p,
            "top_k": top_k,
            "max_tokens": max_tokens,
            "thinking_budget_tokens": thinking_budget_tokens,
            "stream": true,
            "stream_options": {"include_usage": true}
        });
        if settings.thinking_level.is_off() {
            body["chat_template_kwargs"] = json!({"enable_thinking": false, "reasoning": false});
            body["reasoning_effort"] = json!("off");
        } else {
            body["reasoning_effort"] = json!(settings.thinking_level.as_str());
            body["chat_template_kwargs"] = json!({
                "reasoning_effort": settings.thinking_level.as_template_effort(),
                "enable_thinking": true
            });
        }
        let receipt = PromptDraftReceipt {
            target: request.target,
            mode: request.mode,
            model_id: lease.connection.model_id.clone(),
            messages: messages.clone(),
            temperature,
            top_p,
            top_k,
            max_tokens,
            exact_request: body.clone(),
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(3_600))
            .build()
            .map_err(|error| error.to_string())?;
        let response = authorized(
            client.post(format!("{}/chat/completions", lease.connection.endpoint)),
            &lease.connection,
        )
        .json(&body)
        .send()
        .await
        .map_err(|error| error.to_string())?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(format!(
                "prompt collaborator returned {status}: {}",
                truncate(&detail, 600)
            ));
        }
        emit(
            &app,
            &request.request_id,
            "started",
            None,
            Some(&model.name),
            Some(effective_thinking_level),
            Some(receipt),
        );

        let mut bytes = response.bytes_stream();
        let mut decoder = OpenAiSseDecoder::default();
        let mut emitted_bytes = 0usize;
        let mut output_limited = false;
        let mut accept_events = |events: Vec<OpenAiStreamEvent>| -> bool {
            for event in events {
                let OpenAiStreamEvent::Message(value) = event else {
                    continue;
                };
                if let Some(token) = value
                    .pointer("/choices/0/delta/content")
                    .and_then(Value::as_str)
                {
                    let available = remaining_bytes.saturating_sub(emitted_bytes);
                    let accepted = utf8_prefix(token, available);
                    if !accepted.is_empty() {
                        emitted_bytes = emitted_bytes.saturating_add(accepted.len());
                        emit(
                            &app,
                            &request.request_id,
                            "token",
                            Some(accepted),
                            Some(&model.name),
                            Some(effective_thinking_level),
                            None,
                        );
                    }
                    if accepted.len() < token.len() || emitted_bytes >= remaining_bytes {
                        emit(
                            &app,
                            &request.request_id,
                            "limited",
                            None,
                            Some(&model.name),
                            Some(effective_thinking_level),
                            None,
                        );
                        return true;
                    }
                }
                if value
                    .pointer("/choices/0/finish_reason")
                    .and_then(Value::as_str)
                    == Some("length")
                {
                    output_limited = true;
                }
                if let Some(token) = reasoning_delta(&value) {
                    emit(
                        &app,
                        &request.request_id,
                        "reasoning",
                        Some(token),
                        Some(&model.name),
                        Some(effective_thinking_level),
                        None,
                    );
                }
            }
            false
        };
        loop {
            let next = tokio::select! {
                value = bytes.next() => value,
                _ = cancel.cancelled() => {
                    emit(&app, &request.request_id, "cancelled", None, Some(&model.name), Some(effective_thinking_level), None);
                    return Ok(());
                }
            };
            let Some(chunk) = next else { break };
            let events = decoder.push(&chunk.map_err(|error| error.to_string())?)?;
            if accept_events(events) {
                return Ok(());
            }
        }
        let final_events = decoder.finish()?;
        if accept_events(final_events) {
            return Ok(());
        }
        emit(
            &app,
            &request.request_id,
            if output_limited {
                "limited"
            } else {
                "complete"
            },
            None,
            Some(&model.name),
            Some(effective_thinking_level),
            None,
        );
        Ok(())
    }
}

fn build_messages(request: &PromptDraftRequest) -> Vec<Value> {
    let existing = request.existing_text.trim_end();
    let mut messages = vec![json!({"role":"system","content":system_prompt(request.target)})];
    match request.target {
        PromptDraftTarget::Story => {
            if existing.is_empty() {
                messages.push(json!({"role":"user","content":prompt_catalog::text(PromptId::PromptInventStory)}));
            } else {
                messages.push(json!({"role":"user","content":source_instruction(request.mode, "movie story text")}));
                messages.push(json!({"role":"assistant","content":existing}));
                messages.push(json!({"role":"user","content":final_instruction(request.mode, request.target)}));
            }
        }
        PromptDraftTarget::ImageAsset => {
            add_story_context(&mut messages, &request.story_text);
            if !existing.is_empty() {
                messages.push(json!({"role":"user","content":source_instruction(request.mode, "image-description text")}));
                messages.push(json!({"role":"assistant","content":existing}));
            }
            messages.push(
                json!({"role":"user","content":final_instruction(request.mode, request.target)}),
            );
        }
        PromptDraftTarget::ImageComposition => {
            add_image_context(&mut messages, &request.story_text);
            if !existing.is_empty() {
                messages.push(json!({"role":"user","content":source_instruction(request.mode, "current structured image design JSON")}));
                messages.push(json!({"role":"assistant","content":existing}));
            }
            messages.push(
                json!({"role":"user","content":final_instruction(request.mode, request.target)}),
            );
        }
        PromptDraftTarget::ReferenceDescription => {
            add_story_context(&mut messages, &request.story_text);
            messages.push(json!({"role":"user","content":prompt_catalog::render(PromptId::PromptAssetMetadata, &[("asset_name", &format!("{:?}", request.asset_name)), ("asset_kind", &format!("{:?}", request.asset_kind))])}));
            if !existing.is_empty() {
                messages.push(json!({"role":"user","content":source_instruction(request.mode, "reference-placement text")}));
                messages.push(json!({"role":"assistant","content":existing}));
            }
            messages.push(
                json!({"role":"user","content":final_instruction(request.mode, request.target)}),
            );
        }
        PromptDraftTarget::MusicCaption | PromptDraftTarget::MusicLyrics => {
            add_music_context(&mut messages, &request.story_text);
            if !existing.is_empty() {
                let label = if request.target == PromptDraftTarget::MusicCaption {
                    "music-description text"
                } else {
                    "lyrics text"
                };
                messages
                    .push(json!({"role":"user","content":source_instruction(request.mode, label)}));
                messages.push(json!({"role":"assistant","content":existing}));
            }
            messages.push(
                json!({"role":"user","content":final_instruction(request.mode, request.target)}),
            );
        }
    }
    messages
}

fn add_music_context(messages: &mut Vec<Value>, context: &str) {
    if context.trim().is_empty() {
        messages.push(
            json!({"role":"user","content":prompt_catalog::text(PromptId::PromptMusicMissing)}),
        );
    } else {
        messages.push(
            json!({"role":"user","content":prompt_catalog::text(PromptId::PromptMusicContext)}),
        );
        messages.push(json!({"role":"assistant","content":context.trim()}));
    }
}

fn add_image_context(messages: &mut Vec<Value>, context: &str) {
    if context.trim().is_empty() {
        messages.push(
            json!({"role":"user","content":prompt_catalog::text(PromptId::PromptImageMissing)}),
        );
    } else {
        messages.push(
            json!({"role":"user","content":prompt_catalog::text(PromptId::PromptImageContext)}),
        );
        messages.push(json!({"role":"assistant","content":context.trim()}));
    }
}

fn add_story_context(messages: &mut Vec<Value>, story_text: &str) {
    if story_text.trim().is_empty() {
        messages.push(
            json!({"role":"user","content":prompt_catalog::text(PromptId::PromptStoryMissing)}),
        );
    } else {
        messages.push(
            json!({"role":"user","content":prompt_catalog::text(PromptId::PromptStoryContext)}),
        );
        messages.push(json!({"role":"assistant","content":story_text.trim()}));
    }
}

fn source_instruction(mode: PromptDraftMode, label: &str) -> String {
    match mode {
        PromptDraftMode::Develop => {
            prompt_catalog::render(PromptId::PromptSourceDevelop, &[("field", label)])
        }
        PromptDraftMode::Continue => {
            prompt_catalog::render(PromptId::PromptSourceContinue, &[("field", label)])
        }
    }
}

fn final_instruction(mode: PromptDraftMode, target: PromptDraftTarget) -> String {
    prompt_catalog::text(match (mode, target) {
        (PromptDraftMode::Develop, PromptDraftTarget::Story) => PromptId::FinalStoryDevelop,
        (PromptDraftMode::Continue, PromptDraftTarget::Story) => PromptId::FinalStoryContinue,
        (PromptDraftMode::Develop, PromptDraftTarget::ImageAsset) => {
            PromptId::FinalImageAssetDevelop
        }
        (PromptDraftMode::Continue, PromptDraftTarget::ImageAsset) => {
            PromptId::FinalImageAssetContinue
        }
        (PromptDraftMode::Develop, PromptDraftTarget::ImageComposition) => {
            PromptId::FinalImageCompositionDevelop
        }
        (PromptDraftMode::Continue, PromptDraftTarget::ImageComposition) => {
            PromptId::FinalImageCompositionContinue
        }
        (PromptDraftMode::Develop, PromptDraftTarget::ReferenceDescription) => {
            PromptId::FinalReferenceDevelop
        }
        (PromptDraftMode::Continue, PromptDraftTarget::ReferenceDescription) => {
            PromptId::FinalReferenceContinue
        }
        (PromptDraftMode::Develop, PromptDraftTarget::MusicCaption) => {
            PromptId::FinalMusicCaptionDevelop
        }
        (PromptDraftMode::Continue, PromptDraftTarget::MusicCaption) => {
            PromptId::FinalMusicCaptionContinue
        }
        (PromptDraftMode::Develop, PromptDraftTarget::MusicLyrics) => {
            PromptId::FinalMusicLyricsDevelop
        }
        (PromptDraftMode::Continue, PromptDraftTarget::MusicLyrics) => {
            PromptId::FinalMusicLyricsContinue
        }
    })
}

fn system_prompt(target: PromptDraftTarget) -> String {
    match target {
        PromptDraftTarget::Story => prompt_catalog::text(PromptId::StorySystem),
        PromptDraftTarget::ImageAsset => prompt_catalog::text(PromptId::ImageAssetSystem),
        PromptDraftTarget::ImageComposition => {
            prompt_catalog::text(PromptId::ImageCompositionSystem)
        }
        PromptDraftTarget::ReferenceDescription => prompt_catalog::text(PromptId::ReferenceSystem),
        PromptDraftTarget::MusicCaption => prompt_catalog::text(PromptId::MusicCaptionSystem),
        PromptDraftTarget::MusicLyrics => prompt_catalog::text(PromptId::MusicLyricsSystem),
    }
}

fn sampling(target: PromptDraftTarget) -> (f64, f64, u32) {
    match target {
        PromptDraftTarget::Story => (0.85, 0.95, 40),
        PromptDraftTarget::ImageAsset => (0.65, 0.9, 30),
        PromptDraftTarget::ImageComposition => (0.55, 0.9, 30),
        PromptDraftTarget::ReferenceDescription => (0.45, 0.9, 20),
        PromptDraftTarget::MusicCaption => (0.65, 0.92, 30),
        PromptDraftTarget::MusicLyrics => (0.8, 0.95, 40),
    }
}

fn target_limit(target: PromptDraftTarget) -> usize {
    match target {
        PromptDraftTarget::ReferenceDescription => MAX_REFERENCE_DESCRIPTION_BYTES,
        PromptDraftTarget::Story
        | PromptDraftTarget::ImageAsset
        | PromptDraftTarget::ImageComposition
        | PromptDraftTarget::MusicCaption
        | PromptDraftTarget::MusicLyrics => MAX_MOVIE_PROMPT_BYTES,
    }
}

fn output_byte_limit(request: &PromptDraftRequest, existing: &str) -> usize {
    let limit = target_limit(request.target);
    if request.mode == PromptDraftMode::Continue
        && request.target != PromptDraftTarget::ImageComposition
    {
        limit
            .saturating_sub(request.existing_text.len())
            .saturating_sub(usize::from(!existing.is_empty()) * 2)
    } else {
        limit
    }
}

pub fn validate_request(request: &PromptDraftRequest, models: &[ModelInfo]) -> Result<(), String> {
    uuid::Uuid::parse_str(&request.request_id)
        .map_err(|_| "Prompt generation request ID is invalid.".to_string())?;
    if !models.iter().any(|model| model.id == request.model_id) {
        return Err("The selected prompt model is no longer in the local catalog.".into());
    }
    if request.story_text.len() > MAX_MOVIE_PROMPT_BYTES {
        return Err(
            "The creative brief exceeds Studio's 64 KiB collaborator context limit.".into(),
        );
    }
    let limit = target_limit(request.target);
    if request.existing_text.len() > limit {
        return Err(format!(
            "The existing {} exceeds its {} byte limit.",
            target_name(request.target),
            limit
        ));
    }
    if request.target == PromptDraftTarget::ReferenceDescription {
        if request.asset_name.trim().is_empty() || request.asset_name.len() > 1_000 {
            return Err("Reference collaboration requires a bounded asset name.".into());
        }
        if !matches!(request.asset_kind.as_str(), "image" | "video" | "audio") {
            return Err(
                "Reference collaboration requires an image, video, or audio asset type.".into(),
            );
        }
    }
    if request.mode == PromptDraftMode::Continue && request.existing_text.trim().is_empty() {
        return Err("Continue exact draft requires existing text. Choose Develop idea / notes for an empty field.".into());
    }
    if request.mode == PromptDraftMode::Continue
        && request.target != PromptDraftTarget::ImageComposition
    {
        let separator_bytes = usize::from(!request.existing_text.trim_end().is_empty()) * 2;
        if limit
            .saturating_sub(request.existing_text.len())
            .saturating_sub(separator_bytes)
            < 256
        {
            return Err(format!("The {} is too close to its size limit for a useful continuation. Shorten it or choose Develop idea / notes.", target_name(request.target)));
        }
    }
    Ok(())
}

fn target_name(target: PromptDraftTarget) -> &'static str {
    match target {
        PromptDraftTarget::Story => "movie brief",
        PromptDraftTarget::ImageAsset => "image prompt",
        PromptDraftTarget::ImageComposition => "structured image design",
        PromptDraftTarget::ReferenceDescription => "reference description",
        PromptDraftTarget::MusicCaption => "music description",
        PromptDraftTarget::MusicLyrics => "lyrics",
    }
}

pub fn emit_error(app: &AppHandle, request_id: &str, error: String) {
    emit(app, request_id, "error", Some(&error), None, None, None);
}

pub fn emit_settled(app: &AppHandle, request_id: &str) {
    emit(app, request_id, "settled", None, None, None, None);
}

fn emit(
    app: &AppHandle,
    request_id: &str,
    kind: &str,
    content: Option<&str>,
    model_name: Option<&str>,
    thinking_level: Option<ThinkingLevel>,
    receipt: Option<PromptDraftReceipt>,
) {
    let _ = app.emit(
        "studio-prompt-draft",
        PromptDraftEvent {
            request_id: request_id.into(),
            kind: kind.into(),
            content: content.map(str::to_owned),
            model_name: model_name.map(str::to_owned),
            thinking_level,
            receipt,
            at: Utc::now().to_rfc3339(),
        },
    );
}

fn utf8_prefix(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> ModelInfo {
        ModelInfo {
            id: "model-1".into(),
            name: "Local Prompt Model".into(),
            path: "model.gguf".into(),
            source: "test".into(),
            bytes: 1,
            architecture: None,
            context_length: None,
            chat_template: false,
            quantization: None,
            mmproj_path: None,
            supports_vision: false,
            supports_audio: false,
            recommendation: String::new(),
        }
    }

    fn request(target: PromptDraftTarget, mode: PromptDraftMode) -> PromptDraftRequest {
        PromptDraftRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            model_id: "model-1".into(),
            target,
            mode,
            story_text: "A musician searches a flooded city.".into(),
            existing_text: "blue light, brass compass".into(),
            asset_name: "compass.png".into(),
            asset_kind: "image".into(),
            thinking_level: None,
            context_window: None,
            max_output_tokens: None,
        }
    }

    #[test]
    fn accepts_each_target_and_explicit_mode_for_any_catalog_model() {
        let models = vec![model()];
        for target in [
            PromptDraftTarget::Story,
            PromptDraftTarget::ImageAsset,
            PromptDraftTarget::ImageComposition,
            PromptDraftTarget::ReferenceDescription,
            PromptDraftTarget::MusicCaption,
            PromptDraftTarget::MusicLyrics,
        ] {
            for mode in [PromptDraftMode::Develop, PromptDraftMode::Continue] {
                assert!(validate_request(&request(target, mode), &models).is_ok());
            }
        }
    }

    #[test]
    fn prompt_collaboration_respects_the_configured_runtime_limit() {
        let limited = ControlSettings {
            max_output_tokens: 4_096,
            ..ControlSettings::default()
        };
        let allowance = prompt_inference_allowance(&limited);
        assert_eq!(allowance.generation_limit(), 4_096);
        assert_eq!(allowance.thinking_budget_tokens, 3_072);
        assert_eq!(allowance.visible_output_tokens, 1_024);

        let uncapped = ControlSettings {
            max_output_tokens: 65_536,
            ..ControlSettings::default()
        };
        let allowance = prompt_inference_allowance(&uncapped);
        assert_eq!(allowance.generation_limit(), 32_768);
        assert_eq!(allowance.thinking_budget_tokens, 24_576);
        assert_eq!(allowance.visible_output_tokens, 8_192);
        assert!(allowance.generation_limit() <= uncapped.max_output_tokens);

        let off = ControlSettings {
            max_output_tokens: 16_384,
            thinking_level: crate::models::ThinkingLevel::Off,
            ..ControlSettings::default()
        };
        let allowance_off = prompt_inference_allowance(&off);
        assert_eq!(allowance_off.thinking_budget_tokens, 0);
        assert_eq!(allowance_off.visible_output_tokens, 8_192);
    }

    #[test]
    fn develop_treats_existing_text_as_source_while_continue_preserves_a_prefix() {
        let develop = build_messages(&request(
            PromptDraftTarget::ImageAsset,
            PromptDraftMode::Develop,
        ));
        let continuation = build_messages(&request(
            PromptDraftTarget::ImageAsset,
            PromptDraftMode::Continue,
        ));
        assert!(develop.iter().any(|message| message["content"]
            .as_str()
            .unwrap_or_default()
            .contains("loose idea")));
        assert!(continuation.iter().any(|message| message["content"]
            .as_str()
            .unwrap_or_default()
            .contains("immutable prefix")));
        assert!(develop.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("replacement image description"));
        assert!(continuation.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("only new text to append"));
    }

    #[test]
    fn reference_limit_applies_only_to_continuation_output_space() {
        let models = vec![model()];
        let mut near_full = request(
            PromptDraftTarget::ReferenceDescription,
            PromptDraftMode::Continue,
        );
        near_full.existing_text = "x".repeat(MAX_REFERENCE_DESCRIPTION_BYTES - 128);
        assert!(validate_request(&near_full, &models).is_err());
        near_full.mode = PromptDraftMode::Develop;
        assert!(validate_request(&near_full, &models).is_ok());
    }

    #[test]
    fn image_composition_continuation_reserves_a_complete_replacement_object() {
        let models = vec![model()];
        let mut composition = request(
            PromptDraftTarget::ImageComposition,
            PromptDraftMode::Continue,
        );
        composition.existing_text = "x".repeat(MAX_MOVIE_PROMPT_BYTES - 128);
        assert!(validate_request(&composition, &models).is_ok());
        assert_eq!(
            output_byte_limit(&composition, composition.existing_text.trim_end()),
            MAX_MOVIE_PROMPT_BYTES
        );

        composition.target = PromptDraftTarget::Story;
        assert!(validate_request(&composition, &models).is_err());
    }

    #[test]
    fn rejects_unknown_models_and_invalid_reference_metadata() {
        let models = vec![model()];
        let mut value = request(PromptDraftTarget::Story, PromptDraftMode::Develop);
        value.model_id = "missing".into();
        assert!(validate_request(&value, &models).is_err());
        value = request(
            PromptDraftTarget::ReferenceDescription,
            PromptDraftMode::Develop,
        );
        value.asset_kind = "document".into();
        assert!(validate_request(&value, &models).is_err());
        value = request(PromptDraftTarget::ImageAsset, PromptDraftMode::Continue);
        value.existing_text.clear();
        assert!(validate_request(&value, &models).is_err());
    }

    #[test]
    fn byte_limiter_never_splits_utf8() {
        assert_eq!(utf8_prefix("a🎬b", 4), "a");
        assert_eq!(utf8_prefix("a🎬b", 5), "a🎬");
    }
}
