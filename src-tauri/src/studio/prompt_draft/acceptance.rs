//! Uses the very same streaming job, inference lease and request preparation as the image desk.
use super::*;
use std::{fs, path::PathBuf, sync::Mutex};

#[tokio::test]
#[ignore = "explicit live local-model acceptance; reads the selected Kestrel model and saves streamed evidence"]
async fn live_image_collaborator_producer_flow() {
    let output =
        PathBuf::from(std::env::var("KESTREL_IMAGE_ACCEPTANCE_ROOT").expect("acceptance root"));
    let config = PathBuf::from(
        std::env::var("KESTREL_IMAGE_CONFIG_ROOT").expect("existing Kestrel settings root"),
    );
    let backend = std::env::var("KESTREL_IMAGE_BACKEND").unwrap_or_else(|_| "moss".into());
    assert!(matches!(backend.as_str(), "moss" | "ideogram"));
    let output = output.join(format!("agent-{backend}-producer"));
    fs::create_dir_all(&output).unwrap();
    // Only the isolated evidence folder receives a prompt-pack copy.
    prompt_catalog::initialize(&output).unwrap();
    if let Ok(path) = std::env::var("KESTREL_IMAGE_PROMPT_PACK") {
        prompt_catalog::save_text(&fs::read_to_string(path).unwrap()).unwrap();
    }
    let settings: ControlSettings =
        serde_json::from_slice(&fs::read(config.join("control-settings.json")).unwrap()).unwrap();
    let catalog: Value =
        serde_json::from_slice(&fs::read(config.join("model-catalog.json")).unwrap()).unwrap();
    let models: Vec<ModelInfo> = serde_json::from_value(catalog["models"].clone()).unwrap();
    let model_id = settings
        .selected_model_id
        .clone()
        .expect("select a local assistant in Kestrel");
    let model = models
        .iter()
        .find(|model| model.id == model_id)
        .expect("selected catalog model");
    eprintln!("Image collaborator: {} ({backend} prompts)", model.name);
    let runtime = Arc::new(RuntimeManager::new());
    let renderer = crate::studio::MovieStudio::new(&output).unwrap();
    renderer.release_comfy_memory().await;
    let brief = "Generate an editorial poster for a local design exhibition. Square canvas. Exact title: ‘NIGHT / FORM’ at the top. One cobalt blue ceramic vase in the left half and one orange sphere in the right half, fully separated. Cream background, precise geometric illustration, generous margins. Put the exact footer ‘HELSINKI 2026’ at the bottom. Give each object and each text area a placement box. Keep all lettering exactly as supplied.";
    // A freshly created image project already has the editable default style and background.
    // Match the UI's imageCompositionPreview source, rather than sending an empty source block.
    let style = kestrel_app_core::ImageStyle::default();
    let mut existing = serde_json::to_string_pretty(&json!({
        "high_level_description": brief,
        "style_description": {"aesthetics":style.aesthetics,"lighting":style.lighting,"photo":style.photo,"medium":style.medium,"color_palette":style.color_palette},
        "compositional_deconstruction": {"background":"A fully described environment extending naturally to every edge of the frame.","elements":[]}
    })).unwrap();
    for (stage, mode, story) in [
        ("develop", PromptDraftMode::Develop, brief),
        // The Image desk's Develop design action always sends Develop, including revisions.
        ("revise", PromptDraftMode::Develop, "Keep the poster concept and exact title NIGHT / FORM and footer HELSINKI 2026. Move the blue vase to the right half and the orange sphere to the left half, keeping them separate. Preserve the cream background and geometric illustration. Return a complete revised design."),
    ] {
        let observer = Arc::new(Mutex::new(Vec::new()));
        let request = PromptDraftRequest {
            request_id: uuid::Uuid::new_v4().to_string(), model_id: model_id.clone(),
            target: PromptDraftTarget::ImageComposition, mode, story_text: story.into(),
            existing_text: existing.clone(), asset_name: String::new(), asset_kind: String::new(),
            thinking_level: None, context_window: None, max_output_tokens: None,
        };
        let result = PromptDraftJob {
            events: PromptDraftEvents::headless(observer.clone()), runtime: runtime.clone(),
            models: models.clone(), settings: settings.clone(), request, cancel: CancellationToken::new(),
        }.run().await;
        let events = observer.lock().unwrap().clone();
        fs::write(output.join(format!("{stage}.events.json")), serde_json::to_vec_pretty(&events).unwrap()).unwrap();
        fs::write(output.join(format!("{stage}.brief.txt")), story).unwrap();
        if let Err(error) = result {
            runtime.stop_managed().await.unwrap();
            panic!("{stage}: {error}");
        }
        assert!(events.iter().any(|event| event.kind == "complete"), "truncated or cancelled output is not a successful proposal");
        existing = events.iter().filter(|event| event.kind == "token").filter_map(|event| event.content.as_deref()).collect();
        fs::write(output.join(format!("{stage}.proposal.txt")), &existing).unwrap();
        assert!(!existing.trim().is_empty());
        let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let applied = std::process::Command::new("node")
            .current_dir(repository)
            .arg("node_modules/vite-node/dist/cli.mjs")
            .arg("scripts/image-acceptance-proposal.ts")
            .arg(output.join(format!("{stage}.proposal.txt")))
            .arg(output.join(format!("{stage}.edits.json")))
            .arg(output.join(format!("{stage}.preview.json")))
            .status().unwrap();
        if !applied.success() { runtime.stop_managed().await.unwrap(); }
        assert!(applied.success(), "the actual UI parser must accept the proposal");
        existing = fs::read_to_string(output.join(format!("{stage}.preview.json"))).unwrap();
        eprintln!("Preserved and applied {stage}: {} bytes of UI preview for the next request", existing.len());
    }
    runtime.stop_managed().await.unwrap();
}
