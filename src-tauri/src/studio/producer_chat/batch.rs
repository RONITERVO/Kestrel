//! Producer-started, sequential drafting. Native code owns count and checkpoints; the model
//! writes exactly one distinct scene per request. It cannot schedule work or touch media.
use super::*;
use crate::models::{MovieSceneDraftBatch, MovieStudioConversationMode};

impl MovieStudioChatJob {
    pub(super) async fn run_batch(self) -> Result<(), String> {
        let mut batch = self
            .studio
            .begin_scene_batch(&self.request, self.app.as_ref())
            .await
            .map_err(|error| error.to_string())?;
        let initial_completed = batch.completed_scene_ids.len();
        loop {
            if self.cancel.is_cancelled() {
                emit(
                    self.app.as_ref(),
                    &self.request,
                    "",
                    "cancelled",
                    None,
                    None,
                    None,
                    None,
                    Vec::new(),
                );
                return Ok(());
            }
            let completed = batch.completed_scene_ids.len();
            if completed >= batch.scene_count as usize {
                emit(
                    self.app.as_ref(),
                    &self.request,
                    "",
                    "complete",
                    None,
                    None,
                    None,
                    None,
                    Vec::new(),
                );
                return Ok(());
            }
            let mut request = self.request.clone();
            request.scene_batch = None;
            request.story_revision_id = Some(batch.story_revision_id.clone());
            request.selected_scene_ids = batch.context_scene_ids.clone();
            request.instruction = format!(
                "Write distinct scene {} of {} using the saved producer direction for batch {}.",
                completed + 1,
                batch.scene_count,
                batch.id
            );
            // Periodically archive chat to keep durable transcripts bounded. This does not lose
            // any scene or the batch's accepted story, explicit context, or continuity context.
            let workspace = self
                .studio
                .get_producer_workspace(&request.project_id)
                .map_err(|error| error.to_string())?;
            request.conversation_id = workspace.active_scene_conversation_id;
            request.mode = if completed.is_multiple_of(16) || completed == initial_completed {
                MovieStudioConversationMode::Fresh
            } else {
                MovieStudioConversationMode::Continue
            };
            let job = MovieStudioChatJob {
                app: self.app.clone(),
                studio: self.studio.clone(),
                runtime: self.runtime.clone(),
                models: self.models.clone(),
                settings: self.settings.clone(),
                request,
                cancel: self.cancel.clone(),
            };
            job.run_turn(Some(&batch)).await?;
            if self.cancel.is_cancelled() {
                return Ok(());
            }
            let saved = self
                .studio
                .get_producer_workspace(&self.request.project_id)
                .map_err(|error| error.to_string())?;
            batch = saved
                .scene_draft_batch
                .filter(|saved| saved.id == batch.id)
                .ok_or_else(|| {
                    "The scene queue changed while drafting. Saved scenes remain available."
                        .to_string()
                })?;
            if batch.completed_scene_ids.len() != completed + 1 {
                return Err("The scene was not checkpointed. The queue stopped without skipping ahead; resume to retry.".into());
            }
        }
    }
}

pub(super) fn build_batch_messages(
    prepared: &PreparedStudioTurn,
    batch: &MovieSceneDraftBatch,
) -> Result<Vec<Value>, String> {
    if prepared.scene_revision != batch.expected_scene_revision
        || prepared.story_revision_id != batch.story_revision_id
    {
        return Err("Story or scenes changed after the queue checkpoint. Start a new batch from the current cards.".into());
    }
    let context = batch
        .context_scene_ids
        .iter()
        .collect::<std::collections::HashSet<_>>();
    let recent = batch
        .completed_scene_ids
        .iter()
        .rev()
        .take(4)
        .collect::<std::collections::HashSet<_>>();
    let content = json!({
        "acceptedStoryMarkdown": prepared.story_markdown,
        "producerDirection": batch.instruction,
        "sceneNumber": batch.completed_scene_ids.len() + 1,
        "totalScenes": batch.scene_count,
        "durationSeconds": batch.scene_seconds,
        "producerSelectedContext": prepared.scenes.iter().filter(|scene| context.contains(&scene.id)).map(scene_text_json).collect::<Vec<_>>(),
        "recentScenesWrittenByThisQueue": prepared.scenes.iter().filter(|scene| recent.contains(&scene.id)).map(scene_text_json).collect::<Vec<_>>(),
        "instruction": "Write exactly one NEW, distinct scene at this position in the requested full production. Distribute the accepted story across totalScenes; advance its action rather than retelling the opening or repeating recent prompts. Use the requested duration exactly. Preserve continuity with recent queue scenes. Return one add operation at end, sceneId and anchorSceneId null. Give a complete H3 prompt and a visible final-frame state. The producer will choose references and render later."
    });
    let messages = vec![
        json!({"role":"system", "content":SCENE_SYSTEM}),
        json!({"role":"user", "content":serde_json::to_string(&content).map_err(|error| error.to_string())?}),
    ];
    if serde_json::to_vec(&messages)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_CONTEXT_BYTES
    {
        return Err("The accepted story and selected context are too large for one scene request. Select fewer context cards before starting a new batch.".into());
    }
    Ok(messages)
}
