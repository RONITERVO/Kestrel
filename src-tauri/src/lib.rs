#![recursion_limit = "256"]

mod agent;
mod agent_app;
mod app_control;
mod attachments;
mod chat;
mod config;
mod developer;
mod gpu_memory;
mod hardware_profiles;
mod harness;
mod html;
mod ipc_events;
mod kiwix;
mod local_speech;
mod media_composition;
mod media_delivery;
mod model;
mod model_download;
mod models;
mod narration;
mod producer_text;
mod production_artifacts;
mod production_bundle;
mod profile;
mod prompt_catalog;
mod runtime;
mod services;
mod setup;
mod speech_preferences;
mod speech_text;
mod store;
mod strata;
mod strata_dashboard;
mod structured_output;
mod studio;
mod timed_text;
mod voice_library;
mod workspace;

#[cfg(test)]
mod test_support;

#[cfg(test)]
include!(concat!(env!("OUT_DIR"), "/command_bindings.rs"));
include!(concat!(env!("OUT_DIR"), "/app_dispatch.rs"));

use attachments::AttachmentStore;
use config::{ControlSettingsStore, SettingsStore};
use developer::DeveloperAssistant;
use harness::ResearchHarness;
use kestrel_app_core::MovieReferenceAsset;
use local_speech::{
    LocalSpeech, SpeechAlignmentRequest, SpeechClip, SpeechFileRangeTranscriptionRequest,
    SpeechFileTranscriptionRequest, SpeechSnapshot, SpeechSynthesisRequest, SpeechTranscription,
    SpeechTranscriptionRequest,
};
use model::{default_roots, merge_catalogs, ModelCatalogStore, ModelInfo};
use model_download::{
    ModelDownloadInspection, ModelDownloadManager, ModelDownloadRecord, ModelDownloadRequest,
};
use models::{
    AcceptMovieStoryRevisionRequest, AppSnapshot, AttachMovieProducerReferencesRequest,
    ChatSession, ChatSessionSummary, ChatStart, ComputerTaskAccess, ComputerTaskRequest,
    ComputerTaskRun, ComputerTaskSummary, ControlSettings, ControlSnapshot,
    CreateMovieProducerProjectRequest, DeveloperRepairReport, DeveloperRepairRequest,
    MovieProducerWorkspace, MovieStudioChatRequest, MovieStudioConversation, ProfileTransfer,
    ResearchReport, ResearchSettings, ResetMovieStudioConversationRequest,
    ResumeComputerTaskRequest, RunResearchRequest, SaveMovieScenesRequest,
    SaveMovieStoryRevisionRequest, StartChatRequest, SummarizeMovieStudioConversationRequest,
    SystemSnapshot,
};
use runtime::RuntimeManager;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use store::ResearchStore;
use studio::summarize_studio_conversation;
use studio::{
    ComfyWorkload, CreateImageProjectRequest, CreateMusicProjectRequest, ImageProject, ImageStudio,
    ImageSummary, MovieEdit, MovieImageAssetGeneration, MovieProject, MovieReferenceImport,
    MovieRenderState, MovieStudio, MovieStudioChatJob, MovieSummary, MusicLyricsRequest,
    MusicLyricsSaveResult, MusicMidiRequest, MusicMidiSaveResult, MusicProject, MusicStudio,
    MusicSummary, PromptDraftJob, PromptDraftRequest, RepairMusicLyricsRangeRequest,
    SaveMusicLyricsDocumentRequest, SaveMusicMidiDocumentRequest, TranscribeMusicLyricsRequest,
};
use tauri::{AppHandle, Manager, State};
use tokio::sync::{Mutex as AsyncMutex, RwLock};
use tokio_util::sync::CancellationToken;
use voice_library::{
    CreateVoiceProfileRequest, UpdateVoiceProfileRequest, VoiceLibrary, VoiceLibrarySnapshot,
};
use workspace::WorkspaceStore;

/// Shared native state. Keep authority visibly separated: research owns evidence/storage, runtime
/// owns the only model process, and developer owns the optional Codex child.
struct AppState {
    control: app_control::AppControl,
    speech_preferences: speech_preferences::SpeechPreferencesStore,
    store: ResearchStore,
    harness: ResearchHarness,
    research_settings: SettingsStore,
    control_settings: ControlSettingsStore,
    model_catalog: ModelCatalogStore,
    model_downloads: ModelDownloadManager,
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
    voice_library: VoiceLibrary,
    speech_command_gate: AsyncMutex<()>,
    speech_jobs: Mutex<HashMap<String, CancellationToken>>,
    speech_restore_model: Mutex<Option<SpeechRuntimeRestore>>,
    interactive_jobs: Mutex<HashMap<String, CancellationToken>>,
    studio: MovieStudio,
    music: MusicStudio,
    images: ImageStudio,
    movie_jobs: Mutex<HashMap<String, CancellationToken>>,
    music_jobs: Mutex<HashMap<String, CancellationToken>>,
    image_generation_jobs: Mutex<HashMap<String, CancellationToken>>,
    setup_job: Mutex<Option<CancellationToken>>,
    model_download_job: Mutex<Option<CancellationToken>>,
}

async fn release_all_comfy_memory(state: &AppState) {
    tokio::join!(
        state.studio.release_comfy_memory(),
        state.music.release_comfy_memory(),
        // Whisper is cached outside ComfyUI's ordinary model manager. /free alone leaves it
        // resident when narration hands the machine back to the language model.
        state.speech.release_model_memory()
    );
}

#[derive(Clone)]
struct SpeechRuntimeRestore {
    model_id: String,
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

use kestrel_app_core::ContextAttachmentImport;

impl Drop for WorkGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[tauri::command]
fn list_app_actions(filter: Option<String>) -> Vec<String> {
    app_control::catalog(filter.as_deref())
}

#[tauri::command]
fn describe_app_action(command: String) -> Result<kestrel_app_core::AppActionDescription, String> {
    app_control::describe(&command)
}

#[tauri::command]
async fn execute_app_action(
    request: kestrel_app_core::AppActionRequest,
    app: AppHandle,
) -> Result<kestrel_app_core::AppActionReceipt, String> {
    app_control::execute(&app, request).await
}

#[tauri::command]
fn get_app_action_receipt(
    request_id: String,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::AppActionReceipt, String> {
    state.control.receipt(&request_id)
}

#[tauri::command]
fn get_app_job(
    id: String,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::AppJobObservation, String> {
    state.control.job(&id)
}

#[tauri::command]
fn create_producer_workflow(
    plan: kestrel_app_core::ProducerWorkflowPlan,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::ProducerWorkflowRun, String> {
    state.control.create_workflow(plan)
}

#[tauri::command]
fn get_producer_workflow(
    id: String,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::ProducerWorkflowRun, String> {
    state.control.get_workflow(&id)
}

#[tauri::command]
fn list_producer_workflows(
    state: State<'_, AppState>,
) -> Result<Vec<kestrel_app_core::ProducerWorkflowRun>, String> {
    state.control.list_workflows()
}

#[tauri::command]
fn start_producer_workflow(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::ProducerWorkflowRun, String> {
    ensure_workspace_idle(&state)?;
    app_control::start_workflow(app, &id)
}

#[tauri::command]
fn stop_producer_workflow(id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state
        .control
        .workflows
        .lock()
        .map_err(|e| e.to_string())?
        .get(&id)
    {
        cancel.cancel();
    }
    Ok(())
}

#[tauri::command]
async fn import_context_paths(
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<ContextAttachmentImport, String> {
    let paths = app_control::import_paths(paths)?;
    let store = state.attachments.clone();
    tokio::task::spawn_blocking(move || {
        let mut attachments = Vec::new();
        let mut failures = Vec::new();
        for path in paths {
            match store.import_path(&path) {
                Ok(value) => attachments.push(value),
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }
        ContextAttachmentImport {
            attachments,
            failures,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn import_movie_reference_paths(
    paths: Vec<String>,
    state: State<'_, AppState>,
) -> Result<MovieReferenceImport, String> {
    let paths = app_control::import_paths(paths)?;
    let studio = state.studio.clone();
    tokio::task::spawn_blocking(move || {
        let mut references = Vec::new();
        let mut failures = Vec::new();
        for path in paths {
            match studio.import_reference_path(&path) {
                Ok(value) => references.push(value),
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }
        MovieReferenceImport {
            references,
            failures,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn export_production_bundle(
    request: kestrel_app_core::ProductionBundleRequest,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::ProductionBundle, String> {
    let _guard = claim_workspace(&state)?;
    let id = request.job_id.clone();
    let cancel = register_production_job(&state, &id)?;
    let result = production_bundle::export(&state, request, cancel).await;
    state
        .interactive_jobs
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&id);
    result
}

#[tauri::command]
async fn export_media_composition(
    request: kestrel_app_core::MediaCompositionRequest,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::MediaCompositionExport, String> {
    let _guard = claim_workspace(&state)?;
    let id = request.job_id.clone();
    let cancel = register_production_job(&state, &id)?;
    let result = media_composition::export(&state, request, cancel).await;
    state
        .interactive_jobs
        .lock()
        .map_err(|e| e.to_string())?
        .remove(&id);
    result
}

fn register_production_job(state: &AppState, id: &str) -> Result<CancellationToken, String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return Err("Production jobId needs 1–128 letters, digits, hyphens or underscores.".into());
    }
    let cancel = CancellationToken::new();
    let mut jobs = state.interactive_jobs.lock().map_err(|e| e.to_string())?;
    if jobs.contains_key(id) {
        return Err("That production job is already running.".into());
    }
    jobs.insert(id.to_string(), cancel.clone());
    Ok(cancel)
}

#[tauri::command]
fn cancel_production_job(id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state
        .interactive_jobs
        .lock()
        .map_err(|e| e.to_string())?
        .get(&id)
    {
        cancel.cancel();
    }
    Ok(())
}

#[tauri::command]
fn list_media_compositions(
    state: State<'_, AppState>,
) -> Result<Vec<kestrel_app_core::MediaCompositionSummary>, String> {
    media_composition::list(state.store.root())
}

#[tauri::command]
fn list_production_bundles(
    state: State<'_, AppState>,
) -> Result<Vec<kestrel_app_core::ProductionBundleSummary>, String> {
    production_bundle::list(state.store.root())
}

#[tauri::command]
fn get_production_bundle(
    id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    production_bundle::get(state.store.root(), &id)
}

#[tauri::command]
fn get_media_composition(
    id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    media_composition::get(state.store.root(), &id)
}

#[tauri::command]
fn apply_music_text(
    id: String,
    field: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<MusicProject, String> {
    let _guard = claim_workspace(&state)?;
    let project = state.music.get(&id).map_err(|e| e.to_string())?;
    state
        .music
        .save_editable(producer_text::music(project, &field, &text)?)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn apply_image_composition(
    id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<ImageProject, String> {
    let _guard = claim_workspace(&state)?;
    let project = state.images.get(&id).map_err(|e| e.to_string())?;
    state
        .images
        .save_editable(producer_text::image(project, &text)?)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn bootstrap(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    snapshot(&state).await
}

#[tauri::command]
fn get_speech_preferences(
    legacy: Option<kestrel_app_core::LegacySpeechPreferences>,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::SpeechPreferences, String> {
    state.speech_preferences.get(legacy)
}

#[tauri::command]
fn save_vad_settings(
    settings: kestrel_app_core::VadSettings,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::SpeechPreferences, String> {
    state.speech_preferences.save_vad(settings)
}

#[tauri::command]
fn save_narration_speech_preferences(
    settings: kestrel_app_core::NarrationPreferences,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::SpeechPreferences, String> {
    state.speech_preferences.save_narration(settings)
}

#[tauri::command]
fn save_research_speech_preferences(
    settings: kestrel_app_core::ResearchSpeechPreferences,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::SpeechPreferences, String> {
    state.speech_preferences.save_research(settings)
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
    merge_voice_library(
        state.speech.snapshot(&settings.comfy_root).await,
        &state.voice_library,
    )
}

fn merge_voice_library(
    mut snapshot: SpeechSnapshot,
    library: &VoiceLibrary,
) -> Result<SpeechSnapshot, String> {
    let voices = library.snapshot().map_err(|error| error.to_string())?;
    snapshot.voice_profiles = voices.profiles;
    snapshot.default_voice_profile_id = voices.default_profile_id;
    Ok(snapshot)
}

#[tauri::command]
fn get_voice_library(state: State<'_, AppState>) -> Result<VoiceLibrarySnapshot, String> {
    state
        .voice_library
        .snapshot()
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn create_voice_profile(
    request: CreateVoiceProfileRequest,
    state: State<'_, AppState>,
) -> Result<VoiceLibrarySnapshot, String> {
    let library = state.voice_library.clone();
    tokio::task::spawn_blocking(move || library.create(request))
        .await
        .map_err(|error| format!("voice import worker stopped: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn update_voice_profile(
    request: UpdateVoiceProfileRequest,
    state: State<'_, AppState>,
) -> Result<VoiceLibrarySnapshot, String> {
    state
        .voice_library
        .update(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn set_default_voice_profile(
    profile_id: String,
    state: State<'_, AppState>,
) -> Result<VoiceLibrarySnapshot, String> {
    state
        .voice_library
        .set_default(&profile_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_voice_profile(
    profile_id: String,
    state: State<'_, AppState>,
) -> Result<VoiceLibrarySnapshot, String> {
    state
        .voice_library
        .delete(&profile_id)
        .map_err(|error| error.to_string())
}

async fn remember_runtime_for_speech(state: &AppState) {
    let snapshot = state.runtime.snapshot().await;
    let Some(model_id) = snapshot.model_id.filter(|_| snapshot.phase == "ready") else {
        return;
    };
    if let Ok(mut restore) = state.speech_restore_model.lock() {
        if restore.is_none() {
            *restore = Some(SpeechRuntimeRestore { model_id });
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
        state
            .runtime
            .start_model(&model, &settings, Some(app))
            .await
            .map_err(|error| error.to_string())?;
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
        return merge_voice_library(initial, &state.voice_library);
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
        state
            .speech
            .ensure_comfy(&settings.comfy_root, &cancel)
            .await
            .map_err(|error| error.to_string())?;
        merge_voice_library(
            state.speech.snapshot(&settings.comfy_root).await,
            &state.voice_library,
        )
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
    let voice = state
        .voice_library
        .resolve(&request.voice_profile_id)
        .map_err(|error| error.to_string())?;
    let check_model = narration_check_model(&state, &request, &settings.comfy_root);
    // A cached passage plays at once, unless the mistake check is on and has not heard it yet.
    let ready = || -> Result<Option<SpeechClip>, String> {
        let clip = state
            .speech
            .cached_clip(&settings.comfy_root, &request, &voice)
            .map_err(|error| error.to_string())?;
        Ok(clip.filter(|_| {
            check_model.is_none() || !state.speech.needs_narration_check(&request, &voice)
        }))
    };
    if let Some(clip) = ready()? {
        return Ok(clip);
    }
    let cancel = register_speech_job(&state, &request.job_id)?;
    let result: Result<SpeechClip, String> = async {
        let _turn = wait_for_speech_turn(&state, &cancel).await?;
        if let Some(clip) = ready()? {
            return Ok(clip);
        }
        let _guard = claim_workspace(&state)?;
        remember_runtime_for_speech(&state).await;
        state
            .runtime
            .stop_managed()
            .await
            .map_err(|error| error.to_string())?;
        state
            .speech
            .ensure_comfy(&settings.comfy_root, &cancel)
            .await
            .map_err(|error| error.to_string())?;
        match &check_model {
            Some(alignment_model) => state
                .speech
                .synthesize_checked(
                    &settings.comfy_root,
                    &request,
                    alignment_model,
                    &voice,
                    &cancel,
                    Some(&app),
                )
                .await
                .map_err(|error| error.to_string()),
            None => state
                .speech
                .synthesize(&settings.comfy_root, &request, &voice, &cancel, Some(&app))
                .await
                .map_err(|error| error.to_string()),
        }
    }
    .await;
    finish_speech_job(&state, &request.job_id);
    result
}

/// The Whisper model that checks this passage for voice mistakes, when the producer turned the
/// check on or an export chose it. Without an installed Whisper model there is no check.
fn narration_check_model(
    state: &AppState,
    request: &SpeechSynthesisRequest,
    comfy_root: &str,
) -> Option<String> {
    // A request's own choice (an export's checkbox) wins over the saved preference.
    request
        .check_mistakes
        .unwrap_or_else(|| state.speech_preferences.check_narration())
        .then(|| local_speech::discover_whisper_models(std::path::Path::new(comfy_root)))
        .and_then(|models| models.into_iter().next())
        .map(|model| model.id)
}

/// Longest narration an export may join: a long book at a few hundred words a passage.
const MAX_EXPORT_PASSAGES: usize = 4_000;

/// Ask where to save an exported M4A, suggesting a name from its title. Nothing when the
/// producer closes the dialog.
async fn choose_export_file(title: &str) -> Option<std::path::PathBuf> {
    let file = rfd::AsyncFileDialog::new()
        .set_title("Save audio")
        .add_filter("Audio (M4A)", &["m4a"])
        .set_file_name(narration::export_file_name(title))
        .save_file()
        .await?;
    let mut destination = file.path().to_path_buf();
    if !destination
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("m4a"))
    {
        destination.set_extension("m4a");
    }
    Some(destination)
}

fn exported_files(files: Vec<std::path::PathBuf>) -> kestrel_app_core::ExportedFiles {
    kestrel_app_core::ExportedFiles {
        files: files
            .into_iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    }
}

/// Join a reply's generated passages into one audio file saved where the producer chooses, with
/// the words timed beside it when asked. Nothing when the producer closes the save dialog.
#[tauri::command]
async fn export_narration(
    request: kestrel_app_core::NarrationExportRequest,
    destination_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<kestrel_app_core::ExportedFiles>, String> {
    if request.passages.is_empty() || request.passages.len() > MAX_EXPORT_PASSAGES {
        return Err(format!(
            "A narration export needs 1 to {MAX_EXPORT_PASSAGES} passages."
        ));
    }
    let clips = request
        .passages
        .iter()
        .map(|passage| {
            state
                .speech
                .export_clip(&passage.relative_path, &passage.text)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let destination = match destination_path {
        Some(path) => Some(app_control::export_path(&path, "m4a")?),
        None => choose_export_file(&request.title).await,
    };
    let Some(destination) = destination else {
        return Ok(None);
    };
    let cancel = register_speech_job(&state, &request.job_id)?;
    let result = async {
        let spans = narration::write_narration_audio(&clips, &destination, &cancel).await?;
        let mut files = vec![destination.clone()];
        if request.word_timings {
            let text = narration::narration_timed_text(&request.title, &clips, &spans);
            files.extend(timed_text::write_companions(&text, &destination)?);
        }
        Ok::<_, String>(files)
    }
    .await;
    finish_speech_job(&state, &request.job_id);
    result.map(|files| Some(exported_files(files)))
}

/// Save a take as M4A with its saved lyrics, timed word by word beside it when asked. Only a
/// lyric revision that still matches its take is exported. Nothing when the dialog is closed.
#[tauri::command]
async fn export_music_lyrics(
    request: kestrel_app_core::MusicLyricsExportRequest,
    destination_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<kestrel_app_core::ExportedFiles>, String> {
    let lyrics = MusicLyricsRequest {
        project_id: request.project_id.clone(),
        take_id: request.take_id.clone(),
    };
    let loaded = state
        .music
        .load_lyrics_document(lyrics.clone())
        .map_err(|error| error.to_string())?;
    let source = state
        .music
        .lyrics_audio_source(&lyrics)
        .map_err(|error| error.to_string())?;
    let destination = match destination_path {
        Some(path) => Some(app_control::export_path(&path, "m4a")?),
        None => choose_export_file(&loaded.project.title).await,
    };
    let Some(destination) = destination else {
        return Ok(None);
    };
    let cancel = CancellationToken::new();
    narration::write_song_audio(&source.path, &destination, &cancel).await?;
    let mut files = vec![destination.clone()];
    if request.word_timings {
        let text = timed_text::song_timed_text(&loaded.project.title, &loaded.document);
        files.extend(timed_text::write_companions(&text, &destination)?);
    }
    Ok(Some(exported_files(files)))
}

#[tauri::command]
fn get_cached_local_speech_clip(
    request: SpeechSynthesisRequest,
    state: State<'_, AppState>,
) -> Result<Option<SpeechClip>, String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let voice = state
        .voice_library
        .resolve(&request.voice_profile_id)
        .map_err(|error| error.to_string())?;
    let check = narration_check_model(&state, &request, &settings.comfy_root).is_some();
    let clip = state
        .speech
        .cached_clip(&settings.comfy_root, &request, &voice)
        .map_err(|error| error.to_string())?;
    // With the mistake check on, an unchecked passage is generated through the check first.
    Ok(clip.filter(|_| !check || !state.speech.needs_narration_check(&request, &voice)))
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
    let voice = state
        .voice_library
        .resolve(&request.voice_profile_id)
        .map_err(|error| error.to_string())?;
    if let Some(clip) = state
        .speech
        .cached_alignment(&settings.comfy_root, &request, &voice)
        .map_err(|error| error.to_string())?
    {
        return Ok(clip);
    }
    let cancel = register_speech_job(&state, &request.job_id)?;
    let result: Result<SpeechClip, String> = async {
        let _turn = wait_for_speech_turn(&state, &cancel).await?;
        if let Some(clip) = state
            .speech
            .cached_alignment(&settings.comfy_root, &request, &voice)
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
        state
            .speech
            .ensure_comfy(&settings.comfy_root, &cancel)
            .await
            .map_err(|error| error.to_string())?;
        state
            .speech
            .align(&settings.comfy_root, &request, &voice, &cancel, Some(&app))
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
    app_control::ensure_task_access(&state)?;
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
    let mut settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let models = state.models.read().await.clone();
    let model = default_runtime_model(&models, &control)?;
    if !settings.advanced_mode {
        let effective = control.for_model(&model.id);
        settings.context_window = effective.context_window;
        settings.max_output_tokens = effective.max_output_tokens;
    }
    settings.context_window = model.serving_context(settings.context_window);
    let lease = state
        .runtime
        .lease_research(&model.id, &models, &control, &settings, Some(&app))
        .await
        .map_err(|error| error.to_string())?;
    let job_id = uuid::Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    let started = std::time::Instant::now();
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
            cancel.clone(),
        )
        .await
        .map_err(|error| error.to_string());
    if let Err(error) = &result {
        let cancelled = cancel.is_cancelled();
        let _ = ipc_events::emit::<kestrel_app_core::events::ResearchProgress>(
            &app,
            &models::ResearchProgress {
                job_id: job_id.clone(),
                stage: if cancelled { "cancelled" } else { "failed" }.into(),
                title: if cancelled {
                    "Research stopped"
                } else {
                    "Research could not finish"
                }
                .into(),
                detail: error.clone(),
                current: 0,
                total: 6,
                elapsed_seconds: started.elapsed().as_secs(),
            },
        );
    }
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
async fn create_movie_producer_project(
    request: CreateMovieProducerProjectRequest,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    let models = state.models.read().await.clone();
    let model = models
        .iter()
        .find(|model| model.id == request.collaborator_model_id)
        .ok_or_else(|| {
            "The selected Studio collaborator is no longer in the local model catalog.".to_string()
        })?;
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let _guard = claim_workspace(&state)?;
    state
        .studio
        .create_producer_project(
            request,
            &model.name,
            research.advanced_mode || control.advanced_mode,
        )
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn attach_movie_producer_references(
    request: AttachMovieProducerReferencesRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProject, String> {
    let _guard = claim_workspace(&state)?;
    state
        .studio
        .attach_producer_references(request, Some(&app))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_movie_producer_workspace(
    id: String,
    state: State<'_, AppState>,
) -> Result<MovieProducerWorkspace, String> {
    state
        .studio
        .get_producer_workspace(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_movie_studio_conversation(
    project_id: String,
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<MovieStudioConversation, String> {
    state
        .studio
        .get_producer_conversation(&project_id, &conversation_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn save_movie_story_revision(
    request: SaveMovieStoryRevisionRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProducerWorkspace, String> {
    let _guard = claim_workspace(&state)?;
    state
        .studio
        .save_story_revision(request, Some(&app))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn accept_movie_story_revision(
    request: AcceptMovieStoryRevisionRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProducerWorkspace, String> {
    let _guard = claim_workspace(&state)?;
    state
        .studio
        .accept_story_revision(request, Some(&app))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn save_movie_scenes(
    request: SaveMovieScenesRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieProducerWorkspace, String> {
    let _guard = claim_workspace(&state)?;
    state
        .studio
        .save_producer_scenes(request, Some(&app))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn reset_movie_studio_conversation(
    request: ResetMovieStudioConversationRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieStudioConversation, String> {
    let _guard = claim_workspace(&state)?;
    state
        .studio
        .reset_producer_conversation(request, Some(&app))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn summarize_movie_studio_conversation(
    request: SummarizeMovieStudioConversationRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MovieStudioConversation, String> {
    ensure_workspace_idle(&state)?;
    let models = state.models.read().await.clone();
    if !models.iter().any(|model| model.id == request.model_id) {
        return Err(
            "The selected Studio collaborator is no longer in the local model catalog.".into(),
        );
    }
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another local AI job is already active.".to_string())?;
    let _guard = WorkGuard(&state.work_active);
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    summarize_studio_conversation(
        &app,
        &state.studio,
        &state.runtime,
        &models,
        &settings,
        &request,
    )
    .await
}

#[tauri::command]
async fn start_movie_studio_chat(
    request: MovieStudioChatRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    ensure_workspace_idle(&state)?;
    let models = state.models.read().await.clone();
    if !models.iter().any(|model| model.id == request.model_id) {
        return Err(
            "The selected Studio collaborator is no longer in the local model catalog.".into(),
        );
    }
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
    let event_request = request.clone();
    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let managed = app_for_task.state::<AppState>();
        let work_guard = WorkGuard(&managed.work_active);
        let result = match managed.control_settings.load() {
            Ok(settings) => {
                MovieStudioChatJob {
                    app: Some(app_for_task.clone()),
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
            studio::emit_studio_chat_error(&app_for_task, &event_request, error);
        }
        if let Ok(mut jobs) = managed.interactive_jobs.lock() {
            jobs.remove(&event_request.request_id);
        }
        drop(work_guard);
        studio::emit_studio_chat_settled(&app_for_task, &event_request);
    });
    Ok(request_id)
}

#[tauri::command]
fn cancel_movie_studio_chat(request_id: String, state: State<'_, AppState>) -> Result<(), String> {
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
async fn start_studio_prompt_draft(
    request: PromptDraftRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    ensure_workspace_idle(&state)?;
    let models = state.models.read().await.clone();
    studio::validate_prompt_draft_request(&request, &models)?;
    state.control.require_new_job(&request.request_id)?;
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
                    events: app_for_task.clone().into(),
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
fn cancel_studio_prompt_draft(
    request_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
fn get_movie_render_state(
    id: String,
    state: State<'_, AppState>,
) -> Result<MovieRenderState, String> {
    let project = state.studio.get(&id).map_err(|error| error.to_string())?;
    let registered = state
        .movie_jobs
        .lock()
        .map_err(|_| "movie job registry is unavailable".to_string())?
        .contains_key(&id);
    let render_phase = matches!(
        project.phase.as_str(),
        "starting-renderer"
            | "rendering"
            | "rendering-scene-version"
            | "rendering-transition"
            | "assembling"
    );
    state
        .studio
        .movie_render_state(
            &id,
            registered
                && (render_phase
                    || state.studio.editor_state(&id).is_ok_and(|state| {
                        state.jobs.iter().any(|job| {
                            matches!(
                                job.status,
                                models::MovieEditorJobStatus::Writing
                                    | models::MovieEditorJobStatus::Rendering
                            )
                        })
                    })),
        )
        .map_err(|error| error.to_string())
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

/// Brings a finished Image Studio take into the movie reference library so a production can use it
/// as a frame or reference. The take is found by its project and take IDs; no path crosses IPC, and
/// the file must still live inside that image project's folder.
#[tauri::command]
async fn import_image_take_as_movie_reference(
    image_project_id: String,
    take_id: String,
    state: State<'_, AppState>,
) -> Result<MovieReferenceAsset, String> {
    let project = state
        .images
        .get(&image_project_id)
        .map_err(|error| error.to_string())?;
    let (index, take) = project
        .takes
        .iter()
        .enumerate()
        .find(|(_, take)| take.id == take_id)
        .ok_or_else(|| "That image take no longer exists in its project.".to_string())?;
    if take.status != "complete" {
        return Err("Only a finished image take can be used in a movie.".into());
    }
    let folder = state
        .images
        .project_dir(&image_project_id)
        .canonicalize()
        .map_err(|error| format!("Cannot open the image project folder: {error}"))?;
    let path = PathBuf::from(&take.path)
        .canonicalize()
        .map_err(|error| format!("Cannot open the image take {}: {error}", take.path))?;
    if !path.starts_with(&folder) {
        return Err(
            "The image take is outside its project folder, so Kestrel will not import it.".into(),
        );
    }
    let name = format!("{} · take {}", project.title.trim(), index + 1);
    let studio = state.studio.clone();
    tokio::task::spawn_blocking(move || studio.import_reference_path_named(&path, Some(&name)))
        .await
        .map_err(|error| format!("Reference import stopped unexpectedly: {error}"))?
        .map_err(|error| error.to_string())
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
async fn render_movie_scenes(
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
        .approve_producer_scenes(&id, Some(&app))
        .await
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
        .insert(id.clone(), cancel.clone());
    spawn_movie_render(app, id, cancel);
    Ok(project)
}

#[tauri::command]
fn get_movie_editor_state(
    id: String,
    state: State<'_, AppState>,
) -> Result<models::MovieEditorState, String> {
    state
        .studio
        .editor_state(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn prepare_movie_editor_range(
    request: models::MovieEditorRangeRequest,
    edit: MovieEdit,
    state: State<'_, AppState>,
) -> Result<models::MovieEditorJob, String> {
    let _guard = claim_workspace(&state)?;
    state
        .studio
        .prepare_editor_range(request, edit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_movie_editor_generation(
    request: models::MovieEditorGenerateRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    ensure_workspace_idle(&state)?;
    let models = state.models.read().await.clone();
    state
        .studio
        .validate_editor_generation(&request, &models)
        .map_err(|error| error.to_string())?;
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another local AI or Studio job is already active.".to_string())?;
    let cancel = CancellationToken::new();
    let project_id = request.project_id.clone();
    let job_id = request.job_id.clone();
    state
        .movie_jobs
        .lock()
        .map_err(|_| {
            state.work_active.store(false, Ordering::Release);
            "Movie job registry is unavailable".to_string()
        })?
        .insert(project_id.clone(), cancel.clone());
    tauri::async_runtime::spawn(async move {
        let managed = app.state::<AppState>();
        let _guard = WorkGuard(&managed.work_active);
        release_all_comfy_memory(&managed).await;
        let _ = managed
            .studio
            .generate_editor_take(
                request,
                &managed.runtime,
                &models,
                &settings,
                &cancel,
                Some(&app),
            )
            .await;
        if let Ok(mut jobs) = managed.movie_jobs.lock() {
            jobs.remove(&project_id);
        };
    });
    Ok(job_id)
}

fn spawn_movie_render(app: AppHandle, id: String, cancel: CancellationToken) {
    tauri::async_runtime::spawn(async move {
        let managed = app.state::<AppState>();
        let _guard = WorkGuard(&managed.work_active);
        release_all_comfy_memory(&managed).await;
        let _ = managed.runtime.stop_managed().await;
        let result = match managed.research_settings.load() {
            Ok(settings) => managed
                .studio
                .render(&id, &settings.comfy_root, &cancel, Some(&app))
                .await
                .map_err(|error| error.to_string()),
            Err(error) => Err(error.to_string()),
        };
        release_all_comfy_memory(&managed).await;
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
fn cancel_movie_render(id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state
        .movie_jobs
        .lock()
        .map_err(|_| "movie job registry is unavailable".to_string())?
        .get(&id)
    {
        cancel.cancel();
    }
    Ok(())
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
fn list_music_projects(state: State<'_, AppState>) -> Result<Vec<MusicSummary>, String> {
    state.music.list().map_err(|error| error.to_string())
}

#[tauri::command]
fn get_music_project(id: String, state: State<'_, AppState>) -> Result<MusicProject, String> {
    state.music.get(&id).map_err(|error| error.to_string())
}

#[tauri::command]
fn create_music_project(
    mut request: CreateMusicProjectRequest,
    state: State<'_, AppState>,
) -> Result<MusicProject, String> {
    let _guard = claim_workspace(&state)?;
    if request.comfy_root.trim().is_empty() {
        request.comfy_root = state
            .research_settings
            .load()
            .map_err(|e| e.to_string())?
            .comfy_root;
    }
    if request.muscriptor_executable_path.trim().is_empty()
        && request.muscriptor_model_path.trim().is_empty()
    {
        let settings = state
            .research_settings
            .load()
            .map_err(|error| error.to_string())?;
        let (executable, model, _) = setup::configured_muscriptor_paths(&settings);
        if setup::muscriptor_is_ready(&settings) {
            request.muscriptor_executable_path = executable.to_string_lossy().into_owned();
            request.muscriptor_model_path = model.to_string_lossy().into_owned();
        }
    }
    state
        .music
        .create(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_music_project(
    project: kestrel_app_core::MusicProjectEdit,
    state: State<'_, AppState>,
) -> Result<MusicProject, String> {
    let _guard = claim_workspace(&state)?;
    state
        .music
        .save_editable(project)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn create_music_lyrics_draft(
    request: MusicLyricsRequest,
    state: State<'_, AppState>,
) -> Result<MusicLyricsSaveResult, String> {
    let _guard = claim_workspace(&state)?;
    state
        .music
        .create_lyrics_draft(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_music_lyrics_document(
    request: MusicLyricsRequest,
    state: State<'_, AppState>,
) -> Result<MusicLyricsSaveResult, String> {
    state
        .music
        .load_lyrics_document(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_music_lyrics_document(
    request: SaveMusicLyricsDocumentRequest,
    state: State<'_, AppState>,
) -> Result<MusicLyricsSaveResult, String> {
    let _guard = claim_workspace(&state)?;
    state
        .music
        .save_lyrics_document(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn transcribe_music_lyrics(
    request: TranscribeMusicLyricsRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MusicLyricsSaveResult, String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let source_request = MusicLyricsRequest {
        project_id: request.project_id.clone(),
        take_id: request.take_id.clone(),
    };
    let music = state.music.clone();
    let source = tokio::task::spawn_blocking(move || music.lyrics_audio_source(&source_request))
        .await
        .map_err(|error| format!("lyric audio validation stopped unexpectedly: {error}"))?
        .map_err(|error| error.to_string())?;
    let cancel = register_speech_job(&state, &request.job_id)?;
    let result: Result<MusicLyricsSaveResult, String> = async {
        let _turn = wait_for_speech_turn(&state, &cancel).await?;
        let _guard = claim_workspace(&state)?;
        release_all_comfy_memory(&state).await;
        remember_runtime_for_speech(&state).await;
        state
            .runtime
            .stop_managed()
            .await
            .map_err(|error| error.to_string())?;
        state
            .speech
            .ensure_comfy(&settings.comfy_root, &cancel)
            .await
            .map_err(|error| error.to_string())?;
        let transcription = state
            .speech
            .transcribe_file(
                &settings.comfy_root,
                &SpeechFileTranscriptionRequest {
                    job_id: request.job_id.clone(),
                    recording_id: request.take_id.clone(),
                    audio_path: source.path,
                    model_id: request.model_id.clone(),
                    language: request.language.clone(),
                    prompt: source.lyrics,
                },
                &cancel,
                Some(&app),
            )
            .await
            .map_err(|error| error.to_string())?;
        state
            .music
            .persist_lyrics_transcription(&request, transcription)
            .map_err(|error| error.to_string())
    }
    .await;
    finish_speech_job(&state, &request.job_id);
    result
}

#[tauri::command]
async fn repair_music_lyrics_range(
    request: RepairMusicLyricsRangeRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MusicLyricsSaveResult, String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let source_request = MusicLyricsRequest {
        project_id: request.project_id.clone(),
        take_id: request.take_id.clone(),
    };
    let music = state.music.clone();
    let start_seconds = request.start_seconds;
    let end_seconds = request.end_seconds;
    let source = tokio::task::spawn_blocking(move || {
        music.validate_lyrics_range(&source_request, start_seconds, end_seconds, None)
    })
    .await
    .map_err(|error| format!("lyric audio validation stopped unexpectedly: {error}"))?
    .map_err(|error| error.to_string())?;
    let prompt = if request.prompt.trim().is_empty() {
        crate::local_speech::music_opening_prompt(&source.lyrics)
    } else {
        request.prompt.clone()
    };
    let mut effective_request = request.clone();
    effective_request.prompt.clone_from(&prompt);
    let cancel = register_speech_job(&state, &request.job_id)?;
    let result: Result<MusicLyricsSaveResult, String> = async {
        let _turn = wait_for_speech_turn(&state, &cancel).await?;
        let _guard = claim_workspace(&state)?;
        release_all_comfy_memory(&state).await;
        remember_runtime_for_speech(&state).await;
        state
            .runtime
            .stop_managed()
            .await
            .map_err(|error| error.to_string())?;
        state
            .speech
            .ensure_comfy(&settings.comfy_root, &cancel)
            .await
            .map_err(|error| error.to_string())?;
        let transcription = state
            .speech
            .transcribe_file_range(
                &settings.comfy_root,
                &SpeechFileRangeTranscriptionRequest {
                    job_id: effective_request.job_id.clone(),
                    recording_id: effective_request.take_id.clone(),
                    audio_path: source.path,
                    model_id: effective_request.model_id.clone(),
                    language: effective_request.language.clone(),
                    prompt,
                    start_seconds: effective_request.start_seconds,
                    end_seconds: effective_request.end_seconds,
                },
                &cancel,
                Some(&app),
            )
            .await
            .map_err(|error| error.to_string())?;
        state
            .music
            .persist_lyrics_range_repair(&effective_request, transcription)
            .map_err(|error| error.to_string())
    }
    .await;
    finish_speech_job(&state, &request.job_id);
    result
}

#[tauri::command]
async fn draft_lyrics_from_audio_range(
    request: studio::DraftLyricsFromAudioRangeRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<studio::DraftLyricsFromAudioRangeResult, String> {
    let _guard = claim_workspace(&state)?;
    let models = state.models.read().await.clone();
    let model = models
        .iter()
        .find(|m| m.id == request.model_id)
        .cloned()
        .ok_or_else(|| "The selected local model is no longer in the catalog.".to_string())?;
    if !model.supports_audio {
        return Err(format!(
            "{} cannot listen to audio. Choose a catalog model with native audio support.",
            model.name
        ));
    }

    let source_request = studio::MusicLyricsRequest {
        project_id: request.project_id.clone(),
        take_id: request.take_id.clone(),
    };
    let music = state.music.clone();
    let start_seconds = request.start_seconds;
    let end_seconds = request.end_seconds;
    let source = tokio::task::spawn_blocking(move || {
        music.validate_lyrics_range(&source_request, start_seconds, end_seconds, Some(30.0))
    })
    .await
    .map_err(|error| format!("lyric audio validation stopped unexpectedly: {error}"))?
    .map_err(|error| error.to_string())?;
    let buffer_seconds = 1.5_f64;
    let slice_start = (request.start_seconds - buffer_seconds).max(0.0);
    let slice_end = (request.end_seconds + buffer_seconds).min(source.duration_seconds);
    let temp_slice_path = std::env::temp_dir().join(format!(
        "kestrel-audio-copilot-{}.wav",
        uuid::Uuid::new_v4().simple()
    ));
    let slice = crate::local_speech::prepare_audio_slice(
        &source.path,
        temp_slice_path,
        slice_start,
        slice_end,
    )
    .await
    .map_err(|error| error.to_string())?;
    let metadata = tokio::fs::metadata(slice.path())
        .await
        .map_err(|error| format!("Could not inspect the prepared audio excerpt: {error}"))?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Err("The prepared audio excerpt exceeds the 16 MiB local-model limit. Choose a shorter range.".into());
    }
    let wav_bytes = tokio::fs::read(slice.path())
        .await
        .map_err(|error| format!("Could not read the prepared audio excerpt: {error}"))?;
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?
        .for_model(&request.model_id);
    release_all_comfy_memory(&state).await;
    let lease = state
        .runtime
        .lease_model(&request.model_id, &models, &settings, Some(&app))
        .await
        .map_err(|error| error.to_string())?;
    studio::draft_music_lyrics_from_audio(&request, &model, &lease.connection, &wav_bytes).await
}

#[tauri::command]
async fn translate_music_lyrics(
    request: studio::TranslateMusicLyricsRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<studio::TranslateMusicLyricsResult, String> {
    let _guard = claim_workspace(&state)?;
    studio::validate_music_lyrics_translation(&request)?;
    let models = state.models.read().await.clone();
    let model = models
        .iter()
        .find(|m| m.id == request.model_id)
        .cloned()
        .ok_or_else(|| "The selected local model is no longer in the catalog.".to_string())?;

    let source_request = studio::MusicLyricsRequest {
        project_id: request.project_id.clone(),
        take_id: request.take_id.clone(),
    };
    let music = state.music.clone();
    tokio::task::spawn_blocking(move || music.lyrics_audio_source(&source_request))
        .await
        .map_err(|error| format!("lyric take validation stopped unexpectedly: {error}"))?
        .map_err(|error| error.to_string())?;
    if request.lines.is_empty() {
        return Ok(studio::TranslateMusicLyricsResult {
            translations: Vec::new(),
            model_id: model.id,
            model_name: model.name,
        });
    }
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?
        .for_model(&request.model_id);
    release_all_comfy_memory(&state).await;
    let lease = state
        .runtime
        .lease_model(&request.model_id, &models, &settings, Some(&app))
        .await
        .map_err(|error| error.to_string())?;
    studio::translate_music_lyrics_with_model(&request, &model, &lease.connection).await
}

#[tauri::command]
async fn start_music_generation(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<MusicProject, String> {
    ensure_workspace_idle(&state)?;
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another local AI or media job is already active.".to_string())?;
    let (project, take_id) = state
        .music
        .begin_generation(&id, &settings.comfy_root, Some(&app))
        .map_err(|error| {
            state.work_active.store(false, Ordering::Release);
            error.to_string()
        })?;
    let cancel = CancellationToken::new();
    match state.music_jobs.lock() {
        Ok(mut jobs) => {
            jobs.insert(id.clone(), cancel.clone());
        }
        Err(_) => {
            state.work_active.store(false, Ordering::Release);
            return Err("music job registry is unavailable".into());
        }
    }
    tauri::async_runtime::spawn(async move {
        let managed = app.state::<AppState>();
        let _guard = WorkGuard(&managed.work_active);
        let result: Result<(), String> = async {
            release_all_comfy_memory(&managed).await;
            managed
                .runtime
                .stop_managed()
                .await
                .map_err(|error| error.to_string())?;
            managed
                .music
                .render(&id, &take_id, &managed.studio, &cancel, Some(&app))
                .await
                .map_err(|error| error.to_string())?;
            Ok(())
        }
        .await;
        release_all_comfy_memory(&managed).await;
        if let Err(error) = result {
            managed
                .music
                .fail_generation(&id, &take_id, error, cancel.is_cancelled(), Some(&app));
        }
        if let Ok(mut jobs) = managed.music_jobs.lock() {
            jobs.remove(&id);
        };
    });
    Ok(project)
}

#[tauri::command]
fn cancel_music_generation(id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state
        .music_jobs
        .lock()
        .map_err(|_| "music job registry is unavailable".to_string())?
        .get(&id)
    {
        cancel.cancel();
    }
    Ok(())
}

#[tauri::command]
async fn transcribe_music_midi(
    request: MusicMidiRequest,
    state: State<'_, AppState>,
) -> Result<MusicProject, String> {
    let _guard = claim_workspace(&state)?;
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let (executable, model, _) = setup::configured_muscriptor_paths(&settings);
    release_all_comfy_memory(&state).await;
    state
        .runtime
        .stop_managed()
        .await
        .map_err(|error| error.to_string())?;
    state
        .music
        .transcribe_midi(request, (&executable, &model))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_music_midi_document(
    request: MusicMidiRequest,
    state: State<'_, AppState>,
) -> Result<MusicMidiSaveResult, String> {
    let _guard = claim_workspace(&state)?;
    state
        .music
        .load_midi_document(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_music_midi_document(
    request: SaveMusicMidiDocumentRequest,
    state: State<'_, AppState>,
) -> Result<MusicMidiSaveResult, String> {
    let _guard = claim_workspace(&state)?;
    state
        .music
        .save_midi_document(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn export_music_midi(
    request: MusicMidiRequest,
    destination_path: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let (_, title) = state
        .music
        .midi_artifact(&request)
        .map_err(|error| error.to_string())?;
    let filename = format!("{}.mid", safe_export_filename(&title));
    let path = if let Some(path) = destination_path {
        app_control::export_path(&path, "mid")?
    } else {
        let Some(destination) = rfd::AsyncFileDialog::new()
            .set_title("Export editable MIDI")
            .set_file_name(&filename)
            .add_filter("Standard MIDI file", &["mid", "midi"])
            .save_file()
            .await
        else {
            return Ok(None);
        };
        destination.path().to_path_buf()
    };
    state
        .music
        .export_midi_artifact(&request, &path)
        .map_err(|error| error.to_string())?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[tauri::command]
fn reveal_music_midi(request: MusicMidiRequest, state: State<'_, AppState>) -> Result<(), String> {
    let (path, _) = state
        .music
        .midi_artifact(&request)
        .map_err(|error| error.to_string())?;
    reveal_with_explorer(&path)
}

#[tauri::command]
fn reveal_music_project(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let path = state
        .music
        .reveal_path(&id)
        .map_err(|error| error.to_string())?;
    open_with_explorer(&path)
}

#[tauri::command]
fn list_image_projects(state: State<'_, AppState>) -> Result<Vec<ImageSummary>, String> {
    state.images.list().map_err(|error| error.to_string())
}

#[tauri::command]
fn get_image_project(id: String, state: State<'_, AppState>) -> Result<ImageProject, String> {
    state.images.get(&id).map_err(|error| error.to_string())
}

#[tauri::command]
fn create_image_project(
    mut request: CreateImageProjectRequest,
    state: State<'_, AppState>,
) -> Result<ImageProject, String> {
    let _guard = claim_workspace(&state)?;
    if request.comfy_root.trim().is_empty() {
        request.comfy_root = state
            .research_settings
            .load()
            .map_err(|e| e.to_string())?
            .comfy_root;
    }
    state
        .images
        .create(request)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn save_image_project(
    project: kestrel_app_core::ImageProjectEdit,
    state: State<'_, AppState>,
) -> Result<ImageProject, String> {
    let _guard = claim_workspace(&state)?;
    state
        .images
        .save_editable(project)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_image_generation(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ImageProject, String> {
    ensure_workspace_idle(&state)?;
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another local AI or media job is already active.".to_string())?;
    let (project, take_ids) = state
        .images
        .begin_generation(&id, &settings.comfy_root, Some(&app))
        .map_err(|error| {
            state.work_active.store(false, Ordering::Release);
            error.to_string()
        })?;
    let cancel = CancellationToken::new();
    match state.image_generation_jobs.lock() {
        Ok(mut jobs) => {
            jobs.insert(id.clone(), cancel.clone());
        }
        Err(_) => {
            state.work_active.store(false, Ordering::Release);
            return Err("image job registry is unavailable".into());
        }
    }
    tauri::async_runtime::spawn(async move {
        let managed = app.state::<AppState>();
        let _guard = WorkGuard(&managed.work_active);
        let result: Result<(), String> = async {
            release_all_comfy_memory(&managed).await;
            managed
                .runtime
                .stop_managed()
                .await
                .map_err(|error| error.to_string())?;
            managed
                .images
                .render(&id, &take_ids, &managed.studio, &cancel, Some(&app))
                .await
                .map_err(|error| error.to_string())?;
            Ok(())
        }
        .await;
        release_all_comfy_memory(&managed).await;
        if let Err(error) = result {
            managed.images.fail_generation(
                &id,
                &take_ids,
                error,
                cancel.is_cancelled(),
                Some(&app),
            );
        }
        if let Ok(mut jobs) = managed.image_generation_jobs.lock() {
            jobs.remove(&id);
        };
    });
    Ok(project)
}

#[tauri::command]
fn cancel_image_generation(id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state
        .image_generation_jobs
        .lock()
        .map_err(|_| "image job registry is unavailable".to_string())?
        .get(&id)
    {
        cancel.cancel();
    }
    Ok(())
}

#[tauri::command]
fn reveal_image_project(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let path = state
        .images
        .reveal_path(&id)
        .map_err(|error| error.to_string())?;
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
    let control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let models = state.models.read().await.clone();
    let model = default_runtime_model(&models, &control)?;
    state
        .runtime
        .start_model(model, &control, Some(&app))
        .await
        .map_err(|error| error.to_string())?;
    snapshot(&state).await
}

fn default_runtime_model<'a>(
    models: &'a [ModelInfo],
    settings: &ControlSettings,
) -> Result<&'a ModelInfo, String> {
    settings
        .selected_model_id
        .as_deref()
        .and_then(|id| models.iter().find(|model| model.id == id))
        .or_else(|| models.first())
        .ok_or_else(|| {
            "No local model is available. Install or locate a GGUF model in Setup, then rescan in Control."
                .into()
        })
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
    let strata_gpu = strata::detected_gpu().await;
    Ok(setup::snapshot(
        &research,
        &control,
        gpu.as_ref(),
        strata_gpu.as_ref(),
    ))
}

#[tauri::command]
async fn open_comfy_ui(workload: Option<String>, state: State<'_, AppState>) -> Result<(), String> {
    ensure_workspace_idle(&state)?;
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let music = workload.as_deref() == Some("music");
    let workload = ComfyWorkload::from_music(music);
    release_all_comfy_memory(&state).await;
    state
        .runtime
        .stop_managed()
        .await
        .map_err(|error| error.to_string())?;
    services::start_comfy(&settings.comfy_root, workload)
        .await
        .map_err(|error| error.to_string())?;
    std::process::Command::new("explorer.exe")
        .arg(workload.base_url())
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
    let _guard = claim_workspace(&state)?;
    let mut research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let mut control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let previous_strata_root = research.strata_root.clone();
    setup::apply_locations(&mut research, &mut control, locations)
        .map_err(|error| error.to_string())?;
    let comfy_root = std::path::Path::new(&research.comfy_root);
    if comfy_root.join("main.py").is_file()
        && (!comfy_root.join("Start-Kestrel-ComfyUI.ps1").is_file()
            || !comfy_root.join("Start-Kestrel-ComfyUI-Music.ps1").is_file())
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
    if research.strata_root != previous_strata_root {
        let sources = catalog_sources(&state, &control, &research);
        let found = tokio::task::spawn_blocking(move || model::scan_sources(&sources))
            .await
            .map_err(|error| format!("model scan failed after choosing Strata: {error}"))?;
        if let Err(error) = state.model_catalog.save(&found) {
            eprintln!("Kestrel found the Strata models, but its disposable catalog could not be saved: {error}");
        }
        *state.models.write().await = found;
    }
    snapshot(&state).await
}

#[tauri::command]
async fn pick_setup_folder() -> Result<String, String> {
    Ok(rfd::AsyncFileDialog::new()
        .set_title("Choose a local AI model or Kestrel component folder")
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
        "engine" | "ffmpeg" | "ffprobe" | "muscriptor" => {
            dialog.add_filter("Windows program", &["exe"])
        }
        "muscriptorModel" => dialog.add_filter("MuScriptor checkpoint", &["safetensors"]),
        "whisperModel" => dialog.add_filter("OpenAI Whisper checkpoint", &["pt"]),
        "modelAsset" => dialog.add_filter("Local AI model", &["safetensors", "gguf", "pt", "json"]),
        _ => dialog,
    };
    Ok(dialog
        .pick_file()
        .await
        .map(|file| file.path().to_string_lossy().into_owned())
        .unwrap_or_default())
}

#[tauri::command]
async fn scan_setup_model_folder(
    path: String,
) -> Result<std::collections::BTreeMap<String, String>, String> {
    let root = std::path::PathBuf::from(path);
    tokio::task::spawn_blocking(move || setup::scan_existing_model_folder(&root))
        .await
        .map_err(|error| format!("existing model search could not finish: {error}"))?
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn install_setup_component(
    request: setup::SetupInstallRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let _guard = claim_workspace(&state)?;
    let mut research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    if request.component == "strata" {
        // Strata's setup may replace strata.exe, which Windows keeps locked while it runs.
        state
            .runtime
            .stop_managed()
            .await
            .map_err(|error| error.to_string())?;
    }
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
    let result = setup::install_component(Some(&app), &mut research, &request, cancel).await;
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
    }
    if matches!(request.component.as_str(), "assistant" | "strata") {
        let sources = catalog_sources(&state, &control, &research);
        let found = tokio::task::spawn_blocking(move || model::scan_sources(&sources))
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
    system_console_snapshot(&state).await
}

async fn gpu_cleanup_protection(
    state: &AppState,
) -> Result<gpu_memory::GpuProcessProtection, String> {
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let mut protected_pids = vec![std::process::id()];
    if let Some(pid) = state.runtime.owned_process_id().await {
        protected_pids.push(pid);
    }
    let strata_installs = strata::known_installs(&[research.strata_root.as_str()]);
    let mut protected_roots = [
        research.install_root,
        research.bonsai_root,
        research.comfy_root,
    ]
    .into_iter()
    .filter(|path| !path.trim().is_empty())
    .map(PathBuf::from)
    .collect::<Vec<_>>();
    protected_roots.extend(strata_installs);
    let mut protected_paths = [
        control.engine_path,
        research.ffmpeg_path,
        research.ffprobe_path,
    ]
    .into_iter()
    .filter(|path| !path.trim().is_empty())
    .map(PathBuf::from)
    .collect::<Vec<_>>();
    if let Ok(path) = std::env::current_exe() {
        protected_paths.push(path);
    }
    Ok(gpu_memory::GpuProcessProtection::new(
        protected_pids,
        protected_roots,
        protected_paths,
    ))
}

#[tauri::command]
async fn preview_vram_cleanup(
    state: State<'_, AppState>,
) -> Result<gpu_memory::VramCleanupPreview, String> {
    let protection = gpu_cleanup_protection(&state).await?;
    gpu_memory::preview(&protection, services::gpu_snapshot().await)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn clean_vram(
    expected_pids: Vec<u32>,
    state: State<'_, AppState>,
) -> Result<gpu_memory::VramCleanupResult, String> {
    if expected_pids.len() > 256 {
        return Err("VRAM cleanup rejected more than 256 approved process IDs".into());
    }
    let _guard = claim_workspace(&state)?;
    let protection = gpu_cleanup_protection(&state).await?;
    let approved_pids = expected_pids.into_iter().collect();
    gpu_memory::clean(&protection, &approved_pids)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn force_clean_vram(
    expected_processes: Vec<gpu_memory::GpuMemoryProcess>,
    state: State<'_, AppState>,
) -> Result<gpu_memory::VramCleanupResult, String> {
    if expected_processes.len() > 256 {
        return Err("VRAM force cleanup rejected more than 256 expected processes".into());
    }
    let _guard = claim_workspace(&state)?;
    let protection = gpu_cleanup_protection(&state).await?;
    gpu_memory::force_clean(&protection, &expected_processes)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn get_control_snapshot(
    probe_developer: Option<bool>,
    state: State<'_, AppState>,
) -> Result<ControlSnapshot, String> {
    control_snapshot(&state, probe_developer.unwrap_or(true)).await
}

#[tauri::command]
async fn get_strata_dashboard(
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::StrataDashboardSnapshot, String> {
    Ok(strata_dashboard::snapshot(&state.runtime).await)
}

#[tauri::command]
async fn get_strata_request_detail(
    id: String,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::StrataDashboardRequestDetail, String> {
    strata_dashboard::detail(&state.runtime, &id).await
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
    let sources = catalog_sources(&state, &control, &research);
    let found = tokio::task::spawn_blocking(move || model::scan_sources(&sources))
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
async fn get_setup_profile_text(state: State<'_, AppState>) -> Result<String, String> {
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let models = state.models.read().await.clone();
    profile::preview(&research, &control, &models).map_err(|error| error.to_string())
}

#[tauri::command]
fn export_setup_profile_text(
    text: String,
    state: State<'_, AppState>,
) -> Result<ProfileTransfer, String> {
    profile::export_text(state.store.root(), &text).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_prompt_pack_text() -> Result<String, String> {
    prompt_catalog::current_text()
}

#[tauri::command]
fn get_default_prompt_pack_text() -> Result<String, String> {
    prompt_catalog::default_text()
}

#[tauri::command]
fn save_prompt_pack_text(text: String) -> Result<String, String> {
    prompt_catalog::save_text(&text)
}

#[tauri::command]
fn reset_prompt_pack() -> Result<String, String> {
    prompt_catalog::reset()
}

#[tauri::command]
fn export_prompt_pack_text(text: String) -> Result<ProfileTransfer, String> {
    let path = prompt_catalog::export_text(&text)?;
    Ok(ProfileTransfer {
        path: path.to_string_lossy().into_owned(),
        message: "Prompt-only pack validated and exported.".into(),
    })
}

#[tauri::command]
fn import_prompt_pack(path: String) -> Result<String, String> {
    prompt_catalog::import_path(std::path::Path::new(&path))
}

#[tauri::command]
async fn pick_prompt_pack_file() -> Result<Option<String>, String> {
    Ok(rfd::AsyncFileDialog::new()
        .set_title("Choose a Kestrel prompt-only JSON pack")
        .add_filter("Kestrel prompt pack", &["json"])
        .pick_file()
        .await
        .map(|file| file.path().to_string_lossy().into_owned()))
}

#[tauri::command]
async fn import_setup_profile(
    path: String,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let _guard = claim_workspace(&state)?;
    if !std::path::Path::new(&path).is_absolute() {
        return Err("setup profile path must be absolute".into());
    }
    let imported = profile::import(
        std::path::Path::new(&path),
        &state.research_settings,
        &state.control_settings,
    )
    .map_err(|error| error.to_string())?;
    finish_profile_import(&state, imported).await
}

#[tauri::command]
async fn import_setup_profile_text(
    text: String,
    state: State<'_, AppState>,
) -> Result<AppSnapshot, String> {
    let _guard = claim_workspace(&state)?;
    let imported = profile::import_text(&text, &state.research_settings, &state.control_settings)
        .map_err(|error| error.to_string())?;
    finish_profile_import(&state, imported).await
}

async fn finish_profile_import(
    state: &AppState,
    imported: profile::ImportedProfile,
) -> Result<AppSnapshot, String> {
    state
        .runtime
        .stop_managed()
        .await
        .map_err(|error| error.to_string())?;
    apply_media_paths(&imported.research);
    let sources = catalog_sources(state, &imported.control, &imported.research);
    match tokio::task::spawn_blocking(move || model::scan_sources(&sources)).await {
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
    refresh_engine_candidates(state, &imported.control, &imported.research).await;
    snapshot(state).await
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
    let control = state
        .control_settings
        .load()
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
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    refresh_engine_candidates(&state, &settings, &research).await;
    control_snapshot(&state, true).await
}

#[tauri::command]
async fn apply_model_runtime(
    settings: ControlSettings,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SystemSnapshot, String> {
    let _guard = claim_workspace(&state)?;
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
    let research = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    refresh_engine_candidates(&state, &settings, &research).await;
    state
        .runtime
        .stop_managed()
        .await
        .map_err(|error| error.to_string())?;
    let models = state.models.read().await.clone();
    let model = default_runtime_model(&models, &settings)?;
    state
        .runtime
        .start_model(model, &settings, Some(&app))
        .await
        .map_err(|error| error.to_string())?;
    system_console_snapshot(&state).await
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
        let music = state
            .music_jobs
            .lock()
            .map_err(|_| "music job registry is unavailable".to_string())?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let images = state
            .image_generation_jobs
            .lock()
            .map_err(|_| "image job registry is unavailable".to_string())?
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
            .chain(music)
            .chain(images)
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
    services::stop_legacy_bonsai_service(&research.bonsai_root)
        .await
        .map_err(|error| error.to_string())?;
    release_all_comfy_memory(&state).await;
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

/// Save the producer's edited copy of a model reply for listening and narration export. The
/// model keeps reading its original reply.
#[tauri::command]
fn save_chat_reply_edit(
    session_id: String,
    message_id: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<ChatSession, String> {
    state
        .workspace
        .set_reply_edit(&session_id, &message_id, Some(content))
}

#[tauri::command]
fn discard_chat_reply_edit(
    session_id: String,
    message_id: String,
    state: State<'_, AppState>,
) -> Result<ChatSession, String> {
    state
        .workspace
        .set_reply_edit(&session_id, &message_id, None)
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
    let session = match state.workspace.add_user_message_with_attachments(
        &session.id,
        message,
        attachments,
        request.recording.clone(),
    ) {
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

/// Delegate plain-language production through the selected local model's Computer Tasks runner.
#[tauri::command]
async fn start_producer_task(
    objective: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ComputerTaskRun, String> {
    let settings = state.control_settings.load().map_err(|e| e.to_string())?;
    let models = state.models.read().await.clone();
    let model_id = default_runtime_model(&models, &settings)?.id.clone();
    start_computer_task(
        ComputerTaskRequest {
            model_id,
            objective,
            access: ComputerTaskAccess::Workspace,
            max_steps: settings.agent_max_steps,
            max_output_tokens: settings.agent_max_output_tokens,
            attachment_ids: Vec::new(),
            thinking_level: None,
        },
        app,
        state,
    )
    .await
}

#[tauri::command]
fn get_computer_task(id: String, state: State<'_, AppState>) -> Result<ComputerTaskRun, String> {
    state.workspace.get_task(&id)
}

#[tauri::command]
fn get_task_memory(
    id: String,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::TaskMemorySnapshot, String> {
    state.workspace.task_memory(&id)
}

#[tauri::command]
fn get_task_history(
    id: String,
    query: kestrel_app_core::TaskHistoryQuery,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::TaskHistoryPage, String> {
    state.workspace.task_history(&id, query)
}

#[tauri::command]
fn read_task_record(
    id: String,
    sequence: u32,
    offset: u32,
    limit: u32,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::TaskRecordPage, String> {
    state
        .workspace
        .read_task_record(&id, sequence, offset, limit)
}

/// Human conversation input. A running task receives it at its next safe boundary; an idle task
/// resumes only because the human explicitly sent this message. The local agent cannot call this.
#[tauri::command]
async fn send_producer_message(
    request: ResumeComputerTaskRequest,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::ProducerTaskStatus, String> {
    if request.answer.trim().is_empty() || request.answer.len() > 65_536 {
        return Err("Write a message of up to 64 KiB.".into());
    }
    {
        let jobs = state
            .interactive_jobs
            .lock()
            .map_err(|_| "interactive job registry is unavailable")?;
        if jobs.contains_key(&request.run_id) {
            let event = models::ComputerTaskEvent {
                run_id: request.run_id,
                step: 0,
                kind: "user_input".into(),
                title: "You".into(),
                detail: request.answer,
                data: None,
                at: chrono::Utc::now().to_rfc3339(),
            };
            let run = state.workspace.add_task_event(event.clone())?;
            let _ = crate::ipc_events::emit::<kestrel_app_core::events::ComputerTask>(&app, &event);
            return Ok(agent::progress::snapshot(&run, None));
        }
    }
    let run = resume_computer_task(request, app, state).await?;
    Ok(agent::progress::snapshot(&run, None))
}

/// Explicit human review of a crash-uncertain action. Does not assert that the action succeeded,
/// grant access, or resume execution. The local task and saved workflows cannot call this.
#[tauri::command]
fn acknowledge_task_action(
    id: String,
    sequence: u32,
    review_note: String,
    state: State<'_, AppState>,
) -> Result<ComputerTaskRun, String> {
    if review_note.trim().is_empty() || review_note.len() > 4000 {
        return Err(
            "Describe what you checked in up to 4,000 bytes before recording the review.".into(),
        );
    }
    let jobs = state
        .interactive_jobs
        .lock()
        .map_err(|_| "interactive job registry is unavailable")?;
    if jobs.contains_key(&id) {
        return Err("Stop the conversation or wait for its question before reviewing the interrupted action.".into());
    }
    state.workspace.settle_task_action(&id,sequence,"humanReview",serde_json::json!({"note":review_note,"outcome":"reviewedByProducer","successNotAssumed":true}))?;
    state.workspace.get_task(&id)
}

#[tauri::command]
fn get_producer_task_status(
    id: String,
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::ProducerTaskStatus, String> {
    let task = state.workspace.get_task(&id)?;
    let progress = state
        .control
        .task_progress
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    Ok(agent::progress::snapshot(&task, progress))
}

#[tauri::command]
async fn get_producer_environment(
    state: State<'_, AppState>,
) -> Result<kestrel_app_core::ProducerEnvironment, String> {
    let research = state.research_settings.load().map_err(|e| e.to_string())?;
    let control = state.control_settings.load().map_err(|e| e.to_string())?;
    let models = state.models.read().await;
    let selected_model = default_runtime_model(&models, &control).ok().cloned();
    let video_defaults = kestrel_app_core::MovieSettings {
        comfy_root: research.comfy_root.clone(),
        ..Default::default()
    };
    let music_defaults = kestrel_app_core::MusicSettings {
        comfy_root: research.comfy_root.clone(),
        ..Default::default()
    };
    let image_defaults = kestrel_app_core::ImageSettings {
        comfy_root: research.comfy_root.clone(),
        ..Default::default()
    };
    let paths = [
        ("FFmpeg", research.ffmpeg_path),
        ("FFprobe", research.ffprobe_path),
        ("Strata", research.strata_root),
        ("ComfyUI (image, music, video, speech)", research.comfy_root),
        ("Kiwix server", research.kiwix_server_path),
        ("Offline Wikipedia archive", research.wikipedia_zim_path),
        ("MuScriptor executable", research.muscriptor_executable_path),
        ("MuScriptor model", research.muscriptor_model_path),
    ];
    Ok(kestrel_app_core::ProducerEnvironment {
        selected_model,
        generation_defaults: std::collections::BTreeMap::from([
            (
                "video".into(),
                serde_json::to_value(video_defaults).map_err(|e| e.to_string())?,
            ),
            (
                "music".into(),
                serde_json::to_value(music_defaults).map_err(|e| e.to_string())?,
            ),
            (
                "image".into(),
                serde_json::to_value(image_defaults).map_err(|e| e.to_string())?,
            ),
        ]),
        library_directory: state.store.root().to_string_lossy().into_owned(),
        model_download_directory: state
            .model_downloads
            .models_root()
            .to_string_lossy()
            .into_owned(),
        workspace_directories: control.agent_workspace_roots,
        tools: paths
            .into_iter()
            .map(|(name, path)| kestrel_app_core::ProducerToolPath {
                name: name.into(),
                path_exists: std::path::Path::new(&path).exists(),
                path,
            })
            .collect(),
    })
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
        "waiting" | "cancelled" | "interrupted" | "failed" | "completed" | "paused"
    ) {
        return Err(
            "This task is still active. Send a message to steer it, or stop it before continuing."
                .into(),
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
    match state.workspace.add_task_event(input_event.clone()) {
        Ok(_) => {}
        Err(error) => {
            state.work_active.store(false, Ordering::Release);
            return Err(error);
        }
    }
    let _ = crate::ipc_events::emit::<kestrel_app_core::events::ComputerTask>(&app, &input_event);
    // Persist acceptance before spawning. A crash here must leave an interrupted continuation,
    // not a task still labelled completed/waiting after acknowledging the new human message.
    let queued = models::ComputerTaskEvent {
        run_id: run.id.clone(),
        step: 0,
        kind: "queued".into(),
        title: "Message saved".into(),
        detail: "Continuing from your saved message.".into(),
        data: None,
        at: chrono::Utc::now().to_rfc3339(),
    };
    let updated = match state.workspace.add_task_event(queued.clone()) {
        Ok(updated) => updated,
        Err(error) => {
            state.work_active.store(false, Ordering::Release);
            return Err(format!(
                "Your message was saved, but continuation could not be queued: {error}"
            ));
        }
    };
    let _ = crate::ipc_events::emit::<kestrel_app_core::events::ComputerTask>(&app, &queued);
    let task_request = ComputerTaskRequest {
        model_id: run.model_id.clone(),
        objective: run.objective.clone(),
        access: run.access,
        max_steps: settings.agent_max_steps,
        max_output_tokens: settings.agent_max_output_tokens,
        attachment_ids: run.attachments.iter().map(|item| item.id.clone()).collect(),
        thinking_level: None,
    };
    let continuation = "Continue this conversation from its durable task context. Re-inspect current files/projects/jobs before further changes. The latest human message is saved as user_input; completed work and earlier direction remain retrievable through task_history and task_read.".to_string();
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
    app.state::<AppState>()
        .control
        .task_active
        .store(true, Ordering::Release);
    tauri::async_runtime::spawn(async move {
        let managed = app.state::<AppState>();
        let _work_guard = WorkGuard(&managed.work_active);
        let _task_guard = WorkGuard(&managed.control.task_active);
        let models = managed.models.read().await.clone();
        let mut continuation = continuation;
        loop {
            let result = agent::run(
                Some(app.clone()),
                managed.runtime.clone(),
                managed.workspace.clone(),
                managed.attachments.clone(),
                run_id.clone(),
                request.clone(),
                models.clone(),
                settings.clone(),
                cancel.clone(),
                continuation.clone(),
            )
            .await;
            if let Err(error) = result {
                agent::emit_error(Some(&app), &managed.workspace, &run_id, error);
            }
            // A human can send while the final answer is being published. Serialize the shutdown
            // check with message delivery so that accepted direction is never stranded silently.
            if let Ok(mut jobs) = managed.interactive_jobs.lock() {
                let pending = managed.workspace.get_task(&run_id).is_ok_and(|run| {
                    let direction = run.events.iter().rposition(|e| e.kind == "user_input");
                    let terminal = run.events.iter().rposition(|e| {
                        matches!(
                            e.kind.as_str(),
                            "done" | "question" | "error" | "cancelled" | "limit"
                        )
                    });
                    direction.is_some_and(|direction| {
                        terminal.is_some_and(|terminal| direction > terminal)
                    })
                });
                if pending && !cancel.is_cancelled() {
                    continuation = Some("A new human message arrived after the last response. Continue from durable task context and the latest user_input.".into());
                    continue;
                }
                jobs.remove(&run_id);
            }
            break;
        }
    });
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
    let control = control_snapshot(state, false).await?;
    let mut status = services::status(&settings).await;
    apply_model_status(
        &mut status,
        &control.runtime,
        &control.settings,
        &control.models,
    );
    let strata_gpu = strata::detected_gpu().await;
    let setup = setup::snapshot(
        &settings,
        &control.settings,
        control.gpu.as_ref(),
        strata_gpu.as_ref(),
    );
    Ok(AppSnapshot {
        status,
        reports,
        library_root: state.store.root().to_string_lossy().into_owned(),
        settings,
        control,
        setup,
    })
}

async fn system_console_snapshot(state: &AppState) -> Result<SystemSnapshot, String> {
    let settings = state
        .research_settings
        .load()
        .map_err(|error| error.to_string())?;
    let control = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let models = state.models.read().await.clone();
    let managed_runtime = state.runtime.snapshot().await;
    let mut value = services::system_snapshot(settings).await;
    apply_model_status(&mut value.status, &managed_runtime, &control, &models);
    let selected = managed_runtime
        .model_id
        .as_deref()
        .and_then(|id| models.iter().find(|model| model.id == id))
        .or_else(|| {
            control
                .selected_model_id
                .as_deref()
                .and_then(|id| models.iter().find(|model| model.id == id))
        })
        .or_else(|| models.first());
    let effective = selected
        .map(|model| control.for_model(&model.id))
        .unwrap_or_else(|| control.clone());
    value.runtime.context_window = if managed_runtime.context_window > 0 {
        managed_runtime.context_window
    } else {
        selected.map_or(effective.context_window, |model| {
            model
                .fixed_context_window
                .unwrap_or(effective.context_window)
        })
    };
    value.runtime.max_output_tokens = effective.max_output_tokens;
    value.runtime.model_root = selected
        .and_then(|model| std::path::Path::new(&model.path).parent())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    value.runtime.kv_cache = if managed_runtime.phase == "ready" {
        match selected.map(|model| model.engine) {
            Some(model::ModelEngine::Strata) => "managed by Strata".into(),
            _ => "managed by llama.cpp".into(),
        }
    } else {
        "not loaded".into()
    };
    value.control = control;
    value.models = models;
    value.managed_runtime = managed_runtime;
    Ok(value)
}

fn apply_model_status(
    status: &mut models::ServiceStatus,
    runtime: &models::ManagedRuntimeSnapshot,
    settings: &ControlSettings,
    models: &[ModelInfo],
) {
    status.model_runtime = if runtime.phase == "ready" {
        "ready"
    } else {
        "stopped"
    }
    .into();
    status.model = runtime
        .model_name
        .clone()
        .or_else(|| {
            settings.selected_model_id.as_deref().and_then(|id| {
                models
                    .iter()
                    .find(|model| model.id == id)
                    .map(|model| model.name.clone())
            })
        })
        .or_else(|| models.first().map(|model| model.name.clone()))
        .unwrap_or_else(|| "No local model selected".into());
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
        proven_hardware_profiles: hardware_profiles::all_proven_profiles(),
    })
}

#[tauri::command]
fn get_proven_hardware_profiles() -> Vec<hardware_profiles::ProvenHardwareProfile> {
    hardware_profiles::all_proven_profiles()
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

fn catalog_sources(
    state: &AppState,
    control: &ControlSettings,
    research: &ResearchSettings,
) -> model::CatalogSources {
    let mut roots = default_roots(&control.extra_model_roots, &research.bonsai_root);
    roots.push(state.model_downloads.models_root().to_path_buf());
    roots.sort();
    roots.dedup();
    model::CatalogSources {
        gguf_roots: roots,
        strata_installs: strata::known_installs(&[research.strata_root.as_str()]),
    }
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
    let sources = catalog_sources(state, &control, &research);
    let found = tokio::task::spawn_blocking(move || model::scan_sources(&sources))
        .await
        .map_err(|error| format!("model scan failed after download: {error}"))?;
    state.model_catalog.save(&found).map_err(|error| {
        format!("the model downloaded, but its catalog could not be saved: {error}")
    })?;
    *state.models.write().await = found.clone();
    if let Some(app) = app {
        let _ = crate::ipc_events::emit::<kestrel_app_core::events::ModelCatalog>(app, &found);
    }
    Ok(found)
}

fn ensure_workspace_idle(state: &AppState) -> Result<(), String> {
    app_control::ensure_task_access(state)?;
    if state.work_active.load(Ordering::Acquire) {
        Err("Chat, research, a computer task, movie production, or a model transfer is active. Stop or finish it before changing runtime or developer state.".into())
    } else {
        Ok(())
    }
}

fn claim_workspace(state: &AppState) -> Result<WorkGuard<'_>, String> {
    app_control::ensure_task_access(state)?;
    state
        .work_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| {
            "Chat, research, a computer task, movie production, or a model transfer is active. Stop or finish it before changing runtime or developer state.".to_string()
        })?;
    Ok(WorkGuard(&state.work_active))
}

fn open_with_explorer(path: &std::path::Path) -> Result<(), String> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not open {}: {error}", path.display()))
}

fn reveal_with_explorer(path: &std::path::Path) -> Result<(), String> {
    let mut selection = std::ffi::OsString::from("/select,");
    selection.push(path.as_os_str());
    std::process::Command::new("explorer.exe")
        .arg(selection)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not reveal {}: {error}", path.display()))
}

fn safe_export_filename(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, ' ' | '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let value = value.trim_matches([' ', '.']).trim();
    if value.is_empty() {
        "Kestrel MIDI".into()
    } else {
        value.chars().take(120).collect()
    }
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
            prompt_catalog::initialize(store.root()).map_err(|error| error.to_string())?;
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
            let models = model_catalog.load().unwrap_or_else(|error| {
                eprintln!("Kestrel could not restore its disposable model catalog: {error}");
                Vec::new()
            });
            let harness = ResearchHarness::new(store.clone());
            let developer = DeveloperAssistant::new(store.root());
            let workspace = WorkspaceStore::new(store.root())?;
            let attachments = AttachmentStore::new(&store.root().join("workspace"))?;
            let studio = MovieStudio::new(store.root()).map_err(|error| error.to_string())?;
            let music = MusicStudio::new(store.root()).map_err(|error| error.to_string())?;
            let images = ImageStudio::new(store.root()).map_err(|error| error.to_string())?;
            let speech = LocalSpeech::new(store.root()).map_err(|error| error.to_string())?;
            let voice_library = VoiceLibrary::new(store.root()).map_err(|error| error.to_string())?;
            app.manage(AppState {
                control: app_control::AppControl::new(store.root())?,
                speech_preferences: speech_preferences::SpeechPreferencesStore::new(store.root()),
                store,
                harness,
                research_settings,
                control_settings,
                model_catalog,
                model_downloads,
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
                voice_library,
                speech_command_gate: AsyncMutex::new(()),
                speech_jobs: Mutex::new(HashMap::new()),
                speech_restore_model: Mutex::new(None),
                interactive_jobs: Mutex::new(HashMap::new()),
                studio,
                music,
                images,
                movie_jobs: Mutex::new(HashMap::new()),
                music_jobs: Mutex::new(HashMap::new()),
                image_generation_jobs: Mutex::new(HashMap::new()),
                setup_job: Mutex::new(None),
                model_download_job: Mutex::new(None),
            });
            if let Some(path) = std::env::var_os("KESTREL_PRODUCER_SESSION") {
                app_control::transport::start(app.handle().clone(), &PathBuf::from(path))?;
            }
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
                let sources = catalog_sources(&state, &control, &research);
                let found = match tokio::task::spawn_blocking(move || model::scan_sources(&sources)).await {
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
                let _ = crate::ipc_events::emit::<kestrel_app_core::events::ModelCatalog>(&handle, &merged);
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_app_actions,
            describe_app_action,
            execute_app_action,
            get_app_action_receipt,
            get_app_job,
            create_producer_workflow,
            get_producer_workflow,
            list_producer_workflows,
            start_producer_workflow,
            stop_producer_workflow,
            import_context_paths,
            import_movie_reference_paths,
            export_production_bundle,
            export_media_composition,
            cancel_production_job,
            list_media_compositions,
            list_production_bundles,
            get_production_bundle,
            get_media_composition,
            apply_music_text,
            apply_image_composition,
            get_speech_preferences,
            save_vad_settings,
            save_research_speech_preferences,
            save_narration_speech_preferences,
            export_narration,
            export_music_lyrics,
            bootstrap,
            get_report,
            get_local_speech_snapshot,
            get_voice_library,
            create_voice_profile,
            update_voice_profile,
            set_default_voice_profile,
            delete_voice_profile,
            prepare_local_speech,
            get_cached_local_speech_clip,
            synthesize_local_speech,
            transcribe_local_speech,
            align_local_speech,
            cancel_local_speech,
            release_local_speech_memory,
            run_research,
            cancel_research,
            list_movies,
            get_movie,
            create_movie_producer_project,
            attach_movie_producer_references,
            get_movie_producer_workspace,
            get_movie_studio_conversation,
            save_movie_story_revision,
            accept_movie_story_revision,
            save_movie_scenes,
            reset_movie_studio_conversation,
            summarize_movie_studio_conversation,
            start_movie_studio_chat,
            cancel_movie_studio_chat,
            start_studio_prompt_draft,
            cancel_studio_prompt_draft,
            get_movie_render_state,
            pick_movie_reference_files,
            list_movie_image_assets,
            import_image_take_as_movie_reference,
            render_movie_scenes,
            cancel_movie_render,
            save_movie_edits,
            get_movie_editor_state,
            prepare_movie_editor_range,
            start_movie_editor_generation,
            render_movie_edit,
            reveal_movie,
            list_music_projects,
            get_music_project,
            create_music_project,
            save_music_project,
            create_music_lyrics_draft,
            get_music_lyrics_document,
            save_music_lyrics_document,
            transcribe_music_lyrics,
            repair_music_lyrics_range,
            draft_lyrics_from_audio_range,
            translate_music_lyrics,
            start_music_generation,
            cancel_music_generation,
            transcribe_music_midi,
            get_music_midi_document,
            save_music_midi_document,
            export_music_midi,
            reveal_music_midi,
            reveal_music_project,
            list_image_projects,
            get_image_project,
            create_image_project,
            save_image_project,
            start_image_generation,
            cancel_image_generation,
            reveal_image_project,
            prepare_services,
            get_setup_snapshot,
            open_comfy_ui,
            save_setup_locations,
            pick_setup_folder,
            pick_setup_file,
            scan_setup_model_folder,
            install_setup_component,
            cancel_setup_install,
            open_standalone_report,
            reveal_library,
            get_system_snapshot,
            preview_vram_cleanup,
            clean_vram,
            force_clean_vram,
            get_control_snapshot,
            get_strata_dashboard,
            get_strata_request_detail,
            get_proven_hardware_profiles,
            scan_local_models,
            list_model_downloads,
            inspect_model_download,
            start_model_download,
            resume_model_download,
            cancel_model_download,
            export_setup_profile,
            get_setup_profile_text,
            export_setup_profile_text,
            get_prompt_pack_text,
            get_default_prompt_pack_text,
            save_prompt_pack_text,
            reset_prompt_pack,
            export_prompt_pack_text,
            import_prompt_pack,
            pick_prompt_pack_file,
            import_setup_profile,
            import_setup_profile_text,
            save_research_settings,
            save_control_settings,
            apply_model_runtime,
            start_local_model,
            stop_local_model,
            release_ai_memory,
            list_chat_sessions,
            get_chat_session,
            delete_chat_session,
            save_chat_reply_edit,
            discard_chat_reply_edit,
            pick_context_files,
            open_context_attachment,
            pick_local_model_folder,
            start_chat_stream,
            cancel_chat_stream,
            list_computer_tasks,
            get_computer_task,
            get_task_memory,
            get_task_history,
            read_task_record,
            send_producer_message,
            acknowledge_task_action,
            get_producer_task_status,
            get_producer_environment,
            start_computer_task,
            start_producer_task,
            resume_computer_task,
            stop_computer_task,
            open_task_artifact,
            run_native_diagnostics,
            run_codex_repair,
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
