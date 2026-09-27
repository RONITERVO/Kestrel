//! Stills made by the retired H3 stable-frame pass. New pictures for a production come from
//! Image Studio (see `import_image_take_as_movie_reference`); the stills already made stay listed
//! and usable as references because they are the producer's work.
pub use kestrel_app_core::image_assets::MovieImageAssetGeneration;

use super::{write_json_atomic, MovieStudio, StudioError};
use chrono::Utc;
use std::{fs, path::PathBuf};

const MAX_GENERATION_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LISTED_GENERATIONS: usize = 50;

impl MovieStudio {
    pub fn list_image_asset_generations(
        &self,
    ) -> Result<Vec<MovieImageAssetGeneration>, StudioError> {
        let root = self.image_generation_root();
        fs::create_dir_all(&root)?;
        let mut generations: Vec<MovieImageAssetGeneration> = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let path = entry.path().join("generation.json");
            if !path.is_file() {
                continue;
            }
            let size = path.metadata()?.len();
            if size > MAX_GENERATION_MANIFEST_BYTES {
                return Err(StudioError::Invalid(format!(
                    "generated-image receipt is unexpectedly large: {}",
                    path.display()
                )));
            }
            let mut generation: MovieImageAssetGeneration =
                serde_json::from_slice(&fs::read(&path)?)?;
            generation.candidates.retain_mut(|candidate| {
                let Ok(asset) = self.resolve_reference_asset(&candidate.asset.id) else {
                    return false;
                };
                candidate.asset = asset;
                true
            });
            generations.push(generation);
        }
        generations.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        generations.truncate(MAX_LISTED_GENERATIONS);
        Ok(generations)
    }

    /// A receipt left "running" by a closed app is marked interrupted, so it never claims a pass
    /// that did not finish.
    pub(super) fn recover_image_asset_generations(&self) -> Result<(), StudioError> {
        let root = self.image_generation_root();
        fs::create_dir_all(&root)?;
        for entry in fs::read_dir(root)?.filter_map(Result::ok) {
            let path = entry.path().join("generation.json");
            let Ok(bytes) = fs::read(&path) else { continue };
            if bytes.len() as u64 > MAX_GENERATION_MANIFEST_BYTES {
                continue;
            }
            let Ok(mut generation) = serde_json::from_slice::<MovieImageAssetGeneration>(&bytes)
            else {
                continue;
            };
            if generation.status == "running" {
                generation.status = "interrupted".into();
                generation.stage = "interrupted".into();
                generation.detail = "Kestrel closed during the local image pass. No candidate was attached automatically; make new pictures in Image Studio.".into();
                generation.error = "The prior image pass was interrupted before Kestrel recorded a complete candidate set.".into();
                generation.updated_at = Utc::now().to_rfc3339();
                write_json_atomic(&path, &generation)?;
            }
        }
        Ok(())
    }

    fn image_generation_root(&self) -> PathBuf {
        self.root.join("_references").join("generations")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn legacy_generation_manifest_does_not_claim_current_preview_provenance() {
        let generation: MovieImageAssetGeneration = serde_json::from_value(json!({
            "id": "legacy-generation",
            "status": "complete",
            "stage": "ready",
            "detail": "Legacy receipt",
            "prompt": "A compass",
            "renderedPrompt": "A compass",
            "width": 768,
            "height": 768,
            "steps": 20,
            "seed": 7,
            "stabilize": true,
            "workflow": "MiniMax H3 pseudo-image stable-frame workflow",
            "workflowSource": "https://huggingface.co/reverentelusarca/minimax-h3-comfyui-workflows",
            "workflowRevision": "1abf4a61eddffd08fa407e013ea7b7e62fbbbbf4",
            "requestedLength": 8,
            "resolvedFrameCount": 22,
            "candidateStart": 8,
            "candidateCount": 6,
            "comfyPromptId": "legacy-prompt",
            "createdAt": "2026-08-01T00:00:00Z",
            "updatedAt": "2026-08-01T00:00:00Z"
        }))
        .unwrap();

        assert_eq!(
            generation.preview_node_revision,
            "unavailable (legacy generation)"
        );
        assert_eq!(
            generation.preview_decoder_revision,
            "unavailable (legacy generation)"
        );
        assert_eq!(
            generation.preview_decoder_sha256,
            "unavailable (legacy generation)"
        );
    }
}
