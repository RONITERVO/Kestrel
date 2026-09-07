//! Native range resolution and ripple placement. Times are edited-timeline seconds.
use super::*;
use crate::models::MovieEditorEndpoint;

pub(super) fn edit_hash(edit: &MovieEdit) -> Result<String, StudioError> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(edit)?)))
}

pub(super) fn endpoint(
    project: &MovieProject,
    seconds: f64,
    end: bool,
) -> Result<MovieEditorEndpoint, StudioError> {
    let mut cursor = 0.0;
    let mut items: Vec<_> = project
        .edit
        .clips
        .iter()
        .filter(|item| item.enabled)
        .collect();
    items.sort_by_key(|item| item.order);
    for item in items {
        let source = selected_clip_source(project, item)?;
        let duration =
            f64::from((source.duration_seconds - item.trim_start - item.trim_end) / item.speed);
        let boundary = cursor + duration;
        if seconds >= cursor && (seconds < boundary || (end && seconds <= boundary + 0.00001)) {
            if source.path.is_empty() || !Path::new(source.path).is_file() {
                return Err(StudioError::Invalid(
                    "Render the selected timeline sources before generating between their frames."
                        .into(),
                ));
            }
            let source_seconds = (f64::from(item.trim_start)
                + (seconds - cursor) * f64::from(item.speed))
            .min(f64::from(source.duration_seconds - item.trim_end) - 1.0 / 24.0)
            .max(f64::from(item.trim_start));
            let clip = project
                .clips
                .iter()
                .find(|clip| clip.id == item.clip_id)
                .expect("source validated");
            let scene_prompt = if item.source_version_id.is_empty() {
                clip.prompt.clone()
            } else {
                clip.versions
                    .iter()
                    .find(|version| version.id == item.source_version_id)
                    .expect("source version validated")
                    .prompt
                    .clone()
            };
            return Ok(MovieEditorEndpoint {
                edit_id: item.id.clone(),
                clip_id: item.clip_id.clone(),
                version_id: item.source_version_id.clone(),
                source_path: source.path.into(),
                source_sha256: String::new(),
                source_seconds,
                image_path: String::new(),
                image_sha256: String::new(),
                scene_prompt,
            });
        }
        cursor = boundary;
    }
    Err(StudioError::Invalid(
        "Select a start and end inside the rendered timeline.".into(),
    ))
}

pub(super) fn replace_range(
    project: &MovieProject,
    start: f64,
    end: f64,
    replacement: ClipEdit,
    replacement_seconds: f32,
) -> Result<MovieEdit, StudioError> {
    if !start.is_finite() || !end.is_finite() || start < 0.0 || end <= start {
        return Err(StudioError::Invalid(
            "The generation range must end after it starts.".into(),
        ));
    }
    let mut result = project.edit.clone();
    let mut ordered = project.edit.clips.clone();
    ordered.sort_by_key(|item| item.order);
    result.clips.clear();
    let mut cursor = 0.0;
    let mut inserted = false;
    for item in ordered {
        if !item.enabled {
            result.clips.push(item);
            continue;
        }
        let source = selected_clip_source(project, &item)?;
        let duration =
            f64::from((source.duration_seconds - item.trim_start - item.trim_end) / item.speed);
        let next = cursor + duration;
        if next <= start + 0.00001 || cursor >= end - 0.00001 {
            result.clips.push(item);
        } else {
            if start > cursor + 0.00001 {
                let mut leading = item.clone();
                leading.trim_end = source.duration_seconds
                    - item.trim_start
                    - ((start - cursor) as f32 * item.speed);
                leading.fade_out = 0.0;
                leading.audio_fade_out = 0.0;
                clamp_fades(&mut leading, source.duration_seconds);
                result.clips.push(leading);
            }
            if !inserted {
                result.clips.push(replacement.clone());
                inserted = true;
            }
            if end < next - 0.00001 {
                let mut trailing = item.clone();
                trailing.id = format!("edit-{}", uuid::Uuid::new_v4());
                trailing.trim_start += (end - cursor) as f32 * item.speed;
                trailing.fade_in = 0.0;
                trailing.audio_fade_in = 0.0;
                clamp_fades(&mut trailing, source.duration_seconds);
                result.clips.push(trailing);
            }
        }
        cursor = next;
    }
    if !inserted || end > cursor + 0.00001 {
        return Err(StudioError::Invalid(
            "The selected range is outside the timeline.".into(),
        ));
    }
    for (index, item) in result.clips.iter_mut().enumerate() {
        item.order = index as u32;
    }
    let delta = f64::from(replacement_seconds) - (end - start);
    for marker in &mut result.markers {
        let time = f64::from(marker.time_seconds);
        if time >= end {
            marker.time_seconds = (time + delta) as f32;
        } else if time > start {
            marker.time_seconds =
                (start + (time - start) / (end - start) * f64::from(replacement_seconds)) as f32;
        }
    }
    // The caller validates with the new master present. Tiny leftover fragments are rejected
    // explicitly by native edit validation; they are never silently thrown away.
    Ok(result)
}

fn clamp_fades(item: &mut ClipEdit, source_seconds: f32) {
    let seconds = (source_seconds - item.trim_start - item.trim_end) / item.speed;
    item.fade_in = item.fade_in.min(seconds);
    item.fade_out = item.fade_out.min(seconds);
    item.audio_fade_in = item.audio_fade_in.min(seconds);
    item.audio_fade_out = item.audio_fade_out.min(seconds);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fixture(studio: &MovieStudio) -> MovieProject {
        let mut project = studio
            .create_producer_base(
                "An editor test movie.".into(),
                MovieSettings::default(),
                vec![],
                "test",
                false,
            )
            .unwrap();
        project.clips = (0..3)
            .map(|index| RenderedClip {
                id: format!("clip-{index}"),
                index,
                title: format!("Scene {index}"),
                prompt: format!("Written scene {index}"),
                duration_seconds: 10.0,
                seed: 42,
                status: "complete".into(),
                path: format!("source-{index}.mp4"),
                error: String::new(),
                versions: vec![],
            })
            .collect();
        project.edit.clips = project
            .clips
            .iter()
            .map(|clip| ClipEdit {
                id: format!("edit-{}", clip.id),
                clip_id: clip.id.clone(),
                order: clip.index,
                enabled: true,
                trim_start: 0.0,
                trim_end: 0.0,
                source_version_id: String::new(),
                speed: 1.0,
                audio_gain: 1.0,
                fade_in: 0.0,
                fade_out: 0.0,
                audio_fade_in: 0.0,
                audio_fade_out: 0.0,
                label: String::new(),
                notes: String::new(),
            })
            .collect();
        project
    }

    fn duration(project: &MovieProject, edit: &MovieEdit) -> f32 {
        edit.clips
            .iter()
            .filter(|item| item.enabled)
            .map(|item| {
                let source = selected_clip_source(project, item).unwrap();
                (source.duration_seconds - item.trim_start - item.trim_end) / item.speed
            })
            .sum()
    }

    #[test]
    fn range_replacement_splits_retimed_items_preserves_disabled_and_ripples_markers() {
        let dir = tempdir().unwrap();
        let studio = MovieStudio::new(dir.path()).unwrap();
        let mut project = fixture(&studio);
        project.edit.clips[0].trim_start = 2.0;
        project.edit.clips[0].speed = 2.0; // four timeline seconds
        project.edit.clips[1].enabled = false;
        project.edit.clips[2].speed = 0.5; // twenty timeline seconds
        project.edit.markers = [1.0, 4.0, 8.0, 20.0]
            .iter()
            .enumerate()
            .map(|(index, time)| TimelineMarker {
                id: format!("m{index}"),
                time_seconds: *time,
                label: "Keep this marker".into(),
                kind: "marker".into(),
                completed: false,
            })
            .collect();
        let mut new_master = project.clips[0].clone();
        new_master.id = "replacement".into();
        new_master.duration_seconds = 9.0;
        project.clips.push(new_master);
        let mut insert = project.edit.clips[1].clone();
        insert.id = "replacement-edit".into();
        insert.clip_id = "replacement".into();
        insert.enabled = true;
        let before = project.edit.clone();
        let mut next = replace_range(&project, 2.0, 8.0, insert, 9.0).unwrap();
        validate_movie_edit(&project, &mut next).unwrap();
        assert_eq!(project.edit, before);
        assert_eq!(duration(&project, &next), 27.0);
        assert_eq!(next.clips.len(), 4);
        assert_eq!(next.clips[0].trim_end, 4.0);
        assert!(!next.clips[2].enabled);
        assert_eq!(next.clips[3].trim_start, 2.0);
        assert_eq!(
            next.markers
                .iter()
                .map(|marker| marker.time_seconds)
                .collect::<Vec<_>>(),
            vec![1.0, 5.0, 11.0, 23.0]
        );
    }

    #[test]
    fn replacement_within_one_item_keeps_both_halves_and_immutable_source_version() {
        let dir = tempdir().unwrap();
        let studio = MovieStudio::new(dir.path()).unwrap();
        let mut project = fixture(&studio);
        project.clips[0].versions.push(ClipVersion {
            id: "original".into(),
            created_at: String::new(),
            title: "Original".into(),
            prompt: "Original scene".into(),
            duration_seconds: 20.0,
            seed: 7,
            path: "original.mp4".into(),
        });
        project.edit.clips[0].source_version_id = "original".into();
        let mut insert = project.edit.clips[1].clone();
        insert.id = "new".into();
        let mut next = replace_range(&project, 4.0, 5.0, insert, 10.0).unwrap();
        validate_movie_edit(&project, &mut next).unwrap();
        assert_eq!(duration(&project, &next), 49.0);
        assert_eq!(next.clips[0].trim_end, 16.0);
        assert_eq!(next.clips[2].trim_start, 5.0);
        assert_eq!(next.clips[2].source_version_id, "original");
        assert_ne!(next.clips[0].id, next.clips[2].id);
    }

    #[test]
    fn endpoint_at_a_cut_uses_following_start_and_preceding_end() {
        let dir = tempdir().unwrap();
        let studio = MovieStudio::new(dir.path()).unwrap();
        let mut project = fixture(&studio);
        for clip in &mut project.clips {
            let path = dir.path().join(&clip.path);
            fs::write(&path, b"source fixture").unwrap();
            clip.path = path.to_string_lossy().into_owned();
        }
        assert_eq!(endpoint(&project, 10.0, false).unwrap().clip_id, "clip-1");
        let end = endpoint(&project, 10.0, true).unwrap();
        assert_eq!(end.clip_id, "clip-0");
        assert!((end.source_seconds - (10.0 - 1.0 / 24.0)).abs() < 0.0001);
        assert!(endpoint(&project, 31.0, true).is_err());
    }

    #[test]
    fn tiny_fragments_fail_validation_instead_of_being_discarded() {
        let dir = tempdir().unwrap();
        let studio = MovieStudio::new(dir.path()).unwrap();
        let project = fixture(&studio);
        let mut insert = project.edit.clips[1].clone();
        insert.id = "new".into();
        let mut next = replace_range(&project, 0.01, 1.0, insert, 10.0).unwrap();
        assert_eq!(next.clips.len(), 5);
        assert!(validate_movie_edit(&project, &mut next).is_err());
        assert_eq!(project.edit.clips.len(), 3);
    }
}
