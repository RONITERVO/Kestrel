//! Bounded, authenticated observations of the Strata process already owned by RuntimeManager.
use crate::runtime::{authorized, ModelConnection, RuntimeManager};
use kestrel_app_core::{
    StrataDashboardActivity, StrataDashboardHardware, StrataDashboardRequest,
    StrataDashboardRequestDetail, StrataDashboardSnapshot,
};
use serde_json::Value;
use std::time::Duration;

const MAX_RESPONSE: usize = 4 * 1024 * 1024;
const MAX_TEXT: usize = 262_144;

fn endpoint(connection: &ModelConnection, path: &str) -> Result<url::Url, String> {
    let mut url = url::Url::parse(&connection.endpoint).map_err(|_| "Invalid runtime endpoint")?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/v1"
    {
        return Err("The Strata dashboard requires Kestrel's owned loopback runtime.".into());
    }
    url.set_path(path);
    Ok(url)
}

async fn get(connection: &ModelConnection, path: &str, id: Option<&str>) -> Result<Value, String> {
    let mut url = endpoint(connection, path)?;
    if let Some(id) = id {
        if id.len() != 12 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("Invalid Strata request ID.".into());
        }
        url.query_pairs_mut().append_pair("id", id);
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = authorized(client.get(url), connection)
        .send()
        .await
        .map_err(|_| {
            "Strata monitoring is unavailable. Check that the managed model is still running."
        })?;
    if !response.status().is_success() {
        return Err(format!("Strata monitoring returned {}. Request details require a Strata version with API monitoring support. Update Strata if needed, then restart the model in Kestrel.", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RESPONSE as u64)
    {
        return Err("Strata monitoring response exceeds 4 MiB.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Strata monitoring read failed")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
            return Err("Strata monitoring response exceeds 4 MiB.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Strata monitoring returned invalid JSON.".into())
}

fn number(value: &Value, pointer: &str) -> Option<f64> {
    value
        .pointer(pointer)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite() && *n >= 0.0)
}

fn text(value: &Value, pointer: &str, limit: usize) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .take(limit)
        .collect()
}

fn history(value: &Value, pointer: &str, scale: f64) -> Vec<Option<f64>> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .skip(items.len().saturating_sub(120))
                .map(|item| {
                    item.as_f64()
                        .filter(|n| n.is_finite() && *n >= 0.0)
                        .map(|n| n * scale)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn request(value: &Value) -> StrataDashboardRequest {
    StrataDashboardRequest {
        id: text(value, "/id", 64),
        path: text(value, "/path", 128),
        state: text(value, "/state", 64),
        started_at: number(value, "/started_at"),
        wall_seconds: number(value, "/wallclock_s"),
        queue_seconds: number(value, "/queue_s"),
        load_seconds: number(value, "/load_s"),
        first_token_seconds: number(value, "/first_token_s"),
        prompt_tokens: number(value, "/usage/prompt_tokens")
            .or_else(|| number(value, "/usage/input_tokens")),
        reused_tokens: number(value, "/timings/cache_n"),
        output_tokens: number(value, "/usage/completion_tokens")
            .or_else(|| number(value, "/usage/output_tokens")),
        prefill_tokens_per_second: number(value, "/timings/prompt_per_second"),
        output_tokens_per_second: number(value, "/timings/predicted_per_second"),
    }
}

pub async fn snapshot(runtime: &RuntimeManager) -> StrataDashboardSnapshot {
    let mut result = StrataDashboardSnapshot {
        runtime: runtime.snapshot().await, available: false,
        detail: "Start a Strata model in Kestrel to view its dashboard. Opening this view never starts a model.".into(),
        activity: None, hardware: None, requests: Vec::new(), request_capture: false,
    };
    let Some(connection) = runtime.strata_monitor_connection().await else {
        return result;
    };
    let (metrics, requests) = tokio::join!(
        get(&connection, "/metrics", None),
        get(&connection, "/api/requests", None)
    );
    let metrics = match metrics {
        Ok(value) => value,
        Err(error) => {
            result.detail = error;
            return result;
        }
    };
    result.available = true;
    result.detail = "Live observations from Kestrel's managed Strata. Recent request details are held in Strata's memory until the runtime stops.".into();
    result.activity = Some(activity(&metrics, result.runtime.context_window));
    result.hardware = Some(hardware(&metrics));
    match requests {
        Ok(value) => {
            result.request_capture = true;
            result.requests = value
                .get("requests")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(100)
                .map(request)
                .collect();
        }
        Err(error) => result.detail = format!("Live metrics are available. {error}"),
    }
    result
}

fn activity(value: &Value, context: u32) -> StrataDashboardActivity {
    let phase = text(value, "/live/phase", 64);
    StrataDashboardActivity {
        model: text(value, "/engine/model", 256),
        context_window: value
            .pointer("/engine/max_context")
            .and_then(Value::as_u64)
            .and_then(|n| n.try_into().ok())
            .unwrap_or(context),
        images: value
            .pointer("/engine/images")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        phase: if phase.is_empty() {
            text(value, "/live/state", 64)
        } else {
            phase
        },
        queued: value
            .pointer("/live/queued")
            .and_then(Value::as_u64)
            .and_then(|n| n.try_into().ok())
            .unwrap_or(0),
        prompt_tokens: number(value, "/live/prompt_tokens"),
        prompt_read: number(value, "/live/prompt_read"),
        prompt_total: number(value, "/live/prompt_total"),
        generated_tokens: number(value, "/live/generated"),
        max_output_tokens: number(value, "/live/max_tokens"),
        elapsed_seconds: number(value, "/live/elapsed_s"),
        tokens_per_second: number(value, "/live/tok_s"),
        prefill_tokens_per_second: number(value, "/live/prefill_tok_s_mean"),
    }
}

fn hardware(value: &Value) -> StrataDashboardHardware {
    let ram_total = number(value, "/hardware/ram_total");
    StrataDashboardHardware {
        gpu_name: text(value, "/hardware_static/gpu_name", 256),
        gpu_percent: number(value, "/hardware/gpu_util"),
        gpu_used_mib: number(value, "/hardware/gpu_mem_used").map(|n| n / 1_048_576.0),
        gpu_total_mib: number(value, "/hardware/gpu_mem_total").map(|n| n / 1_048_576.0),
        gpu_temperature: number(value, "/hardware/gpu_temp"),
        gpu_watts: number(value, "/hardware/gpu_power"),
        cpu_percent: number(value, "/hardware/cpu"),
        ram_used_gib: number(value, "/hardware/ram_used").map(|n| n / 1_073_741_824.0),
        ram_total_gib: ram_total.map(|n| n / 1_073_741_824.0),
        gpu_history: history(value, "/history/gpu_util", 1.0),
        cpu_history: history(value, "/history/cpu", 1.0),
        ram_percent_history: ram_total
            .filter(|n| *n > 0.0)
            .map(|total| history(value, "/history/ram_used", 100.0 / total))
            .unwrap_or_default(),
        output_speed_history: history(value, "/history/tok_s", 1.0),
    }
}

pub async fn detail(
    runtime: &RuntimeManager,
    id: &str,
) -> Result<StrataDashboardRequestDetail, String> {
    let connection = runtime
        .strata_monitor_connection()
        .await
        .ok_or("The managed Strata runtime is not running.")?;
    let value = get(&connection, "/api/requests", Some(id)).await?;
    let truncated = |key: &str| {
        value
            .get(format!("{key}_truncated"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || value
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|s| s.chars().count() > MAX_TEXT)
    };
    Ok(StrataDashboardRequestDetail {
        request: request(&value),
        input: text(&value, "/input", MAX_TEXT),
        output: text(&value, "/output", MAX_TEXT),
        reasoning: text(&value, "/reasoning", MAX_TEXT),
        input_truncated: truncated("input"),
        output_truncated: truncated("output"),
        reasoning_truncated: truncated("reasoning"),
        error: value
            .get("error")
            .filter(|v| !v.is_null())
            .map(|v| v.to_string().chars().take(4096).collect()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dashboard_uses_native_context_and_engine_timings_without_fabricating_missing_metrics() {
        let metrics = json!({"engine":{"model":"strata", "max_context":65536,"images":true},"live":{"phase":"thinking","generated":123,"tok_s":57.2},"hardware":{"ram_used":32212254720u64,"ram_total":64424509440u64},"history":{"ram_used":[32212254720u64]}});
        let view = activity(&metrics, 32768);
        assert_eq!(view.context_window, 65536);
        assert_eq!(view.tokens_per_second, Some(57.2));
        assert_eq!(view.prompt_tokens, None);
        let hardware = hardware(&metrics);
        assert_eq!(hardware.gpu_percent, None);
        assert_eq!(hardware.ram_percent_history, vec![Some(50.0)]);
        let row = request(
            &json!({"usage":{"prompt_tokens":4096,"completion_tokens":100},"timings":{"cache_n":32,"prompt_per_second":800.0,"predicted_per_second":55.0}}),
        );
        assert_eq!(row.reused_tokens, Some(32.0));
        assert_eq!(row.prefill_tokens_per_second, Some(800.0));
        assert_eq!(row.output_tokens_per_second, Some(55.0));
    }

    #[test]
    fn monitoring_cannot_address_an_unowned_or_remote_endpoint() {
        let mut connection = ModelConnection {
            endpoint: "http://127.0.0.1:19271/v1".into(),
            api_key: Some("secret".into()),
            model_id: "model".into(),
            model_label: "Strata".into(),
            engine: crate::model::ModelEngine::Strata,
        };
        assert_eq!(
            endpoint(&connection, "/metrics").unwrap().as_str(),
            "http://127.0.0.1:19271/metrics"
        );
        for invalid in [
            "http://example.com/v1",
            "http://127.0.0.1:8080/v1?x=1",
            "http://secret@127.0.0.1:8080/v1",
            "file:///v1",
        ] {
            connection.endpoint = invalid.into();
            assert!(endpoint(&connection, "/metrics").is_err());
        }
    }
}
