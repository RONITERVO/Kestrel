#[path = "build/command_bindings.rs"]
mod command_bindings;

fn main() {
    command_bindings::generate();
    tauri_build::build()
}
