//! MiniMax H3 6-channel Full Reference, 3-channel Base, and Character/Head Swap prompt compiler,
//! `@Handle` -> `<Subject N>` / `<Picture N>` expansion, Guide resolution, and 3-pass LoRA selection.

use super::{
    h3_geometry::{adapt_pass_canvas, calculate_take_placement, H3_MIN_FRAMES},
    h3_references::{extract_handles, normalize_handle, resolve_window_references},
    MovieProject, MovieReferenceAsset, StudioError,
};
use kestrel_app_core::{
    H3AdvisorySeverity, H3CompileAdvisory, H3CompiledPromptChannels, H3EnvironmentSnapshot,
    H3GenerationSettings, H3LoraSpec, H3ModelChoice, H3OrdinalManifest, H3PassPreset,
    H3PromptFormat, H3PromptSection, H3ReferencePopulation, H3ReferenceRole, H3ResolvedGuide,
    H3RetentionLevel, H3Scene, H3SceneKind, H3SubjectBinding, H3TakePlacementGeometry, H3TaskMode,
    H3VocalEvent, H3VocalEventType, H3WindowCompileResult, H3WindowConfig, MovieH3Defaults,
};
use std::collections::HashMap;

pub const UNET_REF2VA: &str = "minimax_h3_ref2va_pruned_int8_convrot.safetensors";
pub const UNET_FL2VA: &str = "minimax_h3_fl2va_pruned_int8_convrot.safetensors";

pub fn format_h3_timestamp(seconds: f64) -> String {
    let clamped = seconds.max(0.0);
    let total_ms = (clamped * 1000.0).round() as u64;
    let minutes = total_ms / 60_000;
    let rem_ms = total_ms % 60_000;
    let secs = rem_ms / 1000;
    let millis = rem_ms % 1000;
    format!("{minutes:02}:{secs:02}.{millis:03}")
}

/// Replaces `@Handle` tokens at word boundaries using `expansion_map` (lowercase handle -> tag).
pub fn expand_handles_in_text(
    text: &str,
    expansion_map: &HashMap<String, String>,
    fallback_names: &HashMap<String, String>,
) -> String {
    if !text.contains('@') {
        return text.trim().to_string();
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = String::with_capacity(text.len() + 32);
    let mut i = 0;
    while i < chars.len() {
        let (_, ch) = chars[i];
        let prev = if i > 0 { Some(chars[i - 1].1) } else { None };
        let boundary = super::h3_references::is_handle_boundary_before(prev);
        if ch == '@' && boundary && i + 1 < chars.len() && chars[i + 1].1.is_ascii_alphabetic() {
            let start_byte = chars[i + 1].0;
            let mut end_idx = i + 2;
            while end_idx < chars.len()
                && (chars[end_idx].1.is_ascii_alphanumeric() || chars[end_idx].1 == '_')
            {
                end_idx += 1;
            }
            let end_byte = if end_idx < chars.len() {
                chars[end_idx].0
            } else {
                text.len()
            };
            let handle = &text[start_byte..end_byte];
            if handle.len() <= 48 {
                let lower = handle.to_ascii_lowercase();
                if let Some(tag) = expansion_map.get(&lower) {
                    out.push_str(tag);
                } else if let Some(name) = fallback_names.get(&lower) {
                    out.push_str(name);
                } else {
                    out.push_str(handle);
                }
                i = end_idx;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out.trim().to_string()
}

fn contains_literal_h3_tag(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["<picture ", "<video ", "<audio ", "<subject "]
        .iter()
        .any(|pat| lower.contains(pat))
}

pub fn scene_has_video_in_span(scene: &H3Scene, start_sec: f64, end_sec: f64) -> bool {
    let any_solo = scene.video_lanes.iter().any(|l| l.solo && !l.muted);
    let on_video = scene.video_lanes.iter().any(|lane| {
        if lane.muted || (any_solo && !lane.solo) {
            return false;
        }
        lane.clips.iter().any(|clip| {
            clip.enabled
                && clip.timeline_start_sec + clip.duration_sec > start_sec + 1e-4
                && clip.timeline_start_sec < end_sec - 1e-4
        })
    });
    let on_driver = scene.driver_lanes.iter().any(|lane| {
        !lane.muted
            && lane.clips.iter().any(|clip| {
                clip.enabled
                    && clip.timeline_start_sec + clip.duration_sec > start_sec + 1e-4
                    && clip.timeline_start_sec < end_sec - 1e-4
            })
    });
    on_video || on_driver
}

pub fn scene_has_audio_in_span(scene: &H3Scene, start_sec: f64, end_sec: f64) -> bool {
    let any_audio_solo = scene.audio_lanes.iter().any(|l| l.solo && !l.muted);
    let on_audio = scene.audio_lanes.iter().any(|lane| {
        if lane.muted || (any_audio_solo && !lane.solo) {
            return false;
        }
        lane.clips.iter().any(|clip| {
            clip.enabled
                && clip.timeline_start_sec + clip.duration_sec > start_sec + 1e-4
                && clip.timeline_start_sec < end_sec - 1e-4
        })
    });
    let any_video_solo = scene.video_lanes.iter().any(|l| l.solo && !l.muted);
    let on_video_audio = scene.video_lanes.iter().any(|lane| {
        if lane.muted || (any_video_solo && !lane.solo) {
            return false;
        }
        lane.clips.iter().any(|clip| {
            clip.enabled
                && clip.include_embedded_audio
                && clip.audio_gain > 0.0
                && clip.timeline_start_sec + clip.duration_sec > start_sec + 1e-4
                && clip.timeline_start_sec < end_sec - 1e-4
        })
    });
    on_audio || on_video_audio
}

/// Compiles a scene's `[In, Out]` window into exact MiniMax H3 prompt channels, bound ordinals,
/// guides, pass resolution, UNET/LoRA chain, AV latent mask plan, and actionable advisories.
pub fn compile_h3_window<F>(
    project: &MovieProject,
    scene: &H3Scene,
    window_override: Option<&H3WindowConfig>,
    generation_override: Option<&H3GenerationSettings>,
    env: &H3EnvironmentSnapshot,
    advanced: bool,
    resolve_asset: F,
) -> Result<H3WindowCompileResult, StudioError>
where
    F: Fn(&str) -> Result<MovieReferenceAsset, StudioError>,
{
    let defaults = MovieH3Defaults::default();
    let raw_window = window_override.unwrap_or(&scene.window);
    let gen = generation_override.unwrap_or(&scene.generation);

    // When a continuation window (in_sec >= 0.5) follows an already-placed V1 clip or an adjacent
    // earlier prompt section and the caller omitted pre_context_sec (0.0), automatically bridge
    // 23 frames (~0.958s, exact H3 Video VAE latent token 8 edge) so multi-window continuous takes
    // stay visually and acoustically seamless.
    let mut eff_window = raw_window.clone();
    let has_preceding_v1_video =
        scene_has_video_in_span(scene, (eff_window.in_sec - 0.5).max(0.0), eff_window.in_sec);
    let has_preceding_prompt_section = scene.prompt_sections.iter().any(|ps| {
        ps.enabled
            && ps.timeline_start_sec < eff_window.in_sec - 0.1
            && ps.timeline_start_sec + ps.duration_sec >= eff_window.in_sec - 0.25
    });
    let mut auto_pre_context_from_prompt_only = false;
    if eff_window.in_sec >= 0.5
        && eff_window.pre_context_sec == 0.0
        && gen.task_mode == H3TaskMode::Standard
        && scene.kind != H3SceneKind::Soundtrack
        && !scene
            .guides
            .iter()
            .any(|g| g.enabled && (g.timeline_sec - eff_window.in_sec).abs() < 0.1)
        && (has_preceding_v1_video || has_preceding_prompt_section)
    {
        let sel_dur = (eff_window.out_sec - eff_window.in_sec).max(0.2);
        if sel_dur + 1.0 <= 14.8 {
            let target_pre = (23.0_f64 / 24.0_f64).min(eff_window.in_sec);
            if !has_preceding_v1_video
                || scene_has_video_in_span(
                    scene,
                    (eff_window.in_sec - target_pre).max(0.0),
                    eff_window.in_sec,
                )
            {
                eff_window.pre_context_sec = target_pre;
            } else {
                eff_window.pre_context_sec = (10.0_f64 / 24.0_f64).min(eff_window.in_sec);
            }
            if !has_preceding_v1_video {
                auto_pre_context_from_prompt_only = true;
            }
        }
    }
    let window = &eff_window;

    let mut geometry = calculate_take_placement(window, scene.fps)?;
    let (resolved_width, resolved_height) = adapt_pass_canvas(
        (scene.width, scene.height, scene.kind),
        gen.pass_preset,
        gen.task_mode,
        (gen.custom_width, gen.custom_height),
        advanced,
    )?;

    let (mut ordinals, verdicts, mut advisories, expansion_map) = resolve_window_references(
        project,
        scene,
        window.min_overlap_ratio,
        &geometry,
        &resolve_asset,
    );

    let has_any_refs = !ordinals.pictures.is_empty()
        || !ordinals.videos.is_empty()
        || !ordinals.audios.is_empty();

    // Resolve UNET & conditioning pathway
    let resolved_model = match gen.model_choice {
        H3ModelChoice::Auto => {
            if matches!(
                gen.task_mode,
                H3TaskMode::CharacterSwap | H3TaskMode::HeadSwap
            ) || has_any_refs
            {
                UNET_REF2VA.to_string()
            } else {
                UNET_FL2VA.to_string()
            }
        }
        H3ModelChoice::Ref2va => UNET_REF2VA.to_string(),
        H3ModelChoice::Fl2va => {
            if has_any_refs {
                advisories.push(H3CompileAdvisory {
                    severity: H3AdvisorySeverity::Warning,
                    code: "fl2va_ignores_references".into(),
                    message: "Model choice is forced to FL2VA (ImageToVideo), which ignores staged Picture/Video/Audio references. Switch modelChoice to Auto or Ref2va to bind references.".into(),
                });
            }
            UNET_FL2VA.to_string()
        }
        H3ModelChoice::Ref2vaCondWithFl2vaModel => {
            format!("ref2va_cond+{UNET_FL2VA}")
        }
    };

    let uses_fl2va_weights = resolved_model.contains("fl2va");
    let turbo_role = if uses_fl2va_weights {
        "fl2v_turbo"
    } else {
        "ref2v_turbo"
    };
    let installed_turbo = env
        .discovered_loras
        .iter()
        .find(|l| l.recommended_role == turbo_role);

    let has_source_video =
        scene_has_video_in_span(scene, geometry.render_start_sec, geometry.render_end_sec);
    let has_source_audio =
        scene_has_audio_in_span(scene, geometry.render_start_sec, geometry.render_end_sec);

    // Resolve steps, denoise, and LoRA chain for Sonder's 3-pass quality ladder or Custom.
    let swap_default_denoise = match gen.task_mode {
        H3TaskMode::CharacterSwap if has_source_video => Some(0.82_f32),
        H3TaskMode::HeadSwap if has_source_video => Some(0.72_f32),
        _ => None,
    };
    let (resolved_steps, resolved_denoise, auto_turbo_strength) = match gen.pass_preset {
        H3PassPreset::Draft02Mp => {
            let steps = if installed_turbo.is_some() { 10 } else { 20 };
            if installed_turbo.is_none() {
                advisories.push(H3CompileAdvisory {
                    severity: H3AdvisorySeverity::Info,
                    code: "turbo_lora_missing_adapted_steps".into(),
                    message: "No H3 Turbo LoRA detected in ComfyUI models/loras; adapted Draft 0.2MP pass from 10 to 20 steps. Install H3 Turbo LoRAs in Setup/Studio for faster 10-step drafts.".into(),
                });
            }
            let denoise = if gen.denoise < 0.999 && has_source_video {
                gen.denoise.clamp(0.05, 1.0)
            } else {
                swap_default_denoise.unwrap_or(1.0)
            };
            (steps, denoise, Some(0.8_f32))
        }
        H3PassPreset::Detail1Mp => {
            let steps = if installed_turbo.is_some() { 12 } else { 22 };
            if installed_turbo.is_none() {
                advisories.push(H3CompileAdvisory {
                    severity: H3AdvisorySeverity::Info,
                    code: "turbo_lora_missing_adapted_steps".into(),
                    message: "No H3 Turbo LoRA detected in ComfyUI models/loras; adapted Detail 1MP pass from 12 to 22 steps.".into(),
                });
            }
            let denoise = if has_source_video {
                if gen.denoise < 0.999 {
                    gen.denoise.clamp(0.05, 1.0)
                } else {
                    0.75
                }
            } else {
                advisories.push(H3CompileAdvisory {
                    severity: H3AdvisorySeverity::Info,
                    code: "detail_pass_without_draft_source".into(),
                    message: "Detail 1MP pass has no underlying timeline clip in this window, so denoise defaults to 1.0 (fresh generation). Run Draft 0.2MP first or use expandQualityLadder for Sonder's 3-pass workflow.".into(),
                });
                1.0
            };
            (steps, denoise, Some(0.5_f32))
        }
        H3PassPreset::Upscale1080p => {
            let steps = if installed_turbo.is_some() { 4 } else { 10 };
            if installed_turbo.is_none() {
                advisories.push(H3CompileAdvisory {
                    severity: H3AdvisorySeverity::Info,
                    code: "turbo_lora_missing_adapted_steps".into(),
                    message: "No H3 Turbo LoRA detected in ComfyUI models/loras; adapted 1080p Upscale pass from 4 to 10 steps.".into(),
                });
            }
            let denoise = if has_source_video {
                if gen.denoise < 0.999 {
                    gen.denoise.clamp(0.05, 1.0)
                } else {
                    0.40
                }
            } else {
                advisories.push(H3CompileAdvisory {
                    severity: H3AdvisorySeverity::Warning,
                    code: "upscale_pass_without_source".into(),
                    message: "1080p Upscale pass works best at denoise 0.40 over a Draft/Detail take, but no video clip is placed in this window yet (falling back to denoise 1.0).".into(),
                });
                1.0
            };
            (steps, denoise, Some(0.5_f32))
        }
        H3PassPreset::Custom => (
            gen.steps.clamp(1, if advanced { 100 } else { 50 }),
            gen.denoise.clamp(0.05, 1.0),
            None,
        ),
    };

    let mut resolved_loras = Vec::new();
    if let (Some(turbo), Some(strength)) = (installed_turbo, auto_turbo_strength) {
        let explicitly_configured = gen
            .loras
            .iter()
            .any(|l| l.file_name.eq_ignore_ascii_case(&turbo.file_name));
        if !explicitly_configured {
            resolved_loras.push(H3LoraSpec {
                file_name: turbo.file_name.clone(),
                strength_model: strength,
                enabled: true,
            });
        }
    }
    for custom_lora in &gen.loras {
        if custom_lora.enabled
            && !custom_lora.file_name.trim().is_empty()
            && custom_lora.strength_model.abs() > 1e-4
        {
            match super::validate_comfy_filename(custom_lora.file_name.trim()) {
                Ok(valid_name) => {
                    resolved_loras.push(H3LoraSpec {
                        file_name: valid_name,
                        strength_model: custom_lora.strength_model.clamp(-2.0, 2.0),
                        enabled: true,
                    });
                }
                Err(err) => {
                    advisories.push(H3CompileAdvisory {
                        severity: H3AdvisorySeverity::Error,
                        code: "invalid_lora_filename".into(),
                        message: format!("Rejected LoRA filename '{}': {err}", custom_lora.file_name),
                    });
                }
            }
        }
    }

    // Validate task mode source media requirements & compute AV latent / noise mask flags.
    if matches!(
        gen.task_mode,
        H3TaskMode::AudioOverExistingVideo | H3TaskMode::CharacterSwap | H3TaskMode::HeadSwap
    ) && !has_source_video
    {
        advisories.push(H3CompileAdvisory {
            severity: H3AdvisorySeverity::Error,
            code: "missing_source_video".into(),
            message: format!(
                "Task mode {:?} requires an existing video or driver clip overlapping [{:.2}s..{:.2}s].",
                gen.task_mode, geometry.render_start_sec, geometry.render_end_sec
            ),
        });
    }
    if gen.task_mode == H3TaskMode::KeepExistingAudio && !has_source_audio {
        advisories.push(H3CompileAdvisory {
            severity: H3AdvisorySeverity::Warning,
            code: "missing_source_audio".into(),
            message: "KeepExistingAudio is selected, but no audible clip overlaps this window; silent audio will be preserved.".into(),
        });
    }

    let has_context_frames =
        geometry.mask_start_frame > 0 || geometry.mask_end_frame < geometry.padded_frames;
    let uses_source_av_latent = (resolved_denoise < 0.999 && has_source_video)
        || (has_context_frames && (has_source_video || has_source_audio))
        || (window.spatial_mask_rect.is_some() && has_source_video)
        || matches!(
            gen.task_mode,
            H3TaskMode::KeepExistingAudio
                | H3TaskMode::AudioOverExistingVideo
                | H3TaskMode::CharacterSwap
                | H3TaskMode::HeadSwap
        );

    if has_context_frames && !has_source_video && !has_source_audio {
        if auto_pre_context_from_prompt_only {
            advisories.push(H3CompileAdvisory {
                severity: H3AdvisorySeverity::Info,
                code: "auto_pre_context_armed".into(),
                message: format!(
                    "Automatic {:.2}s (23-frame) pre-context bridge is armed for continuity; render the preceding window first so V1 supplies the frozen lead-in frames.",
                    window.pre_context_sec
                ),
            });
        } else {
            advisories.push(H3CompileAdvisory {
                severity: H3AdvisorySeverity::Warning,
                code: "context_without_source_media".into(),
                message: "Pre/post context is configured, but no existing video or audio clip overlaps the context span yet.".into(),
            });
        }
    }

    let partial_video_tokens = geometry.video_mask_token_lo > 0
        || geometry.video_mask_token_hi < geometry.video_latent_tokens;
    let partial_audio_tokens = geometry.audio_mask_token_lo > 0
        || geometry.audio_mask_token_hi < geometry.audio_latent_tokens;

    let uses_video_noise_mask = uses_source_av_latent
        && (gen.task_mode == H3TaskMode::AudioOverExistingVideo
            || window.spatial_mask_rect.is_some()
            || partial_video_tokens);

    // Resolve overlapping prompt sections first so we also know if vocal events exist.
    let mut overlapping_sections: Vec<&H3PromptSection> = scene
        .prompt_sections
        .iter()
        .filter(|sec| {
            sec.enabled
                && sec.timeline_start_sec + sec.duration_sec > geometry.selection_in_sec + 1e-4
                && sec.timeline_start_sec < geometry.selection_out_sec - 1e-4
        })
        .collect();
    overlapping_sections.sort_by(|a, b| {
        a.timeline_start_sec
            .partial_cmp(&b.timeline_start_sec)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let has_vocal_events = overlapping_sections
        .iter()
        .any(|s| !s.vocal_events.is_empty());

    let freeze_swap_audio = matches!(
        gen.task_mode,
        H3TaskMode::CharacterSwap | H3TaskMode::HeadSwap
    ) && has_source_audio
        && ordinals.audios.is_empty()
        && !has_vocal_events;

    if gen.task_mode == H3TaskMode::KeepExistingAudio || freeze_swap_audio {
        geometry.audio_mask_token_lo = 0;
        geometry.audio_mask_token_hi = 0;
    }

    let uses_audio_noise_mask = uses_source_av_latent
        && (gen.task_mode == H3TaskMode::KeepExistingAudio
            || freeze_swap_audio
            || partial_audio_tokens);

    // Resolve Guides (`MiniMaxH3AddGuide`) inside [render_start_sec, render_end_sec)
    let mut guides = Vec::new();
    let mut sorted_guides: Vec<_> = scene
        .guides
        .iter()
        .filter(|g| g.enabled && (g.use_image || g.use_audio))
        .collect();
    sorted_guides.sort_by(|a, b| {
        a.timeline_sec
            .partial_cmp(&b.timeline_sec)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for guide in sorted_guides {
        let local_sec = guide.timeline_sec - geometry.render_start_sec;
        let frame_idx = (local_sec * geometry.fps as f64).round() as i32;
        if frame_idx < 0 || frame_idx >= geometry.unpadded_frames as i32 {
            continue;
        }
        if guides.len() >= defaults.max_guides {
            advisories.push(H3CompileAdvisory {
                severity: H3AdvisorySeverity::Warning,
                code: "guide_cap_exceeded".into(),
                message: format!(
                    "Guide '{}' exceeded the {} per-window guide limit.",
                    guide.label, defaults.max_guides
                ),
            });
            break;
        }
        let asset = match resolve_asset(&guide.asset_id) {
            Ok(a) => a,
            Err(err) => {
                advisories.push(H3CompileAdvisory {
                    severity: H3AdvisorySeverity::Error,
                    code: "missing_guide_asset".into(),
                    message: format!("Guide '{}' asset is missing: {err}", guide.label),
                });
                continue;
            }
        };
        if asset.kind == "video" {
            let clip_frames = (asset.duration_seconds * geometry.fps as f64).round() as u32;
            if clip_frames < H3_MIN_FRAMES {
                advisories.push(H3CompileAdvisory {
                    severity: H3AdvisorySeverity::Warning,
                    code: "short_guide_video".into(),
                    message: format!(
                        "Video guide '{}' has fewer than 5 frames; capture a still PNG guide for single-frame anchoring.",
                        guide.label
                    ),
                });
                continue;
            }
        }
        let use_audio = guide.use_audio && asset.has_audio;
        if guide.use_audio && !asset.has_audio {
            advisories.push(H3CompileAdvisory {
                severity: H3AdvisorySeverity::Warning,
                code: "silent_guide_audio".into(),
                message: format!(
                    "Guide '{}' enabled audio anchoring, but {} has no audio track.",
                    guide.label, asset.name
                ),
            });
        }
        guides.push(H3ResolvedGuide {
            guide_id: guide.id.clone(),
            asset_id: asset.id,
            label: if guide.label.trim().is_empty() {
                asset.name
            } else {
                guide.label.trim().to_string()
            },
            frame_idx,
            timeline_sec: guide.timeline_sec,
            use_image: guide.use_image && matches!(asset.kind.as_str(), "image" | "video"),
            use_audio,
        });
    }

    // Build fallback entity display name map for unstaged handles & check `@Handle` mentions.
    let fallback_names: HashMap<String, String> = project
        .entities
        .iter()
        .map(|e| {
            let name = if e.display_name.trim().is_empty() {
                e.handle.clone()
            } else {
                e.display_name.trim().to_string()
            };
            (e.handle.to_ascii_lowercase(), name)
        })
        .collect();

    let staged_elsewhere_handles: std::collections::HashSet<String> = scene
        .reference_lanes
        .iter()
        .filter(|lane| !lane.muted)
        .flat_map(|lane| lane.items.iter())
        .filter(|item| item.enabled)
        .filter_map(|item| {
            item.entity_id.as_deref().and_then(|eid| {
                project
                    .entities
                    .iter()
                    .find(|e| e.id == eid)
                    .map(|e| e.handle.to_ascii_lowercase())
            })
        })
        .filter(|lower| !expansion_map.contains_key(lower))
        .collect();

    let mut warned_unstaged = std::collections::HashSet::new();
    let check_prose = |text: &str,
                       shot_idx: Option<u32>,
                       ordinals: &mut H3OrdinalManifest,
                       advisories: &mut Vec<H3CompileAdvisory>,
                       warned_unstaged: &mut std::collections::HashSet<String>| {
        if contains_literal_h3_tag(text) {
            advisories.push(H3CompileAdvisory {
                severity: H3AdvisorySeverity::Warning,
                code: "literal_h3_tag_in_prose".into(),
                message: "Author prompts with @Handle tokens rather than literal <Picture>/<Subject> tags so Kestrel can renumber active window references automatically.".into(),
            });
        }
        for handle in extract_handles(text) {
            let lower = handle.to_ascii_lowercase();
            if let Some(mapped) = expansion_map.get(&lower) {
                if let Some(s_idx) = shot_idx {
                    if let Some(subj) = ordinals
                        .subjects
                        .iter_mut()
                        .find(|s| &s.subject_tag == mapped)
                    {
                        if !subj.shot_indices.contains(&s_idx) {
                            subj.shot_indices.push(s_idx);
                        }
                    }
                }
            } else if !(shot_idx.is_none() && staged_elsewhere_handles.contains(&lower))
                && warned_unstaged.insert(lower.clone())
            {
                if fallback_names.contains_key(&lower) {
                    advisories.push(H3CompileAdvisory {
                        severity: H3AdvisorySeverity::Warning,
                        code: "unstaged_entity_handle".into(),
                        message: format!(
                            "@{handle} is mentioned in the prompt, but is not staged in [{:.2}s..{:.2}s]. Stage it with stage_movie_h3_reference so H3 binds its reference media.",
                            geometry.selection_in_sec, geometry.selection_out_sec
                        ),
                    });
                } else {
                    advisories.push(H3CompileAdvisory {
                        severity: H3AdvisorySeverity::Warning,
                        code: "unknown_entity_handle".into(),
                        message: format!(
                            "@{handle} does not match any entity in this project. Create it with upsert_movie_h3_entity first."
                        ),
                    });
                }
            }
        }
    };

    let filtered_summary_lead = filter_global_lead_for_window(
        &scene.global_prompt.summary_lead,
        &expansion_map,
        &staged_elsewhere_handles,
        &fallback_names,
    );
    let filtered_detailed_lead = filter_global_lead_for_window(
        &scene.global_prompt.detailed_lead,
        &expansion_map,
        &staged_elsewhere_handles,
        &fallback_names,
    );

    check_prose(
        &filtered_summary_lead,
        None,
        &mut ordinals,
        &mut advisories,
        &mut warned_unstaged,
    );
    check_prose(
        &filtered_detailed_lead,
        None,
        &mut ordinals,
        &mut advisories,
        &mut warned_unstaged,
    );
    check_prose(
        &scene.global_prompt.overall_soundscape,
        None,
        &mut ordinals,
        &mut advisories,
        &mut warned_unstaged,
    );
    check_prose(
        &scene.global_prompt.non_diegetic_music,
        None,
        &mut ordinals,
        &mut advisories,
        &mut warned_unstaged,
    );

    for (idx, sec) in overlapping_sections.iter().enumerate() {
        let shot_no = (idx + 1) as u32;
        check_prose(
            &sec.summary_beat,
            Some(shot_no),
            &mut ordinals,
            &mut advisories,
            &mut warned_unstaged,
        );
        check_prose(
            &sec.detailed_action,
            Some(shot_no),
            &mut ordinals,
            &mut advisories,
            &mut warned_unstaged,
        );
        check_prose(
            &sec.camera_and_lighting,
            Some(shot_no),
            &mut ordinals,
            &mut advisories,
            &mut warned_unstaged,
        );
        check_prose(
            &sec.soundscape_override,
            Some(shot_no),
            &mut ordinals,
            &mut advisories,
            &mut warned_unstaged,
        );
        check_prose(
            &sec.music_override,
            Some(shot_no),
            &mut ordinals,
            &mut advisories,
            &mut warned_unstaged,
        );
        if let Some(sw) = &sec.swap_fields {
            let rep = sw.replacement_handle.trim();
            if !rep.is_empty() {
                let with_at = if rep.starts_with('@') {
                    rep.to_string()
                } else {
                    format!("@{rep}")
                };
                check_prose(
                    &with_at,
                    Some(shot_no),
                    &mut ordinals,
                    &mut advisories,
                    &mut warned_unstaged,
                );
            }
            check_prose(
                &sw.target_subject,
                Some(shot_no),
                &mut ordinals,
                &mut advisories,
                &mut warned_unstaged,
            );
            check_prose(
                &sw.extra_instruction,
                Some(shot_no),
                &mut ordinals,
                &mut advisories,
                &mut warned_unstaged,
            );
        }
        for vocal in &sec.vocal_events {
            if let Ok(speaker) = normalize_handle(&vocal.speaker_handle) {
                let lower = speaker.to_ascii_lowercase();
                if let Some(subj) = ordinals
                    .subjects
                    .iter_mut()
                    .find(|s| s.handle.eq_ignore_ascii_case(&speaker))
                {
                    if !subj.shot_indices.contains(&shot_no) {
                        subj.shot_indices.push(shot_no);
                    }
                    if subj.audio_tags.is_empty() && warned_unstaged.insert(format!("voice:{lower}"))
                    {
                        advisories.push(H3CompileAdvisory {
                            severity: H3AdvisorySeverity::Info,
                            code: "speaker_without_voice_reference".into(),
                            message: format!(
                                "@{} has dialogue in [Shot {shot_no}] without an active voice audio reference; attach a Speech Studio clip to lock vocal timbre.",
                                subj.handle
                            ),
                        });
                    }
                } else if warned_unstaged.insert(format!("speaker:{lower}")) {
                    advisories.push(H3CompileAdvisory {
                        severity: H3AdvisorySeverity::Warning,
                        code: "unstaged_speaker".into(),
                        message: format!(
                            "Vocal speaker @{speaker} in [Shot {shot_no}] is not bound to an active <Subject> in this window."
                        ),
                    });
                }
            }
        }
    }

    // Determine effective H3PromptFormat
    let effective_format = match gen.task_mode {
        H3TaskMode::CharacterSwap => H3PromptFormat::CharacterSwap,
        H3TaskMode::HeadSwap => H3PromptFormat::HeadSwap,
        H3TaskMode::SoundtrackPass if !has_any_refs => H3PromptFormat::H3Base,
        _ => {
            if gen.prompt_format == H3PromptFormat::H3FullReference && !has_any_refs {
                H3PromptFormat::H3Base
            } else {
                gen.prompt_format
            }
        }
    };

    let channels = build_prompt_channels(
        scene,
        &overlapping_sections,
        &ordinals,
        &expansion_map,
        &staged_elsewhere_handles,
        &fallback_names,
        &geometry,
        gen.task_mode,
    );

    let compiled_prompt = match effective_format {
        H3PromptFormat::H3FullReference => format!(
            "Subject Definitions:\n{}\n\nSummary:\n{}\n\nRetention Analysis:\n{}\n\nDetailed Description:\n{}\n\nOverall Soundscape:\n{}\n\nNon-Diegetic Music:\n{}",
            channels.subject_definitions,
            channels.summary,
            channels.retention_analysis,
            channels.detailed_description,
            channels.overall_soundscape,
            channels.non_diegetic_music
        ),
        H3PromptFormat::H3Base => format!(
            "Detailed Description:\n{}\n\nOverall Soundscape:\n{}\n\nNon-Diegetic Music:\n{}",
            channels.detailed_description,
            channels.overall_soundscape,
            channels.non_diegetic_music
        ),
        H3PromptFormat::CharacterSwap | H3PromptFormat::HeadSwap => {
            if ordinals.subjects.is_empty() && ordinals.pictures.is_empty() {
                advisories.push(H3CompileAdvisory {
                    severity: H3AdvisorySeverity::Warning,
                    code: "swap_missing_reference".into(),
                    message: "CharacterSwap / HeadSwap has no staged <Subject> or <Picture> reference in this window.".into(),
                });
            }
            build_swap_prompt(
                effective_format,
                &overlapping_sections,
                &ordinals,
                &expansion_map,
                &fallback_names,
                &channels,
            )
        }
    };

    if compiled_prompt.trim().is_empty()
        || (overlapping_sections.is_empty()
            && scene.global_prompt.detailed_lead.trim().is_empty()
            && scene.global_prompt.non_diegetic_music.trim().is_empty()
            && !matches!(
                effective_format,
                H3PromptFormat::CharacterSwap | H3PromptFormat::HeadSwap
            ))
    {
        advisories.push(H3CompileAdvisory {
            severity: H3AdvisorySeverity::Warning,
            code: "empty_window_direction".into(),
            message: format!(
                "No timed prompt section overlaps [{:.2}s..{:.2}s]; add a shot with upsert_movie_h3_prompt_section.",
                geometry.selection_in_sec, geometry.selection_out_sec
            ),
        });
    }

    Ok(H3WindowCompileResult {
        scene_id: scene.id.clone(),
        scene_name: scene.name.clone(),
        resolved_width,
        resolved_height,
        resolved_model,
        resolved_steps,
        resolved_denoise,
        uses_source_av_latent,
        uses_video_noise_mask,
        uses_audio_noise_mask,
        resolved_loras,
        prompt_format: effective_format,
        compiled_prompt,
        negative_prompt: scene.global_prompt.negative_prompt.trim().to_string(),
        channels,
        ordinals,
        verdicts,
        guides,
        geometry,
        advisories,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_prompt_channels(
    scene: &H3Scene,
    sections: &[&H3PromptSection],
    ordinals: &H3OrdinalManifest,
    expansion_map: &HashMap<String, String>,
    staged_elsewhere: &std::collections::HashSet<String>,
    fallback_names: &HashMap<String, String>,
    geometry: &H3TakePlacementGeometry,
    task_mode: H3TaskMode,
) -> H3CompiledPromptChannels {
    // 1. Subject Definitions
    let mut def_lines = Vec::new();
    for subj in &ordinals.subjects {
        let mut bindings = Vec::new();
        if !subj.visual_tags.is_empty() {
            bindings.push(format!(
                "Visual appearance from {}",
                subj.visual_tags.join(", ")
            ));
        }
        if !subj.audio_tags.is_empty() {
            bindings.push(format!(
                "voice timbre from {} {}",
                subj.audio_tags.join(", "),
                subj.speaker_code
            ));
        }
        let binding_clause = if bindings.is_empty() {
            String::new()
        } else {
            format!("{}. ", capitalize_first(&bindings.join("; ")))
        };
        let desc = expand_handles_in_text(&subj.subject_description, expansion_map, fallback_names);
        def_lines.push(format!(
            "- {}: {}{}",
            subj.subject_tag,
            binding_clause,
            ensure_sentence(&desc)
        ));
    }

    // Non-subject bound slots (locations, props, camera/motion videos, ambient sounds)
    for slot in ordinals
        .pictures
        .iter()
        .chain(ordinals.videos.iter())
        .chain(ordinals.audios.iter())
    {
        if slot.subject_tag.is_some() {
            continue;
        }
        // Avoid duplicating an embedded video audio slot already described alongside its <Video k>
        if slot.population == H3ReferencePopulation::Videos
            && slot.tag.starts_with("<Audio ")
        {
            def_lines.push(format!(
                "- {} (sound from {}): {}",
                slot.tag,
                slot.label,
                ensure_sentence(&expand_handles_in_text(
                    &slot.description,
                    expansion_map,
                    fallback_names
                ))
            ));
            continue;
        }
        let role_prefix = match slot.role {
            H3ReferenceRole::Scene => "Environment & location reference — ",
            H3ReferenceRole::Motion => "Motion & choreography reference — ",
            H3ReferenceRole::Camera => "Camera movement reference — ",
            H3ReferenceRole::Sound => "Acoustic reference — ",
            H3ReferenceRole::Outfit => "Wardrobe reference — ",
            _ => "",
        };
        def_lines.push(format!(
            "- {}: {}{}",
            slot.tag,
            role_prefix,
            ensure_sentence(&expand_handles_in_text(
                &slot.description,
                expansion_map,
                fallback_names
            ))
        ));
    }
    let subject_definitions = if def_lines.is_empty() {
        "N/A".to_string()
    } else {
        def_lines.join("\n")
    };

    // 2. Summary (with H3 task tags!)
    let mut task_parts = Vec::new();
    if !ordinals.pictures.is_empty() {
        task_parts.push("reference generation");
    }
    if !ordinals.videos.is_empty() {
        task_parts.push("video reference");
    }
    if !ordinals.audios.is_empty() {
        task_parts.push("audio reference");
    }
    let task_prefix = if task_parts.is_empty() {
        String::new()
    } else {
        format!("[{}] ", task_parts.join(" + "))
    };
    let mut summary_sentences = Vec::new();
    let window_summary_lead = filter_global_lead_for_window(
        &scene.global_prompt.summary_lead,
        expansion_map,
        staged_elsewhere,
        fallback_names,
    );
    if !window_summary_lead.trim().is_empty() {
        summary_sentences.push(ensure_sentence(&expand_handles_in_text(
            &window_summary_lead,
            expansion_map,
            fallback_names,
        )));
    }
    for sec in sections {
        let beat = if !sec.summary_beat.trim().is_empty() {
            &sec.summary_beat
        } else {
            &sec.detailed_action
        };
        if !beat.trim().is_empty() {
            summary_sentences.push(ensure_sentence(&expand_handles_in_text(
                beat,
                expansion_map,
                fallback_names,
            )));
        }
    }
    let summary_body = if summary_sentences.is_empty() {
        if task_mode == H3TaskMode::SoundtrackPass {
            "Cohesive musical underscore timed to the scene.".into()
        } else {
            format!("Timed cinematic sequence in {}.", scene.name)
        }
    } else {
        summary_sentences.join(" ")
    };
    let summary = format!("{task_prefix}{summary_body}").trim().to_string();

    // 3. Retention Analysis
    let mut retention_lines = Vec::new();
    for subj in &ordinals.subjects {
        let mut refs = subj.visual_tags.clone();
        refs.extend(subj.audio_tags.iter().cloned());
        let refs_paren = if refs.is_empty() {
            String::new()
        } else {
            format!(" ({})", refs.join(", "))
        };
        let shots_paren = if subj.shot_indices.is_empty() {
            String::new()
        } else {
            let labels: Vec<String> = subj
                .shot_indices
                .iter()
                .map(|idx| format!("[Shot {idx}]"))
                .collect();
            format!(" (appears in {})", labels.join(", "))
        };
        retention_lines.push(format!(
            "- {}{}{}: {} — {}",
            subj.subject_tag,
            refs_paren,
            shots_paren,
            subj.retention.as_h3_token(),
            retention_guidance_prose(subj.retention, true)
        ));
    }
    for slot in ordinals
        .pictures
        .iter()
        .chain(ordinals.videos.iter())
        .chain(ordinals.audios.iter())
    {
        if slot.subject_tag.is_some() {
            continue;
        }
        retention_lines.push(format!(
            "- {}: {} — {}",
            slot.tag,
            slot.retention.as_h3_token(),
            retention_guidance_prose(slot.retention, false)
        ));
    }
    let retention_analysis = if retention_lines.is_empty() {
        "N/A".into()
    } else {
        retention_lines.join("\n")
    };

    // 4. Detailed Description
    let mut detailed_blocks = Vec::new();
    let window_detailed_lead = filter_global_lead_for_window(
        &scene.global_prompt.detailed_lead,
        expansion_map,
        staged_elsewhere,
        fallback_names,
    );
    if !window_detailed_lead.trim().is_empty() {
        detailed_blocks.push(ensure_sentence(&expand_handles_in_text(
            &window_detailed_lead,
            expansion_map,
            fallback_names,
        )));
    }
    for (idx, sec) in sections.iter().enumerate() {
        let shot_no = idx + 1;
        let rel_sec = (sec.timeline_start_sec - geometry.render_start_sec).max(0.0);
        let ts = format_h3_timestamp(rel_sec);
        let mut parts = Vec::new();
        if !sec.camera_and_lighting.trim().is_empty() {
            parts.push(ensure_sentence(&expand_handles_in_text(
                &sec.camera_and_lighting,
                expansion_map,
                fallback_names,
            )));
        }
        if !sec.detailed_action.trim().is_empty() {
            parts.push(ensure_sentence(&expand_handles_in_text(
                &sec.detailed_action,
                expansion_map,
                fallback_names,
            )));
        }
        for vocal in &sec.vocal_events {
            let line = format_vocal_event(vocal, &ordinals.subjects, expansion_map, fallback_names);
            if !line.is_empty() {
                parts.push(line);
            }
        }
        if !parts.is_empty() {
            detailed_blocks.push(format!(
                "[Shot {shot_no}] At {ts}, {}",
                parts.join(" ")
            ));
        }
    }
    if detailed_blocks.is_empty() {
        if task_mode == H3TaskMode::SoundtrackPass {
            detailed_blocks.push(
                "[Shot 1] At 00:00.000, Minimal dark frame accompanying the stereo musical score."
                    .into(),
            );
        } else {
            detailed_blocks.push(format!(
                "[Shot 1] At 00:00.000, Cinematic 24fps shot in {}.",
                scene.name
            ));
        }
    }
    let detailed_description = detailed_blocks.join("\n");

    // 5. Overall Soundscape
    let mut sound_parts = Vec::new();
    if !scene.global_prompt.overall_soundscape.trim().is_empty() {
        sound_parts.push(ensure_sentence(&expand_handles_in_text(
            &scene.global_prompt.overall_soundscape,
            expansion_map,
            fallback_names,
        )));
    }
    for (idx, sec) in sections.iter().enumerate() {
        if !sec.soundscape_override.trim().is_empty() {
            let rel_sec = (sec.timeline_start_sec - geometry.render_start_sec).max(0.0);
            sound_parts.push(format!(
                "At {} ([Shot {}]): {}",
                format_h3_timestamp(rel_sec),
                idx + 1,
                ensure_sentence(&expand_handles_in_text(
                    &sec.soundscape_override,
                    expansion_map,
                    fallback_names
                ))
            ));
        }
    }
    let overall_soundscape = if sound_parts.is_empty() {
        if task_mode == H3TaskMode::SoundtrackPass {
            "N/A".into()
        } else {
            "Natural room tone and physically grounded foley synchronized with the on-screen motion.".into()
        }
    } else {
        sound_parts.join(" ")
    };

    // 6. Non-Diegetic Music
    let mut music_parts = Vec::new();
    if !scene.global_prompt.non_diegetic_music.trim().is_empty() {
        music_parts.push(ensure_sentence(&expand_handles_in_text(
            &scene.global_prompt.non_diegetic_music,
            expansion_map,
            fallback_names,
        )));
    }
    for (idx, sec) in sections.iter().enumerate() {
        if !sec.music_override.trim().is_empty() {
            let rel_sec = (sec.timeline_start_sec - geometry.render_start_sec).max(0.0);
            music_parts.push(format!(
                "At {} ([Shot {}]): {}",
                format_h3_timestamp(rel_sec),
                idx + 1,
                ensure_sentence(&expand_handles_in_text(
                    &sec.music_override,
                    expansion_map,
                    fallback_names
                ))
            ));
        }
    }
    let non_diegetic_music = if music_parts.is_empty() {
        "N/A".into()
    } else {
        music_parts.join(" ")
    };

    H3CompiledPromptChannels {
        subject_definitions,
        summary,
        retention_analysis,
        detailed_description,
        overall_soundscape,
        non_diegetic_music,
    }
}

fn filter_global_lead_for_window(
    lead: &str,
    expansion_map: &HashMap<String, String>,
    staged_elsewhere: &std::collections::HashSet<String>,
    fallback_names: &HashMap<String, String>,
) -> String {
    if staged_elsewhere.is_empty() || lead.trim().is_empty() {
        return lead.to_string();
    }
    let mentions_set = |clause: &str, set: &std::collections::HashSet<String>| -> bool {
        let handles = extract_handles(clause);
        if handles
            .iter()
            .any(|h| set.contains(&h.to_ascii_lowercase()))
        {
            return true;
        }
        let lower_clause = clause.to_ascii_lowercase();
        let words: Vec<&str> = lower_clause
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|w| !w.is_empty())
            .collect();
        set.iter().any(|h| {
            words.contains(&h.as_str())
                || fallback_names
                    .get(h)
                    .map(|dn| dn.to_ascii_lowercase())
                    .is_some_and(|dn| words.contains(&dn.as_str()))
        })
    };
    let active_handles: std::collections::HashSet<String> =
        expansion_map.keys().cloned().collect();

    let mut kept_sentences = Vec::new();
    for raw_sent in lead.split(". ") {
        let trimmed_sent = raw_sent.trim().trim_end_matches('.');
        if trimmed_sent.is_empty() {
            continue;
        }
        let mut kept_clauses = Vec::new();
        for clause in trimmed_sent.split("; ") {
            let c = clause.trim();
            if c.is_empty() {
                continue;
            }
            let has_elsewhere = mentions_set(c, staged_elsewhere);
            let has_active = mentions_set(c, &active_handles);
            if has_elsewhere && !has_active {
                continue;
            }
            kept_clauses.push(c);
        }
        if !kept_clauses.is_empty() {
            kept_sentences.push(format!("{}.", kept_clauses.join("; ")));
        }
    }
    if kept_sentences.is_empty() {
        lead.to_string()
    } else {
        kept_sentences.join(" ")
    }
}

fn format_vocal_event(
    vocal: &H3VocalEvent,
    subjects: &[H3SubjectBinding],
    expansion_map: &HashMap<String, String>,
    fallback_names: &HashMap<String, String>,
) -> String {
    let spoken = vocal.text.trim();
    if spoken.is_empty() {
        return String::new();
    }
    let handle_norm = vocal.speaker_handle.trim().trim_start_matches('@');
    let speaker = if let Some(subj) = subjects
        .iter()
        .find(|s| s.handle.eq_ignore_ascii_case(handle_norm))
    {
        format!("{} {}", subj.subject_tag, subj.speaker_code)
    } else if let Some(name) = fallback_names.get(&handle_norm.to_ascii_lowercase()) {
        name.clone()
    } else if !handle_norm.is_empty() {
        handle_norm.to_string()
    } else {
        "Speaker".into()
    };

    let verb = match vocal.event_type {
        H3VocalEventType::Dialogue => "says",
        H3VocalEventType::Narration => "narrates",
        H3VocalEventType::Singing => "sings",
        H3VocalEventType::Shout => "shouts",
        H3VocalEventType::Whisper => "whispers",
    };
    let delivery = expand_handles_in_text(&vocal.delivery, expansion_map, fallback_names);
    let delivery_clause = if delivery.is_empty() {
        String::new()
    } else {
        format!(" {delivery}")
    };
    let lang = vocal.language.trim();
    let lang_prefix = if lang.is_empty() {
        String::new()
    } else {
        format!("[{lang}] ")
    };
    format!("{speaker} {verb}{delivery_clause}: <d>{lang_prefix}{spoken}</d>")
}

fn build_swap_prompt(
    format: H3PromptFormat,
    sections: &[&H3PromptSection],
    ordinals: &H3OrdinalManifest,
    expansion_map: &HashMap<String, String>,
    fallback_names: &HashMap<String, String>,
    channels: &H3CompiledPromptChannels,
) -> String {
    let prefix = if format == H3PromptFormat::HeadSwap {
        "head_swap"
    } else {
        "body_swap"
    };
    let swap = sections.iter().find_map(|s| s.swap_fields.as_ref());
    let target = swap
        .map(|s| s.target_subject.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("the primary character in the source video");

    let replacement = if let Some(sw) = swap {
        let rep_norm = sw.replacement_handle.trim().trim_start_matches('@');
        if let Some(subj) = ordinals
            .subjects
            .iter()
            .find(|s| s.handle.eq_ignore_ascii_case(rep_norm))
        {
            if subj.visual_tags.is_empty() {
                subj.subject_tag.clone()
            } else {
                format!("{} ({})", subj.subject_tag, subj.visual_tags.join(", "))
            }
        } else if let Some(mapped) = expansion_map.get(&rep_norm.to_ascii_lowercase()) {
            mapped.clone()
        } else if let Some(pic) = ordinals.pictures.first() {
            pic.tag.clone()
        } else {
            "<Picture 1>".into()
        }
    } else if let Some(subj) = ordinals.subjects.first() {
        if subj.visual_tags.is_empty() {
            subj.subject_tag.clone()
        } else {
            format!("{} ({})", subj.subject_tag, subj.visual_tags.join(", "))
        }
    } else if let Some(pic) = ordinals.pictures.first() {
        pic.tag.clone()
    } else {
        "<Picture 1>".into()
    };

    let keep_clause = if swap.is_none_or(|s| s.keep_pose_and_lighting) {
        " Preserve the original body pose, performance timing, camera movement, scene lighting, and surrounding background without drift."
    } else {
        ""
    };
    let extra = swap
        .map(|s| expand_handles_in_text(&s.extra_instruction, expansion_map, fallback_names))
        .filter(|s| !s.is_empty())
        .map(|s| format!(" {}", ensure_sentence(&s)))
        .unwrap_or_default();

    let mut body = if format == H3PromptFormat::HeadSwap {
        format!(
            "{prefix}: Replace the head and facial identity of {target} with {replacement}.{keep_clause}{extra}"
        )
    } else {
        format!(
            "{prefix}: Replace {target} with {replacement}.{keep_clause}{extra}"
        )
    };
    if !channels.detailed_description.trim().is_empty() {
        body.push_str("\n\n");
        body.push_str(channels.detailed_description.trim());
    }

    format!(
        "Subject Definitions:\n{}\n\nDetailed Description:\n{}\n\nOverall Soundscape:\n{}\n\nNon-Diegetic Music:\n{}",
        channels.subject_definitions,
        body,
        channels.overall_soundscape,
        channels.non_diegetic_music
    )
}

fn retention_guidance_prose(level: H3RetentionLevel, is_subject: bool) -> &'static str {
    match level {
        H3RetentionLevel::FullyPreserved => {
            if is_subject {
                "strictly preserve facial identity, proportions, silhouette, and signature wardrobe across every shot."
            } else {
                "strictly preserve exact visual structure, materials, and details."
            }
        }
        H3RetentionLevel::PartiallyPreserved => {
            "preserve core identity while adapting pose, expression, and scene lighting naturally."
        }
        H3RetentionLevel::AttributeTransfer => {
            "transfer key visual or stylistic attributes into the scene geometry."
        }
        H3RetentionLevel::WeakReference => {
            "use loosely for mood and compositional inspiration."
        }
        H3RetentionLevel::Reference => {
            "maintain recognizable layout, texture, and acoustic character."
        }
        H3RetentionLevel::FullyCopy => {
            "reproduce exact appearance and motion trajectories with minimal deviation."
        }
    }
}

fn capitalize_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

fn ensure_sentence(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.ends_with(['.', '!', '?', '"', '>']) {
        trimmed.to_string()
    } else {
        format!("{trimmed}.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestrel_app_core::{
        H3AudioIntent, H3DiscoveredLora, H3EntityKind, H3GlobalPrompt, H3ReferenceEntity,
        H3ReferenceLane, H3ReferenceMember, H3SceneKind, H3StagedReferenceItem, H3VisualIntent,
        MovieEdit, MovieSettings,
    };
    use serde_json::Value;

    fn dummy_env() -> H3EnvironmentSnapshot {
        H3EnvironmentSnapshot {
            ready: true,
            comfy_root: "/comfy".into(),
            has_ref2va: true,
            has_fl2va: true,
            has_qwen3vl: true,
            has_video_vae: true,
            has_audio_vae: true,
            has_taeh3_preview: true,
            has_ref2v_turbo_lora: true,
            has_fl2v_turbo_lora: true,
            discovered_loras: vec![
                H3DiscoveredLora {
                    file_name: "minimax_h3_ref2v_turbo_4step_v0.1_comfyui_bf16.safetensors".into(),
                    bytes: 1_956_193_000,
                    recommended_role: "ref2v_turbo".into(),
                    default_strength: 0.8,
                },
                H3DiscoveredLora {
                    file_name: "minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors".into(),
                    bytes: 1_956_193_000,
                    recommended_role: "fl2v_turbo".into(),
                    default_strength: 0.8,
                },
            ],
            detail: "Ready".into(),
        }
    }

    fn mk_asset(ch: char, kind: &str) -> MovieReferenceAsset {
        MovieReferenceAsset {
            id: std::iter::repeat_n(ch, 64).collect(),
            name: format!("asset-{ch}"),
            kind: kind.into(),
            mime_type: "image/png".into(),
            bytes: 100,
            duration_seconds: if kind == "image" { 0.0 } else { 3.0 },
            width: 1024,
            height: 1024,
            has_audio: kind != "image",
            path: format!("/tmp/{ch}"),
            created_at: String::new(),
            generation: None,
        }
    }

    #[test]
    fn compiles_6_channel_h3_full_reference_with_subjects_props_and_dialogue() {
        let a_cowboy = mk_asset('1', "image");
        let a_voice = mk_asset('2', "audio");
        let a_bottle = mk_asset('3', "image");
        let assets: HashMap<String, MovieReferenceAsset> =
            [a_cowboy.clone(), a_voice.clone(), a_bottle.clone()]
                .into_iter()
                .map(|a| (a.id.clone(), a))
                .collect();

        let cowboy = H3ReferenceEntity {
            id: "ent-cowboy".into(),
            handle: "Cowboy".into(),
            display_name: "The Drifter".into(),
            kind: H3EntityKind::Character,
            subject_description: "Sun-weathered gunslinger in a dusty leather coat".into(),
            retention_level: H3RetentionLevel::FullyPreserved,
            default_visual_intent: H3VisualIntent::Preserve,
            default_audio_intent: H3AudioIntent::VoiceIdentity,
            members: vec![
                H3ReferenceMember {
                    id: "m1".into(),
                    asset_id: a_cowboy.id.clone(),
                    label: "Portrait".into(),
                    population: H3ReferencePopulation::Pictures,
                    role: H3ReferenceRole::Subject,
                    crop: None,
                    source_start_sec: 0.0,
                    source_end_sec: 0.0,
                    use_embedded_audio: false,
                    description_override: String::new(),
                },
                H3ReferenceMember {
                    id: "m2".into(),
                    asset_id: a_voice.id.clone(),
                    label: "Voice".into(),
                    population: H3ReferencePopulation::StandaloneAudios,
                    role: H3ReferenceRole::Voice,
                    crop: None,
                    source_start_sec: 0.0,
                    source_end_sec: 3.0,
                    use_embedded_audio: false,
                    description_override: String::new(),
                },
            ],
            created_at: String::new(),
            updated_at: String::new(),
        };

        let bottle = H3ReferenceEntity {
            id: "ent-bottle".into(),
            handle: "SonderBottle".into(),
            display_name: "Amber Bottle".into(),
            kind: H3EntityKind::Prop,
            subject_description: "Square amber whiskey bottle with a torn paper label".into(),
            retention_level: H3RetentionLevel::FullyPreserved,
            default_visual_intent: H3VisualIntent::Preserve,
            default_audio_intent: H3AudioIntent::IgnoreEmbedded,
            members: vec![H3ReferenceMember {
                id: "m3".into(),
                asset_id: a_bottle.id.clone(),
                label: "Bottle".into(),
                population: H3ReferencePopulation::Pictures,
                role: H3ReferenceRole::General,
                crop: None,
                source_start_sec: 0.0,
                source_end_sec: 0.0,
                use_embedded_audio: false,
                description_override: String::new(),
            }],
            created_at: String::new(),
            updated_at: String::new(),
        };

        let scene = H3Scene {
            id: "sc-1".into(),
            name: "Saloon Showdown".into(),
            kind: H3SceneKind::Standard,
            width: 1920,
            height: 1088,
            fps: 24,
            duration_sec: 10.0,
            window: H3WindowConfig {
                in_sec: 0.0,
                out_sec: 5.0,
                ..Default::default()
            },
            generation: H3GenerationSettings::default(),
            global_prompt: H3GlobalPrompt {
                summary_lead: "A tense frontier tavern exchange.".into(),
                detailed_lead: "Anamorphic 35mm film grain, warm tungsten bar lanterns.".into(),
                overall_soundscape: "Creaking floorboards and distant desert wind.".into(),
                non_diegetic_music: String::new(),
                negative_prompt: String::new(),
            },
            prompt_sections: vec![H3PromptSection {
                id: "sec-1".into(),
                label: "Shot 1".into(),
                timeline_start_sec: 0.0,
                duration_sec: 5.0,
                summary_beat: "@Cowboy sets @SonderBottle on the bar".into(),
                detailed_action: "@Cowboy slides @SonderBottle across the scarred oak counter".into(),
                camera_and_lighting: "Medium close-up, shallow depth of field".into(),
                soundscape_override: "Glass clinks against oak".into(),
                music_override: String::new(),
                vocal_events: vec![H3VocalEvent {
                    id: "v1".into(),
                    speaker_handle: "@Cowboy".into(),
                    event_type: H3VocalEventType::Dialogue,
                    language: "English".into(),
                    delivery: "in a low rasp".into(),
                    text: "Keep the change.".into(),
                }],
                swap_fields: None,
                enabled: true,
            }],
            video_lanes: Vec::new(),
            audio_lanes: Vec::new(),
            driver_lanes: Vec::new(),
            reference_lanes: vec![H3ReferenceLane {
                id: "r1".into(),
                name: "R1".into(),
                muted: false,
                locked: false,
                items: vec![
                    H3StagedReferenceItem {
                        id: "i1".into(),
                        entity_id: Some("ent-cowboy".into()),
                        member_id: None,
                        asset_id: None,
                        label: "@Cowboy".into(),
                        role_override: None,
                        retention_override: None,
                        description_override: String::new(),
                        timeline_start_sec: 0.0,
                        duration_sec: 5.0,
                        use_visual: true,
                        use_audio: true,
                        enabled: true,
                    },
                    H3StagedReferenceItem {
                        id: "i2".into(),
                        entity_id: Some("ent-bottle".into()),
                        member_id: None,
                        asset_id: None,
                        label: "@SonderBottle".into(),
                        role_override: None,
                        retention_override: None,
                        description_override: String::new(),
                        timeline_start_sec: 0.0,
                        duration_sec: 5.0,
                        use_visual: true,
                        use_audio: false,
                        enabled: true,
                    },
                ],
            }],
            guides: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };

        let project = MovieProject {
            schema_version: 9,
            id: "proj-1".into(),
            prompt: String::new(),
            title: "Western".into(),
            status: "ready".into(),
            phase: "editing".into(),
            detail: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
            model: String::new(),
            legacy_model_roles: Value::Null,
            renderer: "MiniMax H3".into(),
            settings: MovieSettings::default(),
            references: Vec::new(),
            plan: None,
            sources: Vec::new(),
            clips: Vec::new(),
            edit: MovieEdit {
                clips: Vec::new(),
                export_title: "Western".into(),
                export_preset: "publish".into(),
                normalize_audio: false,
                target_lufs: -14.0,
                markers: Vec::new(),
            },
            final_path: String::new(),
            exports: Vec::new(),
            error: String::new(),
            producer_review_required: false,
            producer_approved_at: String::new(),
            story_markdown: String::new(),
            story_revisions: Vec::new(),
            entities: vec![cowboy, bottle],
            scenes: Vec::new(),
            active_scene_id: "sc-1".into(),
            render_jobs: Vec::new(),
            legacy_producer_feedback: Vec::new(),
            legacy_copilot_history: Vec::new(),
        };

        let res = compile_h3_window(
            &project,
            &scene,
            None,
            None,
            &dummy_env(),
            false,
            |aid| assets.get(aid).cloned().ok_or_else(|| StudioError::NotFound(aid.into())),
        )
        .unwrap();

        assert_eq!(res.prompt_format, H3PromptFormat::H3FullReference);
        assert_eq!(res.resolved_width, 768);
        assert_eq!(res.resolved_height, 448);
        assert_eq!(res.resolved_steps, 10);
        assert_eq!(res.resolved_loras.len(), 1);
        assert!((res.resolved_loras[0].strength_model - 0.8).abs() < 1e-4);
        assert!(res.advisories.is_empty());

        // Check exact 6-channel structure & expansions
        assert!(res.compiled_prompt.contains("Subject Definitions:\n- <Subject 1>: Visual appearance from <Picture 1>; voice timbre from <Audio 1> (S1). Sun-weathered gunslinger in a dusty leather coat."));
        assert!(res.compiled_prompt.contains("- <Picture 2>: Square amber whiskey bottle with a torn paper label."));
        assert!(res.compiled_prompt.contains("Summary:\n[reference generation + audio reference] A tense frontier tavern exchange. <Subject 1> sets <Picture 2> on the bar."));
        assert!(res.compiled_prompt.contains("Retention Analysis:\n- <Subject 1> (<Picture 1>, <Audio 1>) (appears in [Shot 1]): fully_preserved"));
        assert!(res.compiled_prompt.contains("[Shot 1] At 00:00.000, Medium close-up, shallow depth of field. <Subject 1> slides <Picture 2> across the scarred oak counter. <Subject 1> (S1) says in a low rasp: <d>[English] Keep the change.</d>"));
        assert!(res.compiled_prompt.contains("Non-Diegetic Music:\nN/A"));
    }
}
