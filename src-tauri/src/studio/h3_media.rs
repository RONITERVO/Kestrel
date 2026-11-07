//! Fixed-argument FFmpeg/FFprobe media preparation, spatial mask PNG generation,
//! multi-lane window source rendering, 3x4 ridge-affine color-drift cancellation,
//! and native H3 take quality telemetry.

use super::{
    h3_geometry::{H3_FPS, H3_MIN_FRAMES},
    h3_graph::{H3StagedGraphGuide, H3StagedGraphRef},
    h3_references::MAX_REFERENCE_SLICE_SEC,
    media_program, truncate, MovieProject, MovieReferenceAsset, StudioError,
};
use kestrel_app_core::{
    H3BoundOrdinalSlot, H3ClipSourceKind, H3ColorDriftReport, H3CropRect, H3ResolvedGuide, H3Scene,
    H3TakePlacementGeometry, H3TakeQualityReport, H3TaskMode,
};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::io::AsyncWriteExt;

const DRIFT_MAX_BIAS: f32 = 10.0;
const DRIFT_MAX_MATRIX_DEVIATION: f32 = 0.15;

/// Writes a valid 8-bit RGB PNG (`width x height`) in pure Rust where pixels inside `rect` are
/// white (`255, 255, 255` -> `noise_mask = 1.0`) and pixels outside are black (`0, 0, 0` -> `0.0`).
pub fn write_spatial_mask_png(
    path: &Path,
    width: u32,
    height: u32,
    rect: &H3CropRect,
) -> Result<(), StudioError> {
    let width = width.clamp(16, 4096);
    let height = height.clamp(16, 4096);
    let x0 = ((rect.x.clamp(0.0, 1.0) * width as f64).floor() as u32).min(width - 1);
    let y0 = ((rect.y.clamp(0.0, 1.0) * height as f64).floor() as u32).min(height - 1);
    let x1 = (((rect.x + rect.width).clamp(0.0, 1.0) * width as f64).ceil() as u32)
        .clamp(x0 + 1, width);
    let y1 = (((rect.y + rect.height).clamp(0.0, 1.0) * height as f64).ceil() as u32)
        .clamp(y0 + 1, height);

    let row_bytes = (width as usize) * 3 + 1; // filter byte 0 + RGB
    let mut raw = Vec::with_capacity(row_bytes * (height as usize));
    for y in 0..height {
        raw.push(0_u8); // filter type None
        let in_y = y >= y0 && y < y1;
        for x in 0..width {
            let val = if in_y && x >= x0 && x < x1 { 255_u8 } else { 0_u8 };
            raw.extend_from_slice(&[val, val, val]);
        }
    }

    // Wrap raw scanlines in valid zlib stored (uncompressed) DEFLATE blocks + Adler32.
    let mut zlib = Vec::with_capacity(raw.len() + (raw.len() / 65_535 + 2) * 5 + 6);
    zlib.extend_from_slice(&[0x78, 0x01]);
    let chunks: Vec<&[u8]> = if raw.is_empty() {
        vec![&[]]
    } else {
        raw.chunks(65_535).collect()
    };
    for (idx, chunk) in chunks.iter().enumerate() {
        let is_last = idx + 1 == chunks.len();
        zlib.push(if is_last { 0x01 } else { 0x00 });
        let len = chunk.len() as u16;
        zlib.extend_from_slice(&len.to_le_bytes());
        zlib.extend_from_slice(&(!len).to_le_bytes());
        zlib.extend_from_slice(chunk);
    }
    zlib.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut png = Vec::with_capacity(zlib.len() + 64);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = [0_u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // bit depth 8
    ihdr[9] = 2; // color type Truecolor RGB
    write_png_chunk(&mut png, b"IHDR", &ihdr);
    write_png_chunk(&mut png, b"IDAT", &zlib);
    write_png_chunk(&mut png, b"IEND", &[]);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(path)?;
    file.write_all(&png)?;
    file.sync_all()?;
    Ok(())
}

fn write_png_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in chunk_type.iter().chain(data.iter()) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
}

fn adler32(bytes: &[u8]) -> u32 {
    let mut a = 1_u32;
    let mut b = 0_u32;
    for &byte in bytes {
        a = (a + byte as u32) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// Stages a bound `<Picture>`, `<Video>`, or standalone `<Audio>` slot into `<comfy_root>/input/`,
/// applying normalized crop and/or `<= 15s` trim via fixed-argument FFmpeg when requested.
pub async fn stage_bound_reference_slot(
    comfy_root: &Path,
    project_id: &str,
    job_id: &str,
    slot: &H3BoundOrdinalSlot,
    asset: &MovieReferenceAsset,
) -> Result<H3StagedGraphRef, StudioError> {
    let src = Path::new(&asset.path);
    if !src.is_file() {
        return Err(StudioError::Render(format!(
            "Reference asset {} is missing on disk at {}.",
            asset.name, asset.path
        )));
    }
    let orig_ext = src
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or(match asset.kind.as_str() {
            "video" => "mp4",
            "audio" => "wav",
            _ => "png",
        });

    let needs_crop = slot.crop.as_ref().is_some_and(|c| {
        c.x > 0.002 || c.y > 0.002 || c.width < 0.998 || c.height < 0.998
    });
    let slice_dur = (slot.source_end_sec - slot.source_start_sec).max(0.0);
    let needs_trim = asset.kind != "image"
        && (slot.source_start_sec > 0.02
            || asset.duration_seconds > MAX_REFERENCE_SLICE_SEC + 0.05
            || (slot.source_end_sec > 0.05
                && (asset.duration_seconds - slot.source_end_sec).abs() > 0.05));

    let pop_slug = match slot.population {
        kestrel_app_core::H3ReferencePopulation::Pictures => "pic",
        kestrel_app_core::H3ReferencePopulation::Videos => "vid",
        kestrel_app_core::H3ReferencePopulation::StandaloneAudios => "aud",
    };

    if !needs_crop && !needs_trim && orig_ext != "opus" {
        let rel = format!(
            "kestrel/{project_id}/{job_id}/ref-{pop_slug}-{:02}.{orig_ext}",
            slot.ordinal
        );
        let dest = comfy_root.join("input").join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        tokio::fs::copy(src, &dest).await?;
        return Ok(H3StagedGraphRef {
            relative_file: rel,
            use_embedded_audio: slot.use_embedded_audio && slot.has_embedded_audio,
        });
    }

    let out_ext = match asset.kind.as_str() {
        "image" => "png",
        "video" => "mp4",
        _ => "wav",
    };
    let rel = format!(
        "kestrel/{project_id}/{job_id}/ref-{pop_slug}-{:02}.{out_ext}",
        slot.ordinal
    );
    let dest = comfy_root.join("input").join(&rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut cmd = tokio::process::Command::new(media_program("ffmpeg"));
    cmd.kill_on_drop(true)
        .args(["-y", "-hide_banner", "-loglevel", "error"]);

    if asset.kind != "image" {
        let start = slot.source_start_sec.max(0.0);
        let dur = if slice_dur > 0.05 {
            slice_dur.min(MAX_REFERENCE_SLICE_SEC)
        } else {
            asset.duration_seconds.clamp(0.25, MAX_REFERENCE_SLICE_SEC)
        };
        let min_dur = if asset.kind == "video" {
            (H3_MIN_FRAMES as f64 / H3_FPS as f64) + 0.05
        } else {
            0.25
        };
        cmd.args(["-ss", &format!("{start:.4}"), "-t", &format!("{:.4}", dur.max(min_dur))]);
    }
    cmd.arg("-i").arg(src);

    match asset.kind.as_str() {
        "image" => {
            if let Some(crop) = &slot.crop {
                let cw = (((asset.width as f64) * crop.width).round() as u32)
                    .max(32)
                    .min(asset.width.max(32));
                let ch = (((asset.height as f64) * crop.height).round() as u32)
                    .max(32)
                    .min(asset.height.max(32));
                let cx = (((asset.width as f64) * crop.x).round() as u32)
                    .min(asset.width.saturating_sub(cw));
                let cy = (((asset.height as f64) * crop.y).round() as u32)
                    .min(asset.height.saturating_sub(ch));
                cmd.arg("-vf")
                    .arg(format!("crop={cw}:{ch}:{cx}:{cy}"))
                    .args(["-frames:v", "1"]);
            }
        }
        "video" => {
            let mut vf = Vec::new();
            if let Some(crop) = &slot.crop {
                let base_w = asset.width.max(64);
                let base_h = asset.height.max(64);
                let cw = ((((base_w as f64) * crop.width).round() as u32) & !1)
                    .max(32)
                    .min(base_w);
                let ch = ((((base_h as f64) * crop.height).round() as u32) & !1)
                    .max(32)
                    .min(base_h);
                let cx = (((base_w as f64) * crop.x).round() as u32)
                    .min(base_w.saturating_sub(cw));
                let cy = (((base_h as f64) * crop.y).round() as u32)
                    .min(base_h.saturating_sub(ch));
                vf.push(format!("crop={cw}:{ch}:{cx}:{cy}"));
            }
            vf.push("fps=24,format=yuv420p".into());
            cmd.arg("-vf").arg(vf.join(",")).args([
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-crf",
                "16",
            ]);
            if slot.use_embedded_audio && slot.has_embedded_audio {
                cmd.args(["-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2"]);
            } else {
                cmd.arg("-an");
            }
        }
        _ => {
            cmd.args(["-vn", "-c:a", "pcm_s16le", "-ar", "48000", "-ac", "2"]);
        }
    }
    cmd.arg(&dest);
    run_ffmpeg(&mut cmd, &format!("stage reference {}", slot.label)).await?;

    Ok(H3StagedGraphRef {
        relative_file: rel,
        use_embedded_audio: slot.use_embedded_audio && slot.has_embedded_audio,
    })
}

/// Stages a resolved `H3ResolvedGuide` asset into `<comfy_root>/input/`.
pub async fn stage_guide_item(
    comfy_root: &Path,
    project_id: &str,
    job_id: &str,
    index: usize,
    guide: &H3ResolvedGuide,
    asset: &MovieReferenceAsset,
) -> Result<H3StagedGraphGuide, StudioError> {
    let src = Path::new(&asset.path);
    if !src.is_file() {
        return Err(StudioError::Render(format!(
            "Guide asset {} is missing on disk.",
            asset.name
        )));
    }
    let ext = src.extension().and_then(|v| v.to_str()).unwrap_or("png");
    if ext == "opus" {
        let rel = format!("kestrel/{project_id}/{job_id}/guide-{index:02}.wav");
        let dest = comfy_root.join("input").join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut cmd = tokio::process::Command::new(media_program("ffmpeg"));
        cmd.kill_on_drop(true)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(src)
            .args(["-vn", "-c:a", "pcm_s16le", "-ar", "48000", "-ac", "2"])
            .arg(&dest);
        run_ffmpeg(&mut cmd, "stage opus guide audio").await?;
        return Ok(H3StagedGraphGuide {
            kind: asset.kind.clone(),
            relative_file: rel,
            frame_idx: guide.frame_idx,
            use_image: guide.use_image,
            use_audio: guide.use_audio,
        });
    }

    let rel = format!("kestrel/{project_id}/{job_id}/guide-{index:02}.{ext}");
    let dest = comfy_root.join("input").join(&rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    tokio::fs::copy(src, &dest).await?;
    Ok(H3StagedGraphGuide {
        kind: asset.kind.clone(),
        relative_file: rel,
        frame_idx: guide.frame_idx,
        use_image: guide.use_image,
        use_audio: guide.use_audio,
    })
}

pub struct WindowSourceOutputs {
    pub comfy_video_rel: Option<String>,
    pub comfy_audio_rel: Option<String>,
    pub local_source_video: Option<PathBuf>,
}

/// Renders the scene's `[render_start_sec, render_end_sec)` + `frame_count_padding` window into
/// ComfyUI's `input/` directory so multi-pass refinement, pre/post context bridges, Swap modes,
/// and Freeze Video / Freeze Audio task modes operate on exact 24 fps / 48 kHz media.
#[allow(clippy::too_many_arguments)]
pub async fn render_scene_window_sources<F>(
    project: &MovieProject,
    scene: &H3Scene,
    geometry: &H3TakePlacementGeometry,
    width: u32,
    height: u32,
    task_mode: H3TaskMode,
    comfy_root: &Path,
    job_id: &str,
    resolve_clip_path: F,
) -> Result<WindowSourceOutputs, StudioError>
where
    F: Fn(H3ClipSourceKind, &str) -> Result<(PathBuf, bool), StudioError>,
{
    let win_start = geometry.render_start_sec;
    let win_end = geometry.render_end_sec;
    let unpadded_dur = geometry.unpadded_frames as f64 / H3_FPS as f64;
    let pad_dur = geometry.frame_count_padding as f64 / H3_FPS as f64;
    let padded_dur = geometry.padded_frames as f64 / H3_FPS as f64;

    // 1. Collect active video overlays (prefer driver_lanes when Swap mode has driver clips!)
    #[derive(Clone)]
    struct VideoSlice {
        path: PathBuf,
        timeline_start: f64,
        duration: f64,
        source_in: f64,
        opacity: f32,
    }

    let mut video_slices: Vec<VideoSlice> = Vec::new();
    if matches!(task_mode, H3TaskMode::CharacterSwap | H3TaskMode::HeadSwap) {
        for lane in &scene.driver_lanes {
            if lane.muted {
                continue;
            }
            for clip in &lane.clips {
                if !clip.enabled {
                    continue;
                }
                if clip.timeline_start_sec + clip.duration_sec > win_start + 1e-4
                    && clip.timeline_start_sec < win_end - 1e-4
                {
                    let (path, _) =
                        resolve_clip_path(H3ClipSourceKind::ReferenceAsset, &clip.asset_id)?;
                    video_slices.push(VideoSlice {
                        path,
                        timeline_start: clip.timeline_start_sec,
                        duration: clip.duration_sec,
                        source_in: clip.source_in_sec,
                        opacity: 1.0,
                    });
                }
            }
        }
    }
    if video_slices.is_empty() {
        let any_solo = scene.video_lanes.iter().any(|l| l.solo && !l.muted);
        // Bottom-to-top compositing: iterate in reverse if lane 0 is top?
        // Convention in NLE: V1 (index 0) is base track, V2 (index 1) overlays above V1!
        for lane in &scene.video_lanes {
            if lane.muted || (any_solo && !lane.solo) {
                continue;
            }
            for clip in &lane.clips {
                if !clip.enabled || clip.opacity <= 0.001 {
                    continue;
                }
                if clip.timeline_start_sec + clip.duration_sec > win_start + 1e-4
                    && clip.timeline_start_sec < win_end - 1e-4
                {
                    let (path, _) = resolve_clip_path(clip.source_kind, &clip.source_id)?;
                    video_slices.push(VideoSlice {
                        path,
                        timeline_start: clip.timeline_start_sec,
                        duration: clip.duration_sec,
                        source_in: clip.source_in_sec,
                        opacity: clip.opacity.clamp(0.0, 1.0),
                    });
                }
            }
        }
    }

    // 2. Collect active audio slices (`audio_lanes` + embedded audio on `video_lanes`)
    #[derive(Clone)]
    struct AudioSlice {
        path: PathBuf,
        timeline_start: f64,
        duration: f64,
        source_in: f64,
        gain: f32,
        fade_in: f64,
        fade_out: f64,
    }
    let mut audio_slices: Vec<AudioSlice> = Vec::new();
    let any_aud_solo = scene.audio_lanes.iter().any(|l| l.solo && !l.muted);
    for lane in &scene.audio_lanes {
        if lane.muted || (any_aud_solo && !lane.solo) {
            continue;
        }
        for clip in &lane.clips {
            if !clip.enabled || clip.gain <= 0.001 {
                continue;
            }
            if clip.timeline_start_sec + clip.duration_sec > win_start + 1e-4
                && clip.timeline_start_sec < win_end - 1e-4
            {
                let (path, has_aud) = resolve_clip_path(clip.source_kind, &clip.source_id)?;
                if has_aud {
                    audio_slices.push(AudioSlice {
                        path,
                        timeline_start: clip.timeline_start_sec,
                        duration: clip.duration_sec,
                        source_in: clip.source_in_sec,
                        gain: clip.gain.clamp(0.0, 4.0),
                        fade_in: clip.fade_in_sec.max(0.0),
                        fade_out: clip.fade_out_sec.max(0.0),
                    });
                }
            }
        }
    }
    let any_vid_solo = scene.video_lanes.iter().any(|l| l.solo && !l.muted);
    for lane in &scene.video_lanes {
        if lane.muted || (any_vid_solo && !lane.solo) {
            continue;
        }
        for clip in &lane.clips {
            if !clip.enabled || !clip.include_embedded_audio || clip.audio_gain <= 0.001 {
                continue;
            }
            if clip.timeline_start_sec + clip.duration_sec > win_start + 1e-4
                && clip.timeline_start_sec < win_end - 1e-4
            {
                let (path, has_aud) = resolve_clip_path(clip.source_kind, &clip.source_id)?;
                if has_aud {
                    audio_slices.push(AudioSlice {
                        path,
                        timeline_start: clip.timeline_start_sec,
                        duration: clip.duration_sec,
                        source_in: clip.source_in_sec,
                        gain: clip.audio_gain.clamp(0.0, 4.0),
                        fade_in: 0.0,
                        fade_out: 0.0,
                    });
                }
            }
        }
    }

    let mut comfy_video_rel = None;
    let mut local_source_video = None;
    if !video_slices.is_empty() {
        let rel = format!("kestrel/{}/{job_id}/window-source.mp4", project.id);
        let dest = comfy_root.join("input").join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let filter_file = dest.with_extension("vfilter.txt");
        let mut cmd = tokio::process::Command::new(media_program("ffmpeg"));
        cmd.kill_on_drop(true)
            .args(["-y", "-hide_banner", "-loglevel", "error"]);

        let mut filters = vec![format!(
            "color=c=black:s={width}x{height}:r={H3_FPS}:d={unpadded_dur:.6}[vbase0]"
        )];
        for (idx, slice) in video_slices.iter().enumerate() {
            let clip_start = slice.timeline_start.max(win_start);
            let clip_end = (slice.timeline_start + slice.duration).min(win_end);
            let seg_dur = (clip_end - clip_start).max(1.0 / H3_FPS as f64);
            let is_still = matches!(
                slice
                    .path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .as_deref(),
                Some("png" | "jpg" | "jpeg" | "webp" | "bmp")
            );
            let src_ss = if is_still {
                0.0
            } else {
                slice.source_in + (clip_start - slice.timeline_start).max(0.0)
            };
            let dst_offset = (clip_start - win_start).max(0.0);

            if is_still {
                cmd.args([
                    "-loop",
                    "1",
                    "-framerate",
                    &H3_FPS.to_string(),
                    "-t",
                    &format!("{seg_dur:.5}"),
                    "-i",
                ])
                .arg(&slice.path);
            } else {
                cmd.arg("-i").arg(&slice.path);
            }
            let mut chain = format!(
                "[{idx}:v]trim=start={src_ss:.5}:duration={seg_dur:.5},setpts=PTS-STARTPTS+{dst_offset:.5}/TB,scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height},setsar=1,fps={H3_FPS}"
            );
            let hold_after_sec = (win_end - clip_end).max(0.0);
            if hold_after_sec > 1e-3 && slice.opacity >= 0.999 {
                // Hold the last frame across any uncovered generation span so the 3D causal
                // Video VAE never encodes an artificial cut-to-black at the pre-context boundary.
                chain.push_str(&format!(
                    ",tpad=stop_mode=clone:stop_duration={hold_after_sec:.5}"
                ));
            }
            if slice.opacity < 0.999 {
                chain.push_str(&format!(
                    ",format=rgba,colorchannelmixer=aa={:.4}",
                    slice.opacity
                ));
            } else {
                chain.push_str(",format=yuv420p");
            }
            chain.push_str(&format!("[vs{idx}]"));
            filters.push(chain);
            filters.push(format!(
                "[vbase{idx}][vs{idx}]overlay=x=0:y=0:eof_action=pass[vbase{}]",
                idx + 1
            ));
        }
        let top = format!("vbase{}", video_slices.len());
        if pad_dur > 1e-5 {
            filters.push(format!(
                "[{top}]tpad=stop_mode=clone:stop_duration={pad_dur:.6},fps={H3_FPS},format=yuv420p[vout]"
            ));
        } else {
            filters.push(format!("[{top}]fps={H3_FPS},format=yuv420p[vout]"));
        }
        fs::write(&filter_file, filters.join(";"))?;
        cmd.arg("-filter_complex_script")
            .arg(&filter_file)
            .args([
                "-map",
                "[vout]",
                "-frames:v",
                &geometry.padded_frames.to_string(),
                "-c:v",
                "libx264",
                "-preset",
                "fast",
                "-crf",
                "14",
                "-an",
            ])
            .arg(&dest);
        let res = run_ffmpeg(&mut cmd, "render window source video").await;
        let _ = fs::remove_file(&filter_file);
        res?;
        comfy_video_rel = Some(rel);
        local_source_video = Some(dest);
    }

    let mut comfy_audio_rel = None;
    if !audio_slices.is_empty()
        || task_mode == H3TaskMode::KeepExistingAudio
    {
        let rel = format!("kestrel/{}/{job_id}/window-source.wav", project.id);
        let dest = comfy_root.join("input").join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let filter_file = dest.with_extension("afilter.txt");
        let mut cmd = tokio::process::Command::new(media_program("ffmpeg"));
        cmd.kill_on_drop(true)
            .args(["-y", "-hide_banner", "-loglevel", "error"]);

        if audio_slices.is_empty() {
            cmd.args([
                "-f",
                "lavfi",
                "-i",
                &format!("anullsrc=r=48000:cl=stereo:d={padded_dur:.6}"),
                "-c:a",
                "pcm_s16le",
                "-ar",
                "48000",
                "-ac",
                "2",
            ])
            .arg(&dest);
            run_ffmpeg(&mut cmd, "render silent window audio").await?;
        } else {
            let mut filters = vec![format!(
                "anullsrc=r=48000:cl=stereo:d={padded_dur:.6}[abase]"
            )];
            let mut mix_inputs = vec!["[abase]".to_string()];
            for (idx, slice) in audio_slices.iter().enumerate() {
                let clip_start = slice.timeline_start.max(win_start);
                let clip_end = (slice.timeline_start + slice.duration).min(win_end);
                let seg_dur = (clip_end - clip_start).max(0.02);
                let src_ss = slice.source_in + (clip_start - slice.timeline_start).max(0.0);
                let delay_ms = ((clip_start - win_start).max(0.0) * 1000.0).round() as u64;

                cmd.arg("-i").arg(&slice.path);
                let mut chain = format!(
                    "[{idx}:a]atrim=start={src_ss:.5}:duration={seg_dur:.5},asetpts=PTS-STARTPTS,aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,volume={:.4}",
                    slice.gain
                );
                if slice.fade_in > 0.01 {
                    chain.push_str(&format!(",afade=t=in:st=0:d={:.4}", slice.fade_in.min(seg_dur)));
                }
                if slice.fade_out > 0.01 {
                    let st = (seg_dur - slice.fade_out).max(0.0);
                    chain.push_str(&format!(
                        ",afade=t=out:st={st:.4}:d={:.4}",
                        slice.fade_out.min(seg_dur)
                    ));
                }
                if delay_ms > 0 {
                    chain.push_str(&format!(",adelay={delay_ms}|{delay_ms}"));
                }
                chain.push_str(&format!("[as{idx}]"));
                filters.push(chain);
                mix_inputs.push(format!("[as{idx}]"));
            }
            filters.push(format!(
                "{}amix=inputs={}:duration=first:normalize=0,atrim=0:{padded_dur:.6}[aout]",
                mix_inputs.join(""),
                mix_inputs.len()
            ));
            fs::write(&filter_file, filters.join(";"))?;
            cmd.arg("-filter_complex_script")
                .arg(&filter_file)
                .args([
                    "-map",
                    "[aout]",
                    "-c:a",
                    "pcm_s16le",
                    "-ar",
                    "48000",
                    "-ac",
                    "2",
                ])
                .arg(&dest);
            let res = run_ffmpeg(&mut cmd, "render window source audio").await;
            let _ = fs::remove_file(&filter_file);
            res?;
        }
        comfy_audio_rel = Some(rel);
    }

    Ok(WindowSourceOutputs {
        comfy_video_rel,
        comfy_audio_rel,
        local_source_video,
    })
}

/// Solves a 3x4 ridge-regularized affine color transform `target_rgb = M * gen_rgb + bias`
/// on `[0..255]` RGB pixel pairs with safety clamps (`|bias| <= 10`, `|M - I| <= 0.15`).
pub fn solve_ridge_color_affine(
    gen_rgb: &[u8],
    target_rgb: &[u8],
) -> ([[f32; 4]; 3], f32, f32) {
    let identity = [
        [1.0_f32, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    let n_pixels = gen_rgb.len().min(target_rgb.len()) / 3;
    if n_pixels < 64 {
        return (identity, 0.0, 0.0);
    }

    // Build X^T X (4x4) and X^T Y (4x3) in f64, normalizing features to [0..1] for conditioning.
    let mut xtx = [[0.0_f64; 4]; 4];
    let mut xty = [[0.0_f64; 3]; 4];
    for idx in 0..n_pixels {
        let base = idx * 3;
        let x = [
            gen_rgb[base] as f64 / 255.0,
            gen_rgb[base + 1] as f64 / 255.0,
            gen_rgb[base + 2] as f64 / 255.0,
            1.0_f64,
        ];
        let y = [
            target_rgb[base] as f64 / 255.0,
            target_rgb[base + 1] as f64 / 255.0,
            target_rgb[base + 2] as f64 / 255.0,
        ];
        for r in 0..4 {
            for c in 0..4 {
                xtx[r][c] += x[r] * x[c];
            }
            for ch in 0..3 {
                xty[r][ch] += x[r] * y[ch];
            }
        }
    }

    let ridge = (n_pixels as f64) * 0.015;
    // Regularize toward Identity (M -> I, bias -> 0)!
    for c in 0..3 {
        xtx[c][c] += ridge;
        xty[c][c] += ridge;
    }
    xtx[3][3] += ridge * 0.25;

    let Some(weights) = solve_4x4_multi(xtx, xty) else {
        return (identity, 0.0, 0.0);
    };

    let mut matrix = identity;
    let mut max_dev = 0.0_f32;
    let mut max_bias = 0.0_f32;
    for out_c in 0..3 {
        for in_c in 0..3 {
            let target_diag = if out_c == in_c { 1.0_f32 } else { 0.0_f32 };
            let raw = weights[in_c][out_c] as f32;
            let dev = (raw - target_diag).clamp(-DRIFT_MAX_MATRIX_DEVIATION, DRIFT_MAX_MATRIX_DEVIATION);
            max_dev = max_dev.max(dev.abs());
            matrix[out_c][in_c] = target_diag + dev;
        }
        let raw_bias = (weights[3][out_c] as f32) * 255.0;
        let clamped_bias = raw_bias.clamp(-DRIFT_MAX_BIAS, DRIFT_MAX_BIAS);
        max_bias = max_bias.max(clamped_bias.abs());
        matrix[out_c][3] = clamped_bias;
    }
    (matrix, max_bias, max_dev)
}

fn solve_4x4_multi(mut a: [[f64; 4]; 4], mut b: [[f64; 3]; 4]) -> Option<[[f64; 3]; 4]> {
    for col in 0..4 {
        let mut pivot = col;
        for row in (col + 1)..4 {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if a[pivot][col].abs() < 1e-9 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        let div = a[col][col];
        for val in a[col].iter_mut().skip(col) {
            *val /= div;
        }
        for val in b[col].iter_mut() {
            *val /= div;
        }
        let pivot_a = a[col];
        let pivot_b = b[col];
        for row in 0..4 {
            if row == col {
                continue;
            }
            let factor = a[row][col];
            for (dst, &pv) in a[row].iter_mut().zip(pivot_a.iter()).skip(col) {
                *dst -= factor * pv;
            }
            for (dst, &pb) in b[row].iter_mut().zip(pivot_b.iter()) {
                *dst -= factor * pb;
            }
        }
    }
    Some(b)
}

/// Compares frozen pre/post context frames between `source_window_mp4` and `raw_comfy_mp4`,
/// fits separate pre/post 3x4 ridge-affine color transforms, interpolates smoothly across the
/// generated span, and writes the drift-cancelled MP4 while copying untouched H3 stereo audio.
pub async fn apply_context_color_drift_correction(
    source_window_mp4: &Path,
    raw_comfy_mp4: &Path,
    output_mp4: &Path,
    geometry: &H3TakePlacementGeometry,
    width: u32,
    height: u32,
) -> Result<H3ColorDriftReport, StudioError> {
    let edges = super::h3_geometry::h3_video_token_edges(geometry.padded_frames);
    let frozen_pre = if geometry.video_mask_token_lo > 0 {
        edges
            .get(geometry.video_mask_token_lo as usize)
            .copied()
            .unwrap_or(geometry.mask_start_frame)
            .min(geometry.mask_start_frame)
    } else {
        geometry.mask_start_frame
    };
    let frozen_post_start = if geometry.video_mask_token_hi > 0
        && geometry.video_mask_token_hi < geometry.video_latent_tokens
    {
        edges
            .get(geometry.video_mask_token_hi as usize)
            .copied()
            .unwrap_or(geometry.mask_end_frame)
            .max(geometry.mask_end_frame)
    } else {
        geometry.mask_end_frame
    };
    let pre_count = frozen_pre.min(geometry.unpadded_frames) as usize;
    let post_start = frozen_post_start.min(geometry.unpadded_frames) as usize;
    let unpadded = geometry.unpadded_frames as usize;
    let post_count = unpadded.saturating_sub(post_start);
    let total_context = pre_count + post_count;

    if total_context < 2 || !source_window_mp4.is_file() || width < 64 || height < 64 {
        tokio::fs::copy(raw_comfy_mp4, output_mp4).await?;
        return Ok(H3ColorDriftReport {
            applied: false,
            matched_context_frames: total_context as u32,
            max_bias_abs: 0.0,
            max_matrix_deviation: 0.0,
            summary: "Skipped color-drift correction (fewer than 2 frozen context frames).".into(),
        });
    }

    let thumb_w = 160_u32;
    let thumb_h = 90_u32;
    let frame_bytes = (thumb_w * thumb_h * 3) as usize;
    let src_thumbs =
        decode_rgb24_frames(source_window_mp4, thumb_w, thumb_h, geometry.unpadded_frames).await?;
    let gen_thumbs =
        decode_rgb24_frames(raw_comfy_mp4, thumb_w, thumb_h, geometry.unpadded_frames).await?;

    let avail_frames = (src_thumbs.len() / frame_bytes).min(gen_thumbs.len() / frame_bytes);
    if avail_frames < 2 {
        tokio::fs::copy(raw_comfy_mp4, output_mp4).await?;
        return Ok(H3ColorDriftReport {
            applied: false,
            matched_context_frames: 0,
            max_bias_abs: 0.0,
            max_matrix_deviation: 0.0,
            summary: "Skipped color-drift correction (could not decode context frames).".into(),
        });
    }

    let pre_n = pre_count.min(avail_frames);
    let (pre_m, pre_bias, pre_dev) = if pre_n > 0 {
        solve_ridge_color_affine(
            &gen_thumbs[..pre_n * frame_bytes],
            &src_thumbs[..pre_n * frame_bytes],
        )
    } else {
        ([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]], 0.0, 0.0)
    };

    let (post_m, post_bias, post_dev) = if post_count > 0 && post_start < avail_frames {
        let s = post_start * frame_bytes;
        let e = avail_frames * frame_bytes;
        solve_ridge_color_affine(&gen_thumbs[s..e], &src_thumbs[s..e])
    } else {
        (pre_m, pre_bias, pre_dev)
    };

    let pre_m = if pre_n > 0 { pre_m } else { post_m };
    let max_bias_abs = pre_bias.max(post_bias);
    let max_matrix_deviation = pre_dev.max(post_dev);

    if max_bias_abs < 0.35 && max_matrix_deviation < 0.004 {
        tokio::fs::copy(raw_comfy_mp4, output_mp4).await?;
        return Ok(H3ColorDriftReport {
            applied: false,
            matched_context_frames: (pre_n + avail_frames.saturating_sub(post_start)) as u32,
            max_bias_abs,
            max_matrix_deviation,
            summary: format!(
                "Context exposure/chroma already matched within {:.2}/255 bias.",
                max_bias_abs
            ),
        });
    }

    // Stream full-res raw frames one frame at a time from ComfyUI output, apply interpolated 3x4
    // affine per frame, and encode into output_mp4 with explicit BT.709 color signaling.
    let full_frame_bytes = (width * height * 3) as usize;
    let mut dec = tokio::process::Command::new(media_program("ffmpeg"));
    dec.kill_on_drop(true)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(raw_comfy_mp4)
        .args([
            "-vf",
            &format!("scale={width}:{height}:flags=bilinear"),
            "-frames:v",
            &geometry.padded_frames.to_string(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut dec_child = dec.spawn()?;
    let mut dec_stdout = dec_child
        .stdout
        .take()
        .ok_or_else(|| StudioError::Render("failed to open FFmpeg decoder stdout pipe".into()))?;

    let mut enc = tokio::process::Command::new(media_program("ffmpeg"));
    enc.kill_on_drop(true)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-s",
            &format!("{width}x{height}"),
            "-r",
            &H3_FPS.to_string(),
            "-i",
            "pipe:0",
            "-i",
        ])
        .arg(raw_comfy_mp4)
        .args([
            "-map",
            "0:v:0",
            "-map",
            "1:a?",
            "-vf",
            "scale=out_color_matrix=bt709",
            "-c:v",
            "libx264",
            "-preset",
            "fast",
            "-crf",
            "15",
            "-pix_fmt",
            "yuv420p",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-c:a",
            "copy",
            "-movflags",
            "+faststart",
        ])
        .arg(output_mp4)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = enc.spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| StudioError::Render("failed to open FFmpeg stdin pipe".into()))?;

    let span_start = pre_n as f32;
    let span_end = (post_start.max(pre_n + 1)) as f32;
    let mut src_frame = vec![0_u8; full_frame_bytes];
    let mut row_buf = vec![0_u8; full_frame_bytes];
    let mut f_idx = 0_usize;

    while f_idx < geometry.padded_frames as usize {
        use tokio::io::AsyncReadExt;
        match dec_stdout.read_exact(&mut src_frame).await {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(StudioError::Io(err)),
        }
        let t = if post_count > 0 && pre_n > 0 {
            ((f_idx as f32 - span_start) / (span_end - span_start)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut m = [[0.0_f32; 4]; 3];
        for r in 0..3 {
            for c in 0..4 {
                m[r][c] = pre_m[r][c] * (1.0 - t) + post_m[r][c] * t;
            }
        }
        for px in (0..full_frame_bytes).step_by(3) {
            let r = src_frame[px] as f32;
            let g = src_frame[px + 1] as f32;
            let b = src_frame[px + 2] as f32;
            row_buf[px] = (m[0][0] * r + m[0][1] * g + m[0][2] * b + m[0][3])
                .round()
                .clamp(0.0, 255.0) as u8;
            row_buf[px + 1] = (m[1][0] * r + m[1][1] * g + m[1][2] * b + m[1][3])
                .round()
                .clamp(0.0, 255.0) as u8;
            row_buf[px + 2] = (m[2][0] * r + m[2][1] * g + m[2][2] * b + m[2][3])
                .round()
                .clamp(0.0, 255.0) as u8;
        }
        stdin.write_all(&row_buf).await?;
        f_idx += 1;
    }
    drop(stdin);
    let _ = dec_child.wait().await;
    let out = child.wait_with_output().await?;
    if f_idx == 0 {
        tokio::fs::copy(raw_comfy_mp4, output_mp4).await?;
        return Ok(H3ColorDriftReport {
            applied: false,
            matched_context_frames: 0,
            max_bias_abs,
            max_matrix_deviation,
            summary: "Fallback to raw take (empty decoded buffer).".into(),
        });
    }
    if !out.status.success() {
        return Err(StudioError::Render(format!(
            "color-drift encode failed: {}",
            truncate(&String::from_utf8_lossy(&out.stderr), 500)
        )));
    }

    Ok(H3ColorDriftReport {
        applied: true,
        matched_context_frames: (pre_n + avail_frames.saturating_sub(post_start)) as u32,
        max_bias_abs,
        max_matrix_deviation,
        summary: format!(
            "Applied 3x4 ridge color-drift cancellation (max bias {:.2}/255, matrix delta {:.3}).",
            max_bias_abs, max_matrix_deviation
        ),
    })
}

async fn decode_rgb24_frames(
    video_path: &Path,
    width: u32,
    height: u32,
    max_frames: u32,
) -> Result<Vec<u8>, StudioError> {
    let output = tokio::process::Command::new(media_program("ffmpeg"))
        .kill_on_drop(true)
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(video_path)
        .args([
            "-vf",
            &format!("scale={width}:{height}:flags=bilinear"),
            "-frames:v",
            &max_frames.to_string(),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "pipe:1",
        ])
        .output()
        .await?;
    if !output.status.success() {
        return Err(StudioError::Render(format!(
            "RGB frame decode failed: {}",
            truncate(&String::from_utf8_lossy(&output.stderr), 400)
        )));
    }
    Ok(output.stdout)
}

/// Probes a completed H3 take MP4, measures audio peak/RMS dBFS, and extracts 3 contact-sheet PNGs
/// (`contact-start.png`, `contact-mid.png`, `contact-end.png`) so both Human UI and Producer Agent
/// can verify visual and acoustic quality without guessing.
pub async fn analyze_take_quality(
    take_mp4: &Path,
    contact_dir: &Path,
    expected_width: u32,
    expected_height: u32,
    expect_audio: bool,
    geometry: Option<&H3TakePlacementGeometry>,
) -> Result<H3TakeQualityReport, StudioError> {
    let probe_out = tokio::process::Command::new(media_program("ffprobe"))
        .kill_on_drop(true)
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(take_mp4)
        .output()
        .await?;
    if !probe_out.status.success() {
        return Err(StudioError::Render(format!(
            "FFprobe verification failed on {}: {}",
            take_mp4.display(),
            truncate(&String::from_utf8_lossy(&probe_out.stderr), 400)
        )));
    }
    let val: Value = serde_json::from_slice(&probe_out.stdout)?;
    let streams = val
        .get("streams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let v_stream = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(Value::as_str) == Some("video"));
    let has_audio = streams
        .iter()
        .any(|s| s.get("codec_type").and_then(Value::as_str) == Some("audio"));

    let width = v_stream
        .and_then(|s| s.get("width"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let height = v_stream
        .and_then(|s| s.get("height"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let fps = v_stream
        .and_then(|s| s.get("r_frame_rate"))
        .and_then(Value::as_str)
        .and_then(parse_rational_fps)
        .unwrap_or(24.0);
    let duration_sec = val
        .pointer("/format/duration")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let frame_count = v_stream
        .and_then(|s| s.get("nb_frames"))
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(|| (duration_sec * fps as f64).round() as u32);

    let mut warnings = Vec::new();
    if width != expected_width || height != expected_height {
        warnings.push(format!(
            "Output dimensions {width}x{height} differ from requested {expected_width}x{expected_height}."
        ));
    }

    let (audio_peak_dbfs, audio_rms_dbfs, audio_is_silent) = if has_audio {
        measure_audio_dbfs(take_mp4).await.unwrap_or((-96.0, -96.0, true))
    } else {
        (-96.0, -96.0, true)
    };
    if expect_audio && (!has_audio || audio_is_silent) {
        warnings.push(
            "Audio stream is silent (<-55 dBFS peak); verify soundscape/vocal prompts and audio VAE."
                .into(),
        );
    }

    fs::create_dir_all(contact_dir)?;
    let mut contact_sheet_paths = Vec::new();
    if width >= 64 && height >= 32 && duration_sec > 0.05 {
        let (sel_start, sel_dur) = if let Some(geom) = geometry {
            let s = geom.trimmed_source_in_sec.clamp(0.0, (duration_sec - 0.1).max(0.0));
            let d = geom
                .trimmed_duration_sec
                .clamp(0.1, (duration_sec - s).max(0.1));
            (s, d)
        } else {
            (0.0, duration_sec)
        };
        let times = [
            ("contact-start.png", sel_start + (sel_dur * 0.12).min(0.35)),
            ("contact-mid.png", sel_start + sel_dur * 0.50),
            (
                "contact-end.png",
                (sel_start + (sel_dur * 0.88).max(0.08)).min((duration_sec - 0.08).max(0.0)),
            ),
        ];
        for (name, t) in times {
            let dst = contact_dir.join(name);
            if capture_frame_png(take_mp4, t, &dst).await.is_ok() {
                contact_sheet_paths.push(dst.to_string_lossy().into_owned());
            }
        }
    }

    Ok(H3TakeQualityReport {
        verified: width > 0 && height > 0 && duration_sec > 0.05,
        width,
        height,
        fps,
        frame_count,
        duration_sec,
        has_audio,
        audio_peak_dbfs,
        audio_rms_dbfs,
        audio_is_silent,
        contact_sheet_paths,
        warnings,
    })
}

async fn measure_audio_dbfs(path: &Path) -> Result<(f32, f32, bool), StudioError> {
    let out = tokio::process::Command::new(media_program("ffmpeg"))
        .kill_on_drop(true)
        .args(["-hide_banner", "-loglevel", "error", "-t", "20", "-i"])
        .arg(path)
        .args(["-vn", "-f", "f32le", "-ac", "2", "-ar", "16000", "pipe:1"])
        .output()
        .await?;
    if !out.status.success() || out.stdout.len() < 4 {
        return Ok((-96.0, -96.0, true));
    }
    let mut peak = 0.0_f32;
    let mut sum_sq = 0.0_f64;
    let mut count = 0_u64;
    for chunk in out.stdout.chunks_exact(4) {
        let s = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if s.is_finite() {
            let a = s.abs();
            if a > peak {
                peak = a;
            }
            sum_sq += (s as f64) * (s as f64);
            count += 1;
        }
    }
    if count == 0 || peak <= 1e-6 {
        return Ok((-96.0, -96.0, true));
    }
    let rms = (sum_sq / count as f64).sqrt() as f32;
    let peak_db = (20.0 * peak.log10()).clamp(-96.0, 6.0);
    let rms_db = if rms > 1e-6 {
        (20.0 * rms.log10()).clamp(-96.0, 6.0)
    } else {
        -96.0
    };
    let silent = peak_db < -55.0 || rms_db < -65.0;
    Ok((peak_db, rms_db, silent))
}

fn parse_rational_fps(text: &str) -> Option<f32> {
    if let Some((num, den)) = text.split_once('/') {
        let n: f32 = num.parse().ok()?;
        let d: f32 = den.parse().ok()?;
        if d > 0.0 {
            return Some(n / d);
        }
    }
    text.parse().ok()
}

pub async fn capture_frame_png(
    source_media: &Path,
    time_sec: f64,
    output_png: &Path,
) -> Result<(), StudioError> {
    if let Some(parent) = output_png.parent() {
        fs::create_dir_all(parent)?;
    }
    let ss = time_sec.max(0.0);
    let mut cmd = tokio::process::Command::new(media_program("ffmpeg"));
    cmd.kill_on_drop(true)
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &format!("{ss:.4}"),
            "-i",
        ])
        .arg(source_media)
        .args(["-frames:v", "1"])
        .arg(output_png);
    run_ffmpeg(&mut cmd, "capture PNG frame").await
}

pub(super) async fn run_ffmpeg(
    cmd: &mut tokio::process::Command,
    stage: &str,
) -> Result<(), StudioError> {
    let out = cmd
        .output()
        .await
        .map_err(|err| StudioError::Render(format!("could not start FFmpeg ({stage}): {err}")))?;
    if !out.status.success() {
        return Err(StudioError::Render(format!(
            "FFmpeg {stage} failed: {}",
            truncate(&String::from_utf8_lossy(&out.stderr), 700)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn pure_rust_spatial_mask_png_has_valid_signature_and_dimensions() {
        let dir = tempdir().unwrap();
        let out = dir.path().join("mask.png");
        write_spatial_mask_png(
            &out,
            64,
            32,
            &H3CropRect {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            },
        )
        .unwrap();
        let bytes = fs::read(&out).unwrap();
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&bytes[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]), 64);
        assert_eq!(u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]), 32);
    }

    #[test]
    fn ridge_color_affine_recovers_exposure_and_bias_within_clamps() {
        let mut gen = Vec::new();
        let mut target = Vec::new();
        for r in (20..220).step_by(10) {
            for g in (20..220).step_by(10) {
                let b = ((r + g) / 2) as u8;
                gen.extend_from_slice(&[r as u8, g as u8, b]);
                // Target is +6 brighter in R, -4 cooler in B
                target.extend_from_slice(&[
                    (r + 6).min(255) as u8,
                    g as u8,
                    (b as i32 - 4).max(0) as u8,
                ]);
            }
        }
        let (m, max_bias, max_dev) = solve_ridge_color_affine(&gen, &target);
        assert!(max_bias > 3.5 && max_bias <= DRIFT_MAX_BIAS);
        assert!(max_dev <= DRIFT_MAX_MATRIX_DEVIATION);
        assert!(m[0][3] > 3.0);
        assert!(m[2][3] < -2.0);
    }
}
