//! Optional repository-scoped Codex maintainer.
//!
//! This module is never called by research or runtime code. It uses fixed executable/argument
//! arrays (no shell strings), workspace-write sandboxing, ephemeral sessions, and the existing
//! Codex CLI login. Native diagnostics work when Codex or the network is unavailable.

use crate::models::{DeveloperRepairReport, DeveloperStatus};
use chrono::Utc;
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use tokio::process::Command;

pub struct DeveloperAssistant {
    running: AtomicBool,
    last_report: std::sync::Mutex<Option<String>>,
    status_cache: std::sync::Mutex<Option<(String, Instant, DeveloperStatus)>>,
    report_root: PathBuf,
}

struct RunGuard<'a>(&'a AtomicBool);

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl DeveloperAssistant {
    pub fn new(library_root: &Path) -> Self {
        Self {
            running: AtomicBool::new(false),
            last_report: std::sync::Mutex::new(None),
            status_cache: std::sync::Mutex::new(None),
            report_root: library_root.join("maintenance"),
        }
    }

    pub async fn status(&self, project_root: &str) -> DeveloperStatus {
        if let Some(mut cached) = self
            .status_cache
            .lock()
            .ok()
            .and_then(|cache| cache.clone())
            .filter(|(root, checked, _)| {
                root == project_root && checked.elapsed() < Duration::from_secs(15)
            })
            .map(|(_, _, status)| status)
        {
            cached.running = self.running.load(Ordering::Acquire);
            cached.last_report = self.last_report.lock().ok().and_then(|value| value.clone());
            return cached;
        }
        let root = PathBuf::from(project_root);
        let version = command_text("codex", &["--version"], None, Duration::from_secs(5)).await;
        let auth = command_text("codex", &["login", "status"], None, Duration::from_secs(10)).await;
        let git_repository = root.join(".git").exists();
        let worktree_clean = git_repository
            && command_text(
                "git",
                &["status", "--porcelain"],
                Some(&root),
                Duration::from_secs(10),
            )
            .await
            .is_some_and(|output| output.trim().is_empty());
        let status = DeveloperStatus {
            codex_available: version.is_some(),
            codex_authenticated: auth
                .as_deref()
                .is_some_and(|value| value.to_lowercase().contains("logged in")),
            codex_version: version.map(|value| value.trim().into()),
            project_root: project_root.into(),
            git_repository,
            worktree_clean,
            running: self.running.load(Ordering::Acquire),
            last_report: self.last_report.lock().ok().and_then(|value| value.clone()),
        };
        if let Ok(mut cache) = self.status_cache.lock() {
            *cache = Some((project_root.into(), Instant::now(), status.clone()));
        }
        status
    }

    pub fn passive_status(&self, project_root: &str) -> DeveloperStatus {
        if let Some(mut status) = self.status_cache.lock().ok().and_then(|cache| {
            cache
                .as_ref()
                .filter(|(root, _, _)| root == project_root)
                .map(|(_, _, status)| status.clone())
        }) {
            status.running = self.running.load(Ordering::Acquire);
            status.last_report = self.last_report.lock().ok().and_then(|value| value.clone());
            return status;
        }
        DeveloperStatus {
            codex_available: false,
            codex_authenticated: false,
            codex_version: None,
            project_root: project_root.into(),
            git_repository: Path::new(project_root).join(".git").exists(),
            worktree_clean: false,
            running: self.running.load(Ordering::Acquire),
            last_report: self.last_report.lock().ok().and_then(|value| value.clone()),
        }
    }

    pub async fn diagnose(
        &self,
        project_root: &str,
        app: Option<&AppHandle>,
    ) -> Result<String, String> {
        let root = validated_repo(project_root)?;
        emit(
            app,
            "diagnostics",
            "Running deterministic backend and interface checks",
        );
        run_diagnostics(&root).await
    }

    pub async fn repair(
        &self,
        project_root: &str,
        issue: &str,
        app: Option<&AppHandle>,
    ) -> Result<DeveloperRepairReport, String> {
        self.running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "A Codex maintenance run is already active.".to_string())?;
        let _guard = RunGuard(&self.running);
        let root = validated_repo(project_root)?;
        let status = self.status(project_root).await;
        if !status.codex_available {
            return Err(
                "Codex CLI is not installed. Native diagnostics remain available offline.".into(),
            );
        }
        if !status.codex_authenticated {
            return Err(
                "Codex CLI is not signed in. Run `codex login`; offline research is unaffected."
                    .into(),
            );
        }
        let before = run_diagnostics(&root).await?;
        if issue.trim().is_empty() && diagnostics_passed(&before) {
            return Err("All native checks passed. Describe the observed backend issue before requesting a repair.".into());
        }
        emit(
            app,
            "codex",
            "Codex is diagnosing and repairing only this Git workspace",
        );
        let prompt = format!(
            "You are the Kestrel backend maintainer. Work only in this repository. Read AGENTS.md first. Diagnose and implement the smallest robust fix for the issue below. Preserve offline research, the single-runtime/inference-lease boundary, citation validation, immutable report storage, and all user work. Do not alter unrelated files, do not use the network from shell commands, do not commit, and run the required checks before finishing.\n\nUSER ISSUE:\n{}\n\nNATIVE DIAGNOSTICS BEFORE REPAIR:\n{}",
            if issue.trim().is_empty() { "The deterministic checks below are failing." } else { issue.trim() },
            truncate(&before, 24_000)
        );
        let mut command = Command::new("codex");
        command
            .args([
                "exec",
                "--ephemeral",
                "--ignore-user-config",
                "--sandbox",
                "workspace-write",
                "--color",
                "never",
                "--json",
                "-C",
            ])
            .arg(&root)
            .arg("-")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command
            .spawn()
            .map_err(|error| format!("Could not start Codex: {error}"))?;
        use tokio::io::AsyncWriteExt;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(|error| format!("Could not give diagnostics to Codex: {error}"))?;
        }
        let output = tokio::time::timeout(Duration::from_secs(3_600), child.wait_with_output())
            .await
            .map_err(|_| "Codex maintenance exceeded one hour and was stopped.".to_string())?
            .map_err(|error| format!("Codex maintenance failed: {error}"))?;
        let codex_log = format!(
            "STDOUT\n{}\n\nSTDERR\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        emit(
            app,
            "verify",
            "Re-running native checks after the proposed repair",
        );
        let after = run_diagnostics(&root).await?;
        let success = output.status.success() && diagnostics_passed(&after);
        let report_path =
            self.persist_report(project_root, issue, &before, &codex_log, &after, success)?;
        if let Ok(mut last) = self.last_report.lock() {
            *last = Some(report_path.clone());
        }
        if let Ok(mut cache) = self.status_cache.lock() {
            *cache = None;
        }
        emit(
            app,
            "complete",
            if success {
                "Repair verified"
            } else {
                "Repair needs developer review"
            },
        );
        Ok(DeveloperRepairReport {
            success,
            summary: if success {
                "Codex changed the workspace and every native check now passes. Review the Git diff before committing.".into()
            } else {
                "Codex finished, but one or more checks still fail. No commit was created; inspect the report and Git diff.".into()
            },
            diagnostics_before: before,
            diagnostics_after: after,
            report_path,
        })
    }

    fn persist_report(
        &self,
        project_root: &str,
        issue: &str,
        before: &str,
        codex: &str,
        after: &str,
        success: bool,
    ) -> Result<String, String> {
        std::fs::create_dir_all(&self.report_root).map_err(|error| error.to_string())?;
        let stamp = Utc::now().format("%Y%m%d-%H%M%S");
        let path = self.report_root.join(format!("maintenance-{stamp}.json"));
        let value = json!({
            "version": 1,
            "createdAt": Utc::now().to_rfc3339(),
            "projectRoot": project_root,
            "issue": issue,
            "success": success,
            "diagnosticsBefore": before,
            "codexEvents": truncate(codex, 200_000),
            "diagnosticsAfter": after,
            "note": "Codex never commits. The Git diff remains the review and rollback boundary."
        });
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Ok(path.to_string_lossy().into_owned())
    }
}

fn validated_repo(project_root: &str) -> Result<PathBuf, String> {
    let root = PathBuf::from(project_root)
        .canonicalize()
        .map_err(|error| format!("Project root is unavailable: {error}"))?;
    if !root.join(".git").exists() || !root.join("src-tauri").join("Cargo.toml").is_file() {
        return Err(
            "Project root must be the Kestrel Git repository containing src-tauri/Cargo.toml."
                .into(),
        );
    }
    Ok(root)
}

async fn run_diagnostics(root: &Path) -> Result<String, String> {
    let checks: [(&str, &str, &[&str], &Path); 6] = [
        ("git diff hygiene", "git", &["diff", "--check"], root),
        (
            "Rust tests",
            "cargo",
            &[
                "test",
                "--all-targets",
                "--manifest-path",
                "src-tauri\\Cargo.toml",
            ],
            root,
        ),
        (
            "Rust lints",
            "cargo",
            &[
                "clippy",
                "--all-targets",
                "--manifest-path",
                "src-tauri\\Cargo.toml",
                "--",
                "-D",
                "warnings",
            ],
            root,
        ),
        ("TypeScript", "npm.cmd", &["run", "check"], root),
        ("Interface tests", "npm.cmd", &["test", "--", "--run"], root),
        ("Production web build", "npm.cmd", &["run", "build"], root),
    ];
    let mut report = String::new();
    for (name, program, args, directory) in checks {
        let output = command_output(program, args, Some(directory), Duration::from_secs(600)).await;
        match output {
            Some((success, text)) => {
                report.push_str(&format!(
                    "\n## {name}: {}\n{}\n",
                    if success { "PASS" } else { "FAIL" },
                    truncate(&text, 32_000)
                ));
            }
            None => report.push_str(&format!(
                "\n## {name}: FAIL\nCommand could not be started or timed out.\n"
            )),
        }
    }
    Ok(report)
}

fn diagnostics_passed(report: &str) -> bool {
    !report.contains(": FAIL") && report.matches(": PASS").count() == 6
}

async fn command_text(
    program: &str,
    args: &[&str],
    directory: Option<&Path>,
    timeout: Duration,
) -> Option<String> {
    let (success, output) = command_output(program, args, directory, timeout).await?;
    success.then_some(output)
}

async fn command_output(
    program: &str,
    args: &[&str],
    directory: Option<&Path>,
    timeout: Duration,
) -> Option<(bool, String)> {
    let mut command = Command::new(program);
    command.args(args).kill_on_drop(true);
    if let Some(directory) = directory {
        command.current_dir(directory);
    }
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .ok()?
        .ok()?;
    Some((
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    ))
}

fn emit(app: Option<&AppHandle>, stage: &str, detail: &str) {
    if let Some(app) = app {
        let _ = app.emit(
            "developer-progress",
            json!({"stage":stage,"detail":detail,"at":Utc::now().to_rfc3339()}),
        );
    }
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_require_every_fixed_check() {
        let passing = (0..6)
            .map(|index| format!("## c{index}: PASS\n"))
            .collect::<String>();
        assert!(diagnostics_passed(&passing));
        assert!(!diagnostics_passed(&(passing + "## later: FAIL\n")));
    }
}
