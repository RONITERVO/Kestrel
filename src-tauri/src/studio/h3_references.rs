//! `@handle` entity normalization, timeline reference overlap & per-lane specificity resolution,
//! and exact ComfyUI `MiniMaxH3ReferenceToVideo` ordinal binding (`<Picture>`, `<Video>`, `<Audio>`, `<Subject>`).

use super::{h3_geometry::validate_normalized_crop, MovieReferenceAsset, MovieProject, StudioError};
use kestrel_app_core::{
    H3AdvisorySeverity, H3BoundOrdinalSlot, H3CompileAdvisory, H3CropRect, H3EntityKind,
    H3OrdinalManifest, H3ReferenceEntity, H3ReferenceItemVerdict, H3ReferenceLane,
    H3ReferencePopulation, H3ReferenceRole, H3ReferenceVerdictKind, H3RetentionLevel, H3Scene,
    H3StagedReferenceItem, H3SubjectBinding, H3TakePlacementGeometry, MovieH3Defaults,
};
use std::collections::{HashMap, HashSet};

pub const MAX_REFERENCE_SLICE_SEC: f64 = 15.0;

/// Normalizes a producer or agent `@handle` into canonical form without a leading `@`
/// (e.g. `"@Cowboy"` -> `"Cowboy"`).
pub fn normalize_handle(raw: &str) -> Result<String, StudioError> {
    let trimmed = raw.trim().trim_start_matches('@').trim();
    if trimmed.is_empty() || trimmed.len() > 48 {
        return Err(StudioError::Invalid(
            "Entity @handle must be 1 to 48 ASCII letters, digits, or underscores.".into(),
        ));
    }
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return Err(StudioError::Invalid("Entity @handle cannot be empty.".into()));
    };
    if !first.is_ascii_alphabetic() {
        return Err(StudioError::Invalid(format!(
            "Entity handle @{trimmed} must start with an ASCII letter."
        )));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(StudioError::Invalid(format!(
            "Entity handle @{trimmed} may contain only ASCII letters, digits, and underscores."
        )));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn is_handle_boundary_before(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(c) => !c.is_alphanumeric() && c != '_' && c != '@',
    }
}

/// Scans prose for `@Handle` mentions (ignoring email-like middle `@` tokens), returning canonical
/// handles without `@` in first-seen order (case-insensitively deduplicated).
pub fn extract_handles(text: &str) -> Vec<String> {
    let mut seen_lower = HashSet::new();
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (_, ch) = chars[i];
        let prev = if i > 0 { Some(chars[i - 1].1) } else { None };
        if ch == '@' && is_handle_boundary_before(prev) && i + 1 < chars.len() {
            let (_, first) = chars[i + 1];
            if first.is_ascii_alphabetic() {
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
                let candidate = &text[start_byte..end_byte];
                if candidate.len() <= 48 && seen_lower.insert(candidate.to_ascii_lowercase()) {
                    out.push(candidate.to_string());
                }
                i = end_idx;
                continue;
            }
        }
        i += 1;
    }
    out
}

pub fn infer_population_for_kind(kind: &str) -> H3ReferencePopulation {
    match kind {
        "video" => H3ReferencePopulation::Videos,
        "audio" => H3ReferencePopulation::StandaloneAudios,
        _ => H3ReferencePopulation::Pictures,
    }
}

pub fn default_role_for_entity(
    entity_kind: H3EntityKind,
    population: H3ReferencePopulation,
) -> H3ReferenceRole {
    match population {
        H3ReferencePopulation::StandaloneAudios => match entity_kind {
            H3EntityKind::Sound | H3EntityKind::Location | H3EntityKind::Prop => {
                H3ReferenceRole::Sound
            }
            _ => H3ReferenceRole::Voice,
        },
        H3ReferencePopulation::Videos => match entity_kind {
            H3EntityKind::Character => H3ReferenceRole::Subject,
            H3EntityKind::Location => H3ReferenceRole::Scene,
            _ => H3ReferenceRole::Motion,
        },
        H3ReferencePopulation::Pictures => match entity_kind {
            H3EntityKind::Character | H3EntityKind::Voice => H3ReferenceRole::Subject,
            H3EntityKind::Outfit => H3ReferenceRole::Outfit,
            H3EntityKind::Location => H3ReferenceRole::Scene,
            _ => H3ReferenceRole::General,
        },
    }
}

pub fn normalize_member_role(
    entity_kind: H3EntityKind,
    population: H3ReferencePopulation,
    role: H3ReferenceRole,
) -> H3ReferenceRole {
    match population {
        H3ReferencePopulation::StandaloneAudios => {
            if matches!(role, H3ReferenceRole::Voice | H3ReferenceRole::Sound) {
                if entity_kind == H3EntityKind::Sound && role == H3ReferenceRole::Voice {
                    H3ReferenceRole::Sound
                } else {
                    role
                }
            } else {
                default_role_for_entity(entity_kind, population)
            }
        }
        H3ReferencePopulation::Pictures | H3ReferencePopulation::Videos => {
            if matches!(role, H3ReferenceRole::Voice | H3ReferenceRole::Sound)
                || (!matches!(entity_kind, H3EntityKind::Character | H3EntityKind::Voice)
                    && role == H3ReferenceRole::Subject)
            {
                default_role_for_entity(entity_kind, population)
            } else {
                role
            }
        }
    }
}

fn entity_uses_subject_slot(kind: H3EntityKind, roles: &[H3ReferenceRole]) -> bool {
    matches!(kind, H3EntityKind::Character | H3EntityKind::Voice)
        || roles.iter().any(|r| {
            matches!(
                r,
                H3ReferenceRole::Subject | H3ReferenceRole::Outfit | H3ReferenceRole::Voice
            )
        })
}

#[derive(Clone)]
struct CandidateMember {
    item_idx: usize,
    item_id: String,
    lane_idx: usize,
    item_duration: f64,
    item_start: f64,
    entity_id: Option<String>,
    handle: Option<String>,
    member_id: Option<String>,
    asset_id: String,
    population: H3ReferencePopulation,
    role: H3ReferenceRole,
    retention: H3RetentionLevel,
    label: String,
    description: String,
    has_embedded_audio: bool,
    use_embedded_audio: bool,
    crop: Option<H3CropRect>,
    source_start_sec: f64,
    source_end_sec: f64,
}

/// Resolves all staged reference items on `scene.reference_lanes` against `geometry` and `window`,
/// applying overlap filtering, tightest-span specificity overrides, H3 population caps (`9` pictures,
/// `3` videos, `3` total audios), and exact `MiniMaxH3ReferenceToVideo` `<Audio>` ordering.
pub fn resolve_window_references<F>(
    project: &MovieProject,
    scene: &H3Scene,
    min_overlap_ratio: f64,
    geometry: &H3TakePlacementGeometry,
    resolve_asset: F,
) -> (
    H3OrdinalManifest,
    Vec<H3ReferenceItemVerdict>,
    Vec<H3CompileAdvisory>,
    HashMap<String, String>,
)
where
    F: Fn(&str) -> Result<MovieReferenceAsset, StudioError>,
{
    let defaults = MovieH3Defaults::default();
    let win_in = geometry.selection_in_sec;
    let win_out = geometry.selection_out_sec;
    let win_len = (win_out - win_in).max(1e-6);
    let threshold = min_overlap_ratio.clamp(0.01, 1.0);

    let entity_by_id: HashMap<&str, &H3ReferenceEntity> = project
        .entities
        .iter()
        .map(|e| (e.id.as_str(), e))
        .collect();

    let mut advisories = Vec::new();
    let mut verdicts: Vec<H3ReferenceItemVerdict> = Vec::new();
    let mut candidates: Vec<CandidateMember> = Vec::new();

    for (lane_idx, lane) in scene.reference_lanes.iter().enumerate() {
        for item in &lane.items {
            let entity = item
                .entity_id
                .as_deref()
                .and_then(|eid| entity_by_id.get(eid).copied());
            let handle = entity.map(|e| e.handle.clone());
            let fallback_label = if !item.label.trim().is_empty() {
                item.label.trim().to_string()
            } else if let Some(ent) = entity {
                format!("@{}", ent.handle)
            } else {
                item.id.clone()
            };

            if lane.muted || !item.enabled {
                verdicts.push(H3ReferenceItemVerdict {
                    item_id: item.id.clone(),
                    lane_id: lane.id.clone(),
                    label: fallback_label,
                    handle,
                    verdict: H3ReferenceVerdictKind::Muted,
                    overlap_ratio: 0.0,
                    assigned_tags: Vec::new(),
                    detail: if lane.muted {
                        format!("Lane {} is muted.", lane.name)
                    } else {
                        "Staged reference item is muted.".into()
                    },
                });
                continue;
            }

            let item_start = item.timeline_start_sec.max(0.0);
            let item_dur = item.duration_sec.max(0.0);
            let item_end = item_start + item_dur;
            let overlap = (item_end.min(win_out) - item_start.max(win_in)).max(0.0);
            if overlap <= 1e-4 {
                verdicts.push(H3ReferenceItemVerdict {
                    item_id: item.id.clone(),
                    lane_id: lane.id.clone(),
                    label: fallback_label,
                    handle,
                    verdict: H3ReferenceVerdictKind::Outside,
                    overlap_ratio: 0.0,
                    assigned_tags: Vec::new(),
                    detail: "Outside the active [In, Out] window.".into(),
                });
                continue;
            }

            let window_ratio = overlap / win_len;
            let self_ratio = if overlap >= 0.20 && item_dur > 0.0 {
                overlap / item_dur
            } else {
                0.0
            };
            let effective_ratio = window_ratio.max(self_ratio).min(1.0);
            if effective_ratio + 1e-6 < threshold {
                verdicts.push(H3ReferenceItemVerdict {
                    item_id: item.id.clone(),
                    lane_id: lane.id.clone(),
                    label: fallback_label,
                    handle,
                    verdict: H3ReferenceVerdictKind::BelowThreshold,
                    overlap_ratio: window_ratio,
                    assigned_tags: Vec::new(),
                    detail: format!(
                        "Window overlap {:.0}% is below the {:.0}% threshold.",
                        window_ratio * 100.0,
                        threshold * 100.0
                    ),
                });
                continue;
            }

            let verdict_idx = verdicts.len();
            verdicts.push(H3ReferenceItemVerdict {
                item_id: item.id.clone(),
                lane_id: lane.id.clone(),
                label: fallback_label.clone(),
                handle: handle.clone(),
                verdict: H3ReferenceVerdictKind::Winner,
                overlap_ratio: effective_ratio,
                assigned_tags: Vec::new(),
                detail: String::new(),
            });

            if let Some(ent) = entity {
                let selected_members: Vec<_> = if let Some(mid) =
                    item.member_id.as_deref().filter(|s| !s.is_empty())
                {
                    ent.members.iter().filter(|m| m.id == mid).collect()
                } else {
                    ent.members.iter().collect()
                };
                if selected_members.is_empty() {
                    if ent.members.is_empty() {
                        advisories.push(H3CompileAdvisory {
                            severity: H3AdvisorySeverity::Warning,
                            code: "entity_has_no_members".into(),
                            message: format!(
                                "@{} is staged in the active window, but has no media members attached.",
                                ent.handle
                            ),
                        });
                        verdicts[verdict_idx].verdict = H3ReferenceVerdictKind::BelowThreshold;
                        verdicts[verdict_idx].detail =
                            format!("@{} has no media members.", ent.handle);
                    } else {
                        advisories.push(H3CompileAdvisory {
                            severity: H3AdvisorySeverity::Warning,
                            code: "missing_entity_member".into(),
                            message: format!(
                                "@{} staged member '{}' was not found on the entity.",
                                ent.handle,
                                item.member_id.as_deref().unwrap_or_default()
                            ),
                        });
                        verdicts[verdict_idx].verdict = H3ReferenceVerdictKind::BelowThreshold;
                        verdicts[verdict_idx].detail =
                            format!("@{} member ID not found.", ent.handle);
                    }
                    continue;
                }

                let mut emitted_for_entity = 0_usize;
                let mut skipped_by_modality = 0_usize;
                for member in selected_members {
                    let asset = match resolve_asset(&member.asset_id) {
                        Ok(asset) => asset,
                        Err(err) => {
                            advisories.push(H3CompileAdvisory {
                                severity: H3AdvisorySeverity::Error,
                                code: "missing_reference_asset".into(),
                                message: format!("@{} member failed to load: {err}", ent.handle),
                            });
                            continue;
                        }
                    };
                    let population = infer_population_for_kind(&asset.kind);
                    match population {
                        H3ReferencePopulation::Pictures => {
                            if !item.use_visual {
                                skipped_by_modality += 1;
                                continue;
                            }
                        }
                        H3ReferencePopulation::Videos => {
                            if !item.use_visual {
                                skipped_by_modality += 1;
                                if item.use_audio && member.use_embedded_audio {
                                    advisories.push(H3CompileAdvisory {
                                        severity: H3AdvisorySeverity::Warning,
                                        code: "video_audio_requires_visual".into(),
                                        message: format!(
                                            "@{} video member '{}' cannot bind embedded audio without visual motion in MiniMax H3; extract or attach standalone audio instead.",
                                            ent.handle, member.label
                                        ),
                                    });
                                }
                                continue;
                            }
                        }
                        H3ReferencePopulation::StandaloneAudios => {
                            if !item.use_audio {
                                skipped_by_modality += 1;
                                continue;
                            }
                        }
                    }

                    let use_embedded_audio = population == H3ReferencePopulation::Videos
                        && item.use_audio
                        && member.use_embedded_audio
                        && asset.has_audio;
                    if population == H3ReferencePopulation::Videos
                        && item.use_audio
                        && member.use_embedded_audio
                        && !asset.has_audio
                    {
                        advisories.push(H3CompileAdvisory {
                            severity: H3AdvisorySeverity::Warning,
                            code: "silent_video_reference".into(),
                            message: format!(
                                "@{} video member '{}' requested embedded audio, but {} has no audio stream.",
                                ent.handle, member.label, asset.name
                            ),
                        });
                    }
                    let role = item.role_override.unwrap_or_else(|| {
                        normalize_member_role(ent.kind, population, member.role)
                    });
                    let retention = item.retention_override.unwrap_or(ent.retention_level);
                    let desc = if !item.description_override.trim().is_empty() {
                        item.description_override.trim().to_string()
                    } else if !member.description_override.trim().is_empty() {
                        member.description_override.trim().to_string()
                    } else if !ent.subject_description.trim().is_empty() {
                        ent.subject_description.trim().to_string()
                    } else {
                        asset.name.clone()
                    };
                    let crop = member
                        .crop
                        .as_ref()
                        .and_then(|c| validate_normalized_crop(c, &ent.handle).ok());
                    let (src_start, src_end) = normalize_member_slice(
                        member.source_start_sec,
                        member.source_end_sec,
                        asset.duration_seconds,
                    );
                    emitted_for_entity += 1;
                    candidates.push(CandidateMember {
                        item_idx: verdict_idx,
                        item_id: item.id.clone(),
                        lane_idx,
                        item_duration: item_dur,
                        item_start,
                        entity_id: Some(ent.id.clone()),
                        handle: Some(ent.handle.clone()),
                        member_id: Some(member.id.clone()),
                        asset_id: asset.id.clone(),
                        population,
                        role,
                        retention,
                        label: if member.label.trim().is_empty() {
                            format!("@{}", ent.handle)
                        } else {
                            format!("@{} · {}", ent.handle, member.label.trim())
                        },
                        description: desc,
                        has_embedded_audio: asset.has_audio,
                        use_embedded_audio,
                        crop,
                        source_start_sec: src_start,
                        source_end_sec: src_end,
                    });
                }
                if emitted_for_entity == 0 {
                    if skipped_by_modality > 0 {
                        verdicts[verdict_idx].verdict = H3ReferenceVerdictKind::Muted;
                        verdicts[verdict_idx].detail =
                            "All entity members were disabled by visual/audio modality toggles."
                                .into();
                    } else {
                        verdicts[verdict_idx].verdict = H3ReferenceVerdictKind::BelowThreshold;
                        verdicts[verdict_idx].detail =
                            "No valid media assets could be resolved for entity members.".into();
                    }
                    continue;
                }
            } else if let Some(aid) = item.asset_id.as_deref().filter(|s| !s.is_empty()) {
                let asset = match resolve_asset(aid) {
                    Ok(asset) => asset,
                    Err(err) => {
                        advisories.push(H3CompileAdvisory {
                            severity: H3AdvisorySeverity::Error,
                            code: "missing_reference_asset".into(),
                            message: format!("Staged asset {aid} failed to load: {err}"),
                        });
                        verdicts[verdict_idx].verdict = H3ReferenceVerdictKind::BelowThreshold;
                        verdicts[verdict_idx].detail = err.to_string();
                        continue;
                    }
                };
                let population = infer_population_for_kind(&asset.kind);
                if (matches!(
                    population,
                    H3ReferencePopulation::Pictures | H3ReferencePopulation::Videos
                ) && !item.use_visual)
                    || (population == H3ReferencePopulation::StandaloneAudios && !item.use_audio)
                {
                    verdicts[verdict_idx].verdict = H3ReferenceVerdictKind::Muted;
                    verdicts[verdict_idx].detail =
                        "Modality toggle disabled for this asset.".into();
                    continue;
                }
                let role = item.role_override.unwrap_or(match population {
                    H3ReferencePopulation::Pictures => H3ReferenceRole::General,
                    H3ReferencePopulation::Videos => H3ReferenceRole::Motion,
                    H3ReferencePopulation::StandaloneAudios => H3ReferenceRole::Sound,
                });
                let retention = item
                    .retention_override
                    .unwrap_or(H3RetentionLevel::Reference);
                let desc = if !item.description_override.trim().is_empty() {
                    item.description_override.trim().to_string()
                } else {
                    project
                        .references
                        .iter()
                        .find(|r| r.asset_id == asset.id && !r.description.trim().is_empty())
                        .map(|r| r.description.trim().to_string())
                        .unwrap_or_else(|| asset.name.clone())
                };
                let (src_start, src_end) =
                    normalize_member_slice(0.0, asset.duration_seconds, asset.duration_seconds);
                candidates.push(CandidateMember {
                    item_idx: verdict_idx,
                    item_id: item.id.clone(),
                    lane_idx,
                    item_duration: item_dur,
                    item_start,
                    entity_id: None,
                    handle: None,
                    member_id: None,
                    asset_id: asset.id.clone(),
                    population,
                    role,
                    retention,
                    label: fallback_label,
                    description: desc,
                    has_embedded_audio: asset.has_audio,
                    use_embedded_audio: population == H3ReferencePopulation::Videos
                        && item.use_audio
                        && asset.has_audio,
                    crop: None,
                    source_start_sec: src_start,
                    source_end_sec: src_end,
                });
            } else {
                verdicts[verdict_idx].verdict = H3ReferenceVerdictKind::BelowThreshold;
                verdicts[verdict_idx].detail =
                    "Staged item references neither an @entity nor an asset.".into();
            }
        }
    }

    // Specificity override: when distinct staged items target the same (entity_id, population, role),
    // the item with the tightest timeline duration (and higher-priority / later lane on tie) wins.
    type SpecificityKey = (String, u8, u8);
    type SpecificityWinner = (usize, f64, usize, String);
    let mut best_item_for_key: HashMap<SpecificityKey, SpecificityWinner> = HashMap::new();
    for cand in &candidates {
        let Some(eid) = &cand.entity_id else {
            continue;
        };
        let key = (eid.clone(), pop_discriminant(cand.population), role_discriminant(cand.role));
        match best_item_for_key.get(&key) {
            None => {
                best_item_for_key.insert(
                    key,
                    (
                        cand.item_idx,
                        cand.item_duration,
                        cand.lane_idx,
                        cand.item_id.clone(),
                    ),
                );
            }
            Some((_, best_dur, best_lane, best_item_id)) => {
                if *best_item_id == cand.item_id {
                    continue;
                }
                let tighter = cand.item_duration + 1e-4 < *best_dur
                    || ((cand.item_duration - *best_dur).abs() <= 1e-4
                        && cand.lane_idx >= *best_lane);
                if tighter {
                    best_item_for_key.insert(
                        key,
                        (
                            cand.item_idx,
                            cand.item_duration,
                            cand.lane_idx,
                            cand.item_id.clone(),
                        ),
                    );
                }
            }
        }
    }

    let mut active_candidates = Vec::new();
    let mut item_had_winner = HashSet::new();
    let mut item_superseded_by: HashMap<usize, String> = HashMap::new();
    for cand in candidates {
        if let Some(eid) = &cand.entity_id {
            let key = (eid.clone(), pop_discriminant(cand.population), role_discriminant(cand.role));
            if let Some((winner_verdict_idx, _, _, winner_item_id)) = best_item_for_key.get(&key) {
                if *winner_item_id != cand.item_id {
                    let winner_label = verdicts[*winner_verdict_idx].label.clone();
                    item_superseded_by.insert(cand.item_idx, winner_label);
                    continue;
                }
            }
        }
        item_had_winner.insert(cand.item_idx);
        active_candidates.push(cand);
    }

    for (sup_idx, by_label) in &item_superseded_by {
        if !item_had_winner.contains(sup_idx) {
            verdicts[*sup_idx].verdict = H3ReferenceVerdictKind::Superseded;
            verdicts[*sup_idx].detail =
                format!("Superseded by tighter-span override ({by_label}).");
        }
    }

    // Deterministic ordering: lane order, then timeline_start_sec, then item_id.
    active_candidates.sort_by(|a, b| {
        a.lane_idx
            .cmp(&b.lane_idx)
            .then_with(|| {
                a.item_start
                    .partial_cmp(&b.item_start)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.item_id.cmp(&b.item_id))
    });

    // Enforce H3 caps (9 pictures, 3 videos, 3 total audios) and deduplicate identical member slices.
    let mut pictures: Vec<(usize, H3BoundOrdinalSlot)> = Vec::new();
    let mut videos: Vec<(usize, H3BoundOrdinalSlot)> = Vec::new();
    let mut standalone_audios: Vec<(usize, H3BoundOrdinalSlot)> = Vec::new();
    let mut seen_dedup: HashMap<String, usize> = HashMap::new();
    let mut dedup_aliases: Vec<(usize, usize)> = Vec::new();
    let mut embedded_audio_count = 0_usize;

    for mut cand in active_candidates {
        let dedup_key = format!(
            "{}:{}:{:.3}:{:.3}:{:?}",
            pop_discriminant(cand.population),
            cand.asset_id,
            cand.source_start_sec,
            cand.source_end_sec,
            cand.crop
        );
        if let Some(&first_idx) = seen_dedup.get(&dedup_key) {
            if first_idx != cand.item_idx {
                dedup_aliases.push((cand.item_idx, first_idx));
            }
            continue;
        }
        seen_dedup.insert(dedup_key, cand.item_idx);

        match cand.population {
            H3ReferencePopulation::Pictures => {
                if pictures.len() >= defaults.max_picture_refs {
                    mark_cap_exceeded(
                        &mut verdicts,
                        &mut advisories,
                        cand.item_idx,
                        &cand.label,
                        "9 pictures",
                    );
                    continue;
                }
                let ordinal = (pictures.len() + 1) as u32;
                let tag = format!("<Picture {ordinal}>");
                verdicts[cand.item_idx].assigned_tags.push(tag.clone());
                pictures.push((cand.item_idx, to_slot(ordinal, tag, cand)));
            }
            H3ReferencePopulation::Videos => {
                if videos.len() >= defaults.max_video_refs {
                    mark_cap_exceeded(
                        &mut verdicts,
                        &mut advisories,
                        cand.item_idx,
                        &cand.label,
                        "3 videos",
                    );
                    continue;
                }
                if cand.use_embedded_audio
                    && (embedded_audio_count + standalone_audios.len()) >= defaults.max_audio_refs
                {
                    cand.use_embedded_audio = false;
                    advisories.push(H3CompileAdvisory {
                        severity: H3AdvisorySeverity::Warning,
                        code: "audio_cap_dropped_embedded".into(),
                        message: format!(
                            "{} dropped embedded audio because the 3-audio H3 cap is already full.",
                            cand.label
                        ),
                    });
                }
                if cand.use_embedded_audio {
                    embedded_audio_count += 1;
                }
                let ordinal = (videos.len() + 1) as u32;
                let tag = format!("<Video {ordinal}>");
                verdicts[cand.item_idx].assigned_tags.push(tag.clone());
                videos.push((cand.item_idx, to_slot(ordinal, tag, cand)));
            }
            H3ReferencePopulation::StandaloneAudios => {
                if embedded_audio_count + standalone_audios.len() >= defaults.max_audio_refs {
                    mark_cap_exceeded(
                        &mut verdicts,
                        &mut advisories,
                        cand.item_idx,
                        &cand.label,
                        "3 audio signals (including embedded video audio)",
                    );
                    continue;
                }
                // Placeholder ordinal; finalized below after embedded video audios are numbered 1..M.
                standalone_audios.push((cand.item_idx, to_slot(0, String::new(), cand)));
            }
        }
    }

    // Finalize exact MiniMaxH3ReferenceToVideo `<Audio 1..A>` numbering:
    // 1. Embedded video audios in `<Video 1..V>` order -> `<Audio 1..M>`
    // 2. Standalone audios -> `<Audio M+1..A>`
    let mut audios: Vec<H3BoundOrdinalSlot> = Vec::new();
    for (v_item_idx, video_slot) in &videos {
        if video_slot.use_embedded_audio && video_slot.has_embedded_audio {
            let audio_ord = (audios.len() + 1) as u32;
            let audio_tag = format!("<Audio {audio_ord}>");
            verdicts[*v_item_idx].assigned_tags.push(audio_tag.clone());
            let mut audio_slot = video_slot.clone();
            audio_slot.ordinal = audio_ord;
            audio_slot.tag = audio_tag;
            audio_slot.role = if video_slot.role == H3ReferenceRole::Subject {
                H3ReferenceRole::Voice
            } else {
                H3ReferenceRole::Sound
            };
            audios.push(audio_slot);
        }
    }
    for (a_item_idx, mut audio_slot) in standalone_audios {
        let audio_ord = (audios.len() + 1) as u32;
        let audio_tag = format!("<Audio {audio_ord}>");
        audio_slot.ordinal = audio_ord;
        audio_slot.tag = audio_tag.clone();
        verdicts[a_item_idx].assigned_tags.push(audio_tag);
        audios.push(audio_slot);
    }

    for (dup_idx, first_idx) in dedup_aliases {
        let shared = verdicts[first_idx].assigned_tags.clone();
        for tag in shared {
            if !verdicts[dup_idx].assigned_tags.contains(&tag) {
                verdicts[dup_idx].assigned_tags.push(tag);
            }
        }
    }

    let mut picture_slots: Vec<H3BoundOrdinalSlot> =
        pictures.into_iter().map(|(_, s)| s).collect();
    let mut video_slots: Vec<H3BoundOrdinalSlot> = videos.into_iter().map(|(_, s)| s).collect();

    // Group bound slots by entity_id in first-bound order to assign `<Subject 1..S>` and `(S1..SS)`
    // AND build `handle_expansion_map` (lowercase handle -> `<Subject i>` or `<Picture p>` / etc.).
    let mut entity_order: Vec<String> = Vec::new();
    let mut entity_roles: HashMap<String, Vec<H3ReferenceRole>> = HashMap::new();
    for slot in picture_slots
        .iter()
        .chain(video_slots.iter())
        .chain(audios.iter())
    {
        if let Some(eid) = &slot.entity_id {
            if !entity_order.contains(eid) {
                entity_order.push(eid.clone());
            }
            entity_roles.entry(eid.clone()).or_default().push(slot.role);
        }
    }

    let mut subjects: Vec<H3SubjectBinding> = Vec::new();
    let mut subject_by_entity: HashMap<String, (String, String)> = HashMap::new();
    let mut handle_expansion_map: HashMap<String, String> = HashMap::new();

    for eid in &entity_order {
        let Some(ent) = entity_by_id.get(eid.as_str()).copied() else {
            continue;
        };
        let roles = entity_roles
            .get(eid)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut visual_tags = Vec::new();
        let mut audio_tags = Vec::new();
        let mut retention = ent.retention_level;
        let mut effective_description = ent.subject_description.trim().to_string();

        for slot in picture_slots.iter().chain(video_slots.iter()) {
            if slot.entity_id.as_deref() == Some(eid.as_str()) {
                visual_tags.push(slot.tag.clone());
                retention = slot.retention;
                if !slot.description.trim().is_empty()
                    && (effective_description.is_empty()
                        || slot.description.trim() != ent.subject_description.trim())
                {
                    effective_description = slot.description.trim().to_string();
                }
            }
        }
        for slot in &audios {
            if slot.entity_id.as_deref() == Some(eid.as_str()) {
                audio_tags.push(slot.tag.clone());
                if visual_tags.is_empty() {
                    retention = slot.retention;
                }
                if effective_description.is_empty() && !slot.description.trim().is_empty() {
                    effective_description = slot.description.trim().to_string();
                }
            }
        }

        if entity_uses_subject_slot(ent.kind, roles) {
            let subj_idx = (subjects.len() + 1) as u32;
            let subj_tag = format!("<Subject {subj_idx}>");
            let speaker_code = format!("(S{subj_idx})");
            subject_by_entity.insert(eid.clone(), (subj_tag.clone(), speaker_code.clone()));
            handle_expansion_map.insert(ent.handle.to_ascii_lowercase(), subj_tag.clone());
            subjects.push(H3SubjectBinding {
                subject_index: subj_idx,
                subject_tag: subj_tag,
                speaker_code,
                handle: ent.handle.clone(),
                display_name: if ent.display_name.trim().is_empty() {
                    ent.handle.clone()
                } else {
                    ent.display_name.trim().to_string()
                },
                entity_id: ent.id.clone(),
                visual_tags,
                audio_tags,
                subject_description: effective_description,
                retention,
                shot_indices: Vec::new(),
            });
        } else if let Some(primary_tag) = visual_tags.first().or_else(|| audio_tags.first()) {
            handle_expansion_map.insert(ent.handle.to_ascii_lowercase(), primary_tag.clone());
        }
    }

    // Stamp `subject_tag` & `speaker_code` onto every bound slot and append `<Subject N>` to verdicts.
    for slot in picture_slots
        .iter_mut()
        .chain(video_slots.iter_mut())
        .chain(audios.iter_mut())
    {
        if let Some(eid) = slot.entity_id.as_deref() {
            if let Some((subj_tag, speaker_code)) = subject_by_entity.get(eid) {
                slot.subject_tag = Some(subj_tag.clone());
                if slot.population == H3ReferencePopulation::StandaloneAudios
                    || (slot.population == H3ReferencePopulation::Videos && slot.use_embedded_audio)
                {
                    slot.speaker_code = Some(speaker_code.clone());
                }
            }
        }
    }

    for verdict in &mut verdicts {
        if verdict.verdict == H3ReferenceVerdictKind::Winner {
            if verdict.assigned_tags.is_empty() {
                verdict.verdict = H3ReferenceVerdictKind::CapExceeded;
                if verdict.detail.is_empty() {
                    verdict.detail = "No active slot remained after H3 reference caps.".into();
                }
            } else {
                if let Some(h) = verdict.handle.as_deref() {
                    if let Some(mapped) = handle_expansion_map.get(&h.to_ascii_lowercase()) {
                        if mapped.starts_with("<Subject ")
                            && !verdict.assigned_tags.contains(mapped)
                        {
                            verdict.assigned_tags.insert(0, mapped.clone());
                        }
                    }
                }
                verdict.detail = format!("Bound as {}.", verdict.assigned_tags.join(", "));
            }
        }
    }

    let manifest = H3OrdinalManifest {
        pictures: picture_slots,
        videos: video_slots,
        audios,
        subjects,
    };

    (manifest, verdicts, advisories, handle_expansion_map)
}

fn normalize_member_slice(start_sec: f64, end_sec: f64, asset_duration: f64) -> (f64, f64) {
    if asset_duration <= 0.0 {
        return (0.0, 0.0);
    }
    let start = if start_sec.is_finite() && start_sec > 0.0 {
        start_sec.min((asset_duration - 0.1).max(0.0))
    } else {
        0.0
    };
    let raw_end = if end_sec.is_finite() && end_sec > start + 0.05 {
        end_sec.min(asset_duration)
    } else {
        asset_duration
    };
    let clamped_end = raw_end.min(start + MAX_REFERENCE_SLICE_SEC);
    (start, clamped_end)
}

fn pop_discriminant(pop: H3ReferencePopulation) -> u8 {
    match pop {
        H3ReferencePopulation::Pictures => 1,
        H3ReferencePopulation::Videos => 2,
        H3ReferencePopulation::StandaloneAudios => 3,
    }
}

fn role_discriminant(role: H3ReferenceRole) -> u8 {
    match role {
        H3ReferenceRole::Subject => 1,
        H3ReferenceRole::Outfit => 2,
        H3ReferenceRole::Scene => 3,
        H3ReferenceRole::Motion => 4,
        H3ReferenceRole::Camera => 5,
        H3ReferenceRole::Voice => 6,
        H3ReferenceRole::Sound => 7,
        H3ReferenceRole::General => 8,
    }
}

fn to_slot(ordinal: u32, tag: String, cand: CandidateMember) -> H3BoundOrdinalSlot {
    H3BoundOrdinalSlot {
        ordinal,
        tag,
        subject_tag: None,
        speaker_code: None,
        population: cand.population,
        role: cand.role,
        retention: cand.retention,
        entity_id: cand.entity_id,
        handle: cand.handle,
        member_id: cand.member_id,
        asset_id: cand.asset_id,
        label: cand.label,
        description: cand.description,
        has_embedded_audio: cand.has_embedded_audio,
        use_embedded_audio: cand.use_embedded_audio,
        crop: cand.crop,
        source_start_sec: cand.source_start_sec,
        source_end_sec: cand.source_end_sec,
    }
}

fn mark_cap_exceeded(
    verdicts: &mut [H3ReferenceItemVerdict],
    advisories: &mut Vec<H3CompileAdvisory>,
    item_idx: usize,
    label: &str,
    cap_name: &str,
) {
    advisories.push(H3CompileAdvisory {
        severity: H3AdvisorySeverity::Warning,
        code: "reference_cap_exceeded".into(),
        message: format!(
            "{label} exceeded MiniMax H3's per-window limit of {cap_name} and was omitted."
        ),
    });
    if verdicts[item_idx].assigned_tags.is_empty() {
        verdicts[item_idx].verdict = H3ReferenceVerdictKind::CapExceeded;
        verdicts[item_idx].detail = format!("Omitted: H3 cap of {cap_name} reached.");
    }
}

/// Places a `H3StagedReferenceItem` onto `preferred_lane_id` (if unlocked and non-colliding) or
/// the first unlocked non-colliding `H3ReferenceLane`, appending a new `R{k}` lane when needed.
pub fn auto_stage_reference(
    scene: &mut H3Scene,
    item: H3StagedReferenceItem,
    preferred_lane_id: Option<&str>,
) -> Result<(String, String), StudioError> {
    let start = item.timeline_start_sec;
    let end = start + item.duration_sec;

    let collides = |lane: &H3ReferenceLane| -> bool {
        lane.items.iter().any(|existing| {
            let ex_start = existing.timeline_start_sec;
            let ex_end = ex_start + existing.duration_sec;
            ex_end > start + 1e-4 && ex_start < end - 1e-4
        })
    };

    if let Some(pref) = preferred_lane_id.map(str::trim).filter(|id| !id.is_empty()) {
        if let Some(lane) = scene
            .reference_lanes
            .iter_mut()
            .find(|l| l.id.eq_ignore_ascii_case(pref) || l.name.eq_ignore_ascii_case(pref))
        {
            if lane.locked {
                return Err(StudioError::Invalid(format!(
                    "Reference lane {} is locked.",
                    lane.name
                )));
            }
            if !collides(lane) {
                let lane_id = lane.id.clone();
                let lane_name = lane.name.clone();
                lane.items.push(item);
                return Ok((lane_id, lane_name));
            }
        } else if let Some(num) = pref
            .strip_prefix('R')
            .or_else(|| pref.strip_prefix('r'))
            .or_else(|| pref.strip_prefix("ref-lane-"))
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|&n| (1..=32).contains(&n))
        {
            let lane_id = format!("ref-lane-{num}");
            let lane_name = format!("R{num}");
            scene.reference_lanes.push(H3ReferenceLane {
                id: lane_id.clone(),
                name: lane_name.clone(),
                muted: false,
                locked: false,
                items: vec![item],
            });
            return Ok((lane_id, lane_name));
        } else {
            return Err(StudioError::Invalid(format!(
                "Reference lane {pref} does not exist in scene."
            )));
        }
    }

    if let Some(lane) = scene
        .reference_lanes
        .iter_mut()
        .find(|l| !l.locked && !l.muted && !collides(l))
    {
        let lane_id = lane.id.clone();
        let lane_name = lane.name.clone();
        lane.items.push(item);
        return Ok((lane_id, lane_name));
    }

    let next_num = scene.reference_lanes.len() + 1;
    let lane_id = format!("ref-lane-{next_num}");
    let lane_name = format!("R{next_num}");
    scene.reference_lanes.push(H3ReferenceLane {
        id: lane_id.clone(),
        name: lane_name.clone(),
        muted: false,
        locked: false,
        items: vec![item],
    });
    Ok((lane_id, lane_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::h3_geometry::calculate_take_placement;
    use kestrel_app_core::{
        H3AudioIntent, H3GenerationSettings, H3GlobalPrompt, H3ReferenceMember, H3SceneKind,
        H3VisualIntent, H3WindowConfig, MovieEdit, MovieSettings,
    };
    use serde_json::Value;

    fn sample_asset(id_char: char, kind: &str, has_audio: bool) -> MovieReferenceAsset {
        let id = std::iter::repeat_n(id_char, 64).collect::<String>();
        MovieReferenceAsset {
            id,
            name: format!("asset-{id_char}"),
            kind: kind.into(),
            mime_type: "application/octet-stream".into(),
            bytes: 1024,
            duration_seconds: if kind == "image" { 0.0 } else { 4.0 },
            width: 1024,
            height: 1024,
            has_audio,
            path: format!("/tmp/asset-{id_char}"),
            created_at: String::new(),
            generation: None,
        }
    }

    #[test]
    fn extract_handles_ignores_emails_and_deduplicates() {
        let text = "At 00:00.000, @Cowboy enters @Saloon_Interior while test@example.com is ignored; @cowboy speaks to (@TavernKeeper).";
        assert_eq!(
            extract_handles(text),
            vec!["Cowboy", "Saloon_Interior", "TavernKeeper"]
        );
    }

    #[test]
    fn specificity_override_and_embedded_video_audio_ordering_match_minimax_h3() {
        let pic_default = sample_asset('a', "image", false);
        let pic_override = sample_asset('b', "image", false);
        let vid_motion = sample_asset('c', "video", true);
        let standalone_voice = sample_asset('d', "audio", true);
        let assets: HashMap<String, MovieReferenceAsset> = [
            pic_default.clone(),
            pic_override.clone(),
            vid_motion.clone(),
            standalone_voice.clone(),
        ]
        .into_iter()
        .map(|a| (a.id.clone(), a))
        .collect();

        let cowboy = H3ReferenceEntity {
            id: "ent-cowboy".into(),
            handle: "Cowboy".into(),
            display_name: "Drifter".into(),
            kind: H3EntityKind::Character,
            subject_description: "Weathered cowboy in dark duster.".into(),
            retention_level: H3RetentionLevel::FullyPreserved,
            default_visual_intent: H3VisualIntent::Preserve,
            default_audio_intent: H3AudioIntent::VoiceIdentity,
            members: vec![
                H3ReferenceMember {
                    id: "mem-default".into(),
                    asset_id: pic_default.id.clone(),
                    label: "Default Portrait".into(),
                    population: H3ReferencePopulation::Pictures,
                    role: H3ReferenceRole::Subject,
                    crop: None,
                    source_start_sec: 0.0,
                    source_end_sec: 0.0,
                    use_embedded_audio: false,
                    description_override: String::new(),
                },
                H3ReferenceMember {
                    id: "mem-override".into(),
                    asset_id: pic_override.id.clone(),
                    label: "Poncho Close-up".into(),
                    population: H3ReferencePopulation::Pictures,
                    role: H3ReferenceRole::Subject,
                    crop: None,
                    source_start_sec: 0.0,
                    source_end_sec: 0.0,
                    use_embedded_audio: false,
                    description_override: "Dusty poncho variant.".into(),
                },
                H3ReferenceMember {
                    id: "mem-voice".into(),
                    asset_id: standalone_voice.id.clone(),
                    label: "Gravel Voice".into(),
                    population: H3ReferencePopulation::StandaloneAudios,
                    role: H3ReferenceRole::Voice,
                    crop: None,
                    source_start_sec: 0.0,
                    source_end_sec: 3.5,
                    use_embedded_audio: false,
                    description_override: String::new(),
                },
            ],
            created_at: String::new(),
            updated_at: String::new(),
        };

        let mut scene = H3Scene {
            id: "scene-1".into(),
            name: "Saloon".into(),
            kind: H3SceneKind::Standard,
            width: 1920,
            height: 1088,
            fps: 24,
            duration_sec: 20.0,
            window: H3WindowConfig {
                in_sec: 5.0,
                out_sec: 10.0,
                ..Default::default()
            },
            generation: H3GenerationSettings::default(),
            global_prompt: H3GlobalPrompt::default(),
            prompt_sections: Vec::new(),
            video_lanes: Vec::new(),
            audio_lanes: Vec::new(),
            driver_lanes: Vec::new(),
            reference_lanes: vec![
                H3ReferenceLane {
                    id: "r1".into(),
                    name: "R1".into(),
                    muted: false,
                    locked: false,
                    items: vec![
                        // Wide default portrait [0..20s]
                        H3StagedReferenceItem {
                            id: "item-wide".into(),
                            entity_id: Some("ent-cowboy".into()),
                            member_id: Some("mem-default".into()),
                            asset_id: None,
                            label: "@Cowboy default".into(),
                            role_override: None,
                            retention_override: None,
                            description_override: String::new(),
                            timeline_start_sec: 0.0,
                            duration_sec: 20.0,
                            use_visual: true,
                            use_audio: true,
                            enabled: true,
                        },
                        // Scene-wide voice [0..20s]
                        H3StagedReferenceItem {
                            id: "item-voice".into(),
                            entity_id: Some("ent-cowboy".into()),
                            member_id: Some("mem-voice".into()),
                            asset_id: None,
                            label: "@Cowboy voice".into(),
                            role_override: None,
                            retention_override: None,
                            description_override: String::new(),
                            timeline_start_sec: 0.0,
                            duration_sec: 20.0,
                            use_visual: false,
                            use_audio: true,
                            enabled: true,
                        },
                    ],
                },
                H3ReferenceLane {
                    id: "r2".into(),
                    name: "R2".into(),
                    muted: false,
                    locked: false,
                    items: vec![
                        // Tight override [5..10s] on R2!
                        H3StagedReferenceItem {
                            id: "item-tight".into(),
                            entity_id: Some("ent-cowboy".into()),
                            member_id: Some("mem-override".into()),
                            asset_id: None,
                            label: "@Cowboy poncho".into(),
                            role_override: None,
                            retention_override: None,
                            description_override: String::new(),
                            timeline_start_sec: 5.0,
                            duration_sec: 5.0,
                            use_visual: true,
                            use_audio: false,
                            enabled: true,
                        },
                        // Motion video with embedded audio [5..10s]
                        H3StagedReferenceItem {
                            id: "item-vid".into(),
                            entity_id: None,
                            member_id: None,
                            asset_id: Some(vid_motion.id.clone()),
                            label: "Saloon doors swing".into(),
                            role_override: Some(H3ReferenceRole::Motion),
                            retention_override: None,
                            description_override: "Heavy wooden doors swinging open with creak.".into(),
                            timeline_start_sec: 5.0,
                            duration_sec: 5.0,
                            use_visual: true,
                            use_audio: true,
                            enabled: true,
                        },
                    ],
                },
            ],
            guides: Vec::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };

        let project = MovieProject {
            schema_version: 9,
            id: "proj-1".into(),
            prompt: String::new(),
            title: "Test".into(),
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
                export_title: "Test".into(),
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
            entities: vec![cowboy],
            scenes: Vec::new(),
            active_scene_id: "scene-1".into(),
            render_jobs: Vec::new(),
            legacy_producer_feedback: Vec::new(),
            legacy_copilot_history: Vec::new(),
        };

        let geom = calculate_take_placement(&scene.window, 24).unwrap();
        let (manifest, verdicts, advisories, handle_map) = resolve_window_references(
            &project,
            &scene,
            scene.window.min_overlap_ratio,
            &geom,
            |aid| assets.get(aid).cloned().ok_or_else(|| StudioError::NotFound(aid.into())),
        );
        assert!(advisories.is_empty());
        // Tight poncho override must win <Picture 1>, superseding wide default!
        assert_eq!(manifest.pictures.len(), 1);
        assert_eq!(manifest.pictures[0].asset_id, pic_override.id);
        assert_eq!(manifest.pictures[0].tag, "<Picture 1>");
        assert_eq!(
            verdicts.iter().find(|v| v.item_id == "item-wide").unwrap().verdict,
            H3ReferenceVerdictKind::Superseded
        );
        // Embedded video audio MUST get <Audio 1>, standalone voice MUST get <Audio 2>!
        assert_eq!(manifest.videos.len(), 1);
        assert_eq!(manifest.videos[0].tag, "<Video 1>");
        assert_eq!(manifest.audios.len(), 2);
        assert_eq!(manifest.audios[0].tag, "<Audio 1>");
        assert_eq!(manifest.audios[0].asset_id, vid_motion.id);
        assert_eq!(manifest.audios[1].tag, "<Audio 2>");
        assert_eq!(manifest.audios[1].asset_id, standalone_voice.id);
        // @Cowboy maps to <Subject 1> with <Picture 1> and <Audio 2>!
        assert_eq!(handle_map.get("cowboy").map(String::as_str), Some("<Subject 1>"));
        assert_eq!(manifest.subjects.len(), 1);
        assert_eq!(manifest.subjects[0].visual_tags, vec!["<Picture 1>"]);
        assert_eq!(manifest.subjects[0].audio_tags, vec!["<Audio 2>"]);

        // Also verify auto_stage_reference creates R3 when [5..10s] collides on R1 & R2.
        let (lane_id, lane_name) = auto_stage_reference(
            &mut scene,
            H3StagedReferenceItem {
                id: "item-new".into(),
                entity_id: Some("ent-cowboy".into()),
                member_id: None,
                asset_id: None,
                label: "Extra".into(),
                role_override: None,
                retention_override: None,
                description_override: String::new(),
                timeline_start_sec: 6.0,
                duration_sec: 2.0,
                use_visual: true,
                use_audio: false,
                enabled: true,
            },
            None,
        )
        .unwrap();
        assert_eq!(lane_id, "ref-lane-3");
        assert_eq!(lane_name, "R3");
    }
}
