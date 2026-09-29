//! Strata engine support: read-only install discovery, validated launch plans, and a process tree
//! that Kestrel owns as one unit.
//!
//! Strata (github.com/Niko1221/Strata, MIT) runs Qwen3.8-Flash-Next, a 125B mixture-of-experts
//! model, on one consumer NVIDIA GPU: every expert lives in system RAM, the most-used experts are
//! cached on the GPU, and a lookup table stays on disk. Its Python server (`serve/server.py`)
//! drives `strata.exe` over a pipe and speaks the OpenAI chat API on loopback.
//!
//! Discovery reads only bounded JSON: the Setup-managed Strata folder and the folders Strata
//! itself recorded for this Windows user. Nothing is executed, downloaded, or rewritten. A run
//! configuration is listed only when its engine, model shards, tokenizer, Python environment,
//! server script, and (when it turns images on) vision encoder files are present and it declares
//! no MCP servers, because Kestrel chat is tool-free.

use crate::model::{self, ModelEngine, ModelInfo};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};

/// GGUF architectures that only Strata's engine can load. The llama.cpp scan skips them so the
/// same weights appear once, through the Strata install that can actually run them.
pub const STRATA_ONLY_ARCHITECTURES: &[&str] = &["qwen4exp"];

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const MAX_SETTINGS_BYTES: u64 = 64 * 1024;
const MAX_INSTALLS: usize = 16;
const MAX_CONFIGS_PER_INSTALL: usize = 16;
const MIN_CONTEXT: u32 = 4_096;
const MAX_CONTEXT: u32 = 1_048_576;

#[cfg(windows)]
const ENGINE_FILE: &str = "strata.exe";
#[cfg(not(windows))]
const ENGINE_FILE: &str = "strata";
#[cfg(windows)]
const VISION_FILE: &str = "strata-vision.exe";
#[cfg(not(windows))]
const VISION_FILE: &str = "strata-vision";

/// A Strata run configuration (`strata-<size>.json`) as its own setup writes it. Unknown fields
/// belong to Strata and are passed through untouched because Kestrel never rewrites the file.
#[derive(Debug, Deserialize)]
struct RunConfig {
    exe: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    tokenizer: String,
    #[serde(default)]
    vision: Option<serde_json::Value>,
    #[serde(default)]
    mcp_servers: Option<serde_json::Value>,
}

/// The `vision` entry Strata's setup writes when images are turned on. Its server starts
/// `strata-vision` with all three files before it loads the model.
#[derive(Debug, Deserialize)]
struct VisionConfig {
    #[serde(default)]
    exe: String,
    #[serde(default)]
    mmproj: String,
    #[serde(default)]
    model: String,
}

/// Everything the runtime needs to start one Strata model, revalidated at every launch because
/// Strata's own setup may repair or replace the configuration between Kestrel sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub root: PathBuf,
    pub python: PathBuf,
    pub server: PathBuf,
    pub config: PathBuf,
    pub context_window: u32,
}

impl LaunchPlan {
    /// Fixed server arguments. `--fit-max-tokens` makes Strata shorten a reply that would run past
    /// its context instead of refusing it, matching how llama.cpp stops at the end of its window.
    /// The API key travels in `STRATA_API_KEY`, never on the command line.
    pub fn server_args(&self, port: u16) -> Vec<String> {
        vec![
            "-u".into(),
            self.server.to_string_lossy().into_owned(),
            "--engine".into(),
            "strata".into(),
            "--config".into(),
            self.config.to_string_lossy().into_owned(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string(),
            "--fit-max-tokens".into(),
        ]
    }
}

/// Strata folders worth inspecting: the configured ones first, then those Strata recorded in
/// `%APPDATA%\Strata\settings.json`. Only existing absolute folders are returned.
pub fn known_installs(configured: &[&str]) -> Vec<PathBuf> {
    let recorded = settings_file()
        .map(|path| recorded_installs(&path))
        .unwrap_or_default();
    let mut seen = HashSet::new();
    configured
        .iter()
        .map(|value| PathBuf::from(value.trim()))
        .chain(recorded)
        .filter(|path| path.is_absolute() && path.is_dir())
        .filter(|path| seen.insert(path.to_string_lossy().to_lowercase()))
        .take(MAX_INSTALLS)
        .collect()
}

fn settings_file() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("Strata").join("settings.json"))
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| directories::BaseDirs::new().map(|base| base.home_dir().join(".config")))
            .map(|path| path.join("strata").join("settings.json"))
    }
}

fn recorded_installs(path: &Path) -> Vec<PathBuf> {
    #[derive(Deserialize)]
    struct Settings {
        #[serde(default)]
        installs: Vec<String>,
    }
    read_bounded(path, MAX_SETTINGS_BYTES)
        .and_then(|bytes| serde_json::from_slice::<Settings>(&bytes).map_err(io::Error::other))
        .map(|settings| {
            settings
                .installs
                .into_iter()
                .take(MAX_INSTALLS)
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Every runnable Strata model in the given install folders.
pub fn discover(installs: &[PathBuf]) -> Vec<ModelInfo> {
    let mut models = Vec::new();
    for root in installs {
        for config in run_configs(root) {
            if let Ok(model) = inspect(&config) {
                models.push(model);
            }
        }
    }
    models
}

/// The Strata folder that holds a ready model, preferring the configured one. This validates
/// configurations and file presence only; it never reads the multi-gigabyte model shards.
pub fn ready_install(installs: &[PathBuf]) -> Option<PathBuf> {
    installs
        .iter()
        .find(|root| {
            run_configs(root)
                .iter()
                .any(|config| validate(config).is_ok())
        })
        .cloned()
}

/// `strata-*.json` run configurations directly inside one install, excluding the
/// `*.shared-settings.json` files the Strata server keeps beside them.
fn run_configs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut configs = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_ascii_lowercase)
                    .is_some_and(|name| {
                        name.starts_with("strata-")
                            && name.ends_with(".json")
                            && !name.contains(".shared-settings")
                    })
        })
        .collect::<Vec<_>>();
    configs.sort();
    configs.truncate(MAX_CONFIGS_PER_INSTALL);
    configs
}

/// Validate one run configuration for launch. Errors name the missing piece so the producer can
/// repair the install with Strata's setup or Kestrel Setup.
pub fn launch_plan(config: &Path) -> Result<LaunchPlan, String> {
    let (plan, _) = validate(config)?;
    Ok(plan)
}

struct Validated {
    native: PathBuf,
    mmproj: Option<PathBuf>,
}

fn validate(config: &Path) -> Result<(LaunchPlan, Validated), String> {
    let config = config.to_path_buf();
    if !config.is_absolute() {
        return Err(format!(
            "the Strata run configuration must be an absolute path: {}",
            config.display()
        ));
    }
    let root = config
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("{} has no Strata folder", config.display()))?;
    let bytes = read_bounded(&config, MAX_CONFIG_BYTES)
        .map_err(|error| format!("could not read {}: {error}", config.display()))?;
    let parsed: RunConfig = serde_json::from_slice(strip_bom(&bytes)).map_err(|error| {
        format!(
            "{} is not a Strata run configuration: {error}",
            config.display()
        )
    })?;
    if parsed
        .mcp_servers
        .as_ref()
        .is_some_and(|value| !is_empty_json(value))
    {
        return Err(format!(
            "{} starts MCP tool servers. Kestrel runs only tool-free Strata configurations; remove \"mcp_servers\" or use another configuration.",
            config.display()
        ));
    }
    let python = venv_python(&root);
    if !python.is_file() {
        return Err(format!(
            "Strata's Python environment is missing ({}). Run Kestrel Setup or Strata's START-HERE.bat to repair it.",
            python.display()
        ));
    }
    let server = root.join("serve").join("server.py");
    if !server.is_file() {
        return Err(format!(
            "Strata's server is missing ({}). Reinstall Strata in Setup.",
            server.display()
        ));
    }
    let engine = PathBuf::from(&parsed.exe);
    let engine_named = engine
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(ENGINE_FILE));
    if !engine.is_absolute() || !engine_named || !engine.is_file() {
        return Err(format!(
            "{} must name an existing {ENGINE_FILE}; found {}",
            config.display(),
            parsed.exe
        ));
    }
    let native = argument(&parsed.args, "--native")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_file())
        .ok_or_else(|| {
            format!(
                "{} refers to a missing model file. Run Strata's setup again to repair it.",
                config.display()
            )
        })?;
    if let Some(ple) = argument(&parsed.args, "--ple-gguf") {
        if !Path::new(ple).is_file() {
            return Err(format!(
                "{} refers to a missing model shard: {ple}",
                config.display()
            ));
        }
    }
    let context_window = argument(&parsed.args, "--max-context")
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| (MIN_CONTEXT..=MAX_CONTEXT).contains(value))
        .ok_or_else(|| {
            format!(
                "{} does not declare a usable --max-context",
                config.display()
            )
        })?;
    let tokenizer = if parsed.tokenizer.trim().is_empty() {
        root.join("pack").join("full").join("tokenizer")
    } else {
        PathBuf::from(&parsed.tokenizer)
    };
    if !tokenizer.join("vocab.json").is_file() {
        return Err(format!(
            "the model's tokenizer is missing ({}). Run Strata's setup again to repair it.",
            tokenizer.display()
        ));
    }
    let mmproj = vision_projector(&config, parsed.vision.as_ref())?;
    Ok((
        LaunchPlan {
            root,
            python,
            server,
            config,
            context_window,
        },
        Validated { native, mmproj },
    ))
}

/// The image projector of a configuration that turns images on. Strata's server starts its
/// vision encoder whenever `vision` is set, so a missing file would stop the whole server at
/// launch; it is refused here with the file named instead.
fn vision_projector(
    config: &Path,
    vision: Option<&serde_json::Value>,
) -> Result<Option<PathBuf>, String> {
    // Python truthiness, as the server reads it: null, false, and empty entries mean no images.
    let Some(value) = vision
        .filter(|value| !is_empty_json(value) && !matches!(value, serde_json::Value::Bool(false)))
    else {
        return Ok(None);
    };
    let vision: VisionConfig = serde_json::from_value(value.clone()).map_err(|error| {
        format!(
            "{} turns on images with an unreadable \"vision\" entry: {error}",
            config.display()
        )
    })?;
    let exe = Path::new(&vision.exe);
    let exe_named = exe
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(VISION_FILE));
    if !exe.is_absolute() || !exe_named || !exe.is_file() {
        return Err(format!(
            "{} turns on images but does not name an existing {VISION_FILE}; found \"{}\". Run Strata's setup again to repair it, or choose no vision there.",
            config.display(),
            vision.exe
        ));
    }
    for (label, file) in [
        ("image projector", &vision.mmproj),
        ("model shard for images", &vision.model),
    ] {
        let path = Path::new(file);
        if !path.is_absolute() || !path.is_file() {
            return Err(format!(
                "{} turns on images, but its {label} is missing: \"{file}\". Run Strata's setup again to repair it, or choose no vision there.",
                config.display()
            ));
        }
    }
    Ok(Some(PathBuf::from(vision.mmproj)))
}

fn inspect(config: &Path) -> Result<ModelInfo, String> {
    let (plan, validated) = validate(config)?;
    let bytes = fs::metadata(&plan.config)
        .map_err(|error| error.to_string())?
        .len();
    let shard_bytes = fs::metadata(&validated.native)
        .map_err(|error| error.to_string())?
        .len();
    let metadata = model::read_gguf_metadata(&validated.native).unwrap_or_default();
    let architecture = metadata
        .get("general.architecture")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    let family = metadata
        .get("general.name")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "Qwen3.8-Flash-Next".into());
    let quantization = quantization_from_file(&validated.native);
    let shard_identity = model::content_identity(&validated.native, shard_bytes)
        .map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(b"strata\0");
    hasher.update(shard_identity.as_bytes());
    let id = hex::encode(hasher.finalize())[..24].to_string();
    let name = match &quantization {
        Some(size) => format!("{family} {size} (Strata)"),
        None => format!("{family} (Strata)"),
    };
    Ok(ModelInfo {
        id,
        name,
        path: plan.config.to_string_lossy().into_owned(),
        source: "Strata".into(),
        bytes,
        architecture,
        context_length: Some(u64::from(plan.context_window)),
        chat_template: true,
        quantization,
        mmproj_path: validated
            .mmproj
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        supports_vision: validated.mmproj.is_some(),
        supports_audio: false,
        recommendation: format!(
            "Runs through Strata: every expert stays in system RAM and the busiest ones on the GPU. This install fixes the context at {} tokens; change it with Strata's own setup.",
            plan.context_window
        ),
        engine: ModelEngine::Strata,
        fixed_context_window: Some(plan.context_window),
    })
}

/// `Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00001-of-00002.gguf` -> `IQ3_S`.
fn quantization_from_file(path: &Path) -> Option<String> {
    path.file_stem()?
        .to_str()?
        .split('-')
        .find(|part| {
            let upper = part.to_ascii_uppercase();
            (upper.starts_with("IQ") || upper.starts_with('Q'))
                && upper.contains('_')
                && upper
                    .chars()
                    .nth(if upper.starts_with("IQ") { 2 } else { 1 })
                    .is_some_and(|character| character.is_ascii_digit())
        })
        .map(str::to_ascii_uppercase)
}

fn venv_python(root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        root.join(".venv").join("Scripts").join("python.exe")
    }
    #[cfg(not(windows))]
    {
        root.join(".venv").join("bin").join("python")
    }
}

fn argument<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|value| value == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn is_empty_json(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::Object(map) => map.is_empty(),
        serde_json::Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

fn read_bounded(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not a file of at most {limit} bytes", path.display()),
        ));
    }
    fs::read(path)
}

/// A Windows job object holding Strata's whole process tree: the Python server, `strata.exe`, and
/// the optional vision encoder it starts. Closing the job (on stop, or when Kestrel exits for any
/// reason) terminates every member, so no 50 GB engine can outlive the runtime that owned it.
#[cfg(windows)]
pub struct ProcessJob(windows_sys::Win32::Foundation::HANDLE);

// The job handle is an owned kernel handle used only through thread-safe Win32 calls.
#[cfg(windows)]
unsafe impl Send for ProcessJob {}
#[cfg(windows)]
unsafe impl Sync for ProcessJob {}

#[cfg(windows)]
impl ProcessJob {
    /// Start `command` inside a new job before it runs a single instruction. The process is
    /// created suspended, assigned to the job, and only then resumed, so nothing it starts can
    /// be created outside the job. This sets the command's creation flags to a hidden, suspended
    /// start. A process Kestrel cannot own is terminated, never left running.
    pub fn spawn(
        command: &mut tokio::process::Command,
    ) -> io::Result<(tokio::process::Child, Self)> {
        use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
        let job = Self::new()?;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        let mut child = command.spawn()?;
        if let Err(error) = job.assign(&child).and_then(|()| resume(&child)) {
            let _ = child.start_kill();
            return Err(error);
        }
        Ok((child, job))
    }

    fn new() -> io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        // SAFETY: plain Win32 calls on a handle owned here; the job handle is closed on drop.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let job = Self(handle);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }
    }

    fn assign(&self, child: &tokio::process::Child) -> io::Result<()> {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("the process exited before Kestrel could own it"))?;
        // SAFETY: both handles are open for the duration of the call.
        if unsafe { AssignProcessToJobObject(self.0, process.cast()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

/// Resume a process created suspended. The standard library does not return its first thread,
/// so the thread is found by process ID in a snapshot of the system's threads.
#[cfg(windows)]
fn resume(child: &tokio::process::Child) -> io::Result<()> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::{
        Foundation::INVALID_HANDLE_VALUE,
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            },
            Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
        },
    };
    let pid = child
        .id()
        .ok_or_else(|| io::Error::other("the process exited before Kestrel could start it"))?;
    // SAFETY: every handle is checked before it is wrapped and closed by its OwnedHandle; the
    // entry is sized as Thread32First requires.
    unsafe {
        let raw = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = OwnedHandle::from_raw_handle(raw);
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..THREADENTRY32::default()
        };
        let mut resumed = 0;
        let mut listed = Thread32First(snapshot.as_raw_handle(), &mut entry) != 0;
        while listed {
            if entry.th32OwnerProcessID == pid {
                let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let thread = OwnedHandle::from_raw_handle(thread);
                if ResumeThread(thread.as_raw_handle()) == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                resumed += 1;
            }
            listed = Thread32Next(snapshot.as_raw_handle(), &mut entry) != 0;
        }
        if resumed == 0 {
            return Err(io::Error::other(
                "Windows listed no thread to start for the new process",
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
impl Drop for ProcessJob {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateJobObjectW and is closed exactly once.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Installed system RAM in bytes, or 0 when it cannot be read.
pub fn system_memory_bytes() -> u64 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..MEMORYSTATUSEX::default()
        };
        // SAFETY: the structure is sized and owned here.
        if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
            return status.ullTotalPhys;
        }
        0
    }
    #[cfg(not(windows))]
    {
        0
    }
}

// The PC Kestrel tested Strata on is its minimum: an NVIDIA RTX 5070 (12 GB, compute capability
// 12.0) in a 64 GB PC, where Qwen3.8-Flash-Next uses 54-57 GB of RAM, nearly all the VRAM, and
// answers at about 40 tokens per second. Kestrel does not offer Strata on less capable hardware
// because it cannot vouch for it; raise these only after testing another class of PC.
/// RTX 50 series (Blackwell). Strata itself starts at 8.0, but Kestrel tested only this one.
const MIN_COMPUTE_CAPABILITY: (u32, u32) = (12, 0);
/// A 12 GB card; the tested RTX 5070 reports 12,227 MiB.
const MIN_VRAM_MIB: u64 = 12_000;
/// Strata's ready-made engine is built with CUDA 13.0, which needs driver 580 or newer.
const MIN_DRIVER: u32 = 580;
const SUPPORTED_GPU: &str = "an NVIDIA RTX 50-series card with 12 GB of VRAM or more";

/// The NVIDIA GPU Strata will run on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    pub index: u32,
    pub name: String,
    pub vram_mib: u64,
    pub compute_capability: (u32, u32),
    pub driver: String,
}

impl Gpu {
    fn driver_major(&self) -> u32 {
        self.driver
            .split('.')
            .next()
            .and_then(|major| major.trim().parse().ok())
            .unwrap_or(0)
    }
}

static DETECTED_GPU: std::sync::OnceLock<Gpu> = std::sync::OnceLock::new();

/// The GPU Strata would run on, for Setup's snapshot. The card, its memory, and its generation
/// cannot change while Windows runs, so the first answer is kept; a driver too old for Strata is
/// probed again each time so an update shows up without restarting Kestrel.
pub async fn detected_gpu() -> Option<Gpu> {
    if let Some(gpu) = DETECTED_GPU.get() {
        return Some(gpu.clone());
    }
    let gpu = probe_gpu().await?;
    if gpu.driver_major() >= MIN_DRIVER {
        let _ = DETECTED_GPU.set(gpu.clone());
    }
    Some(gpu)
}

/// Ask `nvidia-smi` for every NVIDIA GPU and pick the one Strata's setup picks.
pub async fn probe_gpu() -> Option<Gpu> {
    let mut command = tokio::process::Command::new("nvidia-smi.exe");
    command
        .args([
            "--query-gpu=index,name,memory.total,compute_cap,driver_version",
            "--format=csv,noheader,nounits",
        ])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    strata_gpu(parse_gpus(&String::from_utf8_lossy(&output.stdout)))
}

fn parse_gpus(output: &str) -> Vec<Gpu> {
    output
        .lines()
        .filter_map(|line| {
            let values = line.split(',').map(str::trim).collect::<Vec<_>>();
            let [index, name, memory, capability, driver] = values.as_slice() else {
                return None;
            };
            let (major, minor) = capability.split_once('.')?;
            Some(Gpu {
                index: index.parse().ok()?,
                name: (*name).to_string(),
                vram_mib: memory.parse().ok()?,
                compute_capability: (major.parse().ok()?, minor.parse().ok()?),
                driver: (*driver).to_string(),
            })
        })
        .collect()
}

/// Strata's own choice: the card with the most VRAM in whole GiB, the lower number on a tie.
fn strata_gpu(gpus: Vec<Gpu>) -> Option<Gpu> {
    gpus.into_iter().max_by_key(|gpu| {
        (
            (gpu.vram_mib as f64 / 1024.0).round() as u64,
            std::cmp::Reverse(gpu.index),
        )
    })
}

/// Whether this processor has AVX2, which Strata's engine needs for the experts it runs on the
/// CPU.
pub fn cpu_has_avx2() -> bool {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        std::arch::is_x86_feature_detected!("avx2")
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        false
    }
}

/// Why Kestrel will not offer or install Strata on this PC's graphics card and processor, or
/// `None` when they match the tested PC. Setup checks RAM separately for each model size.
pub fn unsupported_reason(gpu: Option<&Gpu>, avx2: bool) -> Option<String> {
    let Some(gpu) = gpu else {
        return Some(format!(
            "No NVIDIA graphics card was detected. Kestrel supports Strata only on {SUPPORTED_GPU}, the class it was tested on."
        ));
    };
    if gpu.compute_capability < MIN_COMPUTE_CAPABILITY {
        return Some(format!(
            "{} (compute capability {}.{}) is older than the RTX 50 series Kestrel tested Strata on. Kestrel supports Strata only on {SUPPORTED_GPU}.",
            gpu.name, gpu.compute_capability.0, gpu.compute_capability.1
        ));
    }
    if gpu.vram_mib < MIN_VRAM_MIB {
        return Some(format!(
            "{} has {:.1} GB of VRAM. Kestrel tested Strata with 12 GB and supports it only on {SUPPORTED_GPU}.",
            gpu.name,
            gpu.vram_mib as f64 / 1024.0
        ));
    }
    if gpu.driver_major() < MIN_DRIVER {
        return Some(format!(
            "The NVIDIA driver is version {}. Strata's engine needs driver {MIN_DRIVER} or newer: update it with the NVIDIA App, then check again.",
            gpu.driver
        ));
    }
    if !avx2 {
        return Some(
            "This processor has no AVX2, which Strata needs to run its experts from system RAM."
                .into(),
        );
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Fixture {
        _directory: tempfile::TempDir,
        root: PathBuf,
        config: PathBuf,
    }

    fn write_gguf(path: &Path, name: &str) {
        use std::io::Write as _;
        let mut file = fs::File::create(path).unwrap();
        file.write_all(b"GGUF").unwrap();
        file.write_all(&3u32.to_le_bytes()).unwrap();
        file.write_all(&0u64.to_le_bytes()).unwrap();
        let entries = [("general.architecture", "qwen4exp"), ("general.name", name)];
        file.write_all(&(entries.len() as u64).to_le_bytes())
            .unwrap();
        for (key, value) in entries {
            file.write_all(&(key.len() as u64).to_le_bytes()).unwrap();
            file.write_all(key.as_bytes()).unwrap();
            file.write_all(&8u32.to_le_bytes()).unwrap();
            file.write_all(&(value.len() as u64).to_le_bytes()).unwrap();
            file.write_all(value.as_bytes()).unwrap();
        }
        file.write_all(&[0u8; 4096]).unwrap();
    }

    fn install(extra: serde_json::Value) -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("Strata");
        let python = venv_python(&root);
        fs::create_dir_all(python.parent().unwrap()).unwrap();
        fs::write(&python, b"python").unwrap();
        fs::create_dir_all(root.join("serve")).unwrap();
        fs::write(root.join("serve").join("server.py"), b"server").unwrap();
        fs::create_dir_all(root.join("engine")).unwrap();
        let engine = root.join("engine").join(ENGINE_FILE);
        fs::write(&engine, b"engine").unwrap();
        let models = directory
            .path()
            .join("Strata-data")
            .join("models")
            .join("IQ3_S");
        fs::create_dir_all(&models).unwrap();
        let native = models.join("Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00001-of-00002.gguf");
        let ple = models.join("Qwen3.8-Flash-Next-GSQ-RCO-IQ3_S-00002-of-00002.gguf");
        write_gguf(&native, "Qwen3.8-Flash-Next GSQ-RCO");
        fs::write(&ple, b"shard two").unwrap();
        let tokenizer = directory
            .path()
            .join("Strata-data")
            .join("packs")
            .join("iq3_s")
            .join("tokenizer");
        fs::create_dir_all(&tokenizer).unwrap();
        fs::write(tokenizer.join("vocab.json"), b"{}").unwrap();
        let mut value = json!({
            "exe": engine,
            "args": ["--pack", "p", "--native", native, "--ple-gguf", ple, "--max-context", "65536", "--kv", "int8"],
            "cwd": root,
            "tokenizer": tokenizer,
            "model_name": "qwen3.8-flash-next-iq3_s",
            "port": 8080
        });
        if let serde_json::Value::Object(extra) = extra {
            value.as_object_mut().unwrap().extend(extra);
        }
        let config = root.join("strata-iq3_s.json");
        fs::write(&config, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        Fixture {
            _directory: directory,
            root,
            config,
        }
    }

    #[test]
    fn discovers_a_complete_install_with_its_fixed_context() {
        let fixture = install(json!({}));
        let models = discover(std::slice::from_ref(&fixture.root));
        assert_eq!(models.len(), 1);
        let model = &models[0];
        assert_eq!(model.engine, ModelEngine::Strata);
        assert_eq!(model.fixed_context_window, Some(65_536));
        assert_eq!(model.quantization.as_deref(), Some("IQ3_S"));
        assert_eq!(model.architecture.as_deref(), Some("qwen4exp"));
        assert_eq!(model.name, "Qwen3.8-Flash-Next GSQ-RCO IQ3_S (Strata)");
        assert_eq!(model.path, fixture.config.to_string_lossy());
        assert!(!model.supports_vision);
        assert_eq!(model.serving_context(98_304), 65_536);
        assert_eq!(model.serving_context(32_768), 32_768);
        assert_eq!(
            ready_install(std::slice::from_ref(&fixture.root)),
            Some(fixture.root.clone())
        );
    }

    #[test]
    fn identity_survives_config_rewrites_but_differs_from_the_llama_identity() {
        let fixture = install(json!({}));
        let first = discover(std::slice::from_ref(&fixture.root))[0].id.clone();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&fixture.config).unwrap()).unwrap();
        value["args"].as_array_mut().unwrap().push("--spec".into());
        fs::write(&fixture.config, serde_json::to_vec(&value).unwrap()).unwrap();
        let second = discover(std::slice::from_ref(&fixture.root))[0].id.clone();
        assert_eq!(first, second);
        let native = argument(
            &serde_json::from_value::<Vec<String>>(value["args"].clone()).unwrap(),
            "--native",
        )
        .map(PathBuf::from)
        .unwrap();
        let llama = model::content_identity(&native, fs::metadata(&native).unwrap().len()).unwrap();
        assert_ne!(first, llama);
    }

    #[test]
    fn launch_plan_is_loopback_and_keeps_the_key_off_the_command_line() {
        let fixture = install(json!({"api_key": "from-config", "host": "0.0.0.0"}));
        let plan = launch_plan(&fixture.config).unwrap();
        let args = plan.server_args(4321);
        assert_eq!(plan.context_window, 65_536);
        assert!(args.windows(2).any(|pair| pair == ["--host", "127.0.0.1"]));
        assert!(args.windows(2).any(|pair| pair == ["--port", "4321"]));
        assert!(args.contains(&"--fit-max-tokens".to_string()));
        assert!(!args
            .iter()
            .any(|value| value == "--api-key" || value == "--open"));
    }

    #[test]
    fn tool_server_configurations_are_refused() {
        let fixture = install(json!({"mcp_servers": {"files": {"command": "npx"}}}));
        let error = launch_plan(&fixture.config).unwrap_err();
        assert!(error.contains("MCP"), "{error}");
        assert!(discover(std::slice::from_ref(&fixture.root)).is_empty());
        let empty = install(json!({"mcp_servers": {}}));
        assert!(launch_plan(&empty.config).is_ok());
    }

    #[test]
    fn missing_pieces_are_named_instead_of_listed() {
        let fixture = install(json!({}));
        fs::remove_file(venv_python(&fixture.root)).unwrap();
        let error = launch_plan(&fixture.config).unwrap_err();
        assert!(error.contains("Python environment"), "{error}");
        assert!(discover(std::slice::from_ref(&fixture.root)).is_empty());
        assert_eq!(ready_install(std::slice::from_ref(&fixture.root)), None);
    }

    #[test]
    fn image_configurations_need_every_vision_file() {
        for off in [json!(null), json!({}), json!(false)] {
            let fixture = install(json!({ "vision": off }));
            assert!(launch_plan(&fixture.config).is_ok());
            assert!(!discover(std::slice::from_ref(&fixture.root))[0].supports_vision);
        }

        let fixture = install(json!({}));
        let engine = fixture.root.join("engine");
        let vision_exe = engine.join(VISION_FILE);
        let mmproj = engine.join("mmproj-Qwen3.8-Flash-Next-BF16.gguf");
        fs::write(&vision_exe, b"vision").unwrap();
        fs::write(&mmproj, b"projector").unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&fixture.config).unwrap()).unwrap();
        let native = argument(
            &serde_json::from_value::<Vec<String>>(value["args"].clone()).unwrap(),
            "--native",
        )
        .unwrap()
        .to_string();
        value["vision"] =
            json!({"exe": vision_exe, "mmproj": mmproj, "model": native, "gpu": true});
        fs::write(&fixture.config, serde_json::to_vec(&value).unwrap()).unwrap();
        let model = &discover(std::slice::from_ref(&fixture.root))[0];
        assert!(model.supports_vision);
        assert_eq!(
            model.mmproj_path.as_deref(),
            Some(mmproj.to_string_lossy().as_ref())
        );

        fs::remove_file(&mmproj).unwrap();
        let error = launch_plan(&fixture.config).unwrap_err();
        assert!(error.contains("image projector"), "{error}");
        assert!(discover(std::slice::from_ref(&fixture.root)).is_empty());
        assert_eq!(ready_install(std::slice::from_ref(&fixture.root)), None);

        fs::write(&mmproj, b"projector").unwrap();
        fs::remove_file(&vision_exe).unwrap();
        let error = launch_plan(&fixture.config).unwrap_err();
        assert!(error.contains(VISION_FILE), "{error}");
    }

    fn gpu(name: &str, vram_mib: u64, capability: (u32, u32), driver: &str) -> Gpu {
        Gpu {
            index: 0,
            name: name.into(),
            vram_mib,
            compute_capability: capability,
            driver: driver.into(),
        }
    }

    #[test]
    fn strata_runs_on_the_card_with_the_most_memory() {
        let gpus = parse_gpus(
            "0, NVIDIA GeForce RTX 5070, 12227, 12.0, 610.74\n\
             1, NVIDIA GeForce RTX 3090, 24576, 8.6, 610.74\n\
             garbage line\n",
        );
        assert_eq!(gpus.len(), 2);
        assert_eq!(
            gpus[0],
            gpu("NVIDIA GeForce RTX 5070", 12_227, (12, 0), "610.74")
        );
        assert_eq!(strata_gpu(gpus).unwrap().name, "NVIDIA GeForce RTX 3090");
        let tie = parse_gpus(
            "0, NVIDIA GeForce RTX 5070 Ti, 16303, 12.0, 610.74\n1, NVIDIA GeForce RTX 5080, 16303, 12.0, 610.74",
        );
        assert_eq!(strata_gpu(tie).unwrap().index, 0);
    }

    #[test]
    fn only_the_tested_class_of_pc_is_supported() {
        let tested = gpu("NVIDIA GeForce RTX 5070", 12_227, (12, 0), "610.74");
        assert_eq!(unsupported_reason(Some(&tested), true), None);
        let larger = gpu("NVIDIA GeForce RTX 5090", 32_607, (12, 0), "610.74");
        assert_eq!(unsupported_reason(Some(&larger), true), None);

        let refused = [
            (None, true, "No NVIDIA"),
            (
                Some(gpu("NVIDIA GeForce RTX 4090", 24_564, (8, 9), "610.74")),
                true,
                "older than the RTX 50",
            ),
            (
                Some(gpu("NVIDIA GeForce RTX 5060", 8_151, (12, 0), "610.74")),
                true,
                "8.0 GB of VRAM",
            ),
            (
                Some(gpu("NVIDIA GeForce RTX 5070", 12_227, (12, 0), "576.02")),
                true,
                "update it",
            ),
            (Some(tested), false, "AVX2"),
        ];
        for (gpu, avx2, expected) in refused {
            let reason = unsupported_reason(gpu.as_ref(), avx2).unwrap();
            assert!(reason.contains(expected), "{reason}");
        }
    }

    #[test]
    fn server_settings_files_are_not_run_configurations() {
        let fixture = install(json!({}));
        fs::write(
            fixture.root.join("strata-iq3_s.shared-settings.json"),
            br#"{"reasoning_effort":"high"}"#,
        )
        .unwrap();
        assert_eq!(run_configs(&fixture.root), vec![fixture.config.clone()]);
    }

    #[test]
    fn recorded_installs_are_bounded_and_absolute_only() {
        let fixture = install(json!({}));
        let directory = tempfile::tempdir().unwrap();
        let settings = directory.path().join("settings.json");
        fs::write(
            &settings,
            serde_json::to_vec(
                &json!({"data_dir": "x", "installs": [fixture.root, "relative\\Strata"]}),
            )
            .unwrap(),
        )
        .unwrap();
        let recorded = recorded_installs(&settings);
        assert_eq!(recorded.len(), 2);
        fs::write(&settings, vec![b' '; (MAX_SETTINGS_BYTES + 1) as usize]).unwrap();
        assert!(recorded_installs(&settings).is_empty());
        let configured = fixture.root.to_string_lossy().into_owned();
        let installs = known_installs(&[configured.as_str(), "relative\\Strata"]);
        assert_eq!(installs.first(), Some(&fixture.root));
        assert!(installs.iter().all(|path| path.is_absolute()));
    }

    #[cfg(windows)]
    fn process_exists(pid: u32) -> bool {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\"")))
            .unwrap_or(false)
    }

    /// Strata's server starts `strata.exe` itself; stopping the job must stop that grandchild.
    #[cfg(windows)]
    #[tokio::test]
    async fn dropping_the_job_stops_processes_the_server_started() {
        use tokio::io::AsyncBufReadExt;
        use windows_sys::Win32::System::JobObjects::IsProcessInJob;
        let mut command = tokio::process::Command::new("powershell.exe");
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "$p = Start-Process ping.exe -ArgumentList '-n','120','127.0.0.1' -PassThru -WindowStyle Hidden; Write-Output $p.Id; Start-Sleep -Seconds 120",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let (mut child, job) = ProcessJob::spawn(&mut command).unwrap();
        let mut in_job = 0;
        // SAFETY: both handles stay open for the call.
        let checked =
            unsafe { IsProcessInJob(child.raw_handle().unwrap().cast(), job.0, &mut in_job) };
        assert!(checked != 0 && in_job != 0, "the server was not in its job");
        let mut lines = tokio::io::BufReader::new(child.stdout.take().unwrap()).lines();
        let grandchild: u32 =
            tokio::time::timeout(std::time::Duration::from_secs(60), lines.next_line())
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .trim()
                .parse()
                .unwrap();
        assert!(process_exists(grandchild));

        drop(job);

        let mut stopped = false;
        for _ in 0..50 {
            if !process_exists(grandchild) {
                stopped = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        assert!(stopped, "the grandchild outlived its job");
        // A job closed with KILL_ON_JOB_CLOSE ends its members with exit code 0; what matters is
        // that the server itself is gone too.
        tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
            .await
            .expect("the server outlived its job")
            .unwrap();
    }

    #[test]
    fn quantization_comes_from_the_shard_name() {
        assert_eq!(
            quantization_from_file(Path::new(
                "Qwen3.8-Flash-Next-GSQ-RCO-IQ2_XS-00001-of-00002.gguf"
            ))
            .as_deref(),
            Some("IQ2_XS")
        );
        assert_eq!(
            quantization_from_file(Path::new(
                "Swift-Qwen3.8-Flash-Next-GSQ-RCO-Q2_0-00001-of-00002.gguf"
            ))
            .as_deref(),
            Some("Q2_0")
        );
    }
}
