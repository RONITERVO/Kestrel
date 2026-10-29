use super::*;
use kestrel_app_core::{ProducerWorkflowPlan, ProducerWorkflowRun};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

impl AppControl {
    pub(super) fn recover_workflows(&self) -> Result<(), String> {
        for path in records(&self.root.join("workflows"))? {
            let mut run: ProducerWorkflowRun = read(&path)?;
            if run.status == "running" {
                run.status = "interrupted".into();
                run.error = Some("Kestrel stopped. Review the last action and explicitly resume; no work was restarted.".into());
                write(&path, &run)?;
            }
        }
        Ok(())
    }
    pub fn create_workflow(
        &self,
        plan: ProducerWorkflowPlan,
    ) -> Result<ProducerWorkflowRun, String> {
        validate_plan(&plan)?;
        let run = ProducerWorkflowRun {
            id: uuid::Uuid::new_v4().to_string(),
            plan,
            workspace_scope: TASK_WORKSPACE.try_with(Clone::clone).ok(),
            status: "ready".into(),
            next_step: 0,
            attempt: 0,
            results: BTreeMap::new(),
            error: None,
            updated_at: now(),
        };
        self.save_workflow(&run)?;
        Ok(run)
    }
    pub fn get_workflow(&self, id: &str) -> Result<ProducerWorkflowRun, String> {
        read(&self.path("workflows", id)?)
    }
    pub fn list_workflows(&self) -> Result<Vec<ProducerWorkflowRun>, String> {
        let mut runs = records(&self.root.join("workflows"))?
            .iter()
            .map(|path| read::<ProducerWorkflowRun>(path))
            .collect::<Result<Vec<_>, _>>()?;
        runs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        runs.truncate(100);
        Ok(runs)
    }
    fn save_workflow(&self, run: &ProducerWorkflowRun) -> Result<(), String> {
        write(&self.path("workflows", &run.id)?, run)
    }
}

fn validate_plan(plan: &ProducerWorkflowPlan) -> Result<(), String> {
    if plan.title.trim().is_empty()
        || plan.title.len() > 200
        || plan.steps.is_empty()
        || plan.steps.len() > 256
    {
        return Err("A workflow needs a title (up to 200 bytes) and 1–256 steps.".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for step in &plan.steps {
        valid_id(&step.id)?;
        validate_refs(&step.args, &ids)?;
        if !ids.insert(step.id.clone()) {
            return Err(format!("Duplicate workflow step: {}", step.id));
        }
        let (access, _) = policy(&step.command)?;
        if !matches!(access.as_str(), "read" | "write")
            || step.command.contains("workflow")
            || matches!(
                step.command.as_str(),
                "start_computer_task"
                    | "start_producer_task"
                    | "resume_computer_task"
                    | "stop_computer_task"
                    | "start_chat_stream"
                    | "start_local_model"
                    | "stop_local_model"
                    | "release_ai_memory"
            )
        {
            return Err(format!(
                "{} cannot run inside a producer workflow.",
                step.command
            ));
        }
        if let Some(wait) = &step.wait {
            if policy(&wait.command)?.0 != "read" {
                return Err("Workflow observations must be read-only native actions.".into());
            }
            if !(1..=43_200).contains(&wait.timeout_seconds) {
                return Err("Workflow wait must be 1–43,200 seconds.".into());
            }
            if !wait.pointer.is_empty() && !wait.pointer.starts_with('/') {
                return Err("Wait pointer must be a JSON pointer starting with '/'.".into());
            }
            validate_refs(&wait.args, &ids)?;
        }
    }
    Ok(())
}

fn validate_refs(
    value: &Value,
    earlier: &std::collections::BTreeSet<String>,
) -> Result<(), String> {
    if let Some(object) = value.as_object() {
        if let Some(source) = object.get("$from") {
            if object.len() != 2
                || !object.get("pointer").is_some_and(Value::is_string)
                || !source.as_str().is_some_and(|id| earlier.contains(id))
            {
                return Err("Workflow references require exactly $from (an earlier step ID) and pointer (a JSON pointer).".into());
            }
        } else {
            for child in object.values() {
                validate_refs(child, earlier)?;
            }
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            validate_refs(child, earlier)?;
        }
    }
    Ok(())
}

fn resolve(value: &Value, results: &BTreeMap<String, Value>) -> Result<Value, String> {
    match value {
        Value::Object(object) if object.contains_key("$from") => {
            let id = object
                .get("$from")
                .and_then(Value::as_str)
                .ok_or("Invalid workflow source.")?;
            let pointer = object
                .get("pointer")
                .and_then(Value::as_str)
                .ok_or("Invalid workflow pointer.")?;
            results
                .get(id)
                .and_then(|result| result.pointer(pointer))
                .cloned()
                .ok_or_else(|| format!("Step {id} has no result at {pointer}."))
        }
        Value::Object(object) => object
            .iter()
            .map(|(key, value)| Ok((key.clone(), resolve(value, results)?)))
            .collect::<Result<serde_json::Map<_, _>, String>>()
            .map(Value::Object),
        Value::Array(array) => array
            .iter()
            .map(|v| resolve(v, results))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        _ => Ok(value.clone()),
    }
}

pub fn start_workflow(app: AppHandle, id: &str) -> Result<ProducerWorkflowRun, String> {
    let state = app.state::<AppState>();
    let mut jobs = state.control.workflows.lock().map_err(|e| e.to_string())?;
    if !jobs.is_empty() {
        return Err("A producer workflow is already active. Stop or finish it first.".into());
    }
    let mut run = state.control.get_workflow(id)?;
    if run.status == "completed" {
        return Err("This workflow is complete. Create a new plan for new work.".into());
    }
    validate_plan(&run.plan)?;
    // Never repeat an uncertain mutation after a crash. A completed receipt can be consumed safely.
    if run.next_step < run.plan.steps.len() && run.attempt > 0 {
        let request_id = format!("{}-{}-{}", run.id, run.next_step, run.attempt);
        let path = state.control.path("actions", &request_id)?;
        if path.exists() || path.with_extension("json.backup").exists() {
            let receipt = state.control.receipt(&request_id)?;
            if matches!(receipt.status.as_str(), "running" | "interrupted") {
                return Err("The last workflow action has an uncertain outcome. Inspect its receipt and project, then create a reconciled plan; it cannot be blindly replayed.".into());
            }
            if receipt.status == "completed" {
                run.results.insert(
                    run.plan.steps[run.next_step].id.clone(),
                    receipt.result.unwrap_or(Value::Null),
                );
            }
        }
    }
    run.attempt += 1;
    // A task cannot use a pre-existing unrestricted plan to escape its workspace.
    if let Ok(roots) = TASK_WORKSPACE.try_with(Clone::clone) {
        run.workspace_scope = Some(roots);
    }
    run.status = "running".into();
    run.error = None;
    run.updated_at = now();
    state.control.save_workflow(&run)?;
    let cancel = CancellationToken::new();
    jobs.insert(id.into(), cancel.clone());
    let started = run.clone();
    drop(jobs);
    let task_owned = TASK_ACTION.try_with(|owned| *owned).unwrap_or(false);
    tauri::async_runtime::spawn(async move {
        let result = TASK_ACTION
            .scope(task_owned, drive(&app, &mut run, &cancel))
            .await;
        run.status = if cancel.is_cancelled() {
            "cancelled"
        } else if result.is_ok() {
            "completed"
        } else {
            "failed"
        }
        .into();
        run.error = result.err();
        run.updated_at = now();
        let state = app.state::<AppState>();
        if let Err(error) = state.control.save_workflow(&run) {
            eprintln!(
                "Workflow {} could not save its final state: {error}",
                run.id
            );
        }
        if let Ok(mut jobs) = state.control.workflows.lock() {
            jobs.remove(&run.id);
        };
    });
    Ok(started)
}

async fn drive(
    app: &AppHandle,
    run: &mut ProducerWorkflowRun,
    cancel: &CancellationToken,
) -> Result<(), String> {
    while run.next_step < run.plan.steps.len() {
        if cancel.is_cancelled() {
            return Err("Stopped before scheduling the next action. An already-started render can be stopped with its native cancel action.".into());
        }
        let step = &run.plan.steps[run.next_step];
        if !run.results.contains_key(&step.id) {
            let deadline = Instant::now() + Duration::from_secs(43_200);
            while app.state::<AppState>().work_active.load(Ordering::Acquire) {
                if cancel.is_cancelled() {
                    return Err(
                        "Stopped before the next action acquired the app's work slot.".into(),
                    );
                }
                if Instant::now() >= deadline {
                    return Err("The app's work slot remained occupied for twelve hours. Review the active job before resuming.".into());
                }
                tokio::select! { _ = cancel.cancelled() => {}, _ = tokio::time::sleep(Duration::from_millis(200)) => {} }
            }
            let args = resolve(&step.args, &run.results)?;
            validate_scoped_args(app, &args, run.workspace_scope.as_deref())?;
            let receipt = execute(
                app,
                AppActionRequest {
                    request_id: format!("{}-{}-{}", run.id, run.next_step, run.attempt),
                    command: step.command.clone(),
                    args,
                },
            )
            .await?;
            if receipt.status != "completed" {
                return Err(receipt
                    .error
                    .unwrap_or_else(|| format!("Action stopped in {}", receipt.status)));
            }
            run.results
                .insert(step.id.clone(), receipt.result.unwrap_or(Value::Null));
            app.state::<AppState>().control.save_workflow(run)?;
        }
        if let Some(wait) = &step.wait {
            let args = resolve(&wait.args, &run.results)?;
            validate_scoped_args(app, &args, run.workspace_scope.as_deref())?;
            let deadline = Instant::now() + Duration::from_secs(wait.timeout_seconds.into());
            loop {
                if cancel.is_cancelled() {
                    return Err("Stopped waiting; no later step will be scheduled. Inspect or cancel the active render explicitly.".into());
                }
                let observation = dispatch_app_command(app, &wait.command, args.clone()).await?;
                if observation.pointer(&wait.pointer) == Some(&wait.equals) {
                    // A waited step publishes its completed native observation, making its
                    // text, take IDs and paths available to subsequent typed substitutions.
                    run.results.insert(step.id.clone(), observation);
                    break;
                }
                if [
                    observation.get("status"),
                    observation.get("phase"),
                    observation.pointer(&wait.pointer),
                ]
                .into_iter()
                .flatten()
                .any(|value| {
                    value.as_str().is_some_and(|status| {
                        matches!(
                            status,
                            "failed" | "error" | "cancelled" | "interrupted" | "limited"
                        )
                    })
                }) {
                    return Err(format!("{} stopped: {}", wait.command, observation));
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "Timed out waiting for {} {} = {}. Review the native job before resuming.",
                        wait.command, wait.pointer, wait.equals
                    ));
                }
                tokio::select! { _ = cancel.cancelled() => {}, _ = tokio::time::sleep(Duration::from_secs(2)) => {} }
            }
        }
        run.next_step += 1;
        run.updated_at = now();
        app.state::<AppState>().control.save_workflow(run)?;
    }
    Ok(())
}

fn validate_scoped_args(
    app: &AppHandle,
    args: &Value,
    scope: Option<&[String]>,
) -> Result<(), String> {
    if let Some(roots) = scope {
        let state = app.state::<AppState>();
        let configured = state.research_settings.load().map_err(|e| e.to_string())?;
        let settings = kestrel_app_core::ControlSettings {
            agent_workspace_roots: roots.to_vec(),
            ..Default::default()
        };
        crate::agent_app::check_paths(
            args,
            &settings,
            kestrel_app_core::ComputerTaskAccess::Workspace,
            &[
                configured.comfy_root,
                configured.muscriptor_executable_path,
                configured.muscriptor_model_path,
            ],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn local_plan_keeps_workspace_scope_after_restart_and_checks_resolved_paths() {
        let temp = tempfile::tempdir().unwrap();
        let approved = temp.path().join("approved");
        fs::create_dir(&approved).unwrap();
        let outside = temp.path().join("outside.mp4");
        fs::write(&outside, b"media").unwrap();
        let roots = vec![approved.to_string_lossy().into_owned()];
        let control = AppControl::new(temp.path()).unwrap();
        let run = TASK_WORKSPACE
            .scope(roots.clone(), async {
                control
                    .create_workflow(ProducerWorkflowPlan {
                        title: "Workspace plan".into(),
                        steps: vec![kestrel_app_core::ProducerWorkflowStep {
                            id: "one".into(),
                            command: "list_music_projects".into(),
                            args: json!({}),
                            wait: None,
                        }],
                    })
                    .unwrap()
            })
            .await;
        let recovered = AppControl::new(temp.path())
            .unwrap()
            .get_workflow(&run.id)
            .unwrap();
        assert_eq!(recovered.workspace_scope, Some(roots.clone()));
        let results = BTreeMap::from([("source".into(), json!({"path":outside}))]);
        let args = resolve(
            &json!({"paths":[{"$from":"source","pointer":"/path"}]}),
            &results,
        )
        .unwrap();
        let settings = kestrel_app_core::ControlSettings {
            agent_workspace_roots: roots,
            ..Default::default()
        };
        assert!(crate::agent_app::check_paths(
            &args,
            &settings,
            kestrel_app_core::ComputerTaskAccess::Workspace,
            &[]
        )
        .is_err());
    }
    #[test]
    fn references_preserve_typed_values_and_missing_results_fail() {
        let results = BTreeMap::from([(
            "create".into(),
            json!({"id":"project","seed":"18446744073709551615","prediction":{"values":[0.25,0.75],"accepted":true,"confidence":0.91,"missing":null}}),
        )]);
        assert_eq!(
            resolve(&json!({"id":{"$from":"create","pointer":"/id"}}), &results).unwrap(),
            json!({"id":"project"})
        );
        assert!(resolve(&json!({"$from":"create","pointer":"/absent"}), &results).is_err());
        assert_eq!(
            resolve(&json!({"input":{"$from":"create","pointer":"/prediction"},"score":{"$from":"create","pointer":"/prediction/confidence"}}), &results).unwrap(),
            json!({"input":{"values":[0.25,0.75],"accepted":true,"confidence":0.91,"missing":null},"score":0.91})
        );
    }
    #[test]
    fn plans_cannot_delegate_human_consent_or_read_future_results() {
        let plan = |command: &str, args: Value| ProducerWorkflowPlan {
            title: "test".into(),
            steps: vec![kestrel_app_core::ProducerWorkflowStep {
                id: "one".into(),
                command: command.into(),
                args,
                wait: None,
            }],
        };
        assert!(validate_plan(&plan("run_codex_repair", json!({}))).is_err());
        assert!(validate_plan(&plan(
            "get_music_project",
            json!({"id":{"$from":"future","pointer":"/id"}})
        ))
        .is_err());
        assert!(validate_plan(&plan("list_music_projects", json!({}))).is_ok());
    }
}
