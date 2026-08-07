use crate::models::{ControlSettings, ResearchSettings};
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

const BONSAI_REVISION: &str = "abbae723028d71be674e71e1a71201a6f43fab22";
const BONSAI_RELEASE: &str = "prism-b9596-9fcaed7";
const H3_REVISION: &str = "0bd506d2e895983a9663037febda27aa3948cf48";
const COMFY_VERSION: &str = "v0.30.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupComponent {
    pub id: String,
    pub label: String,
    pub status: String,
    pub detail: String,
    pub path: String,
    pub download_bytes: u64,
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupSnapshot {
    pub ready: bool,
    pub install_root: String,
    pub available_bytes: u64,
    pub gpu_name: Option<String>,
    pub gpu_memory_bytes: u64,
    pub components: Vec<SetupComponent>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupLocations {
    pub install_root: String,
    pub bonsai_root: String,
    pub engine_path: String,
    pub wikipedia_zim_path: String,
    pub kiwix_server_path: String,
    pub comfy_root: String,
    pub ffmpeg_path: String,
    pub ffprobe_path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupInstallRequest {
    pub component: String,
    pub install_root: String,
    #[serde(default = "compact_wikipedia")]
    pub wikipedia_edition: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupProgress {
    pub component: String,
    pub stage: String,
    pub detail: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_second: u64,
}

#[derive(Debug, Error)]
pub enum SetupError {
    #[error("setup path must be an absolute folder: {0}")]
    InvalidPath(String),
    #[error("setup download was paused; choose Resume when you are ready")]
    Cancelled,
    #[error("download failed for {name}: {details}")]
    Download { name: String, details: String },
    #[error("{name} did not pass its integrity check; the partial file was kept for inspection: {details}")]
    Integrity { name: String, details: String },
    #[error("could not unpack {name}: {details}")]
    Extract { name: String, details: String },
    #[error("setup file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not enough free space for {name}: {needed} remains but only {available} is available. Choose another drive in Setup or free space, then resume")]
    InsufficientSpace {
        name: String,
        needed: String,
        available: String,
    },
    #[error("setup service response was invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("setup request failed: {0}")]
    Request(#[from] reqwest::Error),
}

fn compact_wikipedia() -> String {
    "compact".into()
}

pub fn snapshot(
    research: &ResearchSettings,
    control: &ControlSettings,
    gpu: Option<&crate::models::GpuSnapshot>,
) -> SetupSnapshot {
    let bonsai_root = Path::new(&research.bonsai_root);
    let engine = Path::new(&control.engine_path);
    let model = first_matching_file(&bonsai_root.join("models"), "Q2_0.gguf", false);
    let projector = first_matching_file(&bonsai_root.join("models"), "mmproj", false);
    let engine_ready = engine.is_file() && crate::runtime::is_llama_server_file(engine);
    let assistant_ready = engine_ready && model.is_some() && projector.is_some();
    let assistant_partial = engine_ready || model.is_some() || projector.is_some();

    let zim = Path::new(&research.wikipedia_zim_path);
    let kiwix = Path::new(&research.kiwix_server_path);
    let wikipedia_ready = zim.is_file() && kiwix.is_file();
    let wikipedia_partial = zim.is_file() || kiwix.is_file();

    let comfy = Path::new(&research.comfy_root);
    let h3_files = h3_assets()
        .iter()
        .all(|asset| comfy.join("models").join(asset.relative).is_file());
    let studio_ready = comfy.join("main.py").is_file()
        && comfy.join("Start-ComfyUI-MiniMax-H3.ps1").is_file()
        && h3_files;
    let studio_partial = comfy.join("main.py").is_file()
        || h3_assets()
            .iter()
            .any(|asset| comfy.join("models").join(asset.relative).is_file());

    let ffmpeg = resolve_program(&research.ffmpeg_path, "ffmpeg.exe");
    let ffprobe = resolve_program(&research.ffprobe_path, "ffprobe.exe");
    let media_ready = ffmpeg.is_some() && ffprobe.is_some();

    let components = vec![
        component(
            "assistant",
            "Bonsai assistant",
            (assistant_ready, assistant_partial),
            if assistant_ready {
                "Ready for chat, research planning, computer tasks, and movie direction."
            } else {
                "Needs the local engine, Bonsai 27B model, and image projector."
            },
            &research.bonsai_root,
            (8_447_588_320, false),
        ),
        component(
            "wikipedia",
            "Offline Wikipedia",
            (wikipedia_ready, wikipedia_partial),
            if wikipedia_ready {
                "Ready for private research with no internet connection."
            } else {
                "Choose compact (11.7 GB) or complete text (49.1 GB)."
            },
            &research.wikipedia_zim_path,
            (12_550_000_000, false),
        ),
        component(
            "media",
            "Movie finishing tools",
            (media_ready, ffmpeg.is_some() || ffprobe.is_some()),
            if media_ready {
                "Ready to assemble review cuts without changing native clips."
            } else {
                "Adds FFmpeg and FFprobe for final movie assembly."
            },
            ffmpeg
                .as_deref()
                .and_then(Path::parent)
                .unwrap_or_else(|| Path::new(""))
                .to_string_lossy()
                .as_ref(),
            (80_000_000, true),
        ),
        component(
            "studio",
            "MiniMax H3 Movie Studio",
            (studio_ready, studio_partial),
            if studio_ready {
                "Ready for high-quality local picture-and-sound generation."
            } else {
                "Optional: about 61 GB plus ComfyUI; intended for a capable NVIDIA PC."
            },
            &research.comfy_root,
            (65_550_000_000, true),
        ),
    ];
    SetupSnapshot {
        ready: assistant_ready && wikipedia_ready,
        install_root: research.install_root.clone(),
        available_bytes: available_space_for(Path::new(&research.install_root)),
        gpu_name: gpu.map(|value| value.name.clone()),
        gpu_memory_bytes: gpu.map(|value| value.total_mib * 1024 * 1024).unwrap_or(0),
        components,
    }
}

fn available_space_for(path: &Path) -> u64 {
    let mut current = path;
    while !current.exists() {
        let Some(parent) = current.parent() else {
            return 0;
        };
        current = parent;
    }
    fs2::available_space(current).unwrap_or(0)
}

fn component(
    id: &str,
    label: &str,
    state: (bool, bool),
    detail: &str,
    path: &str,
    package: (u64, bool),
) -> SetupComponent {
    let (ready, partial) = state;
    let (download_bytes, optional) = package;
    SetupComponent {
        id: id.into(),
        label: label.into(),
        status: if ready {
            "ready"
        } else if partial {
            "partial"
        } else {
            "missing"
        }
        .into(),
        detail: detail.into(),
        path: path.into(),
        download_bytes,
        optional,
    }
}

pub fn apply_locations(
    research: &mut ResearchSettings,
    control: &mut ControlSettings,
    locations: SetupLocations,
) -> Result<(), SetupError> {
    for value in [
        &locations.install_root,
        &locations.bonsai_root,
        &locations.engine_path,
        &locations.wikipedia_zim_path,
        &locations.kiwix_server_path,
        &locations.comfy_root,
    ] {
        if !Path::new(value).is_absolute() {
            return Err(SetupError::InvalidPath(value.clone()));
        }
    }
    research.install_root = locations.install_root;
    research.bonsai_root = locations.bonsai_root;
    research.wikipedia_zim_path = locations.wikipedia_zim_path;
    research.kiwix_server_path = locations.kiwix_server_path;
    research.wikipedia_book = Path::new(&research.wikipedia_zim_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(&research.wikipedia_book)
        .to_string();
    research.wikipedia_snapshot = archive_snapshot(&research.wikipedia_book);
    research.comfy_root = locations.comfy_root;
    research.ffmpeg_path = locations.ffmpeg_path;
    research.ffprobe_path = locations.ffprobe_path;
    control.engine_path = locations.engine_path;
    Ok(())
}

pub async fn install_component(
    app: &AppHandle,
    settings: &mut ResearchSettings,
    request: &SetupInstallRequest,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    let root = PathBuf::from(request.install_root.trim());
    if !root.is_absolute() {
        return Err(SetupError::InvalidPath(request.install_root.clone()));
    }
    fs::create_dir_all(&root)?;
    settings.install_root = root.to_string_lossy().into_owned();
    match request.component.as_str() {
        "assistant" => install_assistant(app, settings, &root, cancel).await,
        "wikipedia" => {
            install_wikipedia(app, settings, &root, &request.wikipedia_edition, cancel).await
        }
        "media" => install_media(app, settings, &root, cancel).await,
        "studio" => install_studio(app, settings, &root, cancel).await,
        other => Err(SetupError::Download {
            name: other.into(),
            details: "unknown setup component".into(),
        }),
    }
}

async fn install_assistant(
    app: &AppHandle,
    settings: &mut ResearchSettings,
    root: &Path,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    let bonsai = root.join("Bonsai");
    let runtime = bonsai.join("runtime");
    let downloads = bonsai.join("downloads");
    let models = bonsai.join("models");
    fs::create_dir_all(&downloads)?;
    fs::create_dir_all(&models)?;
    let cuda = crate::services::gpu_snapshot().await.is_some();
    let runtime_assets = if cuda {
        vec![
            Asset::new(
                "Bonsai NVIDIA engine",
                &format!("https://github.com/PrismML-Eng/llama.cpp/releases/download/{BONSAI_RELEASE}/llama-prism-b1-9fcaed7-bin-win-cuda-12.4-x64.zip"),
                "llama-cuda.zip",
                261_776_213,
                "6d109e2930c0eaf2f729c3a6fc58dd7809ce2ba7047bfb294547cc389af6de5d",
            ),
            Asset::new(
                "Bonsai NVIDIA support files",
                &format!("https://github.com/PrismML-Eng/llama.cpp/releases/download/{BONSAI_RELEASE}/cudart-llama-bin-win-cuda-12.4-x64.zip"),
                "llama-cudart.zip",
                391_443_627,
                "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6",
            ),
        ]
    } else {
        vec![Asset::new(
            "Bonsai CPU engine",
            &format!("https://github.com/PrismML-Eng/llama.cpp/releases/download/{BONSAI_RELEASE}/llama-bin-win-cpu-x64.zip"),
            "llama-cpu.zip",
            17_401_584,
            "d0f989f8f035894f4b98c4165305d769a4c14adc841f54a489a157145b1c7a58",
        )]
    };
    for asset in runtime_assets {
        let archive = downloads.join(&asset.file_name);
        download(app, "assistant", &asset, &archive, &cancel).await?;
        unzip(&archive, &runtime, &asset.name)?;
    }
    let model = Asset::new(
        "Bonsai 27B model",
        &format!("https://huggingface.co/prism-ml/Ternary-Bonsai-27B-gguf/resolve/{BONSAI_REVISION}/Ternary-Bonsai-27B-Q2_0.gguf"),
        "Ternary-Bonsai-27B-Q2_0.gguf",
        7_165_121_600,
        "868c11714cf8fe47f5ec9eeb2be0ab1a337112886f92ee0ede6b855c4fa31757",
    );
    download(
        app,
        "assistant",
        &model,
        &models.join(&model.file_name),
        &cancel,
    )
    .await?;
    let projector = Asset::new(
        "Bonsai image understanding",
        &format!("https://huggingface.co/prism-ml/Ternary-Bonsai-27B-gguf/resolve/{BONSAI_REVISION}/Ternary-Bonsai-27B-mmproj-Q8_0.gguf"),
        "Ternary-Bonsai-27B-mmproj-Q8_0.gguf",
        629_246_880,
        "eb561d41a7bbeb0fcf04883c8af11078ef6cae0a66862a0b68443cfca495269d",
    );
    download(
        app,
        "assistant",
        &projector,
        &models.join(&projector.file_name),
        &cancel,
    )
    .await?;
    let settings_path = bonsai.join("settings.json");
    if !settings_path.is_file() {
        fs::write(
            &settings_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "AdvancedMode": true,
                "ContextWindow": settings.context_window,
                "MainMaxOutputTokens": settings.max_output_tokens
            }))?,
        )?;
    }
    settings.bonsai_root = bonsai.to_string_lossy().into_owned();
    emit(
        app,
        "assistant",
        "complete",
        "Bonsai is installed and verified.",
        1,
        1,
        0,
    );
    Ok(())
}

async fn install_wikipedia(
    app: &AppHandle,
    settings: &mut ResearchSettings,
    root: &Path,
    edition: &str,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    let wikipedia = root.join("Wikipedia");
    let downloads = wikipedia.join("downloads");
    let tools = wikipedia.join("tools");
    fs::create_dir_all(&downloads)?;
    let kiwix = Asset::new(
        "Kiwix offline reader",
        "https://download.kiwix.org/release/kiwix-tools/kiwix-tools_win-x86_64-3.8.1.zip",
        "kiwix-tools-3.8.1.zip",
        18_301_924,
        "fcd01ed2b93e9a68632c7863c83b9f66bf64406a66357be1df7b8b75596f3e45",
    );
    let archive = downloads.join(&kiwix.file_name);
    download(app, "wikipedia", &kiwix, &archive, &cancel).await?;
    unzip(&archive, &tools, &kiwix.name)?;
    let server = first_matching_file(&tools, "kiwix-serve.exe", true).ok_or_else(|| {
        SetupError::Extract {
            name: kiwix.name,
            details: "the verified archive did not contain kiwix-serve.exe".into(),
        }
    })?;
    let (name, bytes, sha) = if edition == "complete" {
        (
            "wikipedia_en_all_nopic_2026-06.zim",
            52_690_706_555,
            "441a56d9e05b2d98f8ae9acb7986a513ed47904d73852c92dc6b7d50baa122e5",
        )
    } else {
        (
            "wikipedia_en_all_mini_2026-06.zim",
            12_531_679_311,
            "1d0f8178709481c831272d95f95dccc030e9193e38e732b86b1938ae2606226e",
        )
    };
    let zim = Asset::new(
        "English Wikipedia archive",
        &format!("https://download.kiwix.org/zim/wikipedia/{name}"),
        name,
        bytes,
        sha,
    );
    let zim_path = wikipedia.join(name);
    download(app, "wikipedia", &zim, &zim_path, &cancel).await?;
    settings.kiwix_server_path = server.to_string_lossy().into_owned();
    settings.wikipedia_zim_path = zim_path.to_string_lossy().into_owned();
    settings.wikipedia_book = name.trim_end_matches(".zim").into();
    settings.wikipedia_snapshot = archive_snapshot(&settings.wikipedia_book);
    emit(
        app,
        "wikipedia",
        "complete",
        "Offline Wikipedia is installed and verified.",
        1,
        1,
        0,
    );
    Ok(())
}

async fn install_media(
    app: &AppHandle,
    settings: &mut ResearchSettings,
    root: &Path,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    #[derive(Deserialize)]
    struct ReleaseAsset {
        name: String,
        size: u64,
        browser_download_url: String,
        digest: Option<String>,
    }
    #[derive(Deserialize)]
    struct Release {
        assets: Vec<ReleaseAsset>,
    }
    emit(
        app,
        "media",
        "checking",
        "Finding the current Windows media tools…",
        0,
        0,
        0,
    );
    let client = internet_client()?;
    let release = client
        .get("https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/latest")
        .header(header::USER_AGENT, "Kestrel-Local-Setup")
        .send()
        .await?
        .error_for_status()?
        .json::<Release>()
        .await?;
    let current = release
        .assets
        .into_iter()
        .find(|asset| asset.name == "ffmpeg-master-latest-win64-gpl-shared.zip")
        .ok_or_else(|| SetupError::Download {
            name: "FFmpeg".into(),
            details: "the official build feed has no supported Windows shared archive".into(),
        })?;
    let sha = current
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .ok_or_else(|| SetupError::Download {
            name: "FFmpeg".into(),
            details: "the build feed did not publish a SHA-256 digest".into(),
        })?;
    let media = root.join("MediaTools");
    fs::create_dir_all(&media)?;
    let asset = Asset::new(
        "FFmpeg movie tools",
        &current.browser_download_url,
        &current.name,
        current.size,
        sha,
    );
    let archive = media.join(&current.name);
    download(app, "media", &asset, &archive, &cancel).await?;
    unzip(&archive, &media, &asset.name)?;
    settings.ffmpeg_path = first_matching_file(&media, "ffmpeg.exe", true)
        .ok_or_else(|| SetupError::Extract {
            name: asset.name.clone(),
            details: "the verified archive did not contain ffmpeg.exe".into(),
        })?
        .to_string_lossy()
        .into_owned();
    settings.ffprobe_path = first_matching_file(&media, "ffprobe.exe", true)
        .ok_or_else(|| SetupError::Extract {
            name: asset.name.clone(),
            details: "the verified archive did not contain ffprobe.exe".into(),
        })?
        .to_string_lossy()
        .into_owned();
    emit(
        app,
        "media",
        "complete",
        "Movie finishing tools are installed and verified.",
        1,
        1,
        0,
    );
    Ok(())
}

async fn install_studio(
    app: &AppHandle,
    settings: &mut ResearchSettings,
    root: &Path,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    if crate::services::gpu_snapshot().await.is_none() {
        return Err(SetupError::Download {
            name: "MiniMax H3 Movie Studio".into(),
            details: "no NVIDIA GPU was detected. H3 is optional and is not practical on this computer; the rest of Kestrel can still be installed.".into(),
        });
    }
    let downloads = root.join("downloads");
    fs::create_dir_all(&downloads)?;
    let portable = Asset::new(
        "ComfyUI portable",
        &format!("https://github.com/Comfy-Org/ComfyUI/releases/download/{COMFY_VERSION}/ComfyUI_windows_portable_nvidia.7z"),
        "ComfyUI_windows_portable_nvidia-v0.30.0.7z",
        2_110_797_220,
        "f4353d069dd7342e3bef421f07f003cca53ca84168102705cfc83f66449f5ae5",
    );
    let archive = downloads.join(&portable.file_name);
    download(app, "studio", &portable, &archive, &cancel).await?;
    let portable_root = root.join("ComfyUI_windows_portable");
    if !portable_root.join("ComfyUI").join("main.py").is_file() {
        extract_7z(&archive, root, &portable.name).await?;
    }
    let comfy = portable_root.join("ComfyUI");
    for asset in h3_assets() {
        let destination = comfy.join("models").join(asset.relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        download(
            app,
            "studio",
            &asset.download_asset(),
            &destination,
            &cancel,
        )
        .await?;
    }
    ensure_comfy_launcher(&comfy)?;
    settings.comfy_root = comfy.to_string_lossy().into_owned();
    emit(
        app,
        "studio",
        "complete",
        "MiniMax H3 and ComfyUI are installed and verified.",
        1,
        1,
        0,
    );
    Ok(())
}

struct H3Asset {
    relative: &'static str,
    bytes: u64,
    sha256: &'static str,
}

impl H3Asset {
    fn download_asset(&self) -> Asset {
        let name = self.relative.rsplit('/').next().unwrap_or(self.relative);
        Asset::new(
            name,
            &format!(
                "https://huggingface.co/Comfy-Org/MiniMax-H3/resolve/{H3_REVISION}/{}",
                self.relative
            ),
            name,
            self.bytes,
            self.sha256,
        )
    }
}

fn h3_assets() -> Vec<H3Asset> {
    vec![
        H3Asset {
            relative: "diffusion_models/minimax_h3_fl2va_pruned_int8_convrot.safetensors",
            bytes: 20_970_379_616,
            sha256: "e889202c41dafb67b10d67b97f0d8541508036a6090af23425a5c2615d03c47a",
        },
        H3Asset {
            relative: "diffusion_models/minimax_h3_ref2va_pruned_int8_convrot.safetensors",
            bytes: 20_970_379_616,
            sha256: "9255f52b6677845ad238f20dfaafa94727053694127ab7f255c048f0f9365779",
        },
        H3Asset {
            relative: "text_encoders/qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors",
            bytes: 15_687_142_551,
            sha256: "35a88d51044231fe332301d7a62aa81e3f2cba62febeb446e2c1e3e0ef76f2c6",
        },
        H3Asset {
            relative: "vae/minimax_h3_video_vae_fp16.safetensors",
            bytes: 5_207_808_496,
            sha256: "7c1f131492e7eddacaac9069a61b81bdd39de5cc96561e677c5eab1cdce5e522",
        },
        H3Asset {
            relative: "vae/minimax_h3_audio_vae_fp32.safetensors",
            bytes: 605_254_808,
            sha256: "8e505d95dd1561d47abd43d4238fd40d9bb1ae9e147ed0a4cba778d76ae4db48",
        },
    ]
}

struct Asset {
    name: String,
    url: String,
    file_name: String,
    bytes: u64,
    sha256: String,
}

impl Asset {
    fn new(name: &str, url: &str, file_name: &str, bytes: u64, sha256: &str) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            file_name: file_name.into(),
            bytes,
            sha256: sha256.into(),
        }
    }
}

async fn download(
    app: &AppHandle,
    component: &str,
    asset: &Asset,
    destination: &Path,
    cancel: &CancellationToken,
) -> Result<(), SetupError> {
    if verified(destination, asset).await? {
        emit(
            app,
            component,
            "verified",
            &format!("{} is already complete.", asset.name),
            asset.bytes,
            asset.bytes,
            0,
        );
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = destination.with_extension(format!(
        "{}.part",
        destination
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("download")
    ));
    let existing = fs::metadata(&partial).map(|value| value.len()).unwrap_or(0);
    let remaining = asset.bytes.saturating_sub(existing);
    let space_at = destination.parent().unwrap_or_else(|| Path::new("."));
    let available = fs2::available_space(space_at).unwrap_or(u64::MAX);
    if remaining > available {
        return Err(SetupError::InsufficientSpace {
            name: asset.name.clone(),
            needed: human_bytes(remaining),
            available: human_bytes(available),
        });
    }
    if existing > asset.bytes {
        fs::remove_file(&partial)?;
    }
    let existing = fs::metadata(&partial).map(|value| value.len()).unwrap_or(0);
    let client = internet_client()?;
    let mut request = client.get(&asset.url);
    if existing > 0 {
        request = request.header(header::RANGE, format!("bytes={existing}-"));
    }
    let response = request.send().await.map_err(|error| SetupError::Download {
        name: asset.name.clone(),
        details: error.to_string(),
    })?;
    let resumed = existing > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
    if !response.status().is_success() {
        return Err(SetupError::Download {
            name: asset.name.clone(),
            details: format!("server returned {}", response.status()),
        });
    }
    let mut file = if resumed {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&partial)
            .await?
    } else {
        tokio::fs::File::create(&partial).await?
    };
    let mut downloaded = if resumed { existing } else { 0 };
    let started = Instant::now();
    let mut last_emit = Instant::now() - Duration::from_secs(2);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.is_cancelled() {
            file.flush().await?;
            return Err(SetupError::Cancelled);
        }
        let chunk = chunk.map_err(|error| SetupError::Download {
            name: asset.name.clone(),
            details: error.to_string(),
        })?;
        file.write_all(&chunk).await?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        if last_emit.elapsed() >= Duration::from_millis(400) {
            let rate = downloaded
                .saturating_sub(if resumed { existing } else { 0 })
                .checked_div(started.elapsed().as_secs().max(1))
                .unwrap_or(0);
            emit(
                app,
                component,
                "downloading",
                &format!("Downloading {}", asset.name),
                downloaded,
                asset.bytes,
                rate,
            );
            last_emit = Instant::now();
        }
    }
    file.flush().await?;
    drop(file);
    if fs::metadata(&partial)?.len() != asset.bytes {
        return Err(SetupError::Integrity {
            name: asset.name.clone(),
            details: format!(
                "expected {} bytes, received {}",
                asset.bytes,
                fs::metadata(&partial)?.len()
            ),
        });
    }
    emit(
        app,
        component,
        "verifying",
        &format!("Checking {}", asset.name),
        asset.bytes,
        asset.bytes,
        0,
    );
    let path = partial.clone();
    let expected = asset.sha256.clone();
    let actual = tokio::task::spawn_blocking(move || sha256_file(&path))
        .await
        .map_err(|error| SetupError::Integrity {
            name: asset.name.clone(),
            details: error.to_string(),
        })??;
    if !actual.eq_ignore_ascii_case(&expected) {
        return Err(SetupError::Integrity {
            name: asset.name.clone(),
            details: format!("SHA-256 was {actual}"),
        });
    }
    if destination.is_file() {
        fs::remove_file(destination)?;
    }
    fs::rename(partial, destination)?;
    Ok(())
}

async fn verified(path: &Path, asset: &Asset) -> Result<bool, SetupError> {
    if fs::metadata(path).map(|value| value.len()).unwrap_or(0) != asset.bytes {
        return Ok(false);
    }
    let owned = path.to_path_buf();
    let actual = tokio::task::spawn_blocking(move || sha256_file(&owned))
        .await
        .map_err(|error| SetupError::Integrity {
            name: asset.name.clone(),
            details: error.to_string(),
        })??;
    Ok(actual.eq_ignore_ascii_case(&asset.sha256))
}

fn sha256_file(path: &Path) -> Result<String, SetupError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 4 * 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn unzip(archive: &Path, destination: &Path, name: &str) -> Result<(), SetupError> {
    fs::create_dir_all(destination)?;
    let file = fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| SetupError::Extract {
        name: name.into(),
        details: error.to_string(),
    })?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| SetupError::Extract {
            name: name.into(),
            details: error.to_string(),
        })?;
        let relative = entry.enclosed_name().ok_or_else(|| SetupError::Extract {
            name: name.into(),
            details: "archive contains an unsafe path".into(),
        })?;
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output)?;
        } else {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut target = fs::File::create(output)?;
            std::io::copy(&mut entry, &mut target)?;
            target.flush()?;
        }
    }
    Ok(())
}

async fn extract_7z(archive: &Path, destination: &Path, name: &str) -> Result<(), SetupError> {
    let mut command = tokio::process::Command::new("tar.exe");
    command.arg("-xf").arg(archive).arg("-C").arg(destination);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = command
        .output()
        .await
        .map_err(|error| SetupError::Extract {
            name: name.into(),
            details: format!("Windows archive support could not start: {error}"),
        })?;
    if !output.status.success() {
        return Err(SetupError::Extract {
            name: name.into(),
            details: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(())
}

pub fn ensure_comfy_launcher(comfy: &Path) -> Result<(), SetupError> {
    let script = r#"# Kestrel-managed MiniMax H3 launcher
param(
  [int]$Port = 8188,
  [switch]$NoBrowser
)
$ErrorActionPreference='Stop'
$root=$PSScriptRoot
$python=Join-Path $root '.venv\Scripts\python.exe'
if(-not (Test-Path $python)){ $python=Join-Path (Split-Path $root -Parent) 'python_embeded\python.exe' }
if(-not (Test-Path $python)){ throw "ComfyUI portable Python is missing: $python" }
$required=[ordered]@{
  'models\diffusion_models\minimax_h3_fl2va_pruned_int8_convrot.safetensors'=20970379616
  'models\diffusion_models\minimax_h3_ref2va_pruned_int8_convrot.safetensors'=20970379616
  'models\text_encoders\qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors'=15687142551
  'models\vae\minimax_h3_video_vae_fp16.safetensors'=5207808496
  'models\vae\minimax_h3_audio_vae_fp32.safetensors'=605254808
}
$missing=@($required.GetEnumerator() | Where-Object {
  $path=Join-Path $root $_.Key
  (-not (Test-Path -LiteralPath $path)) -or ((Get-Item -LiteralPath $path).Length -ne $_.Value)
} | ForEach-Object { $_.Key })
if($missing){ throw "MiniMax H3 files are missing or incomplete: $($missing -join ', '). Open Kestrel Setup and resume Movie Studio." }
$arguments=@(
  (Join-Path $root 'main.py'),
  '--listen','127.0.0.1',
  '--port',[string]$Port,
  '--cuda-device','0',
  '--preview-method','none',
  '--lowvram',
  '--async-offload','2',
  '--enable-dynamic-vram',
  '--reserve-vram','1.0',
  '--cache-none',
  '--fast-disk'
)
$env:PYTHONUTF8='1'
$env:CUDA_VISIBLE_DEVICES='0'
& $python @arguments
exit $LASTEXITCODE
"#;
    let target = comfy.join("Start-ComfyUI-MiniMax-H3.ps1");
    let managed = fs::read_to_string(&target)
        .map(|value| value.contains("Kestrel-managed MiniMax H3 launcher"))
        .unwrap_or(false);
    if !target.is_file() || managed {
        fs::write(target, script)?;
    }
    Ok(())
}

fn first_matching_file(root: &Path, needle: &str, exact: bool) -> Option<PathBuf> {
    if !root.is_dir() {
        return None;
    }
    walkdir::WalkDir::new(root)
        .max_depth(5)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| {
            entry.file_type().is_file()
                && entry.file_name().to_str().is_some_and(|name| {
                    if exact {
                        name.eq_ignore_ascii_case(needle)
                    } else {
                        name.to_lowercase().contains(&needle.to_lowercase())
                    }
                })
        })
        .map(walkdir::DirEntry::into_path)
}

pub fn resolve_program(configured: &str, name: &str) -> Option<PathBuf> {
    let configured = Path::new(configured);
    if configured.is_file()
        && configured
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case(name))
    {
        return Some(configured.to_path_buf());
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|path| path.join(name))
            .find(|path| path.is_file())
    })
}

fn archive_snapshot(book: &str) -> String {
    book.rsplit_once('_')
        .map(|(_, value)| value)
        .unwrap_or("unknown")
        .to_string()
}

fn human_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.0} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn internet_client() -> Result<Client, SetupError> {
    Ok(Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(24 * 60 * 60))
        .build()?)
}

fn emit(
    app: &AppHandle,
    component: &str,
    stage: &str,
    detail: &str,
    downloaded_bytes: u64,
    total_bytes: u64,
    bytes_per_second: u64,
) {
    let _ = app.emit(
        "setup-progress",
        SetupProgress {
            component: component.into(),
            stage: stage.into(),
            detail: detail.into(),
            downloaded_bytes,
            total_bytes,
            bytes_per_second,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_locations_reject_relative_paths() {
        let mut research = ResearchSettings::default();
        let mut control = ControlSettings::default();
        let result = apply_locations(
            &mut research,
            &mut control,
            SetupLocations {
                install_root: "relative".into(),
                bonsai_root: "relative".into(),
                engine_path: "relative".into(),
                wikipedia_zim_path: "relative".into(),
                kiwix_server_path: "relative".into(),
                comfy_root: "relative".into(),
                ffmpeg_path: String::new(),
                ffprobe_path: String::new(),
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn setup_snapshot_recognizes_a_complete_local_layout() {
        let root = tempfile::tempdir().unwrap();
        let bonsai = root.path().join("Bonsai");
        let wikipedia = root.path().join("Wikipedia");
        let comfy = root.path().join("ComfyUI");
        for path in [
            bonsai.join("runtime/llama-server.exe"),
            bonsai.join("models/Ternary-Bonsai-27B-Q2_0.gguf"),
            bonsai.join("models/Ternary-Bonsai-27B-mmproj-Q8_0.gguf"),
            wikipedia.join("kiwix-serve.exe"),
            wikipedia.join("archive.zim"),
            comfy.join("main.py"),
            comfy.join("Start-ComfyUI-MiniMax-H3.ps1"),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        for asset in h3_assets() {
            let path = comfy.join("models").join(asset.relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        let research = ResearchSettings {
            bonsai_root: bonsai.to_string_lossy().into_owned(),
            wikipedia_zim_path: wikipedia.join("archive.zim").to_string_lossy().into_owned(),
            kiwix_server_path: wikipedia
                .join("kiwix-serve.exe")
                .to_string_lossy()
                .into_owned(),
            comfy_root: comfy.to_string_lossy().into_owned(),
            ..ResearchSettings::default()
        };
        let control = ControlSettings {
            engine_path: bonsai
                .join("runtime/llama-server.exe")
                .to_string_lossy()
                .into_owned(),
            ..ControlSettings::default()
        };
        let value = snapshot(&research, &control, None);
        assert!(
            value
                .components
                .iter()
                .find(|item| item.id == "assistant")
                .unwrap()
                .status
                == "ready"
        );
        assert!(
            value
                .components
                .iter()
                .find(|item| item.id == "wikipedia")
                .unwrap()
                .status
                == "ready"
        );
        assert!(
            value
                .components
                .iter()
                .find(|item| item.id == "studio")
                .unwrap()
                .status
                == "ready"
        );
    }
}
