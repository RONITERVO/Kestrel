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
//! configuration is listed only when its engine, model shards, tokenizer, Python environment, and
//! server script are present and it declares no MCP servers, because Kestrel chat is tool-free.

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
    vision: Option<VisionConfig>,
    #[serde(default)]
    mcp_servers: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct VisionConfig {
    #[serde(default)]
    mmproj: String,
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
    let mmproj = parsed
        .vision
        .as_ref()
        .map(|vision| PathBuf::from(&vision.mmproj))
        .filter(|path| path.is_file());
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
    /// Contain a just-spawned server before it can start its engine. Child processes created
    /// afterwards join the job automatically.
    pub fn contain(child: &tokio::process::Child) -> io::Result<Self> {
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        let process = child.raw_handle().ok_or_else(|| {
            io::Error::other("the Strata server exited before Kestrel could own it")
        })?;
        // SAFETY: plain Win32 calls on handles owned here; the job handle is closed on drop.
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
            if AssignProcessToJobObject(job.0, process.cast()) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }
    }
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
        let mut child = tokio::process::Command::new("powershell.exe")
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
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let job = ProcessJob::contain(&child).unwrap();
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
