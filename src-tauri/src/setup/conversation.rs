//! Explicit installation only. Normal speech uses local paths and cannot fetch any assets.
use super::*;

#[derive(Deserialize)]
pub(crate) struct SpeechAsset {
    pub relative: String,
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
}

pub(crate) fn assets() -> Vec<SpeechAsset> {
    serde_json::from_str(include_str!(
        "../../resources/conversation_voice/assets.json"
    ))
    .expect("checked-in speech asset manifest")
}

const CONFIGS: [(&str, &str); 7] = [
    (
        "en",
        include_str!("../../resources/conversation_voice/en.yaml"),
    ),
    (
        "fr",
        include_str!("../../resources/conversation_voice/fr.yaml"),
    ),
    (
        "de",
        include_str!("../../resources/conversation_voice/de.yaml"),
    ),
    (
        "pt",
        include_str!("../../resources/conversation_voice/pt.yaml"),
    ),
    (
        "it",
        include_str!("../../resources/conversation_voice/it.yaml"),
    ),
    (
        "es",
        include_str!("../../resources/conversation_voice/es.yaml"),
    ),
    (
        "nl",
        include_str!("../../resources/conversation_voice/nl.yaml"),
    ),
];

fn local_config(template: &str, directory: &Path) -> String {
    template
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let file = if trimmed.starts_with("weights_path:")
                || trimmed.starts_with("weights_path_without_voice_cloning:")
            {
                Some("model.safetensors")
            } else if trimmed.starts_with("tokenizer_path:") {
                Some("tokenizer.json")
            } else {
                None
            };
            match file {
                Some(file) => format!(
                    "{}: {}",
                    line.split_once(':').expect("fixed YAML key").0,
                    serde_json::to_string(&directory.join(file).to_string_lossy())
                        .expect("path string")
                ),
                None => line.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

pub(crate) fn write_local_configs(root: &Path) -> std::io::Result<()> {
    for (language, template) in CONFIGS {
        let directory = root.join("models").join(language);
        fs::write(
            directory.join("config.yaml"),
            local_config(template, &directory),
        )?;
    }
    Ok(())
}

pub async fn install(
    app: Option<&AppHandle>,
    install: &Path,
    cancel: CancellationToken,
) -> Result<(), SetupError> {
    let root = install.join("ConversationVoice");
    fs::create_dir_all(&root)?;
    for item in assets() {
        let destination = root.join(&item.relative);
        fs::create_dir_all(destination.parent().expect("fixed model parent"))?;
        let asset = Asset::new(
            &format!("Conversation {}", item.relative),
            &item.url,
            destination
                .file_name()
                .expect("model filename")
                .to_string_lossy()
                .as_ref(),
            item.bytes,
            &item.sha256,
        );
        download(app, "conversation-voice", &asset, &destination, &cancel).await?;
    }
    write_local_configs(&root)?;
    crate::config::atomic_json_write(
        &root.join("ready.txt"),
        crate::conversation_voice::REVISION.as_bytes(),
    )
    .map_err(|e| SetupError::Dependency {
        name: "Native CPU voice conversation".into(),
        details: e.to_string(),
    })?;
    emit(app, "conversation-voice", "complete", "CPU voice conversation is installed for seven languages. It stays resident alongside the selected LLM. Existing production speech remains manual.", 1, 1, 0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_configs_are_local_and_manifest_is_pinned() {
        let directory = tempfile::tempdir().unwrap();
        for (_, template) in CONFIGS {
            let config = local_config(template, directory.path());
            assert!(!config.contains("hf://"));
            assert!(!config.contains("https://"));
            assert!(config.contains("model.safetensors"));
        }
        let assets = assets();
        assert_eq!(assets.len(), 24);
        assert!(
            assets
                .iter()
                .any(|asset| asset.relative == "models/whisper-medium-en-native/model.safetensors")
        );
        for asset in assets {
            assert!(asset.url.starts_with("https://huggingface.co/"));
            assert_eq!(
                asset
                    .url
                    .split("/resolve/")
                    .nth(1)
                    .unwrap()
                    .split('/')
                    .next()
                    .unwrap()
                    .len(),
                40
            );
            assert_eq!(asset.sha256.len(), 64);
            assert!(asset.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(!asset.relative.contains(".."));
        }
    }

    #[tokio::test]
    #[ignore = "explicitly downloads and installs the pinned CPU speech runtime in KESTREL_VOICE_TEST_ROOT"]
    async fn live_install_cpu_voice() {
        let root = PathBuf::from(
            std::env::var("KESTREL_VOICE_TEST_ROOT").expect("explicit installation test root"),
        );
        assert!(root.is_absolute());
        install(None, &root, CancellationToken::new())
            .await
            .unwrap();
        assert!(crate::conversation_voice::installed(root.to_str().unwrap()));
    }
}
