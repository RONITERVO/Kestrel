use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub(super) const PREVIEW_NODE_ID: &str = "90";
pub(super) const PREVIEW_NODE_REVISION: &str = "5219cd171cb44e2edce9e4daad6cc42c41eded5c";
pub(super) const PREVIEW_DECODER_REVISION: &str = "62f7591f59dfbb4c3c02b7a621d180a9eeaba26c";
pub(super) const PREVIEW_DECODER_SHA256: &str =
    "4fd022bfcab08772fe0536b17ea1a3bbb5625be11e397868d1c5d891863d4c13";
const MAX_ENCODED_PREVIEW_BYTES: usize = 12 * 1024 * 1024;
const MAX_DECODED_PREVIEW_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MovieRenderPreviewEvent {
    pub kind: String,
    pub target: String,
    pub job_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clip_index: Option<usize>,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub average_step_ms: Option<f64>,
    pub preview_node_revision: &'static str,
    pub preview_decoder_revision: &'static str,
    pub preview_decoder_sha256: &'static str,
    pub at: String,
}

#[derive(Debug, Clone)]
pub(super) struct PreviewTarget {
    target: &'static str,
    job_id: String,
    project_id: Option<String>,
    clip_id: Option<String>,
    clip_index: Option<usize>,
}

impl PreviewTarget {
    pub(super) fn image_asset(request_id: &str) -> Self {
        Self {
            target: "imageAsset",
            job_id: request_id.into(),
            project_id: None,
            clip_id: None,
            clip_index: None,
        }
    }

    pub(super) fn movie_clip(
        job_id: String,
        project_id: &str,
        clip_id: &str,
        clip_index: usize,
    ) -> Self {
        Self {
            target: "movieClip",
            job_id,
            project_id: Some(project_id.into()),
            clip_id: Some(clip_id.into()),
            clip_index: Some(clip_index),
        }
    }

    fn event(&self, kind: &str, detail: impl Into<String>) -> MovieRenderPreviewEvent {
        MovieRenderPreviewEvent {
            kind: kind.into(),
            target: self.target.into(),
            job_id: self.job_id.clone(),
            project_id: self.project_id.clone(),
            clip_id: self.clip_id.clone(),
            clip_index: self.clip_index,
            detail: detail.into(),
            mime_type: None,
            data_url: None,
            width: None,
            height: None,
            step: None,
            total: None,
            fps: None,
            step_ms: None,
            average_step_ms: None,
            preview_node_revision: PREVIEW_NODE_REVISION,
            preview_decoder_revision: PREVIEW_DECODER_REVISION,
            preview_decoder_sha256: PREVIEW_DECODER_SHA256,
            at: Utc::now().to_rfc3339(),
        }
    }
}

pub(super) struct LivePreviewSession {
    app: AppHandle,
    target: PreviewTarget,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

pub(super) fn emit_preview_unavailable(app: Option<&AppHandle>, target: PreviewTarget) {
    if let Some(app) = app {
        let _ = app.emit(
            "movie-render-preview",
            target.event(
                "unavailable",
                "Approximate live preview is unavailable; full-quality local rendering continues.",
            ),
        );
    }
}

impl LivePreviewSession {
    pub(super) async fn connect(
        app: Option<&AppHandle>,
        client_id: &str,
        target: PreviewTarget,
    ) -> Option<Self> {
        let app = app?.clone();
        let url = format!("ws://127.0.0.1:8188/ws?clientId={client_id}");
        let (stream, _) = match tokio_tungstenite::connect_async(&url).await {
            Ok(value) => value,
            Err(error) => {
                let _ = app.emit(
                    "movie-render-preview",
                    target.event(
                        "unavailable",
                        format!("Live preview could not connect to the local renderer: {error}"),
                    ),
                );
                return None;
            }
        };
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task_app = app.clone();
        let task_target = target.clone();
        let task = tokio::spawn(async move {
            let (_, mut reader) = stream.split();
            let _ = task_app.emit(
                "movie-render-preview",
                task_target.event(
                    "connected",
                    "Live H3 preview is connected and waiting for the first sample.",
                ),
            );
            loop {
                tokio::select! {
                    _ = task_cancel.cancelled() => break,
                    message = reader.next() => {
                        let Some(message) = message else { break; };
                        match message {
                            Ok(message) if message.is_text() => {
                                if let Ok(text) = message.into_text() {
                                    if let Some(event) = parse_preview_message(&text, &task_target) {
                                        let _ = task_app.emit("movie-render-preview", event);
                                    }
                                }
                            }
                            Ok(message) if message.is_close() => break,
                            Err(error) => {
                                let _ = task_app.emit(
                                    "movie-render-preview",
                                    task_target.event("unavailable", format!("The local live preview stream closed: {error}")),
                                );
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        });
        Some(Self {
            app,
            target,
            cancel,
            task,
        })
    }

    pub(super) fn finish(&self) {
        let _ = self.app.emit(
            "movie-render-preview",
            self.target.event(
                "finished",
                "Sampling finished. Kestrel is preserving the full-VAE master.",
            ),
        );
        self.cancel.cancel();
    }
}

impl Drop for LivePreviewSession {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.task.abort();
    }
}

fn parse_preview_message(text: &str, target: &PreviewTarget) -> Option<MovieRenderPreviewEvent> {
    let value: Value = serde_json::from_str(text).ok()?;
    if value.get("type")?.as_str()? != "kj_preview_override" {
        return None;
    }
    let data = value.get("data")?;
    if data.get("node_id")?.as_str()? != PREVIEW_NODE_ID {
        return None;
    }
    let mime = data.get("mime")?.as_str()?;
    if !matches!(
        mime,
        "image/jpeg" | "image/png" | "image/webp" | "video/mp4"
    ) {
        return None;
    }
    let encoded = data.get("image")?.as_str()?;
    if encoded.is_empty() || encoded.len() > MAX_ENCODED_PREVIEW_BYTES {
        return None;
    }
    let decoded = STANDARD.decode(encoded).ok()?;
    if decoded.is_empty() || decoded.len() > MAX_DECODED_PREVIEW_BYTES {
        return None;
    }
    let width = bounded_u32(data.get("w"), 1, 1_024)?;
    let height = bounded_u32(data.get("h"), 1, 1_024)?;
    let step = bounded_u32(data.get("step"), 0, 1_000)?;
    let total = bounded_u32(data.get("total"), 1, 1_000)?;
    if step > total {
        return None;
    }
    let mut event = target.event(
        "frame",
        format!("Approximate live preview · sample {step} of {total}"),
    );
    event.mime_type = Some(mime.into());
    event.data_url = Some(format!("data:{mime};base64,{encoded}"));
    event.width = Some(width);
    event.height = Some(height);
    event.step = Some(step);
    event.total = Some(total);
    event.fps = bounded_number(data.get("fps"), 0.0, 240.0);
    event.step_ms = bounded_number(data.get("step_ms"), 0.0, 3_600_000.0);
    event.average_step_ms = bounded_number(data.get("avg_step_ms"), 0.0, 3_600_000.0);
    Some(event)
}

fn bounded_u32(value: Option<&Value>, minimum: u32, maximum: u32) -> Option<u32> {
    let value = value?.as_u64()?;
    u32::try_from(value)
        .ok()
        .filter(|value| (*value >= minimum) && (*value <= maximum))
}

fn bounded_number(value: Option<&Value>, minimum: f64, maximum: f64) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= minimum && *value <= maximum)
}

pub(super) fn preview_node(model_node: &str, preview_frames: u32) -> Value {
    serde_json::json!({
        "class_type": "ModelPreviewOverrideKJ",
        "inputs": {
            "model": [model_node, 0],
            "max_resolution": 512,
            "jpeg_quality": 80,
            "suppress_default_preview": true,
            "preview_frames": preview_frames,
            "preview_fps": 6,
            "tiny_vae": "taeh3.safetensors"
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_bounded_preview_payloads_from_the_expected_node() {
        let target = PreviewTarget::image_asset("job");
        let payload = serde_json::json!({
            "type":"kj_preview_override",
            "data":{
                "node_id":"90",
                "image":STANDARD.encode([1_u8, 2, 3]),
                "mime":"image/jpeg",
                "w":512,
                "h":288,
                "step":3,
                "total":20,
                "avg_step_ms":1200.0
            }
        });
        let event = parse_preview_message(&payload.to_string(), &target).unwrap();
        assert_eq!(event.kind, "frame");
        assert_eq!(event.step, Some(3));
        assert!(event
            .data_url
            .unwrap()
            .starts_with("data:image/jpeg;base64,"));

        let wrong_node = payload.to_string().replace("\"90\"", "\"91\"");
        assert!(parse_preview_message(&wrong_node, &target).is_none());
    }
}
