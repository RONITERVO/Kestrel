//! Deterministic, `< 4 KB` `H3ProductionDigest` snapshot for 64k-context Producer LLM Agents
//! and fast UI workspace orientation.

use super::{h3_prompt::compile_h3_window, MovieProject, MovieReferenceAsset, StudioError};
use kestrel_app_core::{
    H3AdvisorySeverity, H3EntityDigest, H3EnvironmentSnapshot, H3JobDigest, H3PassPreset,
    H3ProductionDigest, H3ReferencePopulation, H3RenderJobStatus, H3Scene, H3SceneDigest,
    H3SceneKind, H3TimeSpan,
};

const MAX_STORY_EXCERPT_CHARS: usize = 420;
const MAX_RECENT_JOBS: usize = 8;

pub fn compute_video_coverage(scene: &H3Scene) -> (Vec<H3TimeSpan>, Vec<H3TimeSpan>) {
    let total = scene.duration_sec.max(0.0);
    let any_solo = scene.video_lanes.iter().any(|l| l.solo && !l.muted);
    let mut intervals: Vec<(f64, f64)> = Vec::new();
    for lane in &scene.video_lanes {
        if lane.muted || (any_solo && !lane.solo) {
            continue;
        }
        for clip in &lane.clips {
            if !clip.enabled || clip.duration_sec <= 0.02 || clip.opacity <= 0.001 {
                continue;
            }
            let s = clip.timeline_start_sec.max(0.0);
            let e = (clip.timeline_start_sec + clip.duration_sec).min(total.max(s));
            if e > s + 0.02 {
                intervals.push((s, e));
            }
        }
    }
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (s, e) in intervals {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 + 0.05 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }

    let covered: Vec<H3TimeSpan> = merged
        .iter()
        .map(|&(s, e)| H3TimeSpan {
            start_sec: round2(s),
            end_sec: round2(e),
        })
        .collect();

    let mut gaps = Vec::new();
    if scene.kind != H3SceneKind::Soundtrack && total > 0.1 {
        let mut cursor = 0.0_f64;
        for &(s, e) in &merged {
            if s > cursor + 0.08 {
                gaps.push(H3TimeSpan {
                    start_sec: round2(cursor),
                    end_sec: round2(s),
                });
            }
            cursor = cursor.max(e);
        }
        if total > cursor + 0.08 {
            gaps.push(H3TimeSpan {
                start_sec: round2(cursor),
                end_sec: round2(total),
            });
        }
    }

    (covered, gaps)
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

pub fn build_production_digest<F>(
    project: &MovieProject,
    reference_asset_count: usize,
    env: &H3EnvironmentSnapshot,
    advanced: bool,
    resolve_asset: F,
) -> H3ProductionDigest
where
    F: Fn(&str) -> Result<MovieReferenceAsset, StudioError>,
{
    let story_excerpt = if project.story_markdown.trim().is_empty() {
        truncate_chars(project.prompt.trim(), MAX_STORY_EXCERPT_CHARS)
    } else {
        truncate_chars(project.story_markdown.trim(), MAX_STORY_EXCERPT_CHARS)
    };

    let entities: Vec<H3EntityDigest> = project
        .entities
        .iter()
        .take(24)
        .map(|ent| H3EntityDigest {
            id: ent.id.clone(),
            handle: format!("@{}", ent.handle),
            kind: ent.kind,
            picture_count: ent
                .members
                .iter()
                .filter(|m| m.population == H3ReferencePopulation::Pictures)
                .count(),
            video_count: ent
                .members
                .iter()
                .filter(|m| m.population == H3ReferencePopulation::Videos)
                .count(),
            audio_count: ent
                .members
                .iter()
                .filter(|m| m.population == H3ReferencePopulation::StandaloneAudios || m.use_embedded_audio)
                .count(),
            has_description: !ent.subject_description.trim().is_empty(),
        })
        .collect();

    let mut scenes: Vec<H3SceneDigest> = Vec::with_capacity(project.scenes.len());
    for sc in project.scenes.iter().take(12) {
        let (covered, gaps) = compute_video_coverage(sc);
        let (err_cnt, warn_cnt) = match compile_h3_window(
            project,
            sc,
            None,
            None,
            env,
            advanced,
            &resolve_asset,
        ) {
            Ok(compiled) => {
                let errs = compiled
                    .advisories
                    .iter()
                    .filter(|a| a.severity == H3AdvisorySeverity::Error)
                    .count();
                let warns = compiled
                    .advisories
                    .iter()
                    .filter(|a| a.severity == H3AdvisorySeverity::Warning)
                    .count();
                (errs, warns)
            }
            Err(_) => (1, 0),
        };
        let video_clip_count: usize = sc.video_lanes.iter().map(|l| l.clips.len()).sum();
        let audio_clip_count: usize = sc.audio_lanes.iter().map(|l| l.clips.len()).sum();
        let staged_reference_count: usize = sc.reference_lanes.iter().map(|l| l.items.len()).sum();

        scenes.push(H3SceneDigest {
            id: sc.id.clone(),
            name: sc.name.clone(),
            kind: sc.kind,
            width: sc.width,
            height: sc.height,
            duration_sec: round2(sc.duration_sec),
            window_in_sec: round2(sc.window.in_sec),
            window_out_sec: round2(sc.window.out_sec),
            pass_preset: sc.generation.pass_preset,
            task_mode: sc.generation.task_mode,
            video_clip_count,
            audio_clip_count,
            staged_reference_count,
            guide_count: sc.guides.len(),
            prompt_section_count: sc.prompt_sections.len(),
            covered_video_spans: covered,
            uncovered_video_gaps: gaps,
            advisory_error_count: err_cnt,
            advisory_warning_count: warn_cnt,
        });
    }

    let recent_jobs: Vec<H3JobDigest> = project
        .render_jobs
        .iter()
        .rev()
        .take(MAX_RECENT_JOBS)
        .map(|job| H3JobDigest {
            id: job.id.clone(),
            scene_id: job.scene_id.clone(),
            label: truncate_chars(&job.label, 48),
            status: job.status,
            pass_preset: job.pass_preset,
            duration_sec: round2(job.geometry.trimmed_duration_sec),
            audio_peak_dbfs: job.quality.as_ref().map(|q| q.audio_peak_dbfs),
            verified: job.quality.as_ref().is_some_and(|q| q.verified),
            error: truncate_chars(&job.error, 120),
        })
        .collect();

    let mut next_actions = Vec::new();
    if !env.ready {
        next_actions.push(format!(
            "ComfyUI MiniMax H3 weights are incomplete: {}",
            truncate_chars(&env.detail, 140)
        ));
    }
    if project.story_markdown.trim().is_empty() && project.prompt.trim().is_empty() {
        next_actions.push(
            "Save a concise story outline with save_movie_h3_story so character, prop, and shot continuity stay anchored."
                .into(),
        );
    }
    if entities.is_empty() {
        next_actions.push(
            "Create reusable @Handle entities (Characters, Locations, Props, Voices) via upsert_movie_h3_entity after generating portraits in Image Studio and voice clips in Speech Studio."
                .into(),
        );
    }

    let active_scene = scenes
        .iter()
        .find(|s| s.id == project.active_scene_id)
        .or_else(|| scenes.first());

    if let Some(sc) = active_scene {
        if sc.prompt_section_count == 0 {
            next_actions.push(format!(
                "Add timed shot sections and <d>[Language] ...</d> dialogue to scene '{}' ({}) via upsert_movie_h3_prompt_section.",
                sc.name, sc.id
            ));
        }
        if !entities.is_empty() && sc.staged_reference_count == 0 && sc.kind != H3SceneKind::Soundtrack {
            next_actions.push(format!(
                "Stage @Handle entities across shot spans in scene '{}' ({}) with stage_movie_h3_reference.",
                sc.name, sc.id
            ));
        }
        if let Some(first_gap) = sc.uncovered_video_gaps.first() {
            let gap_end = (first_gap.start_sec + 6.0).min(first_gap.end_sec);
            next_actions.push(format!(
                "Compile & render uncovered span [{:.2}s..{:.2}s] in scene '{}' ({}) using compile_movie_h3_window then start_movie_h3_generation (set expandQualityLadder: true for 0.2MP -> 1MP -> 1080p).",
                first_gap.start_sec, gap_end, sc.name, sc.id
            ));
        } else if sc.kind == H3SceneKind::Standard {
            let has_hi_res = project.render_jobs.iter().any(|j| {
                j.scene_id == sc.id
                    && j.status == H3RenderJobStatus::Complete
                    && matches!(j.pass_preset, H3PassPreset::Detail1Mp | H3PassPreset::Upscale1080p)
            });
            if !has_hi_res && sc.video_clip_count > 0 {
                next_actions.push(format!(
                    "Draft coverage is complete for '{}'; refine windows with Detail1Mp (denoise 0.75) -> Upscale1080p (denoise 0.40) and layer a separate 64x32 Soundtrack or Music Studio score on A2.",
                    sc.name
                ));
            } else if project.exports.is_empty() {
                next_actions.push(format!(
                    "All spans in '{}' are covered; inspect takes with inspect_movie_h3_take and export final MP4 via render_movie_edit.",
                    sc.name
                ));
            }
        }
    }
    next_actions.truncate(4);

    let final_export_ready = !project.final_path.is_empty()
        && std::path::Path::new(&project.final_path).is_file();

    H3ProductionDigest {
        project_id: project.id.clone(),
        title: project.title.clone(),
        status: project.status.clone(),
        phase: project.phase.clone(),
        active_scene_id: project.active_scene_id.clone(),
        story_excerpt,
        reference_asset_count,
        entities,
        scenes,
        recent_jobs,
        export_count: project.exports.len(),
        final_export_ready,
        next_actions,
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in s.chars().enumerate() {
        if idx >= max {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestrel_app_core::{
        H3AudioIntent, H3EntityKind, H3GenerationSettings, H3GlobalPrompt, H3ReferenceEntity,
        H3RetentionLevel, H3TakePlacementMode, H3TaskMode, H3VideoClip, H3VideoLane,
        H3VisualIntent, H3WindowConfig, MovieEdit, MovieSettings,
    };
    use serde_json::Value;

    #[test]
    fn digest_computes_exact_gaps_and_stays_under_4kb_budget() {
        let env = H3EnvironmentSnapshot {
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
            discovered_loras: Vec::new(),
            detail: "Ready".into(),
        };

        let entities: Vec<H3ReferenceEntity> = (0..8)
            .map(|i| H3ReferenceEntity {
                id: format!("ent-{i}"),
                handle: format!("Actor_{i}"),
                display_name: format!("Actor {i}"),
                kind: H3EntityKind::Character,
                subject_description: "Detailed look.".into(),
                retention_level: H3RetentionLevel::FullyPreserved,
                default_visual_intent: H3VisualIntent::Preserve,
                default_audio_intent: H3AudioIntent::VoiceIdentity,
                members: Vec::new(),
                created_at: String::new(),
                updated_at: String::new(),
            })
            .collect();

        let scenes: Vec<H3Scene> = (0..3)
            .map(|i| H3Scene {
                id: format!("scene-{i}"),
                name: format!("Act {}", i + 1),
                kind: H3SceneKind::Standard,
                width: 1920,
                height: 1088,
                fps: 24,
                duration_sec: 15.0,
                window: H3WindowConfig::default(),
                generation: H3GenerationSettings {
                    placement_mode: H3TakePlacementMode::Trimmed,
                    task_mode: H3TaskMode::Standard,
                    ..Default::default()
                },
                global_prompt: H3GlobalPrompt::default(),
                prompt_sections: Vec::new(),
                video_lanes: vec![H3VideoLane {
                    id: "v1".into(),
                    name: "V1".into(),
                    muted: false,
                    solo: false,
                    locked: false,
                    clips: vec![H3VideoClip {
                        id: "c1".into(),
                        source_kind: kestrel_app_core::H3ClipSourceKind::Take,
                        source_id: "job-1".into(),
                        label: "Shot 1".into(),
                        timeline_start_sec: 0.0,
                        duration_sec: 5.0,
                        source_in_sec: 0.0,
                        source_duration_sec: 5.0,
                        opacity: 1.0,
                        audio_gain: 1.0,
                        include_embedded_audio: true,
                        enabled: true,
                    }],
                }],
                audio_lanes: Vec::new(),
                driver_lanes: Vec::new(),
                reference_lanes: Vec::new(),
                guides: Vec::new(),
                created_at: String::new(),
                updated_at: String::new(),
            })
            .collect();

        let project = MovieProject {
            schema_version: 9,
            id: "proj-digest".into(),
            prompt: String::new(),
            title: "Frontier Showcase".into(),
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
                export_title: "Frontier".into(),
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
            story_markdown: "x".repeat(8_000),
            story_revisions: Vec::new(),
            entities,
            scenes,
            active_scene_id: "scene-0".into(),
            render_jobs: Vec::new(),
            legacy_producer_feedback: Vec::new(),
            legacy_copilot_history: Vec::new(),
        };

        let digest = build_production_digest(&project, 12, &env, false, |id| {
            Err(StudioError::NotFound(id.into()))
        });
        assert_eq!(digest.scenes[0].covered_video_spans.len(), 1);
        assert_eq!(digest.scenes[0].uncovered_video_gaps.len(), 1);
        assert!((digest.scenes[0].uncovered_video_gaps[0].start_sec - 5.0).abs() < 1e-4);
        assert!((digest.scenes[0].uncovered_video_gaps[0].end_sec - 15.0).abs() < 1e-4);

        let serialized = serde_json::to_string(&digest).unwrap();
        assert!(
            serialized.len() < 4_096,
            "digest JSON must stay under 4 KB for 64k Producer Agent, got {} bytes",
            serialized.len()
        );
    }
}
