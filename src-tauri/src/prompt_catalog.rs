use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
};

const DEFAULT_PACK_TEXT: &str = include_str!("../prompts/default.json");
const FORMAT: &str = "kestrel.prompt-pack";
const VERSION: u32 = 1;
const MAX_PACK_BYTES: usize = 512 * 1024;
const MAX_PROMPT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptPack {
    pub format: String,
    pub version: u32,
    pub prompts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptId {
    ChatSystem,
    ComputerSystem,
    ComputerAttachmentNotice,
    ComputerObjectiveContinuation,
    ComputerCompaction,
    ResearchPlanningSystem,
    ResearchPlanningUser,
    ResearchSystem,
    ResearchQuestion,
    ResearchRequiredTool,
    ResearchNewSource,
    ResearchSynthesis,
    ResearchRetry,
    ResearchExpeditionRetry,
    MovieAgentSystem,
    MovieInitial,
    MovieResume,
    MovieContinue,
    MovieFirstCheck,
    MovieSecondCheck,
    MovieSubmitBlocked,
    MovieResponseCheckpoint,
    MovieProducerDirection,
    MovieClipAssistantSystem,
    MovieReviewerSystem,
    MovieCopilotSystem,
    MovieCopilotTool,
    MovieWorkspaceTool,
    StorySystem,
    ImageAssetSystem,
    ImageCompositionSystem,
    ReferenceSystem,
    MusicCaptionSystem,
    MusicLyricsSystem,
    PromptInventStory,
    PromptSourceDevelop,
    PromptSourceContinue,
    PromptStoryContext,
    PromptStoryMissing,
    PromptImageContext,
    PromptImageMissing,
    PromptMusicContext,
    PromptMusicMissing,
    PromptAssetMetadata,
    ImageStillnessSuffix,
    StudioQualificationSystem,
    StudioQualificationUser,
    FinalStoryDevelop,
    FinalStoryContinue,
    FinalImageAssetDevelop,
    FinalImageAssetContinue,
    FinalImageCompositionDevelop,
    FinalImageCompositionContinue,
    FinalReferenceDevelop,
    FinalReferenceContinue,
    FinalMusicCaptionDevelop,
    FinalMusicCaptionContinue,
    FinalMusicLyricsDevelop,
    FinalMusicLyricsContinue,
    StudioSubmissionCorrection,
    ToolAskUser,
    ToolListDirectory,
    ToolReadFile,
    ToolWriteFile,
    ToolCreateDirectory,
    ToolMovePath,
    ToolCopyFile,
    ToolReadAttachment,
    ToolRunProgram,
    ToolListProcesses,
    ToolOpenPath,
    ResearchToolSearch,
    ResearchToolSearchQuery,
    ResearchToolRead,
    ResearchToolSourceRef,
    ResearchToolSection,
    MovieWorkspacePath,
    MovieWorkspaceContent,
    MovieWorkspaceFiles,
    StudioSourceRefs,
    StudioReferenceIds,
    StudioPlanAudience,
    StudioContinuityBible,
    StudioQualificationTool,
    MovieCopilotRequest,
    MovieReferenceManifest,
    MovieAuthoritativeMemory,
    MovieWorkspaceContract,
    MovieWorkspaceReferencesEmpty,
    MovieWorkspaceReferencesIntro,
}

impl PromptId {
    pub const ALL: [Self; 90] = [
        Self::ChatSystem,
        Self::ComputerSystem,
        Self::ComputerAttachmentNotice,
        Self::ComputerObjectiveContinuation,
        Self::ComputerCompaction,
        Self::ResearchPlanningSystem,
        Self::ResearchPlanningUser,
        Self::ResearchSystem,
        Self::ResearchQuestion,
        Self::ResearchRequiredTool,
        Self::ResearchNewSource,
        Self::ResearchSynthesis,
        Self::ResearchRetry,
        Self::ResearchExpeditionRetry,
        Self::MovieAgentSystem,
        Self::MovieInitial,
        Self::MovieResume,
        Self::MovieContinue,
        Self::MovieFirstCheck,
        Self::MovieSecondCheck,
        Self::MovieSubmitBlocked,
        Self::MovieResponseCheckpoint,
        Self::MovieProducerDirection,
        Self::MovieClipAssistantSystem,
        Self::MovieReviewerSystem,
        Self::MovieCopilotSystem,
        Self::MovieCopilotTool,
        Self::MovieWorkspaceTool,
        Self::StorySystem,
        Self::ImageAssetSystem,
        Self::ImageCompositionSystem,
        Self::ReferenceSystem,
        Self::MusicCaptionSystem,
        Self::MusicLyricsSystem,
        Self::PromptInventStory,
        Self::PromptSourceDevelop,
        Self::PromptSourceContinue,
        Self::PromptStoryContext,
        Self::PromptStoryMissing,
        Self::PromptImageContext,
        Self::PromptImageMissing,
        Self::PromptMusicContext,
        Self::PromptMusicMissing,
        Self::PromptAssetMetadata,
        Self::ImageStillnessSuffix,
        Self::StudioQualificationSystem,
        Self::StudioQualificationUser,
        Self::FinalStoryDevelop,
        Self::FinalStoryContinue,
        Self::FinalImageAssetDevelop,
        Self::FinalImageAssetContinue,
        Self::FinalImageCompositionDevelop,
        Self::FinalImageCompositionContinue,
        Self::FinalReferenceDevelop,
        Self::FinalReferenceContinue,
        Self::FinalMusicCaptionDevelop,
        Self::FinalMusicCaptionContinue,
        Self::FinalMusicLyricsDevelop,
        Self::FinalMusicLyricsContinue,
        Self::StudioSubmissionCorrection,
        Self::ToolAskUser,
        Self::ToolListDirectory,
        Self::ToolReadFile,
        Self::ToolWriteFile,
        Self::ToolCreateDirectory,
        Self::ToolMovePath,
        Self::ToolCopyFile,
        Self::ToolReadAttachment,
        Self::ToolRunProgram,
        Self::ToolListProcesses,
        Self::ToolOpenPath,
        Self::ResearchToolSearch,
        Self::ResearchToolSearchQuery,
        Self::ResearchToolRead,
        Self::ResearchToolSourceRef,
        Self::ResearchToolSection,
        Self::MovieWorkspacePath,
        Self::MovieWorkspaceContent,
        Self::MovieWorkspaceFiles,
        Self::StudioSourceRefs,
        Self::StudioReferenceIds,
        Self::StudioPlanAudience,
        Self::StudioContinuityBible,
        Self::StudioQualificationTool,
        Self::MovieCopilotRequest,
        Self::MovieReferenceManifest,
        Self::MovieAuthoritativeMemory,
        Self::MovieWorkspaceContract,
        Self::MovieWorkspaceReferencesEmpty,
        Self::MovieWorkspaceReferencesIntro,
    ];

    pub const fn key(self) -> &'static str {
        match self {
            Self::ChatSystem => "chat.system",
            Self::ComputerSystem => "computer.system",
            Self::ComputerAttachmentNotice => "computer.attachment_notice",
            Self::ComputerObjectiveContinuation => "computer.objective_continuation",
            Self::ComputerCompaction => "computer.compaction",
            Self::ResearchPlanningSystem => "research.planning.system",
            Self::ResearchPlanningUser => "research.planning.user",
            Self::ResearchSystem => "research.system",
            Self::ResearchQuestion => "research.question",
            Self::ResearchRequiredTool => "research.required_tool",
            Self::ResearchNewSource => "research.new_source",
            Self::ResearchSynthesis => "research.synthesis",
            Self::ResearchRetry => "research.retry",
            Self::ResearchExpeditionRetry => "research.expedition_retry",
            Self::MovieAgentSystem => "movie.agent.system",
            Self::MovieInitial => "movie.agent.initial",
            Self::MovieResume => "movie.agent.resume",
            Self::MovieContinue => "movie.agent.continue",
            Self::MovieFirstCheck => "movie.agent.first_check",
            Self::MovieSecondCheck => "movie.agent.second_check",
            Self::MovieSubmitBlocked => "movie.agent.submit_blocked",
            Self::MovieResponseCheckpoint => "movie.agent.response_checkpoint",
            Self::MovieProducerDirection => "movie.agent.producer_direction",
            Self::MovieClipAssistantSystem => "movie.clip_assistant.system",
            Self::MovieReviewerSystem => "movie.reviewer.system",
            Self::MovieCopilotSystem => "movie.copilot.system",
            Self::MovieCopilotTool => "movie.copilot.tool",
            Self::MovieWorkspaceTool => "movie.workspace.tool",
            Self::StorySystem => "collaboration.story.system",
            Self::ImageAssetSystem => "collaboration.image_asset.system",
            Self::ImageCompositionSystem => "collaboration.image_composition.system",
            Self::ReferenceSystem => "collaboration.reference.system",
            Self::MusicCaptionSystem => "collaboration.music_caption.system",
            Self::MusicLyricsSystem => "collaboration.music_lyrics.system",
            Self::PromptInventStory => "collaboration.invent_story",
            Self::PromptSourceDevelop => "collaboration.source.develop",
            Self::PromptSourceContinue => "collaboration.source.continue",
            Self::PromptStoryContext => "collaboration.context.story",
            Self::PromptStoryMissing => "collaboration.context.story_missing",
            Self::PromptImageContext => "collaboration.context.image",
            Self::PromptImageMissing => "collaboration.context.image_missing",
            Self::PromptMusicContext => "collaboration.context.music",
            Self::PromptMusicMissing => "collaboration.context.music_missing",
            Self::PromptAssetMetadata => "collaboration.asset_metadata",
            Self::ImageStillnessSuffix => "image.h3.stillness_suffix",
            Self::StudioQualificationSystem => "studio.qualification.system",
            Self::StudioQualificationUser => "studio.qualification.user",
            Self::FinalStoryDevelop => "collaboration.final.story.develop",
            Self::FinalStoryContinue => "collaboration.final.story.continue",
            Self::FinalImageAssetDevelop => "collaboration.final.image_asset.develop",
            Self::FinalImageAssetContinue => "collaboration.final.image_asset.continue",
            Self::FinalImageCompositionDevelop => "collaboration.final.image_composition.develop",
            Self::FinalImageCompositionContinue => "collaboration.final.image_composition.continue",
            Self::FinalReferenceDevelop => "collaboration.final.reference.develop",
            Self::FinalReferenceContinue => "collaboration.final.reference.continue",
            Self::FinalMusicCaptionDevelop => "collaboration.final.music_caption.develop",
            Self::FinalMusicCaptionContinue => "collaboration.final.music_caption.continue",
            Self::FinalMusicLyricsDevelop => "collaboration.final.music_lyrics.develop",
            Self::FinalMusicLyricsContinue => "collaboration.final.music_lyrics.continue",
            Self::StudioSubmissionCorrection => "studio.submission_correction",
            Self::ToolAskUser => "computer.tool.ask_user",
            Self::ToolListDirectory => "computer.tool.list_directory",
            Self::ToolReadFile => "computer.tool.read_file",
            Self::ToolWriteFile => "computer.tool.write_file",
            Self::ToolCreateDirectory => "computer.tool.create_directory",
            Self::ToolMovePath => "computer.tool.move_path",
            Self::ToolCopyFile => "computer.tool.copy_file",
            Self::ToolReadAttachment => "computer.tool.read_attachment",
            Self::ToolRunProgram => "computer.tool.run_program",
            Self::ToolListProcesses => "computer.tool.list_processes",
            Self::ToolOpenPath => "computer.tool.open_path",
            Self::ResearchToolSearch => "research.tool.search_archive",
            Self::ResearchToolSearchQuery => "research.tool.search_query",
            Self::ResearchToolRead => "research.tool.read_source",
            Self::ResearchToolSourceRef => "research.tool.source_ref",
            Self::ResearchToolSection => "research.tool.section",
            Self::MovieWorkspacePath => "movie.workspace.path",
            Self::MovieWorkspaceContent => "movie.workspace.content",
            Self::MovieWorkspaceFiles => "movie.workspace.files",
            Self::StudioSourceRefs => "studio.schema.source_refs",
            Self::StudioReferenceIds => "studio.schema.reference_ids",
            Self::StudioPlanAudience => "studio.schema.audience",
            Self::StudioContinuityBible => "studio.schema.continuity_bible",
            Self::StudioQualificationTool => "studio.qualification.tool",
            Self::MovieCopilotRequest => "movie.copilot.request",
            Self::MovieReferenceManifest => "movie.reference_manifest",
            Self::MovieAuthoritativeMemory => "movie.authoritative_memory",
            Self::MovieWorkspaceContract => "movie.workspace.contract",
            Self::MovieWorkspaceReferencesEmpty => "movie.workspace.references_empty",
            Self::MovieWorkspaceReferencesIntro => "movie.workspace.references_intro",
        }
    }
}

#[derive(Debug)]
struct CatalogState {
    root: PathBuf,
    pack: PromptPack,
}

static CATALOG: OnceLock<RwLock<CatalogState>> = OnceLock::new();

fn default_pack() -> PromptPack {
    serde_json::from_str(DEFAULT_PACK_TEXT).expect("embedded Kestrel prompt pack must be valid")
}

pub fn initialize(root: &Path) -> Result<(), String> {
    let path = root.join("prompt-pack.json");
    let backup = path.with_extension("json.bak");
    let pack = match read_pack(&path) {
        Ok(Some(pack)) => pack,
        Ok(None) | Err(_) if backup.is_file() => {
            let recovered = read_pack(&backup)?
                .ok_or_else(|| "Prompt-pack recovery copy disappeared".to_string())?;
            write_recoverable(
                &path,
                serde_json::to_string_pretty(&recovered)
                    .map_err(|error| error.to_string())?
                    .as_bytes(),
            )?;
            recovered
        }
        Ok(None) => default_pack(),
        Err(error) => {
            eprintln!("The active prompt pack is invalid and no recovery copy is available: {error}");
            default_pack()
        }
    };
    let state = CatalogState {
        root: root.to_path_buf(),
        pack,
    };
    if let Some(lock) = CATALOG.get() {
        *lock
            .write()
            .map_err(|_| "Prompt catalog lock is poisoned".to_string())? = state;
    } else {
        CATALOG
            .set(RwLock::new(state))
            .map_err(|_| "Prompt catalog was initialized twice".to_string())?;
    }
    Ok(())
}

fn read_pack(path: &Path) -> Result<Option<PromptPack>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
    if metadata.len() as usize > MAX_PACK_BYTES {
        return Err("Prompt pack exceeds the 512 KiB limit".into());
    }
    let text = fs::read_to_string(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    parse_and_upgrade(&text).map(Some)
}

pub fn text(id: PromptId) -> String {
    let key = id.key();
    if let Some(lock) = CATALOG.get() {
        let state = match lock.read() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(prompt) = state.pack.prompts.get(key) {
            return prompt.clone();
        }
    }
    default_pack()
        .prompts
        .get(key)
        .cloned()
        .expect("required embedded prompt is missing")
}

pub fn render(id: PromptId, values: &[(&str, &str)]) -> String {
    let mut value = text(id);
    for (name, replacement) in values {
        value = value.replace(&format!("{{{{{name}}}}}"), replacement);
    }
    value
}

pub fn current_text() -> Result<String, String> {
    let pack = match CATALOG.get() {
        Some(lock) => {
            let state = match lock.read() {
                Ok(state) => state,
                Err(poisoned) => poisoned.into_inner(),
            };
            state.pack.clone()
        }
        None => default_pack(),
    };
    serde_json::to_string_pretty(&pack).map_err(|error| error.to_string())
}

/// Read-only access to the build's embedded default pack, so the UI can offer a
/// per-prompt "reset to build default" without mutating the active catalog.
pub fn default_text() -> Result<String, String> {
    serde_json::to_string_pretty(&default_pack()).map_err(|error| error.to_string())
}

pub fn save_text(value: &str) -> Result<String, String> {
    let pack = parse_and_validate(value)?;
    let lock = CATALOG
        .get()
        .ok_or_else(|| "Prompt catalog is not initialized".to_string())?;
    let mut state = lock
        .write()
        .map_err(|_| "Prompt catalog lock is poisoned".to_string())?;
    let path = state.root.join("prompt-pack.json");
    write_recoverable(
        &path,
        serde_json::to_string_pretty(&pack)
            .map_err(|error| error.to_string())?
            .as_bytes(),
    )?;
    state.pack = pack;
    serde_json::to_string_pretty(&state.pack).map_err(|error| error.to_string())
}

pub fn reset() -> Result<String, String> {
    let lock = CATALOG
        .get()
        .ok_or_else(|| "Prompt catalog is not initialized".to_string())?;
    let mut state = lock
        .write()
        .map_err(|_| "Prompt catalog lock is poisoned".to_string())?;
    let path = state.root.join("prompt-pack.json");
    let backup = path.with_extension("json.bak");
    if path.exists() {
        fs::remove_file(&path)
            .map_err(|error| format!("Could not remove the custom prompt pack: {error}"))?;
    }
    if backup.exists() {
        fs::remove_file(&backup)
            .map_err(|error| format!("Could not remove the custom prompt-pack backup: {error}"))?;
    }
    state.pack = default_pack();
    serde_json::to_string_pretty(&state.pack).map_err(|error| error.to_string())
}

pub fn export_text(value: &str) -> Result<PathBuf, String> {
    let pack = parse_and_validate(value)?;
    let lock = CATALOG
        .get()
        .ok_or_else(|| "Prompt catalog is not initialized".to_string())?;
    let state = lock
        .read()
        .map_err(|_| "Prompt catalog lock is poisoned".to_string())?;
    let folder = state.root.join("prompt-packs");
    fs::create_dir_all(&folder)
        .map_err(|error| format!("Could not create the prompt-pack export folder: {error}"))?;
    let path = folder.join(format!(
        "kestrel-prompts-{}.json",
        chrono::Utc::now().format("%Y%m%d-%H%M%S")
    ));
    write_recoverable(
        &path,
        serde_json::to_string_pretty(&pack)
            .map_err(|error| error.to_string())?
            .as_bytes(),
    )?;
    Ok(path)
}

pub fn import_path(path: &Path) -> Result<String, String> {
    if !path.is_absolute() {
        return Err("Prompt-pack path must be absolute".into());
    }
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Could not inspect the prompt pack: {error}"))?;
    if metadata.len() as usize > MAX_PACK_BYTES {
        return Err("Prompt pack exceeds the 512 KiB limit".into());
    }
    let source = fs::read_to_string(path)
        .map_err(|error| format!("Could not read the prompt pack: {error}"))?;
    let upgraded = parse_and_upgrade(&source)?;
    save_text(&serde_json::to_string_pretty(&upgraded).map_err(|error| error.to_string())?)
}

pub fn parse_and_validate(value: &str) -> Result<PromptPack, String> {
    if value.len() > MAX_PACK_BYTES {
        return Err("Prompt pack exceeds the 512 KiB limit".into());
    }
    let pack: PromptPack = serde_json::from_str(value)
        .map_err(|error| format!("Prompt pack JSON is invalid: {error}"))?;
    if pack.format != FORMAT || pack.version != VERSION {
        return Err(format!("Expected {FORMAT} version {VERSION}"));
    }
    let required = PromptId::ALL
        .iter()
        .map(|id| id.key())
        .collect::<BTreeSet<_>>();
    let supplied = pack
        .prompts
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if let Some(key) = required.difference(&supplied).next() {
        return Err(format!("Prompt pack is missing required prompt {key}"));
    }
    if let Some(key) = supplied.difference(&required).next() {
        return Err(format!("Prompt pack contains unknown prompt {key}"));
    }
    let defaults = default_pack();
    for (key, prompt) in &pack.prompts {
        if prompt.trim().is_empty() {
            return Err(format!("Prompt {key} cannot be blank"));
        }
        if prompt.len() > MAX_PROMPT_BYTES {
            return Err(format!("Prompt {key} exceeds the 64 KiB limit"));
        }
        let expected = placeholders(
            defaults
                .prompts
                .get(key)
                .expect("required embedded prompt is missing"),
        );
        let actual = placeholders(prompt);
        if actual != expected {
            return Err(format!(
                "Prompt {key} must preserve placeholders: {}",
                expected.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    Ok(pack)
}

fn parse_and_upgrade(value: &str) -> Result<PromptPack, String> {
    if value.len() > MAX_PACK_BYTES {
        return Err("Prompt pack exceeds the 512 KiB limit".into());
    }
    let mut pack: PromptPack = serde_json::from_str(value)
        .map_err(|error| format!("Prompt pack JSON is invalid: {error}"))?;
    if pack.format != FORMAT || pack.version != VERSION {
        return Err(format!("Expected {FORMAT} version {VERSION}"));
    }
    let defaults = default_pack();
    let required = PromptId::ALL
        .iter()
        .map(|id| id.key())
        .collect::<BTreeSet<_>>();
    if let Some(key) = pack
        .prompts
        .keys()
        .map(String::as_str)
        .find(|key| !required.contains(key))
    {
        return Err(format!("Prompt pack contains unknown prompt {key}"));
    }
    for (key, value) in defaults.prompts {
        pack.prompts.entry(key).or_insert(value);
    }
    parse_and_validate(&serde_json::to_string(&pack).map_err(|error| error.to_string())?)
}

fn placeholders(value: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = value;
    while let Some(start) = rest.find("{{") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("}}") else { break };
        let name = rest[..end].trim();
        if !name.is_empty() {
            found.insert(name.to_string());
        }
        rest = &rest[end + 2..];
    }
    found
}

fn write_recoverable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Prompt pack path has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create prompt pack folder: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)
        .map_err(|error| format!("Could not write prompt pack temporary file: {error}"))?;
    let backup = path.with_extension("json.bak");
    if path.exists() {
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup)
            .map_err(|error| format!("Could not back up the current prompt pack: {error}"))?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(&temporary);
        return Err(format!("Could not replace the prompt pack: {error}"));
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_pack_is_complete_and_prompt_only() {
        let pack = parse_and_validate(DEFAULT_PACK_TEXT).unwrap();
        assert_eq!(pack.prompts.len(), PromptId::ALL.len());
        assert!(pack
            .prompts
            .values()
            .all(|value| !value.contains("C:\\Users\\")));
    }

    #[test]
    fn rejects_missing_and_unknown_prompts() {
        let mut pack = default_pack();
        pack.prompts.remove(PromptId::ChatSystem.key());
        assert!(parse_and_validate(&serde_json::to_string(&pack).unwrap())
            .unwrap_err()
            .contains("missing"));
        let mut pack = default_pack();
        pack.prompts.insert("unknown".into(), "text".into());
        assert!(parse_and_validate(&serde_json::to_string(&pack).unwrap())
            .unwrap_err()
            .contains("unknown"));
    }

    #[test]
    fn rejects_template_edits_that_drop_runtime_placeholders() {
        let mut pack = default_pack();
        pack.prompts.insert(
            PromptId::StudioQualificationUser.key().into(),
            "No nonce here.".into(),
        );
        assert!(parse_and_validate(&serde_json::to_string(&pack).unwrap())
            .unwrap_err()
            .contains("nonce"));
    }

    #[test]
    fn default_text_round_trips_as_a_valid_pack() {
        let text = default_text().unwrap();
        let pack = parse_and_validate(&text).unwrap();
        assert_eq!(pack, default_pack());
    }

    #[test]
    fn older_same_version_packs_gain_new_default_entries() {
        let mut pack = default_pack();
        pack.prompts.remove(PromptId::StudioQualificationTool.key());
        assert_eq!(
            parse_and_upgrade(&serde_json::to_string(&pack).unwrap())
                .unwrap()
                .prompts
                .len(),
            PromptId::ALL.len()
        );
    }
}
