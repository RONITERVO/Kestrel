#![recursion_limit = "256"]

mod config;
mod developer;
mod harness;
mod html;
mod kiwix;
mod model;
mod models;
mod runtime;
mod services;
mod store;

use config::{ControlSettingsStore, SettingsStore};
use developer::DeveloperAssistant;
use harness::ResearchHarness;
use model::{default_roots, ModelInfo};
use models::{
    AppSnapshot, ChatRequest, ChatResponse, ControlSettings, ControlSnapshot,
    DeveloperRepairReport, DeveloperRepairRequest, ResearchReport, ResearchSettings,
    RunResearchRequest, SystemSnapshot,
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
use tauri::{AppHandle, Manager, State};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Shared native state. Keep authority visibly separated: research owns evidence/storage, runtime
/// owns the only model process, and developer owns the optional Codex child.
struct AppState {
    store: ResearchStore,
    harness: ResearchHarness,
    research_settings: SettingsStore,
    control_settings: ControlSettingsStore,
    models: RwLock<Vec<ModelInfo>>,
    runtime: Arc<RuntimeManager>,
    developer: DeveloperAssistant,
    research_active: AtomicBool,
    jobs: Mutex<HashMap<String, CancellationToken>>,
}

struct ResearchGuard<'a>(&'a AtomicBool);

impl Drop for ResearchGuard<'_> {
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
        .research_active
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| "Another offline research job is already active.".to_string())?;
    let _guard = ResearchGuard(&state.research_active);
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
    ensure_not_researching(&state)?;
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
async fn get_control_snapshot(state: State<'_, AppState>) -> Result<ControlSnapshot, String> {
    control_snapshot(&state, true).await
}

#[tauri::command]
async fn scan_local_models(state: State<'_, AppState>) -> Result<ControlSnapshot, String> {
    ensure_not_researching(&state)?;
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
    *state.models.write().await = found;
    control_snapshot(&state, true).await
}

#[tauri::command]
fn save_research_settings(
    settings: ResearchSettings,
    state: State<'_, AppState>,
) -> Result<ResearchSettings, String> {
    state
        .research_settings
        .save(&settings)
        .map_err(|error| error.to_string())?;
    Ok(settings)
}

#[tauri::command]
async fn save_control_settings(
    settings: ControlSettings,
    state: State<'_, AppState>,
) -> Result<ControlSnapshot, String> {
    state
        .control_settings
        .save(&settings)
        .map_err(|error| error.to_string())?;
    control_snapshot(&state, true).await
}

#[tauri::command]
async fn apply_model_runtime(
    settings: ResearchSettings,
    state: State<'_, AppState>,
) -> Result<SystemSnapshot, String> {
    ensure_not_researching(&state)?;
    state
        .research_settings
        .save(&settings)
        .map_err(|error| error.to_string())?;
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
    ensure_not_researching(&state)?;
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
    ensure_not_researching(&state)?;
    state
        .runtime
        .stop_managed()
        .await
        .map_err(|error| error.to_string())?;
    control_snapshot(&state, true).await
}

#[tauri::command]
async fn send_local_chat(
    request: ChatRequest,
    state: State<'_, AppState>,
) -> Result<ChatResponse, String> {
    ensure_not_researching(&state)?;
    let settings = state
        .control_settings
        .load()
        .map_err(|error| error.to_string())?;
    let models = state.models.read().await.clone();
    state
        .runtime
        .chat(request, &models, &settings)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn run_native_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    ensure_not_researching(&state)?;
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
    ensure_not_researching(&state)?;
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
        settings,
        models: state.models.read().await.clone(),
        runtime: state.runtime.snapshot().await,
        gpu: services::gpu_snapshot().await,
        developer,
    })
}

fn ensure_not_researching(state: &AppState) -> Result<(), String> {
    if state.research_active.load(Ordering::Acquire) {
        Err("Strict offline research is active. Stop or finish it before changing runtime/developer state.".into())
    } else {
        Ok(())
    }
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
    tauri::Builder::default()
        .setup(|app| {
            let store = ResearchStore::open_default().map_err(|error| error.to_string())?;
            let research_settings = SettingsStore::new(store.root());
            let control_settings = ControlSettingsStore::new(store.root());
            let research = research_settings
                .load()
                .map_err(|error| error.to_string())?;
            control_settings.load().map_err(|error| error.to_string())?;
            // Startup stays fast: inspect Bonsai immediately and let an explicit rescan cover large libraries.
            let bonsai_root = vec![std::path::Path::new(&research.bonsai_root).join("models")];
            let models = model::scan(&bonsai_root);
            let harness = ResearchHarness::new(store.clone());
            let developer = DeveloperAssistant::new(store.root());
            app.manage(AppState {
                store,
                harness,
                research_settings,
                control_settings,
                models: RwLock::new(models),
                runtime: Arc::new(RuntimeManager::new()),
                developer,
                research_active: AtomicBool::new(false),
                jobs: Mutex::new(HashMap::new()),
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
            save_research_settings,
            save_control_settings,
            apply_model_runtime,
            start_local_model,
            stop_local_model,
            send_local_chat,
            run_native_diagnostics,
            run_codex_repair,
            open_bonsai_control_center,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kestrel Local");
}
