//! Only this adapter may emit Tauri events. The registry fixes each name and payload together.
use kestrel_app_core::events::DesktopEvent;
use tauri::{AppHandle, Emitter, Manager};

pub fn emit<E: DesktopEvent>(app: &AppHandle, payload: &E::Payload) -> tauri::Result<()> {
    if let Some(state) = app.try_state::<crate::AppState>().filter(|_| {
        matches!(
            E::NAME,
            "studio-prompt-draft"
                | "movie-studio-chat"
                | "research-progress"
                | "image-generation"
                | "music-generation"
                | "movie-project"
                | "movie-render-preview"
                | "runtime-progress"
                | "local-speech-progress"
        )
    }) {
        if let Ok(value) = serde_json::to_value(payload) {
            if let Err(error) = state.control.observe(E::NAME, value) {
                eprintln!("Kestrel could not persist the producer observation: {error}");
            }
        }
    }
    app.emit(E::NAME, payload)
}
