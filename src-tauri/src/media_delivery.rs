//! Shared verification and bounded probing of native artifacts.
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;

pub(crate) fn verified_source(
    root: &Path,
    source: &str,
    hash: Option<&str>,
) -> Result<PathBuf, String> {
    let path = PathBuf::from(source)
        .canonicalize()
        .map_err(|e| format!("Production source is unavailable: {source}: {e}"))?;
    if !path.starts_with(root.canonicalize().map_err(|e| e.to_string())?) || !path.is_file() {
        return Err(
            "Production sources must be native artifacts inside the Kestrel library.".into(),
        );
    }
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024 * 1024 {
        return Err("Production source exceeds 32 GiB.".into());
    }
    if let Some(hash) = hash {
        if crate::studio::hash_file(&path)
            .map_err(|e| e.to_string())?
            .1
            != hash
        {
            return Err(
                "Production source hash changed; restore the original immutable take.".into(),
            );
        }
    }
    Ok(path)
}
async fn run(executable: &Path, args: &[OsString]) -> Result<Vec<u8>, String> {
    let output = tokio::time::timeout(
        Duration::from_secs(3600),
        Command::new(executable)
            .args(args)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "FFmpeg export exceeded one hour; partial staging files were preserved.")?
    .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Media command failed: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(4000)
                .collect::<String>()
        ));
    }
    Ok(output.stdout)
}
pub(crate) async fn probe(executable: &Path, path: &Path) -> Result<serde_json::Value, String> {
    serde_json::from_slice(
        &run(
            executable,
            &[
                "-v".into(),
                "error".into(),
                "-show_entries".into(),
                "format=duration:stream=codec_type,codec_name,width,height".into(),
                "-of".into(),
                "json".into(),
                path.as_os_str().into(),
            ],
        )
        .await?,
    )
    .map_err(|e| e.to_string())
}
pub(crate) fn probe_duration(value: &serde_json::Value) -> Result<f64, String> {
    value
        .pointer("/format/duration")
        .and_then(serde_json::Value::as_str)
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= 43_200.0)
        .ok_or_else(|| "Media needs a finite duration of at most twelve hours.".into())
}
pub(crate) fn has_stream(value: &serde_json::Value, kind: &str) -> bool {
    value
        .get("streams")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|streams| {
            streams.iter().any(|stream| {
                stream.get("codec_type").and_then(serde_json::Value::as_str) == Some(kind)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn package_rejects_non_artifact_sources_and_changed_hashes() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("take.wav");
        fs::write(&file, b"audio").unwrap();
        assert!(verified_source(temp.path(), &file.to_string_lossy(), Some("wrong")).is_err());
        assert!(
            verified_source(&temp.path().join("other"), &file.to_string_lossy(), None).is_err()
        );
        assert!(probe_duration(&json!({"format":{"duration":"NaN"}})).is_err());
    }
}
