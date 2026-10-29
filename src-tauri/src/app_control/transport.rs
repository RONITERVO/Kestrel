//! Explicitly enabled, local Windows named pipe. No HTTP listener, MCP, or model integration.
use super::*;

#[cfg(windows)]
pub fn start(app: AppHandle, descriptor: &Path) -> Result<(), String> {
    use std::io::Write;
    use tokio::net::windows::named_pipe::ServerOptions;
    if !descriptor.is_absolute() {
        return Err("Producer session file must be an absolute path.".into());
    }
    let session = kestrel_app_core::ProducerControlSession {
        pipe: format!(r"\\.\pipe\kestrel-producer-{}", uuid::Uuid::new_v4()),
        token: format!("{}{}", uuid::Uuid::new_v4(), uuid::Uuid::new_v4()),
        protocol_version: 1,
    };
    // Tauri setup runs on its main thread, outside Tokio's reactor context.
    let first = tauri::async_runtime::block_on(async {
        ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&session.pipe)
    })
    .map_err(|e| e.to_string())?;
    // Never overwrite a user file or silently recycle a prior authorization.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(descriptor)
        .map_err(|e| {
            format!(
                "Cannot create producer session {}: {e}. Choose a new session filename.",
                descriptor.display()
            )
        })?;
    file.write_all(&serde_json::to_vec_pretty(&session).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn(async move {
        let mut server = first;
        let limit = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
        loop {
            if let Err(error) = server.connect().await {
                eprintln!("Producer pipe stopped: {error}");
                break;
            }
            let next = match ServerOptions::new()
                .reject_remote_clients(true)
                .create(&session.pipe)
            {
                Ok(next) => next,
                Err(error) => {
                    eprintln!("Producer pipe stopped: {error}");
                    break;
                }
            };
            let connected = server;
            server = next;
            let Ok(permit) = limit.clone().try_acquire_owned() else {
                continue;
            };
            let app = app.clone();
            let token = session.token.clone();
            tauri::async_runtime::spawn(async move {
                let _permit = permit;
                if let Err(error) = serve(app, connected, &token).await {
                    eprintln!("Producer connection: {error}");
                }
            });
        }
    });
    Ok(())
}

#[cfg(not(windows))]
pub fn start(_: AppHandle, _: &Path) -> Result<(), String> {
    Err("The producer pipe adapter currently requires Windows. Native actions and Computer Tasks are platform-independent.".into())
}

#[cfg(windows)]
async fn serve(
    app: AppHandle,
    stream: tokio::net::windows::named_pipe::NamedPipeServer,
    token: &str,
) -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Envelope {
        token: String,
        request: AppActionRequest,
    }
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader.take((MAX_BYTES + 1) as u64));
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        reader.read_line(&mut line),
    )
    .await
    .map_err(|_| "Producer request timed out.")?
    .map_err(|e| e.to_string())?;
    let result = async {
        if line.len() > MAX_BYTES || !line.ends_with('\n') {
            return Err("Producer request must be one JSON line up to 8 MiB.".to_string());
        }
        let envelope: Envelope = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        if envelope.token != token {
            return Err("Producer session authorization failed.".into());
        }
        let request = envelope.request;
        if protocol_command(&request.command) {
            dispatch_app_command(&app, &request.command, request.args).await
        } else {
            serde_json::to_value(execute(&app, request).await?).map_err(|e| e.to_string())
        }
    }
    .await;
    let response = match result {
        Ok(value) => json!({"ok":true,"result":value}),
        Err(error) => json!({"ok":false,"error":error}),
    };
    let mut bytes = serde_json::to_vec(&response).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        bytes = serde_json::to_vec(
            &json!({"ok":false,"error":"Response exceeds 8 MiB; use a smaller observation."}),
        )
        .map_err(|e| e.to_string())?;
    }
    bytes.push(b'\n');
    writer.write_all(&bytes).await.map_err(|e| e.to_string())?;
    writer.shutdown().await.map_err(|e| e.to_string())
}

pub fn protocol_command(command: &str) -> bool {
    matches!(
        command,
        "list_app_actions"
            | "describe_app_action"
            | "get_app_action_receipt"
            | "get_app_job"
            | "list_producer_workflows"
            | "get_producer_workflow"
    )
}
