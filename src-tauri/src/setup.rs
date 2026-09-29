use crate::models::{ControlSettings, ResearchSettings};
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tauri::AppHandle;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

pub use kestrel_app_core::{
    SetupComponent, SetupInstallRequest, SetupLocations, SetupModelAsset, SetupProgress,
    SetupSnapshot, SetupStrataChoice,
};

const BONSAI_REVISION: &str = "abbae723028d71be674e71e1a71201a6f43fab22";
const BONSAI_RELEASE: &str = "prism-b9596-9fcaed7";
const H3_REVISION: &str = "0bd506d2e895983a9663037febda27aa3948cf48";
const COMFY_VERSION: &str = "v0.33.1";
const MUSIC_REVISION: &str = "6444666eb6edfb2c7fcab5f8b81da8b84b4b17b6";
const IDEOGRAM_REVISION: &str = "9d0e686d42c1b1e575f0de15104d68e9157f59a0";
const QWEN3_VL_REVISION: &str = "d3f437bd7bd2df08e77c8fe5c51ca4239f753aa3";
const FLUX2_REVISION: &str = "06029c966dd5b73929c909f046cbd29303b98879";
const IDEOGRAM_LICENSE_REVISION: &str = "990fe1c4e950bb9e9dc90e01c0ad98ba434f83c2";
const KJ_PREVIEW_REVISION: &str = "3f20054214fec9f9234fd3841ae6f1e4287948f6";
const KJ_PREVIEW_NODE_BYTES: u64 = 37_233;
const KJ_TINY_VAE_BYTES: u64 = 8_999;
const TAEH3_REVISION: &str = "62f7591f59dfbb4c3c02b7a621d180a9eeaba26c";
const CHATTERBOX_NODE_REVISION: &str = "f0300cf84ee1b8fc9cbd38cb68cb3bace1895063";
const CHATTERBOX_MODEL_REVISION: &str = "ef85ce7bef2f3f1a74d0d837d379d2fcb68203cd";
const KESTREL_WHISPER_ADAPTER_REVISION: &str = "kestrel-whisper-v2";
const MUSCRIPTOR_PACKAGE: &str = "muscriptor==0.3.0";
const MUSCRIPTOR_SETUP_REVISION: &str = "muscriptor-0.3.0-uv-0.11.30-cu128-v1";
const MUSCRIPTOR_MODEL_BYTES: u64 = 5_465_642_136;
const KESTREL_MANAGED_COMFY_MARKER: &str = ".kestrel-managed-portable";
const KESTREL_MANAGED_COMFY_MARKER_CONTENT: &str = "Kestrel-managed ComfyUI portable\n";
/// Strata v0.1.20 (MIT). The source archive, ready-made engine, and model shards are pinned by
/// exact bytes and SHA-256; Strata's own setup then prepares its Python packages and the
/// speculative-decoding draft layer from the original Qwen checkpoint.
const STRATA_COMMIT: &str = "c1e903310f211e6630780c3bd2038778c071c68d";
const STRATA_RELEASE: &str = "v0.1.20";
const STRATA_SOURCE_BYTES: u64 = 12_132_007;
const STRATA_SOURCE_SHA256: &str =
    "a0825fdbe78a263f6b39266d8a4f09531ed93463789a15c17e8a734690166840";
const STRATA_ENGINE_BYTES: u64 = 96_403_781;
const STRATA_ENGINE_SHA256: &str =
    "b25c69d2394d1f1444569b5f491dba7ceee6503a31ddf9613ddfbcdb333265cf";
const STRATA_MODEL_REVISION: &str = "2a55d75962e22f7a4a1d9963ab6eae3678537831";
const STRATA_SHARD_TWO_BYTES: u64 = 28_800_138_432;
const STRATA_SHARD_TWO_SHA256: &str =
    "316b46f3a2dbd68c900f43136ab9449f9dcc3725dfd8c794847c204bc161e113";
/// Strata's draft layer (~5 GB of MTP tensors) plus its Python packages and CUDA wheels.
const STRATA_PREPARED_BYTES: u64 = 6_000_000_000;
/// Every Strata model Setup installs serves this window; Strata fixes it at install time.
const STRATA_CONTEXT: u32 = 65_536;
const STRATA_SETUP_MARKER: &str = ".kestrel-strata-revision";
const GIB: u64 = 1024 * 1024 * 1024;

/// One Qwen3.8-Flash-Next size. Strata keeps every expert in system RAM whatever the GPU, so
/// RAM decides what fits. Kestrel tested Strata on a 64 GB PC, where it uses 54-57 GB, and does
/// not offer any size on less, even where Strata's own setup would try.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StrataSize {
    name: &'static str,
    shard_one_bytes: u64,
    shard_one_sha256: &'static str,
    /// The PC class Kestrel supports this size on (installed RAM).
    pc_memory_gib: u64,
    /// Least RAM Windows may report for that class (the tested 64 GB PC reports 63.1 GiB);
    /// below it Setup refuses the size.
    min_memory_gib: u64,
}

const STRATA_SIZES: [StrataSize; 2] = [
    StrataSize {
        name: "IQ3_S",
        shard_one_bytes: 54_817_524_224,
        shard_one_sha256: "4c1eb2ceb4915e1192f4f386021897bde56a97f40a0bb78bb86465e0f7d2aca3",
        pc_memory_gib: 64,
        min_memory_gib: 60,
    },
    StrataSize {
        name: "IQ2_XS",
        shard_one_bytes: 39_225_954_592,
        shard_one_sha256: "92cee27ae5bbadcd732416a0f7a7f0acc092399dbbe8f5a5efa707c2ec0a49d7",
        pc_memory_gib: 64,
        min_memory_gib: 60,
    },
];

impl StrataSize {
    fn parse(value: &str) -> Result<Self, SetupError> {
        STRATA_SIZES
            .into_iter()
            .find(|size| size.name.eq_ignore_ascii_case(value.trim()))
            .ok_or_else(|| SetupError::Dependency {
                name: "Qwen3.8-Flash-Next".into(),
                details: format!("{value} is not a supported size; choose IQ3_S or IQ2_XS"),
            })
    }

    fn shard_name(self, index: u8) -> String {
        format!(
            "Qwen3.8-Flash-Next-GSQ-RCO-{}-0000{index}-of-00002.gguf",
            self.name
        )
    }

    fn shards(self) -> [ReusableModelAsset; 2] {
        [1, 2].map(|index| {
            let file_name = self.shard_name(index);
            let (bytes, sha256) = if index == 1 {
                (self.shard_one_bytes, self.shard_one_sha256)
            } else {
                (STRATA_SHARD_TWO_BYTES, STRATA_SHARD_TWO_SHA256)
            };
            ReusableModelAsset {
                component: "strata",
                relative: format!("models/{}/{file_name}", self.name),
                download: Asset::new(
                    &format!("Qwen3.8-Flash-Next {} part {index} of 2", self.name),
                    &format!(
                        "https://huggingface.co/ISTA-DASLab/Qwen3.8-Flash-Next-GSQ-RCO-GGUF/resolve/{STRATA_MODEL_REVISION}/{}/{file_name}",
                        self.name
                    ),
                    &file_name,
                    bytes,
                    sha256,
                ),
            }
        })
    }
}

/// Strata keeps model files in `Strata-data` beside its folder, as its own setup does.
fn strata_data_dir(strata_root: &Path) -> PathBuf {
    strata_root
        .parent()
        .unwrap_or(strata_root)
        .join("Strata-data")
}

/// The one Strata folder Setup reports on, resumes, and installs into: the saved Strata folder,
/// or `<install root>\Strata` when none is saved.
fn strata_install_dir(research: &ResearchSettings) -> PathBuf {
    let saved = PathBuf::from(research.strata_root.trim());
    if saved.is_absolute() {
        saved
    } else {
        Path::new(&research.install_root).join("Strata")
    }
}

/// Whether Setup has started a Strata install here: its marker, or model shards downloaded or
/// downloading beside it.
fn strata_partial(strata_root: &Path) -> bool {
    let data = strata_data_dir(strata_root);
    strata_root.join(STRATA_SETUP_MARKER).is_file()
        || STRATA_SIZES.iter().any(|size| {
            size.shards().iter().any(|shard| {
                let path = data.join(&shard.relative);
                path.is_file() || path.with_extension("gguf.part").is_file()
            })
        })
}

fn folder_is_empty_or_missing(folder: &Path) -> bool {
    match fs::read_dir(folder) {
        Ok(mut entries) => entries.next().is_none(),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Whether two saved folders name the same place, as Windows compares paths.
fn same_folder(left: &Path, right: &Path) -> bool {
    let normalize = |path: &Path| {
        path.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    };
    normalize(left) == normalize(right)
}

/// Why this PC gets no Strata offer, or `None` when it matches the tested PC. The graphics card
/// and processor come first; RAM is refused only when no size fits.
fn strata_blocker(gpu: Option<&crate::strata::Gpu>, system_memory_bytes: u64) -> Option<String> {
    crate::strata::unsupported_reason(gpu, crate::strata::cpu_has_avx2()).or_else(|| {
        let least = STRATA_SIZES
            .iter()
            .min_by_key(|size| size.min_memory_gib)
            .expect("Strata sizes are pinned");
        (system_memory_bytes < least.min_memory_gib * GIB).then(|| {
            let reported = if system_memory_bytes == 0 {
                "Windows did not report this PC's RAM".to_string()
            } else {
                format!(
                    "This PC reports {:.1} GB of RAM",
                    system_memory_bytes as f64 / GIB as f64
                )
            };
            format!(
                "{reported}. Kestrel tested Strata on a {} GB PC, where it uses 54-57 GB, and does not offer it on less.",
                least.pc_memory_gib
            )
        })
    })
}

const SPEECH_PYTHON_PACKAGES: [&str; 5] = [
    "openai-whisper==20250625",
    "s3tokenizer==0.3.0",
    "conformer==0.3.2",
    "librosa==0.11.0",
    "soundfile==0.14.0",
];

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

pub fn snapshot(
    research: &ResearchSettings,
    control: &ControlSettings,
    gpu: Option<&crate::models::GpuSnapshot>,
    strata_gpu: Option<&crate::strata::Gpu>,
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
    let image_files = ideogram_assets()
        .iter()
        .all(|asset| file_has_size(&comfy.join(asset.relative), asset.download.bytes));
    let image_nodes = comfy.join("comfy_extras/nodes_ideogram4.py").is_file()
        && comfy.join("comfy_extras/nodes_custom_sampler.py").is_file();
    let image_ready =
        comfy.join("main.py").is_file() && generic_launcher && image_nodes && image_files;
    let image_partial = image_nodes
        || ideogram_assets()
            .iter()
            .any(|asset| comfy.join(asset.relative).is_file());
    let speech_files = speech_assets()
        .iter()
        .all(|asset| file_has_size(&comfy.join(asset.relative), asset.download.bytes));
    let speech_nodes = comfy
        .join("custom_nodes/ComfyUI-Chatterbox/nodes.py")
        .is_file()
        && comfy
            .join("custom_nodes/Kestrel-Whisper/nodes.py")
            .is_file();
    let speech_marker = speech_marker_is_current(comfy);
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
    let (muscriptor_executable, muscriptor_model, muscriptor_marker) =
        managed_muscriptor_paths(Path::new(&research.install_root));
    let muscriptor_ready = muscriptor_executable.is_file()
        && fs::metadata(&muscriptor_model)
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() == MUSCRIPTOR_MODEL_BYTES)
        && read_bounded_text(&muscriptor_marker, 256)
            .is_some_and(|value| value.trim() == MUSCRIPTOR_SETUP_REVISION);
    let muscriptor_partial = muscriptor_executable.is_file()
        || muscriptor_model.is_file()
        || muscriptor_marker.is_file();

    let strata_dir = strata_install_dir(research);
    let strata_root = strata_dir.as_path();
    let strata_installs = crate::strata::known_installs(&[strata_root.to_string_lossy().as_ref()]);
    let strata_ready = crate::strata::ready_install(&strata_installs);
    let system_memory_bytes = crate::strata::system_memory_bytes();
    let strata_blocker = strata_blocker(strata_gpu, system_memory_bytes);
    let strata_choices = strata_choices(strata_root, system_memory_bytes, strata_blocker.is_none());
    let strata_bytes = strata_choices
        .iter()
        .find(|choice| choice.recommended)
        .or_else(|| strata_choices.first())
        .map_or(0, |choice| choice.download_bytes);
    let strata_partial = strata_partial(strata_root);

    let ffmpeg = resolve_program(&research.ffmpeg_path, "ffmpeg.exe");
    let ffprobe = resolve_program(&research.ffprobe_path, "ffprobe.exe");
    let media_ready = ffmpeg.is_some() && ffprobe.is_some();
    let model_assets = setup_model_assets(research);
    let missing_model_bytes = |component: &str| {
        model_assets
            .iter()
            .filter(|asset| asset.component == component && !asset.recognized)
            .map(|asset| asset.bytes)
            .sum::<u64>()
    };
    let missing_comfy_bytes = if comfy.join("main.py").is_file() {
        0
    } else {
        2_133_107_036
    };

    let components = vec![
        component(
            "assistant",
            "Included local model",
            (assistant_ready, assistant_partial),
            if assistant_ready {
                "Ready for chat, research planning, computer tasks, and movie direction."
            } else {
                "Needs the local llama.cpp engine, Ternary Bonsai 27B weights, and image projector."
            },
            &research.bonsai_root,
            (
                missing_model_bytes("assistant")
                    + if engine_ready {
                        0
                    } else if gpu.is_some() {
                        653_219_840
                    } else {
                        17_401_584
                    },
                false,
            ),
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
            (missing_model_bytes("studio") + missing_comfy_bytes, true),
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
            (missing_model_bytes("music") + missing_comfy_bytes, true),
        ),
        component(
            "image",
            "Ideogram 4 Image Studio",
            (image_ready, image_partial),
            if image_ready {
                "Ready for private non-commercial image design with structured layout and typography control."
            } else if comfy.join("main.py").is_file() && !image_nodes {
                "ComfyUI must be updated to 0.33.1 or newer before Kestrel can install the native Ideogram 4 workflow."
            } else {
                "Optional: about 16.4 GiB for the 12 GB NVIDIA profile. Ideogram's license permits non-commercial work only."
            },
            &research.comfy_root,
            (
                missing_model_bytes("image")
                    + missing_comfy_bytes
                    + if file_has_size(
                        &comfy
                            .join("models/diffusion_models/IDEOGRAM-4-NON-COMMERCIAL-LICENSE.txt"),
                        13_646,
                    ) {
                        0
                    } else {
                        13_646
                    },
                true,
            ),
        ),
        component(
            "speech",
            "Whisper dictation + local voice",
            (speech_ready, speech_partial),
            if speech_ready {
                "Ready for private Chatterbox narration and timestamped Whisper dictation across Kestrel."
            } else {
                "Optional: downloads the verified 1.6 GB Whisper large-v3-turbo checkpoint for dictation plus a local Chatterbox voice; no browser or system speech fallback."
            },
            &research.comfy_root,
            (
                missing_model_bytes("speech")
                    + missing_comfy_bytes
                    + if comfy
                        .join("custom_nodes/ComfyUI-Chatterbox/nodes.py")
                        .is_file()
                    {
                        0
                    } else {
                        267_765
                    },
                true,
            ),
        ),
        component(
            "muscriptor",
            "MuScriptor audio to MIDI",
            (muscriptor_ready, muscriptor_partial),
            if muscriptor_ready {
                "Ready for offline, GPU-accelerated transcription from a preserved music take to editable MIDI."
            } else {
                "Optional non-commercial extension. Setup prepares the official Windows GPU runner after you accept the separate terms and choose the gated large checkpoint."
            },
            muscriptor_executable.to_string_lossy().as_ref(),
            (3_500_000_000, true),
        ),
        component(
            "strata",
            "Qwen3.8-Flash-Next 125B via Strata",
            (strata_ready.is_some(), strata_partial),
            if strata_ready.is_some() {
                "Ready: a 125B mixture-of-experts assistant for chat, research, and computer tasks. Its experts live in system RAM, so loading takes a minute or more and uses most of this PC's memory."
            } else {
                "Optional larger assistant, tested on an NVIDIA RTX 5070 (12 GB VRAM) in a 64 GB PC: it uses 54-57 GB of RAM and nearly all the VRAM, and answers at about 40 tokens per second. Setup offers it only on a PC at least that capable. Works alongside the included model."
            },
            strata_ready
                .as_deref()
                .unwrap_or(strata_root)
                .to_string_lossy()
                .as_ref(),
            (strata_bytes, true),
        ),
    ];
    SetupSnapshot {
        ready: assistant_ready && wikipedia_ready,
        install_root: research.install_root.clone(),
        available_bytes: available_space_for(Path::new(&research.install_root)),
        gpu_name: gpu.map(|value| value.name.clone()),
        gpu_memory_bytes: gpu.map(|value| value.total_mib * 1024 * 1024).unwrap_or(0),
        system_memory_bytes,
        strata_choices,
        strata_blocker,
        components,
        model_assets,
    }
}

/// Every size Setup offers, with its remaining download and whether this PC can run it: a
/// supported graphics card and processor, and the tested RAM for that size. The best size that
/// fits is recommended.
fn strata_choices(
    strata_root: &Path,
    system_memory_bytes: u64,
    hardware_supported: bool,
) -> Vec<SetupStrataChoice> {
    let data = strata_data_dir(strata_root);
    let support = strata_support_bytes(strata_root);
    let mut recommended = false;
    STRATA_SIZES
        .into_iter()
        .map(|size| {
            let fits = hardware_supported && system_memory_bytes >= size.min_memory_gib * GIB;
            let missing = size
                .shards()
                .iter()
                .filter(|shard| !file_has_size(&data.join(&shard.relative), shard.download.bytes))
                .map(|shard| shard.download.bytes)
                .sum::<u64>();
            let is_recommended = fits && !recommended;
            recommended |= is_recommended;
            SetupStrataChoice {
                size: size.name.into(),
                download_bytes: missing + support,
                memory_bytes: size.pc_memory_gib * GIB,
                fits,
                recommended: is_recommended,
            }
        })
        .collect()
}

/// Bytes still to fetch besides the model shards: source, engine, draft layer, and packages.
fn strata_support_bytes(strata_root: &Path) -> u64 {
    if strata_root.join("engine").join("BUILD.json").is_file() {
        0
    } else {
        STRATA_SOURCE_BYTES + STRATA_ENGINE_BYTES + STRATA_PREPARED_BYTES
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
    // An older window may not send the Strata folder; keep the saved one then.
    let mut strata_root = locations.strata_root.trim().to_string();
    if !strata_root.is_empty() && !Path::new(&strata_root).is_absolute() {
        return Err(SetupError::InvalidPath(strata_root));
    }
    if strata_root.is_empty() {
        strata_root = research.strata_root.clone();
    }
    // The Strata folder starts as `<install root>\Strata`. While it is still that default and
    // nothing has been installed or downloaded there, it moves with the install root, so moving
    // large files to another drive moves Strata too. A folder the producer chose, or one that
    // already holds Strata files, stays where it is.
    let previous_default = Path::new(&research.install_root).join("Strata");
    if same_folder(Path::new(&strata_root), &previous_default)
        && folder_is_empty_or_missing(&previous_default)
        && !strata_partial(&previous_default)
    {
        strata_root = Path::new(&locations.install_root)
            .join("Strata")
            .to_string_lossy()
            .into_owned();
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
    research.strata_root = strata_root;
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
    validate_existing_model_paths(request)?;
    let root = PathBuf::from(request.install_root.trim());
    if !root.is_absolute() {
        return Err(SetupError::InvalidPath(request.install_root.clone()));
    }
    fs::create_dir_all(&root)?;
    settings.install_root = root.to_string_lossy().into_owned();
    match request.component.as_str() {
        "assistant" => install_assistant(app, settings, &root, request, cancel).await,
        "wikipedia" => {
            install_wikipedia(app, settings, &root, &request.wikipedia_edition, cancel).await
        }
        "media" => install_media(app, settings, &root, cancel).await,
        "studio" => install_studio(app, settings, &root, request, cancel).await,
        "music" => install_music(app, settings, &root, request, cancel).await,
        "image" => {
            install_image(
                app,
                settings,
                &root,
                request,
                request.accept_ideogram_non_commercial_license,
                cancel,
            )
            .await
        }
        "speech" => {
            install_speech(
                app,
                settings,
                &root,
                request,
                &request.whisper_checkpoint_path,
                cancel,
            )
            .await
        }
        "muscriptor" => {
            install_muscriptor(
                app,
                &root,
                request
                    .existing_model_paths
                    .get("muscriptor:model.safetensors")
                    .map(String::as_str)
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or(&request.muscriptor_checkpoint_path),
                request.accept_muscriptor_non_commercial_license,
                cancel,
            )
            .await
        }
        "strata" => install_strata(app, settings, request, cancel).await,
        other => Err(SetupError::Download {
            name: other.into(),
            details: "unknown setup component".into(),
        }),
    }
}

fn validate_existing_model_paths(request: &SetupInstallRequest) -> Result<(), SetupError> {
    if request.existing_model_paths.len() > 32 {
        return Err(SetupError::Dependency {
            name: "existing model selection".into(),
            details: "at most 32 supported model files can be selected for one setup run".into(),
        });
    }
    let mut ids = reusable_model_assets()
        .into_iter()
        .map(|asset| asset.id())
        .collect::<Vec<_>>();
    ids.push("muscriptor:model.safetensors".into());
    for (id, path) in &request.existing_model_paths {
        if !ids.contains(id) {
            return Err(SetupError::Dependency {
                name: "existing model selection".into(),
                details: format!("{id} is not a supported Setup model asset"),
            });
        }
        if path.chars().count() > 32_767 {
            return Err(SetupError::Dependency {
                name: "existing model selection".into(),
                details: format!("the selected path for {id} is too long"),
            });
        }
    }
    Ok(())
}

async fn install_assistant(
    app: &AppHandle,
    settings: &mut ResearchSettings,
    root: &Path,
    request: &SetupInstallRequest,
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
                "Local-model NVIDIA engine",
                &format!("https://github.com/PrismML-Eng/llama.cpp/releases/download/{BONSAI_RELEASE}/llama-prism-b1-9fcaed7-bin-win-cuda-12.4-x64.zip"),
                "llama-cuda.zip",
                261_776_213,
                "6d109e2930c0eaf2f729c3a6fc58dd7809ce2ba7047bfb294547cc389af6de5d",
            ),
            Asset::new(
                "Local-model NVIDIA support files",
                &format!("https://github.com/PrismML-Eng/llama.cpp/releases/download/{BONSAI_RELEASE}/cudart-llama-bin-win-cuda-12.4-x64.zip"),
                "llama-cudart.zip",
                391_443_627,
                "8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6",
            ),
        ]
    } else {
        vec![Asset::new(
            "Local-model CPU engine",
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
    for asset in reusable_model_assets()
        .into_iter()
        .filter(|asset| asset.component == "assistant")
    {
        install_or_reuse_model(app, request, &asset, &bonsai.join(&asset.relative), &cancel)
            .await?;
    }
    settings.bonsai_root = bonsai.to_string_lossy().into_owned();
    emit(
        app,
        "assistant",
        "complete",
        "The included local model is installed and verified.",
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
    request: &SetupInstallRequest,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    if crate::services::gpu_snapshot().await.is_none() {
        return Err(SetupError::Download {
            name: "MiniMax H3 Movie Studio".into(),
            details: "no NVIDIA GPU was detected. H3 is optional and is not practical on this computer; the rest of Kestrel can still be installed.".into(),
        });
    }
    let comfy =
        install_comfy_portable(app, root, "studio", ComfyRequirement::Base, &cancel).await?;
    for asset in reusable_model_assets()
        .into_iter()
        .filter(|asset| asset.component == "studio")
    {
        let destination = comfy.join(&asset.relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        install_or_reuse_model(app, request, &asset, &destination, &cancel).await?;
    }
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

#[derive(Clone, Copy)]
enum ComfyRequirement {
    Base,
    Music,
    Ideogram,
}

impl ComfyRequirement {
    fn available_in(self, comfy: &Path) -> bool {
        match self {
            Self::Base => true,
            Self::Music => comfy.join("comfy_extras/nodes_minimax_music.py").is_file(),
            Self::Ideogram => {
                comfy.join("comfy_extras/nodes_ideogram4.py").is_file()
                    && comfy.join("comfy_extras/nodes_custom_sampler.py").is_file()
            }
        }
    }

    fn missing_detail(self) -> &'static str {
        match self {
            Self::Base => "the installed ComfyUI is incomplete",
            Self::Music => "the installed ComfyUI does not contain native MiniMax Music 3 nodes; remove the stale Kestrel-owned portable folder or update it to 0.33.0+, then resume",
            Self::Ideogram => "the installed ComfyUI does not contain native Ideogram 4 nodes; remove the stale Kestrel-owned portable folder or update it to 0.33.1+, then resume",
        }
    }
}

async fn install_comfy_portable(
    app: &AppHandle,
    root: &Path,
    component: &str,
    requirement: ComfyRequirement,
    cancel: &CancellationToken,
) -> Result<PathBuf, SetupError> {
    let portable = Asset::new(
        "ComfyUI portable",
        &format!("https://github.com/Comfy-Org/ComfyUI/releases/download/{COMFY_VERSION}/ComfyUI_windows_portable_nvidia.7z"),
        "ComfyUI_windows_portable_nvidia-v0.33.1.7z",
        2_133_107_036,
        "4a221588979b96b8244e0e50b2edca03af732acae1deba69d60aa3b4d60b9dba",
    );
    let portable_root = root.join("ComfyUI_windows_portable");
    let comfy = portable_root.join("ComfyUI");
    let needs_extraction = !comfy.join("main.py").is_file();
    let needs_replacement = !needs_extraction
        && !requirement.available_in(&comfy)
        && is_kestrel_managed_comfy_root(&comfy, root);
    if needs_extraction || needs_replacement {
        let downloads = root.join("downloads");
        fs::create_dir_all(&downloads)?;
        let archive = downloads.join(&portable.file_name);
        download(app, component, &portable, &archive, cancel).await?;
        if needs_extraction {
            extract_7z(&archive, root, &portable.name).await?;
            if !comfy.join("main.py").is_file() {
                return Err(SetupError::Extract {
                    name: portable.name,
                    details: "the verified portable archive did not contain ComfyUI/main.py".into(),
                });
            }
            mark_kestrel_managed_comfy_root(&comfy)?;
        } else {
            replace_managed_comfy_portable(&archive, root, &portable.name, requirement).await?;
        }
    }
    if !requirement.available_in(&comfy) {
        return Err(SetupError::Extract {
            name: portable.name,
            details: requirement.missing_detail().into(),
        });
    }
    Ok(comfy)
}

async fn install_music(
    app: &AppHandle,
    settings: &mut ResearchSettings,
    root: &Path,
    request: &SetupInstallRequest,
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
                install_comfy_portable(app, root, "music", ComfyRequirement::Music, &cancel).await?
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
        install_comfy_portable(app, root, "music", ComfyRequirement::Music, &cancel).await?
    };
    for asset in reusable_model_assets()
        .into_iter()
        .filter(|asset| asset.component == "music")
    {
        let destination = comfy.join(&asset.relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        install_or_reuse_model(app, request, &asset, &destination, &cancel).await?;
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

async fn install_image(
    app: &AppHandle,
    settings: &mut ResearchSettings,
    root: &Path,
    request: &SetupInstallRequest,
    accepted_license: bool,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    if !accepted_license {
        return Err(SetupError::Dependency {
            name: "Ideogram 4 Image Studio".into(),
            details: "Ideogram 4 is available only under Ideogram's Non-Commercial Model Agreement. Read and explicitly accept that agreement in Setup before downloading the model.".into(),
        });
    }
    if crate::services::gpu_snapshot().await.is_none() {
        return Err(SetupError::Download {
            name: "Ideogram 4 Image Studio".into(),
            details: "no NVIDIA GPU was detected. Keep using the rest of Kestrel, or configure a supported local ComfyUI computer before installing image generation.".into(),
        });
    }
    let configured = PathBuf::from(settings.comfy_root.trim());
    let comfy = if configured.join("main.py").is_file() {
        if !ComfyRequirement::Ideogram.available_in(&configured) {
            if is_kestrel_managed_comfy_root(&configured, root) {
                install_comfy_portable(app, root, "image", ComfyRequirement::Ideogram, &cancel)
                    .await?
            } else {
                return Err(SetupError::Download {
                    name: "Ideogram 4 Image Studio".into(),
                    details: format!(
                        "{} is older than ComfyUI 0.33.1. Update this producer-managed installation first; Kestrel will not overwrite it.",
                        configured.display()
                    ),
                });
            }
        } else {
            configured
        }
    } else {
        install_comfy_portable(app, root, "image", ComfyRequirement::Ideogram, &cancel).await?
    };
    for asset in reusable_model_assets()
        .into_iter()
        .filter(|asset| asset.component == "image")
    {
        let destination = comfy.join(&asset.relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        install_or_reuse_model(app, request, &asset, &destination, &cancel).await?;
    }
    for asset in ideogram_assets()
        .into_iter()
        .filter(|asset| !asset.relative.ends_with(".safetensors"))
    {
        let destination = comfy.join(asset.relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        download(app, "image", &asset.download, &destination, &cancel).await?;
    }
    ensure_comfy_launcher(&comfy)?;
    settings.comfy_root = comfy.to_string_lossy().into_owned();
    emit(
        app,
        "image",
        "complete",
        "Ideogram 4 is installed and verified for offline non-commercial image production.",
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
    request: &SetupInstallRequest,
    whisper_checkpoint_path: &str,
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
        install_comfy_portable(app, root, "speech", ComfyRequirement::Base, &cancel).await?
    };
    let kestrel_managed = is_kestrel_managed_comfy_root(&comfy, root);
    ensure_speech_python(app, &comfy, kestrel_managed, &cancel).await?;
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
    for asset in reusable_model_assets()
        .into_iter()
        .filter(|asset| asset.component == "speech")
    {
        let destination = comfy.join(&asset.relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        if asset.relative == "models/stt/whisper/large-v3-turbo.pt"
            && !whisper_checkpoint_path.trim().is_empty()
            && !request.existing_model_paths.contains_key(&asset.id())
            && !verified(&destination, &asset.download).await?
        {
            import_verified_asset(
                app,
                "speech",
                Path::new(whisper_checkpoint_path.trim()),
                &destination,
                &asset.download,
            )
            .await?;
        } else {
            install_or_reuse_model(app, request, &asset, &destination, &cancel).await?;
        }
    }
    ensure_comfy_launcher(&comfy)?;
    fs::write(
        comfy.join("custom_nodes/Kestrel-Whisper/.kestrel-speech-ready"),
        speech_marker_contents(),
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

pub(crate) fn managed_muscriptor_paths(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let muscriptor = root.join("MuScriptor");
    (
        muscriptor.join("runtime/uvx.exe"),
        muscriptor.join("models/model.safetensors"),
        muscriptor.join(".kestrel-muscriptor-ready"),
    )
}

async fn install_muscriptor(
    app: &AppHandle,
    root: &Path,
    checkpoint_path: &str,
    accepted_license: bool,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    if !accepted_license {
        return Err(SetupError::Dependency {
            name: "MuScriptor audio to MIDI".into(),
            details: "MuScriptor weights use separate gated CC BY-NC 4.0 terms. Accept those terms on the official model page and confirm them in Setup before preparing this extension.".into(),
        });
    }
    if crate::services::gpu_snapshot().await.is_none() {
        return Err(SetupError::Dependency {
            name: "MuScriptor audio to MIDI".into(),
            details: "no NVIDIA GPU was detected. The official Windows GPU profile cannot be prepared on this computer.".into(),
        });
    }
    let source = PathBuf::from(checkpoint_path.trim());
    validate_muscriptor_checkpoint(&source)?;

    let muscriptor = root.join("MuScriptor");
    let runtime = muscriptor.join("runtime");
    let downloads = muscriptor.join("downloads");
    let (uvx, model, marker) = managed_muscriptor_paths(root);
    fs::create_dir_all(&downloads)?;
    fs::create_dir_all(model.parent().expect("fixed MuScriptor model parent"))?;
    let uv = uv_runner_asset("MuScriptor isolated Python runner");
    let archive = downloads.join(&uv.file_name);
    download(app, "muscriptor", &uv, &archive, &cancel).await?;
    unzip(&archive, &runtime, &uv.name)?;
    if !uvx.is_file() {
        return Err(SetupError::Extract {
            name: uv.name,
            details: "the verified Windows archive did not contain uvx.exe".into(),
        });
    }

    emit(
        app,
        "muscriptor",
        "importing",
        "Copying the producer-approved MuScriptor large checkpoint into the offline extension…",
        0,
        MUSCRIPTOR_MODEL_BYTES,
        0,
    );
    let temporary = model.with_extension("safetensors.importing");
    if temporary.is_file() {
        fs::remove_file(&temporary)?;
    }
    if cancel.is_cancelled() {
        return Err(SetupError::Cancelled);
    }
    if fs::hard_link(&source, &temporary).is_err() {
        tokio::select! {
            result = tokio::fs::copy(&source, &temporary) => {
                result?;
            }
            _ = cancel.cancelled() => {
                let _ = tokio::fs::remove_file(&temporary).await;
                return Err(SetupError::Cancelled);
            }
        }
    }
    validate_muscriptor_checkpoint(&temporary)?;
    let hash_path = temporary.clone();
    let model_sha256 = tokio::task::spawn_blocking(move || sha256_file(&hash_path))
        .await
        .map_err(|error| SetupError::Integrity {
            name: "MuScriptor large checkpoint".into(),
            details: error.to_string(),
        })??;
    let backup = model.with_extension("safetensors.kestrel-backup");
    if backup.is_file() {
        fs::remove_file(&backup)?;
    }
    if model.is_file() {
        fs::rename(&model, &backup)?;
    }
    if let Err(error) = fs::rename(&temporary, &model) {
        if backup.is_file() {
            let _ = fs::rename(&backup, &model);
        }
        return Err(error.into());
    }
    if backup.is_file() {
        fs::remove_file(backup)?;
    }

    emit(
        app,
        "muscriptor",
        "dependencies",
        "Preparing the pinned official MuScriptor package and CUDA runtime for later offline use…",
        0,
        0,
        0,
    );
    run_muscriptor_probe(&uvx, &muscriptor, false, &cancel).await?;
    run_muscriptor_probe(&uvx, &muscriptor, true, &cancel).await?;
    fs::write(&marker, format!("{MUSCRIPTOR_SETUP_REVISION}\n"))?;
    fs::write(
        muscriptor.join("install.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schemaVersion": 1,
            "revision": MUSCRIPTOR_SETUP_REVISION,
            "package": MUSCRIPTOR_PACKAGE,
            "runner": "uv 0.11.30 x86_64-pc-windows-msvc",
            "model": "MuScriptor/muscriptor-large",
            "modelBytes": MUSCRIPTOR_MODEL_BYTES,
            "modelSha256": model_sha256,
            "license": "CC-BY-NC-4.0 plus the producer-accepted gated model conditions",
        }))?,
    )?;
    emit(
        app,
        "muscriptor",
        "complete",
        "MuScriptor large and its isolated NVIDIA runtime are installed and verified for offline use.",
        1,
        1,
        0,
    );
    Ok(())
}

/// Astral's pinned uv, used to provide isolated, checksum-verified Python builds.
fn uv_runner_asset(name: &str) -> Asset {
    Asset::new(
        name,
        "https://github.com/astral-sh/uv/releases/download/0.11.30/uv-x86_64-pc-windows-msvc.zip",
        "uv-x86_64-pc-windows-msvc-0.11.30.zip",
        25_710_044,
        "be8d78c992312212e5cc05e9f9de3fa996db73b7c86a186dfb9231eb9f91d33e",
    )
}

/// Install Strata and one Qwen3.8-Flash-Next size into the saved Strata folder.
///
/// Kestrel first confirms this PC is at least as capable as the one it tested Strata on, so no
/// download starts that Strata could not use. It then downloads and verifies the pinned source,
/// engine, and both model shards itself, gives Strata an isolated Python 3.12 from the pinned uv
/// runner, and runs Strata's own setup non-interactively to prepare its model pack and draft
/// layer. Every step is resumable: finished files are verified and kept, and Strata's setup skips
/// what it already prepared.
async fn install_strata(
    app: &AppHandle,
    settings: &mut ResearchSettings,
    request: &SetupInstallRequest,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    let size = StrataSize::parse(&request.strata_size)?;
    let gpu = crate::strata::probe_gpu().await;
    if let Some(reason) =
        crate::strata::unsupported_reason(gpu.as_ref(), crate::strata::cpu_has_avx2())
    {
        return Err(SetupError::Dependency {
            name: "Qwen3.8-Flash-Next".into(),
            details: reason,
        });
    }
    let memory = crate::strata::system_memory_bytes();
    if memory < size.min_memory_gib * GIB {
        return Err(SetupError::Dependency {
            name: format!("Qwen3.8-Flash-Next {}", size.name),
            details: format!(
                "Kestrel tested this size on a {} GB PC, where Strata uses 54-57 GB of RAM, but this PC reports {:.1} GB.",
                size.pc_memory_gib,
                memory as f64 / GIB as f64
            ),
        });
    }
    let strata = strata_install_dir(settings);
    ensure_kestrel_strata_folder(&strata)?;
    let data = strata_data_dir(&strata);
    let downloads = data.join("downloads");
    let prebuilt = data.join("prebuilt");
    let models = data.join("models").join(size.name);
    for folder in [&strata, &downloads, &prebuilt, &models] {
        fs::create_dir_all(folder)?;
    }

    let source = Asset::new(
        &format!("Strata {STRATA_RELEASE} source"),
        &format!("https://github.com/Niko1221/Strata/archive/{STRATA_COMMIT}.zip"),
        &format!("strata-{STRATA_COMMIT}.zip"),
        STRATA_SOURCE_BYTES,
        STRATA_SOURCE_SHA256,
    );
    let source_archive = downloads.join(&source.file_name);
    download(app, "strata", &source, &source_archive, &cancel).await?;
    if read_bounded_text(&strata.join(STRATA_SETUP_MARKER), 256)
        .is_none_or(|revision| revision.trim() != STRATA_COMMIT)
    {
        emit(app, "strata", "extracting", "Unpacking Strata…", 0, 0, 0);
        install_strata_source(&source_archive, &strata, &source.name)?;
    }

    let engine = Asset::new(
        &format!("Strata {STRATA_RELEASE} NVIDIA engine"),
        &format!(
            "https://github.com/Niko1221/Strata/releases/download/{STRATA_RELEASE}/strata-windows-x64.zip"
        ),
        "strata-windows-x64.zip",
        STRATA_ENGINE_BYTES,
        STRATA_ENGINE_SHA256,
    );
    download(
        app,
        "strata",
        &engine,
        &prebuilt.join(&engine.file_name),
        &cancel,
    )
    .await?;

    for shard in size.shards() {
        let destination = data.join(&shard.relative);
        let explicit = request
            .existing_model_paths
            .get(&shard.id())
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        // Shard 2 is the same file for every size: reuse a verified copy from another size.
        let sibling = STRATA_SIZES
            .into_iter()
            .filter(|other| *other != size)
            .flat_map(StrataSize::shards)
            .find(|other| other.download.sha256 == shard.download.sha256)
            .map(|other| data.join(other.relative))
            .filter(|path| file_has_size(path, shard.download.bytes));
        if explicit.is_none() && !file_has_size(&destination, shard.download.bytes) {
            if let Some(sibling) = sibling {
                import_verified_asset(app, "strata", &sibling, &destination, &shard.download)
                    .await?;
                continue;
            }
        }
        install_or_reuse_model(app, request, &shard, &destination, &cancel).await?;
    }

    let python = ensure_strata_python(app, &data, &strata, &cancel).await?;
    run_strata_setup(
        app, &python, &strata, &data, &models, &prebuilt, size, &cancel,
    )
    .await?;

    let config = strata.join(format!("strata-{}.json", size.name.to_ascii_lowercase()));
    crate::strata::launch_plan(&config).map_err(|details| SetupError::Dependency {
        name: "Strata setup".into(),
        details: format!("Strata finished, but its run configuration is not usable: {details}"),
    })?;
    settings.strata_root = strata.to_string_lossy().into_owned();
    emit(
        app,
        "strata",
        "complete",
        &format!(
            "Qwen3.8-Flash-Next {} is installed through Strata and appears in your local models.",
            size.name
        ),
        1,
        1,
        0,
    );
    Ok(())
}

/// Setup installs Strata only into a folder it owns: missing, empty, or marked by an earlier
/// Kestrel install. A Strata that its own setup installed is used as it is, never overwritten.
fn ensure_kestrel_strata_folder(strata: &Path) -> Result<(), SetupError> {
    if strata.join(STRATA_SETUP_MARKER).is_file() || folder_is_empty_or_missing(strata) {
        return Ok(());
    }
    let (contents, remedy) = if strata.join("setup.py").is_file() {
        (
            "a Strata that its own setup installed",
            ", or run that folder's START-HERE.bat to add this size; Kestrel lists its models once they are ready",
        )
    } else {
        ("other files", "")
    };
    Err(SetupError::Dependency {
        name: "Strata".into(),
        details: format!(
            "{} already holds {contents}, and Kestrel never overwrites files it did not install. Choose an empty Strata folder in Setup's locations{remedy}.",
            strata.display()
        ),
    })
}

/// Replace Strata's source files from the verified archive while keeping what its setup created
/// beside them (`.venv`, `engine`, run configurations, logs).
fn install_strata_source(archive: &Path, strata: &Path, name: &str) -> Result<(), SetupError> {
    let staging = strata.with_extension("kestrel-unpacking");
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    unzip(archive, &staging, name)?;
    let unpacked = staging.join(format!("Strata-{STRATA_COMMIT}"));
    if !unpacked.join("setup.py").is_file() || !unpacked.join("serve").join("server.py").is_file() {
        let _ = fs::remove_dir_all(&staging);
        return Err(SetupError::Extract {
            name: name.into(),
            details: "the verified archive did not contain Strata's setup.py and server".into(),
        });
    }
    // Mark the folder as Kestrel's before the first file lands, so an interrupted copy resumes
    // instead of looking like someone else's install. Only the finished copy records the commit.
    fs::write(strata.join(STRATA_SETUP_MARKER), "unpacking\n")?;
    copy_tree(&unpacked, strata)?;
    fs::remove_dir_all(&staging)?;
    fs::write(
        strata.join(STRATA_SETUP_MARKER),
        format!("{STRATA_COMMIT}\n"),
    )?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), SetupError> {
    for entry in walkdir::WalkDir::new(from).follow_links(false) {
        let entry = entry.map_err(|error| SetupError::Io(std::io::Error::other(error)))?;
        let relative = entry
            .path()
            .strip_prefix(from)
            .map_err(|error| SetupError::Io(std::io::Error::other(error)))?;
        let target = to.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// A private Python 3.12 environment inside the Strata folder, created by the pinned uv runner
/// from its checksum-verified Python builds. Nothing is installed for the Windows user.
async fn ensure_strata_python(
    app: &AppHandle,
    data: &Path,
    strata: &Path,
    cancel: &CancellationToken,
) -> Result<PathBuf, SetupError> {
    let python = strata.join(".venv").join("Scripts").join("python.exe");
    if python.is_file() {
        return Ok(python);
    }
    let runtime = data.join("runtime");
    let uv = uv_runner_asset("Strata isolated Python runner");
    let archive = data.join("downloads").join(&uv.file_name);
    download(app, "strata", &uv, &archive, cancel).await?;
    unzip(&archive, &runtime, &uv.name)?;
    let uv_exe =
        first_matching_file(&runtime, "uv.exe", true).ok_or_else(|| SetupError::Extract {
            name: uv.name.clone(),
            details: "the verified Windows archive did not contain uv.exe".into(),
        })?;
    emit(
        app,
        "strata",
        "dependencies",
        "Preparing Strata's private Python 3.12…",
        0,
        0,
        0,
    );
    let mut command = tokio::process::Command::new(&uv_exe);
    command
        .args([
            "venv",
            "--python",
            "3.12",
            "--python-preference",
            "only-managed",
            "--seed",
        ])
        .arg(strata.join(".venv"))
        .current_dir(strata)
        .env("UV_CACHE_DIR", data.join("cache"))
        .env("UV_PYTHON_INSTALL_DIR", data.join("python"))
        .env("UV_NO_PROGRESS", "1")
        .env("PYTHONUTF8", "1")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::select! {
        result = command.output() => result?,
        _ = cancel.cancelled() => return Err(SetupError::Cancelled),
    };
    if !output.status.success() || !python.is_file() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SetupError::Dependency {
            name: "Strata Python environment".into(),
            details: bounded_setup_detail(&stderr),
        });
    }
    Ok(python)
}

/// Run Strata's setup with fixed arguments. Its progress lines become Setup progress; the whole
/// process tree (pip, the draft-layer fetch) is owned by one job and stops on Pause.
#[allow(clippy::too_many_arguments)]
async fn run_strata_setup(
    app: &AppHandle,
    python: &Path,
    strata: &Path,
    data: &Path,
    models: &Path,
    prebuilt: &Path,
    size: StrataSize,
    cancel: &CancellationToken,
) -> Result<(), SetupError> {
    use tokio::io::AsyncReadExt;
    emit(
        app,
        "strata",
        "preparing",
        "Strata is preparing its model pack and speculative-decoding draft layer (about 5 GB)…",
        0,
        0,
        0,
    );
    let mut command = tokio::process::Command::new(python);
    command
        .arg("-u")
        .arg(strata.join("setup.py"))
        .args([
            "--family",
            "qwen",
            "--model",
            size.name,
            "--context",
            &STRATA_CONTEXT.to_string(),
            "--kv",
            "int8",
            "--vision",
            "no",
            "--experimental-speed-projection",
            "off",
            "--yes",
            "--no-start",
        ])
        .arg("--data-dir")
        .arg(data)
        .arg("--gguf-dir")
        .arg(models)
        .arg("--prebuilt")
        .arg(prebuilt)
        .current_dir(strata)
        .env("PYTHONUNBUFFERED", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Setup joins its job before it runs, so pip and the draft-layer fetch cannot escape Pause.
    #[cfg(windows)]
    let (mut child, _job) =
        crate::strata::ProcessJob::spawn(&mut command).map_err(|error| SetupError::Dependency {
            name: "Strata setup".into(),
            details: format!(
                "Kestrel could not start Strata's setup inside its own process job, so nothing was left running: {error}"
            ),
        })?;
    #[cfg(not(windows))]
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take().expect("piped Strata stdout");
    let stderr = child.stderr.take().expect("piped Strata stderr");
    // Strata explains its own failures on stdout; stderr adds pip output and tracebacks, whose
    // last lines are the useful ones.
    let stderr_task = tokio::spawn(read_tail(stderr, 2 * 1024));
    let mut recent = std::collections::VecDeque::with_capacity(24);
    let mut pending = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    loop {
        let count = tokio::select! {
            result = stdout.read(&mut buffer) => result?,
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(SetupError::Cancelled);
            }
        };
        if count == 0 {
            break;
        }
        pending.extend_from_slice(&buffer[..count]);
        // Strata redraws progress with carriage returns; treat them as line ends.
        while let Some(end) = pending
            .iter()
            .position(|byte| *byte == b'\n' || *byte == b'\r')
        {
            let line = String::from_utf8_lossy(&pending[..end]).trim().to_string();
            pending.drain(..=end);
            if line.is_empty() {
                continue;
            }
            if recent.len() == 24 {
                recent.pop_front();
            }
            recent.push_back(line.clone());
            if line.starts_with("===") || last_emit.elapsed() >= Duration::from_millis(400) {
                emit(
                    app,
                    "strata",
                    "preparing",
                    &format!("Strata: {}", bounded_setup_detail(&line)),
                    0,
                    0,
                    0,
                );
                last_emit = Instant::now();
            }
        }
    }
    let status = tokio::select! {
        result = child.wait() => result?,
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err(SetupError::Cancelled);
        }
    };
    let errors = stderr_task.await.unwrap_or_default();
    if !status.success() {
        let tail = recent.into_iter().collect::<Vec<_>>().join("\n");
        return Err(SetupError::Dependency {
            name: "Strata setup".into(),
            details: bounded_setup_detail(&format!("{tail}\n{errors}")),
        });
    }
    Ok(())
}

/// Read a child's output until it closes, keeping only the last `limit` bytes. Reading to the end
/// keeps the pipe open for the whole run, so the child never writes into a closed pipe.
async fn read_tail<R: tokio::io::AsyncRead + Unpin>(mut reader: R, limit: usize) -> String {
    use tokio::io::AsyncReadExt;
    let mut tail = Vec::new();
    let mut buffer = [0u8; 8192];
    while let Ok(count) = reader.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        tail.extend_from_slice(&buffer[..count]);
        // Trim in batches so a long run does not shift the buffer on every read.
        if tail.len() > limit.saturating_mul(2) {
            tail.drain(..tail.len() - limit);
        }
    }
    if tail.len() > limit {
        tail.drain(..tail.len() - limit);
    }
    String::from_utf8_lossy(&tail).into_owned()
}

async fn run_muscriptor_probe(
    uvx: &Path,
    root: &Path,
    offline: bool,
    cancel: &CancellationToken,
) -> Result<(), SetupError> {
    let mut command = tokio::process::Command::new(uvx);
    if offline {
        command.arg("--offline");
    }
    command.args([
        "--python",
        "3.12",
        "--torch-backend",
        "cu128",
        "--from",
        MUSCRIPTOR_PACKAGE,
        "muscriptor",
        "--help",
    ]);
    command
        .current_dir(root)
        .env("UV_CACHE_DIR", root.join("cache"))
        .env("UV_PYTHON_INSTALL_DIR", root.join("python"))
        .env("UV_NO_PROGRESS", "1")
        .env("UV_LINK_MODE", "copy")
        .env("PYTHONUTF8", "1")
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
            name: "MuScriptor isolated runtime".into(),
            details: bounded_setup_detail(if stderr.trim().is_empty() {
                stdout.as_ref()
            } else {
                stderr.as_ref()
            }),
        });
    }
    Ok(())
}

fn validate_muscriptor_checkpoint(path: &Path) -> Result<(), SetupError> {
    let metadata = fs::metadata(path).map_err(|_| SetupError::Dependency {
        name: "MuScriptor large checkpoint".into(),
        details: format!(
            "choose the completed model.safetensors file downloaded from the official MuScriptor/muscriptor-large page; {} is unavailable",
            path.display()
        ),
    })?;
    if !metadata.is_file() || metadata.len() != MUSCRIPTOR_MODEL_BYTES {
        return Err(SetupError::Integrity {
            name: "MuScriptor large checkpoint".into(),
            details: format!(
                "expected the official {MUSCRIPTOR_MODEL_BYTES}-byte model.safetensors file, but {} contains {} bytes. Wait for the browser download to finish before choosing it.",
                path.display(),
                metadata.len()
            ),
        });
    }
    let mut file = fs::File::open(path)?;
    let mut length = [0_u8; 8];
    file.read_exact(&mut length)?;
    let header_bytes = u64::from_le_bytes(length);
    if header_bytes == 0 || header_bytes > 16 * 1024 * 1024 {
        return Err(SetupError::Integrity {
            name: "MuScriptor large checkpoint".into(),
            details: "the selected file does not contain a bounded safetensors header".into(),
        });
    }
    let mut header = vec![0_u8; header_bytes as usize];
    file.read_exact(&mut header)?;
    let value: serde_json::Value =
        serde_json::from_slice(&header).map_err(|error| SetupError::Integrity {
            name: "MuScriptor large checkpoint".into(),
            details: format!("the safetensors header is invalid: {error}"),
        })?;
    let tensor_found = value.as_object().is_some_and(|entries| {
        entries.iter().any(|(name, value)| {
            name != "__metadata__"
                && value
                    .get("dtype")
                    .and_then(serde_json::Value::as_str)
                    .is_some()
                && value
                    .get("shape")
                    .and_then(serde_json::Value::as_array)
                    .is_some()
                && value
                    .get("data_offsets")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|offsets| offsets.len() == 2)
        })
    });
    if !tensor_found {
        return Err(SetupError::Integrity {
            name: "MuScriptor large checkpoint".into(),
            details: "the selected safetensors file contains no tensor entries".into(),
        });
    }
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
    kestrel_managed: bool,
    cancel: &CancellationToken,
) -> Result<(), SetupError> {
    let python = comfy_python(comfy).ok_or_else(|| SetupError::Dependency {
        name: "local speech Python runtime".into(),
        details: format!(
            "ComfyUI's private Python runtime is missing beside {}. Resume Movie Studio, Music Production, or Whisper dictation + local voice from Setup.",
            comfy.display()
        ),
    })?;
    if speech_python_ready(&python).await {
        return Ok(());
    }
    if !kestrel_managed {
        return Err(SetupError::Dependency {
            name: "local speech Python packages".into(),
            details: format!(
                "{} is producer-managed, so Kestrel did not change its Python environment. Install these exact packages with `{}` -m pip install {}, then choose Resume: {}",
                comfy.display(),
                python.display(),
                SPEECH_PYTHON_PACKAGES.join(" "),
                SPEECH_PYTHON_PACKAGES.join(", ")
            ),
        });
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
    ]);
    command.args(SPEECH_PYTHON_PACKAGES);
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
    let portable_marker = read_bounded_text(&comfy.join(KESTREL_MANAGED_COMFY_MARKER), 256)
        .is_some_and(|value| value == KESTREL_MANAGED_COMFY_MARKER_CONTENT);
    expected_location && (launcher_managed || portable_marker)
}

fn mark_kestrel_managed_comfy_root(comfy: &Path) -> Result<(), SetupError> {
    fs::write(
        comfy.join(KESTREL_MANAGED_COMFY_MARKER),
        KESTREL_MANAGED_COMFY_MARKER_CONTENT,
    )?;
    Ok(())
}

fn speech_marker_contents() -> String {
    format!(
        "{KESTREL_WHISPER_ADAPTER_REVISION}\nchatterbox-node={CHATTERBOX_NODE_REVISION}\nchatterbox-model={CHATTERBOX_MODEL_REVISION}\n"
    )
}

fn speech_marker_is_current(comfy: &Path) -> bool {
    let expected = speech_marker_contents();
    read_bounded_text(
        &comfy.join("custom_nodes/Kestrel-Whisper/.kestrel-speech-ready"),
        1_024,
    )
    .is_some_and(|value| value == expected)
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
    requirement: ComfyRequirement,
) -> Result<(), SetupError> {
    let nonce = uuid::Uuid::new_v4();
    let staging = install_root.join(format!(".kestrel-comfy-update-{nonce}"));
    let staged_portable = staging.join("ComfyUI_windows_portable");
    let portable = install_root.join("ComfyUI_windows_portable");
    let backup = install_root.join(format!("ComfyUI_windows_portable.kestrel-backup-{nonce}"));
    fs::create_dir_all(&staging)?;
    extract_7z(archive, &staging, name).await?;
    let staged_comfy = staged_portable.join("ComfyUI");
    if !staged_comfy.join("main.py").is_file() || !requirement.available_in(&staged_comfy) {
        return Err(SetupError::Extract {
            name: name.into(),
            details: format!(
                "the replacement was unpacked to {}, but {}; the existing installation was not changed",
                staging.display(),
                requirement.missing_detail()
            ),
        });
    }
    mark_kestrel_managed_comfy_root(&staged_comfy)?;
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
    if backup.exists() {
        fs::remove_dir_all(&backup)?;
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

struct IdeogramAsset {
    relative: &'static str,
    download: Asset,
}

fn ideogram_assets() -> Vec<IdeogramAsset> {
    vec![
        IdeogramAsset {
            relative: "models/diffusion_models/ideogram4_nvfp4_mixed.safetensors",
            download: Asset::new(
                "Ideogram 4 conditional NVFP4 model",
                &format!(
                    "https://huggingface.co/Comfy-Org/Ideogram-4/resolve/{IDEOGRAM_REVISION}/diffusion_models/ideogram4_nvfp4_mixed.safetensors"
                ),
                "ideogram4_nvfp4_mixed.safetensors",
                5_490_550_037,
                "e7923b4b0a1129ae5afcc09e63046185688c8b09eb9a1a748cccdbde5d381609",
            ),
        },
        IdeogramAsset {
            relative: "models/diffusion_models/ideogram4_unconditional_nvfp4_mixed.safetensors",
            download: Asset::new(
                "Ideogram 4 unconditional NVFP4 model",
                &format!(
                    "https://huggingface.co/Comfy-Org/Ideogram-4/resolve/{IDEOGRAM_REVISION}/diffusion_models/ideogram4_unconditional_nvfp4_mixed.safetensors"
                ),
                "ideogram4_unconditional_nvfp4_mixed.safetensors",
                5_490_550_037,
                "639e37bd1dd7ee35e23c7cfccf93a518ddc7f4587818956ec42b31e659fd6ac0",
            ),
        },
        IdeogramAsset {
            relative: "models/text_encoders/qwen3vl_8b_nvfp4.safetensors",
            download: Asset::new(
                "Ideogram 4 Qwen3-VL text encoder",
                &format!(
                    "https://huggingface.co/Comfy-Org/Qwen3-VL/resolve/{QWEN3_VL_REVISION}/text_encoders/qwen3vl_8b_nvfp4.safetensors"
                ),
                "qwen3vl_8b_nvfp4.safetensors",
                6_305_221_764,
                "e462e9e0c3b9313ae17f82040d7c77beb92d7aef3e40692d7803228dab7c3b98",
            ),
        },
        IdeogramAsset {
            relative: "models/vae/flux2-vae.safetensors",
            download: Asset::new(
                "Ideogram 4 Flux 2 decoder",
                &format!(
                    "https://huggingface.co/Comfy-Org/flux2-dev/resolve/{FLUX2_REVISION}/split_files/vae/flux2-vae.safetensors"
                ),
                "flux2-vae.safetensors",
                336_213_556,
                "d64f3a68e1cc4f9f4e29b6e0da38a0204fe9a49f2d4053f0ec1fa1ca02f9c4b5",
            ),
        },
        IdeogramAsset {
            relative: "models/diffusion_models/IDEOGRAM-4-NON-COMMERCIAL-LICENSE.txt",
            download: Asset::new(
                "Ideogram 4 Non-Commercial Model Agreement",
                &format!(
                    "https://raw.githubusercontent.com/ideogram-oss/ideogram4/{IDEOGRAM_LICENSE_REVISION}/model_licenses/LICENSE-IDEOGRAM-4-NON-COMMERCIAL"
                ),
                "IDEOGRAM-4-NON-COMMERCIAL-LICENSE.txt",
                13_646,
                "8e631193e8cab3632f93fc4e72689f6e41fb6e2e1b9fab5ab8cb17b5909bc8b2",
            ),
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
    file_has_size(&source, KJ_PREVIEW_NODE_BYTES)
        && file_has_size(&tiny_vae, KJ_TINY_VAE_BYTES)
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
        && file_has_size(
            &root.join("preview_override_node.py"),
            KJ_PREVIEW_NODE_BYTES,
        )
        && file_has_size(&root.join("tiny_vae.py"), KJ_TINY_VAE_BYTES)
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
                KJ_PREVIEW_NODE_BYTES,
                "6060c2382e61c041104a61c1fe06f2a163fcddc6e715d20fbf7a03f0af46715c",
            ),
            (
                "MiniMax tiny-VAE loader",
                "nodes/tiny_vae.py",
                "tiny_vae.py",
                KJ_TINY_VAE_BYTES,
                "f09d1e3ab1cb0f2ee4949f4192b7ea1bb47390c47b80f8517532301283a3472d",
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
            KJ_PREVIEW_NODE_BYTES,
            "6060c2382e61c041104a61c1fe06f2a163fcddc6e715d20fbf7a03f0af46715c",
        ),
        (
            "MiniMax tiny-VAE loader",
            "nodes/tiny_vae.py",
            "tiny_vae.py",
            KJ_TINY_VAE_BYTES,
            "f09d1e3ab1cb0f2ee4949f4192b7ea1bb47390c47b80f8517532301283a3472d",
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

#[derive(Clone)]
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

struct ReusableModelAsset {
    component: &'static str,
    relative: String,
    download: Asset,
}

impl ReusableModelAsset {
    fn id(&self) -> String {
        format!(
            "{}:{}",
            self.component,
            self.download.file_name.to_ascii_lowercase()
        )
    }
}

fn bonsai_model_asset() -> Asset {
    Asset::new(
        "Bonsai 27B model",
        &format!(
            "https://huggingface.co/prism-ml/Ternary-Bonsai-27B-gguf/resolve/{BONSAI_REVISION}/Ternary-Bonsai-27B-Q2_0.gguf"
        ),
        "Ternary-Bonsai-27B-Q2_0.gguf",
        7_165_121_600,
        "868c11714cf8fe47f5ec9eeb2be0ab1a337112886f92ee0ede6b855c4fa31757",
    )
}

fn bonsai_projector_asset() -> Asset {
    Asset::new(
        "Bonsai image understanding",
        &format!(
            "https://huggingface.co/prism-ml/Ternary-Bonsai-27B-gguf/resolve/{BONSAI_REVISION}/Ternary-Bonsai-27B-mmproj-Q8_0.gguf"
        ),
        "Ternary-Bonsai-27B-mmproj-Q8_0.gguf",
        629_246_880,
        "eb561d41a7bbeb0fcf04883c8af11078ef6cae0a66862a0b68443cfca495269d",
    )
}

fn reusable_model_assets() -> Vec<ReusableModelAsset> {
    let mut assets = vec![
        ReusableModelAsset {
            component: "assistant",
            relative: "models/Ternary-Bonsai-27B-Q2_0.gguf".into(),
            download: bonsai_model_asset(),
        },
        ReusableModelAsset {
            component: "assistant",
            relative: "models/Ternary-Bonsai-27B-mmproj-Q8_0.gguf".into(),
            download: bonsai_projector_asset(),
        },
    ];
    assets.extend(h3_assets().into_iter().map(|asset| ReusableModelAsset {
        component: "studio",
        relative: format!("models/{}", asset.relative),
        download: asset.download_asset(),
    }));
    assets.push(ReusableModelAsset {
        component: "studio",
        relative: "models/vae_approx/taeh3.safetensors".into(),
        download: h3_preview_decoder(),
    });
    assets.extend(music_assets().into_iter().map(|asset| ReusableModelAsset {
        component: "music",
        relative: format!("models/{}", asset.relative),
        download: asset.download_asset(),
    }));
    assets.extend(
        ideogram_assets()
            .into_iter()
            .filter(|asset| asset.relative.ends_with(".safetensors"))
            .map(|asset| ReusableModelAsset {
                component: "image",
                relative: asset.relative.into(),
                download: asset.download,
            }),
    );
    assets.extend(speech_assets().into_iter().map(|asset| ReusableModelAsset {
        component: "speech",
        relative: asset.relative.into(),
        download: asset.download,
    }));
    assets.extend(STRATA_SIZES.into_iter().flat_map(StrataSize::shards));
    assets
}

fn configured_model_destination(
    research: &ResearchSettings,
    asset: &ReusableModelAsset,
) -> PathBuf {
    match asset.component {
        "assistant" => Path::new(&research.bonsai_root).join(&asset.relative),
        "strata" => strata_data_dir(&strata_install_dir(research)).join(&asset.relative),
        _ => Path::new(&research.comfy_root).join(&asset.relative),
    }
}

fn setup_model_assets(research: &ResearchSettings) -> Vec<SetupModelAsset> {
    let mut assets = reusable_model_assets()
        .into_iter()
        .map(|asset| {
            let destination = configured_model_destination(research, &asset);
            SetupModelAsset {
                id: asset.id(),
                component: asset.component.into(),
                label: asset.download.name,
                file_name: asset.download.file_name,
                bytes: asset.download.bytes,
                recognized: file_has_size(&destination, asset.download.bytes),
                installed_path: destination.to_string_lossy().into_owned(),
            }
        })
        .collect::<Vec<_>>();
    let (_, muscriptor_model, _) = managed_muscriptor_paths(Path::new(&research.install_root));
    assets.push(SetupModelAsset {
        id: "muscriptor:model.safetensors".into(),
        component: "muscriptor".into(),
        label: "MuScriptor large checkpoint".into(),
        file_name: "model.safetensors".into(),
        bytes: MUSCRIPTOR_MODEL_BYTES,
        recognized: file_has_size(&muscriptor_model, MUSCRIPTOR_MODEL_BYTES),
        installed_path: muscriptor_model.to_string_lossy().into_owned(),
    });
    assets
}

pub fn scan_existing_model_folder(root: &Path) -> Result<BTreeMap<String, String>, SetupError> {
    const MAX_SCAN_ENTRIES: usize = 50_000;
    if !root.is_absolute() || !root.is_dir() {
        return Err(SetupError::InvalidPath(root.to_string_lossy().into_owned()));
    }
    let mut expected = reusable_model_assets()
        .into_iter()
        .map(|asset| {
            (
                asset.download.file_name.to_ascii_lowercase(),
                (asset.id(), asset.download.bytes),
            )
        })
        .collect::<BTreeMap<_, _>>();
    expected.insert(
        "model.safetensors".into(),
        (
            "muscriptor:model.safetensors".into(),
            MUSCRIPTOR_MODEL_BYTES,
        ),
    );
    let mut matches = BTreeMap::new();
    for (index, entry) in walkdir::WalkDir::new(root)
        .max_depth(10)
        .follow_links(false)
        .into_iter()
        .enumerate()
    {
        if index >= MAX_SCAN_ENTRIES {
            return Err(SetupError::Dependency {
                name: "existing model search".into(),
                details: format!(
                    "{} contains more than {MAX_SCAN_ENTRIES} entries. Choose a narrower AI model folder, then scan again.",
                    root.display()
                ),
            });
        }
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(file_name) = entry.file_name().to_str() else {
            continue;
        };
        let Some((id, bytes)) = expected.get(&file_name.to_ascii_lowercase()) else {
            continue;
        };
        if entry
            .metadata()
            .is_ok_and(|metadata| metadata.len() == *bytes)
        {
            matches
                .entry(id.clone())
                .or_insert_with(|| entry.path().to_string_lossy().into_owned());
        }
    }
    Ok(matches)
}

async fn install_or_reuse_model(
    app: &AppHandle,
    request: &SetupInstallRequest,
    asset: &ReusableModelAsset,
    destination: &Path,
    cancel: &CancellationToken,
) -> Result<(), SetupError> {
    if verified(destination, &asset.download).await? {
        emit(
            app,
            asset.component,
            "verified",
            &format!("{} is already complete.", asset.download.name),
            asset.download.bytes,
            asset.download.bytes,
            0,
        );
        return Ok(());
    }
    if let Some(source) = request
        .existing_model_paths
        .get(&asset.id())
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        import_verified_asset(
            app,
            asset.component,
            Path::new(source),
            destination,
            &asset.download,
        )
        .await?;
        return Ok(());
    }
    download(app, asset.component, &asset.download, destination, cancel).await
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

async fn import_verified_asset(
    app: &AppHandle,
    component: &str,
    source: &Path,
    destination: &Path,
    asset: &Asset,
) -> Result<(), SetupError> {
    if !source.is_absolute() || !source.is_file() {
        return Err(SetupError::Dependency {
            name: asset.name.clone(),
            details: format!(
                "the selected existing checkpoint is not a local file: {}",
                source.display()
            ),
        });
    }
    emit(
        app,
        component,
        "verifying",
        &format!("Checking existing {}", asset.name),
        0,
        asset.bytes,
        0,
    );
    if fs::metadata(source)?.len() != asset.bytes {
        return Err(SetupError::Integrity {
            name: asset.name.clone(),
            details: format!(
                "{} is not the supported verified {} checkpoint. Leave the field empty to let Setup download the correct file.",
                source.display(),
                asset.file_name
            ),
        });
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = destination.with_extension(format!(
        "{}.kestrel-importing",
        destination
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("model")
    ));
    if temporary.is_file() {
        fs::remove_file(&temporary)?;
    }
    if fs::hard_link(source, &temporary).is_err() {
        let available =
            fs2::available_space(destination.parent().unwrap_or_else(|| Path::new(".")))
                .unwrap_or(u64::MAX);
        if asset.bytes > available {
            return Err(SetupError::InsufficientSpace {
                name: asset.name.clone(),
                needed: human_bytes(asset.bytes),
                available: human_bytes(available),
            });
        }
        tokio::fs::copy(source, &temporary).await?;
    }
    if !verified(&temporary, asset).await? {
        let _ = fs::remove_file(&temporary);
        return Err(SetupError::Integrity {
            name: asset.name.clone(),
            details: format!(
                "{} is not the supported verified {} checkpoint. Leave the field empty to let Setup download the correct file.",
                source.display(),
                asset.file_name
            ),
        });
    }
    let backup = destination.with_extension(format!(
        "{}.kestrel-backup",
        destination
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("model")
    ));
    if backup.is_file() {
        fs::remove_file(&backup)?;
    }
    if destination.is_file() {
        fs::rename(destination, &backup)?;
    }
    if let Err(error) = fs::rename(&temporary, destination) {
        if backup.is_file() {
            let _ = fs::rename(&backup, destination);
        }
        return Err(error.into());
    }
    if backup.is_file() {
        fs::remove_file(backup)?;
    }
    Ok(())
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
        sevenz_rust2::decompress_file_with_extract_fn(
            &archive,
            &destination,
            |entry, reader, output| {
                let relative = Path::new(entry.name());
                if relative.is_absolute()
                    || relative
                        .components()
                        .any(|component| !matches!(component, std::path::Component::Normal(_)))
                {
                    return Err(sevenz_rust2::Error::Other(
                        "archive contains an unsafe path".into(),
                    ));
                }
                sevenz_rust2::default_entry_extract_fn(entry, reader, output)
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
$bundledPreview=[ordered]@{
  'custom_nodes\ComfyUI-KJNodes\nodes\preview_override_node.py'=__KJ_PREVIEW_NODE_BYTES__
  'custom_nodes\ComfyUI-KJNodes\nodes\tiny_vae.py'=__KJ_TINY_VAE_BYTES__
}
$managedPreview=[ordered]@{
  'custom_nodes\Kestrel-H3-Live-Preview\preview_override_node.py'=__KJ_PREVIEW_NODE_BYTES__
  'custom_nodes\Kestrel-H3-Live-Preview\tiny_vae.py'=__KJ_TINY_VAE_BYTES__
}
$bundledReady=@($bundledPreview.GetEnumerator() | Where-Object {
  $path=Join-Path $root $_.Key
  (-not (Test-Path -LiteralPath $path)) -or ((Get-Item -LiteralPath $path).Length -ne $_.Value)
}).Count -eq 0
$managedReady=(Test-Path -LiteralPath (Join-Path $root 'custom_nodes\Kestrel-H3-Live-Preview\__init__.py')) -and
  (Test-Path -LiteralPath (Join-Path $root 'custom_nodes\Kestrel-H3-Live-Preview\LICENSE')) -and
  @($managedPreview.GetEnumerator() | Where-Object {
    $path=Join-Path $root $_.Key
    (-not (Test-Path -LiteralPath $path)) -or ((Get-Item -LiteralPath $path).Length -ne $_.Value)
  }).Count -eq 0
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
"#
    .replace(
        "__KJ_PREVIEW_NODE_BYTES__",
        &KJ_PREVIEW_NODE_BYTES.to_string(),
    )
    .replace("__KJ_TINY_VAE_BYTES__", &KJ_TINY_VAE_BYTES.to_string());
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
    let _ = crate::ipc_events::emit::<kestrel_app_core::events::SetupProgress>(
        app,
        &SetupProgress {
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
    fn strata_sizes_are_pinned_and_share_the_second_shard() {
        for size in STRATA_SIZES {
            let [first, second] = size.shards();
            for shard in [&first, &second] {
                assert_eq!(shard.component, "strata");
                assert!(shard.download.url.contains(STRATA_MODEL_REVISION));
                assert!(shard.download.url.ends_with(&shard.download.file_name));
                assert_eq!(shard.download.sha256.len(), 64);
                assert!(shard
                    .relative
                    .starts_with(&format!("models/{}/", size.name)));
            }
            assert!(first.download.file_name.ends_with("-00001-of-00002.gguf"));
            assert_eq!(second.download.sha256, STRATA_SHARD_TWO_SHA256);
            assert_eq!(second.download.bytes, STRATA_SHARD_TWO_BYTES);
        }
        assert_eq!(StrataSize::parse("iq3_s").unwrap().name, "IQ3_S");
        assert!(StrataSize::parse("Q2_0").is_err());
        let ids = reusable_model_assets()
            .into_iter()
            .map(|asset| asset.id())
            .collect::<Vec<_>>();
        assert!(ids
            .contains(&"strata:qwen3.8-flash-next-gsq-rco-iq3_s-00001-of-00002.gguf".to_string()));
    }

    #[test]
    fn strata_component_counts_the_default_size_and_its_support_files() {
        let root = tempfile::tempdir().unwrap();
        let research = ResearchSettings {
            strata_root: root.path().join("Strata").to_string_lossy().into_owned(),
            ..ResearchSettings::default()
        };
        let value = snapshot(&research, &ControlSettings::default(), None, None);
        let strata = value
            .components
            .iter()
            .find(|item| item.id == "strata")
            .unwrap();
        assert!(strata.optional);
        let recommended = value
            .strata_choices
            .iter()
            .find(|choice| choice.recommended)
            .or_else(|| value.strata_choices.first())
            .unwrap();
        assert_eq!(strata.download_bytes, recommended.download_bytes);
        assert_eq!(value.strata_choices.len(), STRATA_SIZES.len());
    }

    #[test]
    fn strata_choices_need_the_tested_pc() {
        let root = tempfile::tempdir().unwrap();
        let strata = root.path().join("Strata");
        // The tested 64 GB PC reports 63.1 GiB.
        let tested = strata_choices(&strata, 63 * GIB, true);
        assert_eq!(
            tested
                .iter()
                .map(|choice| choice.size.as_str())
                .collect::<Vec<_>>(),
            ["IQ3_S", "IQ2_XS"]
        );
        assert!(tested[0].fits && tested[0].recommended);
        assert!(tested[1].fits && !tested[1].recommended);
        assert!(tested.iter().all(|choice| choice.memory_bytes == 64 * GIB));
        for (memory, hardware) in [(48 * GIB, true), (0, true), (63 * GIB, false)] {
            assert!(strata_choices(&strata, memory, hardware)
                .iter()
                .all(|choice| !choice.fits && !choice.recommended));
        }
        let large = tested;
        assert_eq!(
            large[1].download_bytes,
            STRATA_SIZES[1].shard_one_bytes
                + STRATA_SHARD_TWO_BYTES
                + STRATA_SOURCE_BYTES
                + STRATA_ENGINE_BYTES
                + STRATA_PREPARED_BYTES
        );
    }

    #[test]
    fn strata_blocker_explains_the_tested_minimum() {
        let tested = crate::strata::Gpu {
            index: 0,
            name: "NVIDIA GeForce RTX 5070".into(),
            vram_mib: 12_227,
            compute_capability: (12, 0),
            driver: "610.74".into(),
        };
        let no_gpu = strata_blocker(None, 63 * GIB).unwrap();
        assert!(no_gpu.contains("No NVIDIA"), "{no_gpu}");
        if crate::strata::cpu_has_avx2() {
            assert_eq!(strata_blocker(Some(&tested), 63 * GIB), None);
            let small = strata_blocker(Some(&tested), 48 * GIB).unwrap();
            assert!(
                small.contains("48.0 GB") && small.contains("64 GB PC"),
                "{small}"
            );
            let unknown = strata_blocker(Some(&tested), 0).unwrap();
            assert!(unknown.contains("did not report"), "{unknown}");
        }
    }

    #[test]
    fn strata_installs_only_into_a_folder_kestrel_owns() {
        let root = tempfile::tempdir().unwrap();
        let strata = root.path().join("Strata");
        assert!(ensure_kestrel_strata_folder(&strata).is_ok());
        fs::create_dir_all(&strata).unwrap();
        assert!(ensure_kestrel_strata_folder(&strata).is_ok());

        fs::write(strata.join("setup.py"), b"their setup").unwrap();
        let error = ensure_kestrel_strata_folder(&strata)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("its own setup") && error.contains("START-HERE"),
            "{error}"
        );
        fs::remove_file(strata.join("setup.py")).unwrap();
        fs::write(strata.join("notes.txt"), b"unrelated").unwrap();
        let error = ensure_kestrel_strata_folder(&strata)
            .unwrap_err()
            .to_string();
        assert!(error.contains("other files"), "{error}");

        fs::write(
            strata.join(STRATA_SETUP_MARKER),
            "unpacking
",
        )
        .unwrap();
        assert!(ensure_kestrel_strata_folder(&strata).is_ok());
    }

    #[test]
    fn default_strata_folder_moves_with_the_install_root_until_it_holds_files() {
        let old = tempfile::tempdir().unwrap();
        let new = tempfile::tempdir().unwrap();
        let locations = |research: &ResearchSettings, install_root: &Path| SetupLocations {
            install_root: install_root.to_string_lossy().into_owned(),
            bonsai_root: research.bonsai_root.clone(),
            engine_path: old
                .path()
                .join("llama-server.exe")
                .to_string_lossy()
                .into_owned(),
            wikipedia_zim_path: research.wikipedia_zim_path.clone(),
            kiwix_server_path: research.kiwix_server_path.clone(),
            comfy_root: research.comfy_root.clone(),
            ffmpeg_path: String::new(),
            ffprobe_path: String::new(),
            strata_root: research.strata_root.clone(),
        };
        let start = || ResearchSettings {
            install_root: old.path().to_string_lossy().into_owned(),
            strata_root: old.path().join("Strata").to_string_lossy().into_owned(),
            ..ResearchSettings::default()
        };

        let mut research = start();
        let request = locations(&research, new.path());
        apply_locations(&mut research, &mut ControlSettings::default(), request).unwrap();
        assert_eq!(
            PathBuf::from(&research.strata_root),
            new.path().join("Strata")
        );
        assert_eq!(strata_install_dir(&research), new.path().join("Strata"));

        // A partial download beside the old folder keeps Strata where its files are.
        let mut research = start();
        let shard = &STRATA_SIZES[0].shards()[0];
        let partial = strata_data_dir(&old.path().join("Strata"))
            .join(&shard.relative)
            .with_extension("gguf.part");
        fs::create_dir_all(partial.parent().unwrap()).unwrap();
        fs::write(&partial, b"partial").unwrap();
        let request = locations(&research, new.path());
        apply_locations(&mut research, &mut ControlSettings::default(), request).unwrap();
        assert_eq!(
            PathBuf::from(&research.strata_root),
            old.path().join("Strata")
        );

        // A folder the producer chose never follows the install root.
        let chosen = new.path().join("Models").join("Strata");
        let mut research = ResearchSettings {
            strata_root: chosen.to_string_lossy().into_owned(),
            ..start()
        };
        let request = locations(&research, new.path());
        apply_locations(&mut research, &mut ControlSettings::default(), request).unwrap();
        assert_eq!(PathBuf::from(&research.strata_root), chosen);
    }

    #[tokio::test]
    async fn read_tail_drains_everything_and_keeps_the_end() {
        let mut output = vec![b'x'; 300 * 1024];
        output.extend_from_slice(b"Traceback: the real error");
        let tail = read_tail(output.as_slice(), 64).await;
        assert!(tail.ends_with("Traceback: the real error"));
        assert_eq!(tail.len(), 64);
        assert_eq!(read_tail(&b"short"[..], 64).await, "short");
    }

    #[test]
    fn strata_source_update_keeps_what_strata_setup_created() {
        use std::io::Write as _;
        let root = tempfile::tempdir().unwrap();
        let archive = root.path().join("strata.zip");
        {
            let mut writer = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            let top = format!("Strata-{STRATA_COMMIT}");
            writer
                .start_file(format!("{top}/setup.py"), options)
                .unwrap();
            writer.write_all(b"new setup").unwrap();
            writer
                .start_file(format!("{top}/serve/server.py"), options)
                .unwrap();
            writer.write_all(b"new server").unwrap();
            writer.finish().unwrap();
        }
        let strata = root.path().join("Strata");
        fs::create_dir_all(strata.join(".venv")).unwrap();
        fs::write(strata.join("setup.py"), b"old setup").unwrap();
        fs::write(strata.join("strata-iq3_s.json"), b"{}").unwrap();

        install_strata_source(&archive, &strata, "Strata").unwrap();

        assert_eq!(fs::read(strata.join("setup.py")).unwrap(), b"new setup");
        assert_eq!(
            fs::read(strata.join("serve/server.py")).unwrap(),
            b"new server"
        );
        assert!(strata.join(".venv").is_dir());
        assert!(strata.join("strata-iq3_s.json").is_file());
        assert_eq!(
            read_bounded_text(&strata.join(STRATA_SETUP_MARKER), 256)
                .unwrap()
                .trim(),
            STRATA_COMMIT
        );
        assert!(!strata.with_extension("kestrel-unpacking").exists());
    }

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
                strata_root: String::new(),
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn existing_model_scan_covers_every_release_profile_and_ignores_wrong_sizes() {
        let root = tempfile::tempdir().unwrap();
        let assets = reusable_model_assets();
        assert!(assets.len() >= 5);
        assert!(assets.iter().any(|asset| asset.component == "assistant"));
        assert!(assets.iter().any(|asset| asset.component == "studio"));
        assert!(assets.iter().any(|asset| asset.component == "music"));
        assert!(assets.iter().any(|asset| asset.component == "image"));
        assert!(assets.iter().any(|asset| asset.component == "speech"));

        let music = assets
            .iter()
            .find(|asset| asset.download.file_name == "minimax_music3_dit_int8_convrot.safetensors")
            .unwrap();
        let nested = root.path().join("ComfyUI/models/diffusion_models");
        fs::create_dir_all(&nested).unwrap();
        crate::test_support::sparse_file(
            nested.join(&music.download.file_name),
            music.download.bytes,
        );
        fs::File::create(nested.join("large-v3-turbo.pt"))
            .unwrap()
            .set_len(1)
            .unwrap();

        let matches = scan_existing_model_folder(root.path()).unwrap();
        assert!(matches.get(&music.id()).is_some_and(|path| paths_equal(
            Path::new(path),
            &nested.join(&music.download.file_name)
        )));
        assert!(!matches.contains_key("speech:large-v3-turbo.pt"));
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
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        fs::write(
            comfy.join("custom_nodes/Kestrel-Whisper/.kestrel-speech-ready"),
            speech_marker_contents(),
        )
        .unwrap();
        for asset in h3_assets() {
            let path = comfy.join("models").join(asset.relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            crate::test_support::sparse_file(path, asset.bytes);
        }
        for asset in speech_assets() {
            let path = comfy.join(asset.relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            crate::test_support::sparse_file(path, asset.download.bytes);
        }
        for asset in ideogram_assets() {
            let path = comfy.join(asset.relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            crate::test_support::sparse_file(path, asset.download.bytes);
        }
        for path in [
            comfy.join("comfy_extras/nodes_ideogram4.py"),
            comfy.join("comfy_extras/nodes_custom_sampler.py"),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
        }
        let decoder = comfy.join("models/vae_approx/taeh3.safetensors");
        fs::create_dir_all(decoder.parent().unwrap()).unwrap();
        crate::test_support::sparse_file(decoder, h3_preview_decoder().bytes);
        let preview = comfy.join("custom_nodes/ComfyUI-KJNodes/nodes");
        fs::create_dir_all(&preview).unwrap();
        let preview_node = preview.join("preview_override_node.py");
        fs::write(
            &preview_node,
            b"class ModelPreviewOverrideKJ: pass\nkj_preview_override = True\ntiny_vae = True",
        )
        .unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(preview_node)
            .unwrap()
            .set_len(KJ_PREVIEW_NODE_BYTES)
            .unwrap();
        fs::File::create(preview.join("tiny_vae.py"))
            .unwrap()
            .set_len(KJ_TINY_VAE_BYTES)
            .unwrap();
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
        let value = snapshot(&research, &control, None, None);
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
        assert_eq!(
            value
                .components
                .iter()
                .find(|item| item.id == "image")
                .unwrap()
                .status,
            "ready"
        );
        fs::write(
            comfy.join("custom_nodes/Kestrel-Whisper/.kestrel-speech-ready"),
            "kestrel-whisper-old\nchatterbox-node=old\nchatterbox-model=old\n",
        )
        .unwrap();
        let stale = snapshot(&research, &control, None, None);
        assert_eq!(
            stale
                .components
                .iter()
                .find(|item| item.id == "speech")
                .unwrap()
                .status,
            "partial"
        );
    }

    #[test]
    fn h3_preview_readiness_rejects_the_legacy_decoder_loader() {
        let root = tempfile::tempdir().unwrap();
        let nodes = root.path().join("custom_nodes/ComfyUI-KJNodes/nodes");
        fs::create_dir_all(&nodes).unwrap();
        let preview = nodes.join("preview_override_node.py");
        fs::write(
            &preview,
            b"class ModelPreviewOverrideKJ: pass\nkj_preview_override = True\ntiny_vae = True",
        )
        .unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&preview)
            .unwrap()
            .set_len(37_220)
            .unwrap();
        fs::File::create(nodes.join("tiny_vae.py"))
            .unwrap()
            .set_len(4_434)
            .unwrap();
        assert!(!bundled_preview_node_ready(root.path()));

        fs::OpenOptions::new()
            .write(true)
            .open(preview)
            .unwrap()
            .set_len(KJ_PREVIEW_NODE_BYTES)
            .unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(nodes.join("tiny_vae.py"))
            .unwrap()
            .set_len(KJ_TINY_VAE_BYTES)
            .unwrap();
        assert!(bundled_preview_node_ready(root.path()));
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
        assert!(!is_kestrel_managed_comfy_root(&managed, root.path()));

        mark_kestrel_managed_comfy_root(&managed).unwrap();
        assert!(is_kestrel_managed_comfy_root(&managed, root.path()));
        fs::remove_file(managed.join(KESTREL_MANAGED_COMFY_MARKER)).unwrap();
        fs::write(
            managed.join("Start-Kestrel-ComfyUI.ps1"),
            "# Kestrel-managed shared ComfyUI launcher",
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
    fn setup_recognizes_only_a_complete_managed_muscriptor_install() {
        let root = tempfile::tempdir().unwrap();
        let (executable, model, marker) = managed_muscriptor_paths(root.path());
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::create_dir_all(model.parent().unwrap()).unwrap();
        fs::write(&executable, b"runner").unwrap();
        crate::test_support::sparse_file(&model, MUSCRIPTOR_MODEL_BYTES);
        fs::write(&marker, format!("{MUSCRIPTOR_SETUP_REVISION}\n")).unwrap();
        let research = ResearchSettings {
            install_root: root.path().to_string_lossy().into_owned(),
            ..ResearchSettings::default()
        };
        let value = snapshot(&research, &ControlSettings::default(), None, None);
        assert_eq!(
            value
                .components
                .iter()
                .find(|item| item.id == "muscriptor")
                .unwrap()
                .status,
            "ready"
        );
        fs::write(&marker, "stale").unwrap();
        let stale = snapshot(&research, &ControlSettings::default(), None, None);
        assert_eq!(
            stale
                .components
                .iter()
                .find(|item| item.id == "muscriptor")
                .unwrap()
                .status,
            "partial"
        );
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
    fn ideogram_assets_are_pinned_and_match_the_setup_download_budget() {
        let assets = ideogram_assets();
        assert_eq!(assets.len(), 5);
        assert_eq!(
            assets.iter().map(|asset| asset.download.bytes).sum::<u64>(),
            17_622_549_040
        );
        assert!(assets.iter().all(|asset| asset.download.sha256.len() == 64));
        assert!(assets.iter().any(|asset| {
            asset
                .relative
                .ends_with("IDEOGRAM-4-NON-COMMERCIAL-LICENSE.txt")
                && asset.download.url.contains(IDEOGRAM_LICENSE_REVISION)
        }));
        assert!(assets.iter().any(|asset| {
            asset.relative.ends_with("qwen3vl_8b_nvfp4.safetensors")
                && asset.download.url.contains(QWEN3_VL_REVISION)
        }));
    }

    #[test]
    fn kestrel_whisper_installer_writes_the_owned_offline_adapter() {
        let root = tempfile::tempdir().unwrap();
        install_kestrel_whisper_node(root.path()).unwrap();

        let node =
            fs::read_to_string(root.path().join("custom_nodes/Kestrel-Whisper/nodes.py")).unwrap();
        assert!(node.contains("class KestrelWhisper"));
        assert!(node.contains("EXPECTED_MODELS"));
        assert!(node.contains("MUSIC_CONTEXT_MODE"));
        assert!(node.contains("carry_initial_prompt"));
        assert!(node.contains("source_audio, seam, source_audio"));
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
