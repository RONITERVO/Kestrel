//! Compatibility adapter for the upstream loader: only existing absolute local files.
use anyhow::{Result, bail};
use std::path::PathBuf;

pub fn download_if_necessary(source: &str) -> Result<PathBuf> {
    let path = PathBuf::from(source);
    if !path.is_absolute() || !path.is_file() {
        bail!("Missing local speech asset. Prepare Voice conversation in Setup: {source}");
    }
    Ok(path)
}
#[cfg(test)]
mod tests {
    #[test]
    fn runtime_never_downloads_or_resolves_relative_paths() {
        for value in [
            "hf://repo/model",
            "https://example.com/model",
            "model.bin",
            "../model",
        ] {
            assert!(super::download_if_necessary(value).is_err());
        }
    }
}
