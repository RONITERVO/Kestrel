//! Producer-triggered endpoint generations. No model tools or model-selected placement.
use super::editor_timeline::{edit_hash, endpoint, replace_range};
use super::model_stream::{OpenAiSseDecoder, OpenAiStreamEvent};
use super::producer::{read_recoverable_json, write_recoverable_json};
use super::*;
use crate::models::{
    MovieEditorEndpoint, MovieEditorGenerateRequest, MovieEditorJob,
    MovieEditorJobStatus as Status, MovieEditorPlacement, MovieEditorRangeRequest,
    MovieEditorState,
};
use crate::{
    model::ModelInfo,
    runtime::{authorized, RuntimeManager},
};
use futures_util::StreamExt;

#[derive(Serialize, Deserialize)]
struct Record {
    schema_version: u32,
    job: MovieEditorJob,
    edit: MovieEdit,
    settings: MovieSettings,
}

impl MovieStudio {
    fn editor_dir(&self, project_id: &str, id: &str) -> Result<PathBuf, StudioError> {
        validate_id(project_id)?;
        uuid::Uuid::parse_str(id)
            .map_err(|_| StudioError::Invalid("Invalid editor generation identity.".into()))?;
        Ok(self
            .project_dir(project_id)
            .join("editor-generations")
            .join(id))
    }

    fn read_editor_record(&self, project_id: &str, id: &str) -> Result<Record, StudioError> {
        let path = self.editor_dir(project_id, id)?.join("job.json");
        for candidate in [&path, &path.with_extension("json.backup")] {
            if candidate.is_file() && candidate.metadata()?.len() > 16 * 1024 * 1024 {
                return Err(StudioError::Invalid(format!("Editor record {} exceeds 16 MiB. Preserve the file and open its recovery copy from the movie folder.", candidate.display())));
            }
        }
        let record: Record = read_recoverable_json(&path)?;
        if record.schema_version != 1
            || record.job.project_id != project_id
            || record.job.id != id
            || record.job.clip_id != format!("editor-{id}")
        {
            return Err(StudioError::Invalid("Editor generation record identity or version is unsupported. Its files were preserved.".into()));
        }
        Ok(record)
    }

    fn save_editor_record(
        &self,
        record: &mut Record,
        app: Option<&AppHandle>,
    ) -> Result<(), StudioError> {
        record.job.updated_at = Utc::now().to_rfc3339();
        if serde_json::to_vec_pretty(record)?.len() > 16 * 1024 * 1024 {
            return Err(StudioError::Invalid("This edit is too large to preserve in one generation record (16 MiB). Shorten clip notes or use a smaller movie before generating.".into()));
        }
        write_recoverable_json(
            &self
                .editor_dir(&record.job.project_id, &record.job.id)?
                .join("job.json"),
            record,
        )?;
        if let Some(app) = app {
            let _ = app.emit("movie-editor-job", &record.job);
        }
        Ok(())
    }

    pub fn editor_state(&self, id: &str) -> Result<MovieEditorState, StudioError> {
        let project = self.get(id)?;
        let jobs = self
            .editor_record_ids(id)?
            .into_iter()
            .take(50)
            .map(|job_id| {
                self.read_editor_record(id, &job_id)
                    .map(|record| record.job)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(MovieEditorState {
            edit_hash: edit_hash(&project.edit)?,
            jobs,
        })
    }

    fn editor_record_ids(&self, id: &str) -> Result<Vec<String>, StudioError> {
        validate_id(id)?;
        let dir = self.project_dir(id).join("editor-generations");
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut records = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if uuid::Uuid::parse_str(&name).is_err() {
                continue;
            }
            if records.len() >= MAX_MOVIE_SCENES as usize {
                return Err(StudioError::Invalid("This movie has reached 4096 editor generations. Start another movie to create more.".into()));
            }
            let path = entry.path().join("job.json");
            let modified = fs::metadata(&path)
                .or_else(|_| fs::metadata(path.with_extension("json.backup")))?
                .modified()?;
            records.push((name, modified));
        }
        // Read only a bounded recent window for the UI. Startup recovery streams the
        // remaining records one at a time instead of retaining thousands of frozen edits.
        records.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
        Ok(records.into_iter().map(|(name, _)| name).collect())
    }

    pub(super) fn recover_editor_generations(&self) -> Result<(), StudioError> {
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            if !entry.file_type()?.is_dir()
                || validate_id(&id).is_err()
                || !entry.path().join("project.json").is_file()
            {
                continue;
            }
            let mut project = self.get(&id)?;
            let mut changed = false;
            for job_id in self.editor_record_ids(&id)? {
                let mut record = self.read_editor_record(&id, &job_id)?;
                if matches!(
                    record.job.status,
                    Status::Preparing | Status::Writing | Status::Rendering
                ) {
                    record.job.status = Status::Stopped;
                    record.job.detail = "Kestrel closed during this generation. Saved frames and partial text are preserved. Start a new take when ready.".into();
                    self.save_editor_record(&mut record, None)?;
                }
                if !record.job.result_path.is_empty()
                    && Path::new(&record.job.result_path).is_file()
                    && !project
                        .clips
                        .iter()
                        .any(|clip| clip.id == record.job.clip_id)
                {
                    // A crash between the durable receipt and the project projection preserves an
                    // audition, but never resumes inference or silently applies a replacement.
                    project.clips.push(master(&record.job));
                    changed = true;
                }
            }
            if changed {
                self.save(&project)?;
            }
        }
        Ok(())
    }

    pub async fn prepare_editor_range(
        &self,
        request: MovieEditorRangeRequest,
        mut edit: MovieEdit,
    ) -> Result<MovieEditorJob, StudioError> {
        if !request.start_seconds.is_finite()
            || !request.end_seconds.is_finite()
            || request.start_seconds < 0.0
            || request.end_seconds <= request.start_seconds
            || !request.duration_seconds.is_finite()
            || !(1.0..=15.0).contains(&request.duration_seconds)
        {
            return Err(StudioError::Invalid("Select an increasing timeline range and a generated duration from 1 to 15 seconds.".into()));
        }
        if request.direction.trim().chars().count() < 3 || request.direction.len() > 16 * 1024 {
            return Err(StudioError::Invalid(
                "Describe the new action in 3 characters to 16 KiB.".into(),
            ));
        }
        let lock = self.project_lock(&request.project_id)?;
        let guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;
        if edit_hash(&project.edit)? != request.expected_edit_hash {
            return Err(StudioError::Invalid("The saved timeline changed. Reload the editor before selecting generation endpoints.".into()));
        }
        validate_movie_edit(&project, &mut edit)?;
        project.edit = edit;
        let first = endpoint(&project, request.start_seconds, false)?;
        let last = endpoint(&project, request.end_seconds, true)?;
        if self.editor_record_ids(&project.id)?.len() >= MAX_MOVIE_SCENES as usize {
            return Err(StudioError::Invalid("This movie has reached its editor generation limit. Start another movie to create more.".into()));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut record = Record {
            schema_version: 1,
            edit: project.edit.clone(),
            settings: project.settings.clone(),
            job: MovieEditorJob {
                id: id.clone(),
                project_id: project.id.clone(),
                status: Status::Preparing,
                detail: "Preserving selected endpoint frames…".into(),
                created_at: Utc::now().to_rfc3339(),
                updated_at: String::new(),
                edit_hash: edit_hash(&project.edit)?,
                start_seconds: request.start_seconds,
                end_seconds: request.end_seconds,
                duration_seconds: (request.duration_seconds * 24.0).round() / 24.0,
                direction: request.direction.trim().into(),
                placement: request.placement,
                render_prompt: String::new(),
                model_id: String::new(),
                seed: u64::from(uuid::Uuid::new_v4().as_fields().0),
                first,
                last,
                clip_id: format!("editor-{id}"),
                result_path: String::new(),
                result_sha256: String::new(),
                raw_path: String::new(),
                raw_sha256: String::new(),
                placed: false,
            },
        };
        // Check a proposed replacement before spending GPU time. Masters-only takes may be
        // generated even if the timeline has no room for extra split items.
        if request.placement == MovieEditorPlacement::ReplaceRange {
            let mut validation_project = project.clone();
            validation_project.clips.push(master(&record.job));
            let mut replacement = replace_range(
                &validation_project,
                request.start_seconds,
                request.end_seconds,
                decision(&record.job),
                record.job.duration_seconds,
            )?;
            validate_movie_edit(&validation_project, &mut replacement)?;
        }
        self.save_editor_record(&mut record, None)?;
        self.save(&project)?;
        drop(guard);
        let dir = self.editor_dir(&project.id, &id)?;
        let result = async {
            extract_endpoint(&mut record.job.first, &dir.join("first.png")).await?;
            extract_endpoint(&mut record.job.last, &dir.join("last.png")).await?;
            Ok::<(), StudioError>(())
        }
        .await;
        match result {
            Ok(()) => {
                record.job.status = Status::Ready;
                record.job.detail =
                    "Endpoint frames are ready. Generate a take from this saved range.".into();
            }
            Err(error) => {
                record.job.status = Status::Failed;
                record.job.detail = error.to_string();
                self.save_editor_record(&mut record, None)?;
                return Err(error);
            }
        }
        self.save_editor_record(&mut record, None)?;
        Ok(record.job)
    }

    pub fn validate_editor_generation(
        &self,
        request: &MovieEditorGenerateRequest,
        models: &[ModelInfo],
    ) -> Result<(), StudioError> {
        let record = self.read_editor_record(&request.project_id, &request.job_id)?;
        if record.job.status != Status::Ready {
            return Err(StudioError::Invalid("This take has already started. Prepare a new range to generate another immutable take.".into()));
        }
        if request.model_id.is_empty() {
            if request.render_prompt.trim().len() < 3
                || request.render_prompt.len() > MAX_MOVIE_PROMPT_BYTES
            {
                return Err(StudioError::Invalid(
                    "Supply an H3 prompt or choose a local writing model.".into(),
                ));
            }
        } else if !models.iter().any(|model| model.id == request.model_id) {
            return Err(StudioError::Invalid(
                "Choose an installed local writing model.".into(),
            ));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn generate_editor_take(
        &self,
        request: MovieEditorGenerateRequest,
        runtime: &Arc<RuntimeManager>,
        models: &[ModelInfo],
        settings: &ControlSettings,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<MovieEditorJob, StudioError> {
        self.validate_editor_generation(&request, models)?;
        let mut record = self.read_editor_record(&request.project_id, &request.job_id)?;
        let result = self
            .generate_editor_inner(
                &request,
                &mut record,
                runtime,
                models,
                settings,
                cancel,
                app,
            )
            .await;
        if let Err(error) = &result {
            record.job.status = if cancel.is_cancelled() {
                Status::Stopped
            } else {
                Status::Failed
            };
            record.job.detail = error.to_string();
            self.save_editor_record(&mut record, app)?;
        }
        self.release_comfy_memory().await;
        result?;
        Ok(record.job)
    }

    #[allow(clippy::too_many_arguments)]
    async fn generate_editor_inner(
        &self,
        request: &MovieEditorGenerateRequest,
        record: &mut Record,
        runtime: &Arc<RuntimeManager>,
        models: &[ModelInfo],
        settings: &ControlSettings,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<(), StudioError> {
        record.job.model_id = request.model_id.clone();
        record.job.render_prompt = request.render_prompt.trim().into();
        if !request.model_id.is_empty() {
            self.release_comfy_memory().await;
            record.job.status = Status::Writing;
            record.job.detail = "Expanding your direction using the selected scenes' written context. Exact frames condition H3 directly.".into();
            self.save_editor_record(record, app)?;
            self.write_editor_prompt(record, runtime, models, settings, cancel, app)
                .await?;
        }
        if cancel.is_cancelled() {
            return Err(StudioError::Cancelled);
        }
        runtime
            .stop_managed()
            .await
            .map_err(|error| StudioError::Render(error.to_string()))?;
        record.job.status = Status::Rendering;
        record.job.detail = "Generating between your preserved endpoint frames…".into();
        self.save_editor_record(record, app)?;
        self.live_previews.clear_movie(&record.job.project_id);
        let dir = self.editor_dir(&record.job.project_id, &record.job.id)?;
        self.ensure_comfy_process(&record.settings.comfy_root, &dir.join("logs"), Some(cancel))
            .await?;
        for frame in [&record.job.first, &record.job.last] {
            if hash_reference(Path::new(&frame.image_path))? != frame.image_sha256 {
                return Err(StudioError::Invalid("A preserved endpoint image changed on disk. Prepare the range again before generating.".into()));
            }
        }
        let mut project = self.get(&record.job.project_id)?;
        project.settings = record.settings.clone();
        project.references = vec![
            frame_reference(&record.job.first, "first"),
            frame_reference(&record.job.last, "last"),
        ];
        let planned = PlannedClip {
            id: record.job.clip_id.clone(),
            title: "Editor take".into(),
            purpose: record.job.direction.clone(),
            duration_seconds: record.job.duration_seconds,
            prompt: record.job.render_prompt.clone(),
            continuity_in: String::new(),
            continuity_out: String::new(),
            transition: String::new(),
            use_previous_frame: false,
            source_refs: vec![],
            reference_ids: vec![],
            first_frame_reference_id: "first".into(),
            last_frame_reference_id: "last".into(),
            reference_selections: vec![],
        };
        let path = self
            .render_clip(
                &project,
                &planned,
                0,
                record.job.seed,
                cancel,
                ClipRenderContext {
                    variant: Some(&record.job.id),
                    app,
                },
            )
            .await?;
        record.job.raw_sha256 = hash_reference(Path::new(&path))?;
        record.job.raw_path = path;
        self.save_editor_record(record, app)?;
        let finished = dir.join("take.mp4");
        super::editor_media::fit_take(
            Path::new(&record.job.raw_path),
            &finished,
            record.job.duration_seconds,
            cancel,
        )
        .await?;
        record.job.result_sha256 = hash_reference(&finished)?;
        record.job.result_path = finished.to_string_lossy().into_owned();
        // Save the master receipt before updating any project projection.
        self.save_editor_record(record, app)?;
        let lock = self.project_lock(&record.job.project_id)?;
        let _guard = lock.lock().await;
        let mut current = self.get(&record.job.project_id)?;
        current.clips.push(master(&record.job));
        self.persist_emit(&mut current, app)?;
        record.job.detail =
            "Take saved in Masters. Drag it into the timeline whenever you are ready.".into();
        if record.job.placement == MovieEditorPlacement::ReplaceRange {
            if edit_hash(&current.edit)? == record.job.edit_hash {
                let mut next = replace_range(
                    &current,
                    record.job.start_seconds,
                    record.job.end_seconds,
                    decision(&record.job),
                    record.job.duration_seconds,
                )?;
                validate_movie_edit(&current, &mut next)?;
                write_recoverable_json(&dir.join("edit-before-placement.json"), &current.edit)?;
                current.edit = next;
                record.job.placed = true;
                record.job.detail = "Take saved and the selected range replaced. The original timeline is preserved with this take.".into();
            } else {
                record.job.detail = "Take saved in Masters. The timeline changed while generating, so its current edits were preserved. Place the take manually.".into();
            }
        }
        self.persist_emit(&mut current, app)?;
        record.job.status = Status::Complete;
        self.save_editor_record(record, app)?;
        Ok(())
    }

    async fn write_editor_prompt(
        &self,
        record: &mut Record,
        runtime: &Arc<RuntimeManager>,
        models: &[ModelInfo],
        settings: &ControlSettings,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<(), StudioError> {
        let effective = settings.for_model(&record.job.model_id);
        let lease = tokio::select! {
            result = runtime.lease_model(&record.job.model_id, models, &effective, app) => result.map_err(|error| StudioError::Render(error.to_string()))?,
            _ = cancel.cancelled() => return Err(StudioError::Cancelled),
        };
        let body = json!({"model":lease.connection.model_id,"messages":[
            {"role":"system","content":"Write one concise H3 video prompt for the producer's requested action between two fixed frames. Output only the plain prompt, without a heading, Markdown, tools, JSON or commentary. Describe a physically coherent action, consistent camera and lighting. Choose exactly one camera instruction: a locked/static camera cannot also push in, pan, track or zoom. Repair contradictions in the old scene descriptions instead of copying them. Use two or three local timed beats from zero to the requested duration, sound and explicit no dialogue unless requested. Respect both fixed endpoints. The scene descriptions are written context, not observations of the actual frames; do not claim you inspected images. Do not choose references, files, render settings or placement. Do not invent new actors or props unrelated to the request."},
            {"role":"user","content":format!("Requested action: {}\nDuration: {} seconds\nStart scene context: {}\nEnd scene context: {}", record.job.direction, record.job.duration_seconds, record.job.first.scene_prompt, record.job.last.scene_prompt)}],
            "temperature":0.6,"top_p":0.9,"max_tokens":effective.max_output_tokens.clamp(1024,8192),"stream":true,
            "reasoning_effort":effective.thinking_level.as_str(),"chat_template_kwargs":{"enable_thinking":!effective.thinking_level.is_off(),"reasoning_effort":effective.thinking_level.as_template_effort()}});
        write_recoverable_json(
            &self
                .editor_dir(&record.job.project_id, &record.job.id)?
                .join("model-request.json"),
            &body,
        )?;
        let response = tokio::select! {
            result = authorized(self.http.post(format!("{}/chat/completions",lease.connection.endpoint)), &lease.connection).json(&body).send() => result?,
            _ = cancel.cancelled() => return Err(StudioError::Cancelled),
        };
        if !response.status().is_success() {
            return Err(StudioError::Render(format!(
                "Local writing model returned {}. Your range is preserved.",
                response.status()
            )));
        }
        let mut stream = response.bytes_stream();
        let mut decoder = OpenAiSseDecoder::default();
        let mut bytes = 0usize;
        let mut finished = false;
        let mut last_save = tokio::time::Instant::now();
        record.job.render_prompt.clear();
        loop {
            let chunk = tokio::select! { result = stream.next() => result, _ = cancel.cancelled() => return Err(StudioError::Cancelled) };
            let Some(chunk) = chunk else {
                break;
            };
            let chunk = chunk?;
            bytes += chunk.len();
            if bytes > 4 * 1024 * 1024 {
                return Err(StudioError::Render(
                    "The writing response exceeded its bounded stream limit.".into(),
                ));
            }
            for event in decoder.push(&chunk).map_err(StudioError::Render)? {
                accept_prompt_event(event, &mut record.job.render_prompt, &mut finished)?;
            }
            if last_save.elapsed() >= Duration::from_secs(1) {
                self.save_editor_record(record, app)?;
                last_save = tokio::time::Instant::now();
            }
        }
        for event in decoder.finish().map_err(StudioError::Render)? {
            accept_prompt_event(event, &mut record.job.render_prompt, &mut finished)?;
        }
        if !finished || record.job.render_prompt.trim().len() < 3 {
            return Err(StudioError::Render("The model did not finish a usable H3 prompt. Partial writing is preserved; create another take or supply your own prompt.".into()));
        }
        self.save_editor_record(record, app)?;
        Ok(())
    }
}

fn accept_prompt_event(
    event: OpenAiStreamEvent,
    output: &mut String,
    finished: &mut bool,
) -> Result<(), StudioError> {
    if let OpenAiStreamEvent::Message(value) = event {
        if value.pointer("/choices/0/delta/tool_calls").is_some() {
            return Err(StudioError::Render(
                "The writing model attempted a tool call. No model tools are available.".into(),
            ));
        }
        if let Some(token) = value
            .pointer("/choices/0/delta/content")
            .and_then(Value::as_str)
        {
            output.push_str(token);
        }
        if output.len() > MAX_MOVIE_PROMPT_BYTES {
            return Err(StudioError::Render("The H3 prompt exceeded 64 KiB.".into()));
        }
        if let Some(reason) = value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
        {
            if reason != "stop" {
                return Err(StudioError::Render(format!("The writing model stopped with {reason}; partial writing was preserved without rendering.")));
            }
            *finished = true;
        }
    }
    Ok(())
}

async fn extract_endpoint(
    endpoint: &mut MovieEditorEndpoint,
    target: &Path,
) -> Result<(), StudioError> {
    endpoint.source_sha256 = hash_reference(Path::new(&endpoint.source_path))?;
    let mut command = tokio::process::Command::new(media_program("ffmpeg"));
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-n",
            "-ss",
            &format!("{:.6}", endpoint.source_seconds),
            "-i",
        ])
        .arg(&endpoint.source_path)
        .args(["-frames:v", "1", "-an", "-update", "1"])
        .arg(target)
        .kill_on_drop(true);
    let result = tokio::time::timeout(Duration::from_secs(60), command.output()).await
        .map_err(|_| StudioError::Render("Extracting an endpoint exceeded 60 seconds. Check that its source video plays locally.".into()))??;
    if !result.status.success() || !target.is_file() {
        return Err(StudioError::Render(format!(
            "Could not preserve an endpoint frame: {}",
            truncate(&String::from_utf8_lossy(&result.stderr), 800)
        )));
    }
    endpoint.image_path = target.to_string_lossy().into_owned();
    endpoint.image_sha256 = hash_reference(target)?;
    Ok(())
}

fn frame_reference(endpoint: &MovieEditorEndpoint, id: &str) -> MovieReference {
    MovieReference {
        asset_id: id.into(),
        tag: String::new(),
        audio_tag: String::new(),
        name: format!("{id} endpoint"),
        kind: "image".into(),
        mime_type: "image/png".into(),
        bytes: 0,
        duration_seconds: 0.0,
        width: 0,
        height: 0,
        has_audio: false,
        path: endpoint.image_path.clone(),
        description: String::new(),
        use_embedded_audio: false,
        embedded_audio_description: String::new(),
        generation: None,
    }
}

fn master(job: &MovieEditorJob) -> RenderedClip {
    RenderedClip {
        id: job.clip_id.clone(),
        index: 0,
        title: format!(
            "Take · {}",
            truncate(
                &job.direction
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                80
            )
        ),
        prompt: job.render_prompt.clone(),
        duration_seconds: job.duration_seconds,
        seed: job.seed,
        status: "complete".into(),
        path: job.result_path.clone(),
        error: String::new(),
        versions: vec![],
    }
}

fn decision(job: &MovieEditorJob) -> ClipEdit {
    ClipEdit {
        id: format!("edit-{}", job.id),
        clip_id: job.clip_id.clone(),
        enabled: true,
        order: 0,
        trim_start: 0.0,
        trim_end: 0.0,
        audio_gain: 1.0,
        source_version_id: String::new(),
        speed: 1.0,
        fade_in: 0.0,
        fade_out: 0.0,
        audio_fade_in: 0.0,
        audio_fade_out: 0.0,
        label: truncate(
            &job.direction
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            80,
        ),
        notes: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn prompt_transport_rejects_tools_and_incomplete_output() {
        let mut output = String::new();
        let mut finished = false;
        accept_prompt_event(
            OpenAiStreamEvent::Message(json!({"choices":[{"delta":{"content":"A head turns."}}]})),
            &mut output,
            &mut finished,
        )
        .unwrap();
        assert!(!finished);
        assert!(accept_prompt_event(
            OpenAiStreamEvent::Message(json!({"choices":[{"delta":{},"finish_reason":"length"}]})),
            &mut output,
            &mut finished
        )
        .is_err());
        assert_eq!(output, "A head turns.");
        assert!(accept_prompt_event(
            OpenAiStreamEvent::Message(json!({"choices":[{"delta":{"tool_calls":[]}}]})),
            &mut output,
            &mut finished
        )
        .is_err());
    }

    #[test]
    fn restart_preserves_completed_master_and_never_applies_pending_replacement() {
        let root = tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let project = studio
            .create_producer_base(
                "Recovery example.".into(),
                MovieSettings::default(),
                vec![],
                "test",
                false,
            )
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let result = studio
            .project_dir(&project.id)
            .join("raw")
            .join("receipt-master.mp4");
        fs::write(&result, b"preserved movie").unwrap();
        let endpoint = MovieEditorEndpoint {
            edit_id: "edit-a".into(),
            clip_id: "source-a".into(),
            version_id: String::new(),
            source_path: String::new(),
            source_sha256: String::new(),
            source_seconds: 0.0,
            image_path: String::new(),
            image_sha256: String::new(),
            scene_prompt: String::new(),
        };
        let mut record = Record {
            schema_version: 1,
            edit: project.edit.clone(),
            settings: project.settings.clone(),
            job: MovieEditorJob {
                id: id.clone(),
                project_id: project.id.clone(),
                status: Status::Rendering,
                detail: String::new(),
                created_at: Utc::now().to_rfc3339(),
                updated_at: String::new(),
                edit_hash: edit_hash(&project.edit).unwrap(),
                start_seconds: 0.0,
                end_seconds: 1.0,
                duration_seconds: 5.0,
                placement: MovieEditorPlacement::ReplaceRange,
                direction: "Looks up".into(),
                render_prompt: "A complete prompt".into(),
                model_id: "local".into(),
                seed: 27,
                first: endpoint.clone(),
                last: endpoint,
                clip_id: format!("editor-{id}"),
                result_path: result.to_string_lossy().into_owned(),
                result_sha256: hash_reference(&result).unwrap(),
                raw_path: result.to_string_lossy().into_owned(),
                raw_sha256: hash_reference(&result).unwrap(),
                placed: false,
            },
        };
        studio.save_editor_record(&mut record, None).unwrap();
        let recovered = MovieStudio::new(root.path()).unwrap();
        let current = recovered.get(&project.id).unwrap();
        assert_eq!(current.edit, project.edit);
        assert_eq!(current.clips.len(), 1);
        assert_eq!(current.clips[0].seed, 27);
        let job = recovered.editor_state(&project.id).unwrap().jobs.remove(0);
        assert_eq!(job.status, Status::Stopped);
        assert!(!job.placed);
        assert_eq!(job.render_prompt, "A complete prompt");
        let reopened = MovieStudio::new(root.path()).unwrap();
        assert_eq!(reopened.get(&project.id).unwrap().clips.len(), 1);
    }

    #[tokio::test]
    #[ignore = "requires explicit acceptance library, local source movie, installed Qwen model and H3; generates a real between-frame take, observes preview frames, exports and verifies recovery"]
    async fn live_editor_qwen_h3_replacement_and_preview() {
        let output = PathBuf::from(
            std::env::var("KESTREL_ACCEPTANCE_LIBRARY").expect("set a separate acceptance library"),
        );
        let source = PathBuf::from(
            std::env::var("KESTREL_EDITOR_SOURCE_MOVIE")
                .expect("set a rendered source project.json"),
        );
        let model_id = std::env::var("KESTREL_LIVE_MODEL_ID").expect("select an installed model");
        let comfy = std::env::var("KESTREL_LIVE_COMFY_ROOT").expect("select installed ComfyUI");
        assert!(output.is_absolute() && source.is_absolute());
        let library = crate::store::default_research_root();
        assert_ne!(output, library);
        let mut settings = crate::config::ControlSettingsStore::new(&library)
            .load()
            .unwrap();
        settings.thinking_level = crate::models::ThinkingLevel::Low;
        settings.model_overrides.clear();
        let models = crate::model::ModelCatalogStore::new(&library)
            .load()
            .unwrap();
        let runtime = Arc::new(RuntimeManager::new());
        let studio = MovieStudio::new(&output).unwrap();
        let original: MovieProject = serde_json::from_slice(&fs::read(&source).unwrap()).unwrap();
        let mut project = studio
            .create_producer_base(
                "Between-frame editor acceptance.".into(),
                MovieSettings {
                    comfy_root: comfy,
                    ..MovieSettings::default()
                },
                vec![],
                "Qwen acceptance",
                false,
            )
            .unwrap();
        project.clips = original.clips.iter().take(3).cloned().collect();
        for clip in &mut project.clips {
            let target = studio
                .project_dir(&project.id)
                .join("raw")
                .join(format!("source-{}.mp4", clip.index));
            fs::copy(&clip.path, &target).unwrap();
            clip.path = target.to_string_lossy().into_owned();
        }
        project.edit = original.edit.clone();
        project.edit.export_title = "Between-frame acceptance".into();
        project.edit.export_preset = "review".into();
        studio.save(&project).unwrap();
        let hashes = project
            .clips
            .iter()
            .map(|clip| hash_reference(Path::new(&clip.path)).unwrap())
            .collect::<Vec<_>>();
        let job=studio.prepare_editor_range(MovieEditorRangeRequest { project_id:project.id.clone(),expected_edit_hash:edit_hash(&project.edit).unwrap(),start_seconds:1.0,end_seconds:2.0,duration_seconds:5.0,direction:"The lighthouse keeper gently lifts his head, glances aside and looks back. Quiet room ambience, no dialogue.".into(),placement:MovieEditorPlacement::ReplaceRange },project.edit.clone()).await.unwrap();
        assert_ne!(job.first.image_sha256, job.last.image_sha256);
        eprintln!("EDITOR_ACCEPTANCE_PROJECT={} JOB={}", project.id, job.id);
        let observe = studio.clone();
        let observe_id = project.id.clone();
        let stop_observe = CancellationToken::new();
        let observe_cancel = stop_observe.clone();
        let preview_task = tokio::spawn(async move {
            let mut frames = 0;
            loop {
                tokio::select! { _=observe_cancel.cancelled()=>break, _=tokio::time::sleep(Duration::from_millis(200))=>{ if observe.live_previews.movie(&observe_id).is_some_and(|preview|preview.data_url.is_some()) { frames+=1; } } }
            }
            frames
        });
        let result = studio
            .generate_editor_take(
                MovieEditorGenerateRequest {
                    project_id: project.id.clone(),
                    job_id: job.id.clone(),
                    model_id,
                    render_prompt: String::new(),
                },
                &runtime,
                &models,
                &settings,
                &CancellationToken::new(),
                None,
            )
            .await;
        let _ = runtime.stop_managed().await;
        stop_observe.cancel();
        let frames = preview_task.await.unwrap();
        let result = result.unwrap();
        assert!(result.placed);
        assert_eq!(result.status, Status::Complete);
        assert!(!result.render_prompt.is_empty());
        assert!(frames > 0, "H3 produced no approximate preview frames");
        let exported = studio.render_edit(&project.id).await.unwrap();
        assert!((exported.exports.last().unwrap().duration_seconds - 19.0).abs() < 0.01);
        for (clip, hash) in project.clips.iter().zip(hashes) {
            assert_eq!(hash_reference(Path::new(&clip.path)).unwrap(), hash);
        }
        let reopened = MovieStudio::new(&output).unwrap();
        let restored = reopened.get(&project.id).unwrap();
        assert_eq!(restored.edit, exported.edit);
        assert_eq!(restored.clips.len(), 4);
        let graph: Value = serde_json::from_slice(
            &fs::read(
                studio
                    .editor_dir(&project.id, &job.id)
                    .unwrap()
                    .join("graph.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(graph["5"]["inputs"]["first_frame"], json!(["15", 0]));
        assert_eq!(graph["5"]["inputs"]["last_frame"], json!(["16", 0]));
        eprintln!(
            "EDITOR_ACCEPTANCE_EXPORT={} PREVIEW_OBSERVATIONS={frames}",
            exported.final_path
        );
    }
}
