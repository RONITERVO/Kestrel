//! Native scene queue checkpoints. Never schedules inference or resumes on load.
use super::*;
use crate::models::MovieSceneDraftBatch;

impl MovieStudio {
    pub(in crate::studio) async fn begin_scene_batch(
        &self,
        request: &MovieStudioChatRequest,
        app: Option<&AppHandle>,
    ) -> Result<MovieSceneDraftBatch, StudioError> {
        validate_id(&request.project_id)?;
        let input = request
            .scene_batch
            .as_ref()
            .ok_or_else(|| StudioError::Invalid("scene batch request is missing".into()))?;
        if request.kind != MovieStudioConversationKind::Scenes
            || !(1..=super::super::MAX_MOVIE_SCENES).contains(&input.scene_count)
        {
            return Err(StudioError::Invalid(
                "choose between 1 and 4096 distinct scenes in the scene room".into(),
            ));
        }
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;
        let mut workspace = self.get_producer_workspace(&request.project_id)?;
        let accepted = workspace
            .accepted_story_revision_id
            .as_deref()
            .ok_or_else(|| StudioError::Invalid("accept a story before drafting scenes".into()))?;
        if request.story_revision_id.as_deref() != Some(accepted) {
            return Err(StudioError::Invalid(
                "the accepted story changed; reload before drafting scenes".into(),
            ));
        }
        if input.resume {
            let batch = workspace.scene_draft_batch.as_ref().ok_or_else(|| {
                StudioError::Invalid("there is no saved scene batch to resume".into())
            })?;
            validate_checkpoint(&workspace, batch)?;
            if input.scene_count != batch.scene_count {
                return Err(StudioError::Invalid(
                    "resume uses the saved scene count; start a new batch to change it".into(),
                ));
            }
            return Ok(batch.clone());
        }
        if request.instruction.trim().is_empty()
            || request.instruction.chars().count() > MAX_STORY_INSTRUCTION_CHARS
        {
            return Err(StudioError::Invalid(
                "scene direction must contain 1–16000 characters".into(),
            ));
        }
        let total = workspace
            .scenes
            .len()
            .saturating_add(input.scene_count as usize);
        if total > super::super::MAX_MOVIE_SCENES as usize {
            return Err(StudioError::Invalid("this batch would exceed the project's 4096 scene limit; use another production for additional scenes".into()));
        }
        if request
            .selected_scene_ids
            .iter()
            .any(|id| !workspace.scenes.iter().any(|scene| &scene.id == id))
        {
            return Err(StudioError::Invalid(
                "a selected context scene no longer exists; reload before drafting".into(),
            ));
        }
        if let Some(previous) = &workspace.scene_draft_batch {
            // Preserve the replaced queue, including its original direction and saved progress.
            let folder = self
                .project_dir(&request.project_id)
                .join("producer/draft-batches");
            fs::create_dir_all(&folder)?;
            super::super::write_json_atomic(
                &folder.join(format!("{}-{}.json", previous.id, uuid::Uuid::new_v4())),
                previous,
            )?;
        }
        let batch = MovieSceneDraftBatch {
            id: uuid::Uuid::new_v4().to_string(),
            story_revision_id: accepted.into(),
            instruction: request.instruction.trim().into(),
            scene_count: input.scene_count,
            scene_seconds: project.settings.clip_seconds,
            completed_scene_ids: Vec::new(),
            context_scene_ids: request.selected_scene_ids.clone(),
            expected_scene_revision: workspace.scene_revision,
            created_at: Utc::now().to_rfc3339(),
        };
        // Explicit batch size expands legacy project capacity without changing media settings.
        project.settings.max_clips = project.settings.max_clips.max(total as u32);
        self.persist_emit(&mut project, app)?;
        workspace.scene_draft_batch = Some(batch.clone());
        self.save_producer_workspace(&workspace)?;
        self.emit_producer_workspace(&workspace, app);
        Ok(batch)
    }

    pub(super) fn persist_batch_scene(
        &self,
        workspace: &MovieProducerWorkspace,
    ) -> Result<(), StudioError> {
        let batch = workspace
            .scene_draft_batch
            .as_ref()
            .expect("validated batch");
        let scene = workspace.scenes.last().expect("validated appended scene");
        let receipt = serde_json::json!({
            "schemaVersion": 1, "batchId": batch.id,
            "sceneRevision": workspace.scene_revision,
            "previousSceneRevision": workspace.scene_revision - 1,
            "operation": "append", "scene": scene,
        });
        // An append receipt avoids storing thousands of complete historical copies of a long film.
        let path = self
            .project_dir(&workspace.project_id)
            .join("producer/scene-history")
            .join(format!("batch-{}-{}.json", batch.id, uuid::Uuid::new_v4()));
        fs::create_dir_all(path.parent().expect("scene history folder"))?;
        super::super::write_json_atomic(&path, &receipt)
    }
}

fn validate_checkpoint(
    workspace: &MovieProducerWorkspace,
    batch: &MovieSceneDraftBatch,
) -> Result<(), StudioError> {
    if workspace.accepted_story_revision_id.as_deref() != Some(&batch.story_revision_id)
        || workspace.scene_revision != batch.expected_scene_revision
        || batch
            .completed_scene_ids
            .iter()
            .any(|id| !workspace.scenes.iter().any(|scene| &scene.id == id))
    {
        return Err(StudioError::Invalid("the story or scene cards changed after this batch checkpoint; start a new batch using the current cards. All completed scenes are preserved".into()));
    }
    if batch.completed_scene_ids.len() >= batch.scene_count as usize {
        return Err(StudioError::Invalid(
            "this scene batch is already complete".into(),
        ));
    }
    Ok(())
}

pub(super) fn validate_batch_turn(
    workspace: &MovieProducerWorkspace,
    batch_id: &str,
    operations: &[SceneTextOperation],
) -> Result<(), StudioError> {
    let batch = workspace
        .scene_draft_batch
        .as_ref()
        .filter(|batch| batch.id == batch_id)
        .ok_or_else(|| {
            StudioError::Invalid(
                "the scene batch changed while the collaborator was writing".into(),
            )
        })?;
    validate_checkpoint(workspace, batch)?;
    if operations.len() != 1
        || operations[0].kind != SceneTextOperationKind::Add
        || operations[0].position != SceneInsertPosition::End
        || operations[0].scene_id.is_some()
        || operations[0].anchor_scene_id.is_some()
    {
        return Err(StudioError::Invalid("the collaborator must write exactly one new scene per queue step; no scene was changed. Resume to try this step again".into()));
    }
    let draft = operations[0]
        .scene
        .as_ref()
        .ok_or_else(|| StudioError::Invalid("the collaborator omitted this scene's text".into()))?;
    if draft.h3_prompt.trim().chars().count() < 20
        || draft.continuity_out.trim().is_empty()
        || (draft.duration_seconds - batch.scene_seconds).abs() > 0.01
    {
        return Err(StudioError::Invalid("this scene needs a complete H3 prompt, a visible final frame, and the requested duration. Saved scenes are preserved; resume to retry this scene".into()));
    }
    if workspace
        .scenes
        .iter()
        .any(|scene| scene.h3_prompt.trim() == draft.h3_prompt.trim())
    {
        return Err(StudioError::Invalid("the collaborator repeated an existing scene prompt. Saved scenes are preserved; resume to request a distinct scene".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::MovieSceneBatchRequest;
    use tempfile::tempdir;

    async fn fixture(studio: &MovieStudio) -> MovieStudioChatRequest {
        let project = studio
            .create_producer_base(
                "A keeper follows a signal through the fog.".into(),
                super::super::super::MovieSettings::default(),
                Vec::new(),
                "test",
                false,
            )
            .unwrap();
        let workspace = studio.save_story_revision(SaveMovieStoryRevisionRequest {
            project_id: project.id.clone(), parent_revision_id: None,
            markdown: "# The signal\nA keeper follows the foghorn to its source, finds a lost boat, and guides it home.".into(), instruction: "Brief story".into(),
        }, None).await.unwrap();
        let revision = workspace.active_story_revision_id.unwrap();
        studio
            .accept_story_revision(
                AcceptMovieStoryRevisionRequest {
                    project_id: project.id.clone(),
                    revision_id: revision.clone(),
                    conversation_mode: MovieStudioConversationMode::Fresh,
                },
                None,
            )
            .await
            .unwrap();
        serde_json::from_value(serde_json::json!({
            "requestId":uuid::Uuid::new_v4().to_string(), "projectId":project.id,
            "kind":"scenes", "mode":"continue", "modelId":"test", "instruction":"Follow the signal.",
            "storyRevisionId":revision, "selectedSceneIds":[], "sceneBatch":{"sceneCount":1440,"resume":false}
        })).unwrap()
    }

    fn operation() -> SceneTextOperation {
        SceneTextOperation { kind: SceneTextOperationKind::Add, scene_id: None, anchor_scene_id: None,
            position: SceneInsertPosition::End, scene: Some(SceneTextDraft {
                title:"The signal".into(), purpose:"Hear the call".into(), duration_seconds:5.0,
                h3_prompt:"0–5s: A keeper in a yellow coat opens the lighthouse door into dense fog. A distant foghorn sounds. No dialogue. End on the keeper's hand holding the door.".into(),
                continuity_in:"Inside the lighthouse".into(), continuity_out:"Keeper holds the open door".into(), transition:"Cut".into(),
            }) }
    }

    #[tokio::test]
    async fn scene_batch_checkpoints_survive_restart_and_do_not_repeat_saved_scenes() {
        let folder = tempdir().unwrap();
        let studio = MovieStudio::new(folder.path()).unwrap();
        let mut request = fixture(&studio).await;
        let batch = studio.begin_scene_batch(&request, None).await.unwrap();
        let prepared = studio.prepare_studio_turn(&request).await.unwrap();
        let (saved, changed) = studio
            .finish_scene_turn_inner(
                &request.project_id,
                &prepared.conversation.id,
                prepared.scene_revision,
                &[],
                "Scene 1 saved".into(),
                vec![operation()],
                Some(&batch.id),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            saved
                .scene_draft_batch
                .as_ref()
                .unwrap()
                .completed_scene_ids,
            changed
        );
        assert_eq!(saved.scenes.len(), 1);
        let restarted = MovieStudio::new(folder.path()).unwrap();
        let restored = restarted
            .get_producer_workspace(&request.project_id)
            .unwrap();
        assert_eq!(restored.scenes, saved.scenes);
        assert_eq!(restored.scene_draft_batch, saved.scene_draft_batch);
        request.scene_batch = Some(MovieSceneBatchRequest {
            scene_count: 1440,
            resume: true,
        });
        let resumed = restarted.begin_scene_batch(&request, None).await.unwrap();
        assert_eq!(resumed.completed_scene_ids.len(), 1);
        let failure = validate_batch_turn(&restored, &batch.id, &[operation()])
            .unwrap_err()
            .to_string();
        assert!(failure.contains("repeated"), "{failure}");
        assert_eq!(
            restarted
                .get_producer_workspace(&request.project_id)
                .unwrap()
                .scenes
                .len(),
            1
        );
        // A no-op save may advance the checkpoint, but changed creative text may not.
        let unchanged = restarted
            .save_producer_scenes(
                SaveMovieScenesRequest {
                    project_id: request.project_id.clone(),
                    expected_revision: restored.scene_revision,
                    scenes: restored.scenes.clone(),
                },
                None,
            )
            .await
            .unwrap();
        assert!(restarted.begin_scene_batch(&request, None).await.is_ok());
        let mut edited_scenes = unchanged.scenes;
        edited_scenes[0]
            .h3_prompt
            .push_str(" The producer adds a seagull landing.");
        restarted
            .save_producer_scenes(
                SaveMovieScenesRequest {
                    project_id: request.project_id.clone(),
                    expected_revision: unchanged.scene_revision,
                    scenes: edited_scenes,
                },
                None,
            )
            .await
            .unwrap();
        assert!(restarted
            .begin_scene_batch(&request, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("changed"));
    }

    #[tokio::test]
    async fn batch_rejects_multi_scene_and_media_mutation_operations_before_saving() {
        let folder = tempdir().unwrap();
        let studio = MovieStudio::new(folder.path()).unwrap();
        let request = fixture(&studio).await;
        let batch = studio.begin_scene_batch(&request, None).await.unwrap();
        let workspace = studio.get_producer_workspace(&request.project_id).unwrap();
        assert!(validate_batch_turn(&workspace, &batch.id, &[operation(), operation()]).is_err());
        let mut changed = operation();
        changed.kind = SceneTextOperationKind::Update;
        assert!(validate_batch_turn(&workspace, &batch.id, &[changed]).is_err());
        let mut wrong_duration = operation();
        wrong_duration.scene.as_mut().unwrap().duration_seconds = 15.0;
        assert!(validate_batch_turn(&workspace, &batch.id, &[wrong_duration]).is_err());
        assert!(studio
            .get_producer_workspace(&request.project_id)
            .unwrap()
            .scenes
            .is_empty());
    }
}
