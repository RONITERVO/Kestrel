//! Resolve native artifact handles once, shared by composition and any delivery bundle.
use crate::{media_delivery::verified_source, AppState};
use kestrel_app_core::ProductionArtifact;
use serde_json::Value;
use std::{fs, path::PathBuf};

pub(crate) struct Source {
    pub path: PathBuf,
    pub hash: String,
    pub still: bool,
    /// Speech's mistake checker can identify an unwanted trailing hallucination.
    pub outpoint: Option<f64>,
}

pub(crate) fn resolve(state: &AppState, source: &ProductionArtifact) -> Result<Source, String> {
    let (path, expected, still, outpoint) = match source {
        ProductionArtifact::Attachment { id } => {
            let attachment = state.attachments.get(id)?;
            let still = attachment.mime_type.starts_with("image/");
            (attachment.stored_path, Some(attachment.sha256), still, None)
        }
        ProductionArtifact::Research { report_id, file } => {
            let html = state
                .store
                .html_path(report_id)
                .map_err(|e| e.to_string())?;
            let folder = html.parent().ok_or("Research report folder is missing.")?;
            use kestrel_app_core::ResearchArtifactFile as F;
            let name = match file {
                F::Html => "index.html",
                F::Report => "report.json",
                F::Sources => "sources.json",
                F::Provenance => "provenance.json",
            };
            (
                folder.join(name).to_string_lossy().into_owned(),
                None,
                false,
                None,
            )
        }
        ProductionArtifact::ImageTake {
            project_id,
            take_id,
        } => {
            let project = state.images.get(project_id).map_err(|e| e.to_string())?;
            let take = project
                .takes
                .iter()
                .find(|t| &t.id == take_id && t.status == "complete")
                .ok_or("Choose a completed image take.")?;
            (take.path.clone(), Some(take.sha256.clone()), true, None)
        }
        ProductionArtifact::MusicTake {
            project_id,
            take_id,
        } => {
            let project = state.music.get(project_id).map_err(|e| e.to_string())?;
            let take = project
                .takes
                .iter()
                .find(|t| &t.id == take_id && t.status == "complete")
                .ok_or("Choose a completed music take.")?;
            (take.path.clone(), Some(take.sha256.clone()), false, None)
        }
        ProductionArtifact::MovieExport { project_id } => {
            let project = state.studio.get(project_id).map_err(|e| e.to_string())?;
            (project.final_path, None, false, None)
        }
        ProductionArtifact::MovieClip {
            project_id,
            clip_id,
            version_id,
        } => {
            let project = state.studio.get(project_id).map_err(|e| e.to_string())?;
            let clip = project
                .clips
                .iter()
                .find(|c| &c.id == clip_id && c.status == "complete")
                .ok_or("Choose a completed movie clip.")?;
            let path = if let Some(version) = version_id {
                clip.versions
                    .iter()
                    .find(|v| &v.id == version)
                    .ok_or("Movie clip version not found.")?
                    .path
                    .clone()
            } else {
                clip.path.clone()
            };
            (path, None, false, None)
        }
        ProductionArtifact::Speech {
            relative_path,
            text,
        } => {
            let clip = state
                .speech
                .export_clip(relative_path, text)
                .map_err(|e| e.to_string())?;
            (
                clip.path.to_string_lossy().into_owned(),
                None,
                false,
                clip.outpoint,
            )
        }
        ProductionArtifact::Composition { id } => {
            uuid::Uuid::parse_str(id).map_err(|_| "Composition ID must be a UUID.")?;
            let folder = state.store.root().join("producer/compositions").join(id);
            let manifest_path = folder.join("manifest.json");
            if fs::metadata(&manifest_path)
                .map_err(|e| e.to_string())?
                .len()
                > 4 * 1024 * 1024
            {
                return Err("Composition manifest exceeds 4 MiB.".into());
            }
            let manifest: Value =
                serde_json::from_slice(&fs::read(manifest_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            let name = manifest["file"]
                .as_str()
                .filter(|name| matches!(*name, "media.mp4" | "media.m4a"))
                .ok_or("Composition has no verified media artifact.")?;
            let hash = manifest["sha256"]
                .as_str()
                .ok_or("Composition has no verified hash.")?;
            (
                folder.join(name).to_string_lossy().into_owned(),
                Some(hash.into()),
                false,
                None,
            )
        }
    };
    let path = verified_source(state.store.root(), &path, expected.as_deref())?;
    let hash = crate::studio::hash_file(&path)
        .map_err(|e| e.to_string())?
        .1;
    Ok(Source {
        path,
        hash,
        still,
        outpoint,
    })
}
