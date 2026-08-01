#![recursion_limit = "256"]

mod agent;
mod attachments;
mod chat;
mod config;
mod developer;
mod guardian;
mod harness;
mod html;
mod kiwix;
mod model;
mod models;
mod profile;
mod runtime;
mod services;
mod store;
mod workspace;

use attachments::{AttachmentStore, ContextAttachment};
use config::{ControlSettingsStore, SettingsStore};
use developer::DeveloperAssistant;
use harness::ResearchHarness;
use model::{default_roots, merge_catalogs, ModelCatalogStore, ModelInfo};
use models::{
    AppSnapshot, ChatSession, ChatSessionSummary, ChatStart, ComputerTaskAccess,
    ComputerTaskApprovalDecision, ComputerTaskRequest, ComputerTaskRun, ComputerTaskSummary,
    ControlSettings, ControlSnapshot, DeveloperRepairReport, DeveloperRepairRequest,
    PendingComputerAction, ProfileTransfer, ResearchReport, ResearchSettings,
    ResolveComputerTaskApprovalRequest, ResumeComputerTaskRequest, RunResearchRequest,
    StartChatRequest, SystemSnapshot,
};
use runtime::RuntimeManager;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use store::ResearchStore;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use workspace::WorkspaceStore;

/// Shared native state. Keep authority visibly separated: research owns evidence/storage, runtime
/// owns the only model process, and developer owns the optional Codex child.
struct AppState {
    store: ResearchStore,
    harness: ResearchHarness,
    research_settings: SettingsStore,
    control_settings: ControlSettingsStore,
    model_catalog: ModelCatalogStore,
    models: RwLock<Vec<ModelInfo>>,
    engine_candidates: RwLock<Vec<models::EngineCandidate>>,
    runtime: Arc<RuntimeManager>,
    developer: DeveloperAssistant,
    workspace: WorkspaceStore,
    attachments: AttachmentStore,
    research_active: AtomicBool,
    work_active: AtomicBool,
    jobs: Mutex<HashMap<String, CancellationToken>>,
    interactive_jobs: Mutex<HashMap<String, CancellationToken>>,
}

struct ResearchGuard<'a> {
    research: &'a AtomicBool,
    work: &'a AtomicBool,
}

impl Drop for ResearchGuard<'_> {
    fn drop(&mut self) {
        self.research.store(false, Ordering::Release);
        self.work.store(false, Ordering::Release);
    }
}

struct WorkGuard<'a>(&'a AtomicBool);

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ContextAttachmentImport {
    attachments: Vec<ContextAttachment>,
    failures: Vec<String>,
}

impl Drop for WorkGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[tauri::command]
async fn bootstrap(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    if state.runtime.snapshot().await.phase == "stopped" {
        let settings = state
            .research_settings
            .load()
            .map_err(|error| error.to_string())?;
        let _ = state.runtime.attach_external_if_ready(&settings).await;
        identify_attached_bonsai(&state).await;
    }
    snapshot(&state).await
}

#[tauri::command]
async fn get_report(id: String, state: State<'_, AppState>) -> Result<ResearchReport, String> {
    state.store.get(&id).map_err(|error| error.to_string())
}

#[tauri::command]
async fn run_research(
    request: RunResearchRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ResearchReport, String> {
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Chat, research, or a computer task is already active.".to_string())?;
    state
        .research_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| {
            state.work_active.store(false, Ordering::Release);
            "Another offline research job is already active.".to_string()
        })?;
    let _guard = ResearchGuard {
        research: &state.research_active,
        work: &state.work_active,
    };
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let lease = state
        .runtime
        .lease_research(&settings)
        .await
        .map_err(|error| error.to_string())?;
    let job_id = uuid::Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    state
        .jobs
        .lock()
        .map_err(|_| "research job registry is unavailable".to_string())?
        .insert(job_id.clone(), cancel.clone());
    let result = state
        .harness
        .run(
            Some(&app),
            request,
            settings,
            &lease.connection,
            &job_id,
            cancel,
        )
        .await
        .map_err(|error| error.to_string());
    if let Ok(mut jobs) = state.jobs.lock() {
        jobs.remove(&job_id);
    }
    result
}

#[tauri::command]
fn cancel_research(job_id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state
        .jobs
        .lock()
        .map_err(|_| "research job registry is unavailable".to_string())?
        .get(&job_id)
    {
        cancel.cancel();
    }
    Ok(())
}

#[tauri::command]
async fn prepare_services(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let _guard = claim_workspace(&state)?;
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    services::prepare_with_root(&settings.bonsai_root)
        .await
        .map_err(|error| error.to_string())?;
    let _ = state.runtime.attach_external_if_ready(&settings).await;
    identify_attached_bonsai(&state).await;
    snapshot(&state).await
}

#[tauri::command]
async fn get_system_snapshot(state: State<'_, AppState>) -> Result<SystemSnapshot, String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    Ok(services::system_snapshot(settings).await)
}

#[tauri::command]
async fn get_control_snapshot(
    probe_developer: Option<bool>,
    state: State<'_, AppState>,
) -> Result<ControlSnapshot, String> {
    control_snapshot(&state, probe_developer.unwrap_or(true)).await
}

#[tauri::command]
async fn scan_local_models(state: State<'_, AppState>) -> Result<ControlSnapshot, String> {
    ensure_workspace_idle(&state)?;
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let roots = default_roots(&control.extra_model_roots, &research.bonsai_root);
    let found = tokio::task::spawn_blocking(move || model::scan(&roots))
        .await
        .map_err(|error| format!("model scan failed: {error}"))?;
    if let Err(error) = state.model_catalog.save(&found) {
        eprintln!(
            "Kestrel model scan completed, but its disposable catalog could not be saved: {error}"
        );
    }
    *state.models.write().await = found;
    control_snapshot(&state, true).await
}

#[tauri::command]
async fn export_setup_profile(state: State<'_, AppState>) -> Result<ProfileTransfer, String> {
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let models = state.models.read().await.clone();
    profile::export(state.store.root(), &research, &control, &models)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn import_setup_profile(
    path: String,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    ensure_workspace_idle(&state)?;
    if !std::path::Path::new(&path).is_absolute() {
        return Err("setup profile path must be absolute".into());
    }
    let imported = profile::import(
        std::path::Path::new(&path),
        &state.research_settings,
        &state.control_settings,
    )
    .map_err(|error| error.to_string())?;
    let roots = default_roots(
        &imported.control.extra_model_roots,
        &imported.research.bonsai_root,
    );
    match tokio::task::spawn_blocking(move || model::scan(&roots)).await {
        Ok(found) => {
            if let Err(error) = state.model_catalog.save(&found) {
                eprintln!("Kestrel imported the profile and found its models, but the disposable catalog could not be saved: {error}");
            }
            *state.models.write().await = found;
        }
        Err(error) => {
            eprintln!("Kestrel imported the profile, but its follow-up model scan could not finish: {error}");
        }
    }
    refresh_engine_candidates(&state, &imported.control, &imported.research).await;
    snapshot(&state).await
}

#[tauri::command]
async fn save_research_settings(
    settings: ResearchSettings,
    state: State<'_, AppState>,
) -> Result<ResearchSettings, String> {
    state
        .research_settings
        .save(&settings)
        .map_err(|error| error.to_string())?;
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let mut control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    control.context_window = settings.context_window;
    control.max_output_tokens = settings.max_output_tokens;
    state
        .control_settings
        .save(&control)
        .map_err(|error| error.to_string())?;
    refresh_engine_candidates(&state, &control, &settings).await;
    Ok(settings)
}

#[tauri::command]
async fn save_control_settings(
    settings: ControlSettings,
    state: State<'_, AppState>,
) -> Result<ControlSnapshot, String> {
    let engine_path = std::path::Path::new(&settings.engine_path);
    if engine_path.is_file() && !runtime::is_llama_server_file(engine_path) {
        return Err(format!(
            "model engine must be a file named llama-server.exe: {}",
            engine_path.display()
        ));
    }
    state
        .control_settings
        .save(&settings)
        .map_err(|error| error.to_string())?;
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let mut research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    research.context_window = settings.context_window;
    research.max_output_tokens = settings.max_output_tokens;
    state
        .research_settings
        .save(&research)
        .map_err(|error| error.to_string())?;
    refresh_engine_candidates(&state, &settings, &research).await;
    control_snapshot(&state, true).await
}

#[tauri::command]
async fn apply_model_runtime(
    settings: ResearchSettings,
    state: State<'_, AppState>,
) -> Result<SystemSnapshot, String> {
    let _guard = claim_workspace(&state)?;
    state
        .research_settings
        .save(&settings)
        .map_err(|error| error.to_string())?;
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let mut control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    control.context_window = settings.context_window;
    control.max_output_tokens = settings.max_output_tokens;
    state
        .control_settings
        .save(&control)
        .map_err(|error| error.to_string())?;
    refresh_engine_candidates(&state, &control, &settings).await;
    state
        .runtime
        .stop_managed()
        .await
        .map_err(|error| error.to_string())?;
    config::apply_bonsai_runtime(&settings).map_err(|error| error.to_string())?;
    services::restart_bonsai(&settings.bonsai_root)
        .await
        .map_err(|error| error.to_string())?;
    let _ = state.runtime.attach_external_if_ready(&settings).await;
    identify_attached_bonsai(&state).await;
    Ok(services::system_snapshot(settings).await)
}

#[tauri::command]
async fn start_local_model(
    model_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ControlSnapshot, String> {
    let _guard = claim_workspace(&state)?;
    let mut settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let model = state
        .models
        .read()
        .await
        .iter()
        .find(|model| model.id == model_id)
        .cloned()
        .ok_or_else(|| "Model is no longer in the local catalog. Rescan first.".to_string())?;
    settings.selected_model_id = Some(model.id.clone());
    state
        .control_settings
        .save(&settings)
        .map_err(|error| error.to_string())?;
    state
        .runtime
        .start_model(&model, &settings, Some(&app))
        .await
        .map_err(|error| error.to_string())?;
    control_snapshot(&state, true).await
}

#[tauri::command]
async fn stop_local_model(state: State<'_, AppState>) -> Result<ControlSnapshot, String> {
    let _guard = claim_workspace(&state)?;
    state
        .runtime
        .stop_managed()
        .await
        .map_err(|error| error.to_string())?;
    control_snapshot(&state, true).await
}

#[tauri::command]
async fn release_ai_memory(state: State<'_, AppState>) -> Result<ControlSnapshot, String> {
    let cancellations = {
        let research = state
            .jobs
            .lock()
            .map_err(|_| "research job registry is unavailable".to_string())?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let interactive = state
            .interactive_jobs
            .lock()
            .map_err(|_| "interactive job registry is unavailable".to_string())?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        research.into_iter().chain(interactive).collect::<Vec<_>>()
    };
    for cancellation in cancellations {
        cancellation.cancel();
    }
    let Some(_idle_permit) = state
        .runtime
        .wait_until_idle(std::time::Duration::from_secs(20))
        .await
    else {
        return Err("The active local request did not release its inference lease within 20 seconds. Its cancellation remains requested; try Release AI memory again after the visible task settles.".into());
    };
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    state
        .runtime
        .stop_managed()
        .await
        .map_err(|error| error.to_string())?;
    services::stop_bonsai(&research.bonsai_root)
        .await
        .map_err(|error| error.to_string())?;
    state
        .runtime
        .stop_orphaned_kestrel_processes()
        .await
        .map_err(|error| error.to_string())?;
    control_snapshot(&state, false).await
}

#[tauri::command]
fn list_chat_sessions(state: State<'_, AppState>) -> Result<Vec<ChatSessionSummary>, String> {
    state.workspace.list_chats()
}

#[tauri::command]
fn get_chat_session(id: String, state: State<'_, AppState>) -> Result<ChatSession, String> {
    state.workspace.get_chat(&id)
}

#[tauri::command]
fn delete_chat_session(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.workspace.delete_chat(&id)
}

#[tauri::command]
async fn pick_context_files(state: State<'_, AppState>) -> Result<ContextAttachmentImport, String> {
    let paths = rfd::AsyncFileDialog::new()
        .set_title("Attach local context")
        .add_filter(
            "Readable context",
            &[
                "png", "jpg", "jpeg", "webp", "gif", "bmp", "wav", "mp3", "flac", "ogg", "m4a",
                "pdf", "docx", "pptx", "xlsx", "txt", "md", "csv", "json", "yaml", "yml", "toml",
                "xml", "html", "log", "rs", "py", "js", "ts", "tsx", "svg",
            ],
        )
        .add_filter("All files", &["*"])
        .pick_files()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|file| file.path().to_path_buf())
        .collect::<Vec<_>>();
    let store = state.attachments.clone();
    tokio::task::spawn_blocking(move || {
        let mut attachments = Vec::new();
        let mut failures = Vec::new();
        for path in paths {
            match store.import_path(&path) {
                Ok(attachment) => attachments.push(attachment),
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }
        ContextAttachmentImport {
            attachments,
            failures,
        }
    })
    .await
    .map_err(|error| format!("Attachment import stopped unexpectedly: {error}"))
}

#[tauri::command]
fn open_context_attachment(id: String, state: State<'_, AppState>) -> Result<(), String> {
    state.attachments.open(&id)
}

#[tauri::command]
async fn pick_local_model_folder() -> Result<Option<String>, String> {
    rfd::AsyncFileDialog::new()
        .set_title("Add a local GGUF model folder")
        .pick_folder()
        .await
        .map(|folder| {
            folder
                .path()
                .canonicalize()
                .map(|value| value.to_string_lossy().into_owned())
                .map_err(|error| error.to_string())
        })
        .transpose()
}

#[tauri::command]
async fn start_chat_stream(
    request: StartChatRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ChatStart, String> {
    ensure_workspace_idle(&state)?;
    if request.message.trim().is_empty() && request.attachment_ids.is_empty() {
        return Err("Write a message or attach local context before sending.".into());
    }
    if request.message.len() > 1_048_576 {
        return Err("A chat message cannot exceed 1 MiB.".into());
    }
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another chat generation or computer task is already active.".to_string())?;
    let attachments = match state.attachments.resolve(&request.attachment_ids) {
        Ok(attachments) => attachments,
        Err(error) => {
            state.work_active.store(false, Ordering::Release);
            return Err(error);
        }
    };
    let message = if request.message.trim().is_empty() {
        "Analyze the attached local context.".to_string()
    } else {
        request.message.trim().to_string()
    };
    let session = match request.session_id.as_deref() {
        Some(id) => {
            let session = match state.workspace.get_chat(id) {
                Ok(session) => session,
                Err(error) => {
                    state.work_active.store(false, Ordering::Release);
                    return Err(error);
                }
            };
            if session.model_id != request.model_id {
                state.work_active.store(false, Ordering::Release);
                return Err("Start a new conversation before switching models.".into());
            }
            session
        }
        None => match state.workspace.create_chat(&request.model_id, &message) {
            Ok(session) => session,
            Err(error) => {
                state.work_active.store(false, Ordering::Release);
                return Err(error);
            }
        },
    };
    let session =
        match state
            .workspace
            .add_user_message_with_attachments(&session.id, message, attachments)
        {
            Ok(session) => session,
            Err(error) => {
                state.work_active.store(false, Ordering::Release);
                return Err(error);
            }
        };
    let request_id = uuid::Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    match state.interactive_jobs.lock() {
        Ok(mut jobs) => {
            jobs.insert(request_id.clone(), cancel.clone());
        }
        Err(_) => {
            state.work_active.store(false, Ordering::Release);
            return Err("interactive job registry is unavailable".into());
        }
    }
    let event_request_id = request_id.clone();
    let session_id = session.id.clone();
    let event_session_id = session_id.clone();
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let managed = app_for_task.state::<AppState>();
        let work_guard = WorkGuard(&managed.work_active);
        let settings = managed.control_settings.load();
        let result = match settings {
            Ok(settings) => {
                let models = managed.models.read().await.clone();
                chat::ChatStreamJob {
                    app: Some(app_for_task.clone()),
                    runtime: managed.runtime.clone(),
                    store: managed.workspace.clone(),
                    attachments: managed.attachments.clone(),
                    request_id: event_request_id.clone(),
                    session_id: event_session_id.clone(),
                    request,
                    models,
                    settings,
                    cancel,
                }
                .run()
                .await
            }
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = result {
            if managed
                .workspace
                .get_chat(&event_session_id)
                .ok()
                .and_then(|session| session.messages.last().cloned())
                .is_some_and(|message| message.role == "user")
            {
                let _ = managed.workspace.add_chat_message_with_status(
                    &event_session_id,
                    "assistant",
                    "Generation stopped before an answer was recorded.".into(),
                    None,
                    Some("interrupted".into()),
                );
            }
            chat::emit_error(
                Some(&app_for_task),
                &event_request_id,
                &event_session_id,
                error,
            );
        }
        if let Ok(mut jobs) = managed.interactive_jobs.lock() {
            jobs.remove(&event_request_id);
        };
        drop(work_guard);
        chat::emit_settled(Some(&app_for_task), &event_request_id, &event_session_id);
    });
    Ok(ChatStart {
        request_id,
        session,
    })
}

#[tauri::command]
fn cancel_chat_stream(request_id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state
        .interactive_jobs
        .lock()
        .map_err(|_| "interactive job registry is unavailable".to_string())?
        .get(&request_id)
    {
        cancel.cancel();
    }
    Ok(())
}

#[tauri::command]
fn list_computer_tasks(state: State<'_, AppState>) -> Result<Vec<ComputerTaskSummary>, String> {
    state.workspace.list_tasks()
}

#[tauri::command]
fn get_computer_task(id: String, state: State<'_, AppState>) -> Result<ComputerTaskRun, String> {
    state.workspace.get_task(&id)
}

#[tauri::command]
async fn start_computer_task(
    mut request: ComputerTaskRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ComputerTaskRun, String> {
    ensure_workspace_idle(&state)?;
    if request.objective.trim().is_empty() && request.attachment_ids.is_empty() {
        return Err(
            "Describe what the computer task should accomplish or attach local context.".into(),
        );
    }
    if request.objective.trim().is_empty() {
        request.objective =
            "Analyze the attached local context and complete the implied task safely.".into();
    }
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    if request.access == ComputerTaskAccess::Full && !settings.allow_full_access_agent {
        return Err("Full computer access is locked in the runtime profile.".into());
    }
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another chat generation or computer task is already active.".to_string())?;
    let attachments = match state.attachments.resolve(&request.attachment_ids) {
        Ok(attachments) => attachments,
        Err(error) => {
            state.work_active.store(false, Ordering::Release);
            return Err(error);
        }
    };
    let run = match state.workspace.create_task_with_attachments(
        &request.model_id,
        &request.objective,
        request.access,
        request.approval_mode,
        attachments,
    ) {
        Ok(run) => run,
        Err(error) => {
            state.work_active.store(false, Ordering::Release);
            return Err(error);
        }
    };
    let cancel = CancellationToken::new();
    match state.interactive_jobs.lock() {
        Ok(mut jobs) => {
            jobs.insert(run.id.clone(), cancel.clone());
        }
        Err(_) => {
            state.work_active.store(false, Ordering::Release);
            return Err("interactive job registry is unavailable".into());
        }
    }
    spawn_computer_task(app, run.id.clone(), request, settings, cancel, None, None);
    Ok(run)
}

#[tauri::command]
async fn resume_computer_task(
    request: ResumeComputerTaskRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ComputerTaskRun, String> {
    ensure_workspace_idle(&state)?;
    let answer = request.answer.trim();
    if answer.is_empty() {
        return Err("Answer the task's question or add a continuation instruction first.".into());
    }
    if answer.len() > 65_536 {
        return Err("A task continuation cannot exceed 64 KiB.".into());
    }
    let run = state.workspace.get_task(&request.run_id)?;
    if !matches!(
        run.status.as_str(),
        "waiting" | "cancelled" | "interrupted" | "failed"
    ) {
        return Err(
            "Only a waiting, stopped, interrupted, or failed task can be continued.".into(),
        );
    }
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    if run.access == ComputerTaskAccess::Full && !settings.allow_full_access_agent {
        return Err("Full computer access is locked in the runtime profile.".into());
    }
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another chat generation or computer task is already active.".to_string())?;
    let input_event = models::ComputerTaskEvent {
        run_id: run.id.clone(),
        step: 0,
        kind: "user_input".into(),
        title: "Direction from you".into(),
        detail: answer.to_string(),
        data: None,
        at: chrono::Utc::now().to_rfc3339(),
    };
    let updated = match state.workspace.add_task_event(input_event.clone()) {
        Ok(updated) => updated,
        Err(error) => {
            state.work_active.store(false, Ordering::Release);
            return Err(error);
        }
    };
    let _ = app.emit("computer-task-event", input_event);
    let task_request = ComputerTaskRequest {
        model_id: run.model_id.clone(),
        objective: run.objective.clone(),
        access: run.access,
        approval_mode: run.approval_mode,
        max_steps: settings.agent_max_steps,
        max_output_tokens: settings.agent_max_output_tokens,
        attachment_ids: run.attachments.iter().map(|item| item.id.clone()).collect(),
    };
    let continuation = task_continuation(&run, answer);
    let cancel = CancellationToken::new();
    match state.interactive_jobs.lock() {
        Ok(mut jobs) => {
            jobs.insert(run.id.clone(), cancel.clone());
        }
        Err(_) => {
            state.work_active.store(false, Ordering::Release);
            return Err("interactive job registry is unavailable".into());
        }
    }
    spawn_computer_task(
        app,
        run.id.clone(),
        task_request,
        settings,
        cancel,
        Some(continuation),
        None,
    );
    Ok(updated)
}

#[tauri::command]
async fn resolve_computer_task_approval(
    request: ResolveComputerTaskApprovalRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ComputerTaskRun, String> {
    ensure_workspace_idle(&state)?;
    uuid::Uuid::parse_str(&request.approval_id)
        .map_err(|_| "The pending approval ID is invalid.".to_string())?;
    if request.note.len() > 8_192 {
        return Err("An approval note cannot exceed 8 KiB.".into());
    }
    let run = state.workspace.get_task(&request.run_id)?;
    if run.status != "approval" {
        return Err("This task is not waiting for an approval decision.".into());
    }
    let (step, action) = pending_computer_action(&run, &request.approval_id)?;
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    if run.access == ComputerTaskAccess::Full && !settings.allow_full_access_agent {
        return Err("Full computer access is locked in the runtime profile.".into());
    }
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another chat generation or computer task is already active.".to_string())?;

    let note = request.note.trim();
    let (kind, title, detail, approved_action, continuation) = match request.decision {
        ComputerTaskApprovalDecision::Approve => {
            let mut approved = action.clone();
            approved.authorization_note = (!note.is_empty()).then(|| note.to_string());
            (
                "approval_approved",
                "Approved by you",
                format!(
                    "Authorized once: {}{}",
                    action.summary,
                    if note.is_empty() {
                        String::new()
                    } else {
                        format!("\nYour note: {note}")
                    }
                ),
                Some(approved),
                String::new(),
            )
        }
        ComputerTaskApprovalDecision::Reject => (
            "approval_rejected",
            "Rejected by you",
            format!(
                "Not authorized: {}{}",
                action.summary,
                if note.is_empty() {
                    String::new()
                } else {
                    format!("\nYour direction: {note}")
                }
            ),
            None,
            format!(
                "The user rejected this exact action: {} Do not attempt the same outcome through a workaround or indirect execution. Find a materially safer alternative, or call ask_user if the task cannot continue safely.{}",
                action.summary,
                if note.is_empty() {
                    String::new()
                } else {
                    format!("\nUser's direction: {note}")
                }
            ),
        ),
    };
    let decision_event = models::ComputerTaskEvent {
        run_id: run.id.clone(),
        step,
        kind: kind.into(),
        title: title.into(),
        detail,
        data: Some(serde_json::json!({
            "approvalId": action.approval_id,
            "decision": match request.decision {
                ComputerTaskApprovalDecision::Approve => "approved",
                ComputerTaskApprovalDecision::Reject => "rejected",
            },
            "note": note,
        })),
        at: chrono::Utc::now().to_rfc3339(),
    };
    let updated = match state.workspace.add_task_event(decision_event.clone()) {
        Ok(updated) => updated,
        Err(error) => {
            state.work_active.store(false, Ordering::Release);
            return Err(error);
        }
    };
    let _ = app.emit("computer-task-event", decision_event);
    let task_request = ComputerTaskRequest {
        model_id: run.model_id.clone(),
        objective: run.objective.clone(),
        attachment_ids: run.attachments.iter().map(|item| item.id.clone()).collect(),
        access: run.access,
        approval_mode: run.approval_mode,
        max_steps: settings.agent_max_steps,
        max_output_tokens: settings.agent_max_output_tokens,
    };
    let cancel = CancellationToken::new();
    match state.interactive_jobs.lock() {
        Ok(mut jobs) => {
            jobs.insert(run.id.clone(), cancel.clone());
        }
        Err(_) => {
            state.work_active.store(false, Ordering::Release);
            return Err("interactive job registry is unavailable".into());
        }
    }
    spawn_computer_task(
        app,
        run.id.clone(),
        task_request,
        settings,
        cancel,
        Some(
            if request.decision == ComputerTaskApprovalDecision::Approve {
                task_replay_continuation(&run)
            } else {
                task_native_continuation(&run, &continuation)
            },
        ),
        approved_action,
    );
    Ok(updated)
}

fn pending_computer_action(
    run: &ComputerTaskRun,
    approval_id: &str,
) -> Result<(u32, PendingComputerAction), String> {
    let event = run
        .events
        .iter()
        .rev()
        .find(|event| event.kind == "approval_required")
        .ok_or_else(|| "The durable task has no pending approval request.".to_string())?;
    let action = serde_json::from_value::<PendingComputerAction>(
        event
            .data
            .clone()
            .ok_or_else(|| "The pending approval record is incomplete.".to_string())?,
    )
    .map_err(|error| format!("The pending approval record is invalid: {error}"))?;
    if action.approval_id != approval_id {
        return Err("That approval is stale. Review the task's current pending action.".into());
    }
    Ok((event.step, action))
}

fn spawn_computer_task(
    app: AppHandle,
    run_id: String,
    request: ComputerTaskRequest,
    settings: ControlSettings,
    cancel: CancellationToken,
    continuation: Option<String>,
    approved_action: Option<PendingComputerAction>,
) {
    tauri::async_runtime::spawn(async move {
        let managed = app.state::<AppState>();
        let _work_guard = WorkGuard(&managed.work_active);
        let models = managed.models.read().await.clone();
        let result = agent::run(
            Some(app.clone()),
            managed.runtime.clone(),
            managed.workspace.clone(),
            managed.attachments.clone(),
            run_id.clone(),
            request,
            models,
            settings,
            cancel,
            continuation,
            approved_action,
        )
        .await;
        if let Err(error) = result {
            agent::emit_error(Some(&app), &managed.workspace, &run_id, error);
        }
        if let Ok(mut jobs) = managed.interactive_jobs.lock() {
            jobs.remove(&run_id);
        };
    });
}

fn task_continuation(run: &ComputerTaskRun, answer: &str) -> String {
    task_continuation_with_label(run, "User's new direction", answer)
}

fn task_native_continuation(run: &ComputerTaskRun, directive: &str) -> String {
    task_continuation_with_label(run, "Kestrel's verified approval state", directive)
}

fn task_replay_continuation(run: &ComputerTaskRun) -> String {
    let transcript = recent_task_transcript(run, false);
    format!(
        "Continue the same durable task. Re-inspect current state before further changes because the prior run may have stopped between actions.\n\nRecent verified tool transcript:\n{transcript}"
    )
}

fn task_continuation_with_label(run: &ComputerTaskRun, label: &str, value: &str) -> String {
    let transcript = recent_task_transcript(run, true);
    format!(
        "Continue the same durable task. Re-inspect current state before further changes because the prior run may have stopped between actions.\n\nRecent verified transcript:\n{transcript}\n{label}:\n{value}"
    )
}

fn recent_task_transcript(run: &ComputerTaskRun, include_approvals: bool) -> String {
    let mut transcript = String::new();
    for event in run
        .events
        .iter()
        .filter(|event| {
            event.kind != "reasoning" && (include_approvals || !event.kind.starts_with("approval_"))
        })
        .rev()
        .take(30)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        transcript.push_str(&format!("- {}: {}\n", event.title, event.detail));
        if transcript.len() >= 24_000 {
            transcript = transcript.chars().take(24_000).collect();
            break;
        }
    }
    transcript
}

#[tauri::command]
fn stop_computer_task(run_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let (cancel, interactive_registry_empty) = {
        let jobs = state
            .interactive_jobs
            .lock()
            .map_err(|_| "interactive job registry is unavailable".to_string())?;
        (jobs.get(&run_id).cloned(), jobs.is_empty())
    };
    if let Some(cancel) = cancel {
        cancel.cancel();
        return Ok(());
    }
    let run = state.workspace.get_task(&run_id)?;
    let terminal = matches!(
        run.status.as_str(),
        "completed" | "cancelled" | "failed" | "interrupted"
    );
    if terminal {
        if interactive_registry_empty && !state.research_active.load(Ordering::Acquire) {
            state.work_active.store(false, Ordering::Release);
        }
        return Ok(());
    }
    Err("That computer task is not registered as an active job. Restart Kestrel to recover its inference lock.".into())
}

#[tauri::command]
fn open_task_artifact(
    run_id: String,
    path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let run = state.workspace.get_task(&run_id)?;
    if !run.artifacts.iter().any(|known| known == &path) {
        return Err("That path is not a recorded artifact of this task.".into());
    }
    let path = std::path::PathBuf::from(path);
    if !path.exists() {
        return Err(format!("Artifact no longer exists: {}", path.display()));
    }
    open_with_explorer(&path)
}

#[tauri::command]
async fn run_native_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    ensure_workspace_idle(&state)?;
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    state
        .developer
        .diagnose(&settings.project_root, Some(&app))
        .await
}

#[tauri::command]
async fn run_codex_repair(
    request: DeveloperRepairRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<DeveloperRepairReport, String> {
    ensure_workspace_idle(&state)?;
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    state
        .developer
        .repair(&settings.project_root, &request.issue, Some(&app))
        .await
}

#[tauri::command]
fn open_bonsai_control_center(state: State<'_, AppState>) -> Result<(), String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let path = std::path::Path::new(&settings.bonsai_root).join("BonsaiLauncher.exe");
    if !path.is_file() {
        return Err(format!(
            "Bonsai control center is missing: {}",
            path.display()
        ));
    }
    std::process::Command::new(&path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not open {}: {error}", path.display()))
}

#[tauri::command]
fn open_standalone_report(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let path = state
        .store
        .html_path(&id)
        .map_err(|error| error.to_string())?;
    open_with_explorer(&path)
}

#[tauri::command]
fn reveal_library(state: State<'_, AppState>) -> Result<(), String> {
    open_with_explorer(state.store.root())
}

async fn snapshot(state: &AppState) -> Result<AppSnapshot, String> {
    let status = services::status().await;
    let reports = state
        .store
        .list(100_000)
        .map_err(|error| error.to_string())?;
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    Ok(AppSnapshot {
        status,
        reports,
        library_root: state.store.root().to_string_lossy().into_owned(),
        settings,
        control: control_snapshot(state, false).await?,
    })
}

async fn control_snapshot(
    state: &AppState,
    probe_developer: bool,
) -> Result<ControlSnapshot, String> {
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let developer = if state.research_active.load(Ordering::Acquire) || !probe_developer {
        state.developer.passive_status(&settings.project_root)
    } else {
        state.developer.status(&settings.project_root).await
    };
    Ok(ControlSnapshot {
        engine_candidates: state.engine_candidates.read().await.clone(),
        settings,
        models: state.models.read().await.clone(),
        runtime: state.runtime.snapshot().await,
        gpu: services::gpu_snapshot().await,
        developer,
        runtime_logs: state.runtime.recent_logs(100).await,
    })
}

async fn refresh_engine_candidates(
    state: &AppState,
    control: &ControlSettings,
    research: &ResearchSettings,
) {
    let engine_path = control.engine_path.clone();
    let bonsai_root = research.bonsai_root.clone();
    match tokio::task::spawn_blocking(move || runtime::detect_engines(&engine_path, &bonsai_root))
        .await
    {
        Ok(candidates) => *state.engine_candidates.write().await = candidates,
        Err(error) => {
            eprintln!("Kestrel could not refresh local engine candidates: {error}");
        }
    }
}

fn ensure_workspace_idle(state: &AppState) -> Result<(), String> {
    if state.work_active.load(Ordering::Acquire) {
        Err("Chat, research, or a computer task is active. Stop or finish it before changing runtime or developer state.".into())
    } else {
        Ok(())
    }
}

fn claim_workspace(state: &AppState) -> Result<WorkGuard<'_>, String> {
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| {
            "Chat, research, or a computer task is active. Stop or finish it before changing runtime or developer state.".to_string()
        })?;
    Ok(WorkGuard(&state.work_active))
}

async fn identify_attached_bonsai(state: &AppState) {
    let model = state
        .models
        .read()
        .await
        .iter()
        .find(|model| {
            format!("{} {}", model.name, model.path)
                .to_lowercase()
                .contains("bonsai")
        })
        .cloned();
    if let Some(model) = model {
        state.runtime.identify_attached_model(&model).await;
    }
}

fn open_with_explorer(path: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not open {}: {error}", path.display()))
}

pub fn run() {
    let application = tauri::Builder::default()
        .setup(|app| {
            let store = ResearchStore::open_default().map_err(|error| error.to_string())?;
            let research_settings = SettingsStore::new(store.root());
            let control_settings = ControlSettingsStore::new(store.root());
            let research = research_settings
                .load()
                .map_err(|error| error.to_string())?;
            let mut control = control_settings.load().map_err(|error| error.to_string())?;
            let mut engine_candidates =
                runtime::detect_engines(&control.engine_path, &research.bonsai_root);
            if !std::path::Path::new(&control.engine_path).is_file() {
                if let Some(candidate) = engine_candidates.first() {
                    control.engine_path.clone_from(&candidate.path);
                    if let Err(error) = control_settings.save(&control) {
                        eprintln!("Kestrel repaired the missing engine path for this session, but could not save it. Check write access to the Kestrel Research folder: {error}");
                    }
                    engine_candidates =
                        runtime::detect_engines(&control.engine_path, &research.bonsai_root);
                }
            }
            // Startup reads only the validated cache; the full scan runs after the window opens.
            let model_catalog = ModelCatalogStore::new(store.root());
            let models = model_catalog.load().unwrap_or_else(|error| {
                eprintln!("Kestrel could not restore its disposable model catalog: {error}");
                Vec::new()
            });
            let harness = ResearchHarness::new(store.clone());
            let developer = DeveloperAssistant::new(store.root());
            let workspace = WorkspaceStore::new(store.root())?;
            let attachments = AttachmentStore::new(&store.root().join("workspace"))?;
            app.manage(AppState {
                store,
                harness,
                research_settings,
                control_settings,
                model_catalog,
                models: RwLock::new(models),
                engine_candidates: RwLock::new(engine_candidates),
                runtime: Arc::new(RuntimeManager::new()),
                developer,
                workspace,
                attachments,
                research_active: AtomicBool::new(false),
                work_active: AtomicBool::new(false),
                jobs: Mutex::new(HashMap::new()),
                interactive_jobs: Mutex::new(HashMap::new()),
            });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<AppState>();
                if let Err(error) = state.runtime.stop_orphaned_kestrel_processes().await {
                    eprintln!("Kestrel could not clean up an abandoned model process: {error}");
                }
                let research = match state.research_settings.load() {
                    Ok(value) => value,
                    Err(_) => return,
                };
                let control = match state.control_settings.load() {
                    Ok(value) => value,
                    Err(_) => return,
                };
                let roots = default_roots(&control.extra_model_roots, &research.bonsai_root);
                let found = match tokio::task::spawn_blocking(move || model::scan(&roots)).await {
                    Ok(value) => value,
                    Err(error) => {
                        eprintln!("Kestrel's background model scan could not finish: {error}");
                        return;
                    }
                };
                let existing = state.models.read().await.clone();
                let merged = merge_catalogs(existing, found);
                if let Err(error) = state.model_catalog.save(&merged) {
                    eprintln!("Kestrel found local models, but its disposable catalog could not be saved: {error}");
                }
                *state.models.write().await = merged.clone();
                let _ = handle.emit("model-catalog-updated", merged);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            get_report,
            run_research,
            cancel_research,
            prepare_services,
            open_standalone_report,
            reveal_library,
            get_system_snapshot,
            get_control_snapshot,
            scan_local_models,
            export_setup_profile,
            import_setup_profile,
            save_research_settings,
            save_control_settings,
            apply_model_runtime,
            start_local_model,
            stop_local_model,
            release_ai_memory,
            list_chat_sessions,
            get_chat_session,
            delete_chat_session,
            pick_context_files,
            open_context_attachment,
            pick_local_model_folder,
            start_chat_stream,
            cancel_chat_stream,
            list_computer_tasks,
            get_computer_task,
            start_computer_task,
            resume_computer_task,
            resolve_computer_task_approval,
            stop_computer_task,
            open_task_artifact,
            run_native_diagnostics,
            run_codex_repair,
            open_bonsai_control_center,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Kestrel Local");
    application.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            let runtime = handle.state::<AppState>().runtime.clone();
            let _ = tauri::async_runtime::block_on(runtime.stop_managed());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ComputerTaskApprovalMode;

    #[test]
    fn approval_resolution_loads_only_the_current_exact_action() {
        let approval_id = uuid::Uuid::new_v4().to_string();
        let run = ComputerTaskRun {
            id: uuid::Uuid::new_v4().to_string(),
            objective: "write notes".into(),
            model_id: "model".into(),
            access: ComputerTaskAccess::Workspace,
            approval_mode: ComputerTaskApprovalMode::Manual,
            status: "approval".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            events: vec![models::ComputerTaskEvent {
                run_id: "run".into(),
                step: 3,
                kind: "approval_required".into(),
                title: "Approval required".into(),
                detail: "Write notes".into(),
                data: Some(serde_json::json!({
                    "approvalId": approval_id,
                    "step": 3,
                    "tool": "write_file",
                    "arguments": {"path":"C:\\Work\\notes.txt","content":"notes"},
                    "summary": "Write notes",
                    "reason": "Manual approval",
                    "risk": "low"
                })),
                at: chrono::Utc::now().to_rfc3339(),
            }],
            artifacts: Vec::new(),
            attachments: Vec::new(),
        };
        let (step, action) = pending_computer_action(&run, &approval_id).unwrap();
        assert_eq!(step, 3);
        assert_eq!(action.approval_id, approval_id);
        assert_eq!(action.arguments["content"], "notes");
        assert!(pending_computer_action(&run, &uuid::Uuid::new_v4().to_string()).is_err());
    }

    #[test]
    fn approved_replay_hides_approval_protocol_from_the_main_agent() {
        let now = chrono::Utc::now().to_rfc3339();
        let run = ComputerTaskRun {
            id: uuid::Uuid::new_v4().to_string(),
            objective: "write notes".into(),
            model_id: "model".into(),
            access: ComputerTaskAccess::Workspace,
            approval_mode: ComputerTaskApprovalMode::Manual,
            status: "running".into(),
            created_at: now.clone(),
            updated_at: now.clone(),
            events: vec![
                models::ComputerTaskEvent {
                    run_id: "run".into(),
                    step: 1,
                    kind: "tool_result".into(),
                    title: "read_file finished".into(),
                    detail: "existing notes".into(),
                    data: None,
                    at: now.clone(),
                },
                models::ComputerTaskEvent {
                    run_id: "run".into(),
                    step: 2,
                    kind: "approval_required".into(),
                    title: "Approval required".into(),
                    detail: "guardian-private explanation".into(),
                    data: None,
                    at: now,
                },
            ],
            artifacts: Vec::new(),
            attachments: Vec::new(),
        };
        let replay = task_replay_continuation(&run);
        assert!(replay.contains("existing notes"));
        assert!(!replay.contains("guardian-private explanation"));
        assert!(!replay.contains("Approval required"));
    }
}
