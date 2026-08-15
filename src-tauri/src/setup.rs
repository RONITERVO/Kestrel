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
const COMFY_VERSION: &str = "v0.33.1";
const MUSIC_REVISION: &str = "6444666eb6edfb2c7fcab5f8b81da8b84b4b17b6";
const KJ_PREVIEW_REVISION: &str = "5219cd171cb44e2edce9e4daad6cc42c41eded5c";
const TAEH3_REVISION: &str = "62f7591f59dfbb4c3c02b7a621d180a9eeaba26c";
const CHATTERBOX_NODE_REVISION: &str = "f0300cf84ee1b8fc9cbd38cb68cb3bace1895063";
const CHATTERBOX_MODEL_REVISION: &str = "ef85ce7bef2f3f1a74d0d837d379d2fcb68203cd";
const KESTREL_WHISPER_ADAPTER_REVISION: &str = "kestrel-whisper-v1";

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
    #[error("could not prepare {name}: {details}")]
    Dependency { name: String, details: String },
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
        .all(|asset| file_has_size(&comfy.join("models").join(asset.relative), asset.bytes));
    let generic_launcher = comfy.join("Start-Kestrel-ComfyUI.ps1").is_file();
    let legacy_h3_launcher = comfy.join("Start-ComfyUI-MiniMax-H3.ps1").is_file();
    let studio_ready = comfy.join("main.py").is_file()
        && (generic_launcher || legacy_h3_launcher)
        && h3_files
        && h3_live_preview_ready(comfy);
    let studio_partial = comfy.join("main.py").is_file()
        || h3_assets()
            .iter()
            .any(|asset| comfy.join("models").join(asset.relative).is_file());
    let music_files = music_assets()
        .iter()
        .all(|asset| file_has_size(&comfy.join("models").join(asset.relative), asset.bytes));
    let music_nodes = comfy
        .join("comfy_extras")
        .join("nodes_minimax_music.py")
        .is_file();
    let music_launcher = comfy.join("Start-Kestrel-ComfyUI-Music.ps1").is_file();
    let music_ready =
        comfy.join("main.py").is_file() && music_launcher && music_nodes && music_files;
    let music_partial = music_launcher
        || music_nodes
        || music_assets()
            .iter()
            .any(|asset| comfy.join("models").join(asset.relative).is_file());
    let speech_files = speech_assets()
        .iter()
        .all(|asset| file_has_size(&comfy.join(asset.relative), asset.download.bytes));
    let speech_nodes = comfy
        .join("custom_nodes/ComfyUI-Chatterbox/nodes.py")
        .is_file()
        && comfy
            .join("custom_nodes/Kestrel-Whisper/nodes.py")
            .is_file();
    let speech_marker = comfy
        .join("custom_nodes/Kestrel-Whisper/.kestrel-speech-ready")
        .is_file();
    let speech_ready = comfy.join("main.py").is_file()
        && generic_launcher
        && speech_nodes
        && speech_files
        && speech_marker;
    let speech_partial = speech_nodes
        || speech_marker
        || speech_assets()
            .iter()
            .any(|asset| comfy.join(asset.relative).is_file());

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
                "Ready for high-quality local picture-and-sound generation with live sampling previews."
            } else {
                "Optional: about 61 GB plus ComfyUI, the full H3 decoders, and a pinned live-preview decoder; intended for a capable NVIDIA PC."
            },
            &research.comfy_root,
            (65_550_000_000, true),
        ),
        component(
            "music",
            "MiniMax Music 3 Production",
            (music_ready, music_partial),
            if music_ready {
                "Ready for private full-song generation with producer-owned structure and immutable takes."
            } else if comfy.join("main.py").is_file() && !music_nodes {
                "ComfyUI must be updated to 0.33.0 or newer before Kestrel can install the native music workflow."
            } else {
                "Optional: about 12 GB for the verified INT8 model, text encoder, full-quality decoder, and dedicated GPU profile."
            },
            &research.comfy_root,
            (11_915_469_696, true),
        ),
        component(
            "speech",
            "Local voice and dictation",
            (speech_ready, speech_partial),
            if speech_ready {
                "Ready for private Chatterbox narration and timestamped Whisper dictation across Kestrel."
            } else {
                "Optional: installs a local Chatterbox voice and Kestrel's commercially distributable Whisper adapter; no browser or system speech fallback."
            },
            &research.comfy_root,
            (4_810_176_394, true),
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

fn file_has_size(path: &Path, bytes: u64) -> bool {
    fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == bytes)
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
        "music" => install_music(app, settings, &root, cancel).await,
        "speech" => install_speech(app, settings, &root, cancel).await,
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
    let comfy = install_comfy_portable(app, root, "studio", false, &cancel).await?;
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
    let decoder = h3_preview_decoder();
    download(
        app,
        "studio",
        &decoder,
        &comfy.join("models/vae_approx/taeh3.safetensors"),
        &cancel,
    )
    .await?;
    ensure_h3_preview_node(app, &comfy, &cancel).await?;
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

async fn install_comfy_portable(
    app: &AppHandle,
    root: &Path,
    component: &str,
    require_music_nodes: bool,
    cancel: &CancellationToken,
) -> Result<PathBuf, SetupError> {
    let downloads = root.join("downloads");
    fs::create_dir_all(&downloads)?;
    let portable = Asset::new(
        "ComfyUI portable",
        &format!("https://github.com/Comfy-Org/ComfyUI/releases/download/{COMFY_VERSION}/ComfyUI_windows_portable_nvidia.7z"),
        "ComfyUI_windows_portable_nvidia-v0.33.1.7z",
        2_133_107_036,
        "4a221588979b96b8244e0e50b2edca03af732acae1deba69d60aa3b4d60b9dba",
    );
    let archive = downloads.join(&portable.file_name);
    download(app, component, &portable, &archive, cancel).await?;
    let portable_root = root.join("ComfyUI_windows_portable");
    let comfy = portable_root.join("ComfyUI");
    if !comfy.join("main.py").is_file() {
        extract_7z(&archive, root, &portable.name).await?;
    } else if require_music_nodes
        && !comfy.join("comfy_extras/nodes_minimax_music.py").is_file()
        && is_kestrel_managed_comfy_root(&comfy, root)
    {
        replace_managed_comfy_portable(&archive, root, &portable.name).await?;
    }
    if require_music_nodes && !comfy.join("comfy_extras/nodes_minimax_music.py").is_file() {
        return Err(SetupError::Extract {
            name: portable.name,
            details: "the installed ComfyUI does not contain native MiniMax Music 3 nodes; remove the stale Kestrel-owned portable folder or update it to 0.33.0+, then resume".into(),
        });
    }
    Ok(comfy)
}

async fn install_music(
    app: &AppHandle,
    settings: &mut ResearchSettings,
    root: &Path,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    if crate::services::gpu_snapshot().await.is_none() {
        return Err(SetupError::Download {
            name: "MiniMax Music 3 Production".into(),
            details: "no NVIDIA GPU was detected. Keep using the rest of Kestrel, or configure a supported local ComfyUI computer before installing music generation.".into(),
        });
    }
    let configured = PathBuf::from(settings.comfy_root.trim());
    let comfy = if configured.join("main.py").is_file() {
        if !configured
            .join("comfy_extras/nodes_minimax_music.py")
            .is_file()
        {
            if is_kestrel_managed_comfy_root(&configured, root) {
                install_comfy_portable(app, root, "music", true, &cancel).await?
            } else {
                return Err(SetupError::Download {
                    name: "MiniMax Music 3 Production".into(),
                    details: format!(
                        "{} is older than ComfyUI 0.33.0. Update this shared ComfyUI installation first; Kestrel will not overwrite a producer-managed installation.",
                        configured.display()
                    ),
                });
            }
        } else {
            configured
        }
    } else {
        install_comfy_portable(app, root, "music", true, &cancel).await?
    };
    for asset in music_assets() {
        let destination = comfy.join("models").join(asset.relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        download(app, "music", &asset.download_asset(), &destination, &cancel).await?;
    }
    ensure_comfy_launcher(&comfy)?;
    settings.comfy_root = comfy.to_string_lossy().into_owned();
    emit(
        app,
        "music",
        "complete",
        "MiniMax Music 3 is installed and verified for offline production.",
        1,
        1,
        0,
    );
    Ok(())
}

async fn install_speech(
    app: &AppHandle,
    settings: &mut ResearchSettings,
    root: &Path,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    if crate::services::gpu_snapshot().await.is_none() {
        return Err(SetupError::Dependency {
            name: "local voice and dictation".into(),
            details: "no NVIDIA GPU was detected. Kestrel never substitutes browser, Windows, or remote speech; install this component on a supported NVIDIA production computer.".into(),
        });
    }
    let configured = PathBuf::from(settings.comfy_root.trim());
    let comfy = if configured.join("main.py").is_file() {
        configured
    } else {
        install_comfy_portable(app, root, "speech", false, &cancel).await?
    };
    let downloads = root.join("downloads");
    fs::create_dir_all(&downloads)?;
    let chatterbox_node = Asset::new(
        "ComfyUI Chatterbox node",
        &format!(
            "https://github.com/wildminder/ComfyUI-Chatterbox/archive/{CHATTERBOX_NODE_REVISION}.zip"
        ),
        "ComfyUI-Chatterbox-f0300cf.zip",
        267_765,
        "a7ad3a531ba5b2b546d27f5dcd8fe4f490a024089f6cb409e5ff27df99c4dc97",
    );
    let chatterbox_archive = downloads.join(&chatterbox_node.file_name);
    download(
        app,
        "speech",
        &chatterbox_node,
        &chatterbox_archive,
        &cancel,
    )
    .await?;
    install_chatterbox_node(&chatterbox_archive, &comfy)?;
    install_kestrel_whisper_node(&comfy)?;
    for asset in speech_assets() {
        let destination = comfy.join(asset.relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        download(app, "speech", &asset.download, &destination, &cancel).await?;
    }
    ensure_speech_python(app, &comfy, &cancel).await?;
    ensure_comfy_launcher(&comfy)?;
    fs::write(
        comfy.join("custom_nodes/Kestrel-Whisper/.kestrel-speech-ready"),
        format!(
            "{KESTREL_WHISPER_ADAPTER_REVISION}\nchatterbox-node={CHATTERBOX_NODE_REVISION}\nchatterbox-model={CHATTERBOX_MODEL_REVISION}\n"
        ),
    )?;
    settings.comfy_root = comfy.to_string_lossy().into_owned();
    emit(
        app,
        "speech",
        "complete",
        "Local Chatterbox narration and timestamped Whisper dictation are installed and verified.",
        1,
        1,
        0,
    );
    Ok(())
}

fn install_chatterbox_node(archive: &Path, comfy: &Path) -> Result<(), SetupError> {
    let custom_nodes = comfy.join("custom_nodes");
    let target = custom_nodes.join("ComfyUI-Chatterbox");
    let installed_revision = read_bounded_text(&target.join(".kestrel-managed-revision"), 256);
    let target_ready =
        target.join("nodes.py").is_file() && target.join("src/chatterbox/tts.py").is_file();
    if target_ready
        && installed_revision
            .as_deref()
            .is_none_or(|revision| revision.trim() == CHATTERBOX_NODE_REVISION)
    {
        return Ok(());
    }
    if target.exists() && installed_revision.is_none() {
        return Err(SetupError::Dependency {
            name: "ComfyUI Chatterbox node".into(),
            details: format!(
                "{} is incomplete and was not overwritten. Move or remove that folder, then choose Resume in Setup.",
                target.display()
            ),
        });
    }
    fs::create_dir_all(&custom_nodes)?;
    let staging = custom_nodes.join(format!(
        ".kestrel-chatterbox-{}",
        uuid::Uuid::new_v4().simple()
    ));
    unzip(archive, &staging, "ComfyUI Chatterbox node")?;
    let source = staging.join(format!("ComfyUI-Chatterbox-{CHATTERBOX_NODE_REVISION}"));
    if !source.join("nodes.py").is_file() || !source.join("LICENSE").is_file() {
        return Err(SetupError::Extract {
            name: "ComfyUI Chatterbox node".into(),
            details: format!(
                "the verified source archive did not contain the pinned node under {}",
                source.display()
            ),
        });
    }
    let backup = custom_nodes.join(format!(
        ".kestrel-chatterbox-backup-{}",
        uuid::Uuid::new_v4().simple()
    ));
    if target.exists() {
        fs::rename(&target, &backup)?;
    }
    if let Err(error) = fs::rename(&source, &target) {
        if backup.exists() {
            let _ = fs::rename(&backup, &target);
        }
        return Err(SetupError::Extract {
            name: "ComfyUI Chatterbox node".into(),
            details: format!(
                "the pinned node could not replace the prior Kestrel-managed revision ({error}); the prior revision was restored"
            ),
        });
    }
    let _ = fs::remove_dir(&staging);
    fs::write(
        target.join(".kestrel-managed-revision"),
        CHATTERBOX_NODE_REVISION,
    )?;
    if backup.exists() {
        let _ = fs::remove_dir_all(&backup);
    }
    Ok(())
}

fn install_kestrel_whisper_node(comfy: &Path) -> Result<(), SetupError> {
    let target = comfy.join("custom_nodes/Kestrel-Whisper");
    fs::create_dir_all(&target)?;
    fs::write(
        target.join("__init__.py"),
        include_str!("../resources/kestrel_whisper/__init__.py"),
    )?;
    fs::write(
        target.join("nodes.py"),
        include_str!("../resources/kestrel_whisper/nodes.py"),
    )?;
    fs::write(
        target.join("LICENSE"),
        include_str!("../resources/kestrel_whisper/LICENSE"),
    )?;
    fs::write(
        target.join("THIRD_PARTY_NOTICES.md"),
        include_str!("../resources/kestrel_whisper/THIRD_PARTY_NOTICES.md"),
    )?;
    Ok(())
}

async fn ensure_speech_python(
    app: &AppHandle,
    comfy: &Path,
    cancel: &CancellationToken,
) -> Result<(), SetupError> {
    let python = comfy_python(comfy).ok_or_else(|| SetupError::Dependency {
        name: "local speech Python runtime".into(),
        details: format!(
            "ComfyUI's private Python runtime is missing beside {}. Resume Movie Studio, Music Production, or Local voice and dictation from Setup.",
            comfy.display()
        ),
    })?;
    if speech_python_ready(&python).await {
        return Ok(());
    }
    emit(
        app,
        "speech",
        "dependencies",
        "Installing pinned speech support inside ComfyUI's private Python runtime…",
        0,
        0,
        0,
    );
    let mut command = tokio::process::Command::new(&python);
    command.args([
        "-m",
        "pip",
        "install",
        "--disable-pip-version-check",
        "--no-input",
        "--upgrade-strategy",
        "only-if-needed",
        "openai-whisper==20250625",
        "s3tokenizer==0.3.0",
        "conformer==0.3.2",
        "librosa==0.11.0",
        "soundfile==0.14.0",
    ]);
    command
        .current_dir(comfy)
        .env("PIP_DISABLE_PIP_VERSION_CHECK", "1")
        .env("PIP_NO_INPUT", "1")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::select! {
        result = command.output() => result?,
        _ = cancel.cancelled() => return Err(SetupError::Cancelled),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(SetupError::Dependency {
            name: "local speech Python packages".into(),
            details: bounded_setup_detail(if stderr.trim().is_empty() {
                stdout.as_ref()
            } else {
                stderr.as_ref()
            }),
        });
    }
    if !speech_python_ready(&python).await {
        return Err(SetupError::Dependency {
            name: "local speech Python packages".into(),
            details: "the package installer reported success, but ComfyUI still cannot import Whisper, Chatterbox's tokenizer, or its decoder support. Choose Resume to repair the private environment.".into(),
        });
    }
    Ok(())
}

async fn speech_python_ready(python: &Path) -> bool {
    let mut command = tokio::process::Command::new(python);
    command.args([
        "-c",
        "import whisper,s3tokenizer,conformer,librosa,soundfile; print('kestrel-speech-ready')",
    ]);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
        .output()
        .await
        .is_ok_and(|output| output.status.success())
}

fn comfy_python(comfy: &Path) -> Option<PathBuf> {
    [
        comfy.join(".venv/Scripts/python.exe"),
        comfy
            .parent()
            .unwrap_or(comfy)
            .join("python_embeded/python.exe"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

fn bounded_setup_detail(value: &str) -> String {
    let value = value.trim();
    if value.chars().count() <= 4_000 {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(4_000).collect::<String>())
    }
}

fn is_kestrel_managed_comfy_root(comfy: &Path, install_root: &Path) -> bool {
    let launcher = comfy.join("Start-Kestrel-ComfyUI.ps1");
    let launcher_managed = read_bounded_text(&launcher, 64 * 1024)
        .is_some_and(|value| value.contains("Kestrel-managed shared ComfyUI launcher"));
    let expected = install_root
        .join("ComfyUI_windows_portable")
        .join("ComfyUI");
    let expected_location = paths_equal(comfy, &expected);
    let version_managed = read_bounded_text(&comfy.join("comfyui_version.py"), 4 * 1024)
        .is_some_and(|value| value.contains("__version__"));
    expected_location && (launcher_managed || version_managed)
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn read_bounded_text(path: &Path, limit: u64) -> Option<String> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    fs::read_to_string(path).ok()
}

async fn replace_managed_comfy_portable(
    archive: &Path,
    install_root: &Path,
    name: &str,
) -> Result<(), SetupError> {
    let nonce = uuid::Uuid::new_v4();
    let staging = install_root.join(format!(".kestrel-comfy-update-{nonce}"));
    let staged_portable = staging.join("ComfyUI_windows_portable");
    let portable = install_root.join("ComfyUI_windows_portable");
    let backup = install_root.join(format!("ComfyUI_windows_portable.kestrel-backup-{nonce}"));
    fs::create_dir_all(&staging)?;
    extract_7z(archive, &staging, name).await?;
    let staged_comfy = staged_portable.join("ComfyUI");
    if !staged_comfy.join("main.py").is_file()
        || !staged_comfy
            .join("comfy_extras/nodes_minimax_music.py")
            .is_file()
    {
        return Err(SetupError::Extract {
            name: name.into(),
            details: format!(
                "the replacement was unpacked to {}, but its native Music 3 files are missing; the existing installation was not changed",
                staging.display()
            ),
        });
    }
    fs::rename(&portable, &backup)?;
    if let Err(error) = fs::rename(&staged_portable, &portable) {
        let _ = fs::rename(&backup, &portable);
        return Err(SetupError::Extract {
            name: name.into(),
            details: format!(
                "the replacement could not become active ({error}); Kestrel restored the previous installation"
            ),
        });
    }
    if let Err(error) = migrate_comfy_data(&backup.join("ComfyUI"), &portable.join("ComfyUI")) {
        let failed_replacement = staging.join("ComfyUI_windows_portable.failed");
        let _ = fs::rename(&portable, &failed_replacement);
        let _ = fs::rename(&backup, &portable);
        return Err(SetupError::Extract {
            name: name.into(),
            details: format!(
                "producer data could not be moved into the replacement ({error}); Kestrel restored the previous installation and kept the failed replacement at {}",
                failed_replacement.display()
            ),
        });
    }
    let _ = fs::remove_dir(&staging);
    Ok(())
}

fn migrate_comfy_data(previous: &Path, replacement: &Path) -> Result<(), std::io::Error> {
    let mut moved: Vec<(&str, PathBuf)> = Vec::new();
    for name in [
        "models",
        "input",
        "output",
        "custom_nodes",
        "user",
        ".cache",
    ] {
        let source = previous.join(name);
        if !source.exists() {
            continue;
        }
        let target = replacement.join(name);
        let packaged = replacement.join(format!(".kestrel-packaged-{name}"));
        if target.exists() {
            fs::rename(&target, &packaged)?;
        }
        if let Err(error) = fs::rename(&source, &target) {
            if packaged.exists() {
                let _ = fs::rename(&packaged, &target);
            }
            for (prior_name, prior_packaged) in moved.into_iter().rev() {
                let prior_target = replacement.join(prior_name);
                let _ = fs::rename(&prior_target, previous.join(prior_name));
                if prior_packaged.exists() {
                    let _ = fs::rename(prior_packaged, prior_target);
                }
            }
            return Err(error);
        }
        moved.push((name, packaged));
    }
    Ok(())
}

struct MusicAsset {
    relative: &'static str,
    bytes: u64,
    sha256: &'static str,
}

impl MusicAsset {
    fn download_asset(&self) -> Asset {
        let name = self.relative.rsplit('/').next().unwrap_or(self.relative);
        Asset::new(
            name,
            &format!(
                "https://huggingface.co/Comfy-Org/MiniMax-Music-3/resolve/{MUSIC_REVISION}/{}",
                self.relative
            ),
            name,
            self.bytes,
            self.sha256,
        )
    }
}

fn music_assets() -> Vec<MusicAsset> {
    vec![
        MusicAsset {
            relative: "diffusion_models/minimax_music3_dit_int8_convrot.safetensors",
            bytes: 2_502_161_682,
            sha256: "d6b959633e69899f99f3a92d6741c0fe79f26958a30811e50e372ef978b24d5f",
        },
        MusicAsset {
            relative: "text_encoders/minimax_music3_text_encoder_pruned_int8_convrot.safetensors",
            bytes: 9_196_611_886,
            sha256: "010b7416d2336a08c711bc22ee65849c9623069ddb7d89bec011a75699e52014",
        },
        MusicAsset {
            relative: "vae/minimax_music3_dav.safetensors",
            bytes: 216_696_128,
            sha256: "2a32155b769be01445fcc2a8663b910fc9e1751e18dc1c3ec528064512d9ef0c",
        },
    ]
}

struct SpeechAsset {
    relative: &'static str,
    download: Asset,
}

fn speech_assets() -> Vec<SpeechAsset> {
    let chatterbox = |name: &'static str, bytes: u64, sha256: &'static str| {
        SpeechAsset {
        relative: match name {
            "conds.pt" => "models/tts/chatterbox/resembleai_default_voice/conds.pt",
            "s3gen.safetensors" => {
                "models/tts/chatterbox/resembleai_default_voice/s3gen.safetensors"
            }
            "t3_cfg.safetensors" => {
                "models/tts/chatterbox/resembleai_default_voice/t3_cfg.safetensors"
            }
            "tokenizer.json" => {
                "models/tts/chatterbox/resembleai_default_voice/tokenizer.json"
            }
            "ve.safetensors" => {
                "models/tts/chatterbox/resembleai_default_voice/ve.safetensors"
            }
            _ => unreachable!("fixed Chatterbox setup asset"),
        },
        download: Asset::new(
            name,
            &format!(
                "https://huggingface.co/ResembleAI/chatterbox/resolve/{CHATTERBOX_MODEL_REVISION}/{name}"
            ),
            name,
            bytes,
            sha256,
        ),
    }
    };
    vec![
        chatterbox(
            "conds.pt",
            107_374,
            "6552d70568833628ba019c6b03459e77fe71ca197d5c560cef9411bee9d87f4e",
        ),
        chatterbox(
            "s3gen.safetensors",
            1_056_484_620,
            "2b78103c654207393955e4900aac14a12de8ef25f4b09424f1ef91941f161d4e",
        ),
        chatterbox(
            "t3_cfg.safetensors",
            2_129_653_744,
            "914cb1696f47527fe8852ca8f1fe1fa63cb34f76f9c715e84e067b744dd0da81",
        ),
        chatterbox(
            "tokenizer.json",
            25_470,
            "d71e3a44eabb1784df9a68e9f95b251ecbf1a7af6a9f50835856b2ca9d8c14a5",
        ),
        chatterbox(
            "ve.safetensors",
            5_695_784,
            "f0921cab452fa278bc25cd23ffd59d36f816d7dc5181dd1bef9751a7fb61f63c",
        ),
        SpeechAsset {
            relative: "models/stt/whisper/large-v3-turbo.pt",
            download: Asset::new(
                "Whisper large-v3-turbo",
                "https://openaipublic.azureedge.net/main/whisper/models/aff26ae408abcba5fbf8813c21e62b0941638c5f6eebfb145be0c9839262a19a/large-v3-turbo.pt",
                "large-v3-turbo.pt",
                1_617_941_637,
                "aff26ae408abcba5fbf8813c21e62b0941638c5f6eebfb145be0c9839262a19a",
            ),
        },
    ]
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

fn h3_preview_decoder() -> Asset {
    Asset::new(
        "MiniMax H3 live-preview decoder",
        &format!(
            "https://raw.githubusercontent.com/madebyollin/taehv/{TAEH3_REVISION}/safetensors/taeh3.safetensors"
        ),
        "taeh3.safetensors",
        22_709_752,
        "4fd022bfcab08772fe0536b17ea1a3bbb5625be11e397868d1c5d891863d4c13",
    )
}

fn bundled_preview_node_ready(comfy: &Path) -> bool {
    let source = comfy.join("custom_nodes/ComfyUI-KJNodes/nodes/preview_override_node.py");
    let tiny_vae = comfy.join("custom_nodes/ComfyUI-KJNodes/nodes/tiny_vae.py");
    source.is_file()
        && tiny_vae.is_file()
        && fs::metadata(&source).is_ok_and(|metadata| metadata.len() <= 128 * 1024)
        && fs::read_to_string(source)
            .map(|value| {
                value.contains("class ModelPreviewOverrideKJ")
                    && value.contains("kj_preview_override")
                    && value.contains("tiny_vae")
            })
            .unwrap_or(false)
}

fn managed_preview_node_ready(comfy: &Path) -> bool {
    let root = comfy.join("custom_nodes/Kestrel-H3-Live-Preview");
    root.join("__init__.py").is_file()
        && root.join("preview_override_node.py").is_file()
        && root.join("tiny_vae.py").is_file()
        && root.join("LICENSE").is_file()
}

fn h3_live_preview_ready(comfy: &Path) -> bool {
    file_has_size(
        &comfy.join("models/vae_approx/taeh3.safetensors"),
        h3_preview_decoder().bytes,
    ) && (bundled_preview_node_ready(comfy) || managed_preview_node_ready(comfy))
}

async fn ensure_h3_preview_node(
    app: &AppHandle,
    comfy: &Path,
    cancel: &CancellationToken,
) -> Result<(), SetupError> {
    if bundled_preview_node_ready(comfy) {
        return Ok(());
    }
    let bundled_root = comfy.join("custom_nodes/ComfyUI-KJNodes/nodes");
    if bundled_root.join("preview_override_node.py").is_file() {
        // Repair a stale but loadable KJNodes registration in place. Installing the managed
        // fallback beside it would let ComfyUI register ModelPreviewOverrideKJ twice.
        for (name, remote, file_name, bytes, sha256) in [
            (
                "MiniMax live-preview node",
                "nodes/preview_override_node.py",
                "preview_override_node.py",
                37_220,
                "327804957a278d72f86ae45b35e8573d0d310d84b6f2469b1384d8922436bcc8",
            ),
            (
                "MiniMax tiny-VAE loader",
                "nodes/tiny_vae.py",
                "tiny_vae.py",
                4_434,
                "27b39e555d876775f179137d86dc1cbf317967ecd471d59197f1a179b6356ee3",
            ),
        ] {
            let asset = Asset::new(
                name,
                &format!(
                    "https://raw.githubusercontent.com/kijai/ComfyUI-KJNodes/{KJ_PREVIEW_REVISION}/{remote}"
                ),
                file_name,
                bytes,
                sha256,
            );
            download(app, "studio", &asset, &bundled_root.join(file_name), cancel).await?;
        }
        return Ok(());
    }
    let root = comfy.join("custom_nodes/Kestrel-H3-Live-Preview");
    fs::create_dir_all(&root)?;
    let assets = [
        (
            "MiniMax live-preview node",
            "nodes/preview_override_node.py",
            "preview_override_node.py",
            37_220,
            "327804957a278d72f86ae45b35e8573d0d310d84b6f2469b1384d8922436bcc8",
        ),
        (
            "MiniMax tiny-VAE loader",
            "nodes/tiny_vae.py",
            "tiny_vae.py",
            4_434,
            "27b39e555d876775f179137d86dc1cbf317967ecd471d59197f1a179b6356ee3",
        ),
        (
            "KJNodes GPL-3 license",
            "LICENSE",
            "LICENSE",
            35_149,
            "3972dc9744f6499f0f9b2dbf76696f2ae7ad8af9b23dde66d6af86c9dfb36986",
        ),
    ];
    for (name, remote, file_name, bytes, sha256) in assets {
        let asset = Asset::new(
            name,
            &format!(
                "https://raw.githubusercontent.com/kijai/ComfyUI-KJNodes/{KJ_PREVIEW_REVISION}/{remote}"
            ),
            file_name,
            bytes,
            sha256,
        );
        download(app, "studio", &asset, &root.join(file_name), cancel).await?;
    }
    let init = r#"# Kestrel-managed minimal H3 live-preview plugin.
# Upstream source and GPL-3 license are pinned beside this file.
from .preview_override_node import ModelPreviewOverrideKJ

NODE_CLASS_MAPPINGS = {"ModelPreviewOverrideKJ": ModelPreviewOverrideKJ}
NODE_DISPLAY_NAME_MAPPINGS = {"ModelPreviewOverrideKJ": "Model Preview Override KJ"}
"#;
    fs::write(root.join("__init__.py"), init)?;
    Ok(())
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
    let archive = archive.to_path_buf();
    let destination = destination.to_path_buf();
    let label = name.to_string();
    tokio::task::spawn_blocking(move || {
        sevenz_rust::decompress_file_with_extract_fn(
            &archive,
            &destination,
            |entry, reader, output| {
                let relative = Path::new(entry.name());
                if relative.is_absolute()
                    || relative
                        .components()
                        .any(|component| !matches!(component, std::path::Component::Normal(_)))
                {
                    return Err(sevenz_rust::Error::other("archive contains an unsafe path"));
                }
                sevenz_rust::default_entry_extract_fn(entry, reader, output)
            },
        )
        .map_err(|error| SetupError::Extract {
            name: label,
            details: format!(
                "Kestrel's built-in 7z reader could not unpack the verified archive: {error}"
            ),
        })
    })
    .await
    .map_err(|error| SetupError::Extract {
        name: name.into(),
        details: format!("the archive worker stopped unexpectedly: {error}"),
    })?
}

pub fn ensure_comfy_launcher(comfy: &Path) -> Result<(), SetupError> {
    let generic_script = r#"# Kestrel-managed shared ComfyUI launcher
param(
  [int]$Port = 8188,
  [switch]$NoBrowser
)
$ErrorActionPreference='Stop'
$root=$PSScriptRoot
$python=Join-Path $root '.venv\Scripts\python.exe'
if(-not (Test-Path $python)){ $python=Join-Path (Split-Path $root -Parent) 'python_embeded\python.exe' }
if(-not (Test-Path $python)){ throw "ComfyUI Python is missing: $python" }
if(-not (Test-Path (Join-Path $root 'main.py'))){ throw "ComfyUI main.py is missing from $root" }
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
$env:HF_HOME=(Join-Path $root '.cache\huggingface')
$env:HF_HUB_OFFLINE='1'
$env:TRANSFORMERS_OFFLINE='1'
& $python @arguments
exit $LASTEXITCODE
"#;
    let generic_target = comfy.join("Start-Kestrel-ComfyUI.ps1");
    let generic_managed = fs::read_to_string(&generic_target)
        .map(|value| value.contains("Kestrel-managed shared ComfyUI launcher"))
        .unwrap_or(false);
    if !generic_target.is_file() || generic_managed {
        fs::write(generic_target, generic_script)?;
    }
    let music_script = r#"# Kestrel-managed MiniMax Music 3 GPU launcher
param(
  [int]$Port = 8189,
  [switch]$NoBrowser
)
$ErrorActionPreference='Stop'
$root=$PSScriptRoot
$python=Join-Path $root '.venv\Scripts\python.exe'
if(-not (Test-Path $python)){ $python=Join-Path (Split-Path $root -Parent) 'python_embeded\python.exe' }
if(-not (Test-Path $python)){ throw "ComfyUI Python is missing: $python" }
if(-not (Test-Path (Join-Path $root 'main.py'))){ throw "ComfyUI main.py is missing from $root" }
$required=[ordered]@{
  'models\diffusion_models\minimax_music3_dit_int8_convrot.safetensors'=2502161682
  'models\text_encoders\minimax_music3_text_encoder_pruned_int8_convrot.safetensors'=9196611886
  'models\vae\minimax_music3_dav.safetensors'=216696128
}
$missing=@($required.GetEnumerator() | Where-Object {
  $path=Join-Path $root $_.Key
  (-not (Test-Path -LiteralPath $path)) -or ((Get-Item -LiteralPath $path).Length -ne $_.Value)
} | ForEach-Object { $_.Key })
if($missing){ throw "MiniMax Music 3 files are missing or incomplete: $($missing -join ', '). Open Kestrel Setup and resume Music Production." }
$arguments=@(
  (Join-Path $root 'main.py'),
  '--listen','127.0.0.1',
  '--port',[string]$Port,
  '--cuda-device','0',
  '--preview-method','none',
  '--disable-async-offload',
  '--enable-dynamic-vram',
  '--reserve-vram','1.0',
  '--cache-none'
)
$env:PYTHONUTF8='1'
$env:CUDA_VISIBLE_DEVICES='0'
$env:HF_HOME=(Join-Path $root '.cache\huggingface')
$env:HF_HUB_OFFLINE='1'
$env:TRANSFORMERS_OFFLINE='1'
& $python -c 'import sageattention' 2>$null
if($LASTEXITCODE -eq 0){ $arguments += '--use-sage-attention' }
& $python @arguments
exit $LASTEXITCODE
"#;
    let music_target = comfy.join("Start-Kestrel-ComfyUI-Music.ps1");
    let music_managed = fs::read_to_string(&music_target)
        .map(|value| value.contains("Kestrel-managed MiniMax Music 3 GPU launcher"))
        .unwrap_or(false);
    if !music_target.is_file() || music_managed {
        fs::write(music_target, music_script)?;
    }
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
  'models\vae_approx\taeh3.safetensors'=22709752
}
$missing=@($required.GetEnumerator() | Where-Object {
  $path=Join-Path $root $_.Key
  (-not (Test-Path -LiteralPath $path)) -or ((Get-Item -LiteralPath $path).Length -ne $_.Value)
} | ForEach-Object { $_.Key })
if($missing){ throw "MiniMax H3 files are missing or incomplete: $($missing -join ', '). Open Kestrel Setup and resume Movie Studio." }
$bundledPreview=@(
  (Join-Path $root 'custom_nodes\ComfyUI-KJNodes\nodes\preview_override_node.py'),
  (Join-Path $root 'custom_nodes\ComfyUI-KJNodes\nodes\tiny_vae.py')
)
$managedPreview=@(
  (Join-Path $root 'custom_nodes\Kestrel-H3-Live-Preview\__init__.py'),
  (Join-Path $root 'custom_nodes\Kestrel-H3-Live-Preview\preview_override_node.py'),
  (Join-Path $root 'custom_nodes\Kestrel-H3-Live-Preview\tiny_vae.py'),
  (Join-Path $root 'custom_nodes\Kestrel-H3-Live-Preview\LICENSE')
)
$bundledReady=@($bundledPreview | Where-Object { Test-Path -LiteralPath $_ }).Count -eq $bundledPreview.Count
$managedReady=@($managedPreview | Where-Object { Test-Path -LiteralPath $_ }).Count -eq $managedPreview.Count
if(-not ($bundledReady -or $managedReady)){
  throw "MiniMax H3 live preview is not installed. Open Kestrel Setup and resume Movie Studio."
}
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
$env:HF_HOME=(Join-Path $root '.cache\huggingface')
$env:HF_HUB_OFFLINE='1'
$env:TRANSFORMERS_OFFLINE='1'
& $python @arguments
exit $LASTEXITCODE
"#;
    let legacy_target = comfy.join("Start-ComfyUI-MiniMax-H3.ps1");
    let managed = fs::read_to_string(&legacy_target)
        .map(|value| value.contains("Kestrel-managed MiniMax H3 launcher"))
        .unwrap_or(false);
    if !legacy_target.is_file() || managed {
        fs::write(legacy_target, script)?;
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
            comfy.join("Start-Kestrel-ComfyUI.ps1"),
            comfy.join("Start-ComfyUI-MiniMax-H3.ps1"),
            comfy.join("custom_nodes/ComfyUI-Chatterbox/nodes.py"),
            comfy.join("custom_nodes/Kestrel-Whisper/nodes.py"),
            comfy.join("custom_nodes/Kestrel-Whisper/.kestrel-speech-ready"),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        for asset in h3_assets() {
            let path = comfy.join("models").join(asset.relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::File::create(path)
                .unwrap()
                .set_len(asset.bytes)
                .unwrap();
        }
        for asset in speech_assets() {
            let path = comfy.join(asset.relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::File::create(path)
                .unwrap()
                .set_len(asset.download.bytes)
                .unwrap();
        }
        let decoder = comfy.join("models/vae_approx/taeh3.safetensors");
        fs::create_dir_all(decoder.parent().unwrap()).unwrap();
        fs::File::create(decoder)
            .unwrap()
            .set_len(h3_preview_decoder().bytes)
            .unwrap();
        let preview = comfy.join("custom_nodes/ComfyUI-KJNodes/nodes");
        fs::create_dir_all(&preview).unwrap();
        fs::write(
            preview.join("preview_override_node.py"),
            b"class ModelPreviewOverrideKJ: pass\nkj_preview_override = True\ntiny_vae = True",
        )
        .unwrap();
        fs::write(preview.join("tiny_vae.py"), b"x").unwrap();
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
                .find(|item| item.id == "speech")
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

    #[test]
    fn only_the_portable_install_root_is_classified_as_kestrel_managed() {
        let root = tempfile::tempdir().unwrap();
        let managed = root.path().join("ComfyUI_windows_portable/ComfyUI");
        fs::create_dir_all(&managed).unwrap();
        fs::write(
            managed.join("comfyui_version.py"),
            "__version__ = \"0.32.0\"",
        )
        .unwrap();
        assert!(is_kestrel_managed_comfy_root(&managed, root.path()));

        let producer = root.path().join("Producer-ComfyUI");
        fs::create_dir_all(&producer).unwrap();
        fs::write(
            producer.join("Start-Kestrel-ComfyUI.ps1"),
            "# Kestrel-managed shared ComfyUI launcher",
        )
        .unwrap();
        assert!(!is_kestrel_managed_comfy_root(&producer, root.path()));
    }

    #[test]
    fn music_launcher_gets_a_dedicated_gpu_resident_profile() {
        let root = tempfile::tempdir().unwrap();
        ensure_comfy_launcher(root.path()).unwrap();

        let music =
            fs::read_to_string(root.path().join("Start-Kestrel-ComfyUI-Music.ps1")).unwrap();
        assert!(music.contains("[int]$Port = 8189"));
        assert!(music.contains("'--disable-async-offload'"));
        assert!(music.contains("'--enable-dynamic-vram'"));
        assert!(music.contains("'--reserve-vram','1.0'"));
        assert!(!music.contains("'--lowvram'"));
        assert!(music.contains("$env:HF_HUB_OFFLINE='1'"));

        let shared = fs::read_to_string(root.path().join("Start-Kestrel-ComfyUI.ps1")).unwrap();
        assert!(shared.contains("[int]$Port = 8188"));
        assert!(shared.contains("'--lowvram'"));
        assert!(shared.contains("$env:HF_HUB_OFFLINE='1'"));
    }

    #[test]
    fn kestrel_whisper_installer_writes_the_owned_offline_adapter() {
        let root = tempfile::tempdir().unwrap();
        install_kestrel_whisper_node(root.path()).unwrap();

        let node =
            fs::read_to_string(root.path().join("custom_nodes/Kestrel-Whisper/nodes.py")).unwrap();
        assert!(node.contains("class KestrelWhisper"));
        assert!(node.contains("EXPECTED_MODELS"));
        assert!(node.contains("/kestrel/speech/free"));
        assert!(!node.contains("http://"));
        assert!(!node.contains("https://"));
        assert!(root
            .path()
            .join("custom_nodes/Kestrel-Whisper/THIRD_PARTY_NOTICES.md")
            .is_file());
    }

    #[test]
    #[ignore = "writes the owned adapter into KESTREL_LIVE_COMFY_ROOT for local acceptance"]
    fn live_install_kestrel_whisper_adapter_into_configured_comfy() {
        let root = std::env::var_os("KESTREL_LIVE_COMFY_ROOT")
            .map(PathBuf::from)
            .expect("KESTREL_LIVE_COMFY_ROOT is required");
        assert!(root.join("main.py").is_file());
        install_kestrel_whisper_node(&root).unwrap();
        ensure_comfy_launcher(&root).unwrap();
        let python = comfy_python(&root).expect("ComfyUI Python is required");
        let status = std::process::Command::new(python)
            .args([
                "-m",
                "py_compile",
                root.join("custom_nodes/Kestrel-Whisper/nodes.py")
                    .to_str()
                    .unwrap(),
            ])
            .status()
            .unwrap();
        assert!(status.success());
    }
}
