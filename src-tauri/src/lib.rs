#![recursion_limit = "256"]

mod agent;
mod attachments;
mod chat;
mod config;
mod developer;
mod harness;
mod html;
mod kiwix;
mod local_speech;
mod model;
mod model_download;
mod model_roles;
mod models;
mod profile;
mod runtime;
mod services;
mod setup;
mod store;
mod studio;
mod workspace;

use attachments::{AttachmentStore, ContextAttachment};
use config::{ControlSettingsStore, SettingsStore};
use developer::DeveloperAssistant;
use harness::ResearchHarness;
use local_speech::{
    LocalSpeech, SpeechAlignmentRequest, SpeechClip, SpeechSnapshot, SpeechSynthesisRequest,
    SpeechTranscription, SpeechTranscriptionRequest,
};
use model::{default_roots, merge_catalogs, ModelCatalogStore, ModelInfo};
use model_download::{
    ModelDownloadInspection, ModelDownloadManager, ModelDownloadRecord, ModelDownloadRequest,
};
use model_roles::{
    is_bonsai, qualification_receipt, ModelCompatibility, ModelQualificationStore,
    STUDIO_PROTOCOL_REVISION,
};
use models::{
    AppSnapshot, ChatSession, ChatSessionSummary, ChatStart, ComputerTaskAccess,
    ComputerTaskRequest, ComputerTaskRun, ComputerTaskSummary, ControlSettings, ControlSnapshot,
    DeveloperRepairReport, DeveloperRepairRequest, ProfileTransfer, ResearchReport,
    ResearchSettings, ResumeComputerTaskRequest, RunResearchRequest, StartChatRequest,
    SystemSnapshot,
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
use studio::{
    MovieClipAssistRequest, MovieClipRenderRequest, MovieClipSuggestion, MovieCopilotJob,
    MovieCopilotReceipt, MovieCopilotRequest, MovieEdit, MovieImageAssetGeneration,
    MovieImageAssetRequest, MovieModelBinding, MovieModelRoleRequest, MovieModelRoles,
    MovieModelRuntime, MoviePlan, MoviePlanFeedbackRequest, MoviePlanningSnapshot, MovieProject,
    MovieReferenceImport, MovieStudio, MovieSummary, PromptDraftJob, PromptDraftRequest,
    StartMovieRequest,
};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{Mutex as AsyncMutex, RwLock};
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
    model_downloads: ModelDownloadManager,
    model_qualifications: ModelQualificationStore,
    models: RwLock<Vec<ModelInfo>>,
    engine_candidates: RwLock<Vec<models::EngineCandidate>>,
    runtime: Arc<RuntimeManager>,
    developer: DeveloperAssistant,
    workspace: WorkspaceStore,
    attachments: AttachmentStore,
    research_active: AtomicBool,
    work_active: AtomicBool,
    jobs: Mutex<HashMap<String, CancellationToken>>,
    speech: LocalSpeech,
    speech_command_gate: AsyncMutex<()>,
    speech_jobs: Mutex<HashMap<String, CancellationToken>>,
    speech_restore_model: Mutex<Option<SpeechRuntimeRestore>>,
    interactive_jobs: Mutex<HashMap<String, CancellationToken>>,
    studio: MovieStudio,
    movie_jobs: Mutex<HashMap<String, CancellationToken>>,
    image_asset_jobs: Mutex<HashMap<String, CancellationToken>>,
    setup_job: Mutex<Option<CancellationToken>>,
    model_download_job: Mutex<Option<CancellationToken>>,
}

#[derive(Clone)]
struct SpeechRuntimeRestore {
    model_id: String,
    attached: bool,
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
async fn get_local_speech_snapshot(state: State<'_, AppState>) -> Result<SpeechSnapshot, String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    Ok(state.speech.snapshot(&settings.comfy_root).await)
}

async fn remember_runtime_for_speech(state: &AppState) {
    let snapshot = state.runtime.snapshot().await;
    let Some(model_id) = snapshot.model_id.filter(|_| snapshot.phase == "ready") else {
        return;
    };
    if let Ok(mut restore) = state.speech_restore_model.lock() {
        if restore.is_none() {
            *restore = Some(SpeechRuntimeRestore {
                model_id,
                attached: snapshot.mode == "attached",
            });
        }
    }
}

async fn restore_runtime_after_speech(state: &AppState, app: &AppHandle) -> Result<(), String> {
    let restore = state
        .speech_restore_model
        .lock()
        .map_err(|_| "Local speech restore state is unavailable".to_string())?
        .take();
    let Some(restore) = restore else {
        return Ok(());
    };
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let model = state
        .models
        .read()
        .await
        .iter()
        .find(|model| model.id == restore.model_id)
        .cloned();
    let Some(model) = model else {
        if let Ok(mut pending) = state.speech_restore_model.lock() {
            *pending = Some(restore);
        }
        return Err(
            "The model used before local speech is no longer in the catalog. Rescan it in Control."
                .into(),
        );
    };
    let result: Result<(), String> = async {
        if restore.attached {
            let research = state
                .research_settings
                .load()
                .map_err(|error| error.to_string())?;
            services::restart_bonsai(&research.bonsai_root)
                .await
                .map_err(|error| error.to_string())?;
            state
                .runtime
                .attach_external_if_ready(&research)
                .await
                .ok_or_else(|| {
                    "the configured attached Bonsai service did not become ready".to_string()
                })?;
            state.runtime.identify_attached_model(&model).await;
        } else {
            state
                .runtime
                .start_model(&model, &settings, Some(app))
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        if let Ok(mut pending) = state.speech_restore_model.lock() {
            *pending = Some(restore);
        }
        return Err(format!(
            "Speech finished, but Kestrel could not restore {}: {error}",
            model.name
        ));
    }
    Ok(())
}

async fn claim_workspace_after_speech(state: &AppState) -> Result<WorkGuard<'_>, String> {
    for _ in 0..80 {
        if let Ok(guard) = claim_workspace(state) {
            return Ok(guard);
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    Err("Local speech is still stopping after 20 seconds. Use Release AI memory in Control; Kestrel kept the previous model identity so it can be restored safely.".into())
}

fn register_speech_job(state: &AppState, job_id: &str) -> Result<CancellationToken, String> {
    let cancel = CancellationToken::new();
    let mut jobs = state
        .speech_jobs
        .lock()
        .map_err(|_| "Local speech job registry is unavailable".to_string())?;
    if jobs.contains_key(job_id) {
        return Err("Local speech job ID is already active".into());
    }
    jobs.insert(job_id.to_string(), cancel.clone());
    Ok(cancel)
}

fn finish_speech_job(state: &AppState, job_id: &str) {
    if let Ok(mut jobs) = state.speech_jobs.lock() {
        jobs.remove(job_id);
    }
}

async fn wait_for_speech_turn<'a>(
    state: &'a AppState,
    cancel: &CancellationToken,
) -> Result<tokio::sync::MutexGuard<'a, ()>, String> {
    tokio::select! {
        turn = state.speech_command_gate.lock() => Ok(turn),
        _ = cancel.cancelled() => Err("Local speech operation was stopped".into()),
    }
}

#[tauri::command]
async fn prepare_local_speech(state: State<'_, AppState>) -> Result<SpeechSnapshot, String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let initial = state.speech.snapshot(&settings.comfy_root).await;
    if (!initial.narration_available && !initial.transcription_available) || initial.comfy_ready {
        return Ok(initial);
    }
    const JOB_ID: &str = "prepare-local-speech";
    let cancel = register_speech_job(&state, JOB_ID)?;
    let result: Result<SpeechSnapshot, String> = async {
        let _turn = wait_for_speech_turn(&state, &cancel).await?;
        let _guard = claim_workspace(&state)?;
        remember_runtime_for_speech(&state).await;
        state
            .runtime
            .stop_managed()
            .await
            .map_err(|error| error.to_string())?;
        services::stop_bonsai(&settings.bonsai_root)
            .await
            .map_err(|error| error.to_string())?;
        state
            .speech
            .ensure_comfy(&settings.comfy_root, &cancel)
            .await
            .map_err(|error| error.to_string())?;
        Ok(state.speech.snapshot(&settings.comfy_root).await)
    }
    .await;
    finish_speech_job(&state, JOB_ID);
    result
}

#[tauri::command]
async fn synthesize_local_speech(
    request: SpeechSynthesisRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SpeechClip, String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    if let Some(clip) = state
        .speech
        .cached_clip(&settings.comfy_root, &request)
        .map_err(|error| error.to_string())?
    {
        return Ok(clip);
    }
    let cancel = register_speech_job(&state, &request.job_id)?;
    let result: Result<SpeechClip, String> = async {
        let _turn = wait_for_speech_turn(&state, &cancel).await?;
        if let Some(clip) = state
            .speech
            .cached_clip(&settings.comfy_root, &request)
            .map_err(|error| error.to_string())?
        {
            return Ok(clip);
        }
        let _guard = claim_workspace(&state)?;
        remember_runtime_for_speech(&state).await;
        state
            .runtime
            .stop_managed()
            .await
            .map_err(|error| error.to_string())?;
        services::stop_bonsai(&settings.bonsai_root)
            .await
            .map_err(|error| error.to_string())?;
        state
            .speech
            .ensure_comfy(&settings.comfy_root, &cancel)
            .await
            .map_err(|error| error.to_string())?;
        state
            .speech
            .synthesize(&settings.comfy_root, &request, &cancel, Some(&app))
            .await
            .map_err(|error| error.to_string())
    }
    .await;
    finish_speech_job(&state, &request.job_id);
    result
}

#[tauri::command]
fn cancel_local_speech(job_id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state
        .speech_jobs
        .lock()
        .map_err(|_| "Local speech job registry is unavailable".to_string())?
        .get(&job_id)
    {
        cancel.cancel();
    }
    Ok(())
}

#[tauri::command]
async fn transcribe_local_speech(
    request: SpeechTranscriptionRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SpeechTranscription, String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let cancel = register_speech_job(&state, &request.job_id)?;
    let result: Result<SpeechTranscription, String> = async {
        let _turn = wait_for_speech_turn(&state, &cancel).await?;
        let _guard = claim_workspace(&state)?;
        remember_runtime_for_speech(&state).await;
        state
            .runtime
            .stop_managed()
            .await
            .map_err(|error| error.to_string())?;
        services::stop_bonsai(&settings.bonsai_root)
            .await
            .map_err(|error| error.to_string())?;
        state
            .speech
            .ensure_comfy(&settings.comfy_root, &cancel)
            .await
            .map_err(|error| error.to_string())?;
        state
            .speech
            .transcribe(&settings.comfy_root, &request, &cancel, Some(&app))
            .await
            .map_err(|error| error.to_string())
    }
    .await;
    finish_speech_job(&state, &request.job_id);
    result
}

#[tauri::command]
async fn align_local_speech(
    request: SpeechAlignmentRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SpeechClip, String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    if let Some(clip) = state
        .speech
        .cached_alignment(&settings.comfy_root, &request)
        .map_err(|error| error.to_string())?
    {
        return Ok(clip);
    }
    let cancel = register_speech_job(&state, &request.job_id)?;
    let result: Result<SpeechClip, String> = async {
        let _turn = wait_for_speech_turn(&state, &cancel).await?;
        if let Some(clip) = state
            .speech
            .cached_alignment(&settings.comfy_root, &request)
            .map_err(|error| error.to_string())?
        {
            return Ok(clip);
        }
        let _guard = claim_workspace(&state)?;
        remember_runtime_for_speech(&state).await;
        state
            .runtime
            .stop_managed()
            .await
            .map_err(|error| error.to_string())?;
        services::stop_bonsai(&settings.bonsai_root)
            .await
            .map_err(|error| error.to_string())?;
        state
            .speech
            .ensure_comfy(&settings.comfy_root, &cancel)
            .await
            .map_err(|error| error.to_string())?;
        state
            .speech
            .align(&settings.comfy_root, &request, &cancel, Some(&app))
            .await
            .map_err(|error| error.to_string())
    }
    .await;
    finish_speech_job(&state, &request.job_id);
    result
}

#[tauri::command]
async fn release_local_speech_memory(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _guard = claim_workspace_after_speech(&state).await?;
    state.speech.release_model_memory().await;
    state.speech.stop_comfy().await;
    restore_runtime_after_speech(&state, &app).await
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
fn list_movies(state: State<'_, AppState>) -> Result<Vec<MovieSummary>, String> {
    state.studio.list().map_err(|error| error.to_string())
}

#[tauri::command]
fn get_movie(id: String, state: State<'_, AppState>) -> Result<MovieProject, String> {
    state.studio.get(&id).map_err(|error| error.to_string())
}

#[tauri::command]
async fn pick_movie_reference_files(
    state: State<'_, AppState>,
) -> Result<MovieReferenceImport, String> {
    let paths = rfd::AsyncFileDialog::new()
        .set_title("Attach H3 producer references")
        .add_filter("Pictures", &["png", "jpg", "jpeg", "webp", "bmp"])
        .add_filter(
            "Video (2-15 seconds)",
            &["mp4", "m4v", "mov", "mkv", "webm"],
        )
        .add_filter(
            "Audio (up to 15 seconds)",
            &["wav", "mp3", "flac", "ogg", "oga", "m4a", "aac"],
        )
        .pick_files()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|file| file.path().to_path_buf())
        .collect::<Vec<_>>();
    let studio = state.studio.clone();
    tokio::task::spawn_blocking(move || {
        let mut references = Vec::new();
        let mut failures = Vec::new();
        for path in paths {
            match studio.import_reference_path(&path) {
                Ok(reference) => references.push(reference),
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }
        MovieReferenceImport {
            references,
            failures,
        }
    })
    .await
    .map_err(|error| format!("Reference import stopped unexpectedly: {error}"))
}

#[tauri::command]
fn list_movie_image_assets(
    state: State<'_, AppState>,
) -> Result<Vec<MovieImageAssetGeneration>, String> {
    state
        .studio
        .list_image_asset_generations()
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_movie_image_asset(
    request: MovieImageAssetRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    ensure_workspace_idle(&state)?;
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    request
        .validate(research.advanced_mode)
        .map_err(|error| error.to_string())?;
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another local AI or Studio job is already active.".to_string())?;
    let cancel = CancellationToken::new();
    if state
        .image_asset_jobs
        .lock()
        .map(|mut jobs| {
            jobs.insert(request.request_id.clone(), cancel.clone());
        })
        .is_err()
    {
        state.work_active.store(false, Ordering::Release);
        return Err("image asset job registry is unavailable".into());
    }
    let request_id = request.request_id.clone();
    let task_id = request_id.clone();
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let managed = app_for_task.state::<AppState>();
        let _guard = WorkGuard(&managed.work_active);
        let renderer_ready: Result<(), String> = async {
            managed
                .runtime
                .stop_managed()
                .await
                .map_err(|error| error.to_string())?;
            services::stop_bonsai(&research.bonsai_root)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        .await;
        if let Err(error) = renderer_ready {
            studio::emit_image_asset_error(&app_for_task, &task_id, error);
        } else {
            let _ = managed
                .studio
                .generate_image_assets(request, &cancel, Some(&app_for_task))
                .await;
        }
        if let Ok(mut jobs) = managed.image_asset_jobs.lock() {
            jobs.remove(&task_id);
        };
    });
    Ok(request_id)
}

#[tauri::command]
fn cancel_movie_image_asset(request_id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state
        .image_asset_jobs
        .lock()
        .map_err(|_| "image asset job registry is unavailable".to_string())?
        .get(&request_id)
    {
        cancel.cancel();
    }
    Ok(())
}

fn studio_runtime_settings(
    mut control: ControlSettings,
    research: &ResearchSettings,
) -> ControlSettings {
    control.context_window = research.context_window;
    control.max_output_tokens = research.max_output_tokens;
    control
}

async fn studio_model_context(
    state: &AppState,
) -> Result<(ResearchSettings, ControlSettings, Vec<ModelInfo>), String> {
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let models = state.models.read().await.clone();
    Ok((
        research.clone(),
        studio_runtime_settings(control, &research),
        models,
    ))
}

fn model_binding(model: &ModelInfo, compatibility: &ModelCompatibility) -> MovieModelBinding {
    MovieModelBinding {
        model_id: model.id.clone(),
        model_name: model.name.clone(),
        compatibility_tier: compatibility.tier.clone(),
        protocol_revision: STUDIO_PROTOCOL_REVISION.into(),
        bound_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn select_default_studio_model<'a>(
    models: &'a [ModelInfo],
    selected_model_id: Option<&str>,
    qualifications: &ModelQualificationStore,
    settings: &ControlSettings,
) -> Result<Option<&'a ModelInfo>, String> {
    if let Some(selected) =
        selected_model_id.and_then(|id| models.iter().find(|model| model.id == id))
    {
        if qualifications.assess(selected, settings)?.studio_ready {
            return Ok(Some(selected));
        }
    }
    Ok(models
        .iter()
        .find(|model| is_bonsai(model))
        .or_else(|| models.first()))
}

fn resolve_movie_model_roles(
    request: &MovieModelRoleRequest,
    producer_authored: bool,
    advanced: bool,
    models: &[ModelInfo],
    settings: &ControlSettings,
    qualifications: &ModelQualificationStore,
) -> Result<MovieModelRoles, String> {
    if models.is_empty() {
        return if producer_authored {
            Ok(MovieModelRoles::default())
        } else {
            Err("No local model is available for Studio planning. Download or scan a chat-template GGUF first.".into())
        };
    }
    let director = if request.director_model_id.trim().is_empty() {
        select_default_studio_model(
            models,
            settings.selected_model_id.as_deref(),
            qualifications,
            settings,
        )?
        .ok_or_else(|| "No local Studio director model is available.".to_string())?
    } else {
        models
            .iter()
            .find(|model| model.id == request.director_model_id)
            .ok_or_else(|| {
                "The selected Studio director is no longer in the local model catalog. Scan models again or choose another model.".to_string()
            })?
    };
    let reviewer = if request.reviewer_model_id.trim().is_empty() {
        director
    } else {
        models
            .iter()
            .find(|model| model.id == request.reviewer_model_id)
            .ok_or_else(|| {
                "The selected Studio reviewer is no longer in the local model catalog. Scan models again or choose another model.".to_string()
            })?
    };
    let director_compatibility = qualifications.assess(director, settings)?;
    let reviewer_compatibility = qualifications.assess(reviewer, settings)?;
    for (role, compatibility) in [
        ("director", &director_compatibility),
        ("reviewer", &reviewer_compatibility),
    ] {
        if matches!(
            compatibility.tier.as_str(),
            "incompatible" | "limited-context"
        ) {
            return Err(format!(
                "The selected Studio {role} cannot be used: {}",
                compatibility.detail
            ));
        }
        if !producer_authored && !advanced && !compatibility.studio_ready {
            return Err(format!(
                "The selected Studio {role} has not passed Kestrel's local protocol check. Run Check for Studio first, or enable Advanced mode for an explicitly supervised trial."
            ));
        }
    }
    Ok(MovieModelRoles {
        director: model_binding(director, &director_compatibility),
        reviewer: model_binding(reviewer, &reviewer_compatibility),
    })
}

fn project_model_ids(
    project: &MovieProject,
    models: &[ModelInfo],
    settings: &ControlSettings,
    qualifications: &ModelQualificationStore,
    advanced: bool,
) -> Result<(String, String), String> {
    let legacy_bonsai = || {
        models
            .iter()
            .find(|model| is_bonsai(model))
            .map(|model| model.id.clone())
            .ok_or_else(|| {
                "This legacy Studio project requires its Bonsai model. Scan or restore that model before resuming.".to_string()
            })
    };
    let resolve = |role: &str, model_id: &str| -> Result<String, String> {
        let resolved = if model_id.trim().is_empty() {
            legacy_bonsai()?
        } else {
            model_id.to_string()
        };
        let model = models
            .iter()
            .find(|model| model.id == resolved)
            .ok_or_else(|| {
                format!(
                    "This project is pinned to a Studio {role} that is not currently available. Scan or restore the exact model before continuing; Kestrel will not silently substitute another model."
                )
            })?;
        let compatibility = qualifications.assess(model, settings)?;
        if matches!(
            compatibility.tier.as_str(),
            "incompatible" | "limited-context"
        ) {
            return Err(format!(
                "The project's pinned Studio {role} cannot be used: {}",
                compatibility.detail
            ));
        }
        if !advanced && !compatibility.studio_ready {
            return Err(format!(
                "The project's pinned Studio {role} no longer has a valid local protocol receipt for the current engine and runtime profile. Run Check for Studio again before using agent help."
            ));
        }
        Ok(resolved)
    };
    Ok((
        resolve("director", &project.model_roles.director.model_id)?,
        resolve("reviewer", &project.model_roles.reviewer.model_id)?,
    ))
}

#[tauri::command]
async fn list_studio_model_compatibility(
    state: State<'_, AppState>,
) -> Result<Vec<ModelCompatibility>, String> {
    let (_, settings, models) = studio_model_context(&state).await?;
    state.model_qualifications.assess_all(&models, &settings)
}

#[tauri::command]
async fn qualify_studio_model(
    model_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ModelCompatibility, String> {
    let _guard = claim_workspace(&state)?;
    let (research, settings, models) = studio_model_context(&state).await?;
    let model = models
        .iter()
        .find(|model| model.id == model_id)
        .ok_or_else(|| "The selected model is no longer in the local catalog.".to_string())?;
    let current = state.model_qualifications.assess(model, &settings)?;
    if matches!(current.tier.as_str(), "incompatible" | "limited-context") {
        return Err(current.detail);
    }
    state.studio.release_comfy_memory().await;
    state
        .runtime
        .stop_managed()
        .await
        .map_err(|error| error.to_string())?;
    services::stop_bonsai(&research.bonsai_root)
        .await
        .map_err(|error| error.to_string())?;
    let lease = state
        .runtime
        .lease_model(&model_id, &models, &settings, Some(&app))
        .await
        .map_err(|error| error.to_string())?;
    let checked = state
        .studio
        .qualify_model_protocol(&lease.connection, settings.max_output_tokens)
        .await;
    drop(lease);
    let _ = state.runtime.stop_managed().await;
    let (passed, checks, detail) = match checked {
        Ok(checks) => (
            true,
            checks,
            "Passed Kestrel's local structured Studio protocol check.".to_string(),
        ),
        Err(error) => (false, Vec::new(), error.to_string()),
    };
    state.model_qualifications.record(qualification_receipt(
        model, &settings, passed, checks, detail,
    )?)?;
    state.model_qualifications.assess(model, &settings)
}

#[tauri::command]
async fn start_movie(
    request: StartMovieRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    let (research, runtime_settings, models) = studio_model_context(&state).await?;
    let roles = resolve_movie_model_roles(
        &request.model_roles,
        false,
        research.advanced_mode || runtime_settings.advanced_mode,
        &models,
        &runtime_settings,
        &state.model_qualifications,
    )?;
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| {
            "Chat, research, a computer task, or another movie production is already active."
                .to_string()
        })?;
    let project = state
        .studio
        .create(
            request,
            research.advanced_mode || runtime_settings.advanced_mode,
            roles,
        )
        .map_err(|error| {
            state.work_active.store(false, Ordering::Release);
            error.to_string()
        })?;
    let cancel = CancellationToken::new();
    state
        .movie_jobs
        .lock()
        .map_err(|_| {
            state.work_active.store(false, Ordering::Release);
            "movie job registry is unavailable".to_string()
        })?
        .insert(project.id.clone(), cancel.clone());
    spawn_movie(app, project.id.clone(), research, cancel, true);
    Ok(project)
}

#[tauri::command]
async fn start_manual_movie(
    request: StartMovieRequest,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    let (research, runtime_settings, models) = studio_model_context(&state).await?;
    let roles = resolve_movie_model_roles(
        &request.model_roles,
        true,
        research.advanced_mode || runtime_settings.advanced_mode,
        &models,
        &runtime_settings,
        &state.model_qualifications,
    )?;
    let _guard = claim_workspace(&state)?;
    state
        .studio
        .create_manual(
            request,
            research.advanced_mode || runtime_settings.advanced_mode,
            roles,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn resume_movie(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    ensure_workspace_idle(&state)?;
    let project = state.studio.get(&id).map_err(|error| error.to_string())?;
    if project.status == "awaiting-review" {
        return Err(
            "This production is paused for human review. Approve the structured plan to begin H3 rendering."
                .into(),
        );
    }
    let needs_plan = project.plan.is_none() || project.status == "planning-checkpoint";
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another local AI job is already active.".to_string())?;
    let project = state
        .studio
        .begin_resume(&id, Some(&app))
        .map_err(|error| error.to_string())?;
    let research = state.research_settings.load().map_err(|error| {
        state.work_active.store(false, Ordering::Release);
        error.to_string()
    })?;
    let cancel = CancellationToken::new();
    state
        .movie_jobs
        .lock()
        .map_err(|_| {
            state.work_active.store(false, Ordering::Release);
            "movie job registry is unavailable".to_string()
        })?
        .insert(id.clone(), cancel.clone());
    spawn_movie(app, id, research, cancel, needs_plan);
    Ok(project)
}

fn spawn_movie(
    app: AppHandle,
    id: String,
    research: ResearchSettings,
    cancel: CancellationToken,
    needs_plan: bool,
) {
    tauri::async_runtime::spawn(async move {
        let managed = app.state::<AppState>();
        let _guard = WorkGuard(&managed.work_active);
        let result: Result<(), String> = async {
            if needs_plan {
                managed.studio.release_comfy_memory().await;
                let (_, runtime_settings, models) = studio_model_context(&managed).await?;
                let project = managed.studio.get(&id).map_err(|error| error.to_string())?;
                let (director_model_id, reviewer_model_id) = project_model_ids(
                    &project,
                    &models,
                    &runtime_settings,
                    &managed.model_qualifications,
                    research.advanced_mode || runtime_settings.advanced_mode,
                )?;
                managed
                    .runtime
                    .stop_managed()
                    .await
                    .map_err(|error| error.to_string())?;
                services::stop_bonsai(&research.bonsai_root)
                    .await
                    .map_err(|error| error.to_string())?;
                let planned = managed
                    .studio
                    .plan(
                        &id,
                        MovieModelRuntime {
                            runtime: &managed.runtime,
                            models: &models,
                            settings: &runtime_settings,
                            director_model_id: &director_model_id,
                            reviewer_model_id: &reviewer_model_id,
                        },
                        &cancel,
                        Some(&app),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                if matches!(
                    planned.status.as_str(),
                    "awaiting-review" | "planning-checkpoint"
                ) {
                    let _ = managed.runtime.stop_managed().await;
                    let _ = services::stop_bonsai(&research.bonsai_root).await;
                    return Ok(());
                }
            }
            managed
                .runtime
                .stop_managed()
                .await
                .map_err(|error| error.to_string())?;
            services::stop_bonsai(&research.bonsai_root)
                .await
                .map_err(|error| error.to_string())?;
            managed
                .studio
                .render(&id, &cancel, Some(&app))
                .await
                .map_err(|error| error.to_string())?;
            Ok(())
        }
        .await;
        if let Err(error) = result {
            if cancel.is_cancelled() {
                let _ = managed.studio.stop(&id, Some(&app));
            } else {
                managed.studio.fail(&id, error, Some(&app));
            }
        }
        if let Ok(mut jobs) = managed.movie_jobs.lock() {
            jobs.remove(&id);
        };
    });
}

#[tauri::command]
fn cancel_movie(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    if let Some(cancel) = state
        .movie_jobs
        .lock()
        .map_err(|_| "movie job registry is unavailable".to_string())?
        .get(&id)
    {
        cancel.cancel();
    }
    state
        .studio
        .stop(&id, Some(&app))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_movie_planning(
    id: String,
    state: State<'_, AppState>,
) -> Result<MoviePlanningSnapshot, String> {
    state
        .studio
        .planning_snapshot(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn direct_movie_planning(
    id: String,
    text: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MoviePlanningSnapshot, String> {
    state
        .studio
        .queue_planning_direction(&id, &text, Some(&app))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn checkpoint_movie_planning(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MoviePlanningSnapshot, String> {
    state
        .studio
        .request_planning_checkpoint(&id, Some(&app))
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_movie_prompt_draft(
    request: PromptDraftRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    ensure_workspace_idle(&state)?;
    let models = state.models.read().await.clone();
    studio::validate_prompt_draft_request(&request, &models)?;
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another local AI job is already active.".to_string())?;
    let cancel = CancellationToken::new();
    match state.interactive_jobs.lock() {
        Ok(mut jobs) => {
            jobs.insert(request.request_id.clone(), cancel.clone());
        }
        Err(_) => {
            state.work_active.store(false, Ordering::Release);
            return Err("interactive job registry is unavailable".into());
        }
    }
    let request_id = request.request_id.clone();
    let event_request_id = request_id.clone();
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let managed = app_for_task.state::<AppState>();
        let work_guard = WorkGuard(&managed.work_active);
        let result = match managed.control_settings.load() {
            Ok(settings) => {
                PromptDraftJob {
                    app: app_for_task.clone(),
                    runtime: managed.runtime.clone(),
                    models,
                    settings,
                    request,
                    cancel,
                }
                .run()
                .await
            }
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = result {
            studio::emit_prompt_draft_error(&app_for_task, &event_request_id, error);
        }
        if let Ok(mut jobs) = managed.interactive_jobs.lock() {
            jobs.remove(&event_request_id);
        }
        drop(work_guard);
        studio::emit_prompt_draft_settled(&app_for_task, &event_request_id);
    });
    Ok(request_id)
}

#[tauri::command]
async fn start_movie_copilot(
    request: MovieCopilotRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    ensure_workspace_idle(&state)?;
    let models = state.models.read().await.clone();
    let project = state
        .studio
        .get(&request.project_id)
        .map_err(|error| error.to_string())?;
    studio::validate_copilot_request(&request, &models, &project)?;
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another local AI job is already active.".to_string())?;
    let cancel = CancellationToken::new();
    match state.interactive_jobs.lock() {
        Ok(mut jobs) => {
            jobs.insert(request.request_id.clone(), cancel.clone());
        }
        Err(_) => {
            state.work_active.store(false, Ordering::Release);
            return Err("interactive job registry is unavailable".into());
        }
    }
    let request_id = request.request_id.clone();
    let project_id = request.project_id.clone();
    let event_request_id = request_id.clone();
    let event_project_id = project_id.clone();
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let managed = app_for_task.state::<AppState>();
        let work_guard = WorkGuard(&managed.work_active);
        let result = match managed.control_settings.load() {
            Ok(settings) => {
                MovieCopilotJob {
                    app: app_for_task.clone(),
                    studio: managed.studio.clone(),
                    runtime: managed.runtime.clone(),
                    models,
                    settings,
                    request,
                    cancel,
                }
                .run()
                .await
            }
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = result {
            studio::emit_copilot_error(&app_for_task, &event_request_id, &event_project_id, error);
        }
        if let Ok(mut jobs) = managed.interactive_jobs.lock() {
            jobs.remove(&event_request_id);
        }
        drop(work_guard);
        studio::emit_copilot_settled(&app_for_task, &event_request_id, &event_project_id);
    });
    Ok(request_id)
}

#[tauri::command]
fn cancel_movie_copilot(request_id: String, state: State<'_, AppState>) -> Result<(), String> {
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
fn get_movie_copilot_receipt(
    project_id: String,
    request_id: String,
    state: State<'_, AppState>,
) -> Result<MovieCopilotReceipt, String> {
    state
        .studio
        .copilot_receipt(&project_id, &request_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn cancel_movie_prompt_draft(request_id: String, state: State<'_, AppState>) -> Result<(), String> {
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
async fn save_movie_edits(
    id: String,
    edit: MovieEdit,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    state
        .studio
        .save_edits(&id, edit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn save_movie_plan(
    id: String,
    plan: MoviePlan,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    ensure_workspace_idle(&state)?;
    state
        .studio
        .save_producer_plan(&id, plan, Some(&app))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn set_movie_model_roles(
    id: String,
    model_roles: MovieModelRoleRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    let (research, runtime_settings, models) = studio_model_context(&state).await?;
    let roles = resolve_movie_model_roles(
        &model_roles,
        false,
        research.advanced_mode || runtime_settings.advanced_mode,
        &models,
        &runtime_settings,
        &state.model_qualifications,
    )?;
    let _guard = claim_workspace(&state)?;
    state
        .studio
        .set_model_roles(&id, roles, Some(&app))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_movie_plan_exchange_prompt(
    id: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    state
        .studio
        .movie_plan_exchange_prompt(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn parse_movie_plan_exchange(
    id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<MoviePlan, String> {
    state
        .studio
        .parse_movie_plan_exchange(&id, &text)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn revise_movie_plan(
    request: MoviePlanFeedbackRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    let _guard = claim_workspace(&state)?;
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let (_, runtime_settings, models) = studio_model_context(&state).await?;
    let project = state
        .studio
        .get(&request.id)
        .map_err(|error| error.to_string())?;
    let (director_model_id, reviewer_model_id) = project_model_ids(
        &project,
        &models,
        &runtime_settings,
        &state.model_qualifications,
        research.advanced_mode || runtime_settings.advanced_mode,
    )?;
    state.studio.release_comfy_memory().await;
    state
        .runtime
        .stop_managed()
        .await
        .map_err(|error| error.to_string())?;
    services::stop_bonsai(&research.bonsai_root)
        .await
        .map_err(|error| error.to_string())?;
    let cancel = CancellationToken::new();
    state
        .movie_jobs
        .lock()
        .map_err(|_| "movie job registry is unavailable".to_string())?
        .insert(request.id.clone(), cancel.clone());
    let result: Result<MovieProject, String> = async {
        state
            .studio
            .revise_with_producer_feedback(
                &request.id,
                &request.feedback,
                MovieModelRuntime {
                    runtime: &state.runtime,
                    models: &models,
                    settings: &runtime_settings,
                    director_model_id: &director_model_id,
                    reviewer_model_id: &reviewer_model_id,
                },
                &cancel,
                Some(&app),
            )
            .await
            .map_err(|error| error.to_string())
    }
    .await;
    let _ = state.runtime.stop_managed().await;
    let _ = services::stop_bonsai(&research.bonsai_root).await;
    if let Ok(mut jobs) = state.movie_jobs.lock() {
        jobs.remove(&request.id);
    }
    result
}

#[tauri::command]
async fn approve_movie_plan(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    ensure_workspace_idle(&state)?;
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another local AI job is already active.".to_string())?;
    let project = state
        .studio
        .approve_producer_plan(&id, Some(&app))
        .await
        .map_err(|error| {
            state.work_active.store(false, Ordering::Release);
            error.to_string()
        })?;
    let research = state.research_settings.load().map_err(|error| {
        state.work_active.store(false, Ordering::Release);
        error.to_string()
    })?;
    let cancel = CancellationToken::new();
    state
        .movie_jobs
        .lock()
        .map_err(|_| {
            state.work_active.store(false, Ordering::Release);
            "movie job registry is unavailable".to_string()
        })?
        .insert(id.clone(), cancel.clone());
    spawn_movie(app, id, research, cancel, false);
    Ok(project)
}

#[tauri::command]
async fn ask_movie_director_clip(
    request: MovieClipAssistRequest,
    state: State<'_, AppState>,
) -> Result<MovieClipSuggestion, String> {
    let _guard = claim_workspace(&state)?;
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let (_, runtime_settings, models) = studio_model_context(&state).await?;
    let project = state
        .studio
        .get(&request.id)
        .map_err(|error| error.to_string())?;
    let (director_model_id, _) = project_model_ids(
        &project,
        &models,
        &runtime_settings,
        &state.model_qualifications,
        research.advanced_mode || runtime_settings.advanced_mode,
    )?;
    state.studio.release_comfy_memory().await;
    state
        .runtime
        .stop_managed()
        .await
        .map_err(|error| error.to_string())?;
    services::stop_bonsai(&research.bonsai_root)
        .await
        .map_err(|error| error.to_string())?;
    let result: Result<MovieClipSuggestion, String> = async {
        let lease = state
            .runtime
            .lease_model(&director_model_id, &models, &runtime_settings, None)
            .await
            .map_err(|error| error.to_string())?;
        state
            .studio
            .assist_clip(
                &request.id,
                &request.clip_id,
                &request.feedback,
                &lease.connection,
                runtime_settings.max_output_tokens,
            )
            .await
            .map_err(|error| error.to_string())
    }
    .await;
    let _ = state.runtime.stop_managed().await;
    let _ = services::stop_bonsai(&research.bonsai_root).await;
    result
}

#[tauri::command]
async fn render_movie_clip_version(
    request: MovieClipRenderRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    let _guard = claim_workspace(&state)?;
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let _ = state.runtime.stop_managed().await;
    let _ = services::stop_bonsai(&research.bonsai_root).await;
    let cancel = CancellationToken::new();
    state
        .movie_jobs
        .lock()
        .map_err(|_| "movie job registry is unavailable".to_string())?
        .insert(request.id.clone(), cancel.clone());
    let id = request.id.clone();
    let result = state
        .studio
        .render_clip_version(request, &cancel, Some(&app))
        .await
        .map_err(|error| error.to_string());
    if let Ok(mut jobs) = state.movie_jobs.lock() {
        jobs.remove(&id);
    }
    result
}

#[tauri::command]
async fn render_movie_edit(id: String, state: State<'_, AppState>) -> Result<MovieProject, String> {
    let _guard = claim_workspace(&state)?;
    state
        .studio
        .render_edit(&id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn reveal_movie(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let project = state.studio.get(&id).map_err(|error| error.to_string())?;
    let path = state.studio.project_dir(&project.id);
    open_with_explorer(&path)
}

#[tauri::command]
async fn prepare_services(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let _guard = claim_workspace(&state)?;
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    services::prepare(&settings)
        .await
        .map_err(|error| error.to_string())?;
    if state
        .runtime
        .attach_external_if_ready(&settings)
        .await
        .is_none()
    {
        let control = state
            .control_settings
            .load()
            .map_err(|error| error.to_string())?;
        let model = runtime_bonsai_model(&state).await?;
        state
            .runtime
            .start_model(&model, &control, Some(&app))
            .await
            .map_err(|error| error.to_string())?;
    }
    identify_attached_bonsai(&state).await;
    snapshot(&state).await
}

async fn runtime_bonsai_model(state: &AppState) -> Result<ModelInfo, String> {
    state
        .models
        .read()
        .await
        .iter()
        .find(|model| {
            format!("{} {}", model.name, model.path)
                .to_lowercase()
                .contains("bonsai")
        })
        .cloned()
        .ok_or_else(|| "Bonsai is not installed or has not been scanned. Open Setup and install or locate the assistant first.".into())
}

#[tauri::command]
async fn get_setup_snapshot(state: State<'_, AppState>) -> Result<setup::SetupSnapshot, String> {
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let gpu = services::gpu_snapshot().await;
    Ok(setup::snapshot(&research, &control, gpu.as_ref()))
}

#[tauri::command]
async fn open_comfy_ui(state: State<'_, AppState>) -> Result<(), String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    services::start_comfy(&settings.comfy_root)
        .await
        .map_err(|error| error.to_string())?;
    std::process::Command::new("explorer.exe")
        .arg("http://127.0.0.1:8188/")
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            format!("ComfyUI is ready, but its web interface could not be opened: {error}")
        })
}

#[tauri::command]
async fn save_setup_locations(
    locations: setup::SetupLocations,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    ensure_workspace_idle(&state)?;
    let mut research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let mut control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    setup::apply_locations(&mut research, &mut control, locations)
        .map_err(|error| error.to_string())?;
    let comfy_root = std::path::Path::new(&research.comfy_root);
    if comfy_root.join("main.py").is_file()
        && !comfy_root.join("Start-ComfyUI-MiniMax-H3.ps1").is_file()
    {
        setup::ensure_comfy_launcher(comfy_root).map_err(|error| error.to_string())?;
    }
    state
        .research_settings
        .save(&research)
        .map_err(|error| error.to_string())?;
    state
        .control_settings
        .save(&control)
        .map_err(|error| error.to_string())?;
    apply_media_paths(&research);
    refresh_engine_candidates(&state, &control, &research).await;
    snapshot(&state).await
}

#[tauri::command]
async fn pick_setup_folder() -> Result<String, String> {
    Ok(rfd::AsyncFileDialog::new()
        .set_title("Choose where Kestrel should keep its AI components")
        .pick_folder()
        .await
        .map(|folder| folder.path().to_string_lossy().into_owned())
        .unwrap_or_default())
}

#[tauri::command]
async fn pick_setup_file(kind: String) -> Result<String, String> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title("Choose an existing Kestrel component");
    dialog = match kind.as_str() {
        "zim" => dialog.add_filter("Kiwix archive", &["zim"]),
        "engine" | "ffmpeg" | "ffprobe" => dialog.add_filter("Windows program", &["exe"]),
        _ => dialog,
    };
    Ok(dialog
        .pick_file()
        .await
        .map(|file| file.path().to_string_lossy().into_owned())
        .unwrap_or_default())
}

#[tauri::command]
async fn install_setup_component(
    request: setup::SetupInstallRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    ensure_workspace_idle(&state)?;
    let mut research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let cancel = CancellationToken::new();
    {
        let mut active = state
            .setup_job
            .lock()
            .map_err(|_| "setup job registry is unavailable".to_string())?;
        if active.is_some() {
            return Err("Another setup download is already active.".into());
        }
        *active = Some(cancel.clone());
    }
    let result = setup::install_component(&app, &mut research, &request, cancel).await;
    if let Ok(mut active) = state.setup_job.lock() {
        *active = None;
    }
    result.map_err(|error| error.to_string())?;
    state
        .research_settings
        .save(&research)
        .map_err(|error| error.to_string())?;
    apply_media_paths(&research);
    let mut control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    if request.component == "assistant" {
        control.engine_path = std::path::Path::new(&research.bonsai_root)
            .join("runtime")
            .join("llama-server.exe")
            .to_string_lossy()
            .into_owned();
        state
            .control_settings
            .save(&control)
            .map_err(|error| error.to_string())?;
        let roots = model_roots(&state, &control, &research);
        let found = tokio::task::spawn_blocking(move || model::scan(&roots))
            .await
            .map_err(|error| format!("model scan failed after setup: {error}"))?;
        state
            .model_catalog
            .save(&found)
            .map_err(|error| error.to_string())?;
        *state.models.write().await = found;
    }
    refresh_engine_candidates(&state, &control, &research).await;
    snapshot(&state).await
}

#[tauri::command]
fn cancel_setup_install(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state
        .setup_job
        .lock()
        .map_err(|_| "setup job registry is unavailable".to_string())?
        .as_ref()
    {
        cancel.cancel();
    }
    Ok(())
}

#[tauri::command]
async fn get_system_snapshot(state: State<'_, AppState>) -> Result<SystemSnapshot, String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let mut value = services::system_snapshot(settings).await;
    if state.runtime.snapshot().await.phase == "ready" {
        value.status.bonsai = "ready".into();
    }
    Ok(value)
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
    let roots = model_roots(&state, &control, &research);
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
fn list_model_downloads(state: State<'_, AppState>) -> Result<Vec<ModelDownloadRecord>, String> {
    state.model_downloads.list()
}

#[tauri::command]
async fn inspect_model_download(
    url: String,
    state: State<'_, AppState>,
) -> Result<ModelDownloadInspection, String> {
    if state
        .setup_job
        .lock()
        .map_err(|_| "setup job registry is unavailable".to_string())?
        .is_some()
    {
        return Err(
            "A setup download is active. Stop or finish it before inspecting a public model repository."
                .into(),
        );
    }
    let _guard = claim_workspace(&state).map_err(|_| {
        "Chat, research, a computer task, or a model transfer is active. Stop or finish it before inspecting a public model repository.".to_string()
    })?;
    state.model_downloads.inspect(&url).await
}

#[tauri::command]
async fn start_model_download(
    request: ModelDownloadRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ModelDownloadRecord, String> {
    run_model_download(Some(request), None, app, state).await
}

#[tauri::command]
async fn resume_model_download(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ModelDownloadRecord, String> {
    run_model_download(None, Some(id), app, state).await
}

#[tauri::command]
fn cancel_model_download(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state
        .model_download_job
        .lock()
        .map_err(|_| "model download job registry is unavailable".to_string())?
        .as_ref()
    {
        cancel.cancel();
    }
    Ok(())
}

async fn run_model_download(
    request: Option<ModelDownloadRequest>,
    resume_id: Option<String>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ModelDownloadRecord, String> {
    if state
        .setup_job
        .lock()
        .map_err(|_| "setup job registry is unavailable".to_string())?
        .is_some()
    {
        return Err(
            "A setup download is active. Stop or finish it before starting a model transfer."
                .into(),
        );
    }
    let _guard = claim_workspace(&state).map_err(|_| {
        "Chat, research, a computer task, or another model transfer is active. Stop or finish it before using the public model downloader.".to_string()
    })?;
    let _awake = model_download::SystemAwakeGuard::acquire()?;
    let cancel = CancellationToken::new();
    {
        let mut active = state
            .model_download_job
            .lock()
            .map_err(|_| "model download job registry is unavailable".to_string())?;
        if active.is_some() {
            return Err("Another model transfer is already active.".into());
        }
        *active = Some(cancel.clone());
    }
    let result = match (request, resume_id) {
        (Some(request), None) => state.model_downloads.start(request, &app, &cancel).await,
        (None, Some(id)) => state.model_downloads.resume(&id, &app, &cancel).await,
        _ => Err("invalid model download action".into()),
    };
    if let Ok(mut active) = state.model_download_job.lock() {
        *active = None;
    }
    let record = result?;
    if record.status == "complete" {
        refresh_model_catalog(&state, Some(&app)).await?;
    }
    Ok(record)
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
    let roots = model_roots(&state, &imported.control, &imported.research);
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
    app: AppHandle,
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
    let script = std::path::Path::new(&settings.bonsai_root).join("Start-BonsaiServer.ps1");
    if script.is_file() {
        services::restart_bonsai(&settings.bonsai_root)
            .await
            .map_err(|error| error.to_string())?;
        let _ = state.runtime.attach_external_if_ready(&settings).await;
    } else {
        let model = runtime_bonsai_model(&state).await?;
        state
            .runtime
            .start_model(&model, &control, Some(&app))
            .await
            .map_err(|error| error.to_string())?;
    }
    identify_attached_bonsai(&state).await;
    let mut value = services::system_snapshot(settings).await;
    if state.runtime.snapshot().await.phase == "ready" {
        value.status.bonsai = "ready".into();
    }
    Ok(value)
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
        let image_assets = state
            .image_asset_jobs
            .lock()
            .map_err(|_| "image asset job registry is unavailable".to_string())?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let speech = state
            .speech_jobs
            .lock()
            .map_err(|_| "local speech job registry is unavailable".to_string())?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        research
            .into_iter()
            .chain(interactive)
            .chain(image_assets)
            .chain(speech)
            .collect::<Vec<_>>()
    };
    *state
        .speech_restore_model
        .lock()
        .map_err(|_| "Local speech restore state is unavailable".to_string())? = None;
    for cancellation in cancellations {
        cancellation.cancel();
    }
    for attempt in 0..80 {
        let speech_idle = state
            .speech_jobs
            .lock()
            .map_err(|_| "local speech job registry is unavailable".to_string())?
            .is_empty();
        if speech_idle {
            break;
        }
        if attempt == 79 {
            return Err("The local speech job did not stop within 20 seconds. Its cancellation remains requested; try Release AI memory again after the visible speech status settles.".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
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
    state.studio.release_comfy_memory().await;
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
    spawn_computer_task(app, run.id.clone(), request, settings, cancel, None);
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
    );
    Ok(updated)
}

fn spawn_computer_task(
    app: AppHandle,
    run_id: String,
    request: ComputerTaskRequest,
    settings: ControlSettings,
    cancel: CancellationToken,
    continuation: Option<String>,
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
    let mut transcript = String::new();
    for event in run
        .events
        .iter()
        .filter(|event| event.kind != "reasoning")
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
    format!(
        "Continue the same durable task. Re-inspect current state before further changes because the prior run may have stopped between actions.\n\nRecent verified transcript:\n{transcript}\nUser's new direction:\n{answer}"
    )
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
    let reports = state
        .store
        .list(100_000)
        .map_err(|error| error.to_string())?;
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let mut status = services::status(&settings).await;
    if state.runtime.snapshot().await.phase == "ready" {
        status.bonsai = "ready".into();
    }
    let control = control_snapshot(state, false).await?;
    let setup = setup::snapshot(&settings, &control.settings, control.gpu.as_ref());
    Ok(AppSnapshot {
        status,
        reports,
        library_root: state.store.root().to_string_lossy().into_owned(),
        settings,
        control,
        setup,
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

fn model_roots(
    state: &AppState,
    control: &ControlSettings,
    research: &ResearchSettings,
) -> Vec<std::path::PathBuf> {
    let mut roots = default_roots(&control.extra_model_roots, &research.bonsai_root);
    roots.push(state.model_downloads.models_root().to_path_buf());
    roots.sort();
    roots.dedup();
    roots
}

async fn refresh_model_catalog(
    state: &AppState,
    app: Option<&AppHandle>,
) -> Result<Vec<ModelInfo>, String> {
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let roots = model_roots(state, &control, &research);
    let found = tokio::task::spawn_blocking(move || model::scan(&roots))
        .await
        .map_err(|error| format!("model scan failed after download: {error}"))?;
    state.model_catalog.save(&found).map_err(|error| {
        format!("the model downloaded, but its catalog could not be saved: {error}")
    })?;
    *state.models.write().await = found.clone();
    if let Some(app) = app {
        let _ = app.emit("model-catalog-updated", &found);
    }
    Ok(found)
}

fn ensure_workspace_idle(state: &AppState) -> Result<(), String> {
    if state.work_active.load(Ordering::Acquire) {
        Err("Chat, research, a computer task, movie production, or a model transfer is active. Stop or finish it before changing runtime or developer state.".into())
    } else {
        Ok(())
    }
}

fn claim_workspace(state: &AppState) -> Result<WorkGuard<'_>, String> {
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| {
            "Chat, research, a computer task, movie production, or a model transfer is active. Stop or finish it before changing runtime or developer state.".to_string()
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

fn apply_media_paths(settings: &ResearchSettings) {
    if std::path::Path::new(&settings.ffmpeg_path).is_file() {
        std::env::set_var("KESTREL_FFMPEG_PATH", &settings.ffmpeg_path);
    } else {
        std::env::remove_var("KESTREL_FFMPEG_PATH");
    }
    if std::path::Path::new(&settings.ffprobe_path).is_file() {
        std::env::set_var("KESTREL_FFPROBE_PATH", &settings.ffprobe_path);
    } else {
        std::env::remove_var("KESTREL_FFPROBE_PATH");
    }
}

pub fn run() {
    let application = tauri::Builder::default()
        .register_uri_scheme_protocol("kestrel-media", |_context, request| {
            studio::media_response(request)
        })
        .register_uri_scheme_protocol("kestrel-speech", |_context, request| {
            local_speech::media_response(request)
        })
        .setup(|app| {
            let store = ResearchStore::open_default().map_err(|error| error.to_string())?;
            let research_settings = SettingsStore::new(store.root());
            let control_settings = ControlSettingsStore::new(store.root());
            let research = research_settings
                .load()
                .map_err(|error| error.to_string())?;
            apply_media_paths(&research);
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
            let model_downloads =
                ModelDownloadManager::new(store.root()).map_err(|error| error.to_string())?;
            let model_qualifications =
                ModelQualificationStore::new(store.root()).map_err(|error| error.to_string())?;
            let models = model_catalog.load().unwrap_or_else(|error| {
                eprintln!("Kestrel could not restore its disposable model catalog: {error}");
                Vec::new()
            });
            let harness = ResearchHarness::new(store.clone());
            let developer = DeveloperAssistant::new(store.root());
            let workspace = WorkspaceStore::new(store.root())?;
            let attachments = AttachmentStore::new(&store.root().join("workspace"))?;
            let studio = MovieStudio::new(store.root()).map_err(|error| error.to_string())?;
            let speech = LocalSpeech::new(store.root()).map_err(|error| error.to_string())?;
            app.manage(AppState {
                store,
                harness,
                research_settings,
                control_settings,
                model_catalog,
                model_downloads,
                model_qualifications,
                models: RwLock::new(models),
                engine_candidates: RwLock::new(engine_candidates),
                runtime: Arc::new(RuntimeManager::new()),
                developer,
                workspace,
                attachments,
                research_active: AtomicBool::new(false),
                work_active: AtomicBool::new(false),
                jobs: Mutex::new(HashMap::new()),
                speech,
                speech_command_gate: AsyncMutex::new(()),
                speech_jobs: Mutex::new(HashMap::new()),
                speech_restore_model: Mutex::new(None),
                interactive_jobs: Mutex::new(HashMap::new()),
                studio,
                movie_jobs: Mutex::new(HashMap::new()),
                image_asset_jobs: Mutex::new(HashMap::new()),
                setup_job: Mutex::new(None),
                model_download_job: Mutex::new(None),
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
                let roots = model_roots(&state, &control, &research);
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
            get_local_speech_snapshot,
            prepare_local_speech,
            synthesize_local_speech,
            transcribe_local_speech,
            align_local_speech,
            cancel_local_speech,
            release_local_speech_memory,
            run_research,
            cancel_research,
            list_movies,
            get_movie,
            pick_movie_reference_files,
            list_movie_image_assets,
            start_movie_image_asset,
            cancel_movie_image_asset,
            list_studio_model_compatibility,
            qualify_studio_model,
            start_movie,
            start_manual_movie,
            resume_movie,
            cancel_movie,
            get_movie_planning,
            direct_movie_planning,
            checkpoint_movie_planning,
            start_movie_prompt_draft,
            cancel_movie_prompt_draft,
            start_movie_copilot,
            cancel_movie_copilot,
            get_movie_copilot_receipt,
            get_movie_plan_exchange_prompt,
            parse_movie_plan_exchange,
            save_movie_plan,
            set_movie_model_roles,
            revise_movie_plan,
            approve_movie_plan,
            ask_movie_director_clip,
            render_movie_clip_version,
            save_movie_edits,
            render_movie_edit,
            reveal_movie,
            prepare_services,
            get_setup_snapshot,
            open_comfy_ui,
            save_setup_locations,
            pick_setup_folder,
            pick_setup_file,
            install_setup_component,
            cancel_setup_install,
            open_standalone_report,
            reveal_library,
            get_system_snapshot,
            get_control_snapshot,
            scan_local_models,
            list_model_downloads,
            inspect_model_download,
            start_model_download,
            resume_model_download,
            cancel_model_download,
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
            let state = handle.state::<AppState>();
            if let Ok(active) = state.model_download_job.lock() {
                if let Some(cancel) = active.as_ref() {
                    cancel.cancel();
                }
            }
            let runtime = state.runtime.clone();
            let speech = state.speech.clone();
            let _ = tauri::async_runtime::block_on(async move {
                speech.stop_comfy().await;
                runtime.stop_managed().await
            });
        }
    });
}
