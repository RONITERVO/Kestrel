mod harness;
mod html;
mod kiwix;
mod models;
mod services;
mod store;

use harness::ResearchHarness;
use models::{AppSnapshot, ResearchReport, RunResearchRequest};
use std::collections::HashMap;
use std::sync::Mutex;
use store::ResearchStore;
use tauri::{AppHandle, Manager, State};
use tokio_util::sync::CancellationToken;

struct AppState {
    store: ResearchStore,
    harness: ResearchHarness,
    jobs: Mutex<HashMap<String, CancellationToken>>,
}

#[tauri::command]
async fn bootstrap(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    snapshot(&state.store).await
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
    let job_id = uuid::Uuid::new_v4().to_string();
    let cancel = CancellationToken::new();
    state
        .jobs
        .lock()
        .map_err(|_| "research job registry is unavailable".to_string())?
        .insert(job_id.clone(), cancel.clone());
    let result = state
        .harness
        .run(Some(&app), request, &job_id, cancel)
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
    services::prepare()
        .await
        .map_err(|error| error.to_string())?;
    snapshot(&state.store).await
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

async fn snapshot(store: &ResearchStore) -> Result<AppSnapshot, String> {
    let status = services::status().await;
    let reports = store.list(100_000).map_err(|error| error.to_string())?;
    Ok(AppSnapshot {
        status,
        reports,
        library_root: store.root().to_string_lossy().into_owned(),
    })
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
            let harness = ResearchHarness::new(store.clone());
            app.manage(AppState {
                store,
                harness,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Kestrel Local");
}
