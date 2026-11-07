//! Bounded offline export: encode bounded timeline chunks, then concatenate using a local manifest.
//! Supports both multi-lane `H3Scene` exports (BT.709 color tags + linear peak-headroom audio) and
//! legacy `MovieEdit` timelines. No command line or filter graph grows unboundedly.

use super::*;
use kestrel_app_core::{H3ClipSourceKind, H3Scene};

const EXPORT_GROUP_SIZE: usize = 16;
const H3_EXPORT_CHUNK_SEC: f64 = 30.0;

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

pub fn export_extension_for_preset(preset: &str) -> Result<&'static str, StudioError> {
    match preset {
        "publish" | "review" => Ok("mp4"),
        "archive" | "lossless" => Ok("mkv"),
        "audio_flac" => Ok("flac"),
        other => Err(StudioError::Invalid(format!(
            "Unsupported export preset '{other}'. Choose publish, archive, lossless, review, or audio_flac."
        ))),
    }
}

/// Renders a multi-lane `H3Scene` into `target` using bounded `30s` chunks, BT.709 color tags,
/// and non-compressing linear peak-headroom audio scaling (`-1.0 dBFS` default).
pub(super) async fn render_h3_scene<F>(
    scene: &H3Scene,
    preset: &str,
    export_title: &str,
    peak_headroom_db: f32,
    target: &Path,
    resolve_clip_path: F,
) -> Result<(f32, usize), StudioError>
where
    F: Fn(H3ClipSourceKind, &str) -> Result<(PathBuf, bool), StudioError>,
{
    let _ = export_extension_for_preset(preset)?;
    let headroom = peak_headroom_db.clamp(-12.0, -0.1);
    let audio_only = preset == "audio_flac";

    #[derive(Clone)]
    struct VItem {
        path: PathBuf,
        start: f64,
        dur: f64,
        src_in: f64,
        opacity: f32,
    }
    #[derive(Clone)]
    struct AItem {
        path: PathBuf,
        start: f64,
        dur: f64,
        src_in: f64,
        gain: f32,
        fade_in: f64,
        fade_out: f64,
    }

    let mut v_items = Vec::new();
    let mut a_items = Vec::new();
    let any_v_solo = scene.video_lanes.iter().any(|l| l.solo && !l.muted);
    for lane in &scene.video_lanes {
        if lane.muted || (any_v_solo && !lane.solo) {
            continue;
        }
        for clip in &lane.clips {
            if !clip.enabled || clip.duration_sec <= 0.02 {
                continue;
            }
            let (path, has_audio) = resolve_clip_path(clip.source_kind, &clip.source_id)?;
            if !audio_only && clip.opacity > 0.001 {
                v_items.push(VItem {
                    path: path.clone(),
                    start: clip.timeline_start_sec.max(0.0),
                    dur: clip.duration_sec,
                    src_in: clip.source_in_sec.max(0.0),
                    opacity: clip.opacity.clamp(0.0, 1.0),
                });
            }
            if clip.include_embedded_audio && has_audio && clip.audio_gain > 0.001 {
                a_items.push(AItem {
                    path,
                    start: clip.timeline_start_sec.max(0.0),
                    dur: clip.duration_sec,
                    src_in: clip.source_in_sec.max(0.0),
                    gain: clip.audio_gain.clamp(0.0, 4.0),
                    fade_in: 0.0,
                    fade_out: 0.0,
                });
            }
        }
    }

    let any_a_solo = scene.audio_lanes.iter().any(|l| l.solo && !l.muted);
    for lane in &scene.audio_lanes {
        if lane.muted || (any_a_solo && !lane.solo) {
            continue;
        }
        for clip in &lane.clips {
            if !clip.enabled || clip.duration_sec <= 0.02 || clip.gain <= 0.001 {
                continue;
            }
            let (path, has_audio) = resolve_clip_path(clip.source_kind, &clip.source_id)?;
            if has_audio {
                a_items.push(AItem {
                    path,
                    start: clip.timeline_start_sec.max(0.0),
                    dur: clip.duration_sec,
                    src_in: clip.source_in_sec.max(0.0),
                    gain: clip.gain.clamp(0.0, 4.0),
                    fade_in: clip.fade_in_sec.max(0.0),
                    fade_out: clip.fade_out_sec.max(0.0),
                });
            }
        }
    }

    if audio_only && a_items.is_empty() {
        return Err(StudioError::Invalid(
            "Scene has no audible clips to export as FLAC.".into(),
        ));
    }
    if !audio_only && v_items.is_empty() && a_items.is_empty() {
        return Err(StudioError::Invalid(
            "Place and enable at least one clip in the scene before exporting.".into(),
        ));
    }

    let max_v = v_items
        .iter()
        .map(|i| i.start + i.dur)
        .fold(0.0_f64, f64::max);
    let max_a = a_items
        .iter()
        .map(|i| i.start + i.dur)
        .fold(0.0_f64, f64::max);
    let total_dur = if audio_only {
        max_a
    } else if max_v > 0.0 {
        max_v.max(max_a)
    } else {
        max_a
    }
    .max(0.25);

    let total_clips = v_items.len() + a_items.len();
    let out_w = scene.width.max(64) & !1;
    let out_h = scene.height.max(32) & !1;
    let fps = scene.fps.max(24);

    let mut scratch = Scratch::new(
        target
            .parent()
            .ok_or_else(|| StudioError::Render("export folder is missing".into()))?,
    )?;

    let mut manifest = String::new();
    let mut chunk_idx = 0_usize;
    let mut c_start = 0.0_f64;

    while c_start < total_dur - 1e-4 || chunk_idx == 0 {
        let mut c_end = (c_start + H3_EXPORT_CHUNK_SEC).min(total_dur);
        while c_end - c_start > 1.0 {
            let v_cnt = v_items
                .iter()
                .filter(|i| i.start + i.dur > c_start + 1e-4 && i.start < c_end - 1e-4)
                .count();
            let a_cnt = a_items
                .iter()
                .filter(|i| i.start + i.dur > c_start + 1e-4 && i.start < c_end - 1e-4)
                .count();
            if v_cnt <= 24 && a_cnt <= 24 {
                break;
            }
            c_end = c_start + ((c_end - c_start) * 0.5).max(1.0);
        }
        let c_dur = (c_end - c_start).max(1.0 / fps as f64);

        let seg_ext = if audio_only { "wav" } else { "mkv" };
        let seg_name = format!("chunk-{chunk_idx:05}.{seg_ext}");
        let seg_path = scratch.file(&seg_name);
        let filter_path = scratch.file(&format!("chunk-filter-{chunk_idx:05}.txt"));

        let chunk_v: Vec<_> = v_items
            .iter()
            .filter(|i| i.start + i.dur > c_start + 1e-4 && i.start < c_end - 1e-4)
            .take(48)
            .collect();
        let chunk_a: Vec<_> = a_items
            .iter()
            .filter(|i| i.start + i.dur > c_start + 1e-4 && i.start < c_end - 1e-4)
            .take(48)
            .collect();

        let mut cmd = tokio::process::Command::new(media_program("ffmpeg"));
        cmd.kill_on_drop(true)
            .args(["-y", "-hide_banner", "-loglevel", "error"]);

        let mut filters = Vec::new();
        let mut input_idx = 0_usize;

        if !audio_only {
            filters.push(format!(
                "color=c=black:s={out_w}x{out_h}:r={fps}:d={c_dur:.6}[vb0]"
            ));
            for (vi, item) in chunk_v.iter().enumerate() {
                let s0 = item.start.max(c_start);
                let s1 = (item.start + item.dur).min(c_end);
                let dur = (s1 - s0).max(1.0 / fps as f64);
                let is_still = matches!(
                    item.path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| e.to_ascii_lowercase())
                        .as_deref(),
                    Some("png" | "jpg" | "jpeg" | "webp" | "bmp")
                );
                let ss = if is_still {
                    0.0
                } else {
                    item.src_in + (s0 - item.start).max(0.0)
                };
                let offset = (s0 - c_start).max(0.0);

                if is_still {
                    cmd.args([
                        "-loop",
                        "1",
                        "-framerate",
                        &fps.to_string(),
                        "-t",
                        &format!("{dur:.5}"),
                        "-i",
                    ])
                    .arg(&item.path);
                } else {
                    cmd.arg("-i").arg(&item.path);
                }
                let idx = input_idx;
                input_idx += 1;

                let mut vchain = format!(
                    "[{idx}:v]trim=start={ss:.5}:duration={dur:.5},setpts=PTS-STARTPTS+{offset:.5}/TB,scale={out_w}:{out_h}:force_original_aspect_ratio=increase,crop={out_w}:{out_h},setsar=1,fps={fps}"
                );
                if item.opacity < 0.999 {
                    vchain.push_str(&format!(
                        ",format=rgba,colorchannelmixer=aa={:.4}",
                        item.opacity
                    ));
                } else {
                    vchain.push_str(",format=yuv420p");
                }
                vchain.push_str(&format!("[vs{vi}]"));
                filters.push(vchain);
                filters.push(format!(
                    "[vb{vi}][vs{vi}]overlay=x=0:y=0:eof_action=pass[vb{}]",
                    vi + 1
                ));
            }
            let top_v = format!("vb{}", chunk_v.len());
            let pix = if preset == "lossless" {
                "rgb24"
            } else {
                "yuv420p"
            };
            filters.push(format!("[{top_v}]fps={fps},format={pix}[vout]"));
        }

        filters.push(format!("anullsrc=r=48000:cl=stereo:d={c_dur:.6}[abase]"));
        let mut mix_labels = vec!["[abase]".to_string()];
        for (ai, item) in chunk_a.iter().enumerate() {
            let s0 = item.start.max(c_start);
            let s1 = (item.start + item.dur).min(c_end);
            let dur = (s1 - s0).max(0.02);
            let ss = item.src_in + (s0 - item.start).max(0.0);
            let delay_ms = ((s0 - c_start).max(0.0) * 1000.0).round() as u64;

            cmd.arg("-i").arg(&item.path);
            let idx = input_idx;
            input_idx += 1;

            let mut achain = format!(
                "[{idx}:a]atrim=start={ss:.5}:duration={dur:.5},asetpts=PTS-STARTPTS,aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,volume={:.4}",
                item.gain
            );
            let clip_rel_start = (s0 - item.start).max(0.0);
            let clip_rel_end = (s1 - item.start).max(0.0);
            if item.fade_in > 0.01 && clip_rel_start < item.fade_in {
                let rem_fade = (item.fade_in - clip_rel_start).min(dur);
                achain.push_str(&format!(",afade=t=in:st=0:d={rem_fade:.4}"));
            }
            if item.fade_out > 0.01 && clip_rel_end > item.dur - item.fade_out {
                let fade_start_global = (item.dur - item.fade_out).max(0.0);
                let local_st = (fade_start_global - clip_rel_start).max(0.0);
                let rem_fade = (dur - local_st).max(0.01);
                achain.push_str(&format!(",afade=t=out:st={local_st:.4}:d={rem_fade:.4}"));
            }
            if delay_ms > 0 {
                achain.push_str(&format!(",adelay={delay_ms}|{delay_ms}"));
            }
            achain.push_str(&format!("[as{ai}]"));
            filters.push(achain);
            mix_labels.push(format!("[as{ai}]"));
        }
        filters.push(format!(
            "{}amix=inputs={}:duration=first:normalize=0,atrim=0:{c_dur:.6}[aout]",
            mix_labels.join(""),
            mix_labels.len()
        ));

        fs::write(&filter_path, filters.join(";"))?;
        cmd.arg("-filter_complex_script").arg(&filter_path);

        if audio_only {
            cmd.args(["-map", "[aout]", "-c:a", "pcm_f32le", "-ar", "48000", "-ac", "2"]);
        } else {
            cmd.args(["-map", "[vout]", "-map", "[aout]"]);
            match preset {
                "lossless" => {
                    cmd.args(["-c:v", "ffv1", "-level", "3", "-c:a", "pcm_s24le"]);
                }
                "archive" => {
                    cmd.args([
                        "-c:v",
                        "libx264",
                        "-preset",
                        "slow",
                        "-crf",
                        "12",
                        "-colorspace",
                        "bt709",
                        "-color_primaries",
                        "bt709",
                        "-color_trc",
                        "bt709",
                        "-c:a",
                        "flac",
                    ]);
                }
                "review" => {
                    cmd.args([
                        "-c:v",
                        "libx264",
                        "-preset",
                        "veryfast",
                        "-crf",
                        "23",
                        "-colorspace",
                        "bt709",
                        "-color_primaries",
                        "bt709",
                        "-color_trc",
                        "bt709",
                        "-c:a",
                        "flac",
                    ]);
                }
                _ => {
                    cmd.args([
                        "-c:v",
                        "libx264",
                        "-preset",
                        "medium",
                        "-crf",
                        "18",
                        "-colorspace",
                        "bt709",
                        "-color_primaries",
                        "bt709",
                        "-color_trc",
                        "bt709",
                        "-c:a",
                        "flac",
                    ]);
                }
            }
        }
        cmd.args(["-map_metadata", "-1"]).arg(&seg_path);
        run(&mut cmd, &format!("export chunk {}", chunk_idx + 1)).await?;
        manifest.push_str(&format!("file '{seg_name}'\n"));
        c_start = c_end;
        chunk_idx += 1;
        if c_start >= total_dur - 1e-4 {
            break;
        }
    }

    let manifest_path = scratch.file("chunks.txt");
    fs::write(&manifest_path, manifest)?;

    // Measure peak dBFS on joined stream so we can apply exact non-pumping linear peak headroom!
    let joined_probe = scratch.file(if audio_only {
        "joined-audio.wav"
    } else {
        "joined-master.mkv"
    });
    let mut join_cmd = tokio::process::Command::new(media_program("ffmpeg"));
    join_cmd
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
        .args(["-c", "copy"])
        .arg(&joined_probe);
    run(&mut join_cmd, "concatenate export chunks").await?;

    let peak_db = measure_export_peak_dbfs(&joined_probe).await.unwrap_or(-96.0);
    let gain_adj_db = if peak_db > headroom {
        headroom - peak_db
    } else {
        0.0
    };

    let mut final_cmd = tokio::process::Command::new(media_program("ffmpeg"));
    final_cmd
        .kill_on_drop(true)
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(&joined_probe);

    if audio_only {
        final_cmd.args(["-map", "0:a:0"]);
        if gain_adj_db.abs() > 0.01 {
            final_cmd.arg("-af").arg(format!("volume={gain_adj_db:.3}dB"));
        }
        final_cmd.args([
            "-c:a",
            "flac",
            "-ar",
            "48000",
            "-ac",
            "2",
            "-dither_method",
            "triangular",
        ]);
    } else {
        final_cmd.args(["-map", "0:v:0", "-map", "0:a:0", "-c:v", "copy"]);
        if gain_adj_db.abs() > 0.01 {
            final_cmd.arg("-af").arg(format!("volume={gain_adj_db:.3}dB"));
        }
        match preset {
            "lossless" => {
                final_cmd.args(["-c:a", "pcm_s24le", "-ar", "48000", "-ac", "2"]);
            }
            "archive" => {
                final_cmd.args(["-c:a", "flac", "-ar", "48000", "-ac", "2"]);
            }
            "review" => {
                final_cmd.args([
                    "-c:a",
                    "aac",
                    "-b:a",
                    "160k",
                    "-ar",
                    "48000",
                    "-ac",
                    "2",
                    "-movflags",
                    "+faststart",
                ]);
            }
            _ => {
                final_cmd.args([
                    "-c:a",
                    "aac",
                    "-b:a",
                    "192k",
                    "-ar",
                    "48000",
                    "-ac",
                    "2",
                    "-movflags",
                    "+faststart",
                ]);
            }
        }
    }
    final_cmd
        .args(["-map_metadata", "-1", "-metadata"])
        .arg(format!("title={export_title}"))
        .arg(target);

    if let Err(err) = run(&mut final_cmd, "final H3 export").await {
        let _ = fs::remove_file(target);
        return Err(err);
    }

    Ok((total_dur as f32, total_clips))
}

async fn measure_export_peak_dbfs(path: &Path) -> Result<f32, StudioError> {
    let out = tokio::process::Command::new(media_program("ffmpeg"))
        .kill_on_drop(true)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(path)
        .args(["-vn", "-f", "f32le", "-ac", "2", "-ar", "24000", "pipe:1"])
        .output()
        .await?;
    if !out.status.success() || out.stdout.len() < 4 {
        return Ok(-96.0);
    }
    let mut peak = 0.0_f32;
    for chunk in out.stdout.chunks_exact(4) {
        let s = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if s.is_finite() {
            peak = peak.max(s.abs());
        }
    }
    if peak <= 1e-6 {
        Ok(-96.0)
    } else {
        Ok((20.0 * peak.log10()).clamp(-96.0, 12.0))
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
    video.push_str(&format!(
        ",scale={}:{}:force_original_aspect_ratio=decrease,pad={}:{}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=24,format=yuv420p[v{index}]",
        project.settings.width, project.settings.height, project.settings.width, project.settings.height
    ));
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
    audio.push_str(
        ",aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,aresample=48000:async=1:first_pts=0[a{index}]",
    );
    // Replace `{index}` in the static suffix above:
    let audio = audio.replace("{index}", &index.to_string());
    (format!("{video};{audio}"), duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestrel_app_core::CreateMovieH3ProjectRequest;
    use tempfile::tempdir;

    #[tokio::test]
    #[ignore = "requires installed local FFmpeg and FFprobe; renders synthetic media without models or network"]
    async fn live_long_timeline_export_joins_groups_and_preserves_sources() {
        let folder = tempdir().unwrap();
        let studio = MovieStudio::new(folder.path()).unwrap();
        let mut project = studio
            .create_h3_project(
                CreateMovieH3ProjectRequest {
                    title: "Export acceptance test".into(),
                    story_markdown: "Synthetic export verification.".into(),
                    width: Some(320),
                    height: Some(320),
                    duration_sec: Some(17.0),
                },
                "/tmp/comfy",
                false,
                None,
            )
            .unwrap();
        project.settings.width = 320;
        project.settings.height = 320;
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
        let receipt = studio
            .render_h3_export(
                kestrel_app_core::RenderMovieExportRequest {
                    project_id: project.id.clone(),
                    scene_id: None,
                    preset: "review".into(),
                    title: Some("Export Test".into()),
                    peak_headroom_db: Some(-1.0),
                    destination_path: None,
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(receipt.export.clip_count, 17);
        assert_eq!(receipt.export.duration_seconds, 17.0);
        assert_eq!(hash_reference(&source).unwrap(), original_hash);
        assert_eq!(
            hash_reference(Path::new(&receipt.export.path)).unwrap(),
            receipt.export.sha256
        );
    }
}
