//! Durable, bounded offline video planning and ComfyUI execution.
//!
//! The local language model creates only a compact story bible. Native code expands it into a
//! potentially multi-hour queue, owns the exact ComfyUI memory profile, serializes generations,
//! verifies every copied artifact, and persists enough state to resume after a restart.

use crate::{
    models::ControlSettings,
    runtime::{authorized, RuntimeManager},
};
use base64::Engine;
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tokio::{process::Command, sync::Mutex};
use tokio_util::sync::CancellationToken;

const VIDEO_ENDPOINT: &str = "http://127.0.0.1:8188";
const MAX_PROJECT_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_REFERENCES: usize = 4_096;
const MAX_PROMPT_CHARS: usize = 32_768;
const MAX_CLIPS: u32 = 20_000;
const MAX_TOTAL_SECONDS: u32 = 43_200;
const MAX_CHAPTERS: usize = 48;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VideoPreset {
    Wan13GpuOnly,
    WanVace13Reference,
    KandinskyDistilled,
    KandinskySft,
    Wan22Offload,
}

impl VideoPreset {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Wan13GpuOnly => "wan-1.3b-gpu-only",
            Self::WanVace13Reference => "wan-vace-1.3b-reference",
            Self::KandinskyDistilled => "kandinsky-distilled",
            Self::KandinskySft => "kandinsky-sft",
            Self::Wan22Offload => "wan-2.2-5b-offload",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Wan13GpuOnly => "Wan 2.1 1.3B · GPU only",
            Self::WanVace13Reference => "Wan VACE 1.3B · Reference studio",
            Self::KandinskyDistilled => "Kandinsky 5 Lite · Distilled",
            Self::KandinskySft => "Kandinsky 5 Lite · SFT quality",
            Self::Wan22Offload => "Wan 2.2 5B · Predictable offload",
        }
    }

    fn profile(&self) -> VideoMemoryProfile {
        match self {
            Self::Wan13GpuOnly => VideoMemoryProfile::GpuOnly,
            Self::WanVace13Reference => VideoMemoryProfile::ReferenceResident,
            Self::KandinskyDistilled | Self::KandinskySft => VideoMemoryProfile::KandinskyResident,
            Self::Wan22Offload => VideoMemoryProfile::ForcedOffload,
        }
    }

    fn native_seconds(&self) -> u32 {
        match self {
            Self::Wan13GpuOnly => 2,
            Self::WanVace13Reference => 5,
            Self::KandinskyDistilled | Self::KandinskySft => 5,
            Self::Wan22Offload => 3,
        }
    }

    fn fps(&self) -> u32 {
        match self {
            Self::Wan13GpuOnly | Self::WanVace13Reference => 16,
            _ => 24,
        }
    }

    fn frames(&self) -> u32 {
        match self {
            Self::Wan13GpuOnly => 33,
            Self::WanVace13Reference => 81,
            Self::KandinskyDistilled | Self::KandinskySft => 121,
            Self::Wan22Offload => 81,
        }
    }

    fn steps(&self) -> u32 {
        match self {
            Self::Wan13GpuOnly => 30,
            Self::WanVace13Reference => 30,
            Self::KandinskyDistilled => 16,
            Self::KandinskySft => 100,
            Self::Wan22Offload => 20,
        }
    }

    fn cfg(&self) -> f64 {
        match self {
            Self::Wan13GpuOnly | Self::WanVace13Reference => 6.0,
            Self::KandinskyDistilled => 1.0,
            Self::KandinskySft | Self::Wan22Offload => 5.0,
        }
    }

    fn required_files(&self) -> &'static [&'static str] {
        match self {
            Self::Wan13GpuOnly => &[
                "models/diffusion_models/wan2.1_t2v_1.3B_bf16.safetensors",
                "models/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                "models/vae/wan_2.1_vae.safetensors",
            ],
            Self::WanVace13Reference => &[
                "models/diffusion_models/wan2.1_vace_1.3B_fp16.safetensors",
                "models/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                "models/vae/wan_2.1_vae.safetensors",
            ],
            Self::KandinskyDistilled => &[
                "models/diffusion_models/kandinsky5lite_t2v_distilled16steps_5s.safetensors",
                "models/text_encoders/qwen_2.5_vl_7b_fp8_scaled.safetensors",
                "models/text_encoders/clip_l.safetensors",
                "models/vae/hunyuan_video_vae_bf16.safetensors",
            ],
            Self::KandinskySft => &[
                "models/diffusion_models/kandinsky5lite_t2v_sft_5s.safetensors",
                "models/text_encoders/qwen_2.5_vl_7b_fp8_scaled.safetensors",
                "models/text_encoders/clip_l.safetensors",
                "models/vae/hunyuan_video_vae_bf16.safetensors",
            ],
            Self::Wan22Offload => &[
                "models/diffusion_models/wan2.2_ti2v_5B_fp16.safetensors",
                "models/text_encoders/umt5_xxl_fp8_e4m3fn_scaled.safetensors",
                "models/vae/wan2.2_vae.safetensors",
            ],
        }
    }

    fn supports_image_reference(&self) -> bool {
        !matches!(self, Self::Wan13GpuOnly)
    }

    fn supports_video_reference(&self) -> bool {
        matches!(self, Self::WanVace13Reference)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VideoMemoryProfile {
    GpuOnly,
    ReferenceResident,
    KandinskyResident,
    ForcedOffload,
}

impl VideoMemoryProfile {
    fn id(self) -> &'static str {
        match self {
            Self::GpuOnly => "gpu-only",
            Self::ReferenceResident => "reference-staged",
            Self::KandinskyResident => "kandinsky-staged",
            Self::ForcedOffload => "forced-offload",
        }
    }

    fn offloading(self) -> &'static str {
        match self {
            Self::GpuOnly => "forbidden",
            Self::ReferenceResident => "stage-boundary-only",
            Self::KandinskyResident => "stage-boundary-only",
            Self::ForcedOffload => "forced",
        }
    }

    fn args(self) -> &'static [&'static str] {
        match self {
            Self::GpuOnly => &[
                "--gpu-only",
                "--disable-async-offload",
                "--disable-dynamic-vram",
                "--reserve-vram",
                "0.25",
                "--cache-none",
            ],
            Self::KandinskyResident => &[
                "--disable-async-offload",
                "--disable-dynamic-vram",
                "--disable-smart-memory",
                "--reserve-vram",
                "0.75",
                "--cache-lru",
                "2",
            ],
            Self::ReferenceResident => &[
                "--disable-async-offload",
                "--disable-dynamic-vram",
                "--disable-smart-memory",
                "--reserve-vram",
                "0.75",
                "--cache-lru",
                "2",
            ],
            Self::ForcedOffload => &[
                "--lowvram",
                "--async-offload",
                "2",
                "--disable-dynamic-vram",
                "--reserve-vram",
                "1.0",
                "--cache-none",
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VideoSettings {
    pub comfy_root: String,
    pub ffmpeg_path: String,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            comfy_root: r"D:\AI\ComfyUI".into(),
            ffmpeg_path: "ffmpeg.exe".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoBoundarySettings {
    pub max_clips: u32,
    pub max_retries_per_clip: u32,
    pub max_failed_clips: u32,
    pub max_runtime_minutes: u32,
    pub min_free_disk_gib: u32,
    pub assemble_final_video: bool,
}

impl Default for VideoBoundarySettings {
    fn default() -> Self {
        Self {
            max_clips: 500,
            max_retries_per_clip: 2,
            max_failed_clips: 3,
            max_runtime_minutes: 720,
            min_free_disk_gib: 20,
            assemble_final_video: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VideoReferenceKind {
    Image,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum VideoReferenceRole {
    Subject,
    Storyboard,
    Motion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoReferenceAsset {
    pub id: String,
    pub name: String,
    pub kind: VideoReferenceKind,
    pub role: VideoReferenceRole,
    pub stored_path: String,
    pub bytes: u64,
    pub sha256: String,
    pub created_at: String,
    #[serde(default)]
    pub preview_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum VideoContinuityMode {
    #[default]
    None,
    Anchor,
    PreviousFrame,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct VideoContinuitySettings {
    pub mode: VideoContinuityMode,
    pub primary_reference_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoPlanRequest {
    pub prompt: String,
    pub audience: String,
    pub use_case: String,
    pub planner_model_id: Option<String>,
    pub preset: VideoPreset,
    pub total_duration_seconds: u32,
    pub orientation: String,
    pub negative_prompt: String,
    pub boundaries: VideoBoundarySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoChapter {
    pub index: u32,
    pub title: String,
    pub narrative_goal: String,
    pub prompt_seed: String,
    pub first_clip: u32,
    pub last_clip: u32,
    #[serde(default)]
    pub reference_asset_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoClip {
    pub index: u32,
    pub chapter_index: u32,
    pub prompt: String,
    pub seed: u64,
    pub status: String,
    pub attempts: u32,
    pub comfy_prompt_id: Option<String>,
    pub output_path: Option<String>,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub error: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    #[serde(default)]
    pub reference_asset_id: Option<String>,
    #[serde(default)]
    pub continuity_frame_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoProject {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub audience: String,
    pub use_case: String,
    pub preset: VideoPreset,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub total_duration_seconds: u32,
    pub clip_duration_seconds: u32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub frames_per_clip: u32,
    pub steps: u32,
    pub cfg: f64,
    pub negative_prompt: String,
    pub continuity_bible: String,
    pub planning_note: String,
    pub chapters: Vec<VideoChapter>,
    pub clips: Vec<VideoClip>,
    pub boundaries: VideoBoundarySettings,
    pub output_directory: String,
    pub final_output_path: Option<String>,
    pub errors: Vec<String>,
    #[serde(default)]
    pub references: Vec<VideoReferenceAsset>,
    #[serde(default)]
    pub continuity: VideoContinuitySettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoProjectSummary {
    pub id: String,
    pub title: String,
    pub preset: VideoPreset,
    pub status: String,
    pub updated_at: String,
    pub total_duration_seconds: u32,
    pub clip_count: u32,
    pub completed_clips: u32,
    pub failed_clips: u32,
}

impl From<&VideoProject> for VideoProjectSummary {
    fn from(project: &VideoProject) -> Self {
        Self {
            id: project.id.clone(),
            title: project.title.clone(),
            preset: project.preset.clone(),
            status: project.status.clone(),
            updated_at: project.updated_at.clone(),
            total_duration_seconds: project.total_duration_seconds,
            clip_count: project.clips.len() as u32,
            completed_clips: project
                .clips
                .iter()
                .filter(|clip| clip.status == "complete")
                .count() as u32,
            failed_clips: project
                .clips
                .iter()
                .filter(|clip| clip.status == "failed")
                .count() as u32,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoPresetStatus {
    pub id: String,
    pub label: String,
    pub profile: String,
    pub offloading: String,
    pub native_clip_seconds: u32,
    pub steps: u32,
    pub available: bool,
    pub missing_files: Vec<String>,
    pub supports_image_reference: bool,
    pub supports_video_reference: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoBackendSnapshot {
    pub endpoint: String,
    pub running: bool,
    pub ready: bool,
    pub owned: bool,
    pub pid: Option<u32>,
    pub profile: Option<String>,
    pub offloading: String,
    pub predictable: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSnapshot {
    pub settings: VideoSettings,
    pub backend: VideoBackendSnapshot,
    pub presets: Vec<VideoPresetStatus>,
    pub projects: Vec<VideoProjectSummary>,
    pub root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoProjectEvent {
    pub project_id: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub clip_index: Option<u32>,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CreativeBrief {
    title: String,
    continuity_bible: String,
    chapters: Vec<ChapterDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChapterDraft {
    title: String,
    narrative_goal: String,
    prompt_seed: String,
    #[serde(default = "default_weight")]
    duration_weight: f64,
}

fn default_weight() -> f64 {
    1.0
}

#[derive(Clone)]
pub struct VideoStore {
    root: PathBuf,
    settings_path: PathBuf,
}

impl VideoStore {
    pub fn new(library_root: &Path) -> Result<Self, String> {
        let root = library_root.join("video-studio");
        fs::create_dir_all(root.join("projects"))
            .map_err(|error| format!("could not create video library: {error}"))?;
        Ok(Self {
            settings_path: root.join("settings.json"),
            root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load_settings(&self) -> Result<VideoSettings, String> {
        if !self.settings_path.is_file() {
            return Ok(VideoSettings::default());
        }
        let metadata = fs::metadata(&self.settings_path).map_err(|error| error.to_string())?;
        if metadata.len() > 64 * 1024 {
            return Err("Video Studio settings exceed 64 KiB.".into());
        }
        let settings = serde_json::from_slice(
            &fs::read(&self.settings_path).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("Video Studio settings are invalid: {error}"))?;
        validate_settings(&settings)?;
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &VideoSettings) -> Result<(), String> {
        validate_settings(settings)?;
        atomic_json(&self.settings_path, settings)
    }

    fn project_path(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_id(id) {
            return Err("Video project id is invalid.".into());
        }
        Ok(self.root.join("projects").join(id).join("project.json"))
    }

    pub fn save_project(&self, project: &VideoProject) -> Result<(), String> {
        let path = self.project_path(&project.id)?;
        fs::create_dir_all(path.parent().expect("project file has a parent"))
            .map_err(|error| format!("could not create video project folder: {error}"))?;
        atomic_json(&path, project)
    }

    pub fn get_project(&self, id: &str) -> Result<VideoProject, String> {
        let path = self.project_path(id)?;
        match read_project_file(&path) {
            Ok(project) => Ok(project),
            Err(primary_error) => {
                let backup = path.with_extension("json.bak");
                read_project_file(&backup).map_err(|backup_error| {
                    format!(
                        "Video project could not be recovered: {primary_error}; backup: {backup_error}"
                    )
                })
            }
        }
    }

    pub fn list_projects(&self) -> Result<Vec<VideoProjectSummary>, String> {
        let mut summaries = Vec::new();
        for item in fs::read_dir(self.root.join("projects")).map_err(|error| error.to_string())? {
            let Ok(item) = item else { continue };
            if !item.path().is_dir() {
                continue;
            }
            let id = item.file_name().to_string_lossy().into_owned();
            let Ok(mut project) = self.get_project(&id) else {
                continue;
            };
            if matches!(
                project.status.as_str(),
                "starting" | "running" | "verifying" | "assembling"
            ) {
                project.status = "interrupted".into();
                project.updated_at = Utc::now().to_rfc3339();
                for clip in &mut project.clips {
                    if matches!(clip.status.as_str(), "queued" | "generating" | "verifying") {
                        clip.status = "planned".into();
                        clip.error = Some(
                            "Kestrel restarted before this clip was verified; it is safe to resume."
                                .into(),
                        );
                    }
                }
                let _ = self.save_project(&project);
            }
            summaries.push(VideoProjectSummary::from(&project));
        }
        summaries.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(summaries)
    }

    fn project_dir(&self, id: &str) -> Result<PathBuf, String> {
        Ok(self
            .project_path(id)?
            .parent()
            .expect("project file has a parent")
            .to_path_buf())
    }
}

struct OwnedComfyProcess {
    child: tokio::process::Child,
    pid: u32,
    profile: VideoMemoryProfile,
}

pub struct VideoManager {
    process: Mutex<Option<OwnedComfyProcess>>,
    client: Client,
}

impl VideoManager {
    pub fn new() -> Self {
        Self {
            process: Mutex::new(None),
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .expect("loopback ComfyUI client"),
        }
    }

    pub async fn snapshot(&self) -> VideoBackendSnapshot {
        let mut process = self.process.lock().await;
        if let Some(owned) = process.as_mut() {
            let exited = owned.child.try_wait().ok().flatten().is_some();
            if exited {
                *process = None;
            } else {
                let ready = self.health().await;
                return VideoBackendSnapshot {
                    endpoint: VIDEO_ENDPOINT.into(),
                    running: true,
                    ready,
                    owned: true,
                    pid: Some(owned.pid),
                    profile: Some(owned.profile.id().into()),
                    offloading: owned.profile.offloading().into(),
                    predictable: ready,
                    detail: if ready {
                        "Kestrel owns this loopback ComfyUI process and its exact memory flags."
                    } else {
                        "Kestrel's ComfyUI process is still starting or no longer answering."
                    }
                    .into(),
                };
            }
        }
        let external = self.health().await;
        VideoBackendSnapshot {
            endpoint: VIDEO_ENDPOINT.into(),
            running: external,
            ready: false,
            owned: false,
            pid: None,
            profile: None,
            offloading: if external { "unknown" } else { "none" }.into(),
            predictable: false,
            detail: if external {
                "An unowned ComfyUI server is using port 8188. Stop it before deterministic generation."
            } else {
                "ComfyUI is stopped. Kestrel will start the selected exact profile after planning."
            }
            .into(),
        }
    }

    async fn health(&self) -> bool {
        self.client
            .get(format!("{VIDEO_ENDPOINT}/system_stats"))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }

    pub async fn start(
        &self,
        settings: &VideoSettings,
        preset: &VideoPreset,
        log_dir: &Path,
    ) -> Result<VideoBackendSnapshot, String> {
        let profile = preset.profile();
        {
            let mut process = self.process.lock().await;
            if let Some(owned) = process.as_mut() {
                if owned.child.try_wait().ok().flatten().is_none() && owned.profile == profile {
                    drop(process);
                    if self.health().await {
                        return Ok(self.snapshot().await);
                    }
                }
            }
        }
        self.stop().await?;
        if self.health().await {
            return Err("Port 8188 already has an unowned ComfyUI server. Stop it so Kestrel can prove the memory policy before generation.".into());
        }
        let root = PathBuf::from(&settings.comfy_root);
        validate_comfy_root(&root, preset)?;
        fs::create_dir_all(log_dir).map_err(|error| error.to_string())?;
        let stdout = File::create(log_dir.join("comfy.stdout.log"))
            .map_err(|error| format!("could not create ComfyUI stdout log: {error}"))?;
        let stderr = File::create(log_dir.join("comfy.stderr.log"))
            .map_err(|error| format!("could not create ComfyUI stderr log: {error}"))?;
        let python = root.join(".venv").join("Scripts").join("python.exe");
        let mut command = Command::new(&python);
        command
            .current_dir(&root)
            .arg(root.join("main.py"))
            .args([
                "--listen",
                "127.0.0.1",
                "--port",
                "8188",
                "--cuda-device",
                "0",
                "--preview-method",
                "none",
                "--fast-disk",
            ])
            .args(profile.args())
            .env("PYTHONUTF8", "1")
            .env("CUDA_VISIBLE_DEVICES", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        #[cfg(windows)]
        command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
        let mut child = command.spawn().map_err(|error| {
            format!("could not start ComfyUI with {}: {error}", python.display())
        })?;
        let pid = child
            .id()
            .ok_or_else(|| "ComfyUI did not expose a process id.".to_string())?;
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err(format!(
                    "ComfyUI exited during startup. Inspect {}.",
                    log_dir.join("comfy.stderr.log").display()
                ));
            }
            if self.health().await {
                break;
            }
            if Instant::now() >= deadline {
                let _ = kill_tree(pid).await;
                return Err("ComfyUI did not become ready on loopback within 120 seconds.".into());
            }
            tokio::time::sleep(Duration::from_millis(750)).await;
        }
        *self.process.lock().await = Some(OwnedComfyProcess {
            child,
            pid,
            profile,
        });
        Ok(self.snapshot().await)
    }

    pub async fn stop(&self) -> Result<(), String> {
        let owned = self.process.lock().await.take();
        if let Some(mut owned) = owned {
            let _ = self
                .client
                .post(format!("{VIDEO_ENDPOINT}/interrupt"))
                .send()
                .await;
            if let Err(error) = kill_tree(owned.pid).await {
                let _ = owned.child.kill().await;
                return Err(error);
            }
            let _ = tokio::time::timeout(Duration::from_secs(10), owned.child.wait()).await;
        }
        Ok(())
    }

    async fn submit(&self, graph: Value) -> Result<String, String> {
        let response = self
            .client
            .post(format!("{VIDEO_ENDPOINT}/prompt"))
            .json(&json!({"prompt": graph}))
            .send()
            .await
            .map_err(|error| format!("ComfyUI prompt submission failed: {error}"))?;
        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|error| format!("ComfyUI returned invalid prompt JSON: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "ComfyUI rejected the prompt ({status}): {}",
                truncate(&body.to_string(), 900)
            ));
        }
        if body
            .get("node_errors")
            .is_some_and(|value| value.as_object().is_some_and(|items| !items.is_empty()))
        {
            return Err(format!(
                "ComfyUI reported node errors: {}",
                truncate(&body.to_string(), 900)
            ));
        }
        body.get("prompt_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "ComfyUI did not return a prompt id.".into())
    }

    async fn wait_for_output(
        &self,
        prompt_id: &str,
        cancel: &CancellationToken,
        deadline: Instant,
    ) -> Result<ComfyOutput, String> {
        loop {
            if cancel.is_cancelled() {
                let _ = self
                    .client
                    .post(format!("{VIDEO_ENDPOINT}/interrupt"))
                    .send()
                    .await;
                return Err("Generation stopped by the user.".into());
            }
            if Instant::now() >= deadline {
                let _ = self
                    .client
                    .post(format!("{VIDEO_ENDPOINT}/interrupt"))
                    .send()
                    .await;
                return Err("Generation exceeded the project runtime boundary.".into());
            }
            let response = self
                .client
                .get(format!("{VIDEO_ENDPOINT}/history/{prompt_id}"))
                .timeout(Duration::from_secs(10))
                .send()
                .await
                .map_err(|error| format!("ComfyUI history check failed: {error}"))?;
            let history: Value = response
                .json()
                .await
                .map_err(|error| format!("ComfyUI history was invalid: {error}"))?;
            if let Some(entry) = history.get(prompt_id) {
                let completed = entry
                    .pointer("/status/completed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let status = entry
                    .pointer("/status/status_str")
                    .and_then(Value::as_str)
                    .unwrap_or("running");
                if completed && status == "success" {
                    return find_video_output(entry)
                        .ok_or_else(|| "ComfyUI completed without a saved video output.".into());
                }
                if completed || status == "error" {
                    return Err(format!("ComfyUI generation ended with status {status}."));
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
}

#[cfg(windows)]
use std::os::windows::process::CommandExt;

async fn kill_tree(pid: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        let status = Command::new("taskkill.exe")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .await
            .map_err(|error| format!("could not stop ComfyUI process tree {pid}: {error}"))?;
        if !status.success() {
            return Err(format!(
                "taskkill could not stop ComfyUI process tree {pid}."
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        Ok(())
    }
}

#[derive(Debug)]
struct ComfyOutput {
    filename: String,
    subfolder: String,
    output_type: String,
}

fn find_video_output(value: &Value) -> Option<ComfyOutput> {
    match value {
        Value::Object(map) => {
            if let Some(filename) = map.get("filename").and_then(Value::as_str) {
                let extension = Path::new(filename)
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default();
                if matches!(extension.to_ascii_lowercase().as_str(), "mp4") {
                    return Some(ComfyOutput {
                        filename: filename.to_string(),
                        subfolder: map
                            .get("subfolder")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        output_type: map
                            .get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("output")
                            .to_string(),
                    });
                }
            }
            map.values().find_map(find_video_output)
        }
        Value::Array(items) => items.iter().find_map(find_video_output),
        _ => None,
    }
}

pub async fn plan_project(
    store: &VideoStore,
    runtime: Arc<RuntimeManager>,
    models: Vec<crate::model::ModelInfo>,
    control: ControlSettings,
    request: VideoPlanRequest,
    app: Option<&AppHandle>,
) -> Result<VideoProject, String> {
    validate_plan_request(&request)?;
    let clip_count = request
        .total_duration_seconds
        .div_ceil(request.preset.native_seconds());
    if clip_count > request.boundaries.max_clips {
        return Err(format!(
            "This project needs {clip_count} clips, above the boundary of {}. Raise the boundary deliberately or shorten the runtime.",
            request.boundaries.max_clips
        ));
    }
    emit(
        app,
        "planning",
        "Planning the visual system",
        "The local model is creating a compact story bible; native code will expand the full queue.",
        None,
        None,
    );
    let brief = if let Some(model_id) = request.planner_model_id.as_deref() {
        match plan_with_model(runtime, &models, &control, model_id, &request, clip_count, app).await {
            Ok(brief) => (brief, "Planned by the selected local model, then bounded and expanded natively.".to_string()),
            Err(error) => (
                deterministic_brief(&request, clip_count),
                format!("The local planner was unavailable ({error}). Kestrel created a deterministic offline plan instead."),
            ),
        }
    } else {
        (
            deterministic_brief(&request, clip_count),
            "Created deterministically without loading a planning model.".into(),
        )
    };
    let id = uuid::Uuid::new_v4().to_string();
    let project_dir = store.project_dir(&id)?;
    fs::create_dir_all(project_dir.join("clips")).map_err(|error| error.to_string())?;
    fs::create_dir_all(project_dir.join("logs")).map_err(|error| error.to_string())?;
    let (width, height) = dimensions(&request.preset, &request.orientation)?;
    let now = Utc::now().to_rfc3339();
    let (chapters, clips) = expand_plan(&request, &brief.0, clip_count);
    let mut project = VideoProject {
        id,
        title: truncate(&brief.0.title, 160),
        prompt: request.prompt.trim().to_string(),
        audience: request.audience.trim().to_string(),
        use_case: request.use_case.trim().to_string(),
        preset: request.preset.clone(),
        status: "planned".into(),
        created_at: now.clone(),
        updated_at: now,
        total_duration_seconds: request.total_duration_seconds,
        clip_duration_seconds: brief_clip_seconds(&request),
        width,
        height,
        fps: request.preset.fps(),
        frames_per_clip: request.preset.frames(),
        steps: request.preset.steps(),
        cfg: request.preset.cfg(),
        negative_prompt: request.negative_prompt.trim().to_string(),
        continuity_bible: truncate(&brief.0.continuity_bible, 12_000),
        planning_note: brief.1,
        chapters,
        clips,
        boundaries: request.boundaries,
        output_directory: project_dir.to_string_lossy().into_owned(),
        final_output_path: None,
        errors: Vec::new(),
        references: Vec::new(),
        continuity: VideoContinuitySettings::default(),
    };
    bound_project(&mut project);
    store.save_project(&project)?;
    emit(
        app,
        "planned",
        "Review the durable production plan",
        &format!(
            "{} clips across {} chapters are ready. No ComfyUI generation has started.",
            project.clips.len(),
            project.chapters.len()
        ),
        Some(&project.id),
        None,
    );
    Ok(project)
}

async fn plan_with_model(
    runtime: Arc<RuntimeManager>,
    models: &[crate::model::ModelInfo],
    control: &ControlSettings,
    model_id: &str,
    request: &VideoPlanRequest,
    clip_count: u32,
    app: Option<&AppHandle>,
) -> Result<CreativeBrief, String> {
    let lease = runtime
        .lease_model(model_id, models, control, app)
        .await
        .map_err(|error| error.to_string())?;
    let chapter_target =
        (request.total_duration_seconds.div_ceil(600) as usize).clamp(1, MAX_CHAPTERS.min(12));
    let system = "You are Kestrel's offline video production planner. Return strict JSON only. Build a coherent visual story bible that can guide thousands of independently generated clips. Do not include copyrighted character names, camera brand marketing, or promises that a model cannot guarantee.";
    let user = format!(
        "Create a compact plan for this local generative-video project.\nPrompt: {}\nAudience: {}\nUse case: {}\nTarget runtime: {} seconds\nNative clips: {}\nPreset: {}\nReturn JSON with title, continuity_bible, and exactly {} chapters. Each chapter needs title, narrative_goal, prompt_seed, duration_weight. Keep continuity_bible under 1500 words and each field concrete, visual, and reusable.",
        request.prompt.trim(), request.audience.trim(), request.use_case.trim(),
        request.total_duration_seconds, clip_count, request.preset.label(), chapter_target
    );
    let body = json!({
        "model": lease.connection.model_id,
        "messages": [
            {"role":"system","content":system},
            {"role":"user","content":user}
        ],
        "temperature": 0.45,
        "top_p": 0.9,
        "max_tokens": control.agent_max_output_tokens.clamp(1024, 8192),
        "stream": false,
        "response_format": {"type":"json_object"}
    });
    let response = authorized(
        Client::builder()
            .timeout(Duration::from_secs(1_800))
            .build()
            .map_err(|error| error.to_string())?
            .post(format!("{}/chat/completions", lease.connection.endpoint)),
        &lease.connection,
    )
    .json(&body)
    .send()
    .await
    .map_err(|error| format!("local planning request failed: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(format!(
            "local planner returned {status}: {}",
            truncate(&detail, 600)
        ));
    }
    let value: Value = response.json().await.map_err(|error| error.to_string())?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| "local planner returned no JSON content".to_string())?;
    let content = strip_json_fence(content);
    let mut brief: CreativeBrief = serde_json::from_str(content)
        .map_err(|error| format!("local planner JSON was invalid: {error}"))?;
    normalize_brief(&mut brief, request, clip_count);
    Ok(brief)
}

fn deterministic_brief(request: &VideoPlanRequest, clip_count: u32) -> CreativeBrief {
    let count = request.total_duration_seconds.div_ceil(600).clamp(1, 12) as usize;
    let chapter_names = [
        "Opening image",
        "Context and world",
        "Development",
        "Contrast",
        "Deepening detail",
        "Turning point",
        "Application",
        "Reflection",
        "Resolution",
        "Closing image",
        "Extended exploration",
        "Final synthesis",
    ];
    let chapters = (0..count)
        .map(|index| ChapterDraft {
            title: chapter_names[index].into(),
            narrative_goal: format!(
                "Advance part {} of the visual explanation for {} without repeating the preceding shots.",
                index + 1,
                request.audience.trim()
            ),
            prompt_seed: format!(
                "{}; visual progression {} of {}; coherent location, subjects, palette, and lighting",
                request.prompt.trim(),
                index + 1,
                count
            ),
            duration_weight: 1.0,
        })
        .collect();
    CreativeBrief {
        title: title_from_prompt(&request.prompt),
        continuity_bible: format!(
            "Core concept: {}. Audience: {}. Intended use: {}. Maintain consistent subject identity, wardrobe, geography, color palette, lighting logic, lens language, motion direction, and era. Prefer visual explanation over on-screen text. Each clip must be understandable alone while matching adjacent clips. Planned native clip count: {}.",
            request.prompt.trim(), request.audience.trim(), request.use_case.trim(), clip_count
        ),
        chapters,
    }
}

fn normalize_brief(brief: &mut CreativeBrief, request: &VideoPlanRequest, clip_count: u32) {
    if brief.title.trim().is_empty() {
        brief.title = title_from_prompt(&request.prompt);
    }
    brief.title = truncate(brief.title.trim(), 160);
    if brief.continuity_bible.trim().is_empty() {
        brief.continuity_bible = deterministic_brief(request, clip_count).continuity_bible;
    }
    brief.continuity_bible = truncate(brief.continuity_bible.trim(), 12_000);
    brief.chapters.retain(|chapter| {
        !chapter.title.trim().is_empty() && !chapter.prompt_seed.trim().is_empty()
    });
    brief
        .chapters
        .truncate(MAX_CHAPTERS.min(clip_count as usize));
    if brief.chapters.is_empty() {
        brief.chapters = deterministic_brief(request, clip_count).chapters;
    }
    for chapter in &mut brief.chapters {
        chapter.title = truncate(chapter.title.trim(), 160);
        chapter.narrative_goal = truncate(chapter.narrative_goal.trim(), 2_000);
        chapter.prompt_seed = truncate(chapter.prompt_seed.trim(), 4_000);
        if !chapter.duration_weight.is_finite() || chapter.duration_weight <= 0.0 {
            chapter.duration_weight = 1.0;
        }
    }
}

fn expand_plan(
    request: &VideoPlanRequest,
    brief: &CreativeBrief,
    clip_count: u32,
) -> (Vec<VideoChapter>, Vec<VideoClip>) {
    let weights = brief
        .chapters
        .iter()
        .map(|chapter| chapter.duration_weight.max(0.01))
        .collect::<Vec<_>>();
    let total_weight = weights.iter().sum::<f64>();
    let mut allocations = weights
        .iter()
        .map(|weight| ((clip_count as f64 * weight / total_weight).floor() as u32).max(1))
        .collect::<Vec<_>>();
    while allocations.iter().sum::<u32>() > clip_count {
        if let Some(index) = allocations
            .iter()
            .enumerate()
            .filter(|(_, value)| **value > 1)
            .max_by_key(|(_, value)| **value)
            .map(|(index, _)| index)
        {
            allocations[index] -= 1;
        } else {
            break;
        }
    }
    let mut cursor = 1u32;
    while allocations.iter().sum::<u32>() < clip_count {
        let index = (cursor as usize - 1) % allocations.len();
        allocations[index] += 1;
        cursor += 1;
    }
    let motion = [
        "slow lateral tracking shot",
        "gentle push-in with foreground parallax",
        "locked composition with meaningful subject motion",
        "wide establishing move",
        "medium observational shot",
        "close detail with shallow depth transitions",
        "measured orbit around the subject",
        "calm pull-back revealing context",
    ];
    let mut chapters = Vec::new();
    let mut clips = Vec::with_capacity(clip_count as usize);
    let base_seed = seed_from_text(&request.prompt);
    let mut global = 1u32;
    for (chapter_index, (chapter, allocation)) in brief.chapters.iter().zip(allocations).enumerate()
    {
        let first = global;
        for local in 0..allocation {
            let prompt = format!(
                "Chapter: {}. Goal: {}. Shot {}/{}: {}; {}.",
                chapter.title,
                chapter.narrative_goal,
                local + 1,
                allocation,
                chapter.prompt_seed,
                motion[((global - 1) as usize) % motion.len()],
            );
            clips.push(VideoClip {
                index: global,
                chapter_index: chapter_index as u32 + 1,
                // Keep the durable ledger compact enough for 20,000-clip plans. The shared
                // concept and continuity bible are joined only while constructing the graph.
                prompt: truncate(&prompt, 900),
                seed: base_seed.wrapping_add(global as u64 * 1_000_003),
                status: "planned".into(),
                attempts: 0,
                comfy_prompt_id: None,
                output_path: None,
                bytes: None,
                sha256: None,
                error: None,
                started_at: None,
                completed_at: None,
                reference_asset_id: None,
                continuity_frame_path: None,
            });
            global += 1;
        }
        chapters.push(VideoChapter {
            index: chapter_index as u32 + 1,
            title: chapter.title.clone(),
            narrative_goal: chapter.narrative_goal.clone(),
            prompt_seed: chapter.prompt_seed.clone(),
            first_clip: first,
            last_clip: global - 1,
            reference_asset_id: None,
        });
    }
    (chapters, clips)
}

fn brief_clip_seconds(request: &VideoPlanRequest) -> u32 {
    request.preset.native_seconds()
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_project(
    app: AppHandle,
    store: VideoStore,
    manager: Arc<VideoManager>,
    project_id: String,
    settings: VideoSettings,
    cancel: CancellationToken,
) -> Result<(), String> {
    let mut project = store.get_project(&project_id)?;
    if matches!(
        project.status.as_str(),
        "completed" | "completed-with-warnings"
    ) {
        return Err("This video project is already complete.".into());
    }
    verify_project_references(&store, &project).await?;
    project.status = "starting".into();
    project.updated_at = Utc::now().to_rfc3339();
    store.save_project(&project)?;
    emit(
        Some(&app),
        "starting",
        "Starting the declared video backend",
        &format!(
            "{} · offloading {}. Generation cannot begin until Kestrel owns and verifies this profile.",
            project.preset.label(),
            project.preset.profile().offloading()
        ),
        Some(&project.id),
        None,
    );
    let project_dir = store.project_dir(&project.id)?;
    manager
        .start(&settings, &project.preset, &project_dir.join("logs"))
        .await?;
    let backend = manager.snapshot().await;
    if !backend.predictable || backend.profile.as_deref() != Some(project.preset.profile().id()) {
        manager.stop().await?;
        return Err("ComfyUI started without the requested predictable memory policy.".into());
    }
    project.status = "running".into();
    project.updated_at = Utc::now().to_rfc3339();
    store.save_project(&project)?;
    let started = Instant::now();
    let deadline =
        started + Duration::from_secs(project.boundaries.max_runtime_minutes as u64 * 60);
    // A deliberate resume retries prior failures, so only failures from this execution window
    // count against its reviewed boundary.
    let mut failed = 0;
    for position in 0..project.clips.len() {
        if cancel.is_cancelled() {
            project.status = "cancelled".into();
            project.updated_at = Utc::now().to_rfc3339();
            store.save_project(&project)?;
            manager.stop().await?;
            emit(
                Some(&app),
                "cancelled",
                "Generation stopped safely",
                "Verified clips remain durable. Resume starts from the first unverified clip.",
                Some(&project.id),
                None,
            );
            return Ok(());
        }
        if project.clips[position].status == "complete" {
            continue;
        }
        if Instant::now() >= deadline {
            project.status = "paused-boundary".into();
            project.errors.push("Maximum project runtime reached. Resume deliberately to continue with a fresh runtime window.".into());
            project.updated_at = Utc::now().to_rfc3339();
            store.save_project(&project)?;
            manager.stop().await?;
            return Ok(());
        }
        let free = fs2::available_space(&project_dir)
            .map_err(|error| format!("could not inspect free disk space: {error}"))?;
        let required = project.boundaries.min_free_disk_gib as u64 * 1024 * 1024 * 1024;
        if free < required {
            project.status = "paused-boundary".into();
            project.errors.push(format!(
                "Free disk space fell below the {} GiB boundary.",
                project.boundaries.min_free_disk_gib
            ));
            project.updated_at = Utc::now().to_rfc3339();
            store.save_project(&project)?;
            manager.stop().await?;
            return Ok(());
        }
        let clip_index = project.clips[position].index;
        let prepared_references =
            prepare_clip_references(&settings, &store, &project, position).await?;
        let mut last_error = None;
        for attempt in 0..=project.boundaries.max_retries_per_clip {
            project.clips[position].attempts += 1;
            project.clips[position].status = "generating".into();
            project.clips[position].started_at = Some(Utc::now().to_rfc3339());
            project.clips[position].error = None;
            project.updated_at = Utc::now().to_rfc3339();
            store.save_project(&project)?;
            emit(
                Some(&app),
                "clip-started",
                &format!("Generating clip {} of {}", clip_index, project.clips.len()),
                &format!(
                    "Attempt {} · {} · {} steps",
                    attempt + 1,
                    project.preset.label(),
                    project.steps
                ),
                Some(&project.id),
                Some(clip_index),
            );
            let prefix = format!("video/kestrel/{}/clip_{clip_index:05}", project.id);
            let graph = build_graph(
                &project,
                &project.clips[position],
                &prefix,
                &prepared_references,
            );
            let result: Result<VerifiedOutput, String> = async {
                let prompt_id = manager.submit(graph).await?;
                project.clips[position].comfy_prompt_id = Some(prompt_id.clone());
                store.save_project(&project)?;
                let output = manager
                    .wait_for_output(&prompt_id, &cancel, deadline)
                    .await?;
                let mut verified =
                    verify_and_copy_output(&settings, &store, &project.id, clip_index, output)
                        .await?;
                if project.continuity.mode == VideoContinuityMode::PreviousFrame {
                    verified.continuity_frame_path = Some(
                        extract_continuity_frame(
                            &settings,
                            &store,
                            &project.id,
                            clip_index,
                            &verified.path,
                        )
                        .await?,
                    );
                }
                Ok(verified)
            }
            .await;
            match result {
                Ok(verified) => {
                    project.clips[position].status = "complete".into();
                    project.clips[position].output_path = Some(verified.path);
                    project.clips[position].bytes = Some(verified.bytes);
                    project.clips[position].sha256 = Some(verified.sha256);
                    project.clips[position].completed_at = Some(Utc::now().to_rfc3339());
                    project.clips[position].continuity_frame_path = verified.continuity_frame_path;
                    project.clips[position].error = None;
                    project.updated_at = Utc::now().to_rfc3339();
                    store.save_project(&project)?;
                    emit(
                        Some(&app),
                        "clip-verified",
                        &format!("Verified clip {clip_index}"),
                        "The MP4 was copied into the durable project bundle and hashed.",
                        Some(&project.id),
                        Some(clip_index),
                    );
                    last_error = None;
                    break;
                }
                Err(error) => {
                    last_error = Some(error.clone());
                    project.clips[position].error = Some(error.clone());
                    store.save_project(&project)?;
                    emit(
                        Some(&app),
                        "clip-retry",
                        &format!("Clip {clip_index} needs attention"),
                        &format!(
                            "{error} Retry {}/{}.",
                            attempt + 1,
                            project.boundaries.max_retries_per_clip + 1
                        ),
                        Some(&project.id),
                        Some(clip_index),
                    );
                    if cancel.is_cancelled() {
                        break;
                    }
                    if attempt < project.boundaries.max_retries_per_clip
                        && Instant::now() < deadline
                    {
                        manager.stop().await?;
                        tokio::time::sleep(Duration::from_secs(3 * (attempt as u64 + 1))).await;
                        manager
                            .start(&settings, &project.preset, &project_dir.join("logs"))
                            .await?;
                    }
                }
            }
        }
        if cancel.is_cancelled() {
            project.status = "cancelled".into();
            project.updated_at = Utc::now().to_rfc3339();
            store.save_project(&project)?;
            manager.stop().await?;
            emit(
                Some(&app),
                "cancelled",
                "Generation stopped safely",
                "Verified clips remain durable. Resume starts from the first unverified clip.",
                Some(&project.id),
                Some(clip_index),
            );
            return Ok(());
        }
        if Instant::now() >= deadline {
            project.status = "paused-boundary".into();
            project.errors.push(
                "Maximum project runtime reached during clip generation. Resume deliberately to continue with a fresh runtime window."
                    .into(),
            );
            project.updated_at = Utc::now().to_rfc3339();
            store.save_project(&project)?;
            manager.stop().await?;
            return Ok(());
        }
        if let Some(error) = last_error {
            project.clips[position].status = "failed".into();
            project.clips[position].error = Some(error.clone());
            failed += 1;
            project.errors.push(format!("Clip {clip_index}: {error}"));
            project.updated_at = Utc::now().to_rfc3339();
            store.save_project(&project)?;
            if failed > project.boundaries.max_failed_clips {
                project.status = "paused-failures".into();
                project.errors.push(format!(
                    "Failure boundary exceeded: {failed} failed clips, limit {}.",
                    project.boundaries.max_failed_clips
                ));
                project.updated_at = Utc::now().to_rfc3339();
                store.save_project(&project)?;
                manager.stop().await?;
                emit(Some(&app), "paused", "Failure boundary reached", "Kestrel stopped the backend. Review the failed clips, then resume to retry only unfinished work.", Some(&project.id), Some(clip_index));
                return Ok(());
            }
        }
    }
    let failed_count = project
        .clips
        .iter()
        .filter(|clip| clip.status == "failed")
        .count();
    if failed_count > 0 {
        project.status = "paused-failures".into();
        project.updated_at = Utc::now().to_rfc3339();
        store.save_project(&project)?;
    } else if project.boundaries.assemble_final_video {
        project.status = "assembling".into();
        project.updated_at = Utc::now().to_rfc3339();
        store.save_project(&project)?;
        emit(
            Some(&app),
            "assembling",
            "Assembling verified clips",
            "FFmpeg receives only the ordered, hashed project copies.",
            Some(&project.id),
            None,
        );
        match assemble(&settings, &store, &project).await {
            Ok(path) => {
                project.final_output_path = Some(path);
                project.status = "completed".into();
            }
            Err(error) => {
                project.errors.push(error);
                project.status = "completed-with-warnings".into();
            }
        }
        project.updated_at = Utc::now().to_rfc3339();
        store.save_project(&project)?;
    } else {
        project.status = "completed".into();
        project.updated_at = Utc::now().to_rfc3339();
        store.save_project(&project)?;
    }
    // Release the GPU after every terminal state so llama.cpp remains Kestrel's only other
    // possible model owner. The ComfyUI process stays warm for the entire batch, not afterward.
    manager.stop().await?;
    emit(
        Some(&app),
        "completed",
        "Video project finished",
        &format!(
            "{} verified clips. Final status: {}.",
            project.clips.len(),
            project.status
        ),
        Some(&project.id),
        None,
    );
    Ok(())
}

async fn verify_project_references(
    store: &VideoStore,
    project: &VideoProject,
) -> Result<(), String> {
    let mut used_reference_ids = BTreeSet::new();
    if project.continuity.mode != VideoContinuityMode::None
        && project.continuity.primary_reference_id.is_none()
    {
        return Err(
            "The selected continuity policy needs a primary subject/storyboard image.".into(),
        );
    }
    if let Some(primary) = project.continuity.primary_reference_id.as_deref() {
        let asset = find_reference(project, primary)?;
        if asset.kind != VideoReferenceKind::Image {
            return Err("The primary continuity reference must be an image.".into());
        }
        if project.continuity.mode != VideoContinuityMode::None {
            used_reference_ids.insert(primary.to_string());
        }
    }
    for clip in &project.clips {
        if let Some(reference_id) = clip.reference_asset_id.as_deref() {
            let asset = find_reference(project, reference_id)?;
            used_reference_ids.insert(reference_id.to_string());
            if asset.kind == VideoReferenceKind::Video && !project.preset.supports_video_reference()
            {
                return Err(format!(
                    "Clip {} has a motion video but this preset cannot consume it.",
                    clip.index
                ));
            }
        }
    }
    for chapter in &project.chapters {
        if let Some(reference_id) = chapter.reference_asset_id.as_deref() {
            let asset = find_reference(project, reference_id)?;
            used_reference_ids.insert(reference_id.to_string());
            if asset.kind == VideoReferenceKind::Video && !project.preset.supports_video_reference()
            {
                return Err(format!(
                    "Chapter {} has a motion video but this preset cannot consume it.",
                    chapter.index
                ));
            }
        }
    }
    let project_dir = store.project_dir(&project.id)?;
    let assets = project
        .references
        .iter()
        .filter(|asset| used_reference_ids.contains(&asset.id))
        .cloned()
        .collect::<Vec<_>>();
    tokio::task::spawn_blocking(move || {
        let canonical_project = project_dir
            .canonicalize()
            .map_err(|error| format!("Could not resolve the video project: {error}"))?;
        for asset in assets {
            let path = PathBuf::from(&asset.stored_path);
            let canonical = path
                .canonicalize()
                .map_err(|error| format!("Reference {} is unavailable: {error}", asset.name))?;
            if !canonical.starts_with(&canonical_project) {
                return Err(format!(
                    "Reference {} escaped its durable project directory.",
                    asset.name
                ));
            }
            let bytes = fs::metadata(&canonical)
                .map_err(|error| error.to_string())?
                .len();
            if bytes != asset.bytes || hash_file(&canonical)? != asset.sha256 {
                return Err(format!(
                    "Reference {} changed after import. Re-import it before generation.",
                    asset.name
                ));
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| format!("Reference verification worker stopped: {error}"))?
}

struct VerifiedOutput {
    path: String,
    bytes: u64,
    sha256: String,
    continuity_frame_path: Option<String>,
}

#[derive(Default)]
struct PreparedReferences {
    image_input: Option<String>,
    video_input: Option<String>,
}

async fn prepare_clip_references(
    settings: &VideoSettings,
    store: &VideoStore,
    project: &VideoProject,
    position: usize,
) -> Result<PreparedReferences, String> {
    let clip = &project.clips[position];
    let explicit = clip
        .reference_asset_id
        .as_deref()
        .map(|id| find_reference(project, id))
        .transpose()?;
    let chapter_reference = project
        .chapters
        .iter()
        .find(|chapter| chapter.index == clip.chapter_index && chapter.first_clip == clip.index)
        .and_then(|chapter| chapter.reference_asset_id.as_deref())
        .map(|id| find_reference(project, id))
        .transpose()?;
    let explicit_image = explicit
        .filter(|asset| asset.kind == VideoReferenceKind::Image)
        .or_else(|| chapter_reference.filter(|asset| asset.kind == VideoReferenceKind::Image));
    let explicit_video = explicit
        .filter(|asset| asset.kind == VideoReferenceKind::Video)
        .or_else(|| chapter_reference.filter(|asset| asset.kind == VideoReferenceKind::Video));
    let continuity_frame = if explicit_image.is_none()
        && project.continuity.mode == VideoContinuityMode::PreviousFrame
        && position > 0
    {
        project.clips[..position]
            .iter()
            .rev()
            .find(|previous| previous.status == "complete")
            .and_then(|previous| previous.continuity_frame_path.as_deref())
            .map(PathBuf::from)
    } else {
        None
    };
    let primary = if explicit_image.is_none()
        && continuity_frame.is_none()
        && project.continuity.mode != VideoContinuityMode::None
    {
        project
            .continuity
            .primary_reference_id
            .as_deref()
            .map(|id| find_reference(project, id))
            .transpose()?
            .filter(|asset| asset.kind == VideoReferenceKind::Image)
    } else {
        None
    };
    let (image_source, image_stem) = if let Some(asset) = explicit_image {
        (
            Some(PathBuf::from(&asset.stored_path)),
            format!("asset_{}", asset.id),
        )
    } else if let Some(frame) = continuity_frame {
        (Some(frame), format!("clip_{:05}_continuity", clip.index))
    } else if let Some(asset) = primary {
        (
            Some(PathBuf::from(&asset.stored_path)),
            format!("asset_{}", asset.id),
        )
    } else {
        (None, format!("clip_{:05}_reference", clip.index))
    };
    let video_source = explicit_video.map(|asset| {
        (
            PathBuf::from(&asset.stored_path),
            format!("asset_{}", asset.id),
        )
    });
    let project_dir = store.project_dir(&project.id)?;
    let image_input = if let Some(source) = image_source {
        Some(stage_comfy_input(settings, &project_dir, &project.id, source, &image_stem).await?)
    } else {
        None
    };
    let video_input = if let Some((source, stem)) = video_source {
        Some(stage_comfy_input(settings, &project_dir, &project.id, source, &stem).await?)
    } else {
        None
    };
    Ok(PreparedReferences {
        image_input,
        video_input,
    })
}

async fn stage_comfy_input(
    settings: &VideoSettings,
    project_dir: &Path,
    project_id: &str,
    source: PathBuf,
    stem: &str,
) -> Result<String, String> {
    let comfy_input = PathBuf::from(&settings.comfy_root)
        .join("input")
        .join("kestrel")
        .join(project_id);
    let project_dir = project_dir.to_path_buf();
    let relative = format!(
        "kestrel/{project_id}/{stem}.{}",
        source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("bin")
            .to_ascii_lowercase()
    );
    let destination = PathBuf::from(&settings.comfy_root)
        .join("input")
        .join(relative.replace('/', "\\"));
    tokio::task::spawn_blocking(move || {
        let canonical_project = project_dir
            .canonicalize()
            .map_err(|error| format!("Could not resolve the video project: {error}"))?;
        let canonical_source = source
            .canonicalize()
            .map_err(|error| format!("Reference asset is unavailable: {error}"))?;
        if !canonical_source.starts_with(&canonical_project) {
            return Err("Reference asset escaped its durable project directory.".into());
        }
        fs::create_dir_all(&comfy_input).map_err(|error| error.to_string())?;
        let source_bytes = fs::metadata(&canonical_source)
            .map_err(|error| error.to_string())?
            .len();
        if fs::metadata(&destination).is_ok_and(|metadata| metadata.len() == source_bytes) {
            return Ok(relative);
        }
        let partial = destination.with_extension("partial");
        fs::copy(&canonical_source, &partial)
            .map_err(|error| format!("Could not stage the reference for ComfyUI: {error}"))?;
        if destination.exists() {
            fs::remove_file(&destination).map_err(|error| error.to_string())?;
        }
        fs::rename(&partial, &destination).map_err(|error| error.to_string())?;
        Ok(relative)
    })
    .await
    .map_err(|error| format!("Reference staging worker stopped: {error}"))?
}

async fn extract_continuity_frame(
    settings: &VideoSettings,
    store: &VideoStore,
    project_id: &str,
    clip_index: u32,
    video_path: &str,
) -> Result<String, String> {
    let destination = store
        .project_dir(project_id)?
        .join("continuity")
        .join(format!("clip_{clip_index:05}_last.png"));
    fs::create_dir_all(
        destination
            .parent()
            .ok_or_else(|| "Continuity frame has no parent directory.".to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let ffmpeg = validated_ffmpeg_command(&settings.ffmpeg_path)?;
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-sseof",
            "-0.1",
            "-i",
        ])
        .arg(video_path)
        .args(["-frames:v", "1"])
        .arg(&destination);
    #[cfg(windows)]
    command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    let output = command
        .output()
        .await
        .map_err(|error| format!("FFmpeg could not extract the continuity frame: {error}"))?;
    if !output.status.success()
        || fs::metadata(&destination).map_or(true, |metadata| metadata.len() < 256)
    {
        return Err(format!(
            "Could not extract a verified last frame for continuity: {}",
            truncate(&String::from_utf8_lossy(&output.stderr), 600)
        ));
    }
    Ok(destination.to_string_lossy().into_owned())
}

async fn verify_and_copy_output(
    settings: &VideoSettings,
    store: &VideoStore,
    project_id: &str,
    clip_index: u32,
    output: ComfyOutput,
) -> Result<VerifiedOutput, String> {
    if output.output_type != "output" || Path::new(&output.filename).is_absolute() {
        return Err("ComfyUI returned an unsafe output reference.".into());
    }
    let root = PathBuf::from(&settings.comfy_root).join("output");
    let source = root.join(&output.subfolder).join(&output.filename);
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("could not resolve ComfyUI output folder: {error}"))?;
    let canonical_source = source
        .canonicalize()
        .map_err(|error| format!("ComfyUI output is missing: {error}"))?;
    if !canonical_source.starts_with(&canonical_root) {
        return Err("ComfyUI output escaped its configured output folder.".into());
    }
    let metadata = fs::metadata(&canonical_source).map_err(|error| error.to_string())?;
    if metadata.len() < 1_024 {
        return Err("ComfyUI output is too small to be a valid video.".into());
    }
    let destination = store
        .project_dir(project_id)?
        .join("clips")
        .join(format!("clip_{clip_index:05}.mp4"));
    let partial = destination.with_extension("mp4.partial");
    fs::copy(&canonical_source, &partial)
        .map_err(|error| format!("could not copy verified clip into project: {error}"))?;
    if destination.exists() {
        fs::remove_file(&destination).map_err(|error| error.to_string())?;
    }
    fs::rename(&partial, &destination).map_err(|error| error.to_string())?;
    let path = destination.clone();
    tokio::task::spawn_blocking(move || {
        let bytes = fs::metadata(&path)
            .map_err(|error| error.to_string())?
            .len();
        let sha256 = hash_file(&path)?;
        Ok(VerifiedOutput {
            path: path.to_string_lossy().into_owned(),
            bytes,
            sha256,
            continuity_frame_path: None,
        })
    })
    .await
    .map_err(|error| format!("video verification worker stopped: {error}"))?
}

async fn assemble(
    settings: &VideoSettings,
    store: &VideoStore,
    project: &VideoProject,
) -> Result<String, String> {
    let project_dir = store.project_dir(&project.id)?;
    let manifest = project_dir.join("concat.txt");
    let mut file = File::create(&manifest).map_err(|error| error.to_string())?;
    for clip in &project.clips {
        let path = clip
            .output_path
            .as_deref()
            .ok_or_else(|| format!("Clip {} has no verified output.", clip.index))?;
        let safe = path.replace('\\', "/").replace('\'', "'\\''");
        writeln!(file, "file '{safe}'").map_err(|error| error.to_string())?;
    }
    file.sync_all().map_err(|error| error.to_string())?;
    let final_path = project_dir.join("final.mp4");
    let ffmpeg = validated_ffmpeg_command(&settings.ffmpeg_path)?;
    let mut command = Command::new(ffmpeg);
    command.args([
        "-hide_banner",
        "-loglevel",
        "warning",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
        "-i",
    ]);
    command
        .arg(&manifest)
        .args([
            "-t",
            &project.total_duration_seconds.to_string(),
            "-c",
            "copy",
        ])
        .arg(&final_path);
    #[cfg(windows)]
    command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    let output = command.output().await.map_err(|error| {
        format!("FFmpeg is unavailable; verified clips remain complete: {error}")
    })?;
    if !output.status.success() {
        return Err(format!(
            "FFmpeg could not assemble the final video; verified clips remain complete: {}",
            truncate(&String::from_utf8_lossy(&output.stderr), 1_200)
        ));
    }
    if fs::metadata(&final_path).map_or(true, |metadata| metadata.len() < 1_024) {
        return Err(
            "FFmpeg exited without a valid final video; verified clips remain complete.".into(),
        );
    }
    Ok(final_path.to_string_lossy().into_owned())
}

fn build_graph(
    project: &VideoProject,
    clip: &VideoClip,
    prefix: &str,
    references: &PreparedReferences,
) -> Value {
    match project.preset {
        VideoPreset::Wan13GpuOnly => wan_graph(
            "wan2.1_t2v_1.3B_bf16.safetensors",
            "wan_2.1_vae.safetensors",
            "EmptyHunyuanLatentVideo",
            project,
            clip,
            prefix,
            references,
        ),
        VideoPreset::WanVace13Reference => vace_graph(project, clip, prefix, references),
        VideoPreset::Wan22Offload => wan_graph(
            "wan2.2_ti2v_5B_fp16.safetensors",
            "wan2.2_vae.safetensors",
            "Wan22ImageToVideoLatent",
            project,
            clip,
            prefix,
            references,
        ),
        VideoPreset::KandinskyDistilled | VideoPreset::KandinskySft => {
            kandinsky_graph(project, clip, prefix, references)
        }
    }
}

fn wan_graph(
    model: &str,
    vae: &str,
    latent_node: &str,
    project: &VideoProject,
    clip: &VideoClip,
    prefix: &str,
    references: &PreparedReferences,
) -> Value {
    let mut graph = BTreeMap::<String, Value>::new();
    graph.insert(
        "1".into(),
        node(
            "UNETLoader",
            json!({"unet_name":model,"weight_dtype":"default"}),
        ),
    );
    graph.insert("2".into(), node("CLIPLoader", json!({"clip_name":"umt5_xxl_fp8_e4m3fn_scaled.safetensors","type":"wan","device":"default"})));
    graph.insert("3".into(), node("VAELoader", json!({"vae_name":vae})));
    let prompt = generation_prompt(project, clip);
    graph.insert(
        "4".into(),
        node("CLIPTextEncode", json!({"text":prompt,"clip":["2",0]})),
    );
    graph.insert(
        "5".into(),
        node(
            "CLIPTextEncode",
            json!({"text":project.negative_prompt,"clip":["2",0]}),
        ),
    );
    let mut latent_inputs = json!({"vae":["3",0],"width":project.width,"height":project.height,"length":project.frames_per_clip,"batch_size":1});
    if latent_node != "EmptyHunyuanLatentVideo" {
        if let Some(image) = references.image_input.as_deref() {
            graph.insert("12".into(), node("LoadImage", json!({"image":image})));
            latent_inputs["start_image"] = json!(["12", 0]);
        }
    }
    graph.insert("6".into(), node(latent_node, latent_inputs));
    graph.insert(
        "7".into(),
        node("ModelSamplingSD3", json!({"model":["1",0],"shift":8.0})),
    );
    graph.insert("8".into(), node("KSampler", json!({"model":["7",0],"seed":clip.seed,"steps":project.steps,"cfg":project.cfg,"sampler_name":"uni_pc","scheduler":"simple","positive":["4",0],"negative":["5",0],"latent_image":["6",0],"denoise":1.0})));
    let decoder = if latent_node == "EmptyHunyuanLatentVideo" {
        node("VAEDecode", json!({"samples":["8",0],"vae":["3",0]}))
    } else {
        tiled_vae_decode(json!(["8", 0]), json!(["3", 0]))
    };
    graph.insert("9".into(), decoder);
    graph.insert(
        "10".into(),
        node(
            "CreateVideo",
            json!({"images":["9",0],"fps":project.fps as f64}),
        ),
    );
    graph.insert(
        "11".into(),
        node(
            "SaveVideo",
            json!({"video":["10",0],"filename_prefix":prefix,"format":"mp4","codec":"h264"}),
        ),
    );
    json!(graph)
}

fn kandinsky_graph(
    project: &VideoProject,
    clip: &VideoClip,
    prefix: &str,
    references: &PreparedReferences,
) -> Value {
    let model = if project.preset == VideoPreset::KandinskySft {
        "kandinsky5lite_t2v_sft_5s.safetensors"
    } else {
        "kandinsky5lite_t2v_distilled16steps_5s.safetensors"
    };
    let prompt = generation_prompt(project, clip);
    let mut graph = json!({
        "1": node("DualCLIPLoader", json!({"clip_name1":"qwen_2.5_vl_7b_fp8_scaled.safetensors","clip_name2":"clip_l.safetensors","type":"kandinsky5","device":"default"})),
        "2": node("VAELoader", json!({"vae_name":"hunyuan_video_vae_bf16.safetensors"})),
        "3": node("UNETLoader", json!({"unet_name":model,"weight_dtype":"default"})),
        "4": node("CLIPTextEncode", json!({"text":prompt,"clip":["1",0]})),
        "5": node("CLIPTextEncode", json!({"text":project.negative_prompt,"clip":["1",0]})),
        "6": node("Kandinsky5ImageToVideo", json!({"positive":["4",0],"negative":["5",0],"vae":["2",0],"width":project.width,"height":project.height,"length":project.frames_per_clip,"batch_size":1})),
        "7": node("ModelSamplingSD3", json!({"model":["3",0],"shift":5.0})),
        "8": node("KSampler", json!({"model":["7",0],"seed":clip.seed,"steps":project.steps,"cfg":project.cfg,"sampler_name":"euler_ancestral","scheduler":"beta","positive":["6",0],"negative":["6",1],"latent_image":["6",2],"denoise":1.0})),
        "9": tiled_vae_decode(json!(["8",0]), json!(["2",0])),
        "10": node("CreateVideo", json!({"images":["9",0],"fps":project.fps as f64})),
        "11": node("SaveVideo", json!({"video":["10",0],"filename_prefix":prefix,"format":"mp4","codec":"h264"}))
    });
    if let Some(image) = references.image_input.as_deref() {
        graph["12"] = node("LoadImage", json!({"image":image}));
        graph["6"]["inputs"]["start_image"] = json!(["12", 0]);
    }
    graph
}

fn vace_graph(
    project: &VideoProject,
    clip: &VideoClip,
    prefix: &str,
    references: &PreparedReferences,
) -> Value {
    let prompt = generation_prompt(project, clip);
    let mut graph = json!({
        "1": node("UNETLoader", json!({"unet_name":"wan2.1_vace_1.3B_fp16.safetensors","weight_dtype":"default"})),
        "2": node("CLIPLoader", json!({"clip_name":"umt5_xxl_fp8_e4m3fn_scaled.safetensors","type":"wan","device":"default"})),
        "3": node("VAELoader", json!({"vae_name":"wan_2.1_vae.safetensors"})),
        "4": node("CLIPTextEncode", json!({"text":prompt,"clip":["2",0]})),
        "5": node("CLIPTextEncode", json!({"text":project.negative_prompt,"clip":["2",0]})),
        "6": node("WanVaceToVideo", json!({"positive":["4",0],"negative":["5",0],"vae":["3",0],"width":project.width,"height":project.height,"length":project.frames_per_clip,"batch_size":1,"strength":1.0})),
        "7": node("ModelSamplingSD3", json!({"model":["1",0],"shift":8.0})),
        "8": node("KSampler", json!({"model":["7",0],"seed":clip.seed,"steps":project.steps,"cfg":project.cfg,"sampler_name":"uni_pc","scheduler":"simple","positive":["6",0],"negative":["6",1],"latent_image":["6",2],"denoise":1.0})),
        "9": node("TrimVideoLatent", json!({"samples":["8",0],"trim_amount":["6",3]})),
        "10": tiled_vae_decode(json!(["9",0]), json!(["3",0])),
        "11": node("CreateVideo", json!({"images":["10",0],"fps":project.fps as f64})),
        "12": node("SaveVideo", json!({"video":["11",0],"filename_prefix":prefix,"format":"mp4","codec":"h264"}))
    });
    if let Some(image) = references.image_input.as_deref() {
        graph["13"] = node("LoadImage", json!({"image":image}));
        graph["6"]["inputs"]["reference_image"] = json!(["13", 0]);
    }
    if let Some(video) = references.video_input.as_deref() {
        graph["14"] = node("LoadVideo", json!({"file":video}));
        graph["15"] = node("GetVideoComponents", json!({"video":["14",0]}));
        graph["6"]["inputs"]["control_video"] = json!(["15", 0]);
    }
    graph
}

fn tiled_vae_decode(samples: Value, vae: Value) -> Value {
    node(
        "VAEDecodeTiled",
        json!({
            "samples": samples,
            "vae": vae,
            "tile_size": 512,
            "overlap": 64,
            "temporal_size": 64,
            "temporal_overlap": 8
        }),
    )
}

fn generation_prompt(project: &VideoProject, clip: &VideoClip) -> String {
    truncate(
        &format!(
            "{}. {}. Maintain this continuity: {}. Audience: {}. No subtitles, captions, logos, watermarks, title cards, or readable text.",
            project.prompt.trim(),
            clip.prompt.trim(),
            project.continuity_bible.trim(),
            project.audience.trim(),
        ),
        14_000,
    )
}

fn node(class_type: &str, inputs: Value) -> Value {
    json!({"class_type":class_type,"inputs":inputs})
}

pub fn snapshot(
    store: &VideoStore,
    settings: VideoSettings,
    backend: VideoBackendSnapshot,
) -> Result<VideoSnapshot, String> {
    let root = PathBuf::from(&settings.comfy_root);
    let presets = [
        VideoPreset::Wan13GpuOnly,
        VideoPreset::WanVace13Reference,
        VideoPreset::KandinskyDistilled,
        VideoPreset::KandinskySft,
        VideoPreset::Wan22Offload,
    ]
    .into_iter()
    .map(|preset| {
        let missing_files = preset
            .required_files()
            .iter()
            .filter(|relative| !root.join(relative.replace('/', "\\")).is_file())
            .map(|value| (*value).to_string())
            .collect::<Vec<_>>();
        VideoPresetStatus {
            id: preset.id().into(),
            label: preset.label().into(),
            profile: preset.profile().id().into(),
            offloading: preset.profile().offloading().into(),
            native_clip_seconds: preset.native_seconds(),
            steps: preset.steps(),
            available: missing_files.is_empty()
                && root.join("main.py").is_file()
                && root.join(".venv/Scripts/python.exe").is_file(),
            missing_files,
            supports_image_reference: preset.supports_image_reference(),
            supports_video_reference: preset.supports_video_reference(),
        }
    })
    .collect();
    Ok(VideoSnapshot {
        settings,
        backend,
        presets,
        projects: store.list_projects()?,
        root: store.root().to_string_lossy().into_owned(),
    })
}

pub fn save_settings(store: &VideoStore, settings: VideoSettings) -> Result<VideoSettings, String> {
    store.save_settings(&settings)?;
    Ok(settings)
}

pub fn reveal_project(store: &VideoStore, id: &str) -> Result<PathBuf, String> {
    store.project_dir(id)
}

pub fn cleanup_staged_inputs(settings: &VideoSettings, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err("Video project id is invalid.".into());
    }
    let input_root = PathBuf::from(&settings.comfy_root).join("input");
    let target = input_root.join("kestrel").join(id);
    if !target.exists() {
        return Ok(());
    }
    let canonical_root = input_root
        .canonicalize()
        .map_err(|error| format!("Could not resolve ComfyUI input root: {error}"))?;
    let canonical_target = target
        .canonicalize()
        .map_err(|error| format!("Could not resolve staged references: {error}"))?;
    if !canonical_target.starts_with(canonical_root.join("kestrel")) {
        return Err("Refused to clean a staged-reference path outside ComfyUI input.".into());
    }
    fs::remove_dir_all(&canonical_target)
        .map_err(|error| format!("Could not clean staged ComfyUI references: {error}"))
}

pub fn update_clip_prompt(
    store: &VideoStore,
    id: &str,
    clip_index: u32,
    prompt: &str,
) -> Result<VideoProject, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() || prompt.chars().count() > 14_000 {
        return Err("Clip prompt must contain 1 to 14,000 characters.".into());
    }
    let mut project = store.get_project(id)?;
    if matches!(
        project.status.as_str(),
        "starting"
            | "running"
            | "verifying"
            | "assembling"
            | "completed"
            | "completed-with-warnings"
    ) {
        return Err("Only a stopped, unfinished project can be edited.".into());
    }
    let clip = project
        .clips
        .iter_mut()
        .find(|clip| clip.index == clip_index)
        .ok_or_else(|| format!("Clip {clip_index} was not found."))?;
    if clip.status == "complete" {
        return Err("A verified clip is immutable; edit an unfinished clip instead.".into());
    }
    clip.prompt = prompt.to_string();
    clip.status = "planned".into();
    clip.error = None;
    clip.comfy_prompt_id = None;
    project.updated_at = Utc::now().to_rfc3339();
    store.save_project(&project)?;
    Ok(project)
}

pub fn import_reference(
    store: &VideoStore,
    settings: &VideoSettings,
    id: &str,
    source: &Path,
    role: VideoReferenceRole,
) -> Result<VideoProject, String> {
    let mut project = store.get_project(id)?;
    ensure_editable_project(&project)?;
    if project.references.len() >= MAX_REFERENCES {
        return Err(format!(
            "A video project can contain at most {MAX_REFERENCES} reference assets."
        ));
    }
    let source = source
        .canonicalize()
        .map_err(|error| format!("Reference file is unavailable: {error}"))?;
    if !source.is_file() {
        return Err("Reference must be a regular local file.".into());
    }
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let kind = if matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp") {
        VideoReferenceKind::Image
    } else if matches!(extension.as_str(), "mp4" | "mov" | "webm" | "mkv" | "m4v") {
        VideoReferenceKind::Video
    } else {
        return Err("Reference must be PNG, JPEG, WebP, BMP, MP4, MOV, WebM, MKV, or M4V.".into());
    };
    if kind == VideoReferenceKind::Image && !project.preset.supports_image_reference() {
        return Err(format!(
            "{} is text-to-video only. Choose Kandinsky, Wan 2.2 TI2V, or Wan VACE before planning a reference production.",
            project.preset.label()
        ));
    }
    if kind == VideoReferenceKind::Video && !project.preset.supports_video_reference() {
        return Err(
            "Motion-reference video requires the Wan VACE 1.3B Reference Studio preset.".into(),
        );
    }
    if kind == VideoReferenceKind::Video && role != VideoReferenceRole::Motion {
        return Err("Video references use the motion role.".into());
    }
    if kind == VideoReferenceKind::Image && role == VideoReferenceRole::Motion {
        return Err("Motion references must be video files.".into());
    }
    let metadata = fs::metadata(&source).map_err(|error| error.to_string())?;
    let limit = if kind == VideoReferenceKind::Image {
        256 * 1024 * 1024
    } else {
        4 * 1024 * 1024 * 1024u64
    };
    if metadata.len() == 0 || metadata.len() > limit {
        return Err(if kind == VideoReferenceKind::Image {
            "Reference image must be between 1 byte and 256 MiB."
        } else {
            "Reference video must be between 1 byte and 4 GiB."
        }
        .into());
    }
    let asset_id = uuid::Uuid::new_v4().to_string();
    let reference_dir = store.project_dir(id)?.join("references");
    fs::create_dir_all(&reference_dir).map_err(|error| error.to_string())?;
    let free = fs2::available_space(&reference_dir)
        .map_err(|error| format!("Could not inspect free disk space: {error}"))?;
    let reserve = project.boundaries.min_free_disk_gib as u64 * 1024 * 1024 * 1024;
    if free < reserve.saturating_add(metadata.len()) {
        return Err(format!(
            "Importing this reference would cross the project's {} GiB free-disk boundary.",
            project.boundaries.min_free_disk_gib
        ));
    }
    let destination = reference_dir.join(format!("{asset_id}.{extension}"));
    let partial = destination.with_extension(format!("{extension}.partial"));
    fs::copy(&source, &partial)
        .map_err(|error| format!("Could not copy reference into the project: {error}"))?;
    fs::rename(&partial, &destination).map_err(|error| error.to_string())?;
    let stored_bytes = fs::metadata(&destination)
        .map_err(|error| error.to_string())?
        .len();
    let asset = VideoReferenceAsset {
        id: asset_id.clone(),
        name: source
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("reference")
            .to_string(),
        kind,
        role,
        stored_path: destination.to_string_lossy().into_owned(),
        bytes: stored_bytes,
        sha256: hash_file(&destination)?,
        created_at: Utc::now().to_rfc3339(),
        preview_path: create_reference_preview(
            settings,
            &destination,
            &reference_dir.join(format!("{asset_id}.preview.jpg")),
        ),
    };
    if project.continuity.primary_reference_id.is_none() && asset.kind == VideoReferenceKind::Image
    {
        project.continuity.mode = VideoContinuityMode::Anchor;
        project.continuity.primary_reference_id = Some(asset_id);
    }
    project.references.push(asset);
    project.updated_at = Utc::now().to_rfc3339();
    store.save_project(&project)?;
    Ok(project)
}

pub fn reference_preview(store: &VideoStore, id: &str, asset_id: &str) -> Result<String, String> {
    let project = store.get_project(id)?;
    let asset = find_reference(&project, asset_id)?;
    let preview = asset
        .preview_path
        .as_deref()
        .ok_or_else(|| "This reference has no local thumbnail.".to_string())?;
    let project_dir = store
        .project_dir(id)?
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let path = PathBuf::from(preview)
        .canonicalize()
        .map_err(|error| format!("Reference thumbnail is unavailable: {error}"))?;
    if !path.starts_with(project_dir) {
        return Err("Reference thumbnail escaped its project directory.".into());
    }
    let bytes = fs::read(&path).map_err(|error| error.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Reference thumbnail exceeds 2 MiB.".into());
    }
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

fn create_reference_preview(
    settings: &VideoSettings,
    source: &Path,
    destination: &Path,
) -> Option<String> {
    let ffmpeg = validated_ffmpeg_command(&settings.ffmpeg_path).ok()?;
    let mut command = std::process::Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(source)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=512:-2:force_original_aspect_ratio=decrease",
            "-q:v",
            "3",
        ])
        .arg(destination);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command.output().ok()?;
    if output.status.success()
        && fs::metadata(destination).is_ok_and(|metadata| metadata.len() >= 256)
    {
        Some(destination.to_string_lossy().into_owned())
    } else {
        let _ = fs::remove_file(destination);
        None
    }
}

pub fn set_continuity(
    store: &VideoStore,
    id: &str,
    mode: VideoContinuityMode,
    primary_reference_id: Option<String>,
) -> Result<VideoProject, String> {
    let mut project = store.get_project(id)?;
    ensure_editable_project(&project)?;
    if mode != VideoContinuityMode::None && !project.preset.supports_image_reference() {
        return Err("This preset cannot consume an image reference.".into());
    }
    if let Some(reference_id) = primary_reference_id.as_deref() {
        let asset = find_reference(&project, reference_id)?;
        if asset.kind != VideoReferenceKind::Image {
            return Err("The primary subject/storyboard reference must be an image.".into());
        }
    }
    project.continuity = VideoContinuitySettings {
        mode,
        primary_reference_id,
    };
    project.updated_at = Utc::now().to_rfc3339();
    store.save_project(&project)?;
    Ok(project)
}

pub fn set_clip_reference(
    store: &VideoStore,
    id: &str,
    clip_index: u32,
    reference_asset_id: Option<String>,
) -> Result<VideoProject, String> {
    let mut project = store.get_project(id)?;
    ensure_editable_project(&project)?;
    if let Some(reference_id) = reference_asset_id.as_deref() {
        let asset = find_reference(&project, reference_id)?;
        if asset.kind == VideoReferenceKind::Image && !project.preset.supports_image_reference() {
            return Err("This preset cannot consume image references.".into());
        }
        if asset.kind == VideoReferenceKind::Video && !project.preset.supports_video_reference() {
            return Err("This preset cannot consume motion-reference video.".into());
        }
    }
    let clip = project
        .clips
        .iter_mut()
        .find(|clip| clip.index == clip_index)
        .ok_or_else(|| format!("Clip {clip_index} was not found."))?;
    if clip.status == "complete" {
        return Err("A verified clip is immutable.".into());
    }
    clip.reference_asset_id = reference_asset_id;
    clip.error = None;
    project.updated_at = Utc::now().to_rfc3339();
    store.save_project(&project)?;
    Ok(project)
}

pub fn set_chapter_reference(
    store: &VideoStore,
    id: &str,
    chapter_index: u32,
    reference_asset_id: Option<String>,
) -> Result<VideoProject, String> {
    let mut project = store.get_project(id)?;
    ensure_editable_project(&project)?;
    if let Some(reference_id) = reference_asset_id.as_deref() {
        let asset = find_reference(&project, reference_id)?;
        if asset.kind == VideoReferenceKind::Image && !project.preset.supports_image_reference() {
            return Err("This preset cannot consume image references.".into());
        }
        if asset.kind == VideoReferenceKind::Video && !project.preset.supports_video_reference() {
            return Err("This preset cannot consume motion-reference video.".into());
        }
    }
    let chapter = project
        .chapters
        .iter_mut()
        .find(|chapter| chapter.index == chapter_index)
        .ok_or_else(|| format!("Chapter {chapter_index} was not found."))?;
    chapter.reference_asset_id = reference_asset_id;
    project.updated_at = Utc::now().to_rfc3339();
    store.save_project(&project)?;
    Ok(project)
}

fn ensure_editable_project(project: &VideoProject) -> Result<(), String> {
    if matches!(
        project.status.as_str(),
        "starting"
            | "running"
            | "verifying"
            | "assembling"
            | "completed"
            | "completed-with-warnings"
    ) {
        Err("Only a stopped, unfinished project can change references.".into())
    } else {
        Ok(())
    }
}

fn find_reference<'a>(
    project: &'a VideoProject,
    reference_id: &str,
) -> Result<&'a VideoReferenceAsset, String> {
    if !valid_id(reference_id) {
        return Err("Reference id is invalid.".into());
    }
    project
        .references
        .iter()
        .find(|asset| asset.id == reference_id)
        .ok_or_else(|| "Reference asset was not found in this project.".into())
}

fn validate_plan_request(request: &VideoPlanRequest) -> Result<(), String> {
    let prompt = request.prompt.trim();
    if prompt.is_empty() {
        return Err("Describe the video before planning.".into());
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err("Video prompt exceeds 32,768 characters.".into());
    }
    if request.audience.trim().is_empty() || request.audience.len() > 2_048 {
        return Err("Audience must be present and no longer than 2,048 characters.".into());
    }
    if request.use_case.trim().is_empty() || request.use_case.len() > 2_048 {
        return Err("Use case must be present and no longer than 2,048 characters.".into());
    }
    if !(2..=MAX_TOTAL_SECONDS).contains(&request.total_duration_seconds) {
        return Err("Target runtime must be between 2 seconds and 12 hours.".into());
    }
    if !(1..=MAX_CLIPS).contains(&request.boundaries.max_clips) {
        return Err(format!("Maximum clips must be between 1 and {MAX_CLIPS}."));
    }
    if request.boundaries.max_retries_per_clip > 10 {
        return Err("Retries per clip cannot exceed 10.".into());
    }
    if request.boundaries.max_failed_clips > request.boundaries.max_clips {
        return Err("Failed-clip boundary cannot exceed the clip boundary.".into());
    }
    if !(1..=100_800).contains(&request.boundaries.max_runtime_minutes) {
        return Err("Runtime boundary must be between 1 minute and 10 weeks.".into());
    }
    if request.boundaries.min_free_disk_gib > 10_000 {
        return Err("Free-disk boundary cannot exceed 10,000 GiB.".into());
    }
    dimensions(&request.preset, &request.orientation)?;
    Ok(())
}

fn validate_settings(settings: &VideoSettings) -> Result<(), String> {
    let root = Path::new(settings.comfy_root.trim());
    if !root.is_absolute() {
        return Err("ComfyUI root must be an absolute local path.".into());
    }
    validated_ffmpeg_command(&settings.ffmpeg_path)?;
    Ok(())
}

fn validate_comfy_root(root: &Path, preset: &VideoPreset) -> Result<(), String> {
    for path in [
        root.join("main.py"),
        root.join(".venv").join("Scripts").join("python.exe"),
    ] {
        if !path.is_file() {
            return Err(format!(
                "Required ComfyUI file is missing: {}",
                path.display()
            ));
        }
    }
    let missing = preset
        .required_files()
        .iter()
        .map(|relative| root.join(relative.replace('/', "\\")))
        .filter(|path| !path.is_file())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "{} is missing required local assets: {}",
            preset.label(),
            missing
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

fn validated_ffmpeg_command(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("FFmpeg path cannot be empty.".into());
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        if !path.is_file() {
            return Err(format!("FFmpeg is missing: {}", path.display()));
        }
        if !path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("ffmpeg.exe") || value == "ffmpeg")
        {
            return Err("Configured FFmpeg file must be named ffmpeg.exe.".into());
        }
        return Ok(path);
    }
    if !matches!(
        trimmed.to_ascii_lowercase().as_str(),
        "ffmpeg" | "ffmpeg.exe"
    ) {
        return Err("Relative FFmpeg command must be exactly ffmpeg.exe.".into());
    }
    Ok(path)
}

fn dimensions(preset: &VideoPreset, orientation: &str) -> Result<(u32, u32), String> {
    match (preset, orientation) {
        (VideoPreset::Wan13GpuOnly | VideoPreset::Wan22Offload, "landscape") => Ok((832, 480)),
        (VideoPreset::Wan13GpuOnly | VideoPreset::Wan22Offload, "portrait") => Ok((480, 832)),
        (VideoPreset::WanVace13Reference, "landscape") => Ok((832, 480)),
        (VideoPreset::WanVace13Reference, "portrait") => Ok((480, 832)),
        (VideoPreset::WanVace13Reference, "square") => Ok((624, 624)),
        (VideoPreset::Wan13GpuOnly, "square") => Ok((624, 624)),
        (VideoPreset::Wan22Offload, "square") => Ok((640, 608)),
        (_, "landscape") => Ok((768, 512)),
        (_, "portrait") => Ok((512, 768)),
        (_, "square") => Ok((624, 624)),
        _ => Err("Orientation must be landscape, portrait, or square.".into()),
    }
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    std::io::copy(&mut file, &mut hash).map_err(|error| error.to_string())?;
    Ok(hex::encode(hash.finalize()))
}

fn seed_from_text(value: &str) -> u64 {
    let bytes = Sha256::digest(value.as_bytes());
    u64::from_le_bytes(bytes[..8].try_into().expect("slice is eight bytes"))
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_PROJECT_FILE_BYTES {
        return Err("Video project record exceeds the 32 MiB durable-state limit.".into());
    }
    let parent = path
        .parent()
        .ok_or_else(|| "JSON path has no parent.".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.bak");
    {
        let mut file = File::create(&temporary).map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
    }
    if path.exists() {
        if backup.exists() {
            fs::remove_file(&backup).map_err(|error| error.to_string())?;
        }
        fs::rename(path, &backup).map_err(|error| error.to_string())?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        return Err(error.to_string());
    }
    if backup.exists() {
        fs::remove_file(&backup).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn read_project_file(path: &Path) -> Result<VideoProject, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("{} is unavailable: {error}", path.display()))?;
    if metadata.len() > MAX_PROJECT_FILE_BYTES {
        return Err(format!("{} exceeds 32 MiB", path.display()));
    }
    serde_json::from_slice(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|error| format!("{} is invalid: {error}", path.display()))
}

fn bound_project(project: &mut VideoProject) {
    project.errors.truncate(500);
    project.title = truncate(&project.title, 160);
    project.planning_note = truncate(&project.planning_note, 2_000);
}

fn valid_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}

fn strip_json_fence(value: &str) -> &str {
    let trimmed = value.trim();
    let without_open = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    without_open
        .strip_suffix("```")
        .unwrap_or(without_open)
        .trim()
}

fn title_from_prompt(prompt: &str) -> String {
    let title = prompt
        .split(['.', '\n'])
        .next()
        .unwrap_or("Untitled video")
        .trim();
    if title.is_empty() {
        "Untitled video".into()
    } else {
        truncate(title, 100)
    }
}

fn truncate(value: &str, chars: usize) -> String {
    if value.chars().count() <= chars {
        value.to_string()
    } else {
        format!(
            "{}…",
            value
                .chars()
                .take(chars.saturating_sub(1))
                .collect::<String>()
        )
    }
}

fn emit(
    app: Option<&AppHandle>,
    kind: &str,
    title: &str,
    detail: &str,
    project_id: Option<&str>,
    clip_index: Option<u32>,
) {
    if let (Some(app), Some(project_id)) = (app, project_id) {
        let _ = app.emit(
            "video-project-event",
            VideoProjectEvent {
                project_id: project_id.into(),
                kind: kind.into(),
                title: title.into(),
                detail: detail.into(),
                clip_index,
                at: Utc::now().to_rfc3339(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn request(total: u32, preset: VideoPreset) -> VideoPlanRequest {
        VideoPlanRequest {
            prompt: "A visual history of flight".into(),
            audience: "secondary school students".into(),
            use_case: "educational short".into(),
            planner_model_id: None,
            preset,
            total_duration_seconds: total,
            orientation: "landscape".into(),
            negative_prompt: "blurry, text, watermark".into(),
            boundaries: VideoBoundarySettings {
                max_clips: MAX_CLIPS,
                ..VideoBoundarySettings::default()
            },
        }
    }

    #[test]
    fn multi_hour_plan_expands_without_model_sized_output() {
        let request = request(7_200, VideoPreset::KandinskyDistilled);
        validate_plan_request(&request).unwrap();
        let count = request
            .total_duration_seconds
            .div_ceil(request.preset.native_seconds());
        let brief = deterministic_brief(&request, count);
        let (chapters, clips) = expand_plan(&request, &brief, count);
        assert_eq!(clips.len(), 1_440);
        assert!(chapters.len() <= 12);
        assert_eq!(clips.first().unwrap().index, 1);
        assert_eq!(clips.last().unwrap().index, 1_440);
    }

    #[test]
    fn maximum_clip_ledger_stays_inside_the_durable_state_limit() {
        let request = request(40_000, VideoPreset::Wan13GpuOnly);
        let brief = deterministic_brief(&request, MAX_CLIPS);
        let (_, clips) = expand_plan(&request, &brief, MAX_CLIPS);
        assert_eq!(clips.len(), MAX_CLIPS as usize);
        assert!(serde_json::to_vec(&clips).unwrap().len() < 28 * 1024 * 1024);
    }

    #[test]
    fn planner_cannot_allocate_more_chapters_than_clips() {
        let request = request(2, VideoPreset::Wan13GpuOnly);
        let mut brief = deterministic_brief(&request, 1);
        let chapter = brief.chapters[0].clone();
        brief.chapters = vec![chapter; 10];
        normalize_brief(&mut brief, &request, 1);
        let (chapters, clips) = expand_plan(&request, &brief, 1);
        assert_eq!(chapters.len(), 1);
        assert_eq!(clips.len(), 1);
    }

    #[test]
    fn clip_boundary_fails_before_generation() {
        let mut request = request(3_600, VideoPreset::Wan13GpuOnly);
        request.boundaries.max_clips = 100;
        let count = request
            .total_duration_seconds
            .div_ceil(request.preset.native_seconds());
        assert!(count > request.boundaries.max_clips);
    }

    #[tokio::test]
    async fn interrupted_projects_recover_to_resumable_state() {
        let directory = tempdir().unwrap();
        let store = VideoStore::new(directory.path()).unwrap();
        let request = request(10, VideoPreset::KandinskyDistilled);
        let brief = deterministic_brief(&request, 2);
        let (chapters, mut clips) = expand_plan(&request, &brief, 2);
        clips[0].status = "generating".into();
        let project_dir = store
            .root
            .join("projects")
            .join(uuid::Uuid::new_v4().to_string());
        let id = project_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let project = VideoProject {
            id,
            title: "Recovery".into(),
            prompt: request.prompt,
            audience: request.audience,
            use_case: request.use_case,
            preset: request.preset,
            status: "running".into(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            total_duration_seconds: 10,
            clip_duration_seconds: 5,
            width: 768,
            height: 512,
            fps: 24,
            frames_per_clip: 121,
            steps: 16,
            cfg: 1.0,
            negative_prompt: request.negative_prompt,
            continuity_bible: brief.continuity_bible,
            planning_note: "test".into(),
            chapters,
            clips,
            boundaries: request.boundaries,
            output_directory: project_dir.to_string_lossy().into_owned(),
            final_output_path: None,
            errors: Vec::new(),
            references: Vec::new(),
            continuity: VideoContinuitySettings::default(),
        };
        store.save_project(&project).unwrap();
        let summaries = store.list_projects().unwrap();
        assert_eq!(summaries[0].status, "interrupted");
        assert_eq!(
            store.get_project(&project.id).unwrap().clips[0].status,
            "planned"
        );
        let edited = update_clip_prompt(&store, &project.id, 1, "A corrected opening shot")
            .expect("stopped unfinished clips are editable");
        assert_eq!(edited.clips[0].prompt, "A corrected opening shot");
        assert_eq!(edited.clips[0].status, "planned");
        let source = directory.path().join("subject.png");
        fs::write(&source, vec![7u8; 2_048]).unwrap();
        let referenced = import_reference(
            &store,
            &VideoSettings::default(),
            &project.id,
            &source,
            VideoReferenceRole::Subject,
        )
        .unwrap();
        assert_eq!(referenced.references.len(), 1);
        assert_eq!(referenced.continuity.mode, VideoContinuityMode::Anchor);
        assert!(Path::new(&referenced.references[0].stored_path).is_file());
        let independent = set_continuity(
            &store,
            &project.id,
            VideoContinuityMode::None,
            Some(referenced.references[0].id.clone()),
        )
        .unwrap();
        let prepared = prepare_clip_references(
            &VideoSettings {
                comfy_root: directory
                    .path()
                    .join("comfy")
                    .to_string_lossy()
                    .into_owned(),
                ffmpeg_path: "ffmpeg.exe".into(),
            },
            &store,
            &independent,
            0,
        )
        .await
        .unwrap();
        assert!(prepared.image_input.is_none());
        assert!(prepared.video_input.is_none());
        let chapter_reference = set_chapter_reference(
            &store,
            &project.id,
            1,
            Some(referenced.references[0].id.clone()),
        )
        .unwrap();
        assert_eq!(
            chapter_reference.chapters[0].reference_asset_id,
            Some(referenced.references[0].id.clone())
        );
    }

    #[test]
    fn memory_profiles_declare_fixed_offload_behavior() {
        let gpu_only = VideoMemoryProfile::GpuOnly.args();
        assert!(gpu_only.contains(&"--gpu-only"));
        assert!(gpu_only.contains(&"--disable-async-offload"));
        assert!(gpu_only.contains(&"--disable-dynamic-vram"));
        assert_eq!(VideoMemoryProfile::GpuOnly.offloading(), "forbidden");

        for profile in [
            VideoMemoryProfile::ReferenceResident,
            VideoMemoryProfile::KandinskyResident,
        ] {
            assert!(!profile.args().iter().any(|argument| matches!(
                *argument,
                "--gpu-only" | "--highvram" | "--lowvram" | "--novram"
            )));
            assert!(profile.args().contains(&"--disable-async-offload"));
            assert!(profile.args().contains(&"--disable-dynamic-vram"));
            assert!(profile.args().contains(&"--disable-smart-memory"));
            assert_eq!(profile.offloading(), "stage-boundary-only");
        }

        let forced = VideoMemoryProfile::ForcedOffload.args();
        assert!(forced.contains(&"--lowvram"));
        assert!(forced.contains(&"--async-offload"));
        assert_eq!(VideoMemoryProfile::ForcedOffload.offloading(), "forced");
    }

    #[test]
    fn graphs_use_expected_native_nodes() {
        let request = request(5, VideoPreset::KandinskyDistilled);
        let brief = deterministic_brief(&request, 1);
        let (_, clips) = expand_plan(&request, &brief, 1);
        let project = VideoProject {
            id: uuid::Uuid::new_v4().to_string(),
            title: brief.title,
            prompt: request.prompt,
            audience: request.audience,
            use_case: request.use_case,
            preset: request.preset,
            status: "planned".into(),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            total_duration_seconds: 5,
            clip_duration_seconds: 5,
            width: 768,
            height: 512,
            fps: 24,
            frames_per_clip: 121,
            steps: 16,
            cfg: 1.0,
            negative_prompt: request.negative_prompt,
            continuity_bible: brief.continuity_bible,
            planning_note: "test".into(),
            chapters: Vec::new(),
            clips: clips.clone(),
            boundaries: request.boundaries,
            output_directory: "test".into(),
            final_output_path: None,
            errors: Vec::new(),
            references: Vec::new(),
            continuity: VideoContinuitySettings::default(),
        };
        let graph = build_graph(
            &project,
            &clips[0],
            "video/test",
            &PreparedReferences::default(),
        );
        assert_eq!(
            graph.pointer("/6/class_type").and_then(Value::as_str),
            Some("Kandinsky5ImageToVideo")
        );
        assert_eq!(
            graph.pointer("/9/class_type").and_then(Value::as_str),
            Some("VAEDecodeTiled")
        );
        assert_eq!(
            graph
                .pointer("/11/inputs/filename_prefix")
                .and_then(Value::as_str),
            Some("video/test")
        );
        let references = PreparedReferences {
            image_input: Some("kestrel/test/subject.png".into()),
            video_input: Some("kestrel/test/motion.mp4".into()),
        };
        let referenced_kandinsky = build_graph(&project, &clips[0], "video/test", &references);
        assert_eq!(
            referenced_kandinsky
                .pointer("/6/inputs/start_image/0")
                .and_then(Value::as_str),
            Some("12")
        );
        let mut vace = project.clone();
        vace.preset = VideoPreset::WanVace13Reference;
        vace.width = 832;
        vace.height = 480;
        vace.frames_per_clip = 81;
        let vace_graph = build_graph(&vace, &clips[0], "video/vace", &references);
        assert_eq!(
            vace_graph.pointer("/6/class_type").and_then(Value::as_str),
            Some("WanVaceToVideo")
        );
        assert_eq!(
            vace_graph.pointer("/10/class_type").and_then(Value::as_str),
            Some("VAEDecodeTiled")
        );
        assert_eq!(
            vace_graph
                .pointer("/6/inputs/reference_image/0")
                .and_then(Value::as_str),
            Some("13")
        );
        assert_eq!(
            vace_graph
                .pointer("/6/inputs/control_video/0")
                .and_then(Value::as_str),
            Some("15")
        );
    }
}
