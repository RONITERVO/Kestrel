//! Durable MiniMax H3 Studio orchestration on `MovieStudio`:
//! environment & LoRA discovery, `@Handle` entity library, multi-scene timelines,
//! 3-pass quality ladder queueing, ComfyUI execution, color-drift cancellation,
//! native take quality telemetry, and BT.709 / peak-headroom export.

use super::{
    check_cancel, comfy_execution_error, export, find_output_media,
    h3_digest::build_production_digest,
    h3_geometry::{
        calculate_take_placement, default_scene_dimensions, validate_canvas_dimensions,
        validate_normalized_crop, H3_FPS,
    },
    h3_graph::{build_h3_comfy_graph, H3ComfyGraphBuild, H3InstalledModelNames},
    h3_media::{
        analyze_take_quality, apply_context_color_drift_correction, capture_frame_png,
        render_scene_window_sources, stage_bound_reference_slot, stage_guide_item,
        write_spatial_mask_png,
    },
    h3_prompt::{compile_h3_window, format_h3_timestamp},
    h3_references::{
        auto_stage_reference, default_role_for_entity, infer_population_for_kind, normalize_handle,
    },
    hash_file, hash_reference,
    live_preview::{emit_preview_unavailable, LivePreviewSession, PreviewTarget},
    safe_export_stem, truncate, write_json_atomic, MovieExport, MovieProject,
    MovieReferenceAsset, MovieStudio, RenderedClip, StudioError, COMFY_BASE, COMFY_RENDER_TIMEOUT,
    SCHEMA_VERSION,
};
use chrono::Utc;
use futures_util::StreamExt;
use kestrel_app_core::{
    CaptureMovieH3FrameRequest, CompileMovieH3WindowRequest, CreateMovieH3ProjectRequest,
    CreateMovieH3SceneRequest, DeleteMovieH3EntityRequest, H3AdvisorySeverity, H3AudioClip,
    H3AudioLane, H3ClipSourceKind, H3DiscoveredLora, H3DriverClip, H3DriverLane, H3EntityKind,
    H3EnvironmentSnapshot, H3GenerationSettings, H3GenerationStartReceipt, H3GlobalPrompt,
    H3GuideItem, H3PassPreset, H3ProductionDigest, H3PromptFormat, H3PromptSection,
    H3QueuedWindowSpec, H3ReferenceEntity, H3ReferenceLane, H3RenderJobStatus, H3RenderQueueJob,
    H3Scene, H3SceneKind, H3StageReferenceReceipt, H3StagedReferenceItem,
    H3StartGenerationRequest, H3TakeInspection, H3TakePlacementGeometry, H3TakePlacementMode,
    H3TaskMode, H3UpsertPromptSectionReceipt, H3VideoClip, H3VideoLane, H3WindowCompileResult,
    H3WindowConfig, MovieEdit, MovieExportReceipt, MovieSettings, MovieStoryRevision,
    MovieStoryRevisionOrigin, PlaceMovieH3TakeRequest, RenderMovieExportRequest,
    SaveMovieH3SceneRequest, SaveMovieH3StoryRequest, StageMovieH3ReferenceRequest,
    UpsertMovieH3EntityRequest, UpsertMovieH3PromptSectionRequest,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use tauri::AppHandle;
use tokio_util::sync::CancellationToken;

const MAX_STORY_MARKDOWN_BYTES: usize = 256 * 1024;
const MAX_ENTITIES_PER_PROJECT: usize = 256;
const MAX_SCENES_PER_PROJECT: usize = 32;
const MAX_SECTIONS_PER_SCENE: usize = 256;
const MAX_QUEUED_WINDOWS_PER_CALL: usize = 24;

const H3_LORA_REPO_REV: &str = "54472221d27c695fca9621b0c3ed52f06c4af707";

struct PinnedTurboLora {
    file_name: &'static str,
    remote_path: &'static str,
    bytes: u64,
    sha256: &'static str,
}

const PINNED_TURBO_LORAS: &[PinnedTurboLora] = &[
    PinnedTurboLora {
        file_name: "minimax_h3_ref2v_turbo_4step_v0.1_comfyui_bf16.safetensors",
        remote_path: "loras/minimax_h3_ref2v_turbo_4step_v0.1_comfyui_bf16.safetensors",
        bytes: 1_956_193_000,
        sha256: "5b9ab5ade15d0775676d01a907268a69a1468dc6033b3b0d3ded5502f3ebb84c",
    },
    PinnedTurboLora {
        file_name: "minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors",
        remote_path: "loras/minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors",
        bytes: 1_956_193_000,
        sha256: "2339acdf19bfe123f46b971ea35d367a84adb85de43627e1eceafa5a5b2b111e",
    },
];

pub fn resolve_installed_h3_models(
    comfy_root: &str,
) -> (H3InstalledModelNames, H3EnvironmentSnapshot) {
    let root = Path::new(comfy_root);
    let models_dir = root.join("models");

    let find_first = |folder: &str, candidates: &[&str]| -> Option<String> {
        let base = models_dir.join(folder);
        for &cand in candidates {
            if base.join(cand).is_file() {
                return Some(cand.to_string());
            }
            let h3_sub = Path::new("H3").join(cand);
            if base.join(&h3_sub).is_file() {
                let sep = std::path::MAIN_SEPARATOR;
                return Some(format!("H3{sep}{cand}"));
            }
        }
        None
    };

    let found_ref2va = find_first(
        "diffusion_models",
        &[
            "minimax_h3_ref2va_pruned_int8_convrot.safetensors",
            "minimax_h3_ref2va_pruned_fp8_scaled.safetensors",
            "minimax_h3_ref2va_pruned_w6a8.safetensors",
            "minimax_h3_ref2va_int8_convrot.safetensors",
            "minimax_h3_ref2va_pruned_bf16.safetensors",
            "minimax_h3_ref2va_bf16.safetensors",
        ],
    );
    let found_fl2va = find_first(
        "diffusion_models",
        &[
            "minimax_h3_fl2va_pruned_int8_convrot.safetensors",
            "minimax_h3_fl2va_pruned_fp8_scaled.safetensors",
            "minimax_h3_fl2va_pruned_w6a8.safetensors",
            "minimax_h3_fl2va_int8_convrot.safetensors",
            "minimax_h3_fl2va_pruned_bf16.safetensors",
            "minimax_h3_fl2va_bf16.safetensors",
        ],
    );
    let found_qwen = find_first(
        "text_encoders",
        &[
            "qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors",
            "qwen3vl_32b_minimax_h3_int8_convrot.safetensors",
            "qwen3vl_32b_minimax_h3_bf16.safetensors",
        ],
    );
    let found_vvae = find_first(
        "vae",
        &[
            "minimax_h3_video_vae_fp16.safetensors",
            "minimax_h3_video_vae_int8_convrot.safetensors",
        ],
    );
    let found_avae = find_first("vae", &["minimax_h3_audio_vae_fp32.safetensors"]);
    let has_taeh3 = models_dir
        .join("vae_approx")
        .join("taeh3.safetensors")
        .is_file();

    let discovered_loras = scan_h3_loras(&models_dir.join("loras"));
    let has_ref2v_turbo_lora = discovered_loras
        .iter()
        .any(|l| l.recommended_role == "ref2v_turbo");
    let has_fl2v_turbo_lora = discovered_loras
        .iter()
        .any(|l| l.recommended_role == "fl2v_turbo");

    let mut defaults = H3InstalledModelNames::default();
    if let Some(ref name) = found_ref2va {
        defaults.unet_ref2va = name.clone();
    }
    if let Some(ref name) = found_fl2va {
        defaults.unet_fl2va = name.clone();
    }
    if let Some(ref name) = found_qwen {
        defaults.clip_qwen3vl = name.clone();
    }
    if let Some(ref name) = found_vvae {
        defaults.video_vae = name.clone();
    }
    if let Some(ref name) = found_avae {
        defaults.audio_vae = name.clone();
    }

    let ready = found_ref2va.is_some()
        && found_fl2va.is_some()
        && found_qwen.is_some()
        && found_vvae.is_some()
        && found_avae.is_some();

    let detail = if ready {
        if has_ref2v_turbo_lora && has_fl2v_turbo_lora {
            "MiniMax H3 Ref2VA + FL2VA + Qwen3-VL + Video/Audio VAEs + Turbo LoRAs are ready."
                .into()
        } else {
            "MiniMax H3 core weights are ready. Optional H3 Turbo LoRAs can be installed for 10-step 0.2MP / 4-step 1080p passes."
                .into()
        }
    } else {
        let mut missing = Vec::new();
        if found_ref2va.is_none() {
            missing.push("Ref2VA UNET");
        }
        if found_fl2va.is_none() {
            missing.push("FL2VA UNET");
        }
        if found_qwen.is_none() {
            missing.push("Qwen3-VL 32B text encoder");
        }
        if found_vvae.is_none() {
            missing.push("H3 Video VAE");
        }
        if found_avae.is_none() {
            missing.push("H3 Audio VAE");
        }
        format!(
            "Missing MiniMax H3 weights in {}: {}. Open Setup to install Movie Studio.",
            comfy_root,
            missing.join(", ")
        )
    };

    (
        defaults,
        H3EnvironmentSnapshot {
            ready,
            comfy_root: comfy_root.to_string(),
            has_ref2va: found_ref2va.is_some(),
            has_fl2va: found_fl2va.is_some(),
            has_qwen3vl: found_qwen.is_some(),
            has_video_vae: found_vvae.is_some(),
            has_audio_vae: found_avae.is_some(),
            has_taeh3_preview: has_taeh3,
            has_ref2v_turbo_lora,
            has_fl2v_turbo_lora,
            discovered_loras,
            detail,
        },
    )
}

fn scan_h3_loras(loras_root: &Path) -> Vec<H3DiscoveredLora> {
    if !loras_root.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(loras_root)
        .max_depth(2)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .take(512)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("safetensors"))
        {
            continue;
        }
        let Ok(rel) = path.strip_prefix(loras_root) else {
            continue;
        };
        let rel_str = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(std::path::MAIN_SEPARATOR_STR);
        let lower = rel_str.to_ascii_lowercase();
        let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let (role, strength) = if lower.contains("ref2v")
            && (lower.contains("turbo") || lower.contains("4step") || lower.contains("dmad"))
        {
            ("ref2v_turbo", 0.8)
        } else if lower.contains("fl2v")
            && (lower.contains("turbo")
                || lower.contains("8step")
                || lower.contains("4step")
                || lower.contains("dmad"))
        {
            ("fl2v_turbo", 0.8)
        } else if lower.contains("h3") || lower.contains("minimax") || lower.contains("pika_h3") {
            ("style", 0.6)
        } else {
            ("other", 0.5)
        };
        out.push(H3DiscoveredLora {
            file_name: rel_str,
            bytes,
            recommended_role: role.into(),
            default_strength: strength,
        });
    }
    out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    out
}

pub fn build_default_h3_scene(
    id: String,
    name: String,
    kind: H3SceneKind,
    width: Option<u32>,
    height: Option<u32>,
    duration_sec: Option<f64>,
    advanced: bool,
) -> Result<H3Scene, StudioError> {
    let (def_w, def_h) = default_scene_dimensions(kind);
    let w = width.unwrap_or(def_w);
    let h = height.unwrap_or(def_h);
    let (w, h) = validate_canvas_dimensions(w, h, kind == H3SceneKind::Soundtrack, advanced)?;
    let dur = duration_sec
        .filter(|v| v.is_finite())
        .unwrap_or(15.0)
        .clamp(1.0, 600.0);
    let win_out = dur.min(5.0);
    let now = Utc::now().to_rfc3339();

    let mut generation = H3GenerationSettings::default();
    if kind == H3SceneKind::Soundtrack {
        generation.task_mode = H3TaskMode::SoundtrackPass;
        generation.prompt_format = H3PromptFormat::H3Base;
        generation.custom_width = 64;
        generation.custom_height = 32;
    } else {
        generation.custom_width = w;
        generation.custom_height = h;
    }

    Ok(H3Scene {
        id,
        name: name.trim().into(),
        kind,
        width: w,
        height: h,
        fps: H3_FPS,
        duration_sec: dur,
        window: H3WindowConfig {
            in_sec: 0.0,
            out_sec: win_out,
            ..Default::default()
        },
        generation,
        global_prompt: H3GlobalPrompt::default(),
        prompt_sections: Vec::new(),
        video_lanes: vec![H3VideoLane {
            id: "vid-lane-1".into(),
            name: "V1".into(),
            muted: false,
            solo: false,
            locked: false,
            clips: Vec::new(),
        }],
        audio_lanes: vec![
            H3AudioLane {
                id: "aud-lane-1".into(),
                name: "A1 · Dialogue & Foley".into(),
                muted: false,
                solo: false,
                locked: false,
                clips: Vec::new(),
            },
            H3AudioLane {
                id: "aud-lane-2".into(),
                name: "A2 · Score".into(),
                muted: false,
                solo: false,
                locked: false,
                clips: Vec::new(),
            },
        ],
        driver_lanes: vec![H3DriverLane {
            id: "drv-lane-1".into(),
            name: "D1 · Swap / Pose Driver".into(),
            muted: false,
            locked: false,
            clips: Vec::new(),
        }],
        reference_lanes: vec![
            H3ReferenceLane {
                id: "ref-lane-1".into(),
                name: "R1".into(),
                muted: false,
                locked: false,
                items: Vec::new(),
            },
            H3ReferenceLane {
                id: "ref-lane-2".into(),
                name: "R2".into(),
                muted: false,
                locked: false,
                items: Vec::new(),
            },
        ],
        guides: Vec::new(),
        created_at: now.clone(),
        updated_at: now,
    })
}

/// Non-destructively upgrades legacy `v8` projects in memory on load so existing stories, planned
/// clips, and completed masters open directly inside the H3 Studio timeline.
pub(super) fn migrate_legacy_project_in_memory(project_dir: &Path, project: &mut MovieProject) {
    if project.story_markdown.trim().is_empty() {
        let legacy_story_path = project_dir.join("producer").join("story.json");
        if let Ok(bytes) = fs::read(&legacy_story_path) {
            if let Ok(val) = serde_json::from_slice::<Value>(&bytes) {
                if let Some(revs) = val.get("revisions").and_then(Value::as_array) {
                    let active_id = val
                        .get("activeRevisionId")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if let Some(found) = revs
                        .iter()
                        .find(|r| r.get("id").and_then(Value::as_str) == Some(active_id))
                        .or_else(|| revs.last())
                    {
                        if let Some(md) = found.get("markdown").and_then(Value::as_str) {
                            project.story_markdown = md.to_string();
                        }
                    }
                }
            }
        }
        if project.story_markdown.trim().is_empty() && !project.prompt.trim().is_empty() {
            project.story_markdown = project.prompt.clone();
        }
    }

    if !project.scenes.is_empty() {
        if project.active_scene_id.is_empty()
            || !project.scenes.iter().any(|s| s.id == project.active_scene_id)
        {
            project.active_scene_id = project.scenes[0].id.clone();
        }
        return;
    }

    let w = if project.settings.width >= 256 && project.settings.width.is_multiple_of(32) {
        project.settings.width.clamp(256, 1920)
    } else {
        1920
    };
    let h = if project.settings.height >= 256 && project.settings.height.is_multiple_of(32) {
        project.settings.height.clamp(256, 1920)
    } else {
        1088
    };

    if project.entities.is_empty() && !project.references.is_empty() {
        for (r_idx, r) in project.references.iter().enumerate() {
            if r.asset_id.trim().is_empty() {
                continue;
            }
            let raw_handle: String = r
                .name
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(24)
                .collect();
            let handle = if raw_handle.is_empty() || !raw_handle.chars().next().unwrap().is_ascii_alphabetic() {
                format!("Ref{}", r_idx + 1)
            } else {
                raw_handle
            };
            let pop = infer_population_for_kind(&r.kind);
            let kind = match pop {
                kestrel_app_core::H3ReferencePopulation::StandaloneAudios => H3EntityKind::Voice,
                _ => H3EntityKind::Character,
            };
            project.entities.push(H3ReferenceEntity {
                id: format!("legacy-ent-{}", r_idx + 1),
                handle: handle.clone(),
                display_name: if r.name.trim().is_empty() {
                    handle
                } else {
                    r.name.clone()
                },
                kind,
                subject_description: r.description.clone(),
                retention_level: kestrel_app_core::H3RetentionLevel::FullyPreserved,
                default_visual_intent: kestrel_app_core::H3VisualIntent::Preserve,
                default_audio_intent: kestrel_app_core::H3AudioIntent::VoiceIdentity,
                members: vec![kestrel_app_core::H3ReferenceMember {
                    id: format!("legacy-mem-{}", r_idx + 1),
                    asset_id: r.asset_id.clone(),
                    population: pop,
                    role: default_role_for_entity(kind, pop),
                    label: r.name.clone(),
                    description_override: String::new(),
                    use_embedded_audio: r.use_embedded_audio,
                    crop: None,
                    source_start_sec: 0.0,
                    source_end_sec: 0.0,
                }],
                created_at: project.created_at.clone(),
                updated_at: project.updated_at.clone(),
            });
        }
    }

    let mut cursor_sec = 0.0_f64;
    let mut sections = Vec::new();
    if let Some(plan) = &project.plan {
        for (idx, clip) in plan.clips.iter().enumerate() {
            let raw_dur = clip.duration_seconds as f64;
            let dur = if raw_dur.is_finite() {
                raw_dur.clamp(2.0, 15.0)
            } else {
                5.0
            };
            sections.push(H3PromptSection {
                id: if clip.id.is_empty() {
                    format!("legacy-sec-{}", idx + 1)
                } else {
                    clip.id.clone()
                },
                label: clip.title.clone(),
                timeline_start_sec: cursor_sec,
                duration_sec: dur,
                summary_beat: clip.purpose.clone(),
                detailed_action: clip.prompt.clone(),
                camera_and_lighting: String::new(),
                soundscape_override: String::new(),
                music_override: String::new(),
                vocal_events: Vec::new(),
                swap_fields: None,
                enabled: true,
            });
            cursor_sec += dur;
        }
    }

    let mut v1_clips = Vec::new();
    let mut v_cursor = 0.0_f64;
    for clip in &project.clips {
        if clip.status == "complete" && !clip.path.is_empty() && Path::new(&clip.path).is_file() {
            let raw_dur = clip.duration_seconds as f64;
            let dur = if raw_dur.is_finite() {
                raw_dur.max(1.0)
            } else {
                5.0
            };
            let job_id = format!("legacy-take-{}", clip.id);
            let geom = calculate_take_placement(
                &H3WindowConfig {
                    in_sec: v_cursor,
                    out_sec: v_cursor + dur.min(15.0),
                    ..Default::default()
                },
                H3_FPS,
            )
            .unwrap_or_else(|_| {
                calculate_take_placement(&H3WindowConfig::default(), H3_FPS).unwrap()
            });
            if !project.render_jobs.iter().any(|j| j.id == job_id) {
                project.render_jobs.push(H3RenderQueueJob {
                    id: job_id.clone(),
                    project_id: project.id.clone(),
                    scene_id: "scene-main".into(),
                    scene_name: "Main Scene".into(),
                    label: clip.title.clone(),
                    status: H3RenderJobStatus::Complete,
                    pass_preset: H3PassPreset::Custom,
                    task_mode: H3TaskMode::Standard,
                    placement_mode: H3TakePlacementMode::Trimmed,
                    width: project.settings.width,
                    height: project.settings.height,
                    steps: project.settings.steps,
                    denoise: 1.0,
                    seed: clip.seed,
                    compiled_prompt: clip.prompt.clone(),
                    resolved_model: "minimax_h3".into(),
                    geometry: geom,
                    output_clip_id: Some(clip.id.clone()),
                    take_media_path: clip.path.clone(),
                    take_sha256: String::new(),
                    color_drift: None,
                    quality: None,
                    error: String::new(),
                    created_at: project.created_at.clone(),
                    updated_at: project.updated_at.clone(),
                });
            }
            v1_clips.push(H3VideoClip {
                id: format!("vclip-{}", clip.id),
                source_kind: H3ClipSourceKind::Take,
                source_id: job_id,
                label: clip.title.clone(),
                timeline_start_sec: v_cursor,
                duration_sec: dur,
                source_in_sec: 0.0,
                source_duration_sec: dur,
                opacity: 1.0,
                audio_gain: 1.0,
                include_embedded_audio: true,
                enabled: true,
            });
            v_cursor += dur;
        }
    }

    let scene_dur = cursor_sec.max(v_cursor).max(15.0);
    if let Ok(mut scene) = build_default_h3_scene(
        "scene-main".into(),
        "Main Scene".into(),
        H3SceneKind::Standard,
        Some(w),
        Some(h),
        Some(scene_dur),
        true,
    ) {
        scene.prompt_sections = sections;
        if let Some(v1) = scene.video_lanes.first_mut() {
            v1.clips = v1_clips;
        }
        project.active_scene_id = scene.id.clone();
        project.scenes.push(scene);
    }
}

impl MovieStudio {
    pub fn environment_snapshot(&self, comfy_root: &str) -> H3EnvironmentSnapshot {
        resolve_installed_h3_models(comfy_root).1
    }

    /// Explicitly downloads the pinned Comfy-Org MiniMax H3 Ref2V & FL2V Turbo LoRAs into
    /// `<comfy_root>/models/loras/` with byte-range resume and SHA-256 verification.
    pub async fn install_turbo_loras(
        &self,
        comfy_root: &str,
        cancel: &CancellationToken,
    ) -> Result<H3EnvironmentSnapshot, StudioError> {
        let root = Path::new(comfy_root);
        if !root.is_absolute() || !root.is_dir() {
            return Err(StudioError::Invalid(
                "Choose a valid local ComfyUI folder in Setup before installing H3 Turbo LoRAs."
                    .into(),
            ));
        }
        let loras_dir = root.join("models").join("loras");
        fs::create_dir_all(&loras_dir)?;

        for item in PINNED_TURBO_LORAS {
            check_cancel(cancel)?;
            let target = loras_dir.join(item.file_name);
            if target.is_file() && target.metadata()?.len() == item.bytes {
                continue;
            }
            let part = loras_dir.join(format!("{}.part", item.file_name));
            let existing_len = part.metadata().map(|m| m.len()).unwrap_or(0);
            let url = format!(
                "https://huggingface.co/Comfy-Org/MiniMax-H3/resolve/{H3_LORA_REPO_REV}/{}",
                item.remote_path
            );

            let mut req = self.http.get(&url);
            if existing_len > 0 && existing_len < item.bytes {
                req = req.header("Range", format!("bytes={existing_len}-"));
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                return Err(StudioError::Render(format!(
                    "Failed to download {}: HTTP {}",
                    item.file_name,
                    resp.status()
                )));
            }
            let append = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT && existing_len > 0;
            let mut file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(!append)
                .open(&part)?;
            if append {
                file.seek(SeekFrom::End(0))?;
            }
            let mut stream = resp.bytes_stream();
            while let Some(chunk_res) = stream.next().await {
                check_cancel(cancel)?;
                let chunk = chunk_res?;
                file.write_all(&chunk)?;
            }
            file.sync_all()?;
            drop(file);

            let (actual_bytes, actual_sha) = hash_file(&part)?;
            if actual_bytes != item.bytes || actual_sha != item.sha256 {
                let _ = fs::remove_file(&part);
                return Err(StudioError::Render(format!(
                    "{} failed SHA-256 verification (got {actual_sha}, expected {}).",
                    item.file_name, item.sha256
                )));
            }
            if target.exists() {
                let _ = fs::remove_file(&target);
            }
            fs::rename(&part, &target)?;
        }

        Ok(self.environment_snapshot(comfy_root))
    }

    pub fn list_reference_assets(&self) -> Result<Vec<MovieReferenceAsset>, StudioError> {
        let meta_dir = self.root.join("_references").join("meta");
        fs::create_dir_all(&meta_dir)?;
        let mut out = Vec::new();
        for entry in fs::read_dir(&meta_dir)?.filter_map(Result::ok).take(2_000) {
            let path = entry.path();
            if !path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Ok(asset) = self.resolve_reference_asset(stem) {
                out.push(asset);
            }
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(out)
    }

    pub fn production_digest(
        &self,
        project_id: &str,
        comfy_root: &str,
        advanced: bool,
    ) -> Result<H3ProductionDigest, StudioError> {
        let project = self.get(project_id)?;
        let env = self.environment_snapshot(comfy_root);
        let asset_count = self
            .list_reference_assets()
            .map(|a| a.len())
            .unwrap_or(project.references.len());
        Ok(build_production_digest(
            &project,
            asset_count,
            &env,
            advanced,
            |aid| self.resolve_reference_asset(aid),
        ))
    }

    pub fn create_h3_project(
        &self,
        request: CreateMovieH3ProjectRequest,
        comfy_root: &str,
        advanced: bool,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let title = request.title.trim();
        if title.is_empty() || title.chars().count() > 140 || title.chars().any(char::is_control) {
            return Err(StudioError::Invalid(
                "Project title must contain 1 to 140 printable characters.".into(),
            ));
        }
        if request.story_markdown.len() > MAX_STORY_MARKDOWN_BYTES {
            return Err(StudioError::Invalid(
                "Story markdown cannot exceed 256 KiB.".into(),
            ));
        }
        let scene_id = format!("scene-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let main_scene = build_default_h3_scene(
            scene_id.clone(),
            "Scene 1 · Main".into(),
            H3SceneKind::Standard,
            request.width,
            request.height,
            request.duration_sec,
            advanced,
        )?;

        let id = uuid::Uuid::new_v4().to_string();
        let folder = self.project_dir(&id);
        fs::create_dir_all(folder.join("takes"))?;
        fs::create_dir_all(folder.join("exports"))?;
        fs::create_dir_all(folder.join("stills"))?;
        fs::create_dir_all(folder.join("references"))?;

        let now = Utc::now().to_rfc3339();
        let story_md = request.story_markdown.trim().to_string();
        let mut story_revisions = Vec::new();
        if !story_md.is_empty() {
            story_revisions.push(MovieStoryRevision {
                id: format!("rev-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
                number: 1,
                parent_revision_id: None,
                created_at: now.clone(),
                origin: MovieStoryRevisionOrigin::Producer,
                instruction: "Initial production brief".into(),
                markdown: story_md.clone(),
                accepted_at: Some(now.clone()),
            });
        }

        let settings = MovieSettings {
            width: main_scene.width.min(1344),
            height: main_scene.height.min(768),
            comfy_root: comfy_root.into(),
            ref_image_size: "max".into(),
            ..MovieSettings::default()
        };

        let mut project = MovieProject {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            prompt: if story_md.is_empty() {
                title.to_string()
            } else {
                story_md.clone()
            },
            title: title.to_string(),
            status: "ready".into(),
            phase: "authoring".into(),
            detail: "MiniMax H3 Studio workspace ready. Build @Handle references, stage shots, and render windows.".into(),
            created_at: now.clone(),
            updated_at: now,
            model: "MiniMax H3 Studio".into(),
            legacy_model_roles: Value::Null,
            renderer: "MiniMax H3 / ComfyUI native".into(),
            settings,
            references: Vec::new(),
            plan: None,
            sources: Vec::new(),
            clips: Vec::new(),
            edit: MovieEdit {
                clips: Vec::new(),
                export_title: title.to_string(),
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
            story_markdown: story_md,
            story_revisions,
            entities: Vec::new(),
            scenes: vec![main_scene],
            active_scene_id: scene_id,
            render_jobs: Vec::new(),
            legacy_producer_feedback: Vec::new(),
            legacy_copilot_history: Vec::new(),
        };

        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub async fn save_h3_story(
        &self,
        request: SaveMovieH3StoryRequest,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        if request.story_markdown.len() > MAX_STORY_MARKDOWN_BYTES {
            return Err(StudioError::Invalid(
                "Story markdown cannot exceed 256 KiB.".into(),
            ));
        }
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;
        if let Some(title) = request.title.as_deref() {
            let trimmed = title.trim();
            if !trimmed.is_empty() && trimmed.chars().count() <= 140 && !trimmed.chars().any(char::is_control) {
                project.title = trimmed.into();
                project.edit.export_title = trimmed.into();
            }
        }
        let md = request.story_markdown.trim().to_string();
        project.story_markdown = md.clone();
        if !md.is_empty() {
            project.prompt = md.clone();
            let next_number = project
                .story_revisions
                .last()
                .map(|rev| rev.number.saturating_add(1))
                .unwrap_or(1);
            let parent_id = project.story_revisions.last().map(|rev| rev.id.clone());
            let now_str = Utc::now().to_rfc3339();
            project.story_revisions.push(MovieStoryRevision {
                id: format!("rev-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
                number: next_number,
                parent_revision_id: parent_id,
                created_at: now_str.clone(),
                origin: MovieStoryRevisionOrigin::Producer,
                instruction: if request.instruction.trim().is_empty() {
                    "Updated story brief".into()
                } else {
                    truncate(request.instruction.trim(), 2_000)
                },
                markdown: md,
                accepted_at: Some(now_str),
            });
            if project.story_revisions.len() > 64 {
                let drain = project.story_revisions.len() - 64;
                project.story_revisions.drain(0..drain);
            }
        }
        project.detail = "Saved story brief.".into();
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub async fn upsert_h3_entity(
        &self,
        request: UpsertMovieH3EntityRequest,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;
        let mut entity = request.entity;
        entity.handle = normalize_handle(&entity.handle)?;
        if entity.id.trim().is_empty() {
            entity.id = format!("ent-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        } else {
            validate_id_like(&entity.id, "entity id")?;
        }
        if entity.display_name.trim().is_empty() {
            entity.display_name = entity.handle.clone();
        } else {
            entity.display_name = truncate(entity.display_name.trim(), 120);
        }
        if entity.subject_description.len() > 8_000 {
            return Err(StudioError::Invalid(
                "Entity subjectDescription cannot exceed 8,000 bytes.".into(),
            ));
        }

        // Prevent handle collisions across distinct entity IDs!
        if let Some(conflict) = project.entities.iter().find(|existing| {
            existing.id != entity.id && existing.handle.eq_ignore_ascii_case(&entity.handle)
        }) {
            return Err(StudioError::Invalid(format!(
                "Handle @{} is already used by entity {} ({}). Update that entity or choose another handle.",
                entity.handle, conflict.display_name, conflict.id
            )));
        }

        if entity.members.len() > 32 {
            return Err(StudioError::Invalid(
                "An @Handle entity can contain at most 32 reference members.".into(),
            ));
        }
        let mut member_ids = HashSet::new();
        for (idx, member) in entity.members.iter_mut().enumerate() {
            if member.id.trim().is_empty() || !member_ids.insert(member.id.clone()) {
                member.id = format!("mem-{}-{:02}", &entity.id, idx + 1);
                member_ids.insert(member.id.clone());
            }
            let asset = self.resolve_reference_asset(&member.asset_id)?;
            member.population = infer_population_for_kind(&asset.kind);
            if member.label.trim().is_empty() {
                member.label = asset.name.clone();
            } else {
                member.label = truncate(member.label.trim(), 120);
            }
            if let Some(crop) = &member.crop {
                member.crop = Some(validate_normalized_crop(crop, &entity.handle)?);
            }
            if member.source_start_sec < 0.0 || !member.source_start_sec.is_finite() {
                member.source_start_sec = 0.0;
            }
            if member.source_end_sec < 0.0 || !member.source_end_sec.is_finite() {
                member.source_end_sec = 0.0;
            }
            member.role = super::h3_references::normalize_member_role(
                entity.kind,
                member.population,
                member.role,
            );
            self.ensure_project_reference_copy(&mut project, &asset, &entity.subject_description)?;
        }

        let now = Utc::now().to_rfc3339();
        entity.updated_at = now.clone();
        if let Some(existing) = project.entities.iter_mut().find(|e| e.id == entity.id) {
            if entity.created_at.is_empty() {
                entity.created_at = existing.created_at.clone();
            }
            *existing = entity.clone();
        } else {
            if project.entities.len() >= MAX_ENTITIES_PER_PROJECT {
                return Err(StudioError::Invalid(format!(
                    "A project can define at most {MAX_ENTITIES_PER_PROJECT} @Handle entities."
                )));
            }
            entity.created_at = now;
            project.entities.push(entity.clone());
        }
        project.detail = format!("Saved @{} in the Reference Library.", entity.handle);
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub async fn delete_h3_entity(
        &self,
        request: DeleteMovieH3EntityRequest,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;
        let before = project.entities.len();
        project.entities.retain(|e| e.id != request.entity_id);
        if project.entities.len() == before {
            return Err(StudioError::NotFound(format!(
                "Entity {} not found.",
                request.entity_id
            )));
        }
        for scene in &mut project.scenes {
            for lane in &mut scene.reference_lanes {
                lane.items
                    .retain(|item| item.entity_id.as_deref() != Some(request.entity_id.as_str()));
            }
        }
        project.detail = "Removed entity from Reference Library.".into();
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub async fn create_h3_scene(
        &self,
        request: CreateMovieH3SceneRequest,
        advanced: bool,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let name = request.name.trim();
        if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
            return Err(StudioError::Invalid(
                "Scene name must contain 1 to 100 printable characters.".into(),
            ));
        }
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;
        if project.scenes.len() >= MAX_SCENES_PER_PROJECT {
            return Err(StudioError::Invalid(format!(
                "A project can contain at most {MAX_SCENES_PER_PROJECT} scenes."
            )));
        }
        let scene_id = format!("scene-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        let scene = build_default_h3_scene(
            scene_id.clone(),
            name.to_string(),
            request.kind,
            request.width,
            request.height,
            request.duration_sec,
            advanced,
        )?;
        project.scenes.push(scene);
        project.active_scene_id = scene_id;
        project.detail = format!("Created scene '{name}'.");
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub async fn save_h3_scene(
        &self,
        request: SaveMovieH3SceneRequest,
        advanced: bool,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;
        let mut scene = request.scene;

        // If a digest-driven caller (such as the 64k Producer Agent) updates `window` or `generation`
        // without passing full lane/shot arrays, preserve the existing scene's collections rather than
        // wiping them with `#[serde(default)]` empty vectors!
        if let Some(existing) = project.scenes.iter().find(|s| s.id == scene.id) {
            if scene.name.trim().is_empty() {
                scene.name = existing.name.clone();
            }
            if scene.video_lanes.is_empty() {
                scene.video_lanes = existing.video_lanes.clone();
            }
            if scene.audio_lanes.is_empty() {
                scene.audio_lanes = existing.audio_lanes.clone();
            }
            if scene.driver_lanes.is_empty() {
                scene.driver_lanes = existing.driver_lanes.clone();
            }
            if scene.reference_lanes.is_empty() {
                scene.reference_lanes = existing.reference_lanes.clone();
            }
            if !request.replace_empty_collections {
                if scene.prompt_sections.is_empty() && !existing.prompt_sections.is_empty() {
                    scene.prompt_sections = existing.prompt_sections.clone();
                }
                if scene.guides.is_empty() && !existing.guides.is_empty() {
                    scene.guides = existing.guides.clone();
                }
                if scene.global_prompt == H3GlobalPrompt::default()
                    && existing.global_prompt != H3GlobalPrompt::default()
                {
                    scene.global_prompt = existing.global_prompt.clone();
                }
            }
        }

        validate_h3_scene(&mut scene, advanced)?;
        scene.updated_at = Utc::now().to_rfc3339();

        if let Some(existing) = project.scenes.iter_mut().find(|s| s.id == scene.id) {
            if scene.created_at.is_empty() {
                scene.created_at = existing.created_at.clone();
            }
            *existing = scene.clone();
        } else {
            if project.scenes.len() >= MAX_SCENES_PER_PROJECT {
                return Err(StudioError::Invalid(format!(
                    "A project can contain at most {MAX_SCENES_PER_PROJECT} scenes."
                )));
            }
            if scene.created_at.is_empty() {
                scene.created_at = scene.updated_at.clone();
            }
            project.scenes.push(scene.clone());
        }
        if request.set_active {
            project.active_scene_id = scene.id.clone();
        }
        project.detail = format!("Saved scene '{}'.", scene.name);
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub async fn stage_h3_reference(
        &self,
        request: StageMovieH3ReferenceRequest,
        app: Option<&AppHandle>,
    ) -> Result<H3StageReferenceReceipt, StudioError> {
        if !request.start_sec.is_finite()
            || !request.end_sec.is_finite()
            || request.start_sec < 0.0
            || request.end_sec <= request.start_sec + 0.1
        {
            return Err(StudioError::Invalid(
                "Staged reference span requires startSec >= 0 and endSec > startSec + 0.1s.".into(),
            ));
        }
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;

        let label = if let Some(eid) = request.entity_id.as_deref().filter(|s| !s.is_empty()) {
            let ent = project
                .entities
                .iter()
                .find(|e| e.id == eid || e.handle.eq_ignore_ascii_case(eid.trim_start_matches('@')))
                .ok_or_else(|| StudioError::NotFound(format!("Entity {eid} not found.")))?;
            format!("@{}", ent.handle)
        } else if let Some(aid) = request.asset_id.as_deref().filter(|s| !s.is_empty()) {
            let asset = self.resolve_reference_asset(aid)?;
            self.ensure_project_reference_copy(&mut project, &asset, &asset.name)?;
            asset.name
        } else {
            return Err(StudioError::Invalid(
                "Provide either entityId or assetId when staging a reference.".into(),
            ));
        };

        // Also allow producer agents to pass `@Handle` in `entity_id` directly for convenience!
        let resolved_entity_id = request
            .entity_id
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(|eid| {
                project
                    .entities
                    .iter()
                    .find(|e| {
                        e.id == eid || e.handle.eq_ignore_ascii_case(eid.trim_start_matches('@'))
                    })
                    .map(|e| e.id.clone())
            });

        let scene = project
            .scenes
            .iter_mut()
            .find(|s| s.id == request.scene_id)
            .ok_or_else(|| StudioError::NotFound(format!("Scene {} not found.", request.scene_id)))?;

        let item_id = format!("sref-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
        let item = H3StagedReferenceItem {
            id: item_id.clone(),
            entity_id: resolved_entity_id,
            member_id: request.member_id.filter(|s| !s.trim().is_empty()),
            asset_id: request.asset_id.filter(|s| !s.trim().is_empty()),
            label: label.clone(),
            role_override: request.role_override,
            retention_override: None,
            description_override: String::new(),
            timeline_start_sec: request.start_sec,
            duration_sec: request.end_sec - request.start_sec,
            use_visual: request.use_visual,
            use_audio: request.use_audio,
            enabled: true,
        };
        if request.end_sec > scene.duration_sec {
            scene.duration_sec = request.end_sec;
        }
        let (lane_id, lane_name) =
            auto_stage_reference(scene, item, request.lane_id.as_deref())?;
        scene.updated_at = Utc::now().to_rfc3339();
        let scene_id = scene.id.clone();
        project.detail = format!(
            "Staged {label} on {lane_name} [{:.2}s..{:.2}s].",
            request.start_sec, request.end_sec
        );
        self.persist_emit(&mut project, app)?;

        Ok(H3StageReferenceReceipt {
            project_id: project.id,
            scene_id,
            lane_id,
            lane_name,
            item_id,
            label,
            start_sec: request.start_sec,
            end_sec: request.end_sec,
        })
    }

    pub async fn upsert_h3_prompt_section(
        &self,
        request: UpsertMovieH3PromptSectionRequest,
        app: Option<&AppHandle>,
    ) -> Result<H3UpsertPromptSectionReceipt, StudioError> {
        let mut sec = request.section;
        if !sec.timeline_start_sec.is_finite()
            || !sec.duration_sec.is_finite()
            || sec.timeline_start_sec < 0.0
            || sec.duration_sec <= 0.1
        {
            return Err(StudioError::Invalid(
                "Prompt section requires timelineStartSec >= 0 and durationSec > 0.1s.".into(),
            ));
        }
        if sec.id.trim().is_empty() {
            sec.id = format!("shot-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        }
        for (vi, vocal) in sec.vocal_events.iter_mut().enumerate() {
            if vocal.id.trim().is_empty() {
                vocal.id = format!("{}-voc-{}", sec.id, vi + 1);
            }
            if vocal.language.trim().is_empty() {
                vocal.language = "English".into();
            }
        }

        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;
        let scene = project
            .scenes
            .iter_mut()
            .find(|s| s.id == request.scene_id)
            .ok_or_else(|| StudioError::NotFound(format!("Scene {} not found.", request.scene_id)))?;

        if let Some(gp) = request.global_prompt {
            scene.global_prompt = gp;
        }
        let end_sec = sec.timeline_start_sec + sec.duration_sec;
        if end_sec > scene.duration_sec {
            scene.duration_sec = end_sec;
        }
        let section_id = sec.id.clone();
        if let Some(existing) = scene.prompt_sections.iter_mut().find(|s| s.id == sec.id) {
            *existing = sec;
        } else {
            if scene.prompt_sections.len() >= MAX_SECTIONS_PER_SCENE {
                return Err(StudioError::Invalid(format!(
                    "A scene can contain at most {MAX_SECTIONS_PER_SCENE} prompt sections."
                )));
            }
            scene.prompt_sections.push(sec);
        }
        scene.prompt_sections.sort_by(|a, b| {
            a.timeline_start_sec
                .partial_cmp(&b.timeline_start_sec)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let shot_count = scene.prompt_sections.len();
        let scene_id = scene.id.clone();
        scene.updated_at = Utc::now().to_rfc3339();
        project.detail = format!("Saved prompt section ({shot_count} shots in scene).");
        self.persist_emit(&mut project, app)?;

        Ok(H3UpsertPromptSectionReceipt {
            project_id: project.id,
            scene_id,
            section_id,
            shot_count,
        })
    }

    pub async fn capture_h3_frame_guide(
        &self,
        request: CaptureMovieH3FrameRequest,
        app: Option<&AppHandle>,
    ) -> Result<MovieReferenceAsset, StudioError> {
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;

        let (source_path, default_label) = if let Some(jid) =
            request.source_job_id.as_deref().filter(|s| !s.is_empty())
        {
            let job = project
                .render_jobs
                .iter()
                .find(|j| j.id == jid)
                .ok_or_else(|| StudioError::NotFound(format!("Take job {jid} not found.")))?;
            (
                PathBuf::from(&job.take_media_path),
                format!("{} @ {}", job.label, format_h3_timestamp(request.time_sec)),
            )
        } else if let Some(aid) = request.source_asset_id.as_deref().filter(|s| !s.is_empty()) {
            let asset = self.resolve_reference_asset(aid)?;
            (
                PathBuf::from(&asset.path),
                format!("{} @ {}", asset.name, format_h3_timestamp(request.time_sec)),
            )
        } else {
            return Err(StudioError::Invalid(
                "Provide sourceJobId or sourceAssetId to capture a frame.".into(),
            ));
        };

        if !source_path.is_file() {
            return Err(StudioError::Render(format!(
                "Source media {} is missing on disk.",
                source_path.display()
            )));
        }

        let stills = self.project_dir(&project.id).join("stills");
        fs::create_dir_all(&stills)?;
        let tmp_png = stills.join(format!("capture-{}.png", uuid::Uuid::new_v4()));
        capture_frame_png(&source_path, request.time_sec, &tmp_png).await?;
        let chosen_label = if request.label.trim().is_empty() {
            default_label
        } else {
            truncate(request.label.trim(), 120)
        };
        let asset = self.import_reference_path_named(&tmp_png, Some(&chosen_label))?;
        let _ = fs::remove_file(&tmp_png);
        self.ensure_project_reference_copy(&mut project, &asset, &chosen_label)?;

        if let Some(guide_sec) = request.add_guide_at_sec {
            let scene = project
                .scenes
                .iter_mut()
                .find(|s| s.id == request.scene_id)
                .ok_or_else(|| {
                    StudioError::NotFound(format!("Scene {} not found.", request.scene_id))
                })?;
            scene.guides.push(H3GuideItem {
                id: format!("guide-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
                asset_id: asset.id.clone(),
                label: chosen_label.clone(),
                timeline_sec: guide_sec.max(0.0),
                end_sec: None,
                use_image: true,
                use_audio: false,
                enabled: true,
            });
            scene.updated_at = Utc::now().to_rfc3339();
            project.detail = format!("Captured guide '{}' at {:.2}s.", chosen_label, guide_sec);
            self.persist_emit(&mut project, app)?;
        }

        Ok(asset)
    }

    pub fn compile_h3_window_preview(
        &self,
        request: &CompileMovieH3WindowRequest,
        comfy_root: &str,
        advanced: bool,
    ) -> Result<H3WindowCompileResult, StudioError> {
        let project = self.get(&request.project_id)?;
        let scene = project
            .scenes
            .iter()
            .find(|s| s.id == request.scene_id)
            .ok_or_else(|| StudioError::NotFound(format!("Scene {} not found.", request.scene_id)))?;
        let env = self.environment_snapshot(comfy_root);
        let mut res = compile_h3_window(
            &project,
            scene,
            request.window_override.as_ref(),
            request.generation_override.as_ref(),
            &env,
            advanced,
            |aid| self.resolve_reference_asset(aid),
        )?;
        if res.verdicts.len() > 24 {
            res.verdicts
                .retain(|v| v.verdict != kestrel_app_core::H3ReferenceVerdictKind::Outside);
            res.verdicts.truncate(24);
        }
        Ok(res)
    }

    /// Validates and enqueues one or more H3 window render jobs (optionally expanding each window
    /// into Sonder's 3-pass `0.2MP -> 1MP -> 1080p` quality ladder).
    pub async fn prepare_h3_generation(
        &self,
        request: H3StartGenerationRequest,
        comfy_root: &str,
        advanced: bool,
        app: Option<&AppHandle>,
    ) -> Result<H3GenerationStartReceipt, StudioError> {
        if !Path::new(comfy_root).is_absolute() {
            return Err(StudioError::Invalid(
                "Choose an absolute ComfyUI folder in Setup before rendering.".into(),
            ));
        }
        let (_, env) = resolve_installed_h3_models(comfy_root);
        if !env.ready {
            return Err(StudioError::Invalid(env.detail));
        }

        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;
        project.settings.comfy_root = comfy_root.to_string();

        let requested_windows: Vec<H3QueuedWindowSpec> = if request.windows.is_empty() {
            let sid = if !project.active_scene_id.is_empty() {
                project.active_scene_id.clone()
            } else {
                project
                    .scenes
                    .first()
                    .map(|s| s.id.clone())
                    .ok_or_else(|| StudioError::Invalid("Project has no scenes.".into()))?
            };
            vec![H3QueuedWindowSpec {
                scene_id: sid,
                label: String::new(),
                window_override: None,
                generation_override: None,
            }]
        } else {
            request.windows
        };

        if requested_windows.len() > MAX_QUEUED_WINDOWS_PER_CALL {
            return Err(StudioError::Invalid(format!(
                "Queue at most {MAX_QUEUED_WINDOWS_PER_CALL} windows per generation batch."
            )));
        }

        let now = Utc::now().to_rfc3339();
        let mut job_ids = Vec::new();
        let mut base_seed = Utc::now().timestamp_nanos_opt().unwrap_or(1) as u64;

        for (w_idx, spec) in requested_windows.iter().enumerate() {
            let scene = project
                .scenes
                .iter()
                .find(|s| s.id == spec.scene_id)
                .ok_or_else(|| {
                    StudioError::NotFound(format!("Scene {} not found.", spec.scene_id))
                })?
                .clone();

            let win_cfg = spec
                .window_override
                .clone()
                .unwrap_or_else(|| scene.window.clone());
            let base_gen = spec
                .generation_override
                .clone()
                .unwrap_or_else(|| scene.generation.clone());

            let passes: Vec<H3GenerationSettings> = if request.expand_quality_ladder
                && !matches!(
                    base_gen.task_mode,
                    H3TaskMode::SoundtrackPass | H3TaskMode::AudioOverExistingVideo
                )
                && scene.kind != H3SceneKind::Soundtrack
            {
                let mut p1 = base_gen.clone();
                p1.pass_preset = H3PassPreset::Draft02Mp;
                p1.denoise = 1.0;
                p1.placement_mode = H3TakePlacementMode::Trimmed;

                let mut p2 = base_gen.clone();
                p2.pass_preset = H3PassPreset::Detail1Mp;
                p2.denoise = 0.75;
                p2.placement_mode = H3TakePlacementMode::Trimmed;
                if matches!(
                    base_gen.task_mode,
                    H3TaskMode::CharacterSwap | H3TaskMode::HeadSwap
                ) {
                    p2.prompt_format = if base_gen.task_mode == H3TaskMode::HeadSwap {
                        H3PromptFormat::HeadSwap
                    } else {
                        H3PromptFormat::CharacterSwap
                    };
                    p2.task_mode = H3TaskMode::Standard;
                }

                let mut p3 = base_gen;
                p3.pass_preset = H3PassPreset::Upscale1080p;
                p3.denoise = 0.40;
                p3.placement_mode = H3TakePlacementMode::Trimmed;
                if p2.task_mode == H3TaskMode::Standard
                    && matches!(
                        p2.prompt_format,
                        H3PromptFormat::CharacterSwap | H3PromptFormat::HeadSwap
                    )
                {
                    p3.prompt_format = p2.prompt_format;
                    p3.task_mode = H3TaskMode::Standard;
                }
                vec![p1, p2, p3]
            } else {
                vec![base_gen]
            };

            let shared_seed = if !passes[0].randomize_seed && passes[0].seed > 0 {
                passes[0].seed
            } else {
                base_seed = base_seed
                    .wrapping_add(((w_idx + 1) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
                    & (i64::MAX as u64);
                base_seed.max(1)
            };

            for (p_idx, pass_gen) in passes.into_iter().enumerate() {
                let compiled = compile_h3_window(
                    &project,
                    &scene,
                    Some(&win_cfg),
                    Some(&pass_gen),
                    &env,
                    advanced,
                    |aid| self.resolve_reference_asset(aid),
                )?;

                // Reject blocking compile errors before touching GPU VRAM!
                if let Some(blocking) = compiled
                    .advisories
                    .iter()
                    .find(|a| a.severity == H3AdvisorySeverity::Error)
                {
                    return Err(StudioError::Invalid(blocking.message.clone()));
                }

                let job_id = format!("h3job-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
                let pass_tag = match pass_gen.pass_preset {
                    H3PassPreset::Draft02Mp => "0.2MP Draft",
                    H3PassPreset::Detail1Mp => "1MP Detail",
                    H3PassPreset::Upscale1080p => "1080p Upscale",
                    H3PassPreset::Custom => "Custom",
                };
                let label = if !spec.label.trim().is_empty() {
                    if request.expand_quality_ladder {
                        format!("{} · {pass_tag}", spec.label.trim())
                    } else {
                        spec.label.trim().to_string()
                    }
                } else {
                    format!(
                        "{} [{:.1}s–{:.1}s] · {pass_tag}",
                        scene.name, win_cfg.in_sec, win_cfg.out_sec
                    )
                };

                let take_dir = self.project_dir(&project.id).join("takes").join(&job_id);
                fs::create_dir_all(&take_dir)?;
                write_json_atomic(
                    &take_dir.join("queued-spec.json"),
                    &json!({
                        "window": win_cfg,
                        "generation": pass_gen,
                        "passIndex": p_idx,
                    }),
                )?;

                let queued_denoise = if request.expand_quality_ladder && p_idx > 0 {
                    pass_gen.denoise.clamp(0.05, 1.0)
                } else {
                    compiled.resolved_denoise
                };
                let seed = shared_seed.wrapping_add(p_idx as u64) & (i64::MAX as u64);
                project.render_jobs.push(H3RenderQueueJob {
                    id: job_id.clone(),
                    project_id: project.id.clone(),
                    scene_id: scene.id.clone(),
                    scene_name: scene.name.clone(),
                    label,
                    status: H3RenderJobStatus::Queued,
                    pass_preset: pass_gen.pass_preset,
                    task_mode: pass_gen.task_mode,
                    placement_mode: pass_gen.placement_mode,
                    width: compiled.resolved_width,
                    height: compiled.resolved_height,
                    steps: compiled.resolved_steps,
                    denoise: queued_denoise,
                    seed,
                    compiled_prompt: compiled.compiled_prompt,
                    resolved_model: compiled.resolved_model,
                    geometry: compiled.geometry,
                    output_clip_id: None,
                    take_media_path: String::new(),
                    take_sha256: String::new(),
                    color_drift: None,
                    quality: None,
                    error: String::new(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                });
                job_ids.push(job_id);
            }
        }

        project.status = "running".into();
        project.phase = "queued".into();
        project.error.clear();
        project.detail = format!("Queued {} MiniMax H3 render pass(es).", job_ids.len());
        self.persist_emit(&mut project, app)?;

        Ok(H3GenerationStartReceipt {
            project_id: project.id,
            queued_count: job_ids.len(),
            detail: project.detail,
            job_ids,
        })
    }

    /// Executes all `Queued` `H3RenderQueueJob`s in `project_id` sequentially in a single ComfyUI
    /// session so Pass 2 (`Detail1Mp`) and Pass 3 (`Upscale1080p`) automatically refine Pass 1!
    pub async fn run_h3_generation_queue(
        &self,
        project_id: &str,
        comfy_root: &str,
        advanced: bool,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
        mut on_progress: impl FnMut(String),
    ) -> Result<MovieProject, StudioError> {
        let mut project = self.get(project_id)?;
        project.settings.comfy_root = comfy_root.to_string();
        self.live_previews.clear_movie(project_id);

        on_progress("Starting local ComfyUI MiniMax H3 service...".into());
        self.ensure_comfy(comfy_root, &mut project, app).await?;
        let (installed_models, env) = resolve_installed_h3_models(comfy_root);
        let preview_available = self.comfy_preview_available().await;

        let queued_ids: Vec<String> = project
            .render_jobs
            .iter()
            .filter(|j| j.status == H3RenderJobStatus::Queued)
            .map(|j| j.id.clone())
            .collect();
        let total_jobs = queued_ids.len();

        for (idx, job_id) in queued_ids.iter().enumerate() {
            check_cancel(cancel)?;
            project = self.get(project_id)?;

            let Some(job_snapshot) = project.render_jobs.iter().find(|j| j.id == *job_id).cloned()
            else {
                continue;
            };
            let take_dir = self.project_dir(project_id).join("takes").join(job_id);
            fs::create_dir_all(&take_dir)?;

            // Load the exact window & generation settings saved when the job was queued, and
            // RE-COMPILE against the CURRENT scene state so Pass 2/3 pick up Pass 1's placed clip!
            let spec_val: Value = fs::read(take_dir.join("queued-spec.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_else(|| json!({}));
            let win_cfg: Option<H3WindowConfig> = spec_val
                .get("window")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            let gen_cfg: Option<H3GenerationSettings> = spec_val
                .get("generation")
                .and_then(|v| serde_json::from_value(v.clone()).ok());

            let scene = project
                .scenes
                .iter()
                .find(|s| s.id == job_snapshot.scene_id)
                .ok_or_else(|| {
                    StudioError::NotFound(format!("Scene {} disappeared.", job_snapshot.scene_id))
                })?
                .clone();

            let eff_win = win_cfg.unwrap_or_else(|| scene.window.clone());
            let eff_gen = gen_cfg.unwrap_or_else(|| scene.generation.clone());

            let compiled = compile_h3_window(
                &project,
                &scene,
                Some(&eff_win),
                Some(&eff_gen),
                &env,
                advanced,
                |aid| self.resolve_reference_asset(aid),
            )?;

            self.update_job_state(&mut project, job_id, |j| {
                j.status = H3RenderJobStatus::Preparing;
                j.width = compiled.resolved_width;
                j.height = compiled.resolved_height;
                j.steps = compiled.resolved_steps;
                j.denoise = compiled.resolved_denoise;
                j.compiled_prompt = compiled.compiled_prompt.clone();
                j.resolved_model = compiled.resolved_model.clone();
                j.geometry = compiled.geometry.clone();
            });
            project.phase = "preparing".into();
            project.detail = format!(
                "Preparing H3 pass {} of {total_jobs}: {}",
                idx + 1,
                job_snapshot.label
            );
            on_progress(project.detail.clone());
            self.persist_emit(&mut project, app)?;

            let exec_res: Result<(), StudioError> = async {
                let comfy_path = Path::new(comfy_root);

                // 1. Stage bound references
                let mut pic_refs = Vec::with_capacity(compiled.ordinals.pictures.len());
                for slot in &compiled.ordinals.pictures {
                    let asset = self.resolve_reference_asset(&slot.asset_id)?;
                    pic_refs.push(
                        stage_bound_reference_slot(comfy_path, project_id, job_id, slot, &asset)
                            .await?,
                    );
                }
                let mut vid_refs = Vec::with_capacity(compiled.ordinals.videos.len());
                for slot in &compiled.ordinals.videos {
                    let asset = self.resolve_reference_asset(&slot.asset_id)?;
                    vid_refs.push(
                        stage_bound_reference_slot(comfy_path, project_id, job_id, slot, &asset)
                            .await?,
                    );
                }
                let mut standalone_aud_refs = Vec::new();
                for slot in &compiled.ordinals.audios {
                    if slot.population == kestrel_app_core::H3ReferencePopulation::StandaloneAudios
                    {
                        let asset = self.resolve_reference_asset(&slot.asset_id)?;
                        standalone_aud_refs.push(
                            stage_bound_reference_slot(comfy_path, project_id, job_id, slot, &asset)
                                .await?,
                        );
                    }
                }

                // 2. Stage guides
                let mut staged_guides = Vec::with_capacity(compiled.guides.len());
                for (gi, g) in compiled.guides.iter().enumerate() {
                    let asset = self.resolve_reference_asset(&g.asset_id)?;
                    staged_guides
                        .push(stage_guide_item(comfy_path, project_id, job_id, gi, g, &asset).await?);
                }

                // 3. Render window source video/audio if needed (`uses_source_av_latent`)
                let win_sources = if compiled.uses_source_av_latent {
                    render_scene_window_sources(
                        &project,
                        &scene,
                        &compiled.geometry,
                        compiled.resolved_width,
                        compiled.resolved_height,
                        eff_gen.task_mode,
                        comfy_path,
                        job_id,
                        |kind, sid| self.resolve_timeline_clip_source(&project, kind, sid),
                    )
                    .await?
                } else {
                    super::h3_media::WindowSourceOutputs {
                        comfy_video_rel: None,
                        comfy_audio_rel: None,
                        local_source_video: None,
                    }
                };

                // 4. Spatial mask PNG if active
                let spatial_mask_rel = if let Some(rect) = &eff_win.spatial_mask_rect {
                    let rel = format!("kestrel/{project_id}/{job_id}/spatial-mask.png");
                    let dest = comfy_path.join("input").join(&rel);
                    write_spatial_mask_png(
                        &dest,
                        compiled.resolved_width,
                        compiled.resolved_height,
                        rect,
                    )?;
                    Some(rel)
                } else {
                    None
                };

                let prefix = format!("kestrel_h3/{project_id}/{job_id}");
                let graph = build_h3_comfy_graph(H3ComfyGraphBuild {
                    compile: &compiled,
                    models: &installed_models,
                    task_mode: eff_gen.task_mode,
                    sampler_name: &eff_gen.sampler_name,
                    scheduler: &eff_gen.scheduler,
                    cfg: eff_gen.cfg,
                    shift_video: eff_gen.shift_video,
                    shift_audio: eff_gen.shift_audio,
                    seed: job_snapshot.seed,
                    ref_image_size: &eff_gen.ref_image_size,
                    output_prefix: &prefix,
                    picture_refs: &pic_refs,
                    video_refs: &vid_refs,
                    standalone_audio_refs: &standalone_aud_refs,
                    guides: &staged_guides,
                    source_video_file: win_sources.comfy_video_rel.as_deref(),
                    source_audio_file: win_sources.comfy_audio_rel.as_deref(),
                    spatial_mask_file: spatial_mask_rel.as_deref(),
                    preview_available,
                });

                write_json_atomic(&take_dir.join("compile.json"), &compiled)?;
                write_json_atomic(&take_dir.join("graph.json"), &graph)?;

                self.update_job_state(&mut project, job_id, |j| {
                    j.status = H3RenderJobStatus::Rendering;
                });
                project.phase = "rendering".into();
                project.detail = format!(
                    "Rendering {} ({}x{}, {} steps, denoise {:.2})",
                    job_snapshot.label,
                    compiled.resolved_width,
                    compiled.resolved_height,
                    compiled.resolved_steps,
                    compiled.resolved_denoise
                );
                on_progress(project.detail.clone());
                self.persist_emit(&mut project, app)?;

                let client_id = format!("kestrel-h3-{}", uuid::Uuid::new_v4().simple());
                let preview_target = PreviewTarget::movie_clip(
                    job_id.clone(),
                    project_id,
                    &scene.id,
                    idx,
                );
                let preview = if preview_available
                    && compiled.resolved_width >= 256
                    && compiled.resolved_height >= 256
                {
                    LivePreviewSession::connect(
                        app,
                        &client_id,
                        preview_target,
                        &self.live_previews,
                    )
                    .await
                } else {
                    emit_preview_unavailable(app, &self.live_previews, preview_target);
                    None
                };

                let resp = self
                    .http
                    .post(format!("{COMFY_BASE}/prompt"))
                    .json(&json!({ "prompt": graph, "client_id": client_id }))
                    .send()
                    .await?;
                let status = resp.status();
                let sub_val: Value = resp.json().await?;
                write_json_atomic(&take_dir.join("submission.json"), &sub_val)?;
                if !status.is_success() {
                    return Err(StudioError::Render(format!(
                        "ComfyUI rejected H3 graph: {}",
                        truncate(&sub_val.to_string(), 700)
                    )));
                }
                let prompt_id = sub_val
                    .get("prompt_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        StudioError::Render(format!("ComfyUI returned no prompt_id: {sub_val}"))
                    })?;

                let raw_mp4 = take_dir.join("comfy-raw.mp4");
                let deadline = tokio::time::Instant::now() + COMFY_RENDER_TIMEOUT;
                loop {
                    if cancel.is_cancelled() {
                        let _ = self
                            .http
                            .post(format!("{COMFY_BASE}/interrupt"))
                            .send()
                            .await;
                        return Err(StudioError::Cancelled);
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return Err(StudioError::Render(format!(
                            "ComfyUI timed out waiting for H3 prompt {prompt_id}."
                        )));
                    }
                    let history: Value = self
                        .http
                        .get(format!("{COMFY_BASE}/history/{prompt_id}"))
                        .send()
                        .await?
                        .json()
                        .await?;
                    if let Some(entry) = history.get(prompt_id) {
                        if entry.pointer("/status/status_str").and_then(Value::as_str)
                            == Some("error")
                        {
                            let detail = comfy_execution_error(entry).unwrap_or_else(|| {
                                truncate(&entry.to_string(), 900)
                            });
                            return Err(StudioError::Render(format!("ComfyUI {detail}")));
                        }
                        if entry.pointer("/status/completed").and_then(Value::as_bool)
                            == Some(true)
                        {
                            if let Some(p) = &preview {
                                p.finish();
                            }
                            let media = find_output_media(entry, "videos").ok_or_else(|| {
                                StudioError::Render(
                                    "ComfyUI completed H3 prompt without saving a video output."
                                        .into(),
                                )
                            })?;
                            let valid_name = super::validate_comfy_filename(&media.0)?;
                            let sub_path = Path::new(&media.1);
                            if sub_path.is_absolute()
                                || sub_path.components().any(|c| {
                                    matches!(
                                        c,
                                        std::path::Component::ParentDir
                                            | std::path::Component::RootDir
                                            | std::path::Component::Prefix(_)
                                    )
                                })
                            {
                                return Err(StudioError::Invalid(format!(
                                    "Rejected unsafe ComfyUI output subfolder '{}'.",
                                    media.1
                                )));
                            }
                            let output_root = comfy_path.join("output");
                            let comfy_out = output_root.join(sub_path).join(&valid_name);
                            if let (Ok(canon_root), Ok(canon_out)) =
                                (output_root.canonicalize(), comfy_out.canonicalize())
                            {
                                if !canon_out.starts_with(&canon_root) {
                                    return Err(StudioError::Invalid(
                                        "ComfyUI output path escaped output directory.".into(),
                                    ));
                                }
                            }
                            tokio::fs::copy(&comfy_out, &raw_mp4).await?;
                            write_json_atomic(
                                &take_dir.join("result.json"),
                                &json!({
                                    "promptId": prompt_id,
                                    "comfyOutput": comfy_out,
                                }),
                            )?;
                            break;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }

                // 5. Post-process: optional 3x4 ridge color-drift cancellation + quality analysis
                self.update_job_state(&mut project, job_id, |j| {
                    j.status = H3RenderJobStatus::PostProcessing;
                });
                project.phase = "post-processing".into();
                project.detail = format!("Verifying & color-matching {}...", job_snapshot.label);
                on_progress(project.detail.clone());
                self.persist_emit(&mut project, app)?;

                let final_take_mp4 = take_dir.join("take.mp4");
                let drift_report = if eff_gen.color_drift_correction {
                    if let Some(src_vid) = &win_sources.local_source_video {
                        Some(
                            apply_context_color_drift_correction(
                                src_vid,
                                &raw_mp4,
                                &final_take_mp4,
                                &compiled.geometry,
                                compiled.resolved_width,
                                compiled.resolved_height,
                            )
                            .await?,
                        )
                    } else {
                        tokio::fs::copy(&raw_mp4, &final_take_mp4).await?;
                        None
                    }
                } else {
                    tokio::fs::copy(&raw_mp4, &final_take_mp4).await?;
                    None
                };
                if final_take_mp4.is_file() {
                    let _ = fs::remove_file(&raw_mp4);
                }

                let take_sha = hash_reference(&final_take_mp4)?;
                let quality = analyze_take_quality(
                    &final_take_mp4,
                    &take_dir,
                    compiled.resolved_width,
                    compiled.resolved_height,
                    true,
                )
                .await?;
                let take_path_str = final_take_mp4.to_string_lossy().into_owned();

                // Sync latest project state, record completed job + RenderedClip, and auto-place!
                let lock = self.project_lock(project_id)?;
                let _guard = lock.lock().await;
                project = self.get(project_id)?;
                let placed_clip_id = place_completed_take_on_scene(
                    &mut project,
                    &scene.id,
                    job_id,
                    &job_snapshot.label,
                    eff_gen.task_mode,
                    eff_gen.placement_mode,
                    &compiled.geometry,
                    None,
                    None,
                    None,
                )?;
                sync_rendered_clip_entry(
                    &mut project,
                    job_id,
                    &job_snapshot.label,
                    &compiled.compiled_prompt,
                    compiled.geometry.trimmed_duration_sec as f32,
                    job_snapshot.seed,
                    &take_path_str,
                );
                self.update_job_state(&mut project, job_id, |j| {
                    j.status = H3RenderJobStatus::Complete;
                    j.take_media_path = take_path_str;
                    j.take_sha256 = take_sha;
                    j.color_drift = drift_report;
                    j.quality = Some(quality);
                    j.output_clip_id = placed_clip_id;
                    j.error.clear();
                });
                self.persist_emit(&mut project, app)?;
                Ok(())
            }
            .await;

            if let Err(err) = exec_res {
                let is_cancel = matches!(err, StudioError::Cancelled);
                let lock = self.project_lock(project_id)?;
                let _guard = lock.lock().await;
                if let Ok(mut latest) = self.get(project_id) {
                    for j in &mut latest.render_jobs {
                        if j.id == *job_id {
                            j.status = if is_cancel {
                                H3RenderJobStatus::Cancelled
                            } else {
                                H3RenderJobStatus::Failed
                            };
                            j.error = err.to_string();
                            j.updated_at = Utc::now().to_rfc3339();
                        } else if j.status == H3RenderJobStatus::Queued {
                            j.status = H3RenderJobStatus::Cancelled;
                            j.error = if is_cancel {
                                "Cancelled.".into()
                            } else {
                                format!("Cancelled because an earlier queued pass failed: {err}")
                            };
                            j.updated_at = Utc::now().to_rfc3339();
                        }
                    }
                    latest.status = if is_cancel { "cancelled" } else { "failed" }.into();
                    latest.phase = latest.status.clone();
                    latest.error = if is_cancel {
                        String::new()
                    } else {
                        err.to_string()
                    };
                    latest.detail = if is_cancel {
                        "Generation stopped. Completed takes remain safe on disk.".into()
                    } else {
                        format!("Generation failed: {err}")
                    };
                    let _ = self.persist_emit(&mut latest, app);
                }
                return Err(err);
            }
        }

        let lock = self.project_lock(project_id)?;
        let _guard = lock.lock().await;
        project = self.get(project_id)?;
        project.status = "complete".into();
        project.phase = "ready".into();
        project.error.clear();
        project.detail = format!("Completed {total_jobs} MiniMax H3 pass(es).");
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub async fn place_h3_take(
        &self,
        request: PlaceMovieH3TakeRequest,
        app: Option<&AppHandle>,
    ) -> Result<MovieProject, StudioError> {
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;

        if let Some(asset_id) = request
            .asset_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let asset = self.resolve_reference_asset(asset_id)?;
            self.ensure_project_reference_copy(&mut project, &asset, &asset.name)?;
            let _ = place_reference_asset_on_scene(&mut project, &request, &asset)?;
            project.detail = format!("Placed asset '{}' onto scene timeline.", asset.name);
            self.persist_emit(&mut project, app)?;
            return Ok(project);
        }

        let job_id = request.job_id.trim();
        if job_id.is_empty() {
            return Err(StudioError::Invalid(
                "Supply either jobId (completed take) or assetId (reference asset) to place on the timeline.".into(),
            ));
        }
        let job = project
            .render_jobs
            .iter()
            .find(|j| j.id == job_id)
            .cloned()
            .ok_or_else(|| StudioError::NotFound(format!("Take {job_id} not found.")))?;
        if job.status != H3RenderJobStatus::Complete || !Path::new(&job.take_media_path).is_file() {
            return Err(StudioError::Invalid(format!(
                "Take {job_id} does not have a completed media file on disk."
            )));
        }

        if let Some(driver_lane_id) = request
            .target_driver_lane_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let clip_id = place_take_on_driver_lane(
                &mut project,
                &request.target_scene_id,
                &job,
                request.placement_mode,
                driver_lane_id,
                request.timeline_start_override_sec,
                request.duration_override_sec,
            )?;
            self.update_job_state(&mut project, &job.id, |j| {
                j.output_clip_id = Some(clip_id);
            });
            project.detail = format!("Placed take '{}' onto driver lane.", job.label);
            self.persist_emit(&mut project, app)?;
            return Ok(project);
        }

        let clip_id = place_completed_take_on_scene(
            &mut project,
            &request.target_scene_id,
            &job.id,
            &job.label,
            job.task_mode,
            request.placement_mode,
            &job.geometry,
            request.target_video_lane_id.as_deref(),
            request.target_audio_lane_id.as_deref(),
            request.timeline_start_override_sec,
        )?;
        self.update_job_state(&mut project, &job.id, |j| {
            if clip_id.is_some() {
                j.output_clip_id = clip_id;
            }
        });
        project.detail = format!("Placed take '{}' onto scene timeline.", job.label);
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub fn inspect_h3_take(
        &self,
        project_id: &str,
        job_id: &str,
    ) -> Result<H3TakeInspection, StudioError> {
        validate_id_like(job_id, "job id")?;
        let project = self.get(project_id)?;
        let job = project
            .render_jobs
            .iter()
            .find(|j| j.id == job_id)
            .ok_or_else(|| StudioError::NotFound(format!("Take job {job_id} not found.")))?;
        let take_dir = self.project_dir(project_id).join("takes").join(job_id);
        let provenance_summary = fs::read(take_dir.join("result.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok());

        Ok(H3TakeInspection {
            job_id: job.id.clone(),
            project_id: project.id.clone(),
            scene_id: job.scene_id.clone(),
            label: job.label.clone(),
            status: job.status,
            pass_preset: job.pass_preset,
            task_mode: job.task_mode,
            resolved_model: job.resolved_model.clone(),
            seed: job.seed,
            compiled_prompt: job.compiled_prompt.clone(),
            take_media_path: job.take_media_path.clone(),
            take_sha256: job.take_sha256.clone(),
            geometry: job.geometry.clone(),
            color_drift: job.color_drift.clone(),
            quality: job.quality.clone(),
            provenance_summary,
            error: job.error.clone(),
        })
    }

    pub async fn render_h3_export(
        &self,
        request: RenderMovieExportRequest,
        app: Option<&AppHandle>,
    ) -> Result<MovieExportReceipt, StudioError> {
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;

        let preset = if request.preset.trim().is_empty() {
            "publish"
        } else {
            request.preset.trim()
        };
        let ext = export::export_extension_for_preset(preset)?;
        let title = request
            .title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&project.title)
            .to_string();
        let headroom = request.peak_headroom_db.unwrap_or(-1.0);

        // Validate optional destinationPath up front before running FFmpeg.
        let validated_destination = if let Some(dest_str) = request
            .destination_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if Path::new(dest_str)
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(StudioError::Invalid(
                    "destinationPath must not contain parent-directory ('..') traversal.".into(),
                ));
            }
            Some(
                crate::app_control::export_path(dest_str, ext)
                    .map_err(StudioError::Invalid)?,
            )
        } else {
            None
        };

        let scene_id = request
            .scene_id
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| project.active_scene_id.clone());

        let export_id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
        let file_stem = format!(
            "{}-{}-{export_id}",
            safe_export_stem(&title),
            Utc::now().format("%Y%m%d-%H%M%S")
        );
        let exports_dir = self.project_dir(&project.id).join("exports");
        fs::create_dir_all(&exports_dir)?;
        let target = exports_dir.join(format!("{file_stem}.{ext}"));
        let temporary = exports_dir.join(format!("{file_stem}.partial.{ext}"));

        // Use H3 multi-lane export when the target scene has placed clips; otherwise fall back to
        // legacy `MovieEdit` timeline if present.
        let target_scene = project
            .scenes
            .iter()
            .find(|s| s.id == scene_id)
            .or_else(|| project.scenes.first())
            .cloned();

        let has_h3_clips = target_scene.as_ref().is_some_and(|sc| {
            sc.video_lanes.iter().any(|l| !l.clips.is_empty())
                || sc.audio_lanes.iter().any(|l| !l.clips.is_empty())
        });

        let (duration_seconds, clip_count, used_scene_id) = if has_h3_clips {
            let sc = target_scene.unwrap();
            let (dur, count) = export::render_h3_scene(
                &sc,
                preset,
                &title,
                headroom,
                &temporary,
                |kind, sid| self.resolve_timeline_clip_source(&project, kind, sid),
            )
            .await?;
            (dur, count, sc.id)
        } else if !project.edit.clips.is_empty() {
            project.edit.export_preset = preset.to_string();
            project.edit.export_title = title.clone();
            let mut edits: Vec<_> = project
                .edit
                .clips
                .iter()
                .filter(|c| c.enabled)
                .cloned()
                .collect();
            edits.sort_by_key(|c| c.order);
            if edits.is_empty() {
                return Err(StudioError::Invalid(
                    "Enable at least one clip before exporting.".into(),
                ));
            }
            let dur = export::render_timeline(&project, &edits, &temporary).await?;
            (dur, edits.len(), scene_id)
        } else {
            return Err(StudioError::Invalid(
                "Scene has no placed video or audio clips to export yet.".into(),
            ));
        };

        let bytes = temporary.metadata()?.len();
        let sha256 = hash_reference(&temporary)?;
        let export_record = MovieExport {
            id: export_id,
            created_at: Utc::now().to_rfc3339(),
            title: title.clone(),
            preset: preset.to_string(),
            path: target.to_string_lossy().into_owned(),
            bytes,
            sha256,
            duration_seconds,
            clip_count,
        };

        let sidecar = exports_dir.join(format!("{file_stem}.json"));
        write_json_atomic(
            &sidecar,
            &json!({
                "schemaVersion": SCHEMA_VERSION,
                "projectId": project.id,
                "sceneId": used_scene_id,
                "peakHeadroomDb": headroom,
                "export": export_record,
            }),
        )?;
        fs::rename(&temporary, &target)?;

        let mut destination_copy_path = None;
        if let Some(dest) = validated_destination {
            fs::copy(&target, &dest)?;
            destination_copy_path = Some(dest.to_string_lossy().into_owned());
        }

        if ext != "flac" {
            project.final_path = target.to_string_lossy().into_owned();
        }
        project.exports.push(export_record.clone());
        project.detail = format!("Exported '{}' ({preset}).", title);
        self.persist_emit(&mut project, app)?;

        Ok(MovieExportReceipt {
            project_id: project.id,
            scene_id: used_scene_id,
            export: export_record,
            destination_copy_path,
        })
    }

    pub(super) fn recover_h3_render_jobs(&self) -> Result<(), StudioError> {
        for entry in fs::read_dir(&self.root)?.filter_map(Result::ok) {
            let path = entry.path().join("project.json");
            if !path.is_file() {
                continue;
            }
            let Ok(mut project) = self.load_path(&path) else {
                continue;
            };
            let mut dirty = false;
            for job in &mut project.render_jobs {
                if matches!(
                    job.status,
                    H3RenderJobStatus::Queued
                        | H3RenderJobStatus::Preparing
                        | H3RenderJobStatus::Rendering
                        | H3RenderJobStatus::PostProcessing
                ) {
                    job.status = H3RenderJobStatus::Failed;
                    job.error = "Kestrel closed before this H3 render pass finished. Completed takes remain preserved.".into();
                    job.updated_at = Utc::now().to_rfc3339();
                    dirty = true;
                }
            }
            if dirty {
                write_json_atomic(&path, &project)?;
            }
        }
        Ok(())
    }

    fn resolve_timeline_clip_source(
        &self,
        project: &MovieProject,
        kind: H3ClipSourceKind,
        source_id: &str,
    ) -> Result<(PathBuf, bool), StudioError> {
        match kind {
            H3ClipSourceKind::Take => {
                let job = project
                    .render_jobs
                    .iter()
                    .find(|j| j.id == source_id)
                    .ok_or_else(|| {
                        StudioError::Invalid(format!("Timeline clip references unknown take {source_id}."))
                    })?;
                let path = PathBuf::from(&job.take_media_path);
                if !path.is_file() {
                    return Err(StudioError::Invalid(format!(
                        "Take media for {source_id} is missing on disk at {}.",
                        job.take_media_path
                    )));
                }
                let has_audio = job.quality.as_ref().is_none_or(|q| q.has_audio);
                Ok((path, has_audio))
            }
            H3ClipSourceKind::ReferenceAsset => {
                if let Ok(asset) = self.resolve_reference_asset(source_id) {
                    return Ok((PathBuf::from(asset.path), asset.has_audio));
                }
                if let Some(job) = project.render_jobs.iter().find(|j| j.id == source_id) {
                    let path = PathBuf::from(&job.take_media_path);
                    if path.is_file() {
                        let has_audio = job.quality.as_ref().is_none_or(|q| q.has_audio);
                        return Ok((path, has_audio));
                    }
                }
                let asset = self.resolve_reference_asset(source_id)?;
                Ok((PathBuf::from(asset.path), asset.has_audio))
            }
        }
    }

    fn ensure_project_reference_copy(
        &self,
        project: &mut MovieProject,
        asset: &MovieReferenceAsset,
        desc: &str,
    ) -> Result<(), StudioError> {
        if project.references.iter().any(|r| r.asset_id == asset.id) {
            return Ok(());
        }
        let refs_dir = self.project_dir(&project.id).join("references");
        fs::create_dir_all(&refs_dir)?;
        let ext = Path::new(&asset.path)
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("bin");
        let target = refs_dir.join(format!("asset-{}.{}", &asset.id[..12], ext));
        if !target.is_file() && fs::hard_link(&asset.path, &target).is_err() {
            fs::copy(&asset.path, &target)?;
        }
        project.references.push(super::MovieReference {
            asset_id: asset.id.clone(),
            tag: String::new(),
            audio_tag: String::new(),
            name: asset.name.clone(),
            kind: asset.kind.clone(),
            mime_type: asset.mime_type.clone(),
            bytes: asset.bytes,
            duration_seconds: asset.duration_seconds,
            width: asset.width,
            height: asset.height,
            has_audio: asset.has_audio,
            path: target.to_string_lossy().into_owned(),
            description: truncate(
                if desc.trim().is_empty() {
                    &asset.name
                } else {
                    desc.trim()
                },
                1_000,
            ),
            use_embedded_audio: asset.kind == "video" && asset.has_audio,
            embedded_audio_description: String::new(),
            generation: asset.generation.clone(),
        });
        Ok(())
    }

    fn update_job_state(
        &self,
        project: &mut MovieProject,
        job_id: &str,
        mutate: impl FnOnce(&mut H3RenderQueueJob),
    ) {
        if let Some(job) = project.render_jobs.iter_mut().find(|j| j.id == job_id) {
            mutate(job);
            job.updated_at = Utc::now().to_rfc3339();
        }
    }
}

fn validate_h3_scene(scene: &mut H3Scene, advanced: bool) -> Result<(), StudioError> {
    validate_id_like(&scene.id, "scene id")?;
    let name = scene.name.trim();
    if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(StudioError::Invalid(
            "Scene name must contain 1 to 100 printable characters.".into(),
        ));
    }
    scene.name = name.to_string();
    let (w, h) = validate_canvas_dimensions(
        scene.width,
        scene.height,
        scene.kind == H3SceneKind::Soundtrack,
        advanced,
    )?;
    scene.width = w;
    scene.height = h;
    scene.fps = H3_FPS;
    if !scene.duration_sec.is_finite() || !(1.0..=1_800.0).contains(&scene.duration_sec) {
        return Err(StudioError::Invalid(
            "Scene duration must be between 1s and 1800s.".into(),
        ));
    }
    let _ = calculate_take_placement(&scene.window, scene.fps)?;

    if scene.video_lanes.is_empty() {
        scene.video_lanes.push(H3VideoLane {
            id: "vid-lane-1".into(),
            name: "V1".into(),
            muted: false,
            solo: false,
            locked: false,
            clips: Vec::new(),
        });
    }
    if scene.audio_lanes.is_empty() {
        scene.audio_lanes.push(H3AudioLane {
            id: "aud-lane-1".into(),
            name: "A1 (Dialogue & Foley)".into(),
            muted: false,
            solo: false,
            locked: false,
            clips: Vec::new(),
        });
        scene.audio_lanes.push(H3AudioLane {
            id: "aud-lane-2".into(),
            name: "A2 (Score)".into(),
            muted: false,
            solo: false,
            locked: false,
            clips: Vec::new(),
        });
    }
    if scene.driver_lanes.is_empty() {
        scene.driver_lanes.push(H3DriverLane {
            id: "drv-lane-1".into(),
            name: "D1 (Pose / Motion Driver)".into(),
            muted: false,
            locked: false,
            clips: Vec::new(),
        });
    }
    if scene.reference_lanes.is_empty() {
        scene.reference_lanes.push(H3ReferenceLane {
            id: "ref-lane-1".into(),
            name: "R1 (Cast & World)".into(),
            muted: false,
            locked: false,
            items: Vec::new(),
        });
    }

    for lane in &scene.video_lanes {
        for clip in &lane.clips {
            validate_timeline_span(
                clip.timeline_start_sec,
                clip.duration_sec,
                clip.source_in_sec,
                &format!("video clip '{}'", clip.id),
            )?;
        }
    }
    for lane in &scene.audio_lanes {
        for clip in &lane.clips {
            validate_timeline_span(
                clip.timeline_start_sec,
                clip.duration_sec,
                clip.source_in_sec,
                &format!("audio clip '{}'", clip.id),
            )?;
        }
    }
    for lane in &scene.driver_lanes {
        for clip in &lane.clips {
            validate_timeline_span(
                clip.timeline_start_sec,
                clip.duration_sec,
                clip.source_in_sec,
                &format!("driver clip '{}'", clip.id),
            )?;
        }
    }
    for lane in &scene.reference_lanes {
        for item in &lane.items {
            validate_timeline_span(
                item.timeline_start_sec,
                item.duration_sec,
                0.0,
                &format!("reference item '{}'", item.id),
            )?;
        }
    }
    for sec in &scene.prompt_sections {
        validate_timeline_span(
            sec.timeline_start_sec,
            sec.duration_sec,
            0.0,
            &format!("prompt section '{}'", sec.id),
        )?;
    }
    for g in &scene.guides {
        if !g.timeline_sec.is_finite() || g.timeline_sec < 0.0 || g.timeline_sec > 86_400.0 {
            return Err(StudioError::Invalid(format!(
                "Guide '{}' timelineSec must be between 0s and 86400s.",
                g.id
            )));
        }
    }
    for lora in &scene.generation.loras {
        if !lora.file_name.trim().is_empty() {
            super::validate_comfy_filename(lora.file_name.trim())?;
        }
    }
    Ok(())
}

fn validate_timeline_span(
    start_sec: f64,
    duration_sec: f64,
    source_in_sec: f64,
    label: &str,
) -> Result<(), StudioError> {
    if !start_sec.is_finite()
        || !duration_sec.is_finite()
        || !source_in_sec.is_finite()
        || !(0.0..=86_400.0).contains(&start_sec)
        || duration_sec <= 0.0
        || duration_sec > 86_400.0
        || source_in_sec < 0.0
    {
        return Err(StudioError::Invalid(format!(
            "Invalid timeline span on {label}: start={start_sec}, duration={duration_sec}, sourceIn={source_in_sec}."
        )));
    }
    Ok(())
}

fn validate_id_like(id: &str, label: &str) -> Result<(), StudioError> {
    if id.trim().is_empty()
        || id.len() > 96
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err(StudioError::Invalid(format!(
            "Invalid {label} '{id}': use 1-96 ASCII letters, digits, hyphens, or underscores."
        )));
    }
    Ok(())
}

fn select_audio_lane_index(
    lanes: &[H3AudioLane],
    explicit_id: Option<&str>,
    prefer_score_lane: bool,
) -> Result<usize, StudioError> {
    if let Some(lid) = explicit_id.map(str::trim).filter(|s| !s.is_empty()) {
        let idx = lanes
            .iter()
            .position(|l| l.id == lid)
            .ok_or_else(|| StudioError::NotFound(format!("Audio lane {lid} not found.")))?;
        if lanes[idx].locked {
            return Err(StudioError::Invalid(format!(
                "Audio lane '{}' is locked; unlock it or choose another lane.",
                lanes[idx].name
            )));
        }
        return Ok(idx);
    }
    let any_solo = lanes.iter().any(|l| l.solo);
    if prefer_score_lane && lanes.len() > 1 {
        let l = &lanes[1];
        if !l.locked && !l.muted && (!any_solo || l.solo) {
            return Ok(1);
        }
    }
    if let Some(idx) = lanes
        .iter()
        .position(|l| !l.locked && !l.muted && (!any_solo || l.solo))
    {
        return Ok(idx);
    }
    if let Some(idx) = lanes.iter().position(|l| !l.locked) {
        return Ok(idx);
    }
    Err(StudioError::Invalid(
        "All audio lanes on this scene are locked.".into(),
    ))
}

fn select_video_lane_index(
    lanes: &[H3VideoLane],
    explicit_id: Option<&str>,
) -> Result<usize, StudioError> {
    if let Some(lid) = explicit_id.map(str::trim).filter(|s| !s.is_empty()) {
        let idx = lanes
            .iter()
            .position(|l| l.id == lid)
            .ok_or_else(|| StudioError::NotFound(format!("Video lane {lid} not found.")))?;
        if lanes[idx].locked {
            return Err(StudioError::Invalid(format!(
                "Video lane '{}' is locked; unlock it or choose another lane.",
                lanes[idx].name
            )));
        }
        return Ok(idx);
    }
    let any_solo = lanes.iter().any(|l| l.solo);
    if let Some(idx) = lanes
        .iter()
        .position(|l| !l.locked && !l.muted && (!any_solo || l.solo))
    {
        return Ok(idx);
    }
    if let Some(idx) = lanes.iter().position(|l| !l.locked) {
        return Ok(idx);
    }
    Err(StudioError::Invalid(
        "All video lanes on this scene are locked.".into(),
    ))
}

fn select_driver_lane_index(
    lanes: &[H3DriverLane],
    explicit_id: Option<&str>,
) -> Result<usize, StudioError> {
    if let Some(lid) = explicit_id.map(str::trim).filter(|s| !s.is_empty()) {
        let idx = lanes
            .iter()
            .position(|l| l.id == lid)
            .ok_or_else(|| StudioError::NotFound(format!("Driver lane {lid} not found.")))?;
        if lanes[idx].locked {
            return Err(StudioError::Invalid(format!(
                "Driver lane '{}' is locked; unlock it or choose another lane.",
                lanes[idx].name
            )));
        }
        return Ok(idx);
    }
    if let Some(idx) = lanes.iter().position(|l| !l.locked && !l.muted) {
        return Ok(idx);
    }
    if let Some(idx) = lanes.iter().position(|l| !l.locked) {
        return Ok(idx);
    }
    Err(StudioError::Invalid(
        "All driver lanes on this scene are locked.".into(),
    ))
}

fn place_take_on_driver_lane(
    project: &mut MovieProject,
    scene_id: &str,
    job: &H3RenderQueueJob,
    placement_mode: H3TakePlacementMode,
    driver_lane_id: &str,
    timeline_start_override_sec: Option<f64>,
    duration_override_sec: Option<f64>,
) -> Result<String, StudioError> {
    let scene = project
        .scenes
        .iter_mut()
        .find(|s| s.id == scene_id)
        .ok_or_else(|| StudioError::NotFound(format!("Scene {scene_id} not found.")))?;
    if scene.driver_lanes.is_empty() {
        scene.driver_lanes.push(H3DriverLane {
            id: "drv-lane-1".into(),
            name: "D1 (Pose / Motion Driver)".into(),
            muted: false,
            locked: false,
            clips: Vec::new(),
        });
    }
    let lane_idx = select_driver_lane_index(&scene.driver_lanes, Some(driver_lane_id))?;
    let (tl_start, src_in, default_dur) = match placement_mode {
        H3TakePlacementMode::Trimmed | H3TakePlacementMode::AssetOnly => (
            timeline_start_override_sec.unwrap_or(job.geometry.trimmed_timeline_start_sec),
            job.geometry.trimmed_source_in_sec,
            job.geometry.trimmed_duration_sec,
        ),
        H3TakePlacementMode::Untrimmed => (
            timeline_start_override_sec.unwrap_or(job.geometry.untrimmed_timeline_start_sec),
            0.0,
            job.geometry.untrimmed_duration_sec,
        ),
    };
    let dur = duration_override_sec.unwrap_or(default_dur).max(0.25);
    let tl_end = tl_start + dur;
    if tl_end > scene.duration_sec {
        scene.duration_sec = tl_end;
    }
    let clip_id = format!("drv-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
    let lane = &mut scene.driver_lanes[lane_idx];
    overwrite_driver_lane_span(&mut lane.clips, tl_start, tl_end);
    lane.clips.push(H3DriverClip {
        id: clip_id.clone(),
        asset_id: job.id.clone(),
        label: job.label.clone(),
        timeline_start_sec: tl_start,
        duration_sec: dur,
        source_in_sec: src_in,
        enabled: true,
    });
    lane.clips.sort_by(|a, b| {
        a.timeline_start_sec
            .partial_cmp(&b.timeline_start_sec)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scene.updated_at = Utc::now().to_rfc3339();
    Ok(clip_id)
}

fn place_reference_asset_on_scene(
    project: &mut MovieProject,
    request: &PlaceMovieH3TakeRequest,
    asset: &MovieReferenceAsset,
) -> Result<Option<String>, StudioError> {
    if request.placement_mode == H3TakePlacementMode::AssetOnly {
        return Ok(None);
    }
    let scene = project
        .scenes
        .iter_mut()
        .find(|s| s.id == request.target_scene_id)
        .ok_or_else(|| {
            StudioError::NotFound(format!("Scene {} not found.", request.target_scene_id))
        })?;
    let tl_start = request
        .timeline_start_override_sec
        .unwrap_or(scene.window.in_sec)
        .max(0.0);
    let asset_dur = asset.duration_seconds.max(0.25);
    let default_dur = if asset.kind == "image" {
        (scene.window.out_sec - scene.window.in_sec).max(1.0)
    } else {
        asset_dur
    };
    let dur = request
        .duration_override_sec
        .unwrap_or(default_dur)
        .max(0.25);
    let tl_end = tl_start + dur;
    if tl_end > scene.duration_sec {
        scene.duration_sec = tl_end;
    }

    let clip_id = format!("clip-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);

    if let Some(driver_lane_id) = request
        .target_driver_lane_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if scene.driver_lanes.is_empty() {
            scene.driver_lanes.push(H3DriverLane {
                id: "drv-lane-1".into(),
                name: "D1 (Pose / Motion Driver)".into(),
                muted: false,
                locked: false,
                clips: Vec::new(),
            });
        }
        let lane_idx = select_driver_lane_index(&scene.driver_lanes, Some(driver_lane_id))?;
        let lane = &mut scene.driver_lanes[lane_idx];
        overwrite_driver_lane_span(&mut lane.clips, tl_start, tl_end);
        lane.clips.push(H3DriverClip {
            id: clip_id.clone(),
            asset_id: asset.id.clone(),
            label: asset.name.clone(),
            timeline_start_sec: tl_start,
            duration_sec: dur,
            source_in_sec: 0.0,
            enabled: true,
        });
        lane.clips.sort_by(|a, b| {
            a.timeline_start_sec
                .partial_cmp(&b.timeline_start_sec)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scene.updated_at = Utc::now().to_rfc3339();
        return Ok(Some(clip_id));
    }

    let place_as_audio = asset.kind == "audio"
        || (request
            .target_audio_lane_id
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            && request
                .target_video_lane_id
                .as_deref()
                .is_none_or(|s| s.trim().is_empty()));

    if place_as_audio {
        if scene.audio_lanes.is_empty() {
            scene.audio_lanes.push(H3AudioLane {
                id: "aud-lane-1".into(),
                name: "A1".into(),
                muted: false,
                solo: false,
                locked: false,
                clips: Vec::new(),
            });
        }
        let lane_idx = select_audio_lane_index(
            &scene.audio_lanes,
            request.target_audio_lane_id.as_deref(),
            true,
        )?;
        let lane = &mut scene.audio_lanes[lane_idx];
        overwrite_audio_lane_span(&mut lane.clips, tl_start, tl_end);
        lane.clips.push(H3AudioClip {
            id: clip_id.clone(),
            source_kind: H3ClipSourceKind::ReferenceAsset,
            source_id: asset.id.clone(),
            label: asset.name.clone(),
            timeline_start_sec: tl_start,
            duration_sec: dur,
            source_in_sec: 0.0,
            source_duration_sec: dur,
            gain: if lane_idx == 1 { 0.65 } else { 1.0 },
            fade_in_sec: 0.0,
            fade_out_sec: 0.0,
            enabled: true,
        });
        lane.clips.sort_by(|a, b| {
            a.timeline_start_sec
                .partial_cmp(&b.timeline_start_sec)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        if scene.video_lanes.is_empty() {
            scene.video_lanes.push(H3VideoLane {
                id: "vid-lane-1".into(),
                name: "V1".into(),
                muted: false,
                solo: false,
                locked: false,
                clips: Vec::new(),
            });
        }
        let lane_idx = select_video_lane_index(
            &scene.video_lanes,
            request.target_video_lane_id.as_deref(),
        )?;
        let lane = &mut scene.video_lanes[lane_idx];
        overwrite_video_lane_span(&mut lane.clips, tl_start, tl_end);
        lane.clips.push(H3VideoClip {
            id: clip_id.clone(),
            source_kind: H3ClipSourceKind::ReferenceAsset,
            source_id: asset.id.clone(),
            label: asset.name.clone(),
            timeline_start_sec: tl_start,
            duration_sec: dur,
            source_in_sec: 0.0,
            source_duration_sec: dur,
            opacity: 1.0,
            audio_gain: 1.0,
            include_embedded_audio: asset.has_audio,
            enabled: true,
        });
        lane.clips.sort_by(|a, b| {
            a.timeline_start_sec
                .partial_cmp(&b.timeline_start_sec)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    scene.updated_at = Utc::now().to_rfc3339();
    Ok(Some(clip_id))
}

#[allow(clippy::too_many_arguments)]
fn place_completed_take_on_scene(
    project: &mut MovieProject,
    scene_id: &str,
    job_id: &str,
    label: &str,
    task_mode: H3TaskMode,
    placement_mode: H3TakePlacementMode,
    geometry: &H3TakePlacementGeometry,
    target_video_lane_id: Option<&str>,
    target_audio_lane_id: Option<&str>,
    timeline_start_override_sec: Option<f64>,
) -> Result<Option<String>, StudioError> {
    if placement_mode == H3TakePlacementMode::AssetOnly {
        return Ok(None);
    }
    let scene = project
        .scenes
        .iter_mut()
        .find(|s| s.id == scene_id)
        .ok_or_else(|| StudioError::NotFound(format!("Scene {scene_id} not found.")))?;

    let (tl_start, src_in, dur) = match placement_mode {
        H3TakePlacementMode::Trimmed => (
            timeline_start_override_sec.unwrap_or(geometry.trimmed_timeline_start_sec),
            geometry.trimmed_source_in_sec,
            geometry.trimmed_duration_sec,
        ),
        H3TakePlacementMode::Untrimmed => (
            timeline_start_override_sec.unwrap_or(geometry.untrimmed_timeline_start_sec),
            0.0,
            geometry.untrimmed_duration_sec,
        ),
        H3TakePlacementMode::AssetOnly => return Ok(None),
    };
    let tl_end = tl_start + dur;
    if tl_end > scene.duration_sec {
        scene.duration_sec = tl_end;
    }

    let clip_id = format!("clip-{}", &uuid::Uuid::new_v4().simple().to_string()[..10]);
    let audio_only_mode = matches!(
        task_mode,
        H3TaskMode::SoundtrackPass | H3TaskMode::AudioOverExistingVideo
    ) || scene.kind == H3SceneKind::Soundtrack;

    if audio_only_mode {
        if scene.audio_lanes.is_empty() {
            scene.audio_lanes.push(H3AudioLane {
                id: "aud-lane-1".into(),
                name: "A1".into(),
                muted: false,
                solo: false,
                locked: false,
                clips: Vec::new(),
            });
        }
        let prefer_score =
            task_mode == H3TaskMode::SoundtrackPass || scene.kind == H3SceneKind::Soundtrack;
        let lane_idx = select_audio_lane_index(
            &scene.audio_lanes,
            target_audio_lane_id,
            prefer_score,
        )?;
        let lane = &mut scene.audio_lanes[lane_idx];
        overwrite_audio_lane_span(&mut lane.clips, tl_start, tl_end);
        lane.clips.push(H3AudioClip {
            id: clip_id.clone(),
            source_kind: H3ClipSourceKind::Take,
            source_id: job_id.to_string(),
            label: label.to_string(),
            timeline_start_sec: tl_start,
            duration_sec: dur,
            source_in_sec: src_in,
            source_duration_sec: geometry.untrimmed_duration_sec,
            gain: if task_mode == H3TaskMode::SoundtrackPass {
                0.65
            } else {
                1.0
            },
            fade_in_sec: 0.0,
            fade_out_sec: 0.0,
            enabled: true,
        });
        lane.clips.sort_by(|a, b| {
            a.timeline_start_sec
                .partial_cmp(&b.timeline_start_sec)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        if scene.video_lanes.is_empty() {
            scene.video_lanes.push(H3VideoLane {
                id: "vid-lane-1".into(),
                name: "V1".into(),
                muted: false,
                solo: false,
                locked: false,
                clips: Vec::new(),
            });
        }
        let lane_idx = select_video_lane_index(&scene.video_lanes, target_video_lane_id)?;
        let lane = &mut scene.video_lanes[lane_idx];
        // Non-destructive overwrite on the target lane (`V1` by default) prevents stacked
        // multi-pass takes from doubling embedded audio while preserving prior takes in `render_jobs`!
        overwrite_video_lane_span(&mut lane.clips, tl_start, tl_end);
        lane.clips.push(H3VideoClip {
            id: clip_id.clone(),
            source_kind: H3ClipSourceKind::Take,
            source_id: job_id.to_string(),
            label: label.to_string(),
            timeline_start_sec: tl_start,
            duration_sec: dur,
            source_in_sec: src_in,
            source_duration_sec: geometry.untrimmed_duration_sec,
            opacity: 1.0,
            audio_gain: 1.0,
            include_embedded_audio: task_mode != H3TaskMode::KeepExistingAudio,
            enabled: true,
        });
        lane.clips.sort_by(|a, b| {
            a.timeline_start_sec
                .partial_cmp(&b.timeline_start_sec)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    scene.updated_at = Utc::now().to_rfc3339();
    Ok(Some(clip_id))
}

fn overwrite_driver_lane_span(clips: &mut Vec<H3DriverClip>, cut_start: f64, cut_end: f64) {
    let mut updated = Vec::with_capacity(clips.len() + 1);
    for clip in clips.drain(..) {
        let c_start = clip.timeline_start_sec;
        let c_end = c_start + clip.duration_sec;
        if c_end <= cut_start + 1e-3 || c_start >= cut_end - 1e-3 {
            updated.push(clip);
            continue;
        }
        if c_start < cut_start - 0.04 {
            let mut left = clip.clone();
            left.duration_sec = cut_start - c_start;
            updated.push(left);
        }
        if c_end > cut_end + 0.04 {
            let mut right = clip;
            right.id = format!("{}-tail", right.id);
            let delta = cut_end - c_start;
            right.timeline_start_sec = cut_end;
            right.source_in_sec += delta;
            right.duration_sec = c_end - cut_end;
            updated.push(right);
        }
    }
    *clips = updated;
}

fn overwrite_video_lane_span(clips: &mut Vec<H3VideoClip>, cut_start: f64, cut_end: f64) {
    let mut updated = Vec::with_capacity(clips.len() + 1);
    for clip in clips.drain(..) {
        let c_start = clip.timeline_start_sec;
        let c_end = c_start + clip.duration_sec;
        if c_end <= cut_start + 1e-3 || c_start >= cut_end - 1e-3 {
            updated.push(clip);
            continue;
        }
        if c_start < cut_start - 0.04 {
            let mut left = clip.clone();
            left.duration_sec = cut_start - c_start;
            updated.push(left);
        }
        if c_end > cut_end + 0.04 {
            let mut right = clip;
            right.id = format!("{}-tail", right.id);
            let delta = cut_end - c_start;
            right.timeline_start_sec = cut_end;
            right.source_in_sec += delta;
            right.duration_sec = c_end - cut_end;
            updated.push(right);
        }
    }
    *clips = updated;
}

fn overwrite_audio_lane_span(clips: &mut Vec<H3AudioClip>, cut_start: f64, cut_end: f64) {
    let mut updated = Vec::with_capacity(clips.len() + 1);
    for clip in clips.drain(..) {
        let c_start = clip.timeline_start_sec;
        let c_end = c_start + clip.duration_sec;
        if c_end <= cut_start + 1e-3 || c_start >= cut_end - 1e-3 {
            updated.push(clip);
            continue;
        }
        if c_start < cut_start - 0.04 {
            let mut left = clip.clone();
            left.duration_sec = cut_start - c_start;
            updated.push(left);
        }
        if c_end > cut_end + 0.04 {
            let mut right = clip;
            right.id = format!("{}-tail", right.id);
            let delta = cut_end - c_start;
            right.timeline_start_sec = cut_end;
            right.source_in_sec += delta;
            right.duration_sec = c_end - cut_end;
            updated.push(right);
        }
    }
    *clips = updated;
}

fn sync_rendered_clip_entry(
    project: &mut MovieProject,
    job_id: &str,
    label: &str,
    prompt: &str,
    duration_seconds: f32,
    seed: u64,
    path: &str,
) {
    if let Some(existing) = project.clips.iter_mut().find(|c| c.id == job_id) {
        existing.status = "complete".into();
        existing.path = path.to_string();
        existing.duration_seconds = duration_seconds;
        existing.prompt = prompt.to_string();
        existing.error.clear();
        return;
    }
    let idx = project.clips.len() as u32;
    project.clips.push(RenderedClip {
        id: job_id.to_string(),
        index: idx,
        title: label.to_string(),
        prompt: prompt.to_string(),
        duration_seconds,
        seed,
        status: "complete".into(),
        path: path.to_string(),
        error: String::new(),
        versions: Vec::new(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn h3_project_entity_staging_and_ladder_queue_work_end_to_end_without_gpu() {
        let root = tempdir().unwrap();
        let comfy = tempdir().unwrap();
        for rel in [
            "models/diffusion_models/minimax_h3_ref2va_pruned_int8_convrot.safetensors",
            "models/diffusion_models/minimax_h3_fl2va_pruned_int8_convrot.safetensors",
            "models/text_encoders/qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors",
            "models/vae/minimax_h3_video_vae_fp16.safetensors",
            "models/vae/minimax_h3_audio_vae_fp32.safetensors",
            "models/loras/minimax_h3_ref2v_turbo_4step_v0.1_comfyui_bf16.safetensors",
        ] {
            let p = comfy.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, b"stub").unwrap();
        }
        let comfy_str = comfy.path().to_string_lossy().into_owned();

        let studio = MovieStudio::new(root.path()).unwrap();
        let env = studio.environment_snapshot(&comfy_str);
        assert!(env.ready);
        assert!(env.has_ref2v_turbo_lora);

        let project = studio
            .create_h3_project(
                CreateMovieH3ProjectRequest {
                    title: "Desert Nocturne".into(),
                    story_markdown: "A lone rider crosses the canyon at dusk.".into(),
                    width: Some(1920),
                    height: Some(1088),
                    duration_sec: Some(12.0),
                },
                &comfy_str,
                false,
                None,
            )
            .unwrap();
        let scene_id = project.active_scene_id.clone();

        let sec_receipt = studio
            .upsert_h3_prompt_section(
                UpsertMovieH3PromptSectionRequest {
                    project_id: project.id.clone(),
                    scene_id: scene_id.clone(),
                    section: H3PromptSection {
                        id: "shot-1".into(),
                        label: "Opening".into(),
                        timeline_start_sec: 0.0,
                        duration_sec: 5.0,
                        summary_beat: "Rider halts at canyon rim.".into(),
                        detailed_action: "Wide tracking shot across red sandstone.".into(),
                        camera_and_lighting: "Golden hour backlight".into(),
                        soundscape_override: "Desert wind".into(),
                        music_override: String::new(),
                        vocal_events: Vec::new(),
                        swap_fields: None,
                        enabled: true,
                    },
                    global_prompt: None,
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(sec_receipt.shot_count, 1);

        // Partial scene update must preserve prompt_sections when omitted!
        let saved = studio
            .save_h3_scene(
                SaveMovieH3SceneRequest {
                    project_id: project.id.clone(),
                    scene: H3Scene {
                        id: scene_id.clone(),
                        name: "Scene 1 — Updated".into(),
                        kind: H3SceneKind::Standard,
                        width: 1920,
                        height: 1088,
                        fps: 24,
                        duration_sec: 12.0,
                        window: H3WindowConfig {
                            in_sec: 0.0,
                            out_sec: 5.0,
                            ..Default::default()
                        },
                        video_lanes: Vec::new(),
                        audio_lanes: Vec::new(),
                        driver_lanes: Vec::new(),
                        reference_lanes: Vec::new(),
                        prompt_sections: Vec::new(),
                        guides: Vec::new(),
                        global_prompt: H3GlobalPrompt::default(),
                        generation: H3GenerationSettings::default(),
                        created_at: String::new(),
                        updated_at: String::new(),
                    },
                    set_active: true,
                    replace_empty_collections: false,
                },
                false,
                None,
            )
            .await
            .unwrap();
        assert_eq!(saved.scenes[0].prompt_sections.len(), 1);

        // Queue 3-pass quality ladder!
        let start_receipt = studio
            .prepare_h3_generation(
                H3StartGenerationRequest {
                    project_id: project.id.clone(),
                    windows: Vec::new(),
                    expand_quality_ladder: true,
                },
                &comfy_str,
                false,
                None,
            )
            .await
            .unwrap();
        assert_eq!(start_receipt.queued_count, 3);
        let reloaded = studio.get(&project.id).unwrap();
        assert_eq!(reloaded.render_jobs[0].pass_preset, H3PassPreset::Draft02Mp);
        assert_eq!(reloaded.render_jobs[1].pass_preset, H3PassPreset::Detail1Mp);
        assert_eq!(
            reloaded.render_jobs[2].pass_preset,
            H3PassPreset::Upscale1080p
        );
    }
}
