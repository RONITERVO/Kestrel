//! Only this adapter may emit Tauri events. The registry fixes each name and payload together.
use kestrel_app_core::events::DesktopEvent;
use tauri::{AppHandle, Emitter};

pub fn emit<E: DesktopEvent>(app: &AppHandle, payload: &E::Payload) -> tauri::Result<()> {
    app.emit(E::NAME, payload)
}
