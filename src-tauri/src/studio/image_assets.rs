use super::live_preview::{preview_node, LivePreviewSession, PreviewTarget};
use super::{
    comfy_execution_error, truncate, write_json_atomic, MovieReferenceAsset, MovieStudio,
    StudioError, COMFY_BASE, COMFY_RENDER_TIMEOUT, MAX_MOVIE_PROMPT_BYTES,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;

const WORKFLOW_NAME: &str = "MiniMax H3 pseudo-image stable-frame workflow";
const WORKFLOW_SOURCE: &str =
    "https://huggingface.co/reverentelusarca/minimax-h3-comfyui-workflows";
const WORKFLOW_REVISION: &str = "1abf4a61eddffd08fa407e013ea7b7e62fbbbbf4";
const REQUESTED_LENGTH: u32 = 8;
const RESOLVED_FRAME_COUNT: u32 = 22;
const CANDIDATE_START: u32 = 8;
const CANDIDATE_COUNT: u32 = 6;
const MAX_GENERATION_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LISTED_GENERATIONS: usize = 50;
const STILLNESS_SUFFIX: &str = "Create a single static image composition. Keep identity, lettering, geometry, texture, lighting, and fine details stable across the internal frame pass. No camera movement, subject motion, or temporal progression.";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieImageAssetRequest {
    pub request_id: String,
    pub prompt: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    #[serde(default)]
    pub seed: u64,
    pub comfy_root: String,
    #[serde(default = "default_stabilize")]
    pub stabilize: bool,
}

fn default_stabilize() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImageProvenance {
    pub generation_id: String,
    pub workflow: String,
    pub workflow_source: String,
    pub workflow_revision: String,
    pub prompt: String,
    pub rendered_prompt: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: u64,
    pub requested_length: u32,
    pub resolved_frame_count: u32,
    pub frame_index: u32,
    pub sampler: String,
    pub scheduler: String,
    pub diffusion_model: String,
    pub text_encoder: String,
    pub vae: String,
    pub comfy_prompt_id: String,
    pub created_at: String,
    #[serde(default)]
    pub exact_graph: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieImageAssetCandidate {
    pub frame_index: u32,
    pub asset: MovieReferenceAsset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieImageAssetGeneration {
    pub id: String,
    pub status: String,
    pub stage: String,
    pub detail: String,
    pub prompt: String,
    pub rendered_prompt: String,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub seed: u64,
    pub stabilize: bool,
    pub workflow: String,
    pub workflow_source: String,
    pub workflow_revision: String,
    pub requested_length: u32,
    pub resolved_frame_count: u32,
    pub candidate_start: u32,
    pub candidate_count: u32,
    pub comfy_prompt_id: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub completed_at: String,
    #[serde(default)]
    pub error: String,
    #[serde(default)]
    pub candidates: Vec<MovieImageAssetCandidate>,
    #[serde(default)]
    pub exact_graph: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieImageAssetEvent {
    pub request_id: String,
    pub kind: String,
    pub stage: String,
    pub detail: String,
    pub progress: u8,
    pub at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<MovieImageAssetGeneration>,
}

pub fn emit_image_asset_error(app: &AppHandle, request_id: &str, error: impl ToString) {
    let _ = app.emit(
        "movie-image-asset",
        MovieImageAssetEvent {
            request_id: request_id.into(),
            kind: "error".into(),
            stage: "failed".into(),
            detail: error.to_string(),
            progress: 0,
            at: Utc::now().to_rfc3339(),
            generation: None,
        },
    );
}

impl MovieImageAssetRequest {
    pub fn validate(&self, advanced: bool) -> Result<(), StudioError> {
        if uuid::Uuid::parse_str(&self.request_id).is_err() {
            return Err(StudioError::Invalid(
                "image generation request ID must be a UUID".into(),
            ));
        }
        if self.prompt.trim().chars().count() < 3 || self.prompt.len() > MAX_MOVIE_PROMPT_BYTES {
            return Err(StudioError::Invalid(
                "image prompt must be between 3 characters and 64 KiB".into(),
            ));
        }
        if !self.width.is_multiple_of(32) || !self.height.is_multiple_of(32) {
            return Err(StudioError::Invalid(
                "image width and height must be multiples of 32".into(),
            ));
        }
        let max_edge = if advanced { 2_048 } else { 1_344 };
        let max_pixels = if advanced {
            2_048_u64 * 2_048
        } else {
            1_048_576
        };
        if self.width < 320
            || self.height < 320
            || self.width > max_edge
            || self.height > max_edge
            || u64::from(self.width) * u64::from(self.height) > max_pixels
        {
            return Err(StudioError::Invalid(format!(
                "image canvas must be 320-{max_edge} pixels per edge and within the safe local H3 pixel budget"
            )));
        }
        let max_steps = if advanced { 100 } else { 40 };
        if self.steps == 0 || self.steps > max_steps {
            return Err(StudioError::Invalid(format!(
                "image sampling steps must be between 1 and {max_steps}"
            )));
        }
        if !Path::new(&self.comfy_root).is_absolute() {
            return Err(StudioError::Invalid(
                "ComfyUI root must be an absolute local path".into(),
            ));
        }
        Ok(())
    }
}

impl MovieStudio {
    pub fn list_image_asset_generations(
        &self,
    ) -> Result<Vec<MovieImageAssetGeneration>, StudioError> {
        let root = self.image_generation_root();
        fs::create_dir_all(&root)?;
        let mut generations: Vec<MovieImageAssetGeneration> = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path().join("generation.json");
            if !path.is_file() {
                continue;
            }
            let size = path.metadata()?.len();
            if size > MAX_GENERATION_MANIFEST_BYTES {
                return Err(StudioError::Invalid(format!(
                    "generated-image receipt is unexpectedly large: {}",
                    path.display()
                )));
            }
            let mut generation: MovieImageAssetGeneration =
                serde_json::from_slice(&fs::read(&path)?)?;
            for candidate in &mut generation.candidates {
                candidate.asset = self.resolve_reference_asset(&candidate.asset.id)?;
            }
            generations.push(generation);
        }
        generations.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        generations.truncate(MAX_LISTED_GENERATIONS);
        Ok(generations)
    }

    pub(super) fn recover_image_asset_generations(&self) -> Result<(), StudioError> {
        let root = self.image_generation_root();
        fs::create_dir_all(&root)?;
        for entry in fs::read_dir(root)?.filter_map(Result::ok) {
            let path = entry.path().join("generation.json");
            let Ok(bytes) = fs::read(&path) else { continue };
            if bytes.len() as u64 > MAX_GENERATION_MANIFEST_BYTES {
                continue;
            }
            let Ok(mut generation) = serde_json::from_slice::<MovieImageAssetGeneration>(&bytes)
            else {
                continue;
            };
            if generation.status == "running" {
                generation.status = "interrupted".into();
                generation.stage = "interrupted".into();
                generation.detail = "Kestrel closed during the local image pass. No candidate was attached automatically; generate again when ready.".into();
                generation.error = "The prior image pass was interrupted before Kestrel recorded a complete candidate set.".into();
                generation.updated_at = Utc::now().to_rfc3339();
                write_json_atomic(&path, &generation)?;
            }
        }
        Ok(())
    }

    pub async fn generate_image_assets(
        &self,
        request: MovieImageAssetRequest,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<MovieImageAssetGeneration, StudioError> {
        let seed = resolved_seed(request.seed);
        let rendered_prompt = if request.stabilize {
            format!("{}\n\n{}", request.prompt.trim(), STILLNESS_SUFFIX)
        } else {
            request.prompt.trim().to_string()
        };
        let prefix = format!("kestrel_images/{}/candidate", request.request_id);
        let graph = pseudo_image_graph(
            &rendered_prompt,
            request.width,
            request.height,
            request.steps,
            seed,
            &prefix,
        );
        let now = Utc::now().to_rfc3339();
        let mut generation = MovieImageAssetGeneration {
            id: request.request_id.clone(),
            status: "running".into(),
            stage: "preparing".into(),
            detail: "Preparing the private H3 image workflow.".into(),
            prompt: request.prompt.clone(),
            rendered_prompt: rendered_prompt.clone(),
            width: request.width,
            height: request.height,
            steps: request.steps,
            seed,
            stabilize: request.stabilize,
            workflow: WORKFLOW_NAME.into(),
            workflow_source: WORKFLOW_SOURCE.into(),
            workflow_revision: WORKFLOW_REVISION.into(),
            requested_length: REQUESTED_LENGTH,
            resolved_frame_count: RESOLVED_FRAME_COUNT,
            candidate_start: CANDIDATE_START,
            candidate_count: CANDIDATE_COUNT,
            comfy_prompt_id: String::new(),
            created_at: now.clone(),
            updated_at: now,
            completed_at: String::new(),
            error: String::new(),
            candidates: Vec::new(),
            exact_graph: graph.clone(),
        };
        let folder = self.image_generation_dir(&generation.id);
        fs::create_dir_all(folder.join("logs"))?;
        write_json_atomic(&folder.join("request.json"), &request)?;
        write_json_atomic(&folder.join("graph.json"), &graph)?;
        self.save_image_generation(&generation)?;
        emit_image_event(app, &generation, "started", 2, None);

        let result = self
            .run_image_asset_graph(&request, &mut generation, cancel, app)
            .await;
        match result {
            Ok(candidates) => {
                generation.status = "complete".into();
                generation.stage = "ready".into();
                generation.detail = format!(
                    "{} stable-frame candidates are ready. Choose the one that best preserves the intended identity and detail.",
                    candidates.len()
                );
                generation.candidates = candidates;
                generation.completed_at = Utc::now().to_rfc3339();
                generation.updated_at.clone_from(&generation.completed_at);
                self.save_image_generation(&generation)?;
                emit_image_event(app, &generation, "complete", 100, Some(generation.clone()));
                Ok(generation)
            }
            Err(error) => {
                let cancelled = matches!(&error, StudioError::Cancelled);
                generation.status = if cancelled { "cancelled" } else { "failed" }.into();
                generation.stage.clone_from(&generation.status);
                generation.detail = if cancelled {
                    "Image generation stopped. No candidate was attached to the movie.".into()
                } else {
                    "Image generation stopped with a recoverable error.".into()
                };
                generation.error = error.to_string();
                generation.updated_at = Utc::now().to_rfc3339();
                self.save_image_generation(&generation)?;
                emit_image_event(
                    app,
                    &generation,
                    if cancelled { "cancelled" } else { "error" },
                    0,
                    Some(generation.clone()),
                );
                Err(error)
            }
        }
    }

    async fn run_image_asset_graph(
        &self,
        request: &MovieImageAssetRequest,
        generation: &mut MovieImageAssetGeneration,
        cancel: &CancellationToken,
        app: Option<&AppHandle>,
    ) -> Result<Vec<MovieImageAssetCandidate>, StudioError> {
        check_image_cancel(cancel)?;
        generation.stage = "starting-renderer".into();
        generation.detail = "Starting the private local ComfyUI renderer if needed.".into();
        generation.updated_at = Utc::now().to_rfc3339();
        self.save_image_generation(generation)?;
        emit_image_event(app, generation, "progress", 5, None);
        self.ensure_comfy_process(
            &request.comfy_root,
            &self.image_generation_dir(&generation.id).join("logs"),
            Some(cancel),
        )
        .await?;

        check_image_cancel(cancel)?;
        generation.stage = "queued".into();
        generation.detail = format!(
            "H3 is generating {} internal frames and will preserve frames {}-{} as candidates.",
            RESOLVED_FRAME_COUNT,
            CANDIDATE_START,
            CANDIDATE_START + CANDIDATE_COUNT - 1
        );
        generation.updated_at = Utc::now().to_rfc3339();
        self.save_image_generation(generation)?;
        emit_image_event(app, generation, "progress", 10, None);
        let client_id = format!("kestrel-image-{}", generation.id);
        let preview = LivePreviewSession::connect(
            app,
            &client_id,
            PreviewTarget::image_asset(&generation.id),
        )
        .await;
        let response = self
            .http
            .post(format!("{COMFY_BASE}/prompt"))
            .json(&json!({
                "prompt": generation.exact_graph,
                "client_id": client_id,
            }))
            .send()
            .await?;
        let status = response.status();
        let value: Value = response.json().await?;
        if !status.is_success() {
            return Err(StudioError::Render(format!(
                "ComfyUI rejected the image workflow: {}",
                truncate(&value.to_string(), 700)
            )));
        }
        generation.comfy_prompt_id = value
            .get("prompt_id")
            .and_then(Value::as_str)
            .ok_or_else(|| StudioError::Render(format!("ComfyUI returned no prompt id: {value}")))?
            .to_string();
        generation.stage = "rendering".into();
        generation.detail = "H3 is rendering the stable-frame pass locally.".into();
        generation.updated_at = Utc::now().to_rfc3339();
        self.save_image_generation(generation)?;
        emit_image_event(app, generation, "progress", 15, None);

        let deadline = tokio::time::Instant::now() + COMFY_RENDER_TIMEOUT;
        let mut last_heartbeat = tokio::time::Instant::now();
        loop {
            if cancel.is_cancelled() {
                let _ = self
                    .http
                    .post(format!("{COMFY_BASE}/interrupt"))
                    .send()
                    .await;
                return Err(StudioError::Cancelled);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(StudioError::Render(format!(
                    "ComfyUI did not finish image prompt {} within 24 hours. The durable receipt is preserved; verify the local ComfyUI queue before trying again.",
                    generation.comfy_prompt_id
                )));
            }
            let history: Value = self
                .http
                .get(format!(
                    "{COMFY_BASE}/history/{}",
                    generation.comfy_prompt_id
                ))
                .send()
                .await?
                .json()
                .await?;
            if let Some(entry) = history.get(&generation.comfy_prompt_id) {
                if entry.pointer("/status/status_str").and_then(Value::as_str) == Some("error") {
                    let detail = comfy_execution_error(entry).unwrap_or_else(|| {
                        format!("execution failed: {}", truncate(&entry.to_string(), 1_000))
                    });
                    return Err(StudioError::Render(format!("ComfyUI {detail}")));
                }
                if entry.pointer("/status/completed").and_then(Value::as_bool) == Some(true) {
                    if let Some(preview) = &preview {
                        preview.finish();
                    }
                    generation.stage = "preserving".into();
                    generation.detail =
                        "Preserving every candidate in Kestrel's private, content-addressed library."
                            .into();
                    generation.updated_at = Utc::now().to_rfc3339();
                    self.save_image_generation(generation)?;
                    emit_image_event(app, generation, "progress", 92, None);
                    return self.preserve_image_candidates(request, generation, entry);
                }
            }
            if last_heartbeat.elapsed() >= Duration::from_secs(10) {
                emit_image_event(app, generation, "progress", 35, None);
                last_heartbeat = tokio::time::Instant::now();
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    fn preserve_image_candidates(
        &self,
        request: &MovieImageAssetRequest,
        generation: &MovieImageAssetGeneration,
        history: &Value,
    ) -> Result<Vec<MovieImageAssetCandidate>, StudioError> {
        let outputs = find_candidate_outputs(history);
        if outputs.is_empty() {
            return Err(StudioError::Render(
                "completed H3 image workflow exposed no saved candidate images".into(),
            ));
        }
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        for (offset, (filename, subfolder)) in outputs.into_iter().enumerate() {
            let frame_index = CANDIDATE_START + offset as u32;
            let source = safe_comfy_output(&request.comfy_root, &subfolder, &filename)?;
            let provenance = GeneratedImageProvenance {
                generation_id: generation.id.clone(),
                workflow: generation.workflow.clone(),
                workflow_source: generation.workflow_source.clone(),
                workflow_revision: generation.workflow_revision.clone(),
                prompt: generation.prompt.clone(),
                rendered_prompt: generation.rendered_prompt.clone(),
                width: generation.width,
                height: generation.height,
                steps: generation.steps,
                seed: generation.seed,
                requested_length: generation.requested_length,
                resolved_frame_count: generation.resolved_frame_count,
                frame_index,
                sampler: "euler".into(),
                scheduler: "simple".into(),
                diffusion_model: "minimax_h3_fl2va_pruned_int8_convrot.safetensors".into(),
                text_encoder: "qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors".into(),
                vae: "minimax_h3_video_vae_fp16.safetensors".into(),
                comfy_prompt_id: generation.comfy_prompt_id.clone(),
                created_at: Utc::now().to_rfc3339(),
                exact_graph: generation.exact_graph.clone(),
            };
            let mut asset = self.import_reference_path(&source)?;
            if !seen.insert(asset.id.clone()) {
                continue;
            }
            asset.name = format!("Generated H3 image - frame {frame_index:02}");
            asset.generation = Some(provenance);
            write_json_atomic(
                &self
                    .root
                    .join("_references")
                    .join("meta")
                    .join(format!("{}.json", asset.id)),
                &asset,
            )?;
            candidates.push(MovieImageAssetCandidate { frame_index, asset });
        }
        if candidates.is_empty() {
            return Err(StudioError::Render(
                "H3 returned duplicate or unreadable image candidates".into(),
            ));
        }
        Ok(candidates)
    }

    fn image_generation_root(&self) -> PathBuf {
        self.root.join("_references").join("generations")
    }

    fn image_generation_dir(&self, id: &str) -> PathBuf {
        self.image_generation_root().join(id)
    }

    fn save_image_generation(
        &self,
        generation: &MovieImageAssetGeneration,
    ) -> Result<(), StudioError> {
        let folder = self.image_generation_dir(&generation.id);
        fs::create_dir_all(&folder)?;
        write_json_atomic(&folder.join("generation.json"), generation)
    }
}

fn pseudo_image_graph(
    prompt: &str,
    width: u32,
    height: u32,
    steps: u32,
    seed: u64,
    prefix: &str,
) -> Value {
    let mut graph = json!({
        "1":{"class_type":"UNETLoader","inputs":{"unet_name":"minimax_h3_fl2va_pruned_int8_convrot.safetensors","weight_dtype":"default"}},
        "2":{"class_type":"CLIPLoader","inputs":{"clip_name":"qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors","type":"minimax","device":"default"}},
        "3":{"class_type":"VAELoader","inputs":{"vae_name":"minimax_h3_video_vae_fp16.safetensors"}},
        "4":{"class_type":"MiniMaxH3ImageToVideo","inputs":{"clip":["2",0],"vae":["3",0],"prompt":prompt,"width":width,"height":height,"length":REQUESTED_LENGTH}},
        "5":{"class_type":"RandomNoise","inputs":{"noise_seed":seed}},
        "6":{"class_type":"BasicScheduler","inputs":{"model":["90",0],"scheduler":"simple","steps":steps,"denoise":1.0}},
        "7":{"class_type":"KSamplerSelect","inputs":{"sampler_name":"euler"}},
        "8":{"class_type":"BasicGuider","inputs":{"model":["90",0],"conditioning":["4",0]}},
        "9":{"class_type":"SamplerCustomAdvanced","inputs":{"noise":["5",0],"guider":["8",0],"sampler":["7",0],"sigmas":["6",0],"latent_image":["4",1]}},
        "10":{"class_type":"VAEDecode","inputs":{"samples":["9",0],"vae":["3",0]}},
        "11":{"class_type":"ImageFromBatch","inputs":{"image":["10",0],"batch_index":CANDIDATE_START,"length":CANDIDATE_COUNT}},
        "12":{"class_type":"SaveImage","inputs":{"images":["11",0],"filename_prefix":prefix}}
    });
    graph["90"] = preview_node("1", 6);
    graph
}

fn find_candidate_outputs(entry: &Value) -> Vec<(String, String)> {
    entry
        .pointer("/outputs/12/images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|image| {
            let filename = image.get("filename")?.as_str()?.to_string();
            let subfolder = image
                .get("subfolder")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some((filename, subfolder))
        })
        .take(CANDIDATE_COUNT as usize)
        .collect()
}

fn safe_comfy_output(root: &str, subfolder: &str, filename: &str) -> Result<PathBuf, StudioError> {
    let filename_path = Path::new(filename);
    if filename_path.file_name().and_then(|value| value.to_str()) != Some(filename) {
        return Err(StudioError::Render(
            "ComfyUI returned an unsafe image filename".into(),
        ));
    }
    let subfolder_path = Path::new(subfolder);
    if subfolder_path.is_absolute()
        || subfolder_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(StudioError::Render(
            "ComfyUI returned an unsafe image output folder".into(),
        ));
    }
    let output_root = PathBuf::from(root).join("output").canonicalize()?;
    let source = output_root
        .join(subfolder_path)
        .join(filename)
        .canonicalize()?;
    if !source.starts_with(&output_root) || !source.is_file() {
        return Err(StudioError::Render(
            "ComfyUI image output is outside the configured private renderer".into(),
        ));
    }
    Ok(source)
}

fn resolved_seed(seed: u64) -> u64 {
    if seed == 0 {
        Utc::now().timestamp_nanos_opt().unwrap_or_default() as u64 & i64::MAX as u64
    } else {
        seed & i64::MAX as u64
    }
}

fn check_image_cancel(cancel: &CancellationToken) -> Result<(), StudioError> {
    if cancel.is_cancelled() {
        Err(StudioError::Cancelled)
    } else {
        Ok(())
    }
}

fn emit_image_event(
    app: Option<&AppHandle>,
    generation: &MovieImageAssetGeneration,
    kind: &str,
    progress: u8,
    completed: Option<MovieImageAssetGeneration>,
) {
    let Some(app) = app else { return };
    let _ = app.emit(
        "movie-image-asset",
        MovieImageAssetEvent {
            request_id: generation.id.clone(),
            kind: kind.into(),
            stage: generation.stage.clone(),
            detail: generation.detail.clone(),
            progress,
            at: Utc::now().to_rfc3339(),
            generation: completed,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> MovieImageAssetRequest {
        MovieImageAssetRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            prompt: "A precise identity portrait for the lead character.".into(),
            width: 768,
            height: 1_344,
            steps: 20,
            seed: 7,
            comfy_root: r"D:\AI\ComfyUI".into(),
            stabilize: true,
        }
    }

    #[test]
    fn producer_presets_fit_the_safe_offline_canvas() {
        for (width, height) in [(1_344, 768), (768, 1_344), (1_024, 1_024)] {
            let mut value = request();
            value.width = width;
            value.height = height;
            value.validate(false).unwrap();
        }
    }

    #[test]
    fn pseudo_image_graph_matches_the_researched_stable_frame_pass() {
        let graph = pseudo_image_graph("test", 1_344, 768, 20, 7, "kestrel/test");
        assert_eq!(graph["4"]["inputs"]["length"], REQUESTED_LENGTH);
        assert_eq!(graph["7"]["inputs"]["sampler_name"], "euler");
        assert_eq!(graph["11"]["inputs"]["batch_index"], CANDIDATE_START);
        assert_eq!(graph["11"]["inputs"]["length"], CANDIDATE_COUNT);
        assert_eq!(graph["12"]["class_type"], "SaveImage");
        assert_eq!(graph["90"]["class_type"], "ModelPreviewOverrideKJ");
        assert_eq!(graph["90"]["inputs"]["tiny_vae"], "taeh3.safetensors");
        assert_eq!(graph["6"]["inputs"]["model"], json!(["90", 0]));
        assert_eq!(graph["8"]["inputs"]["model"], json!(["90", 0]));
        assert!(graph.get("CreateVideo").is_none());
    }

    #[test]
    fn image_prompt_keeps_the_same_long_form_limit_as_the_movie_brief() {
        let mut value = request();
        value.prompt = "x".repeat(MAX_MOVIE_PROMPT_BYTES);
        value.validate(false).unwrap();
        value.prompt.push('x');
        assert!(value.validate(false).is_err());
    }

    #[tokio::test]
    #[ignore = "requires the installed MiniMax H3 ComfyUI stack and several minutes"]
    async fn live_h3_image_pass_preserves_selectable_candidates_and_provenance() {
        use futures_util::StreamExt as _;

        let library = tempfile::tempdir().unwrap();
        let studio = MovieStudio::new(library.path()).unwrap();
        let mut value = request();
        value.comfy_root =
            std::env::var("KESTREL_LIVE_COMFY_ROOT").unwrap_or_else(|_| r"D:\AI\ComfyUI".into());
        value.width = 512;
        value.height = 512;
        value.prompt = "A single handmade brass compass on a dark green linen field, centered product reference, precise engraved details, soft north-window light, no hands, no labels, no motion.".into();
        let client_id = format!("kestrel-image-{}", value.request_id);
        let (socket, _) = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:8188/ws?clientId={client_id}"
        ))
        .await
        .unwrap();
        let (_, mut reader) = socket.split();
        let preview = tokio::spawn(async move {
            while let Some(message) = reader.next().await {
                let message = message.unwrap();
                if !message.is_text() {
                    continue;
                }
                let value: Value = serde_json::from_str(&message.into_text().unwrap()).unwrap();
                if value.get("type").and_then(Value::as_str) == Some("kj_preview_override") {
                    return value;
                }
            }
            panic!("ComfyUI closed before emitting a live H3 preview")
        });
        let generation = studio
            .generate_image_assets(value, &CancellationToken::new(), None)
            .await
            .unwrap();
        let preview = tokio::time::timeout(Duration::from_secs(30), preview)
            .await
            .expect("live preview did not arrive")
            .unwrap();
        assert_eq!(
            preview.pointer("/data/node_id").and_then(Value::as_str),
            Some("90")
        );
        assert!(preview
            .pointer("/data/image")
            .and_then(Value::as_str)
            .is_some_and(|value| value.len() > 1_000));
        assert_eq!(generation.status, "complete");
        assert!(!generation.comfy_prompt_id.is_empty());
        assert!(!generation.candidates.is_empty());
        assert!(generation.candidates.len() <= CANDIDATE_COUNT as usize);
        for candidate in &generation.candidates {
            assert!(Path::new(&candidate.asset.path).is_file());
            let provenance = candidate.asset.generation.as_ref().unwrap();
            assert_eq!(provenance.generation_id, generation.id);
            assert_eq!(provenance.seed, generation.seed);
            assert_eq!(provenance.exact_graph, generation.exact_graph);
        }
    }
}
