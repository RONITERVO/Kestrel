//! Bounded offline export: encode small groups, then concatenate using a local manifest.
//! No command line or filter graph grows with the length of the complete production.
use super::*;

const EXPORT_GROUP_SIZE: usize = 16;

struct Scratch {
    root: PathBuf,
    files: Vec<PathBuf>,
}

impl Scratch {
    fn new(parent: &Path) -> Result<Self, StudioError> {
        let root = parent.join(format!(".export-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root)?;
        Ok(Self {
            root,
            files: Vec::new(),
        })
    }

    fn file(&mut self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        self.files.push(path.clone());
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Delete only the explicit temporary files owned by this export. Unknown contents and
        // interrupted prior exports are never recursively removed.
        for file in &self.files {
            let _ = fs::remove_file(file);
        }
        let _ = fs::remove_dir(&self.root);
    }
}

pub(super) async fn render_timeline(
    project: &MovieProject,
    edits: &[ClipEdit],
    target: &Path,
) -> Result<f32, StudioError> {
    let mut scratch = Scratch::new(
        target
            .parent()
            .ok_or_else(|| StudioError::Render("export folder is missing".into()))?,
    )?;
    let (preset, crf, audio_bitrate) = match project.edit.export_preset.as_str() {
        "archive" => ("slow", "14", "320k"),
        "review" => ("veryfast", "24", "128k"),
        _ => ("medium", "18", "192k"),
    };
    let mut manifest = String::new();
    let mut duration = 0.0;
    for (group_index, group) in edits.chunks(EXPORT_GROUP_SIZE).enumerate() {
        let name = format!("segment-{group_index:05}.mkv");
        let segment = scratch.file(&name);
        let filter_path = scratch.file(&format!("filters-{group_index:05}.txt"));
        let mut command = tokio::process::Command::new(media_program("ffmpeg"));
        command
            .kill_on_drop(true)
            .args(["-y", "-hide_banner", "-loglevel", "error"]);
        let mut filters = Vec::new();
        for (index, edit) in group.iter().enumerate() {
            let source = selected_clip_source(project, edit)?;
            if !Path::new(source.path).is_file() {
                return Err(StudioError::Invalid(format!(
                    "timeline item {} has a missing preserved source: {}",
                    edit.id, source.path
                )));
            }
            command.arg("-i").arg(source.path);
            let (filter, seconds) = item_filter(project, edit, source.duration_seconds, index);
            filters.push(filter);
            duration += seconds;
        }
        let streams = (0..group.len())
            .map(|index| format!("[v{index}][a{index}]"))
            .collect::<String>();
        filters.push(format!("{streams}concat=n={}:v=1:a=1[v][a]", group.len()));
        fs::write(&filter_path, filters.join(";"))?;
        command
            .arg("-filter_complex_script")
            .arg(&filter_path)
            .args([
                "-map",
                "[v]",
                "-map",
                "[a]",
                "-c:v",
                "libx264",
                "-preset",
                preset,
                "-crf",
                crf,
                "-c:a",
                "flac",
                "-map_metadata",
                "-1",
            ])
            .arg(&segment);
        run(
            &mut command,
            &format!(
                "export segment {} of {}",
                group_index + 1,
                edits.len().div_ceil(EXPORT_GROUP_SIZE)
            ),
        )
        .await?;
        // Only generated portable names enter the concat grammar; producer filenames stay in
        // native argument arrays. FLAC prevents audio encoder delay at segment joins.
        manifest.push_str(&format!("file '{name}'\n"));
    }
    let manifest_path = scratch.file("segments.txt");
    fs::write(&manifest_path, manifest)?;
    let mut command = tokio::process::Command::new(media_program("ffmpeg"));
    command
        .kill_on_drop(true)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "concat",
            "-safe",
            "1",
            "-i",
        ])
        .arg(&manifest_path)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0",
            "-c:v",
            "copy",
            "-c:a",
            "aac",
            "-b:a",
            audio_bitrate,
        ]);
    if project.edit.normalize_audio {
        command.arg("-af").arg(format!(
            "loudnorm=I={}:TP=-1.5:LRA=11",
            project.edit.target_lufs
        ));
    }
    command
        .args([
            "-map_metadata",
            "-1",
            "-movflags",
            "+faststart",
            "-metadata",
        ])
        .arg(format!("title={}", project.edit.export_title))
        .arg(target);
    if let Err(error) = run(&mut command, "final export assembly").await {
        let _ = fs::remove_file(target);
        return Err(error);
    }
    Ok(duration)
}

async fn run(command: &mut tokio::process::Command, stage: &str) -> Result<(), StudioError> {
    let output = command
        .output()
        .await
        .map_err(|error| StudioError::Render(format!("could not start {stage}: {error}")))?;
    if !output.status.success() {
        return Err(StudioError::Render(format!(
            "{stage} failed: {}. Preserved source clips were not changed.",
            truncate(&String::from_utf8_lossy(&output.stderr), 1000)
        )));
    }
    Ok(())
}

fn item_filter(
    project: &MovieProject,
    edit: &ClipEdit,
    source_seconds: f32,
    index: usize,
) -> (String, f32) {
    let end = source_seconds - edit.trim_end;
    let duration = (end - edit.trim_start) / edit.speed;
    let mut video = format!(
        "[{index}:v]trim=start={}:end={},setpts=(PTS-STARTPTS)/{}",
        edit.trim_start, end, edit.speed
    );
    if edit.fade_in > 0.0 {
        video.push_str(&format!(",fade=t=in:st=0:d={}", edit.fade_in));
    }
    if edit.fade_out > 0.0 {
        video.push_str(&format!(
            ",fade=t=out:st={}:d={}",
            (duration - edit.fade_out).max(0.0),
            edit.fade_out
        ));
    }
    video.push_str(&format!(",scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=24,format=yuv420p[v{index}]", project.settings.width, project.settings.height, project.settings.width, project.settings.height));
    let mut audio = format!(
        "[{index}:a]atrim=start={}:end={},asetpts=PTS-STARTPTS",
        edit.trim_start, end
    );
    for stage in atempo_filters(edit.speed) {
        audio.push_str(&format!(",atempo={stage}"));
    }
    audio.push_str(&format!(",volume={}", edit.audio_gain));
    if edit.audio_fade_in > 0.0 {
        audio.push_str(&format!(",afade=t=in:st=0:d={}", edit.audio_fade_in));
    }
    if edit.audio_fade_out > 0.0 {
        audio.push_str(&format!(
            ",afade=t=out:st={}:d={}",
            (duration - edit.audio_fade_out).max(0.0),
            edit.audio_fade_out
        ));
    }
    audio.push_str(&format!(",aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,aresample=48000:async=1:first_pts=0[a{index}]"));
    (format!("{video};{audio}"), duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    #[ignore = "requires KESTREL_ACCEPTANCE_LIBRARY, KESTREL_ACCEPTANCE_MOVIE_ID and KESTREL_LIVE_COMFY_ROOT; renders the saved producer scenes with local H3 and exports them"]
    async fn live_producer_h3_render_and_export() {
        let library = PathBuf::from(
            std::env::var("KESTREL_ACCEPTANCE_LIBRARY").expect("set a separate acceptance library"),
        );
        assert!(library.is_absolute());
        assert_ne!(library, crate::store::default_research_root());
        let id = std::env::var("KESTREL_ACCEPTANCE_MOVIE_ID")
            .expect("choose the acceptance movie explicitly");
        let studio = MovieStudio::new(&library).unwrap();
        let mut project = studio.get(&id).unwrap();
        project.settings.comfy_root =
            std::env::var("KESTREL_LIVE_COMFY_ROOT").expect("choose installed ComfyUI");
        studio.save(&project).unwrap();
        studio.approve_producer_scenes(&id, None).await.unwrap();
        let result = studio.render(&id, &CancellationToken::new(), None).await;
        studio.release_comfy_memory().await;
        let rendered = result.unwrap();
        assert!(rendered
            .clips
            .iter()
            .all(|clip| clip.status == "complete" && Path::new(&clip.path).is_file()));
        let exported = studio.render_edit(&id).await.unwrap();
        assert!(Path::new(&exported.final_path).is_file());
        println!("H3 producer acceptance export: {}", exported.final_path);
    }

    #[tokio::test]
    #[ignore = "requires installed local FFmpeg and FFprobe; renders synthetic media without models or network"]
    async fn live_long_timeline_export_joins_groups_and_preserves_sources() {
        let folder = tempdir().unwrap();
        let studio = MovieStudio::new(folder.path()).unwrap();
        let mut project = studio
            .create_producer_base(
                "Export acceptance test".into(),
                MovieSettings {
                    width: 320,
                    height: 320,
                    ..MovieSettings::default()
                },
                Vec::new(),
                "none",
                false,
            )
            .unwrap();
        project.edit.export_preset = "review".into();
        let source = folder.path().join("producer's source.mp4");
        let output = tokio::process::Command::new(media_program("ffmpeg"))
            .args([
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=320x320:r=24",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-c:a",
                "aac",
            ])
            .arg(&source)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let original_hash = hash_reference(&source).unwrap();
        project.clips = vec![RenderedClip {
            id: "source".into(),
            index: 0,
            title: "Source".into(),
            prompt: "Synthetic".into(),
            duration_seconds: 1.0,
            seed: 1,
            status: "complete".into(),
            path: source.to_string_lossy().into_owned(),
            error: String::new(),
            versions: Vec::new(),
        }];
        let edits = (0..17)
            .map(|index| {
                serde_json::from_value::<ClipEdit>(json!({
                    "id":format!("edit-{index}"), "clipId":"source", "enabled":true, "order":index,
                    "trimStart":0.0, "trimEnd":0.0, "audioGain":1.0,
                }))
                .unwrap()
            })
            .collect::<Vec<_>>();
        project.edit.clips = edits;
        studio.save(&project).unwrap();
        let exported = studio.render_edit(&project.id).await.unwrap();
        assert_eq!(exported.exports.len(), 1);
        assert_eq!(exported.exports[0].clip_count, 17);
        assert_eq!(exported.exports[0].duration_seconds, 17.0);
        let probe = tokio::process::Command::new(media_program("ffprobe"))
            .args([
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-of",
                "json",
            ])
            .arg(&exported.final_path)
            .output()
            .await
            .unwrap();
        assert!(probe.status.success());
        let value: Value = serde_json::from_slice(&probe.stdout).unwrap();
        let actual_duration: f32 = value["format"]["duration"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(
            (actual_duration - 17.0).abs() < 0.15,
            "duration {actual_duration}"
        );
        assert_eq!(value["streams"].as_array().unwrap().len(), 2);
        assert_eq!(hash_reference(&source).unwrap(), original_hash);
        assert_eq!(
            hash_reference(Path::new(&exported.final_path)).unwrap(),
            exported.exports[0].sha256
        );
        assert!(
            fs::read_dir(studio.project_dir(&project.id).join("exports"))
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".export-"))
        );
    }
}
