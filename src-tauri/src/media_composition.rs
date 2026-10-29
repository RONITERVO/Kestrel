//! Small, typed timeline compositor. Studios own generation; this module only joins artifacts.
use crate::{
    media_delivery::{has_stream, probe, probe_duration},
    production_artifacts::{resolve, Source},
    AppState,
};
use kestrel_app_core::{MediaCompositionExport, MediaCompositionRequest, MediaFrameShape};
use serde_json::{json, Value};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

fn read_record(path: &Path) -> Result<Value, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > 4 * 1024 * 1024 {
        return Err("Composition record exceeds 4 MiB.".into());
    }
    serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

pub fn get(root: &Path, id: &str) -> Result<Value, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Composition ID must be a UUID.")?;
    let parent = root.join("producer/compositions");
    let complete = parent.join(id).join("manifest.json");
    if complete.is_file() {
        return read_record(&complete);
    }
    let request_path = parent.join(format!(".partial-{id}")).join("request.json");
    if !request_path.is_file() {
        return Err("Composition not found. Use the composition id returned by list_media_compositions or export_media_composition; jobId is only the cancellation identity.".into());
    }
    let request = read_record(&request_path)?;
    Ok(
        json!({"id":id,"status":"incomplete","request":request,"detail":"Partial work is preserved. Inspect it before starting a new composition; nothing resumes automatically."}),
    )
}

pub fn list(root: &Path) -> Result<Vec<kestrel_app_core::MediaCompositionSummary>, String> {
    let parent = root.join("producer/compositions");
    if !parent.is_dir() {
        return Ok(vec![]);
    }
    let mut entries = fs::read_dir(&parent)
        .map_err(|e| e.to_string())?
        .take(10_001)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if entries.len() > 10_000 {
        return Err(
            "Composition catalog exceeds 10,000 entries; archive older outputs before listing."
                .into(),
        );
    }
    entries
        .sort_by_key(|entry| std::cmp::Reverse(entry.metadata().and_then(|m| m.modified()).ok()));
    let mut results = Vec::new();
    for entry in entries.into_iter().take(200) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let id = name.strip_prefix(".partial-").unwrap_or(&name);
        if uuid::Uuid::parse_str(id).is_err() {
            continue;
        }
        let manifest = get(root, id)?;
        let request: MediaCompositionRequest =
            serde_json::from_value(manifest["request"].clone()).map_err(|e| e.to_string())?;
        let native_path = manifest["file"]
            .as_str()
            .filter(|file| matches!(*file, "media.mp4" | "media.m4a"))
            .map(|file| parent.join(id).join(file).to_string_lossy().into_owned());
        results.push(kestrel_app_core::MediaCompositionSummary {
            id: id.into(),
            title: request.title,
            status: if native_path.is_some() {
                "complete"
            } else {
                "incomplete"
            }
            .into(),
            duration_seconds: request.duration_seconds,
            native_path,
        });
    }
    Ok(results)
}

fn bounded(value: f64, min: f64, max: f64) -> bool {
    value.is_finite() && value >= min && value <= max
}

fn validate(request: &MediaCompositionRequest) -> Result<(), String> {
    if request.title.trim().is_empty()
        || request.title.chars().count() > 200
        || !bounded(request.duration_seconds, 0.1, 3600.0)
        || request.visuals.len() > 64
        || request.audio.len() > 64
        || (request.visuals.is_empty() && request.audio.is_empty())
    {
        return Err("A composition needs a title, 0.1–3,600 seconds, and 1–64 visuals and/or 1–64 audio layers.".into());
    }
    let mut visual_duration = 0.0;
    for (index, segment) in request.visuals.iter().enumerate() {
        let previous = index
            .checked_sub(1)
            .map(|i| request.visuals[i].duration_seconds)
            .unwrap_or(0.0);
        if !bounded(segment.duration_seconds, 0.1, 3600.0)
            || !bounded(segment.source_start_seconds, 0.0, 43_200.0)
            || !bounded(segment.transition_seconds, 0.0, 5.0)
            || segment.transition_seconds >= segment.duration_seconds
            || (segment.transition_seconds > 0.0 && segment.transition_seconds >= previous)
        {
            return Err(format!("Visual {} needs a positive duration and a crossfade shorter than both adjacent segments; the first transition must be zero.", index + 1));
        }
        visual_duration += segment.duration_seconds - segment.transition_seconds;
    }
    if !request.visuals.is_empty() && (visual_duration - request.duration_seconds).abs() > 0.035 {
        return Err(format!("Visual duration minus transition overlaps is {visual_duration:.3}s; it must equal the requested {:.3}s. Stills need an explicit hold duration.", request.duration_seconds));
    }
    for layer in &request.audio {
        if !bounded(layer.duration_seconds, 0.1, request.duration_seconds)
            || !bounded(layer.start_seconds, 0.0, request.duration_seconds)
            || layer.start_seconds + layer.duration_seconds > request.duration_seconds + 0.001
            || !bounded(layer.source_start_seconds, 0.0, 43_200.0)
            || !bounded(layer.gain, 0.0, 4.0)
            || !bounded(layer.fade_in_seconds, 0.0, layer.duration_seconds)
            || !bounded(layer.fade_out_seconds, 0.0, layer.duration_seconds)
        {
            return Err("Audio layers need bounded source/timeline ranges, gain 0–4, and fades within the layer duration.".into());
        }
    }
    if !Path::new(&request.output_directory).is_absolute()
        || request.output_directory.contains(['*', '?'])
    {
        return Err("Choose an absolute output directory without wildcards.".into());
    }
    Ok(())
}

fn base_args() -> Vec<OsString> {
    ["-nostdin", "-hide_banner", "-loglevel", "error", "-n"]
        .into_iter()
        .map(Into::into)
        .collect()
}
fn video_encoding(args: &mut Vec<OsString>) {
    args.extend(
        [
            "-c:v", "libx264", "-preset", "fast", "-crf", "18", "-pix_fmt", "yuv420p", "-threads",
            "2",
        ]
        .into_iter()
        .map(Into::into),
    );
}
pub(crate) async fn run(
    executable: &Path,
    args: &[OsString],
    cancel: &CancellationToken,
) -> Result<(), String> {
    let operation = tokio::process::Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::select! {
        _ = cancel.cancelled() => return Err("Composition stopped. Its request and partial native files are preserved; it will not resume automatically.".into()),
        output = tokio::time::timeout(Duration::from_secs(3600), operation) => output
            .map_err(|_| "Composition command exceeded one hour; partial files were preserved.")?
            .map_err(|e| e.to_string())?,
    };
    if !output.status.success() {
        return Err(format!(
            "Composition failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(4000)
                .collect::<String>()
        ));
    }
    Ok(())
}

async fn check_range(
    ffprobe: &Path,
    source: &Source,
    kind: &str,
    start: f64,
    duration: f64,
) -> Result<(), String> {
    let info = probe(ffprobe, &source.path).await?;
    if !has_stream(&info, kind) {
        return Err(format!(
            "Selected source has no {kind} stream: {}",
            source.path.display()
        ));
    }
    if source.still {
        if start != 0.0 {
            return Err("A still image has no source offset; use zero.".into());
        }
    } else {
        let available = source.outpoint.unwrap_or(probe_duration(&info)?);
        if start + duration > available + 0.05 {
            return Err(format!("Requested excerpt ends at {:.3}s, beyond the source's {available:.3}s. Shorten the excerpt or explicitly add another segment; audio and video are never silently looped.", start + duration));
        }
    }
    Ok(())
}

pub async fn export(
    state: &AppState,
    request: MediaCompositionRequest,
    cancel: CancellationToken,
) -> Result<MediaCompositionExport, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let result = assemble(state, request, cancel, id.clone()).await;
    result.map_err(|error| {
        let parent = state.store.root().join("producer/compositions");
        if parent.join(&id).exists() || parent.join(format!(".partial-{id}")).exists() {
            format!("{error} Preserved composition id: {id}. Inspect get_media_composition with this id.")
        } else { error }
    })
}

async fn assemble(
    state: &AppState,
    request: MediaCompositionRequest,
    cancel: CancellationToken,
    id: String,
) -> Result<MediaCompositionExport, String> {
    validate(&request)?;
    let settings = state.research_settings.load().map_err(|e| e.to_string())?;
    let ffmpeg = PathBuf::from(settings.ffmpeg_path);
    let ffprobe = PathBuf::from(settings.ffprobe_path);
    if !ffmpeg.is_file() || !ffprobe.is_file() {
        return Err("Configure FFmpeg and FFprobe in Setup before composing media.".into());
    }
    let visuals = request
        .visuals
        .iter()
        .map(|v| resolve(state, &v.source))
        .collect::<Result<Vec<_>, _>>()?;
    let audio = request
        .audio
        .iter()
        .map(|a| resolve(state, &a.source))
        .collect::<Result<Vec<_>, _>>()?;
    for (segment, source) in request.visuals.iter().zip(&visuals) {
        check_range(
            &ffprobe,
            source,
            "video",
            segment.source_start_seconds,
            segment.duration_seconds,
        )
        .await?;
    }
    for (layer, source) in request.audio.iter().zip(&audio) {
        check_range(
            &ffprobe,
            source,
            "audio",
            layer.source_start_seconds,
            layer.duration_seconds,
        )
        .await?;
    }
    let parent = state.store.root().join("producer/compositions");
    fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let stage = parent.join(format!(".partial-{id}"));
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    fs::write(
        stage.join("request.json"),
        serde_json::to_vec_pretty(&request).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let (width, height) = match request.frame_shape {
        MediaFrameShape::Landscape => (1920, 1080),
        MediaFrameShape::Portrait => (1080, 1920),
        MediaFrameShape::Square => (1080, 1080),
    };
    let mut commands: Vec<Vec<String>> = Vec::new();
    let mut sequence: Option<PathBuf> = None;
    let mut elapsed = 0.0;
    // Normalize one segment at a time. Pairwise joins bound decoder memory for long slideshows.
    for (index, (segment, source)) in request.visuals.iter().zip(&visuals).enumerate() {
        state.control.observe("media-composition", json!({"title":format!("Composing visual {} of {}", index + 1, visuals.len()),"detail":"Normalizing and joining the selected native artifacts. No model generation is running."}))?;
        let clip = stage.join(format!("segment-{index}.mp4"));
        let mut args = base_args();
        if source.still {
            args.extend(["-loop".into(), "1".into(), "-framerate".into(), "30".into()]);
        } else {
            args.extend([
                "-ss".into(),
                format!("{:.6}", segment.source_start_seconds).into(),
            ]);
        }
        args.extend(["-i".into(), source.path.as_os_str().into(), "-t".into(), format!("{:.6}", segment.duration_seconds).into(), "-an".into(), "-vf".into(),
            format!("scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=30,format=yuv420p").into()]);
        video_encoding(&mut args);
        args.push(clip.as_os_str().into());
        run(&ffmpeg, &args, &cancel).await?;
        record(&mut commands, &args);
        if let Some(prior) = sequence {
            let joined = stage.join(format!("sequence-{index}.mp4"));
            let transition = segment.transition_seconds;
            let filter = if transition > 0.0 {
                // setpts can clear frame-rate metadata even on normalized CFR files.
                format!("[0:v]settb=AVTB,setpts=PTS-STARTPTS,fps=30[a];[1:v]settb=AVTB,setpts=PTS-STARTPTS,fps=30[b];[a][b]xfade=transition=fade:duration={transition:.6}:offset={:.6}[v]", elapsed-transition)
            } else {
                "[0:v][1:v]concat=n=2:v=1:a=0[v]".into()
            };
            let mut args = base_args();
            args.extend([
                "-i".into(),
                prior.as_os_str().into(),
                "-i".into(),
                clip.as_os_str().into(),
                "-filter_complex_threads".into(),
                "1".into(),
                "-filter_complex".into(),
                filter.into(),
                "-map".into(),
                "[v]".into(),
                "-an".into(),
            ]);
            video_encoding(&mut args);
            args.push(joined.as_os_str().into());
            run(&ffmpeg, &args, &cancel).await?;
            record(&mut commands, &args);
            sequence = Some(joined);
        } else {
            sequence = Some(clip);
        }
        elapsed += segment.duration_seconds - segment.transition_seconds;
    }
    let name = if visuals.is_empty() {
        "media.m4a"
    } else {
        "media.mp4"
    };
    let target = stage.join(name);
    state.control.observe("media-composition", json!({"title":"Mixing and verifying delivery","detail":format!("{} audio layers; {:.1} seconds.",audio.len(),request.duration_seconds)}))?;
    let mut args = base_args();
    if let Some(path) = &sequence {
        args.extend(["-i".into(), path.as_os_str().into()]);
    }
    for source in &audio {
        args.extend(["-i".into(), source.path.as_os_str().into()]);
    }
    let mut filters = Vec::new();
    let offset = usize::from(sequence.is_some());
    for (index, layer) in request.audio.iter().enumerate() {
        let fade_out = layer.duration_seconds - layer.fade_out_seconds;
        let mut filter = format!("[{}:a]atrim=start={:.6}:duration={:.6},asetpts=PTS-STARTPTS,aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo,volume={:.6}", index+offset, layer.source_start_seconds, layer.duration_seconds, layer.gain);
        if layer.fade_in_seconds > 0.0 {
            filter.push_str(&format!(",afade=t=in:d={:.6}", layer.fade_in_seconds));
        }
        if layer.fade_out_seconds > 0.0 {
            filter.push_str(&format!(
                ",afade=t=out:st={fade_out:.6}:d={:.6}",
                layer.fade_out_seconds
            ));
        }
        filter.push_str(&format!(
            ",adelay={}S:all=1[a{index}]",
            (layer.start_seconds * 48000.0).round() as u64
        ));
        filters.push(filter);
    }
    if !audio.is_empty() {
        let inputs = (0..audio.len())
            .map(|i| format!("[a{i}]"))
            .collect::<String>();
        filters.push(format!("{inputs}amix=inputs={}:duration=longest:normalize=0:dropout_transition=0,alimiter=limit=0.95:level=false:latency=true,apad,atrim=duration={:.6}[a]", audio.len(), request.duration_seconds));
        args.extend([
            "-filter_complex_threads".into(),
            "1".into(),
            "-filter_complex".into(),
            filters.join(";").into(),
            "-map".into(),
            "[a]".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "256k".into(),
        ]);
    }
    if sequence.is_some() {
        args.extend(["-map".into(), "0:v:0".into(), "-c:v".into(), "copy".into()]);
    }
    args.extend([
        "-t".into(),
        format!("{:.6}", request.duration_seconds).into(),
        "-movflags".into(),
        "+faststart".into(),
        target.as_os_str().into(),
    ]);
    run(&ffmpeg, &args, &cancel).await?;
    record(&mut commands, &args);
    let output_probe = probe(&ffprobe, &target).await?;
    verify_output(&output_probe, &request, width, height)?;
    let (bytes, hash) = crate::studio::hash_file(&target).map_err(|e| e.to_string())?;
    let source_receipts = |sources: &[Source]| {
        sources
            .iter()
            .map(|s| json!({"path":s.path,"sha256":s.hash,"outpoint":s.outpoint}))
            .collect::<Vec<_>>()
    };
    let manifest = json!({"schemaVersion":1,"id":id,"file":name,"request":request,"sha256":hash,"bytes":bytes,"probe":output_probe,
        "sources":{"visuals":source_receipts(&visuals),"audio":source_receipts(&audio)},"ffmpegArguments":commands,"uploadPerformed":false});
    fs::write(
        stage.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    // Intermediate encodes are disposable, but only after the verified artifact and manifest exist.
    for index in 0..request.visuals.len() {
        for prefix in ["segment", "sequence"] {
            let path = stage.join(format!("{prefix}-{index}.mp4"));
            if path.is_file() {
                fs::remove_file(path).map_err(|e| e.to_string())?;
            }
        }
    }
    let native = parent.join(&id);
    fs::rename(&stage, &native).map_err(|e| e.to_string())?;
    fs::create_dir_all(&request.output_directory).map_err(|e| format!("Composition {id} is saved in the library, but its delivery folder could not be created: {e}"))?;
    let delivery_stage =
        PathBuf::from(&request.output_directory).join(format!(".kestrel-composition-{id}"));
    let delivery = PathBuf::from(&request.output_directory).join(format!("composition-{id}"));
    fs::create_dir(&delivery_stage).map_err(|e| e.to_string())?;
    for file in [name, "manifest.json", "request.json"] {
        fs::copy(native.join(file), delivery_stage.join(file)).map_err(|e| e.to_string())?;
    }
    fs::rename(&delivery_stage, &delivery).map_err(|e| e.to_string())?;
    Ok(MediaCompositionExport {
        id,
        directory: delivery.to_string_lossy().into_owned(),
        media_path: delivery.join(name).to_string_lossy().into_owned(),
        native_path: native.join(name).to_string_lossy().into_owned(),
        manifest_path: delivery
            .join("manifest.json")
            .to_string_lossy()
            .into_owned(),
        duration_seconds: request.duration_seconds,
        sha256: hash,
    })
}

fn record(commands: &mut Vec<Vec<String>>, args: &[OsString]) {
    commands.push(
        args.iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect(),
    );
}

fn verify_output(
    info: &Value,
    request: &MediaCompositionRequest,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let streams = info["streams"]
        .as_array()
        .ok_or("Composition has no streams.")?;
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let audio = streams.iter().find(|s| s["codec_type"] == "audio");
    if (probe_duration(info)? - request.duration_seconds).abs() > 0.08
        || video.is_some() == request.visuals.is_empty()
        || audio.is_some() == request.audio.is_empty()
        || video.is_some_and(|v| {
            v["codec_name"] != "h264" || v["width"] != width || v["height"] != height
        })
        || audio.is_some_and(|a| a["codec_name"] != "aac")
    {
        return Err("Composition verification failed: duration, dimensions or requested audio/video streams do not match. Partial files were preserved.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kestrel_app_core::{MediaAudioLayer, MediaVisualSegment, ProductionArtifact};
    fn request() -> MediaCompositionRequest {
        MediaCompositionRequest {
            job_id: "test".into(),
            title: "Lesson".into(),
            duration_seconds: 9.0,
            frame_shape: MediaFrameShape::Landscape,
            visuals: (0..2)
                .map(|i| MediaVisualSegment {
                    source: ProductionArtifact::ImageTake {
                        project_id: "p".into(),
                        take_id: "t".into(),
                    },
                    duration_seconds: 5.0,
                    source_start_seconds: 0.0,
                    transition_seconds: if i == 0 { 0.0 } else { 1.0 },
                })
                .collect(),
            audio: vec![],
            output_directory: std::env::temp_dir().to_string_lossy().into_owned(),
        }
    }
    #[test]
    fn timeline_accounts_for_overlap_and_rejects_silent_truncation() {
        let mut r = request();
        assert!(validate(&r).is_ok());
        r.duration_seconds = 10.0;
        assert!(validate(&r).is_err());
        r.duration_seconds = 9.0;
        r.visuals[0].transition_seconds = 0.5;
        assert!(validate(&r).is_err());
        r.visuals[0].transition_seconds = 0.0;
        r.visuals[1].source_start_seconds = f64::NAN;
        assert!(validate(&r).is_err());
    }
    #[test]
    fn audio_only_and_silent_video_are_valid_but_empty_or_overflow_are_not() {
        let mut r = request();
        assert!(validate(&r).is_ok());
        r.visuals.clear();
        assert!(validate(&r).is_err());
        r.audio.push(MediaAudioLayer {
            source: ProductionArtifact::MusicTake {
                project_id: "p".into(),
                take_id: "t".into(),
            },
            start_seconds: 0.0,
            duration_seconds: 9.0,
            source_start_seconds: 0.0,
            gain: 0.4,
            fade_in_seconds: 0.5,
            fade_out_seconds: 1.0,
        });
        assert!(validate(&r).is_ok());
        r.audio[0].start_seconds = 1.0;
        assert!(validate(&r).is_err());
    }
    #[test]
    fn verifies_actual_streams_and_duration_not_just_process_exit() {
        let r = request();
        assert!(verify_output(&json!({"format":{"duration":"9.000"},"streams":[{"codec_type":"video","codec_name":"h264","width":1920,"height":1080}]}),&r,1920,1080).is_ok());
        assert!(verify_output(&json!({"format":{"duration":"9.000"},"streams":[{"codec_type":"video","codec_name":"h264","width":720,"height":480}]}),&r,1920,1080).is_err());
        assert!(serde_json::from_value::<ProductionArtifact>(
            json!({"kind":"composition","id":"x","path":"C:/escape"})
        )
        .is_err());
    }
}
