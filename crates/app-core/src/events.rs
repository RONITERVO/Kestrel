//! The single event registry. Native emission and generated UI subscriptions share this table.
use serde::Serialize;
use ts_rs::TS;

pub trait DesktopEvent {
    type Payload: Serialize + Clone + TS;
    const NAME: &'static str;
}

macro_rules! events {
    ($($name:ident: $payload:ty => $wire:literal),+ $(,)?) => {
        $(pub struct $name;
        impl DesktopEvent for $name {
            type Payload = $payload;
            const NAME: &'static str = $wire;
        })+
        pub fn bindings() -> serde_json::Value {
            serde_json::json!([$( {"name": $wire, "type": <$payload as TS>::name(&ts_rs::Config::from_env())} ),+])
        }
    };
}

events! {
    ChatStream: crate::ChatStreamEvent => "chat-stream",
    ComputerTask: crate::ComputerTaskEvent => "computer-task-event",
    DeveloperProgress: crate::OperationProgress => "developer-progress",
    ImageGeneration: crate::ImageGenerationEvent => "image-generation",
    ImageProject: crate::ImageProject => "image-project-updated",
    LocalSpeechProgress: crate::SpeechProgress => "local-speech-progress",
    ModelCatalog: Vec<crate::ModelInfo> => "model-catalog-updated",
    ModelDownload: crate::ModelDownloadRecord => "model-download",
    MovieEditorJob: crate::MovieEditorJob => "movie-editor-job",
    MovieImageAsset: crate::MovieImageAssetEvent => "movie-image-asset",
    MovieProducerWorkspace: crate::MovieProducerWorkspace => "movie-producer-workspace",
    MovieProject: crate::MovieProject => "movie-project",
    MovieRenderPreview: crate::MovieRenderPreviewEvent => "movie-render-preview",
    MovieStudioChat: crate::MovieStudioChatEvent => "movie-studio-chat",
    MusicGeneration: crate::MusicGenerationEvent => "music-generation",
    MusicProject: crate::MusicProject => "music-project-updated",
    ResearchProgress: crate::ResearchProgress => "research-progress",
    RuntimeLog: crate::RuntimeLog => "runtime-log",
    RuntimeProgress: crate::OperationProgress => "runtime-progress",
    SetupProgress: crate::SetupProgress => "setup-progress",
    StudioPromptDraft: crate::PromptDraftEvent => "studio-prompt-draft",
}
