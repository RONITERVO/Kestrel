//! Explicit, opt-in acceptance through the same Setup operations used by the desktop.
use super::*;

#[tokio::test]
#[ignore = "explicit producer-authorized local setup; KESTREL_SETUP_PLAN selects paths and components"]
async fn live_configure_existing_installations() {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Plan {
        library: PathBuf,
        locations: SetupLocations,
        source_folders: Vec<PathBuf>,
        components: Vec<String>,
        evidence: PathBuf,
    }
    let plan: Plan = serde_json::from_slice(
        &fs::read(std::env::var("KESTREL_SETUP_PLAN").expect("explicit setup plan required"))
            .unwrap(),
    )
    .unwrap();
    let store = crate::config::SettingsStore::new(&plan.library);
    let control_store = crate::config::ControlSettingsStore::new(&plan.library);
    let mut research = store.load().unwrap();
    let mut control = control_store.load().unwrap();
    let original = fs::read(plan.library.join("research-settings.json")).unwrap();
    let original_control = fs::read(plan.library.join("control-settings.json")).unwrap();
    apply_locations(&mut research, &mut control, plan.locations).unwrap();
    for root in &plan.source_folders {
        research
            .existing_model_paths
            .extend(scan_existing_model_folder(root).unwrap());
    }
    for component in &plan.components {
        println!("Preparing {component} in {}", research.comfy_root);
        let request: SetupInstallRequest = serde_json::from_value(serde_json::json!({
            "component": component,
            "installRoot": research.install_root,
            "existingModelPaths": research.existing_model_paths,
        }))
        .unwrap();
        install_component(None, &mut research, &request, CancellationToken::new())
            .await
            .unwrap();
    }
    let status = snapshot(&research, &control, None, None);
    for component in &status.components {
        println!(
            "{}: {} ({})",
            component.id, component.status, component.path
        );
        if component.id != "assistant" {
            assert_eq!(
                component.status, "ready",
                "{}: {}",
                component.id, component.detail
            );
        }
    }
    // Do not overwrite another window's intervening edits. Keep explicit recovery copies.
    assert_eq!(
        original,
        fs::read(plan.library.join("research-settings.json")).unwrap()
    );
    assert_eq!(
        original_control,
        fs::read(plan.library.join("control-settings.json")).unwrap()
    );
    fs::create_dir_all(&plan.evidence).unwrap();
    fs::write(
        plan.evidence.join("research-settings.before.json"),
        original,
    )
    .unwrap();
    fs::write(
        plan.evidence.join("control-settings.before.json"),
        original_control,
    )
    .unwrap();
    store.save(&research).unwrap();
    control_store.save(&control).unwrap();
    fs::write(
        plan.evidence.join("setup-snapshot.json"),
        serde_json::to_vec_pretty(&status).unwrap(),
    )
    .unwrap();
    assert_eq!(store.load().unwrap().comfy_root, research.comfy_root);
}
