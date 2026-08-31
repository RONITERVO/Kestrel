//! Producer-owned story and scene workspaces.
//!
//! This module deliberately has no model tool loop. Story prose, scene order, frame bindings, and
//! reference selections are durable producer data. Local-model collaboration is a bounded chat
//! operation layered over these records; rendering remains an explicit native command elsewhere.

use crate::models::{
    AcceptMovieStoryRevisionRequest, AttachMovieProducerReferencesRequest,
    CreateMovieProducerProjectRequest, MovieProducerWorkspace, MovieSceneDraft,
    MovieSceneFrameSourceKind, MovieSceneReferenceSelection, MovieStoryRevision,
    MovieStoryRevisionOrigin, MovieStudioChatRequest, MovieStudioConversation,
    MovieStudioConversationKind, MovieStudioConversationMode, MovieStudioConversationSummary,
    MovieStudioMessage, MovieStudioMessageRole, ResetMovieStudioConversationRequest,
    SaveMovieScenesRequest, SaveMovieStoryRevisionRequest,
};
use chrono::Utc;
use serde::{de::DeserializeOwned, Serialize};
use std::{collections::HashSet, fs, path::Path};
use tauri::{AppHandle, Emitter};

use super::{
    default_gain, default_speed, validate_id, ClipEdit, ClipVersion, MoviePlan, MovieQualityReview,
    MovieSettings, MovieStudio, PlannedClip, ProducerReferenceRequest, RenderedClip, StudioError,
};

const PRODUCER_SCHEMA_VERSION: u32 = 1;
const MAX_STORY_BYTES: usize = 256 * 1024;
const MAX_STORY_INSTRUCTION_CHARS: usize = 16_000;
const MAX_SCENE_PROMPT_BYTES: usize = 64 * 1024;
const MAX_REFERENCE_GUIDANCE_CHARS: usize = 4_000;
const MAX_CONVERSATION_BYTES: u64 = 16 * 1024 * 1024;

pub(super) struct PreparedStudioTurn {
    pub conversation: MovieStudioConversation,
    pub story_revision_id: String,
    pub story_markdown: String,
    pub scene_revision: u64,
    pub scenes: Vec<MovieSceneDraft>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SceneHistorySnapshot<'a> {
    schema_version: u32,
    scene_revision: u64,
    scenes: &'a [MovieSceneDraft],
    /// Render state before this scene revision was applied. This keeps masters and versions for
    /// removed scenes recoverable without returning them to the active plan.
    previous_rendered_clips: &'a [RenderedClip],
}

#[derive(Debug, Clone)]
pub(super) struct SceneTextDraft {
    pub title: String,
    pub purpose: String,
    pub duration_seconds: f32,
    pub h3_prompt: String,
    pub continuity_in: String,
    pub continuity_out: String,
    pub transition: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SceneTextOperationKind {
    Add,
    Update,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SceneInsertPosition {
    Before,
    After,
    End,
}

#[derive(Debug, Clone)]
pub(super) struct SceneTextOperation {
    pub kind: SceneTextOperationKind,
    pub scene_id: Option<String>,
    pub anchor_scene_id: Option<String>,
    pub position: SceneInsertPosition,
    pub scene: Option<SceneTextDraft>,
}

impl MovieStudio {
    pub fn create_producer_project(
        &self,
        request: CreateMovieProducerProjectRequest,
        collaborator_name: &str,
        advanced: bool,
    ) -> Result<super::MovieProject, StudioError> {
        if request.starting_material.trim().chars().count() < 3 {
            return Err(StudioError::Invalid(
                "starting material must contain at least three characters".into(),
            ));
        }
        if request.collaborator_model_id.trim().is_empty() {
            return Err(StudioError::Invalid(
                "choose a local story collaborator before creating the project".into(),
            ));
        }
        let settings = MovieSettings {
            width: request.settings.width,
            height: request.settings.height,
            clip_seconds: request.settings.clip_seconds,
            steps: request.settings.steps,
            max_clips: request.settings.max_clips,
            seed: request.settings.seed,
            temperature: 0.7,
            top_p: 0.95,
            top_k: 40,
            thinking_budget: request.settings.thinking_budget,
            max_output_tokens: request.settings.max_output_tokens,
            context_window: request.settings.context_window.unwrap_or_default(),
            comfy_root: request.settings.comfy_root,
            ref_image_size: request.settings.ref_image_size,
        };
        let references = request
            .references
            .into_iter()
            .map(|reference| ProducerReferenceRequest {
                asset_id: reference.asset_id,
                description: reference.description,
                use_embedded_audio: reference.include_embedded_audio,
                embedded_audio_description: reference.embedded_audio_description,
            })
            .collect();
        let project = self.create_producer_base(
            request.starting_material,
            settings,
            references,
            collaborator_name,
            advanced,
        )?;
        self.get_producer_workspace(&project.id)?;
        Ok(project)
    }

    pub async fn attach_producer_references(
        &self,
        request: AttachMovieProducerReferencesRequest,
        app: Option<&AppHandle>,
    ) -> Result<super::MovieProject, StudioError> {
        validate_id(&request.project_id)?;
        if request.references.is_empty() {
            return Err(StudioError::Invalid(
                "choose at least one reference to attach".into(),
            ));
        }
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut project = self.get(&request.project_id)?;
        let mut combined = project
            .references
            .iter()
            .map(|reference| ProducerReferenceRequest {
                asset_id: reference.asset_id.clone(),
                description: reference.description.clone(),
                use_embedded_audio: reference.use_embedded_audio,
                embedded_audio_description: reference.embedded_audio_description.clone(),
            })
            .collect::<Vec<_>>();
        for reference in request.references {
            if combined
                .iter()
                .any(|existing| existing.asset_id == reference.asset_id)
            {
                return Err(StudioError::Invalid(
                    "one of the selected references is already attached to this project".into(),
                ));
            }
            combined.push(ProducerReferenceRequest {
                asset_id: reference.asset_id,
                description: reference.description,
                use_embedded_audio: reference.include_embedded_audio,
                embedded_audio_description: reference.embedded_audio_description,
            });
        }
        project.references = self.materialize_references(&project.id, combined)?;
        project.detail = format!(
            "{} producer reference{} available. Scene selections were not changed.",
            project.references.len(),
            if project.references.len() == 1 {
                " is"
            } else {
                "s are"
            }
        );
        super::write_json_atomic(
            &self.project_dir(&project.id).join("references.json"),
            &project.references,
        )?;
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub(super) async fn prepare_studio_turn(
        &self,
        request: &MovieStudioChatRequest,
    ) -> Result<PreparedStudioTurn, StudioError> {
        validate_id(&request.project_id)?;
        validate_id(&request.request_id)
            .map_err(|_| StudioError::Invalid("Studio chat request identity is invalid".into()))?;
        validate_instruction(&request.instruction)?;
        if request.instruction.trim().chars().count() < 2 {
            return Err(StudioError::Invalid(
                "Studio chat direction must contain at least two characters".into(),
            ));
        }
        if let Some(id) = request.conversation_id.as_deref() {
            validate_id(id).map_err(|_| {
                StudioError::Invalid("Studio conversation identity is invalid".into())
            })?;
        }
        if let Some(id) = request.story_revision_id.as_deref() {
            validate_id(id)
                .map_err(|_| StudioError::Invalid("story revision identity is invalid".into()))?;
        }
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut workspace = self.get_producer_workspace(&request.project_id)?;
        let (story_revision_id, story_markdown) = match request.kind {
            MovieStudioConversationKind::Story => {
                let requested = request
                    .story_revision_id
                    .clone()
                    .or_else(|| workspace.active_story_revision_id.clone());
                match requested {
                    Some(id) => {
                        let revision = workspace
                            .story_revisions
                            .iter()
                            .find(|revision| revision.id == id)
                            .ok_or_else(|| {
                                StudioError::Invalid(
                                    "the selected story revision is no longer available".into(),
                                )
                            })?;
                        (revision.id.clone(), revision.markdown.clone())
                    }
                    None => (String::new(), String::new()),
                }
            }
            MovieStudioConversationKind::Scenes => {
                let id = workspace
                    .accepted_story_revision_id
                    .clone()
                    .ok_or_else(|| {
                        StudioError::Invalid(
                            "accept a story revision before opening the scene collaborator".into(),
                        )
                    })?;
                if request
                    .story_revision_id
                    .as_deref()
                    .is_some_and(|requested| requested != id)
                {
                    return Err(StudioError::Invalid(
                        "the scene collaborator can only use the currently accepted story revision"
                            .into(),
                    ));
                }
                let revision = workspace
                    .story_revisions
                    .iter()
                    .find(|revision| revision.id == id)
                    .ok_or_else(|| {
                        StudioError::Invalid(
                            "the accepted story revision is missing from the producer workspace"
                                .into(),
                        )
                    })?;
                (id, revision.markdown.clone())
            }
        };
        let selected = request
            .selected_scene_ids
            .iter()
            .cloned()
            .collect::<HashSet<_>>();
        if selected.len() != request.selected_scene_ids.len()
            || selected
                .iter()
                .any(|id| !workspace.scenes.iter().any(|scene| &scene.id == id))
        {
            return Err(StudioError::Invalid(
                "selected scene context contains an unknown or duplicate scene".into(),
            ));
        }
        let mut conversation = if request.mode == MovieStudioConversationMode::Continue {
            request
                .conversation_id
                .as_deref()
                .or(match request.kind {
                    MovieStudioConversationKind::Story => {
                        workspace.active_story_conversation_id.as_deref()
                    }
                    MovieStudioConversationKind::Scenes => {
                        workspace.active_scene_conversation_id.as_deref()
                    }
                })
                .map(|id| self.get_producer_conversation(&request.project_id, id))
                .transpose()?
                .filter(|conversation| conversation.kind == request.kind && !conversation.archived)
        } else {
            None
        }
        .unwrap_or_else(|| {
            new_conversation(
                request.kind,
                &story_revision_id,
                match request.kind {
                    MovieStudioConversationKind::Story => "Story room".into(),
                    MovieStudioConversationKind::Scenes => "Scene room".into(),
                },
            )
        });
        if conversation.kind != request.kind {
            return Err(StudioError::Invalid(
                "the selected conversation belongs to the other Studio room".into(),
            ));
        }
        conversation
            .story_revision_id
            .clone_from(&story_revision_id);
        conversation.updated_at = Utc::now().to_rfc3339();
        conversation.messages.push(MovieStudioMessage {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: conversation.updated_at.clone(),
            role: MovieStudioMessageRole::Producer,
            markdown: request.instruction.trim().into(),
            story_revision_id: nonempty(&story_revision_id),
            selected_scene_ids: request.selected_scene_ids.clone(),
        });
        self.save_conversation(&request.project_id, &conversation)?;
        upsert_conversation_summary(&mut workspace, &conversation);
        match request.kind {
            MovieStudioConversationKind::Story => {
                workspace.active_story_conversation_id = Some(conversation.id.clone());
            }
            MovieStudioConversationKind::Scenes => {
                workspace.active_scene_conversation_id = Some(conversation.id.clone());
            }
        }
        workspace.updated_at = Utc::now().to_rfc3339();
        self.save_producer_workspace(&workspace)?;
        Ok(PreparedStudioTurn {
            conversation,
            story_revision_id,
            story_markdown,
            scene_revision: workspace.scene_revision,
            scenes: workspace.scenes,
        })
    }

    pub(super) async fn finish_story_turn(
        &self,
        project_id: &str,
        conversation_id: &str,
        parent_revision_id: Option<String>,
        instruction: &str,
        markdown: String,
        app: Option<&AppHandle>,
    ) -> Result<MovieStoryRevision, StudioError> {
        validate_story_markdown(&markdown)?;
        let lock = self.project_lock(project_id)?;
        let _guard = lock.lock().await;
        let mut workspace = self.get_producer_workspace(project_id)?;
        if parent_revision_id.as_deref().is_some_and(|parent| {
            !workspace
                .story_revisions
                .iter()
                .any(|revision| revision.id == parent)
        }) {
            return Err(StudioError::Invalid(
                "the story revision used by this response is no longer available".into(),
            ));
        }
        let revision = new_story_revision(
            &workspace,
            parent_revision_id,
            MovieStoryRevisionOrigin::Collaborator,
            instruction.into(),
            markdown,
        );
        self.persist_story_revision(project_id, &revision)?;
        workspace.active_story_revision_id = Some(revision.id.clone());
        workspace.story_revisions.push(revision.clone());
        let mut conversation = self.get_producer_conversation(project_id, conversation_id)?;
        conversation.story_revision_id = revision.id.clone();
        conversation.updated_at = Utc::now().to_rfc3339();
        conversation.messages.push(MovieStudioMessage {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: conversation.updated_at.clone(),
            role: MovieStudioMessageRole::Collaborator,
            markdown: revision.markdown.clone(),
            story_revision_id: Some(revision.id.clone()),
            selected_scene_ids: Vec::new(),
        });
        self.save_conversation(project_id, &conversation)?;
        upsert_conversation_summary(&mut workspace, &conversation);
        workspace.updated_at = Utc::now().to_rfc3339();
        self.save_producer_workspace(&workspace)?;
        self.emit_producer_workspace(&workspace, app);
        Ok(revision)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn finish_scene_turn(
        &self,
        project_id: &str,
        conversation_id: &str,
        expected_scene_revision: u64,
        selected_scene_ids: &[String],
        reply_markdown: String,
        operations: Vec<SceneTextOperation>,
        app: Option<&AppHandle>,
    ) -> Result<(MovieProducerWorkspace, Vec<String>), StudioError> {
        if operations.len() > 64 {
            return Err(StudioError::Invalid(
                "the scene collaborator returned more than 64 changes in one response".into(),
            ));
        }
        let lock = self.project_lock(project_id)?;
        let _guard = lock.lock().await;
        let project = self.get(project_id)?;
        let mut workspace = self.get_producer_workspace(project_id)?;
        if workspace.scene_revision != expected_scene_revision {
            return Err(StudioError::Invalid(
                "scene cards changed while the collaborator was writing; its response was saved in chat but no scene was overwritten"
                    .into(),
            ));
        }
        let accepted_story = workspace
            .accepted_story_revision_id
            .clone()
            .ok_or_else(|| StudioError::Invalid("no story revision is accepted".into()))?;
        let selected = selected_scene_ids.iter().collect::<HashSet<_>>();
        let now = Utc::now().to_rfc3339();
        let mut changed = Vec::new();
        for operation in operations {
            match operation.kind {
                SceneTextOperationKind::Add => {
                    let draft = operation.scene.ok_or_else(|| {
                        StudioError::Invalid("an add-scene response omitted the scene text".into())
                    })?;
                    let id = uuid::Uuid::new_v4().to_string();
                    let scene = MovieSceneDraft {
                        id: id.clone(),
                        revision: 1,
                        title: draft.title,
                        purpose: draft.purpose,
                        duration_seconds: draft.duration_seconds,
                        h3_prompt: draft.h3_prompt,
                        continuity_in: draft.continuity_in,
                        continuity_out: draft.continuity_out,
                        transition: draft.transition,
                        first_frame: None,
                        last_frame: None,
                        references: Vec::new(),
                        story_revision_id: accepted_story.clone(),
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    };
                    let insertion = match operation.position {
                        SceneInsertPosition::End => workspace.scenes.len(),
                        SceneInsertPosition::Before | SceneInsertPosition::After => {
                            let anchor = operation.anchor_scene_id.as_deref().ok_or_else(|| {
                                StudioError::Invalid(
                                    "a before/after scene insertion omitted its anchor".into(),
                                )
                            })?;
                            let index = workspace
                                .scenes
                                .iter()
                                .position(|scene| scene.id == anchor)
                                .ok_or_else(|| {
                                    StudioError::Invalid(
                                        "the scene collaborator used an unknown insertion anchor"
                                            .into(),
                                    )
                                })?;
                            index + usize::from(operation.position == SceneInsertPosition::After)
                        }
                    };
                    workspace.scenes.insert(insertion, scene);
                    changed.push(id);
                }
                SceneTextOperationKind::Update => {
                    let id = operation.scene_id.as_deref().ok_or_else(|| {
                        StudioError::Invalid("an update response omitted its scene id".into())
                    })?;
                    if !selected.contains(&id.to_string()) {
                        return Err(StudioError::Invalid(
                            "the collaborator tried to update a scene the producer did not include in context"
                                .into(),
                        ));
                    }
                    let draft = operation.scene.ok_or_else(|| {
                        StudioError::Invalid("an update response omitted the scene text".into())
                    })?;
                    let scene = workspace
                        .scenes
                        .iter_mut()
                        .find(|scene| scene.id == id)
                        .ok_or_else(|| {
                            StudioError::Invalid("updated scene no longer exists".into())
                        })?;
                    scene.revision = scene.revision.saturating_add(1);
                    scene.title = draft.title;
                    scene.purpose = draft.purpose;
                    scene.duration_seconds = draft.duration_seconds;
                    scene.h3_prompt = draft.h3_prompt;
                    scene.continuity_in = draft.continuity_in;
                    scene.continuity_out = draft.continuity_out;
                    scene.transition = draft.transition;
                    scene.story_revision_id.clone_from(&accepted_story);
                    scene.updated_at = now.clone();
                    changed.push(id.into());
                }
                SceneTextOperationKind::Remove => {
                    let id = operation.scene_id.as_deref().ok_or_else(|| {
                        StudioError::Invalid("a remove response omitted its scene id".into())
                    })?;
                    if !selected.contains(&id.to_string()) {
                        return Err(StudioError::Invalid(
                            "the collaborator tried to remove a scene the producer did not include in context"
                                .into(),
                        ));
                    }
                    let before = workspace.scenes.len();
                    workspace.scenes.retain(|scene| scene.id != id);
                    if workspace.scenes.len() == before {
                        return Err(StudioError::Invalid(
                            "removed scene no longer exists".into(),
                        ));
                    }
                    changed.push(id.into());
                }
            }
        }
        workspace.scenes = validate_scenes(
            workspace.scenes,
            &project.references,
            &accepted_story,
            project.settings.max_clips,
        )?;
        workspace.scene_revision = workspace.scene_revision.saturating_add(1);
        workspace.updated_at = now.clone();
        self.persist_scene_snapshot(&workspace, &project.clips)?;
        let mut conversation = self.get_producer_conversation(project_id, conversation_id)?;
        conversation.updated_at = now;
        conversation.messages.push(MovieStudioMessage {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: conversation.updated_at.clone(),
            role: MovieStudioMessageRole::Collaborator,
            markdown: reply_markdown,
            story_revision_id: Some(accepted_story),
            selected_scene_ids: selected_scene_ids.to_vec(),
        });
        self.save_conversation(project_id, &conversation)?;
        upsert_conversation_summary(&mut workspace, &conversation);
        self.save_producer_workspace(&workspace)?;
        self.sync_scene_plan(project_id, &workspace, app)?;
        self.emit_producer_workspace(&workspace, app);
        Ok((workspace, changed))
    }

    pub(super) async fn preserve_interrupted_turn(
        &self,
        project_id: &str,
        conversation_id: &str,
        partial: &str,
        label: &str,
    ) -> Result<(), StudioError> {
        if partial.trim().is_empty() {
            return Ok(());
        }
        let lock = self.project_lock(project_id)?;
        let _guard = lock.lock().await;
        let mut workspace = self.get_producer_workspace(project_id)?;
        let mut conversation = self.get_producer_conversation(project_id, conversation_id)?;
        conversation.updated_at = Utc::now().to_rfc3339();
        conversation.messages.push(MovieStudioMessage {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: conversation.updated_at.clone(),
            role: MovieStudioMessageRole::System,
            markdown: format!("{label}\n\n{partial}"),
            story_revision_id: nonempty(&conversation.story_revision_id),
            selected_scene_ids: Vec::new(),
        });
        self.save_conversation(project_id, &conversation)?;
        upsert_conversation_summary(&mut workspace, &conversation);
        workspace.updated_at = Utc::now().to_rfc3339();
        self.save_producer_workspace(&workspace)
    }

    pub fn get_producer_workspace(
        &self,
        project_id: &str,
    ) -> Result<MovieProducerWorkspace, StudioError> {
        validate_id(project_id)?;
        let project = self.get(project_id)?;
        let path = self.producer_workspace_path(project_id);
        if path.is_file() || path.with_extension("json.backup").is_file() {
            let workspace: MovieProducerWorkspace = read_recoverable_json(&path)?;
            if workspace.project_id != project_id {
                return Err(StudioError::Invalid(
                    "movie producer workspace belongs to a different project".into(),
                ));
            }
            return Ok(workspace);
        }
        let now = Utc::now().to_rfc3339();
        let workspace = MovieProducerWorkspace {
            schema_version: PRODUCER_SCHEMA_VERSION,
            project_id: project_id.into(),
            created_at: project.created_at,
            updated_at: now,
            active_story_revision_id: None,
            accepted_story_revision_id: None,
            active_story_conversation_id: None,
            active_scene_conversation_id: None,
            story_revisions: Vec::new(),
            conversations: Vec::new(),
            scenes: Vec::new(),
            scene_revision: 0,
        };
        Ok(workspace)
    }

    pub fn get_producer_conversation(
        &self,
        project_id: &str,
        conversation_id: &str,
    ) -> Result<MovieStudioConversation, StudioError> {
        validate_id(project_id)?;
        validate_id(conversation_id)?;
        let workspace = self.get_producer_workspace(project_id)?;
        if !workspace
            .conversations
            .iter()
            .any(|conversation| conversation.id == conversation_id)
        {
            return Err(StudioError::Invalid(
                "movie studio conversation is not part of this project".into(),
            ));
        }
        read_recoverable_json(&self.producer_conversation_path(project_id, conversation_id))
    }

    pub async fn save_story_revision(
        &self,
        request: SaveMovieStoryRevisionRequest,
        app: Option<&AppHandle>,
    ) -> Result<MovieProducerWorkspace, StudioError> {
        validate_id(&request.project_id)?;
        validate_story_markdown(&request.markdown)?;
        validate_instruction(&request.instruction)?;
        if let Some(parent) = request.parent_revision_id.as_deref() {
            validate_id(parent)?;
        }
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut workspace = self.get_producer_workspace(&request.project_id)?;
        if let Some(parent) = request.parent_revision_id.as_deref() {
            if !workspace
                .story_revisions
                .iter()
                .any(|revision| revision.id == parent)
            {
                return Err(StudioError::Invalid(
                    "the parent story revision is no longer available".into(),
                ));
            }
        }
        let revision = new_story_revision(
            &workspace,
            request.parent_revision_id,
            MovieStoryRevisionOrigin::Producer,
            request.instruction,
            request.markdown,
        );
        self.persist_story_revision(&request.project_id, &revision)?;
        workspace.active_story_revision_id = Some(revision.id.clone());
        workspace.story_revisions.push(revision);
        workspace.updated_at = Utc::now().to_rfc3339();
        self.save_producer_workspace(&workspace)?;
        self.emit_producer_workspace(&workspace, app);
        Ok(workspace)
    }

    pub async fn accept_story_revision(
        &self,
        request: AcceptMovieStoryRevisionRequest,
        app: Option<&AppHandle>,
    ) -> Result<MovieProducerWorkspace, StudioError> {
        validate_id(&request.project_id)?;
        validate_id(&request.revision_id)?;
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut workspace = self.get_producer_workspace(&request.project_id)?;
        let now = Utc::now().to_rfc3339();
        let selected = workspace
            .story_revisions
            .iter_mut()
            .find(|revision| revision.id == request.revision_id)
            .ok_or_else(|| StudioError::Invalid("story revision is no longer available".into()))?;
        selected.accepted_at = Some(now.clone());
        let accepted_number = selected.number;
        let accepted_markdown = selected.markdown.clone();
        workspace.active_story_revision_id = Some(request.revision_id.clone());
        workspace.accepted_story_revision_id = Some(request.revision_id.clone());

        let conversation = match request.conversation_mode {
            MovieStudioConversationMode::Continue => workspace
                .active_scene_conversation_id
                .as_deref()
                .and_then(|id| self.get_producer_conversation(&request.project_id, id).ok())
                .filter(|conversation| {
                    conversation.kind == MovieStudioConversationKind::Scenes
                        && !conversation.archived
                })
                .map(|mut conversation| {
                    conversation.story_revision_id = request.revision_id.clone();
                    conversation.updated_at = now.clone();
                    conversation.messages.push(MovieStudioMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        created_at: now.clone(),
                        role: MovieStudioMessageRole::System,
                        markdown: format!(
                            "Story context changed to revision {accepted_number}. Existing scenes were preserved."
                        ),
                        story_revision_id: Some(request.revision_id.clone()),
                        selected_scene_ids: Vec::new(),
                    });
                    conversation
                }),
            MovieStudioConversationMode::Fresh => None,
        }
        .unwrap_or_else(|| {
            new_conversation(
                MovieStudioConversationKind::Scenes,
                &request.revision_id,
                format!("Scenes from story revision {accepted_number}"),
            )
        });
        self.save_conversation(&request.project_id, &conversation)?;
        upsert_conversation_summary(&mut workspace, &conversation);
        workspace.active_scene_conversation_id = Some(conversation.id.clone());
        workspace.updated_at = now;
        self.save_producer_workspace(&workspace)?;

        let mut project = self.get(&request.project_id)?;
        project.title = markdown_title(&accepted_markdown).unwrap_or_else(|| project.title.clone());
        project.edit.export_title.clone_from(&project.title);
        project.status = "awaiting-review".into();
        project.phase = "story-approved".into();
        project.detail = format!(
            "Story revision {accepted_number} is the active scene context. Existing scene cards and masters were preserved."
        );
        project.error.clear();
        self.persist_emit(&mut project, app)?;
        self.emit_producer_workspace(&workspace, app);
        Ok(workspace)
    }

    pub async fn save_producer_scenes(
        &self,
        request: SaveMovieScenesRequest,
        app: Option<&AppHandle>,
    ) -> Result<MovieProducerWorkspace, StudioError> {
        validate_id(&request.project_id)?;
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let project = self.get(&request.project_id)?;
        let mut workspace = self.get_producer_workspace(&request.project_id)?;
        if workspace.scene_revision != request.expected_revision {
            return Err(StudioError::Invalid(format!(
                "scene cards changed after this editor loaded them (current revision {}); reload before saving so no producer work is overwritten",
                workspace.scene_revision
            )));
        }
        let accepted_story = workspace
            .accepted_story_revision_id
            .as_deref()
            .ok_or_else(|| {
                StudioError::Invalid("accept a story revision before creating scene cards".into())
            })?;
        let scenes = validate_scenes(
            request.scenes,
            &project.references,
            accepted_story,
            project.settings.max_clips,
        )?;
        workspace.scene_revision = workspace.scene_revision.saturating_add(1);
        workspace.scenes = scenes;
        workspace.updated_at = Utc::now().to_rfc3339();
        self.persist_scene_snapshot(&workspace, &project.clips)?;
        self.save_producer_workspace(&workspace)?;
        self.sync_scene_plan(&project.id, &workspace, app)?;
        self.emit_producer_workspace(&workspace, app);
        Ok(workspace)
    }

    pub async fn approve_producer_scenes(
        &self,
        project_id: &str,
        app: Option<&AppHandle>,
    ) -> Result<super::MovieProject, StudioError> {
        validate_id(project_id)?;
        let lock = self.project_lock(project_id)?;
        let _guard = lock.lock().await;
        let workspace = self.get_producer_workspace(project_id)?;
        if workspace.accepted_story_revision_id.is_none() {
            return Err(StudioError::Invalid(
                "accept a story revision before rendering scenes".into(),
            ));
        }
        if workspace.scenes.is_empty() {
            return Err(StudioError::Invalid(
                "add at least one producer-owned scene card before rendering".into(),
            ));
        }
        let mut missing = Vec::new();
        for (index, scene) in workspace.scenes.iter().enumerate() {
            if scene.h3_prompt.trim().chars().count() < 20 {
                missing.push(format!("scene {} needs a usable H3 prompt", index + 1));
            }
            if scene.continuity_out.trim().is_empty() {
                missing.push(format!(
                    "scene {} needs a visible final-frame state",
                    index + 1
                ));
            }
        }
        if !missing.is_empty() {
            return Err(StudioError::Invalid(format!(
                "scene cards are not render-ready: {}",
                missing.join("; ")
            )));
        }
        self.sync_scene_plan(project_id, &workspace, app)?;
        let mut project = self.get(project_id)?;
        project.status = "running".into();
        project.phase = "producer-approved".into();
        project.detail = format!(
            "The producer approved scene revision {}. H3 rendering may begin.",
            workspace.scene_revision
        );
        project.producer_approved_at = Utc::now().to_rfc3339();
        project.error.clear();
        self.persist_emit(&mut project, app)?;
        Ok(project)
    }

    pub async fn reset_producer_conversation(
        &self,
        request: ResetMovieStudioConversationRequest,
        app: Option<&AppHandle>,
    ) -> Result<MovieStudioConversation, StudioError> {
        validate_id(&request.project_id)?;
        validate_id(&request.conversation_id)?;
        let lock = self.project_lock(&request.project_id)?;
        let _guard = lock.lock().await;
        let mut workspace = self.get_producer_workspace(&request.project_id)?;
        let mut previous =
            self.get_producer_conversation(&request.project_id, &request.conversation_id)?;
        previous.archived = true;
        previous.updated_at = Utc::now().to_rfc3339();
        self.save_conversation(&request.project_id, &previous)?;
        upsert_conversation_summary(&mut workspace, &previous);

        let story_revision_id = if previous.kind == MovieStudioConversationKind::Story {
            workspace
                .active_story_revision_id
                .clone()
                .unwrap_or_default()
        } else {
            workspace
                .accepted_story_revision_id
                .clone()
                .ok_or_else(|| {
                    StudioError::Invalid(
                        "accept a story revision before starting a fresh scene conversation".into(),
                    )
                })?
        };
        let mut fresh = new_conversation(
            previous.kind,
            &story_revision_id,
            if previous.kind == MovieStudioConversationKind::Story {
                "Fresh story conversation".into()
            } else {
                "Fresh scene conversation".into()
            },
        );
        if request.keep_summary && !previous.summary.trim().is_empty() {
            fresh.summary.clone_from(&previous.summary);
            fresh.messages.push(MovieStudioMessage {
                id: uuid::Uuid::new_v4().to_string(),
                created_at: Utc::now().to_rfc3339(),
                role: MovieStudioMessageRole::System,
                markdown: format!("Prior conversation summary:\n\n{}", previous.summary),
                story_revision_id: nonempty(&story_revision_id),
                selected_scene_ids: Vec::new(),
            });
        }
        self.save_conversation(&request.project_id, &fresh)?;
        upsert_conversation_summary(&mut workspace, &fresh);
        if fresh.kind == MovieStudioConversationKind::Story {
            workspace.active_story_conversation_id = Some(fresh.id.clone());
        } else {
            workspace.active_scene_conversation_id = Some(fresh.id.clone());
        }
        workspace.updated_at = Utc::now().to_rfc3339();
        self.save_producer_workspace(&workspace)?;
        self.emit_producer_workspace(&workspace, app);
        Ok(fresh)
    }

    pub async fn save_producer_conversation_summary(
        &self,
        project_id: &str,
        conversation_id: &str,
        summary: String,
        app: Option<&AppHandle>,
    ) -> Result<MovieStudioConversation, StudioError> {
        validate_id(project_id)?;
        validate_id(conversation_id)?;
        let summary = summary.trim();
        if summary.chars().count() < 3 || summary.len() > 32 * 1024 {
            return Err(StudioError::Invalid(
                "conversation summary must contain 3 characters to 32 KiB".into(),
            ));
        }
        let lock = self.project_lock(project_id)?;
        let _guard = lock.lock().await;
        let mut workspace = self.get_producer_workspace(project_id)?;
        let mut conversation = self.get_producer_conversation(project_id, conversation_id)?;
        if conversation.archived {
            return Err(StudioError::Invalid(
                "archived Studio conversations cannot be changed".into(),
            ));
        }
        conversation.summary = summary.into();
        conversation.updated_at = Utc::now().to_rfc3339();
        self.save_conversation(project_id, &conversation)?;
        upsert_conversation_summary(&mut workspace, &conversation);
        workspace.updated_at = Utc::now().to_rfc3339();
        self.save_producer_workspace(&workspace)?;
        self.emit_producer_workspace(&workspace, app);
        Ok(conversation)
    }

    pub(super) fn producer_workspace_path(&self, project_id: &str) -> std::path::PathBuf {
        self.producer_root(project_id).join("workspace.json")
    }

    pub(super) fn producer_conversation_path(
        &self,
        project_id: &str,
        conversation_id: &str,
    ) -> std::path::PathBuf {
        self.producer_root(project_id)
            .join("conversations")
            .join(format!("{conversation_id}.json"))
    }

    pub(super) fn save_conversation(
        &self,
        project_id: &str,
        conversation: &MovieStudioConversation,
    ) -> Result<(), StudioError> {
        let path = self.producer_conversation_path(project_id, &conversation.id);
        let bytes = serde_json::to_vec_pretty(conversation)?;
        if bytes.len() as u64 > MAX_CONVERSATION_BYTES {
            return Err(StudioError::Invalid(
                "this Studio conversation reached its 16 MiB durable limit; summarize or clear it before continuing"
                    .into(),
            ));
        }
        write_recoverable_bytes(&path, &bytes)
    }

    pub(super) fn save_producer_workspace(
        &self,
        workspace: &MovieProducerWorkspace,
    ) -> Result<(), StudioError> {
        write_recoverable_json(
            &self.producer_workspace_path(&workspace.project_id),
            workspace,
        )
    }

    pub(super) fn emit_producer_workspace(
        &self,
        workspace: &MovieProducerWorkspace,
        app: Option<&AppHandle>,
    ) {
        if let Some(app) = app {
            let _ = app.emit("movie-producer-workspace", workspace);
        }
    }

    fn producer_root(&self, project_id: &str) -> std::path::PathBuf {
        self.project_dir(project_id).join("producer")
    }

    fn persist_story_revision(
        &self,
        project_id: &str,
        revision: &MovieStoryRevision,
    ) -> Result<(), StudioError> {
        let path = self
            .producer_root(project_id)
            .join("story-revisions")
            .join(format!("{:06}-{}.json", revision.number, revision.id));
        if path.exists() {
            return Err(StudioError::Invalid(
                "story revision identity already exists; no data was overwritten".into(),
            ));
        }
        write_recoverable_json(&path, revision)
    }

    fn persist_scene_snapshot(
        &self,
        workspace: &MovieProducerWorkspace,
        previous_rendered_clips: &[RenderedClip],
    ) -> Result<(), StudioError> {
        let path = self
            .producer_root(&workspace.project_id)
            .join("scene-history")
            .join(format!("{:010}.json", workspace.scene_revision));
        if path.exists() {
            return Err(StudioError::Invalid(
                "scene revision already exists; no producer work was overwritten".into(),
            ));
        }
        write_recoverable_json(
            &path,
            &SceneHistorySnapshot {
                schema_version: 1,
                scene_revision: workspace.scene_revision,
                scenes: &workspace.scenes,
                previous_rendered_clips,
            },
        )
    }

    fn sync_scene_plan(
        &self,
        project_id: &str,
        workspace: &MovieProducerWorkspace,
        app: Option<&AppHandle>,
    ) -> Result<(), StudioError> {
        let mut project = self.get(project_id)?;
        let accepted = workspace
            .accepted_story_revision_id
            .as_deref()
            .and_then(|id| {
                workspace
                    .story_revisions
                    .iter()
                    .find(|revision| revision.id == id)
            });
        let title = accepted
            .and_then(|revision| markdown_title(&revision.markdown))
            .unwrap_or_else(|| project.title.clone());
        let plan = MoviePlan {
            title: title.clone(),
            logline: String::new(),
            audience: String::new(),
            creative_direction: String::new(),
            continuity_bible: Vec::new(),
            source_credits: Vec::new(),
            quality_review: MovieQualityReview {
                attempts: 0,
                score: 0,
                verdict: "Producer-owned scene cards approved explicitly by the producer.".into(),
            },
            clips: workspace.scenes.iter().map(scene_to_planned_clip).collect(),
        };
        project.title = title.clone();
        project.edit.export_title = title;
        let previous_plan = project
            .plan
            .as_ref()
            .map(|plan| plan.clips.clone())
            .unwrap_or_default();
        let previous_renders = project.clips.clone();
        project.clips = plan
            .clips
            .iter()
            .enumerate()
            .map(|(index, clip)| {
                let previous = previous_renders
                    .iter()
                    .find(|existing| existing.id == clip.id);
                let changed = previous_plan
                    .iter()
                    .find(|existing| existing.id == clip.id)
                    .is_none_or(|existing| existing != clip);
                let mut versions = previous
                    .map(|existing| existing.versions.clone())
                    .unwrap_or_default();
                if changed {
                    if let Some(existing) = previous.filter(|existing| !existing.path.is_empty()) {
                        if !versions.iter().any(|version| version.path == existing.path) {
                            versions.push(ClipVersion {
                                id: uuid::Uuid::new_v4().to_string(),
                                created_at: Utc::now().to_rfc3339(),
                                title: existing.title.clone(),
                                prompt: existing.prompt.clone(),
                                duration_seconds: existing.duration_seconds,
                                seed: existing.seed,
                                path: existing.path.clone(),
                            });
                        }
                    }
                }
                RenderedClip {
                    id: clip.id.clone(),
                    index: index as u32,
                    title: clip.title.clone(),
                    prompt: clip.prompt.clone(),
                    duration_seconds: clip.duration_seconds,
                    seed: previous
                        .map(|existing| existing.seed)
                        .unwrap_or_else(|| super::derive_seed(project.settings.seed, index as u64)),
                    status: if changed {
                        "queued".into()
                    } else {
                        previous
                            .map(|existing| existing.status.clone())
                            .unwrap_or_else(|| "queued".into())
                    },
                    path: if changed {
                        String::new()
                    } else {
                        previous
                            .map(|existing| existing.path.clone())
                            .unwrap_or_default()
                    },
                    error: if changed {
                        String::new()
                    } else {
                        previous
                            .map(|existing| existing.error.clone())
                            .unwrap_or_default()
                    },
                    versions,
                }
            })
            .collect();
        let previous_edits = project.edit.clips.clone();
        project.edit.clips = project
            .clips
            .iter()
            .enumerate()
            .map(|(index, clip)| {
                previous_edits
                    .iter()
                    .find(|edit| edit.clip_id == clip.id)
                    .cloned()
                    .map(|mut edit| {
                        edit.order = index as u32;
                        edit
                    })
                    .unwrap_or_else(|| ClipEdit {
                        id: format!("edit-{}", clip.id),
                        clip_id: clip.id.clone(),
                        enabled: true,
                        order: index as u32,
                        trim_start: 0.0,
                        trim_end: 0.0,
                        audio_gain: default_gain(),
                        source_version_id: String::new(),
                        speed: default_speed(),
                        fade_in: 0.0,
                        fade_out: 0.0,
                        audio_fade_in: 0.0,
                        audio_fade_out: 0.0,
                        label: clip.title.clone(),
                        notes: String::new(),
                    })
            })
            .collect();
        project.plan = Some(plan.clone());
        project.status = "awaiting-review".into();
        project.phase = "scene-draft".into();
        project.detail = format!(
            "Producer scene revision {} is saved with {} scene card{}.",
            workspace.scene_revision,
            workspace.scenes.len(),
            if workspace.scenes.len() == 1 { "" } else { "s" }
        );
        project.producer_approved_at.clear();
        self.persist_emit(&mut project, app)?;
        super::write_json_atomic(&self.project_dir(project_id).join("plan.json"), &plan)
    }
}

fn scene_to_planned_clip(scene: &MovieSceneDraft) -> PlannedClip {
    PlannedClip {
        id: scene.id.clone(),
        title: scene.title.clone(),
        purpose: scene.purpose.clone(),
        duration_seconds: scene.duration_seconds,
        prompt: scene.h3_prompt.clone(),
        continuity_in: scene.continuity_in.clone(),
        continuity_out: scene.continuity_out.clone(),
        transition: scene.transition.clone(),
        use_previous_frame: scene
            .first_frame
            .as_ref()
            .is_some_and(|frame| frame.kind == MovieSceneFrameSourceKind::PreviousScene),
        source_refs: Vec::new(),
        reference_ids: scene
            .references
            .iter()
            .map(|reference| reference.asset_id.clone())
            .collect(),
        first_frame_reference_id: scene
            .first_frame
            .as_ref()
            .filter(|frame| frame.kind == MovieSceneFrameSourceKind::ReferenceImage)
            .and_then(|frame| frame.asset_id.clone())
            .unwrap_or_default(),
        last_frame_reference_id: scene
            .last_frame
            .as_ref()
            .filter(|frame| frame.kind == MovieSceneFrameSourceKind::ReferenceImage)
            .and_then(|frame| frame.asset_id.clone())
            .unwrap_or_default(),
        reference_selections: scene.references.clone(),
    }
}

fn validate_scenes(
    scenes: Vec<MovieSceneDraft>,
    references: &[super::MovieReference],
    accepted_story_id: &str,
    maximum: u32,
) -> Result<Vec<MovieSceneDraft>, StudioError> {
    if scenes.len() > maximum as usize {
        return Err(StudioError::Invalid(format!(
            "this project allows at most {maximum} scene cards"
        )));
    }
    let known = references
        .iter()
        .map(|reference| (reference.asset_id.as_str(), reference))
        .collect::<std::collections::HashMap<_, _>>();
    let now = Utc::now().to_rfc3339();
    let mut seen_scenes = HashSet::new();
    let mut result = Vec::with_capacity(scenes.len());
    for (index, mut scene) in scenes.into_iter().enumerate() {
        validate_id(&scene.id).map_err(|_| {
            StudioError::Invalid(format!("scene {} has an invalid identity", index + 1))
        })?;
        if !seen_scenes.insert(scene.id.clone()) {
            return Err(StudioError::Invalid(format!(
                "scene {} duplicates another scene identity",
                index + 1
            )));
        }
        if scene.title.trim().is_empty() || scene.title.chars().count() > 160 {
            return Err(StudioError::Invalid(format!(
                "scene {} needs a title of at most 160 characters",
                index + 1
            )));
        }
        if !(5.0..=15.0).contains(&scene.duration_seconds) {
            return Err(StudioError::Invalid(format!(
                "scene {} duration must be between 5 and 15 seconds",
                index + 1
            )));
        }
        if scene.h3_prompt.len() > MAX_SCENE_PROMPT_BYTES {
            return Err(StudioError::Invalid(format!(
                "scene {} H3 prompt exceeds 64 KiB",
                index + 1
            )));
        }
        scene.title = scene.title.trim().into();
        scene.purpose = scene.purpose.trim().into();
        scene.h3_prompt = scene.h3_prompt.trim().into();
        scene.continuity_in = scene.continuity_in.trim().into();
        scene.continuity_out = scene.continuity_out.trim().into();
        scene.transition = scene.transition.trim().into();
        scene.story_revision_id = accepted_story_id.into();
        if scene.created_at.trim().is_empty() {
            scene.created_at = now.clone();
        }
        scene.updated_at = now.clone();
        if (scene.first_frame.is_some() || scene.last_frame.is_some())
            && !scene.references.is_empty()
        {
            return Err(StudioError::Invalid(format!(
                "scene {} cannot combine H3 first/last-frame conditioning with native references",
                index + 1
            )));
        }
        if index == 0
            && scene
                .first_frame
                .as_ref()
                .is_some_and(|frame| frame.kind == MovieSceneFrameSourceKind::PreviousScene)
        {
            return Err(StudioError::Invalid(
                "the first scene cannot use a previous scene frame".into(),
            ));
        }
        if scene
            .last_frame
            .as_ref()
            .is_some_and(|frame| frame.kind == MovieSceneFrameSourceKind::PreviousScene)
        {
            return Err(StudioError::Invalid(
                "a last-frame binding must use a producer-selected image".into(),
            ));
        }
        for frame in [&scene.first_frame, &scene.last_frame]
            .into_iter()
            .flatten()
        {
            match frame.kind {
                MovieSceneFrameSourceKind::PreviousScene => {
                    if frame.asset_id.is_some() {
                        return Err(StudioError::Invalid(
                            "previous-scene frame bindings cannot contain an asset id".into(),
                        ));
                    }
                }
                MovieSceneFrameSourceKind::ReferenceImage => {
                    let asset_id = frame.asset_id.as_deref().ok_or_else(|| {
                        StudioError::Invalid(
                            "producer-selected frame binding is missing its image".into(),
                        )
                    })?;
                    if known
                        .get(asset_id)
                        .is_none_or(|reference| reference.kind != "image")
                    {
                        return Err(StudioError::Invalid(
                            "first and last frame bindings must use an image in this project"
                                .into(),
                        ));
                    }
                }
            }
        }
        let mut selected = HashSet::new();
        for reference in &mut scene.references {
            validate_scene_reference(reference, &known)?;
            if !selected.insert(reference.asset_id.clone()) {
                return Err(StudioError::Invalid(format!(
                    "scene {} selects the same native reference more than once",
                    index + 1
                )));
            }
            reference.guidance = reference.guidance.trim().into();
        }
        result.push(scene);
    }
    Ok(result)
}

fn validate_scene_reference(
    selection: &MovieSceneReferenceSelection,
    known: &std::collections::HashMap<&str, &super::MovieReference>,
) -> Result<(), StudioError> {
    let reference = known.get(selection.asset_id.as_str()).ok_or_else(|| {
        StudioError::Invalid("a scene selects a reference that is not in this project".into())
    })?;
    if !selection.use_visual && !selection.use_audio {
        return Err(StudioError::Invalid(
            "a selected scene reference must enable its visual/motion signal, exact audio, or both"
                .into(),
        ));
    }
    if selection.use_visual && !matches!(reference.kind.as_str(), "image" | "video") {
        return Err(StudioError::Invalid(
            "audio-only references cannot be used as a visual or motion signal".into(),
        ));
    }
    if selection.use_audio && !(reference.kind == "audio" || reference.has_audio) {
        return Err(StudioError::Invalid(
            "the selected reference has no audio signal".into(),
        ));
    }
    if reference.kind == "video" && selection.use_audio && !selection.use_visual {
        return Err(StudioError::Invalid(
            "H3 embedded video audio requires that video's motion reference to be enabled too; import the audio separately to use audio alone"
                .into(),
        ));
    }
    if selection.guidance.chars().count() > MAX_REFERENCE_GUIDANCE_CHARS {
        return Err(StudioError::Invalid(
            "per-scene reference guidance must not exceed 4,000 characters".into(),
        ));
    }
    Ok(())
}

fn validate_story_markdown(markdown: &str) -> Result<(), StudioError> {
    let length = markdown.trim().chars().count();
    if length < 3 || markdown.len() > MAX_STORY_BYTES {
        return Err(StudioError::Invalid(
            "story Markdown must contain at least 3 characters and no more than 256 KiB".into(),
        ));
    }
    Ok(())
}

fn validate_instruction(instruction: &str) -> Result<(), StudioError> {
    if instruction.chars().count() > MAX_STORY_INSTRUCTION_CHARS {
        return Err(StudioError::Invalid(
            "story revision note must not exceed 16,000 characters".into(),
        ));
    }
    Ok(())
}

fn new_story_revision(
    workspace: &MovieProducerWorkspace,
    parent_revision_id: Option<String>,
    origin: MovieStoryRevisionOrigin,
    instruction: String,
    markdown: String,
) -> MovieStoryRevision {
    MovieStoryRevision {
        id: uuid::Uuid::new_v4().to_string(),
        number: workspace
            .story_revisions
            .iter()
            .map(|revision| revision.number)
            .max()
            .unwrap_or_default()
            .saturating_add(1),
        parent_revision_id,
        created_at: Utc::now().to_rfc3339(),
        origin,
        instruction: instruction.trim().into(),
        markdown: markdown.trim().into(),
        accepted_at: None,
    }
}

pub(super) fn new_conversation(
    kind: MovieStudioConversationKind,
    story_revision_id: &str,
    title: String,
) -> MovieStudioConversation {
    let now = Utc::now().to_rfc3339();
    MovieStudioConversation {
        id: uuid::Uuid::new_v4().to_string(),
        kind,
        created_at: now.clone(),
        updated_at: now,
        story_revision_id: story_revision_id.into(),
        title,
        summary: String::new(),
        archived: false,
        messages: Vec::new(),
    }
}

pub(super) fn upsert_conversation_summary(
    workspace: &mut MovieProducerWorkspace,
    conversation: &MovieStudioConversation,
) {
    let summary = MovieStudioConversationSummary {
        id: conversation.id.clone(),
        kind: conversation.kind,
        created_at: conversation.created_at.clone(),
        updated_at: conversation.updated_at.clone(),
        story_revision_id: conversation.story_revision_id.clone(),
        title: conversation.title.clone(),
        summary: conversation.summary.clone(),
        message_count: conversation.messages.len(),
        archived: conversation.archived,
    };
    if let Some(existing) = workspace
        .conversations
        .iter_mut()
        .find(|existing| existing.id == conversation.id)
    {
        *existing = summary;
    } else {
        workspace.conversations.push(summary);
    }
    workspace
        .conversations
        .sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
}

fn markdown_title(markdown: &str) -> Option<String> {
    markdown.lines().find_map(|line| {
        let value = line.trim().trim_start_matches('#').trim();
        (!value.is_empty()).then(|| value.chars().take(160).collect())
    })
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.into())
}

fn write_recoverable_json<T: Serialize>(path: &Path, value: &T) -> Result<(), StudioError> {
    write_recoverable_bytes(path, &serde_json::to_vec_pretty(value)?)
}

fn write_recoverable_bytes(path: &Path, bytes: &[u8]) -> Result<(), StudioError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.backup");
    fs::write(&temporary, bytes)?;
    if path.is_file() {
        fs::copy(path, &backup)?;
        fs::remove_file(path)?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.is_file() {
            let _ = fs::copy(&backup, path);
        }
        return Err(error.into());
    }
    Ok(())
}

fn read_recoverable_json<T: DeserializeOwned>(path: &Path) -> Result<T, StudioError> {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(value) => Ok(value),
            Err(primary) => read_backup(path).map_err(|_| primary.into()),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            read_backup(path).map_err(|_| error.into())
        }
        Err(error) => Err(error.into()),
    }
}

fn read_backup<T: DeserializeOwned>(path: &Path) -> Result<T, StudioError> {
    let backup = path.with_extension("json.backup");
    let value = serde_json::from_slice(&fs::read(&backup)?)?;
    fs::copy(backup, path)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MovieSceneFrameSource, MovieSceneReferenceSelection};
    use tempfile::tempdir;

    fn seed_image_reference(studio: &MovieStudio, name: &str, contents: &[u8]) -> String {
        let object_root = studio.root.join("_references").join("objects");
        let temporary = object_root.join(format!("{name}.tmp"));
        fs::write(&temporary, contents).unwrap();
        let (_, id) = super::super::hash_file(&temporary).unwrap();
        let object = object_root.join(format!("{id}.png"));
        fs::rename(temporary, &object).unwrap();
        let asset = super::super::MovieReferenceAsset {
            id: id.clone(),
            name: name.into(),
            kind: "image".into(),
            mime_type: "image/png".into(),
            bytes: contents.len() as u64,
            duration_seconds: 0.0,
            width: 64,
            height: 64,
            has_audio: false,
            path: object.to_string_lossy().into_owned(),
            created_at: Utc::now().to_rfc3339(),
            generation: None,
        };
        super::super::write_json_atomic(
            &studio
                .root
                .join("_references")
                .join("meta")
                .join(format!("{id}.json")),
            &asset,
        )
        .unwrap();
        id
    }

    fn project(studio: &MovieStudio) -> super::super::MovieProject {
        studio
            .create_producer_base(
                "A lighthouse keeper hears a voice in the fog.".into(),
                super::super::MovieSettings::default(),
                Vec::new(),
                "test collaborator",
                false,
            )
            .unwrap()
    }

    #[test]
    fn reading_a_missing_producer_workspace_does_not_persist_it() {
        let root = tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let path = studio.producer_workspace_path(&project.id);

        let workspace = studio.get_producer_workspace(&project.id).unwrap();

        assert_eq!(workspace.project_id, project.id);
        assert!(!path.exists());
        assert!(!path.with_extension("json.backup").exists());
    }

    #[tokio::test]
    async fn every_story_save_creates_an_immutable_revision() {
        let root = tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let first = studio
            .save_story_revision(
                SaveMovieStoryRevisionRequest {
                    project_id: project.id.clone(),
                    parent_revision_id: None,
                    markdown: "# First\n\nA fogbound keeper waits.".into(),
                    instruction: "Initial producer draft".into(),
                },
                None,
            )
            .await
            .unwrap();
        let first_id = first.story_revisions[0].id.clone();
        let second = studio
            .save_story_revision(
                SaveMovieStoryRevisionRequest {
                    project_id: project.id.clone(),
                    parent_revision_id: Some(first_id.clone()),
                    markdown: "# Second\n\nThe signal answers her.".into(),
                    instruction: "Make the signal active".into(),
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(second.story_revisions.len(), 2);
        assert_eq!(second.story_revisions[1].parent_revision_id, Some(first_id));
        assert_eq!(
            second.story_revisions[0].markdown,
            "# First\n\nA fogbound keeper waits."
        );
    }

    #[tokio::test]
    async fn accepting_a_new_story_can_keep_scenes_and_start_fresh_context() {
        let root = tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let workspace = studio
            .save_story_revision(
                SaveMovieStoryRevisionRequest {
                    project_id: project.id.clone(),
                    parent_revision_id: None,
                    markdown: "# Keeper\n\nA short film.".into(),
                    instruction: String::new(),
                },
                None,
            )
            .await
            .unwrap();
        let accepted = studio
            .accept_story_revision(
                AcceptMovieStoryRevisionRequest {
                    project_id: project.id.clone(),
                    revision_id: workspace.story_revisions[0].id.clone(),
                    conversation_mode: MovieStudioConversationMode::Fresh,
                },
                None,
            )
            .await
            .unwrap();
        let conversation_id = accepted.active_scene_conversation_id.clone().unwrap();
        let scene = MovieSceneDraft {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            title: "Fog signal".into(),
            purpose: "Open the mystery".into(),
            duration_seconds: 5.0,
            h3_prompt: "[0s-5s] A static wide shot. No dialogue.".into(),
            continuity_in: "Fog covers the island.".into(),
            continuity_out: "The lamp turns toward camera.".into(),
            transition: "hard cut".into(),
            first_frame: None,
            last_frame: None,
            references: Vec::new(),
            story_revision_id: accepted.story_revisions[0].id.clone(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let saved = studio
            .save_producer_scenes(
                SaveMovieScenesRequest {
                    project_id: project.id.clone(),
                    expected_revision: 0,
                    scenes: vec![scene],
                },
                None,
            )
            .await
            .unwrap();
        let revised = studio
            .save_story_revision(
                SaveMovieStoryRevisionRequest {
                    project_id: project.id.clone(),
                    parent_revision_id: saved.accepted_story_revision_id.clone(),
                    markdown: "# Keeper\n\nA longer film.".into(),
                    instruction: "Expand it".into(),
                },
                None,
            )
            .await
            .unwrap();
        let next = studio
            .accept_story_revision(
                AcceptMovieStoryRevisionRequest {
                    project_id: project.id.clone(),
                    revision_id: revised.active_story_revision_id.clone().unwrap(),
                    conversation_mode: MovieStudioConversationMode::Fresh,
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(next.scenes.len(), 1);
        assert_ne!(next.active_scene_conversation_id.unwrap(), conversation_id);
    }

    #[tokio::test]
    async fn changing_a_scene_versions_its_old_master_and_queues_a_new_render() {
        let root = tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let project = project(&studio);
        let story = studio
            .save_story_revision(
                SaveMovieStoryRevisionRequest {
                    project_id: project.id.clone(),
                    parent_revision_id: None,
                    markdown: "# Keeper\n\nA short film.".into(),
                    instruction: String::new(),
                },
                None,
            )
            .await
            .unwrap();
        let accepted = studio
            .accept_story_revision(
                AcceptMovieStoryRevisionRequest {
                    project_id: project.id.clone(),
                    revision_id: story.story_revisions[0].id.clone(),
                    conversation_mode: MovieStudioConversationMode::Fresh,
                },
                None,
            )
            .await
            .unwrap();
        let scene_id = uuid::Uuid::new_v4().to_string();
        let mut scene = MovieSceneDraft {
            id: scene_id.clone(),
            revision: 1,
            title: "Fog signal".into(),
            purpose: "Open the mystery".into(),
            duration_seconds: 5.0,
            h3_prompt: "[0s-5s] A static wide shot. No dialogue.".into(),
            continuity_in: "Fog covers the island.".into(),
            continuity_out: "The lamp turns toward camera.".into(),
            transition: "hard cut".into(),
            first_frame: None,
            last_frame: None,
            references: Vec::new(),
            story_revision_id: accepted.story_revisions[0].id.clone(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let saved = studio
            .save_producer_scenes(
                SaveMovieScenesRequest {
                    project_id: project.id.clone(),
                    expected_revision: 0,
                    scenes: vec![scene.clone()],
                },
                None,
            )
            .await
            .unwrap();
        let mut rendered = studio.get(&project.id).unwrap();
        rendered.clips[0].status = "complete".into();
        rendered.clips[0].path = root
            .path()
            .join("old-master.mp4")
            .to_string_lossy()
            .into_owned();
        studio.save(&rendered).unwrap();

        scene.revision = 2;
        scene.h3_prompt = "[0s-5s] A slow push toward the lamp. No dialogue.".into();
        let revised = studio
            .save_producer_scenes(
                SaveMovieScenesRequest {
                    project_id: project.id.clone(),
                    expected_revision: saved.scene_revision,
                    scenes: vec![scene],
                },
                None,
            )
            .await
            .unwrap();
        let updated = studio.get(&project.id).unwrap();
        assert_eq!(updated.clips[0].status, "queued");
        assert!(updated.clips[0].path.is_empty());
        assert_eq!(updated.clips[0].versions.len(), 1);
        assert!(updated.clips[0].versions[0]
            .path
            .ends_with("old-master.mp4"));

        let removed = studio
            .save_producer_scenes(
                SaveMovieScenesRequest {
                    project_id: project.id.clone(),
                    expected_revision: revised.scene_revision,
                    scenes: Vec::new(),
                },
                None,
            )
            .await
            .unwrap();
        assert!(studio.get(&project.id).unwrap().clips.is_empty());
        let history_path = studio
            .producer_root(&project.id)
            .join("scene-history")
            .join(format!("{:010}.json", removed.scene_revision));
        let history: serde_json::Value =
            serde_json::from_slice(&fs::read(history_path).unwrap()).unwrap();
        assert_eq!(history["previousRenderedClips"][0]["id"], scene_id);
        assert!(history["previousRenderedClips"][0]["versions"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with("old-master.mp4"));
    }

    #[tokio::test]
    async fn attaching_a_reference_reuses_verified_existing_project_copies() {
        let root = tempdir().unwrap();
        let studio = MovieStudio::new(root.path()).unwrap();
        let first_id = seed_image_reference(&studio, "first", b"first immutable image");
        let second_id = seed_image_reference(&studio, "second", b"second immutable image");
        let project = studio
            .create_producer_base(
                "A keeper waits beside a fog signal.".into(),
                super::super::MovieSettings::default(),
                vec![super::super::ProducerReferenceRequest {
                    asset_id: first_id.clone(),
                    description: "The keeper's visual identity".into(),
                    use_embedded_audio: false,
                    embedded_audio_description: String::new(),
                }],
                "test collaborator",
                false,
            )
            .unwrap();
        let first_path = project.references[0].path.clone();

        let updated = studio
            .attach_producer_references(
                AttachMovieProducerReferencesRequest {
                    project_id: project.id,
                    references: vec![crate::models::MovieProducerReferenceRequest {
                        asset_id: second_id,
                        description: "The lighthouse exterior".into(),
                        include_embedded_audio: false,
                        embedded_audio_description: String::new(),
                    }],
                },
                None,
            )
            .await
            .unwrap();
        assert_eq!(updated.references.len(), 2);
        assert_eq!(updated.references[0].path, first_path);
        assert_eq!(
            super::super::hash_reference(Path::new(&first_path)).unwrap(),
            first_id
        );
    }

    #[test]
    fn frame_conditioning_and_native_references_are_mutually_exclusive() {
        let scene = MovieSceneDraft {
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            title: "Conflict".into(),
            purpose: String::new(),
            duration_seconds: 5.0,
            h3_prompt: String::new(),
            continuity_in: String::new(),
            continuity_out: String::new(),
            transition: "cut".into(),
            first_frame: Some(MovieSceneFrameSource {
                kind: MovieSceneFrameSourceKind::PreviousScene,
                asset_id: None,
            }),
            last_frame: None,
            references: vec![MovieSceneReferenceSelection {
                asset_id: "missing".into(),
                use_visual: true,
                use_audio: false,
                guidance: String::new(),
            }],
            story_revision_id: String::new(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let error = validate_scenes(vec![scene], &[], "story", 12).unwrap_err();
        assert!(error.to_string().contains("cannot combine"));
    }
}
