//! MiniMax H3 frame alignment (`17k + 5`), video/audio VAE latent token mapping,
//! aspect-preserving pass canvas scaling, and `authored_v1` take placement geometry.

use super::StudioError;
use kestrel_app_core::{
    H3CropRect, H3PassPreset, H3SceneKind, H3TakePlacementGeometry, H3TaskMode, H3WindowConfig,
};

pub const H3_FPS: u32 = 24;
pub const H3_FRAME_STEP: u32 = 17;
pub const H3_FRAME_OFFSET: u32 = 5;
pub const H3_MIN_FRAMES: u32 = 5;
pub const H3_MAX_FRAMES: u32 = 362; // 17 * 21 + 5 = 362 frames (~15.08s @ 24 fps)
pub const TAKE_PLACEMENT_POLICY: &str = "authored_v1";

const DRAFT_TARGET_AREA: f64 = (608 * 352) as f64;
const DETAIL_TARGET_AREA: f64 = (1344 * 768) as f64;
const UPSCALE_TARGET_AREA: f64 = (1920 * 1088) as f64;

/// Rounds `frames` UP to the nearest valid MiniMax H3 generation length `17k + 5` (`>= 5`).
pub fn align_h3_frames(frames: u32) -> u32 {
    let clamped = frames.max(H3_MIN_FRAMES);
    let rem = (clamped - H3_FRAME_OFFSET) % H3_FRAME_STEP;
    if rem == 0 {
        clamped
    } else {
        clamped + (H3_FRAME_STEP - rem)
    }
}

/// Number of temporal tokens `T_video` in H3's `[1, 24, T_video, H/16, W/16]` video latent.
/// Each 17-frame group encodes into 5 latent frames with receptive widths `[1, 4, 4, 4, 4]`,
/// after the initial 5-frame header (`[1, 4]`, 2 tokens).
pub fn h3_video_latent_t(frame_count: u32) -> u32 {
    let aligned = align_h3_frames(frame_count);
    if aligned <= H3_MIN_FRAMES {
        2
    } else {
        ((aligned - H3_MIN_FRAMES) / H3_FRAME_STEP) * 5 + 2
    }
}

/// Returns the `T_video + 1` pixel-frame boundary edges for each video latent token `0..T_video`.
/// Token `i` covers pixel frames `[edges[i], edges[i + 1])`.
pub fn h3_video_token_edges(frame_count: u32) -> Vec<u32> {
    let aligned = align_h3_frames(frame_count);
    let t_video = h3_video_latent_t(aligned) as usize;
    let mut edges = Vec::with_capacity(t_video + 1);
    edges.push(0);
    // Initial 5 frames -> [0..1), [1..5)
    edges.push(1);
    edges.push(5);
    let groups = (aligned - H3_MIN_FRAMES) / H3_FRAME_STEP;
    let mut cursor = 5_u32;
    for _ in 0..groups {
        for width in [1_u32, 4, 4, 4, 4] {
            cursor += width;
            edges.push(cursor);
        }
    }
    debug_assert_eq!(edges.len(), t_video + 1);
    debug_assert_eq!(edges.last().copied(), Some(aligned));
    edges
}

/// Conservatively maps pixel frame span `[mask_start_frame, mask_end_frame)` onto H3 video latent
/// token indices `[lo, hi)` (`0 <= lo <= hi <= T_video`). Any token whose pixel receptive span
/// overlaps the requested generation range is unmasked (`1.0`) so transition boundaries never tear.
pub fn resolve_h3_video_mask_span(
    frame_count: u32,
    mask_start_frame: u32,
    mask_end_frame: u32,
) -> (u32, u32) {
    let edges = h3_video_token_edges(frame_count);
    let t_video = (edges.len() - 1) as u32;
    if mask_end_frame <= mask_start_frame {
        return (0, 0);
    }
    let mut lo = t_video;
    let mut hi = 0_u32;
    for token in 0..t_video {
        let idx = token as usize;
        let tok_start = edges[idx];
        let tok_end = edges[idx + 1];
        if tok_end > mask_start_frame && tok_start < mask_end_frame {
            lo = lo.min(token);
            hi = hi.max(token + 1);
        }
    }
    if lo > hi {
        (0, 0)
    } else {
        (lo, hi)
    }
}

/// Number of temporal tokens `T_audio` in H3's `[1, 32, 2, T_audio]` stereo audio latent (`40 Hz`).
pub fn h3_audio_latent_t(frame_count: u32) -> u32 {
    let aligned = align_h3_frames(frame_count);
    (((aligned as f64) / (H3_FPS as f64)) * 40.0)
        .round()
        .max(1.0) as u32
}

/// Maps pixel frame span `[mask_start_frame, mask_end_frame)` onto `40 Hz` audio latent tokens.
pub fn resolve_h3_audio_mask_span(
    frame_count: u32,
    mask_start_frame: u32,
    mask_end_frame: u32,
) -> (u32, u32) {
    let aligned = align_h3_frames(frame_count);
    let t_audio = h3_audio_latent_t(aligned);
    if mask_end_frame <= mask_start_frame {
        return (0, 0);
    }
    let start_sec = (mask_start_frame as f64) / (H3_FPS as f64);
    let end_sec = (mask_end_frame.min(aligned) as f64) / (H3_FPS as f64);
    let lo = (start_sec * 40.0).floor().max(0.0) as u32;
    let hi = if mask_end_frame >= aligned {
        t_audio
    } else {
        (end_sec * 40.0).ceil().max(0.0) as u32
    };
    let lo = lo.min(t_audio);
    let hi = hi.clamp(lo, t_audio);
    (lo, hi)
}

pub fn default_scene_dimensions(kind: H3SceneKind) -> (u32, u32) {
    match kind {
        H3SceneKind::Standard => (1920, 1088),
        H3SceneKind::Audition => (960, 544),
        H3SceneKind::Soundtrack => (64, 32),
    }
}

pub fn validate_canvas_dimensions(
    width: u32,
    height: u32,
    allow_tiny_soundtrack: bool,
    advanced: bool,
) -> Result<(u32, u32), StudioError> {
    if !width.is_multiple_of(32) || !height.is_multiple_of(32) {
        return Err(StudioError::Invalid(format!(
            "H3 canvas dimensions ({width}x{height}) must be multiples of 32."
        )));
    }
    if allow_tiny_soundtrack && width == 64 && height == 32 {
        return Ok((64, 32));
    }
    let min_edge = 256_u32;
    let max_edge = if advanced { 2_048_u32 } else { 1_920_u32 };
    if width < min_edge || height < min_edge || width > max_edge || height > max_edge {
        return Err(StudioError::Invalid(format!(
            "H3 canvas dimensions ({width}x{height}) must be between {min_edge} and {max_edge} pixels per edge (or 64x32 for a Soundtrack pass)."
        )));
    }
    Ok((width, height))
}

fn snap_aspect_to_area(aspect: f64, target_area: f64, max_edge: u32) -> (u32, u32) {
    let clamped_aspect = aspect.clamp(0.25, 4.0);
    let raw_w = (target_area * clamped_aspect).sqrt();
    let raw_h = target_area / raw_w;
    let snap = |v: f64| -> u32 {
        let rounded = ((v / 32.0).round() as u32).saturating_mul(32);
        rounded.clamp(256, max_edge)
    };
    (snap(raw_w), snap(raw_h))
}

/// Resolves the concrete `(width, height)` multiple-of-32 H3 generation canvas for a pass.
pub fn adapt_pass_canvas(
    scene_canvas: (u32, u32, H3SceneKind),
    preset: H3PassPreset,
    task_mode: H3TaskMode,
    custom_canvas: (u32, u32),
    advanced: bool,
) -> Result<(u32, u32), StudioError> {
    let (scene_width, scene_height, scene_kind) = scene_canvas;
    let (custom_width, custom_height) = custom_canvas;
    if task_mode == H3TaskMode::SoundtrackPass || scene_kind == H3SceneKind::Soundtrack {
        return Ok((64, 32));
    }
    let base_w = scene_width.max(64) as f64;
    let base_h = scene_height.max(32) as f64;
    let aspect = base_w / base_h;
    let max_edge = if advanced { 2_048 } else { 1_920 };
    match preset {
        H3PassPreset::Draft02Mp => Ok(snap_aspect_to_area(aspect, DRAFT_TARGET_AREA, max_edge)),
        H3PassPreset::Detail1Mp => Ok(snap_aspect_to_area(aspect, DETAIL_TARGET_AREA, max_edge)),
        H3PassPreset::Upscale1080p => {
            Ok(snap_aspect_to_area(aspect, UPSCALE_TARGET_AREA, max_edge))
        }
        H3PassPreset::Custom => validate_canvas_dimensions(
            custom_width,
            custom_height,
            task_mode == H3TaskMode::SoundtrackPass,
            advanced,
        ),
    }
}

pub fn validate_normalized_crop(
    rect: &H3CropRect,
    label: &str,
) -> Result<H3CropRect, StudioError> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
    {
        return Err(StudioError::Invalid(format!(
            "{label} crop coordinates must be finite numbers."
        )));
    }
    if rect.x < -1e-4
        || rect.y < -1e-4
        || rect.width <= 0.02
        || rect.height <= 0.02
        || rect.x + rect.width > 1.0001
        || rect.y + rect.height > 1.0001
    {
        return Err(StudioError::Invalid(format!(
            "{label} crop must stay normalized inside [0, 1] with width and height >= 0.02."
        )));
    }
    let x = rect.x.clamp(0.0, 0.98);
    let y = rect.y.clamp(0.0, 0.98);
    let width = rect.width.clamp(0.02, 1.0 - x);
    let height = rect.height.clamp(0.02, 1.0 - y);
    Ok(H3CropRect {
        x,
        y,
        width,
        height,
    })
}

/// Computes `authored_v1` render window bounds, `17k + 5` frame padding, video/audio latent noise
/// mask spans, and Trimmed vs Untrimmed timeline placement coordinates.
pub fn calculate_take_placement(
    window: &H3WindowConfig,
    fps: u32,
) -> Result<H3TakePlacementGeometry, StudioError> {
    let fps = if fps == 0 { H3_FPS } else { fps };
    if fps != H3_FPS {
        return Err(StudioError::Invalid(format!(
            "MiniMax H3 operates at a fixed {H3_FPS} fps; got {fps} fps."
        )));
    }
    let values = [
        window.in_sec,
        window.out_sec,
        window.pre_context_sec,
        window.post_context_sec,
        window.mask_start_offset_sec,
        window.mask_end_offset_sec,
        window.min_overlap_ratio,
    ];
    if values.iter().any(|v| !v.is_finite()) {
        return Err(StudioError::Invalid(
            "H3 render window values must be finite numbers.".into(),
        ));
    }
    if window.in_sec < 0.0 {
        return Err(StudioError::Invalid(
            "H3 window In point cannot be negative.".into(),
        ));
    }
    if window.out_sec <= window.in_sec + 0.15 {
        return Err(StudioError::Invalid(
            "H3 window Out point must be at least 0.2s after In.".into(),
        ));
    }
    if window.pre_context_sec < 0.0 || window.post_context_sec < 0.0 {
        return Err(StudioError::Invalid(
            "H3 pre/post context durations cannot be negative.".into(),
        ));
    }
    if let Some(rect) = &window.spatial_mask_rect {
        validate_normalized_crop(rect, "Spatial mask")?;
    }

    let fps_f = fps as f64;
    // Snap selection and render bounds to exact 1/24s frame boundaries.
    let in_frame = (window.in_sec * fps_f).round().max(0.0) as u32;
    let out_frame = (window.out_sec * fps_f).round().max((in_frame + 5) as f64) as u32;
    let pre_frames = (window.pre_context_sec * fps_f).round().max(0.0) as u32;
    let post_frames = (window.post_context_sec * fps_f).round().max(0.0) as u32;

    let render_start_frame = in_frame.saturating_sub(pre_frames);
    let actual_pre_frames = in_frame - render_start_frame;
    let render_end_frame = out_frame.saturating_add(post_frames);
    let unpadded_frames = render_end_frame
        .saturating_sub(render_start_frame)
        .max(H3_MIN_FRAMES);

    if unpadded_frames > H3_MAX_FRAMES {
        return Err(StudioError::Invalid(format!(
            "H3 render window plus context spans {unpadded_frames} frames ({:.2}s), exceeding MiniMax H3's {H3_MAX_FRAMES}-frame ({:.2}s) single-window limit. Split into ~5-12s windows.",
            unpadded_frames as f64 / fps_f,
            H3_MAX_FRAMES as f64 / fps_f
        )));
    }

    let padded_frames = align_h3_frames(unpadded_frames);
    let frame_count_padding = padded_frames - unpadded_frames;

    let raw_mask_start = (actual_pre_frames as i64)
        + (window.mask_start_offset_sec * fps_f).round() as i64;
    let mask_start_frame = raw_mask_start.clamp(0, padded_frames as i64) as u32;

    let mask_end_frame = if post_frames == 0 && window.mask_end_offset_sec >= -1e-6 {
        // No post-context freeze: unmask right through the 17k+5 tail so H3 generates natural
        // trailing frames; Trimmed placement slices back to selection_out_sec cleanly.
        padded_frames
    } else {
        let local_selection_end = (out_frame - render_start_frame) as i64;
        let raw_mask_end =
            local_selection_end + (window.mask_end_offset_sec * fps_f).round() as i64;
        raw_mask_end.clamp(mask_start_frame as i64, unpadded_frames as i64) as u32
    };

    if mask_end_frame <= mask_start_frame {
        return Err(StudioError::Invalid(
            "H3 active generation mask span is empty; check pre/post context and mask offsets."
                .into(),
        ));
    }

    let video_latent_tokens = h3_video_latent_t(padded_frames);
    let (video_mask_token_lo, video_mask_token_hi) =
        resolve_h3_video_mask_span(padded_frames, mask_start_frame, mask_end_frame);
    let audio_latent_tokens = h3_audio_latent_t(padded_frames);
    let (audio_mask_token_lo, audio_mask_token_hi) =
        resolve_h3_audio_mask_span(padded_frames, mask_start_frame, mask_end_frame);

    let render_start_sec = render_start_frame as f64 / fps_f;
    let render_end_sec = render_end_frame as f64 / fps_f;
    let selection_in_sec = in_frame as f64 / fps_f;
    let selection_out_sec = out_frame as f64 / fps_f;

    let trimmed_timeline_start_sec = selection_in_sec;
    let trimmed_source_in_sec = actual_pre_frames as f64 / fps_f;
    let trimmed_duration_sec = (out_frame - in_frame) as f64 / fps_f;
    let untrimmed_timeline_start_sec = render_start_sec;
    let untrimmed_duration_sec = unpadded_frames as f64 / fps_f;

    Ok(H3TakePlacementGeometry {
        policy: TAKE_PLACEMENT_POLICY.into(),
        fps,
        render_start_sec,
        render_end_sec,
        selection_in_sec,
        selection_out_sec,
        mask_start_frame,
        mask_end_frame,
        unpadded_frames,
        padded_frames,
        frame_count_padding,
        video_latent_tokens,
        video_mask_token_lo,
        video_mask_token_hi,
        audio_latent_tokens,
        audio_mask_token_lo,
        audio_mask_token_hi,
        trimmed_timeline_start_sec,
        trimmed_duration_sec,
        trimmed_source_in_sec,
        untrimmed_timeline_start_sec,
        untrimmed_duration_sec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h3_frame_alignment_follows_17k_plus_5() {
        assert_eq!(align_h3_frames(1), 5);
        assert_eq!(align_h3_frames(5), 5);
        assert_eq!(align_h3_frames(6), 22);
        assert_eq!(align_h3_frames(22), 22);
        assert_eq!(align_h3_frames(120), 124); // 5.0s = 120f -> 17*7 + 5 = 124f
        assert_eq!(align_h3_frames(124), 124);
    }

    #[test]
    fn h3_video_latent_edges_and_mask_snapping_match_minimax_vae() {
        // 39 frames = 17*2 + 5 -> 2 + 2*5 = 12 latent tokens
        assert_eq!(h3_video_latent_t(39), 12);
        let edges = h3_video_token_edges(39);
        assert_eq!(edges, vec![0, 1, 5, 6, 10, 14, 18, 22, 23, 27, 31, 35, 39]);

        // Pre-context of 12 frames -> mask starts at frame 12 (inside token 4: [10..14))
        // Post-context after frame 30 (inside token 9: [27..31))
        let (lo, hi) = resolve_h3_video_mask_span(39, 12, 30);
        assert_eq!(lo, 4);
        assert_eq!(hi, 10);
    }

    #[test]
    fn pass_canvas_adapts_aspect_ratio_and_soundtrack_64x32() {
        assert_eq!(
            adapt_pass_canvas(
                (1920, 1088, H3SceneKind::Standard),
                H3PassPreset::Draft02Mp,
                H3TaskMode::Standard,
                (1920, 1088),
                false
            )
            .unwrap(),
            (608, 352)
        );
        assert_eq!(
            adapt_pass_canvas(
                (1920, 1088, H3SceneKind::Standard),
                H3PassPreset::Detail1Mp,
                H3TaskMode::Standard,
                (1920, 1088),
                false
            )
            .unwrap(),
            (1344, 768)
        );
        assert_eq!(
            adapt_pass_canvas(
                (1920, 1088, H3SceneKind::Standard),
                H3PassPreset::Upscale1080p,
                H3TaskMode::Standard,
                (1920, 1088),
                false
            )
            .unwrap(),
            (1920, 1088)
        );
        assert_eq!(
            adapt_pass_canvas(
                (1920, 1088, H3SceneKind::Standard),
                H3PassPreset::Draft02Mp,
                H3TaskMode::SoundtrackPass,
                (1920, 1088),
                false
            )
            .unwrap(),
            (64, 32)
        );
    }

    #[test]
    fn authored_v1_take_placement_tracks_context_and_padding() {
        let window = H3WindowConfig {
            in_sec: 2.0,
            out_sec: 7.0,
            pre_context_sec: 0.5,
            post_context_sec: 0.5,
            mask_start_offset_sec: 0.0,
            mask_end_offset_sec: 0.0,
            min_overlap_ratio: 0.5,
            spatial_mask_rect: None,
        };
        let geom = calculate_take_placement(&window, 24).unwrap();
        assert_eq!(geom.policy, "authored_v1");
        assert!((geom.render_start_sec - 1.5).abs() < 1e-6);
        assert!((geom.render_end_sec - 7.5).abs() < 1e-6);
        assert_eq!(geom.unpadded_frames, 144); // 6.0s * 24 = 144f
        assert_eq!(geom.padded_frames, 158); // 17*9 + 5 = 158f
        assert_eq!(geom.frame_count_padding, 14);
        assert_eq!(geom.mask_start_frame, 12); // 0.5s pre-context
        assert_eq!(geom.mask_end_frame, 132); // 5.5s = 132f (excludes 0.5s post-context + 14f padding)
        assert!((geom.trimmed_timeline_start_sec - 2.0).abs() < 1e-6);
        assert!((geom.trimmed_source_in_sec - 0.5).abs() < 1e-6);
        assert!((geom.trimmed_duration_sec - 5.0).abs() < 1e-6);
        assert!((geom.untrimmed_timeline_start_sec - 1.5).abs() < 1e-6);
        assert!((geom.untrimmed_duration_sec - 6.0).abs() < 1e-6);
    }
}
