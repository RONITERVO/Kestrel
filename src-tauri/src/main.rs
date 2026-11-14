#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--conversation-voice-worker")) {
        let result = std::env::args_os().nth(2).ok_or_else(|| "Missing speech asset directory".to_string())
            .and_then(|root| kestrel_conversation_engine::worker::run(root.into()).map_err(|e| e.to_string()));
        if let Err(error) = result { eprintln!("{error}"); std::process::exit(1); }
        return;
    }
    kestrel_local_lib::run();
}
