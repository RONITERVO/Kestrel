//! Explicit, ignored developer acceptance runs. No application benchmark or background work.
use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    brief: String,
    edits: Value,
    review: Vec<String>,
}

fn apply_case(
    project: ImageProject,
    case: &Case,
    comfy: &str,
) -> kestrel_app_core::ImageProjectEdit {
    let mut edit = serde_json::to_value(kestrel_app_core::ImageProjectEdit::from(project)).unwrap();
    for (key, value) in case.edits.as_object().expect("case edits") {
        if key == "settings" {
            for (key, value) in value.as_object().unwrap() {
                edit["settings"][key] = value.clone();
            }
        } else if key != "id" {
            edit[key] = value.clone();
        }
    }
    edit["settings"]["comfyRoot"] = json!(comfy);
    serde_json::from_value(edit).unwrap()
}

#[test]
fn acceptance_cases_are_valid_producer_edits_for_both_models() {
    let cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../../tests/image-acceptance/cases.json"
    ))
    .unwrap();
    // Windows fixture-authoring scripts must use UTF-8 explicitly. Check the code points so
    // a mojibake fixture cannot be mistaken for a model typography failure.
    let unicode = cases
        .iter()
        .find(|case| case.name == "multiline-unicode")
        .unwrap();
    assert_eq!(unicode.edits["elements"][0]["text"], "CAFÉ\nHELSINKI");
    assert_eq!(unicode.edits["elements"][1]["text"], "茶 • 2026");
    let root = tempfile::tempdir().unwrap();
    let studio = ImageStudio::new(root.path()).unwrap();
    for case in cases {
        let project = studio
            .create(CreateImageProjectRequest {
                title: case.name.clone(),
                idea: case.brief.clone(),
                comfy_root: String::new(),
            })
            .unwrap();
        let project = studio
            .save_editable(apply_case(project, &case, "D:/acceptance/ComfyUI"))
            .unwrap_or_else(|error| panic!("{}: {error}", case.name));
        validate_render_ready(&project).unwrap();
        for backend in [ImageBackend::Moss, ImageBackend::Ideogram] {
            let (_, prompt) = backend.prompt(&project).unwrap();
            assert!(!prompt.is_empty());
            let graph = backend.graph(&project.settings, 42621, &prompt, "fixture-check");
            assert_eq!(graph["12"]["inputs"]["width"], project.settings.width);
        }
    }
}

#[tokio::test]
#[ignore = "explicit GPU acceptance: requires KESTREL_IMAGE_ACCEPTANCE_ROOT, KESTREL_LIVE_COMFY_ROOT and installed weights"]
async fn live_image_producer_acceptance() {
    let output = PathBuf::from(
        std::env::var("KESTREL_IMAGE_ACCEPTANCE_ROOT").expect("acceptance output directory"),
    );
    assert!(output.is_absolute());
    let comfy = std::env::var("KESTREL_LIVE_COMFY_ROOT").expect("isolated ComfyUI directory");
    crate::setup::ensure_comfy_launcher(Path::new(&comfy)).unwrap();
    let backend = std::env::var("KESTREL_IMAGE_BACKEND").unwrap_or_else(|_| "moss".into());
    assert!(matches!(backend.as_str(), "moss" | "ideogram"));
    let filter = std::env::var("KESTREL_IMAGE_CASE").unwrap_or_default();
    let mut cases: Vec<Case> = serde_json::from_str(include_str!(
        "../../../../tests/image-acceptance/cases.json"
    ))
    .unwrap();
    if let Ok(proposal) = std::env::var("KESTREL_IMAGE_PROPOSAL") {
        let name = std::env::var("KESTREL_IMAGE_PROPOSAL_NAME")
            .unwrap_or_else(|_| "agent-proposal".into());
        assert!(matches!(
            name.as_str(),
            "agent-proposal" | "agent-develop" | "agent-revise"
        ));
        let brief_path = PathBuf::from(&proposal)
            .with_file_name(format!("{}.brief.txt", name.trim_start_matches("agent-")));
        let brief =
            fs::read_to_string(brief_path).unwrap_or_else(|_| "Live collaborator proposal".into());
        cases = vec![Case { name, brief,
            edits: serde_json::from_slice(&fs::read(proposal).unwrap()).unwrap(),
            review: vec!["Compare the saved user brief, streamed proposal, applied editor state, native prompt and pixels.".into()] }];
    }
    let library = output.join(&backend);
    fs::create_dir_all(&library).unwrap();
    let mut studio = ImageStudio::new(&library).unwrap();
    studio.backend = if backend == "ideogram" {
        ImageBackend::Ideogram
    } else {
        ImageBackend::Moss
    };
    let renderer = MovieStudio::new(&library).unwrap();
    let mut ran = 0;
    for case in cases
        .into_iter()
        .filter(|case| filter.is_empty() || filter.split(',').any(|name| name == case.name))
    {
        let manifest = library.join(format!("{}.json", case.name));
        if manifest.is_file() {
            let prior: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
            let expected_input = json!({"brief":case.brief,"edits":case.edits});
            let compatible = serde_json::from_value::<ImageProject>(prior["project"].clone())
                .ok()
                .is_some_and(|project| {
                    studio
                        .backend
                        .prompt(&project)
                        .ok()
                        .is_some_and(|(_, text)| {
                            project.takes.iter().all(|take| {
                                let prefix = take
                                    .exact_graph
                                    .pointer("/15/inputs/filename_prefix")
                                    .and_then(Value::as_str)
                                    .unwrap_or("");
                                text == take.exact_prompt_text
                                    && studio.backend.graph(
                                        &project.settings,
                                        take.seed,
                                        &text,
                                        prefix,
                                    ) == take.exact_graph
                                    && hash_file(Path::new(&take.path)).ok()
                                        == Some((take.bytes, take.sha256.clone()))
                            })
                        })
                });
            if prior["status"] == "rendered-awaiting-review"
                && prior["caseInput"] == expected_input
                && compatible
            {
                eprintln!("Already rendered: {backend}/{}", case.name);
                ran += 1;
                continue;
            }
        }
        let project = studio
            .create(CreateImageProjectRequest {
                title: format!("Acceptance {backend}: {}", case.name),
                idea: case.brief.clone(),
                comfy_root: comfy.clone(),
            })
            .unwrap();
        let project = studio
            .save_editable(apply_case(project, &case, &comfy))
            .unwrap();
        eprintln!(
            "Rendering {backend}/{}: {}x{}, {}, batch {}",
            case.name,
            project.settings.width,
            project.settings.height,
            project.settings.preset,
            project.settings.batch_size
        );
        let started = Instant::now();
        let (_, ids) = studio.begin_generation(&project.id, None).unwrap();
        let result = studio
            .render(
                &project.id,
                &ids,
                &renderer,
                &CancellationToken::new(),
                None,
            )
            .await;
        let mut record = json!({"case":case.name,"brief":case.brief,"backend":backend,"projectId":project.id,"reviewCriteria":case.review,"qualityVerdict":"pending","elapsedSeconds":started.elapsed().as_secs_f64()});
        record["caseInput"] = json!({"brief":case.brief,"edits":case.edits});
        match result {
            Ok(project) => {
                assert_eq!(project.takes.len(), ids.len());
                for take in &project.takes {
                    assert_eq!(take.status, "complete");
                    assert_eq!(
                        hash_file(Path::new(&take.path)).unwrap(),
                        (take.bytes, take.sha256.clone())
                    );
                    verify_png_dimensions(Path::new(&take.path), take.width, take.height).unwrap();
                }
                record["status"] = json!("rendered-awaiting-review");
                record["project"] = serde_json::to_value(project).unwrap();
            }
            Err(error) => {
                studio.fail_generation(&project.id, &ids, error.to_string(), false, None);
                record["status"] = json!("failed");
                record["error"] = json!(error.to_string());
                write_json_recoverable(&manifest, &record).unwrap();
                panic!("{backend}/{}: {error}", case.name);
            }
        }
        write_json_recoverable(&manifest, &record).unwrap();
        ran += 1;
    }
    assert!(ran > 0, "no acceptance cases selected");
    renderer.release_comfy_memory().await;
}

#[tokio::test]
#[ignore = "explicit live cancellation after the render matrix; requires an isolated idle ComfyUI"]
async fn live_image_cancel_and_restart_preserve_completed_takes() {
    let output = PathBuf::from(std::env::var("KESTREL_IMAGE_ACCEPTANCE_ROOT").unwrap());
    let backend = std::env::var("KESTREL_IMAGE_BACKEND").unwrap_or_else(|_| "moss".into());
    assert!(matches!(backend.as_str(), "moss" | "ideogram"));
    let library = output.join(&backend);
    let prior: Value =
        serde_json::from_slice(&fs::read(library.join("boxes-square.json")).unwrap()).unwrap();
    let id = prior["projectId"].as_str().unwrap();
    let mut studio = ImageStudio::new(&library).unwrap();
    studio.backend = if backend == "ideogram" {
        ImageBackend::Ideogram
    } else {
        ImageBackend::Moss
    };
    let renderer = MovieStudio::new(&library).unwrap();
    let mut project = studio.get(id).unwrap();
    let preserved: Vec<_> = project
        .takes
        .iter()
        .filter(|take| take.status == "complete")
        .cloned()
        .collect();
    assert!(!preserved.is_empty());
    // A new seed prevents a cached graph from completing before cancellation can be exercised.
    project.settings.seed = 93117;
    studio.save_editable(project).unwrap();
    let (_, ids) = studio.begin_generation(id, None).unwrap();
    let cancel = CancellationToken::new();
    let started = Instant::now();
    let observe = async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
        loop {
            let current = studio.get(id).unwrap();
            if current
                .takes
                .iter()
                .any(|take| ids.contains(&take.id) && !take.prompt_id.is_empty())
            {
                cancel.cancel();
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                cancel.cancel();
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    let (result, submitted) =
        tokio::join!(studio.render(id, &ids, &renderer, &cancel, None), observe);
    assert!(
        submitted,
        "cancellation must reach a submitted Comfy prompt"
    );
    assert!(matches!(result, Err(StudioError::Cancelled)));
    studio.fail_generation(
        id,
        &ids,
        "Stopped by the acceptance producer".into(),
        true,
        None,
    );
    let cancelled = studio.get(id).unwrap();
    assert!(cancelled
        .takes
        .iter()
        .filter(|take| ids.contains(&take.id))
        .all(|take| take.status == "cancelled"));
    assert_eq!(cancelled.status, "ready");
    let (_, interrupted_ids) = studio.begin_generation(id, None).unwrap();
    studio.recover_interrupted().unwrap();
    let recovered = studio.get(id).unwrap();
    assert_eq!(recovered.phase, "interrupted");
    assert_eq!(recovered.status, "ready");
    assert!(recovered
        .takes
        .iter()
        .filter(|take| interrupted_ids.contains(&take.id))
        .all(|take| take.status == "interrupted"));
    for original in &preserved {
        let current = recovered
            .takes
            .iter()
            .find(|take| take.id == original.id)
            .unwrap();
        assert_eq!(
            serde_json::to_value(original).unwrap(),
            serde_json::to_value(current).unwrap()
        );
        assert_eq!(
            hash_file(Path::new(&current.path)).unwrap(),
            (original.bytes, original.sha256.clone())
        );
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
    loop {
        let queue: Value = studio
            .http
            .get(format!("{COMFY_BASE}/queue"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if queue["queue_running"].as_array().unwrap().is_empty()
            && queue["queue_pending"].as_array().unwrap().is_empty()
        {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "Comfy queue did not drain after interrupt"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    renderer.release_comfy_memory().await;
    write_json_recoverable(&library.join("cancel-restart.json"), &json!({
        "backend":backend,"status":"passed","cancelReachedSubmittedPrompt":submitted,
        "completedTakesUnchanged":preserved.len(),"elapsedSeconds":started.elapsed().as_secs_f64(),
        "cancelledProject":cancelled,"recoveredProject":recovered
    })).unwrap();
}
