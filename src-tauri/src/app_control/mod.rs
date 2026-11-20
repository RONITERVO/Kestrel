//! Shared semantic producer actions. No model provider or network transport lives here.
use crate::{describe_app_command, dispatch_app_command, AppState, APP_COMMAND_NAMES};
use kestrel_app_core::{
    AppActionDescription, AppActionReceipt, AppActionRequest, AppJobObservation,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex as AsyncMutex;

pub mod transport;
mod workflow;
pub use workflow::*;

pub type TypeDescription = (Value, String, BTreeMap<String, String>);
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
tokio::task_local! { pub static TASK_ACTION: bool; }
tokio::task_local! { pub static TASK_WORKSPACE: Vec<String>; }

pub struct AppControl {
    pub root: PathBuf,
    pub serial: AsyncMutex<()>,
    pub task_active: AtomicBool,
    pub task_progress: Mutex<Option<(String, String, String)>>,
    pub workflows: Mutex<HashMap<String, tokio_util::sync::CancellationToken>>,
    jobs: Mutex<HashMap<String, AppJobObservation>>,
}

impl AppControl {
    pub fn new(root: &Path) -> Result<Self, String> {
        let root = root.join("producer");
        for folder in ["actions", "jobs", "workflows"] {
            fs::create_dir_all(root.join(folder)).map_err(|e| e.to_string())?;
        }
        let control = Self {
            root,
            serial: AsyncMutex::new(()),
            task_active: AtomicBool::new(false),
            task_progress: Mutex::new(None),
            workflows: Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
        };
        for path in records(&control.root.join("actions"))? {
            let mut receipt: AppActionReceipt = read(&path)?;
            if receipt.status == "running" {
                receipt.status = "interrupted".into();
                receipt.error = Some("The app stopped before this action's outcome was recorded. Inspect its project before intentionally issuing a new request ID; this action will never replay automatically.".into());
                write(&path, &receipt)?;
            }
        }
        control.recover_workflows()?;
        for path in records(&control.root.join("jobs"))? {
            let mut job: AppJobObservation = read(&path)?;
            if !terminal(&job.status) {
                job.status = "interrupted".into();
                write(&path, &job)?;
            }
        }
        Ok(control)
    }

    pub fn receipt(&self, id: &str) -> Result<AppActionReceipt, String> {
        read(&self.path("actions", id)?)
    }
    pub fn job(&self, id: &str) -> Result<AppJobObservation, String> {
        valid_id(id)?;
        if let Some(value) = self.jobs.lock().map_err(|e| e.to_string())?.get(id) {
            return Ok(value.clone());
        }
        read(&self.path("jobs", id)?)
    }
    pub fn require_new_job(&self, id: &str) -> Result<(), String> {
        let path = self.path("jobs", id)?;
        if path.exists()
            || path.with_extension("json.backup").exists()
            || self
                .jobs
                .lock()
                .map_err(|e| e.to_string())?
                .contains_key(id)
        {
            return Err("That draft request ID already exists. Read get_app_job for its preserved result, or use a new ID for an intentional new draft.".into());
        }
        Ok(())
    }
    pub fn path(&self, folder: &str, id: &str) -> Result<PathBuf, String> {
        valid_id(id)?;
        Ok(self.root.join(folder).join(format!("{id}.json")))
    }

    /// Streaming drafts are producer data, available after the originating window disconnects.
    pub fn observe(&self, event: &str, value: Value) -> Result<(), String> {
        if self.task_active.load(Ordering::Acquire)
            && matches!(
                event,
                "research-progress"
                    | "image-generation"
                    | "music-generation"
                    | "movie-project"
                    | "movie-render-preview"
                    | "runtime-progress"
                    | "local-speech-progress"
                    | "media-composition"
            )
        {
            let mut title = value
                .get("title")
                .or_else(|| value.get("phase"))
                .or_else(|| value.get("stage"))
                .and_then(Value::as_str)
                .unwrap_or(event)
                .to_string();
            if event == "movie-render-preview" {
                if let Some(index) = value.get("clipIndex").and_then(Value::as_u64) {
                    title = format!("Rendering video scene {}", index.saturating_add(1));
                }
            }
            if event == "local-speech-progress" {
                title = match title.as_str() {
                    "generating" => "Generating narration".into(),
                    "aligning" => "Timing the narration".into(),
                    _ => title,
                };
            }
            let mut detail: String = value
                .get("detail")
                .and_then(Value::as_str)
                .unwrap_or("")
                .chars()
                .take(1800)
                .collect();
            if let (Some(step), Some(total)) = (
                value.get("step").and_then(Value::as_u64),
                value.get("total").and_then(Value::as_u64),
            ) {
                detail = format!("Step {step} of {total}. {detail}");
            }
            *self.task_progress.lock().map_err(|e| e.to_string())? = Some((title, detail, now()));
        }
        if event != "studio-prompt-draft" {
            return Ok(());
        }
        let Some(id) = value.get("requestId").and_then(Value::as_str) else {
            return Ok(());
        };
        let path = self.path("jobs", id)?;
        let mut jobs = self.jobs.lock().map_err(|e| e.to_string())?;
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("running");
        let job = jobs.entry(id.into()).or_insert_with(|| AppJobObservation {
            id: id.into(),
            event: event.into(),
            status: "running".into(),
            text: String::new(),
            latest: Value::Null,
            provenance: Value::Null,
            updated_at: now(),
        });
        if let Some(receipt) = value.get("receipt").filter(|v| !v.is_null()) {
            job.provenance = receipt.clone();
        }
        let before = job.text.len();
        if kind == "token" {
            if let Some(text) = value.get("content").and_then(Value::as_str) {
                if job.text.len() + text.len() > MAX_BYTES {
                    return Err("Draft observation exceeds 8 MiB.".into());
                }
                job.text.push_str(text);
            }
        }
        if kind != "reasoning" && kind != "token" && kind != "settled" {
            job.latest = value.clone();
        }
        if terminal(kind) {
            job.status = kind.into();
        }
        job.updated_at = now();
        if terminal(kind) || kind == "queued" || before / 4096 != job.text.len() / 4096 {
            write(&path, job)?;
        }
        if jobs.len() > 128 {
            // Durable completed observations remain addressable on disk.
            let remove = jobs
                .iter()
                .find(|(key, job)| key.as_str() != id && terminal(&job.status))
                .map(|(key, _)| key.clone());
            if let Some(key) = remove {
                jobs.remove(&key);
            }
        }
        Ok(())
    }
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub fn terminal(status: &str) -> bool {
    matches!(
        status,
        "complete" | "completed" | "failed" | "error" | "cancelled" | "interrupted" | "limited"
    )
}
pub fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 120
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("IDs must contain 1–120 ASCII letters, digits, underscores or hyphens.".into());
    }
    Ok(())
}
pub fn read<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let attempt = |path: &Path| -> Result<T, String> {
        if fs::metadata(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .len()
            > MAX_BYTES as u64
        {
            return Err("Producer record exceeds 8 MiB.".into());
        }
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    attempt(path)
        .or_else(|original| attempt(&path.with_extension("json.backup")).map_err(|_| original))
}
pub fn write<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    if data.len() > MAX_BYTES {
        return Err("Producer record exceeds 8 MiB; split the workflow into smaller runs.".into());
    }
    crate::config::atomic_json_write(path, &data).map_err(|e| e.to_string())
}
fn records(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = std::collections::BTreeSet::new();
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            paths.insert(path);
        } else if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().ends_with(".json.backup"))
        {
            // A process can stop between removing the old primary and renaming its replacement.
            paths.insert(path.with_extension(""));
        }
        if paths.len() > 100_000 {
            return Err("Producer library exceeds the bounded 100,000-record scan.".into());
        }
    }
    Ok(paths.into_iter().collect())
}

pub fn describe_type<T: ts_rs::TS + 'static + ?Sized>(types: &mut BTreeMap<String, String>) {
    struct Visitor<'a>(&'a mut BTreeMap<String, String>);
    impl ts_rs::TypeVisitor for Visitor<'_> {
        fn visit<T: ts_rs::TS + 'static + ?Sized>(&mut self) {
            describe_type::<T>(self.0);
        }
    }
    let cfg = ts_rs::Config::from_env();
    let name = T::name(&cfg);
    if types.contains_key(&name) {
        return;
    }
    types.insert(name, T::inline(&cfg));
    T::visit_dependencies(&mut Visitor(types));
    T::visit_generics(&mut Visitor(types));
}
pub fn validate_keys(args: &Value, keys: &[&str]) -> Result<(), String> {
    let object = args.as_object().ok_or("Command args must be an object.")?;
    if let Some(key) = object.keys().find(|key| !keys.contains(&key.as_str())) {
        return Err(format!(
            "Unknown command argument: {key}. Use describe_app_action for the exact contract."
        ));
    }
    Ok(())
}

pub fn import_paths(paths: Vec<String>) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() || paths.len() > 32 {
        return Err("Import between 1 and 32 explicit local paths.".into());
    }
    paths
        .into_iter()
        .map(|path| {
            if !Path::new(&path).is_absolute() || path.contains(['*', '?']) {
                return Err("Import paths must be absolute, without wildcards.".into());
            }
            PathBuf::from(path)
                .canonicalize()
                .map_err(|e| e.to_string())
        })
        .collect()
}

pub fn export_path(value: &str, extension: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if !path.is_absolute()
        || value.contains(['*', '?'])
        || !path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
    {
        return Err(format!(
            "Supply an explicit absolute .{extension} destinationPath without wildcards."
        ));
    }
    if path.exists() {
        return Err("The export destination already exists. Choose a new filename; agent exports never overwrite user files.".into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// An explicit reviewed disposition for EVERY registered desktop command; additions fail tests
/// until assigned. Human-only actions remain discoverable with their reason and alternative.
pub fn policy(name: &str) -> Result<(String, String), String> {
    let policies: BTreeMap<String, (String, String)> =
        serde_json::from_str(include_str!("policies.json")).map_err(|e| e.to_string())?;
    policies
        .get(name)
        .cloned()
        .ok_or_else(|| format!("Command {name} has no reviewed producer policy."))
}
pub fn catalog(filter: Option<&str>) -> Vec<String> {
    APP_COMMAND_NAMES
        .iter()
        .filter(|name| filter.is_none_or(|filter| name.contains(filter)))
        .map(|name| (*name).into())
        .collect()
}
const H3_WORKFLOW_OVERVIEW: &str = " H3 workflow: track state via get_movie_h3_digest ({id}, <4 KB), not get_movie. Build reusable @Handle entities (upsert_movie_h3_entity + import_*_as_movie_reference), stage @Handle spans across active [startSec..endSec] lanes (stage_movie_h3_reference), author 4s-6s [Shot k] beats (upsert_movie_h3_prompt_section), dry-run compile_movie_h3_window (caps: 9 pics, 3 vids, 3 audios per window), and render draft02Mp (768x448, expandQualityLadder:false) sequentially or with windows:[] (auto 23-frame preContextSec continuity bridge). Place scores on A2 with place_movie_h3_take (gain/fades), verify takes via inspect_movie_h3_take, render timeline via render_movie_edit, and bundle outputs with export_production_bundle.";

fn needs_h3_workflow_overview(name: &str) -> bool {
    name.contains("movie_h3")
        || matches!(
            name,
            "create_movie_producer_project"
                | "render_movie_edit"
                | "export_production_bundle"
                | "export_media_composition"
        )
}

pub fn describe(name: &str) -> Result<AppActionDescription, String> {
    let (access, mut guidance) = policy(name)?;
    if name == "apply_image_composition" {
        // The action accepts a string containing another format. Describe that format from
        // the same literal schema used by native drafting; editor state is a different type.
        guidance.push_str(" The text argument contains JSON matching this schema, not ImageProject editor fields. Preserve the listed key order; bbox is [top,left,bottom,right] in 0–1000 coordinates. Schema: ");
        guidance.push_str(crate::studio::IMAGE_COMPOSITION_SCHEMA.text);
    } else if needs_h3_workflow_overview(name) {
        guidance.push_str(H3_WORKFLOW_OVERVIEW);
    }
    let (arguments, result_type, types) = describe_app_command(name)?;
    if types.len() > 8 {
        if let Some(arr) = arguments.as_array() {
            for arg in arr {
                if let Some(arg_type) = arg.get("type").and_then(Value::as_str) {
                    if let Some(inline_def) = types.get(arg_type) {
                        guidance.push_str(&format!(" Input contract ({arg_type}): {inline_def}"));
                    }
                }
            }
        }
    }
    Ok(AppActionDescription {
        command: name.into(),
        access,
        guidance,
        arguments,
        result_type,
        types,
    })
}

/// Local models sometimes JSON-encode a large nested `args` object into a string or drop 1–3
/// trailing closing braces on deeply nested payloads. When that string decodes into one JSON
/// object, use the object; any other string stays a string and is rejected by the normal check.
pub fn coerce_stringified_object(value: Value) -> Value {
    let Value::String(text) = &value else {
        return value;
    };
    let trimmed = text.trim();
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(trimmed) {
        return Value::Object(map);
    }
    if trimmed.starts_with('{') {
        for suffix in ["}", "}}", "}}}"] {
            let candidate = format!("{trimmed}{suffix}");
            if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&candidate) {
                return Value::Object(map);
            }
        }
    }
    value
}

pub fn normalize_action_args(command: &str, mut args: Value) -> Value {
    if let Ok((arguments, _, _)) = describe_app_command(command) {
        if let Some(arr) = arguments.as_array() {
            if arr.len() == 1 {
                if let Some(key) = arr[0].get("name").and_then(Value::as_str) {
                    if key == "id" {
                        if let Some(obj) = args.as_object() {
                            if obj.len() == 1 && !obj.contains_key("id") {
                                if let Some(val) = [
                                    "projectId",
                                    "movieId",
                                    "musicProjectId",
                                    "imageProjectId",
                                    "jobId",
                                    "reportId",
                                    "bundleId",
                                ]
                                .into_iter()
                                .find_map(|alias| obj.get(alias))
                                {
                                    args = json!({ "id": val.clone() });
                                }
                            }
                        }
                    } else if matches!(
                        key,
                        "request" | "edited" | "project" | "settings" | "draft"
                    ) {
                        if let Some(obj) = args.as_object() {
                            if !obj.contains_key(key) && !obj.is_empty() {
                                args = json!({ key: args });
                            }
                        }
                    }
                }
            }
        }
    }
    args
}

pub async fn execute(
    app: &AppHandle,
    mut request: AppActionRequest,
) -> Result<AppActionReceipt, String> {
    valid_id(&request.request_id)?;
    request.args = coerce_stringified_object(std::mem::take(&mut request.args));
    request.args = normalize_action_args(&request.command, std::mem::take(&mut request.args));
    if matches!(
        request.command.as_str(),
        "export_narration" | "export_music_lyrics" | "export_music_midi"
    ) && !request
        .args
        .get("destinationPath")
        .is_some_and(Value::is_string)
    {
        return Err("Agent exports require an explicit destinationPath; the desktop save dialog cannot run unattended.".into());
    }
    let (access, guidance) = policy(&request.command)?;
    if access == "human" || access == "protocol" {
        return Err(format!(
            "{} requires its explicit native user action. {guidance}",
            request.command
        ));
    }
    if !request.args.is_object() {
        return Err("Action args must be a JSON object, not a JSON-encoded string or array. Pass the described fields directly inside args.".into());
    }
    if request.args.to_string().len() > MAX_BYTES / 2 {
        return Err("Action args exceed 4 MiB; split the work into smaller actions.".into());
    }
    let state = app.state::<AppState>();
    let serial = state.control.serial.lock().await;
    let path = state.control.path("actions", &request.request_id)?;
    // Read actions have no side effects, so a reused request ID re-reads current state.
    // Replaying an old receipt here showed a long-running producer stale project data.
    if access != "read" && (path.exists() || path.with_extension("json.backup").exists()) {
        let prior: AppActionReceipt = read(&path)?;
        if prior.request.command != request.command || prior.request.args != request.args {
            return Err("Retry conflict: that request ID belongs to different arguments. Use a new ID for a new intentional action.".into());
        }
        return Ok(prior);
    }
    let mut receipt = AppActionReceipt {
        request,
        status: "running".into(),
        result: None,
        error: None,
        started_at: now(),
        finished_at: None,
    };
    write(&path, &receipt)?;
    // Reserve the ID atomically, then let the normal native work gates arbitrate execution.
    // Holding the journal lock during inference would prevent cancellation and observation.
    drop(serial);
    match dispatch_app_command(app, &receipt.request.command, receipt.request.args.clone()).await {
        Ok(result) => {
            receipt.result = Some(result);
            receipt.status = "completed".into();
        }
        Err(error) => {
            receipt.error = Some(error);
            receipt.status = "failed".into();
        }
    }
    receipt.finished_at = Some(now());
    write(&path, &receipt)?;
    Ok(receipt)
}

pub fn ensure_task_access(state: &AppState) -> Result<(), String> {
    if state.control.task_active.load(Ordering::Acquire)
        && !TASK_ACTION.try_with(|allowed| *allowed).unwrap_or(false)
    {
        Err("A Computer Task owns this producer workflow. Stop or finish it before starting other work.".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_desktop_action_has_an_explicit_policy_and_contract() {
        let policies: BTreeMap<String, Value> =
            serde_json::from_str(include_str!("policies.json")).unwrap();
        assert_eq!(policies.len(), APP_COMMAND_NAMES.len());
        for name in APP_COMMAND_NAMES {
            let desc = describe(name).unwrap();
            assert!(!desc.guidance.is_empty(), "{name}");
            assert!(!desc.result_type.is_empty(), "{name}");
        }
        let result = describe("export_production_bundle").unwrap();
        assert_eq!(result.result_type, "ProductionBundle");
        assert!(result.types.contains_key("ProductionBundle"));
        assert!(result.types.contains_key("ProductionArtifact"));
        assert_eq!(policy("run_codex_repair").unwrap().0, "human");
        assert_eq!(policy("save_control_settings").unwrap().0, "human");
    }
    #[test]
    fn ids_and_top_level_arguments_fail_closed() {
        for id in ["../escape", "C:\\escape", "", "a/b", "a?b"] {
            assert!(valid_id(id).is_err());
        }
        assert!(validate_keys(&json!({"id":"ok","typo":true}), &["id"]).is_err());
    }
    #[test]
    fn stringified_object_args_are_decoded_and_other_strings_are_kept() {
        let nested = json!({"request": {"scene": {"audioLanes": [{"clips": [{"gain": 0.32}]}]}}});
        let decoded = coerce_stringified_object(Value::String(format!(" {nested} ")));
        assert_eq!(decoded, nested);
        let one_missing = coerce_stringified_object(json!(
            "{\"request\":{\"projectId\":\"mov-1\",\"title\":\"Aurora Watch\""
        ));
        assert_eq!(
            one_missing,
            json!({"request": {"projectId": "mov-1", "title": "Aurora Watch"}})
        );
        let two_missing = coerce_stringified_object(json!(
            "{\"request\":{\"scene\":{\"id\":\"scene-1\",\"name\":\"Main\"}"
        ));
        assert_eq!(
            two_missing,
            json!({"request": {"scene": {"id": "scene-1", "name": "Main"}}})
        );
        let three_missing = coerce_stringified_object(json!(
            "{\"request\":{\"scene\":{\"id\":\"scene-1\",\"name\":\"Main\""
        ));
        assert_eq!(
            three_missing,
            json!({"request": {"scene": {"id": "scene-1", "name": "Main"}}})
        );
        for kept in [
            json!("plain text"),
            json!("[1,2]"),
            json!("\"quoted\""),
            json!(3),
            json!("{\"unclosed\":\"value"),
        ] {
            assert_eq!(coerce_stringified_object(kept.clone()), kept);
        }
        assert_eq!(coerce_stringified_object(nested.clone()), nested);

        let h3_desc = describe("create_movie_h3_project").unwrap();
        assert!(h3_desc.guidance.contains("get_movie_h3_digest"));
        let place_desc = describe("place_movie_h3_take").unwrap();
        assert!(place_desc
            .guidance
            .contains("Input contract (PlaceMovieH3TakeRequest):"));
        assert_eq!(
            normalize_action_args("get_movie_h3_digest", json!({"projectId": "mov-1"})),
            json!({"id": "mov-1"})
        );
        assert_eq!(
            normalize_action_args("get_music_project", json!({"musicProjectId": "mus-1"})),
            json!({"id": "mus-1"})
        );
        assert_eq!(
            normalize_action_args(
                "place_movie_h3_take",
                json!({"projectId": "mov-1", "jobId": "job-1"})
            ),
            json!({"request": {"projectId": "mov-1", "jobId": "job-1"}})
        );
        let non_h3_desc = describe("get_setup_snapshot").unwrap();
        assert!(!non_h3_desc.guidance.contains("get_movie_h3_digest"));
    }
    #[test]
    fn unfinished_actions_and_drafts_recover_without_replay() {
        let temp = tempfile::tempdir().unwrap();
        let control = AppControl::new(temp.path()).unwrap();
        let receipt = AppActionReceipt {
            request: AppActionRequest {
                request_id: "one".into(),
                command: "create_music_project".into(),
                args: json!({}),
            },
            status: "running".into(),
            result: None,
            error: None,
            started_at: now(),
            finished_at: None,
        };
        write(&control.path("actions", "one").unwrap(), &receipt).unwrap();
        fs::remove_file(control.path("actions", "one").unwrap()).unwrap();
        control
            .observe(
                "studio-prompt-draft",
                json!({"requestId":"draft","kind":"queued"}),
            )
            .unwrap();
        let recovered = AppControl::new(temp.path()).unwrap();
        assert_eq!(recovered.receipt("one").unwrap().status, "interrupted");
        assert_eq!(recovered.job("draft").unwrap().status, "interrupted");
    }
}
