//! Fit the complete H3 frame grid to the producer's requested duration without trimming
//! away its conditioned final frames. The original full-VAE master remains immutable.
use super::*;

pub(super) async fn fit_take(
    source: &Path,
    target: &Path,
    seconds: f32,
    cancel: &CancellationToken,
) -> Result<(), StudioError> {
    let frames = (seconds * 24.0).round() as u32;
    let mut probe = tokio::process::Command::new(media_program("ffprobe"));
    probe
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,nb_frames",
            "-of",
            "json",
        ])
        .arg(source)
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(30), probe.output())
        .await
        .map_err(|_| StudioError::Render("Timed out inspecting the completed H3 take.".into()))??;
    let metadata: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
        StudioError::Render(
            "Could not inspect the completed H3 take. Its original master is preserved.".into(),
        )
    })?;
    let streams = metadata["streams"]
        .as_array()
        .ok_or_else(|| StudioError::Render("The H3 take has no media streams.".into()))?;
    let has_audio = streams.iter().any(|stream| stream["codec_type"] == "audio");
    let source_frames = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .and_then(|stream| stream["nb_frames"].as_str())
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|count| (2..=1024).contains(count));
    let Some(source_frames) = source_frames.filter(|_| output.status.success()) else {
        return Err(StudioError::Render(
            "The H3 master has no valid frame count. Its original file is preserved in raw/."
                .into(),
        ));
    };
    if !(24..=360).contains(&frames) {
        return Err(StudioError::Invalid(
            "Editor takes require 1–15 seconds at 24 frames per second.".into(),
        ));
    }
    let video = format!(
        "setpts=N*{}/({}*24*TB),fps=24",
        frames - 1,
        source_frames - 1
    );
    let tempo = f64::from(source_frames - 1) / f64::from(frames - 1);
    if !(0.5..=2.0).contains(&tempo) {
        return Err(StudioError::Render("H3 returned a duration too far from the requested take. The original master is preserved.".into()));
    }
    let audio =
        format!("atempo={tempo:.9},apad=whole_dur={seconds:.9},atrim=duration={seconds:.9}");
    // Keep synchronized streams in one graph. Separate -vf/-af graphs can stall
    // FFmpeg 7.1's scheduler on actual H3 AAC masters during duration fitting.
    let graph = if has_audio {
        format!("[0:v:0]{video}[video];[0:a:0]{audio}[audio]")
    } else {
        format!("[0:v:0]{video}[video]")
    };
    let mut command = tokio::process::Command::new(media_program("ffmpeg"));
    command
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-n", "-i"])
        .arg(source)
        .args(["-filter_complex", &graph, "-map", "[video]"]);
    if has_audio {
        command.args(["-map", "[audio]"]);
    }
    command
        .args([
            "-frames:v",
            &frames.to_string(),
            "-t",
            &seconds.to_string(),
            "-c:v",
            "libx264",
            "-crf",
            "16",
            "-preset",
            "medium",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
        ])
        .arg(target)
        .kill_on_drop(true);
    let output = tokio::select! {
        result=tokio::time::timeout(Duration::from_secs(300),command.output()) => result.map_err(|_|StudioError::Render("Finishing the editor take exceeded five minutes. Its original H3 master is preserved.".into()))??,
        _=cancel.cancelled() => return Err(StudioError::Cancelled),
    };
    if !output.status.success() {
        return Err(StudioError::Render(format!(
            "Could not finish the take at the selected duration: {}",
            truncate(&String::from_utf8_lossy(&output.stderr), 800)
        )));
    }
    producer::write_recoverable_json(
        &target.with_extension("media.json"),
        &json!({"source":source,"sourceFrames":source_frames,"sourceSha256":hash_reference(source)?,"frames":frames,"fps":24,"durationSeconds":seconds,"videoFilter":video,"audioFilter":audio,"resultSha256":hash_reference(target)?}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires local FFmpeg and FFprobe; verifies the last H3 frame survives duration fitting"]
    async fn live_editor_duration_fit_preserves_last_conditioned_frame() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("h3-grid.mp4");
        let target = root.path().join("take.mp4");
        let output = tokio::process::Command::new(media_program("ffmpeg"))
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=64x64:r=24:d=5.1666667",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=32000:duration=5.167",
                "-vf",
                "drawbox=color=red:t=fill:enable='gte(t,5.12)'",
                "-frames:v",
                "124",
                "-c:v",
                "libx264",
                "-crf",
                "10",
                "-c:a",
                "aac",
            ])
            .arg(&source)
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        let original_hash = hash_reference(&source).unwrap();
        tokio::time::timeout(
            Duration::from_secs(15),
            fit_take(&source, &target, 5.0, &CancellationToken::new()),
        )
        .await
        .expect("Five-second take finishing stalled")
        .unwrap();
        assert_eq!(hash_reference(&source).unwrap(), original_hash);
        let last = tokio::process::Command::new(media_program("ffmpeg"))
            .args(["-v", "error", "-ss", "4.958333", "-i"])
            .arg(&target)
            .args([
                "-frames:v",
                "1",
                "-pix_fmt",
                "rgb24",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .await
            .unwrap();
        assert!(last.status.success());
        assert_eq!(last.stdout.len(), 64 * 64 * 3);
        let pixel = &last.stdout[(32 * 64 + 32) * 3..][..3];
        assert!(
            pixel[0] > 200 && pixel[1] < 40 && pixel[2] < 40,
            "Final red conditioning frame was lost: {pixel:?}"
        );
        let media = probe_reference(&target, "video").unwrap();
        assert!((media.duration_seconds - 5.0).abs() < 0.002);
        assert!(media.has_audio);
    }
}
