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
            let file = if trimmed.starts_with("weights_path:") {
                // Optional, producer-supplied authorized cloning weights. The official public
                // checkpoint and voice embeddings work without a Hugging Face login.
                Some(if directory.join("voice-cloning.safetensors").is_file() {
                    "voice-cloning.safetensors"
                } else {
                    "model.safetensors"
                })
            } else if trimmed.starts_with("weights_path_without_voice_cloning:") {
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
    let runtime = root.join("runtime");
    let downloads = root.join("downloads");
    fs::create_dir_all(&downloads)?;
    let uv_asset = uv_runner_asset("Conversation speech isolated Python runner");
    let archive = downloads.join(&uv_asset.file_name);
    download(app, "conversation-voice", &uv_asset, &archive, &cancel).await?;
    unzip(&archive, &runtime, &uv_asset.name)?;
    let uv = runtime.join("uv.exe");
    if !uv.is_file() {
        return Err(dependency("The verified uv archive is missing uv.exe."));
    }
    let requirements = root.join("requirements.lock");
    fs::write(
        &requirements,
        include_bytes!("../../resources/conversation_voice/requirements.lock"),
    )?;
    emit(app, "conversation-voice", "dependencies", "Preparing isolated CPU Pocket TTS and compact Whisper; existing speech and the selected LLM are unchanged…", 0, 0, 0);
    let mut command = uv_command(&uv, &root);
    command
        .args(["venv", "--python", "3.12.12", "--managed-python"])
        .arg(root.join(".venv"));
    run(command, &cancel).await?;
    let python = root.join(".venv/Scripts/python.exe");
    let mut command = uv_command(&uv, &root);
    command
        .args([
            "pip",
            "sync",
            "--require-hashes",
            "--only-binary",
            ":all:",
            "--python",
        ])
        .arg(&python)
        .arg(&requirements)
        .args([
            "--extra-index-url",
            "https://download.pytorch.org/whl/cpu",
            "--index-strategy",
            "unsafe-best-match",
        ]);
    run(command, &cancel).await?;
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
    fs::write(
        root.join("worker.py"),
        include_bytes!("../../resources/conversation_voice/worker.py"),
    )?;
    let mut probe = tokio::process::Command::new(&python);
    probe.args(["-I", "-c", "import torch, pocket_tts, faster_whisper; assert torch.version.cuda is None, 'CPU-only PyTorch is required'"])
        .env("HF_HUB_OFFLINE", "1").env("CUDA_VISIBLE_DEVICES", "");
    run(probe, &cancel).await?;
    crate::config::atomic_json_write(
        &root.join("ready.txt"),
        crate::conversation_voice::REVISION.as_bytes(),
    )
    .map_err(|e| dependency(&e.to_string()))?;
    emit(app, "conversation-voice", "complete", "CPU voice conversation is installed for seven languages. It stays resident alongside the selected LLM. Existing production speech remains manual.", 1, 1, 0);
    Ok(())
}

fn uv_command(uv: &Path, root: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(uv);
    command
        .current_dir(root)
        .env("UV_CACHE_DIR", root.join("cache"))
        .env("UV_PYTHON_INSTALL_DIR", root.join("python"))
        .env("UV_NO_PROGRESS", "1")
        .env("UV_LINK_MODE", "copy")
        .env("PYTHONUTF8", "1");
    command
}

fn dependency(detail: &str) -> SetupError {
    SetupError::Dependency {
        name: "CPU voice conversation".into(),
        details: detail.into(),
    }
}

async fn run(
    mut command: tokio::process::Command,
    cancel: &CancellationToken,
) -> Result<(), SetupError> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    let (mut child, _job) = crate::strata::ProcessJob::spawn(&mut command)?;
    #[cfg(not(windows))]
    let mut child = command.spawn()?;
    let stderr = child.stderr.take().expect("piped setup stderr");
    let tail = tokio::spawn(read_tail(stderr, 4096));
    let status = tokio::select! {
        result = child.wait() => result?,
        _ = cancel.cancelled() => { let _ = child.kill().await; return Err(SetupError::Cancelled); }
    };
    let detail = tail.await.unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(dependency(&format!("{status}: {detail}")))
    }
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
        assert_eq!(assets.len(), 25);
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
