import { invoke, listen, type UnlistenFn } from "./transport";
import type { MovieEditorGenerateRequest, MovieEditorJob, MovieEditorRangeRequest, MovieEditorState } from "../contracts/index";
import { readLegacySpeechPreferences } from "./legacySpeechPreferences";
import type { DesktopCommands, ResearchSpeechPreferences, VadSettings } from "../contracts/index";
import type {
  AppSnapshot,
  ChatSession,
  ChatSessionSummary,
  ChatStart,
  ChatStreamEvent,
  ComputerTaskEvent,
  ComputerTaskRequest,
  ComputerTaskRun,
  ComputerTaskSummary,
  ContextAttachmentImport,
  ControlSettings,
  ControlSnapshot,
  DeveloperRepairReport,
  GpuMemoryProcess,
  OperationProgress,
  ProfileTransfer,
  ResearchProgress,
  ResearchReport,
  LocalSpeechSnapshot,
  CreateVoiceProfileRequest,
  UpdateVoiceProfileRequest,
  VoiceLibrarySnapshot,
  SpeechAlignmentRequest,
  SpeechClip,
  SpeechProgress,
  SpeechSynthesisRequest,
  SpeechTranscription,
  SpeechTranscriptionRequest,
  ResearchSettings,
  ResumeComputerTaskRequest,
  RunResearchRequest,
  StartChatRequest,
  SystemSnapshot,
  SetupInstallRequest,
  SetupLocations,
  SetupProgress,
  SetupSnapshot,
  MovieEdit,
  MovieImageAssetEvent,
  MovieImageAssetGeneration,
  MovieImageAssetRequest,
  MovieProject,
  MovieRenderPreviewEvent,
  MovieRenderState,
  MovieReferenceImport,
  ModelDownloadRecord,
  ModelDownloadRequest,
  ModelDownloadInspection,
  MovieSummary,
  AttachMovieProducerReferencesRequest,
  CreateMovieProducerProjectRequest,
  MovieProducerWorkspace,
  MovieStudioChatEvent,
  MovieStudioChatRequest,
  MovieStudioConversation,
  AcceptMovieStoryRevisionRequest,
  ResetMovieStudioConversationRequest,
  SaveMovieScenesRequest,
  SaveMovieStoryRevisionRequest,
  SummarizeMovieStudioConversationRequest,
  MusicGenerationEvent,
  MusicLyricsDocument,
  MusicLyricsSaveResult,
  MusicMidiDocument,
  MusicMidiSaveResult,
  MusicProject,
  MusicSummary,
  DraftLyricsFromAudioRangeRequest,
  DraftLyricsFromAudioRangeResult,
  TranslateMusicLyricsRequest,
  TranslateMusicLyricsResult,
  RepairMusicLyricsRangeRequest,
  ImageGenerationEvent,
  ImageProject,
  ImageSummary,
  VramCleanupPreview,
  VramCleanupResult,
  PromptDraftEvent,
  PromptDraftRequest,
  ThinkingLevel,
} from "../contracts/index";

export async function getMovieEditorState(id: string): Promise<MovieEditorState> {
  return invoke("get_movie_editor_state", { id });
}

export async function getMovieImageAssetRenderState(requestId: string): Promise<MovieRenderState> {
  return invoke("get_movie_image_asset_render_state", { requestId });
}

export async function prepareMovieEditorRange(request: MovieEditorRangeRequest, edit: MovieEdit): Promise<MovieEditorJob> {
  return invoke("prepare_movie_editor_range", { request, edit });
}

export async function startMovieEditorGeneration(request: MovieEditorGenerateRequest): Promise<string> {
  return invoke("start_movie_editor_generation", { request });
}

export async function onMovieEditorJob(callback: (job: MovieEditorJob) => void): Promise<UnlistenFn> {
  return listen("movie-editor-job", (event) => callback(event.payload));
}

export function getSpeechPreferences() {
  return invoke("get_speech_preferences", { legacy: readLegacySpeechPreferences() });
}
export function saveVadSettings(settings: VadSettings) {
  return invoke("save_vad_settings", { settings });
}
export function saveResearchSpeechPreferences(settings: ResearchSpeechPreferences) {
  return invoke("save_research_speech_preferences", { settings });
}

const isTauri = (): boolean => "__TAURI_INTERNALS__" in window;

export async function bootstrap(): Promise<AppSnapshot> {
  return invoke("bootstrap");
}

export async function getReport(id: string): Promise<ResearchReport> {
  return invoke("get_report", { id });
}

export async function runResearch(
  request: RunResearchRequest,
): Promise<ResearchReport> {
  return invoke("run_research", { request });
}

export async function cancelResearch(jobId: string): Promise<void> {
  await invoke("cancel_research", { jobId });
}

export async function prepareServices(): Promise<AppSnapshot> {
  return invoke("prepare_services");
}

export async function previewVramCleanup(): Promise<VramCleanupPreview> {
  return invoke("preview_vram_cleanup");
}

export async function cleanVram(expectedPids: number[]): Promise<VramCleanupResult> {
  return invoke("clean_vram", { expectedPids });
}

export async function forceCleanVram(expectedProcesses: GpuMemoryProcess[]): Promise<VramCleanupResult> {
  return invoke("force_clean_vram", { expectedProcesses });
}

export async function getSetupSnapshot(): Promise<SetupSnapshot> {
  return invoke("get_setup_snapshot");
}

export async function openComfyUi(workload: "studio" | "music" | "image"): Promise<void> {
  await invoke("open_comfy_ui", { workload });
}

export async function saveSetupLocations(locations: SetupLocations): Promise<AppSnapshot> {
  return invoke("save_setup_locations", { locations });
}

export async function pickSetupFolder(): Promise<string> {
  return invoke("pick_setup_folder");
}

export async function pickSetupFile(kind: string): Promise<string> {
  return invoke("pick_setup_file", { kind });
}

export async function scanSetupModelFolder(path: string): Promise<Record<string, string>> {
  return invoke("scan_setup_model_folder", { path });
}

export async function installSetupComponent(request: SetupInstallRequest): Promise<AppSnapshot> {
  return invoke("install_setup_component", { request });
}

export async function cancelSetupInstall(): Promise<void> {
  await invoke("cancel_setup_install");
}

export async function onSetupProgress(callback: (progress: SetupProgress) => void): Promise<UnlistenFn> {
  return listen("setup-progress", (event) => callback(event.payload));
}

export async function openStandalone(id: string): Promise<void> {
  await invoke("open_standalone_report", { id });
}

export async function revealLibrary(): Promise<void> {
  await invoke("reveal_library");
}

export async function onProgress(
  callback: (progress: ResearchProgress) => void,
): Promise<UnlistenFn> {
  return listen("research-progress", (event) =>
      callback(event.payload),
    );
}

export async function listMovies(): Promise<MovieSummary[]> {
  return invoke("list_movies");
}

export async function getMovie(id: string): Promise<MovieProject> {
  return invoke("get_movie", { id });
}

export async function createMovieProducerProject(request: CreateMovieProducerProjectRequest): Promise<MovieProject> {
  return invoke("create_movie_producer_project", { request });
}

export async function attachMovieProducerReferences(request: AttachMovieProducerReferencesRequest): Promise<MovieProject> {
  return invoke("attach_movie_producer_references", { request });
}

export async function getMovieProducerWorkspace(id: string): Promise<MovieProducerWorkspace> {
  return invoke("get_movie_producer_workspace", { id });
}

export async function getMovieStudioConversation(projectId: string, conversationId: string): Promise<MovieStudioConversation> {
  return invoke("get_movie_studio_conversation", { projectId, conversationId });
}

export async function saveMovieStoryRevision(request: SaveMovieStoryRevisionRequest): Promise<MovieProducerWorkspace> {
  return invoke("save_movie_story_revision", { request });
}

export async function acceptMovieStoryRevision(request: AcceptMovieStoryRevisionRequest): Promise<MovieProducerWorkspace> {
  return invoke("accept_movie_story_revision", { request });
}

export async function saveMovieScenes(request: SaveMovieScenesRequest): Promise<MovieProducerWorkspace> {
  return invoke("save_movie_scenes", { request });
}

export async function renderMovieScenes(id: string): Promise<MovieProject> {
  return invoke("render_movie_scenes", { id });
}

export async function resetMovieStudioConversation(request: ResetMovieStudioConversationRequest): Promise<MovieStudioConversation> {
  return invoke("reset_movie_studio_conversation", { request });
}

export async function summarizeMovieStudioConversation(request: SummarizeMovieStudioConversationRequest): Promise<MovieStudioConversation> {
  return invoke("summarize_movie_studio_conversation", { request });
}

export async function startMovieStudioChat(request: MovieStudioChatRequest): Promise<string> {
  return invoke("start_movie_studio_chat", { request });
}

export async function cancelMovieStudioChat(requestId: string): Promise<void> {
  await invoke("cancel_movie_studio_chat", { requestId });
}

export async function onMovieStudioChat(callback: (event: MovieStudioChatEvent) => void): Promise<UnlistenFn> {
  return listen("movie-studio-chat", (event) => callback(event.payload));
}

export async function onMovieProducerWorkspace(callback: (workspace: MovieProducerWorkspace) => void): Promise<UnlistenFn> {
  return listen("movie-producer-workspace", (event) => callback(event.payload));
}

export async function pickMovieReferenceFiles(): Promise<MovieReferenceImport> {
  return invoke("pick_movie_reference_files");
}

export async function listMovieImageAssets(): Promise<MovieImageAssetGeneration[]> {
  return invoke("list_movie_image_assets");
}

export async function startMovieImageAsset(request: MovieImageAssetRequest): Promise<string> {
  return invoke("start_movie_image_asset", { request });
}

export async function cancelMovieImageAsset(requestId: string): Promise<void> {
  await invoke("cancel_movie_image_asset", { requestId });
}

export async function onMovieImageAsset(callback: (event: MovieImageAssetEvent) => void): Promise<UnlistenFn> {
  return listen("movie-image-asset", (event) => callback(event.payload));
}

export async function onMovieRenderPreview(callback: (event: MovieRenderPreviewEvent) => void): Promise<UnlistenFn> {
  return listen("movie-render-preview", (event) => callback(event.payload));
}

export async function listMusicProjects(): Promise<MusicSummary[]> {
  return invoke("list_music_projects");
}

export async function getMusicProject(id: string): Promise<MusicProject> {
  return invoke("get_music_project", { id });
}

export async function createMusicProject(request: DesktopCommands["create_music_project"]["args"]["request"]): Promise<MusicProject> {
  return invoke("create_music_project", { request });
}

export async function saveMusicProject(project: DesktopCommands["save_music_project"]["args"]["project"]): Promise<MusicProject> {
  return invoke("save_music_project", { project });
}

export async function startMusicGeneration(id: string): Promise<MusicProject> {
  return invoke("start_music_generation", { id });
}

export async function cancelMusicGeneration(id: string): Promise<void> {
  await invoke("cancel_music_generation", { id });
}

export async function transcribeMusicMidi(projectId: string, takeId: string): Promise<MusicProject> {
  return invoke("transcribe_music_midi", { request: { projectId, takeId } });
}

export async function revealMusicProject(id: string): Promise<void> {
  await invoke("reveal_music_project", { id });
}

export async function createMusicLyricsDraft(projectId: string, takeId: string): Promise<MusicLyricsSaveResult> {
  return invoke("create_music_lyrics_draft", { request: { projectId, takeId } });
}

export async function getMusicLyricsDocument(projectId: string, takeId: string): Promise<MusicLyricsSaveResult> {
  return invoke("get_music_lyrics_document", { request: { projectId, takeId } });
}

export async function saveMusicLyricsDocument(projectId: string, takeId: string, document: MusicLyricsDocument): Promise<MusicLyricsSaveResult> {
  return invoke("save_music_lyrics_document", { request: { projectId, takeId, document } });
}

export async function transcribeMusicLyrics(request: DesktopCommands["transcribe_music_lyrics"]["args"]["request"]): Promise<MusicLyricsSaveResult> {
  return invoke("transcribe_music_lyrics", { request });
}

export async function repairMusicLyricsRange(request: RepairMusicLyricsRangeRequest): Promise<MusicLyricsSaveResult> {
  return invoke("repair_music_lyrics_range", { request });
}

export async function draftLyricsFromAudioRange(request: DraftLyricsFromAudioRangeRequest): Promise<DraftLyricsFromAudioRangeResult> {
  return invoke("draft_lyrics_from_audio_range", { request });
}

export async function translateMusicLyrics(request: TranslateMusicLyricsRequest): Promise<TranslateMusicLyricsResult> {
  return invoke("translate_music_lyrics", { request });
}

export async function onMusicGeneration(callback: (event: MusicGenerationEvent) => void): Promise<UnlistenFn> {
  return listen("music-generation", (event) => callback(event.payload));
}

export async function onMusicProjectUpdated(callback: (project: MusicProject) => void): Promise<UnlistenFn> {
  return listen("music-project-updated", (event) => callback(event.payload));
}

export async function listImageProjects(): Promise<ImageSummary[]> {
  return invoke("list_image_projects");
}

export async function getImageProject(id: string): Promise<ImageProject> {
  return invoke("get_image_project", { id });
}

export async function createImageProject(request: DesktopCommands["create_image_project"]["args"]["request"]): Promise<ImageProject> {
  return invoke("create_image_project", { request });
}

export async function saveImageProject(project: DesktopCommands["save_image_project"]["args"]["project"]): Promise<ImageProject> {
  return invoke("save_image_project", { project });
}

export async function startImageGeneration(id: string): Promise<ImageProject> {
  return invoke("start_image_generation", { id });
}

export async function cancelImageGeneration(id: string): Promise<void> {
  await invoke("cancel_image_generation", { id });
}

export async function revealImageProject(id: string): Promise<void> {
  await invoke("reveal_image_project", { id });
}

export async function onImageGeneration(callback: (event: ImageGenerationEvent) => void): Promise<UnlistenFn> {
  return listen("image-generation", (event) => callback(event.payload));
}

export async function onImageProjectUpdated(callback: (project: ImageProject) => void): Promise<UnlistenFn> {
  return listen("image-project-updated", (event) => callback(event.payload));
}

export async function getLocalSpeechSnapshot(): Promise<LocalSpeechSnapshot> {
  return invoke("get_local_speech_snapshot");
}

export async function getVoiceLibrary(): Promise<VoiceLibrarySnapshot> {
  return invoke("get_voice_library");
}

export async function createVoiceProfile(request: CreateVoiceProfileRequest): Promise<VoiceLibrarySnapshot> {
  return invoke("create_voice_profile", { request });
}

export async function updateVoiceProfile(request: UpdateVoiceProfileRequest): Promise<VoiceLibrarySnapshot> {
  return invoke("update_voice_profile", { request });
}

export async function setDefaultVoiceProfile(profileId: string): Promise<VoiceLibrarySnapshot> {
  return invoke("set_default_voice_profile", { profileId });
}

export async function deleteVoiceProfile(profileId: string): Promise<VoiceLibrarySnapshot> {
  return invoke("delete_voice_profile", { profileId });
}

export async function prepareLocalSpeech(): Promise<LocalSpeechSnapshot> {
  return invoke("prepare_local_speech");
}

export async function synthesizeLocalSpeech(request: SpeechSynthesisRequest): Promise<SpeechClip> {
  return invoke("synthesize_local_speech", { request });
}

export async function getCachedLocalSpeechClip(request: SpeechSynthesisRequest): Promise<SpeechClip | null> {
  return invoke("get_cached_local_speech_clip", { request });
}

export async function alignLocalSpeech(request: SpeechAlignmentRequest): Promise<SpeechClip> {
  return invoke("align_local_speech", { request });
}

export async function transcribeLocalSpeech(request: SpeechTranscriptionRequest): Promise<SpeechTranscription> {
  return invoke("transcribe_local_speech", { request });
}

export async function cancelLocalSpeech(jobId: string): Promise<void> {
  await invoke("cancel_local_speech", { jobId });
}

export async function releaseLocalSpeechMemory(): Promise<void> {
  await invoke("release_local_speech_memory");
}

export async function onLocalSpeechProgress(
  callback: (progress: SpeechProgress) => void,
): Promise<UnlistenFn> {
  return listen("local-speech-progress", (event) => callback(event.payload));
}

export function localSpeechMediaUrl(relativePath: string): string {
  if (!relativePath || !isTauri()) return "";
  return `http://kestrel-speech.localhost/${relativePath.split("/").map(encodeURIComponent).join("/")}`;
}

export async function startStudioPromptDraft(request: PromptDraftRequest): Promise<string> {
  return invoke("start_studio_prompt_draft", { request });
}

export async function cancelStudioPromptDraft(requestId: string): Promise<void> {
  await invoke("cancel_studio_prompt_draft", { requestId });
}

export async function onStudioPromptDraft(callback: (event: PromptDraftEvent) => void): Promise<UnlistenFn> {
  return listen("studio-prompt-draft", (event) => callback(event.payload));
}

export async function getMusicMidiDocument(projectId: string, takeId: string): Promise<MusicMidiSaveResult> {
  return invoke("get_music_midi_document", { request: { projectId, takeId } });
}

export async function saveMusicMidiDocument(projectId: string, takeId: string, document: MusicMidiDocument): Promise<MusicMidiSaveResult> {
  return invoke("save_music_midi_document", { request: { projectId, takeId, document } });
}

export async function exportMusicMidi(projectId: string, takeId: string): Promise<string | undefined> {
  return (await invoke("export_music_midi", { request: { projectId, takeId } })) ?? undefined;
}

export async function revealMusicMidi(projectId: string, takeId: string): Promise<void> {
  await invoke("reveal_music_midi", { request: { projectId, takeId } });
}

export async function getMovieRenderState(id: string): Promise<MovieRenderState> {
  return invoke("get_movie_render_state", { id });
}

export async function cancelMovieRender(id: string): Promise<void> {
  await invoke("cancel_movie_render", { id });
}

export async function saveMovieEdits(id: string, edit: MovieEdit): Promise<MovieProject> {
  return invoke("save_movie_edits", { id, edit });
}

export async function renderMovieEdit(id: string): Promise<MovieProject> {
  return invoke("render_movie_edit", { id });
}

export async function revealMovie(id: string): Promise<void> {
  await invoke("reveal_movie", { id });
}

export async function onMovieProject(callback: (project: MovieProject) => void): Promise<UnlistenFn> {
  return listen("movie-project", (event) => callback(event.payload));
}

export function movieMediaUrl(path: string): string {
  if (!path || !isTauri()) return "";
  const normalized = path.replaceAll("\\", "/");
  const marker = "/movies/";
  const offset = normalized.toLowerCase().lastIndexOf(marker);
  if (offset < 0) return "";
  const relative = normalized.slice(offset + marker.length);
  return `http://kestrel-media.localhost/${relative.split("/").map(encodeURIComponent).join("/")}`;
}

export function musicMediaUrl(path: string): string {
  if (!path || !isTauri()) return "";
  const normalized = path.replaceAll("\\", "/");
  const marker = "/music/";
  const offset = normalized.toLowerCase().lastIndexOf(marker);
  if (offset < 0) return "";
  const relative = normalized.slice(offset + marker.length);
  return `http://kestrel-media.localhost/music/${relative.split("/").map(encodeURIComponent).join("/")}`;
}

export function imageMediaUrl(path: string, download = false): string {
  if (!path || !isTauri()) return "";
  const normalized = path.replaceAll("\\", "/");
  const marker = "/images/";
  const offset = normalized.toLowerCase().lastIndexOf(marker);
  if (offset < 0) return "";
  const relative = normalized.slice(offset + marker.length);
  const url = `http://kestrel-media.localhost/images/${relative.split("/").map(encodeURIComponent).join("/")}`;
  return download ? `${url}?download=1` : url;
}

export async function onRuntimeProgress(
  callback: (progress: OperationProgress) => void,
): Promise<UnlistenFn> {
  return listen("runtime-progress", (event) =>
      callback(event.payload),
    );
}

export async function onDeveloperProgress(
  callback: (progress: OperationProgress) => void,
): Promise<UnlistenFn> {
  return listen("developer-progress", (event) =>
      callback(event.payload),
    );
}

export async function getSystemSnapshot(): Promise<SystemSnapshot> {
  return invoke("get_system_snapshot");
}

export async function saveResearchSettings(
  settings: ResearchSettings,
): Promise<ResearchSettings> {
  return invoke("save_research_settings", { settings });
}

export async function applyModelRuntime(
  settings: ControlSettings,
): Promise<SystemSnapshot> {
  return invoke("apply_model_runtime", { settings });
}

export async function getControlSnapshot(
  probeDeveloper = true,
): Promise<ControlSnapshot> {
  return invoke("get_control_snapshot", { probeDeveloper });
}

export async function scanLocalModels(): Promise<ControlSnapshot> {
  return invoke("scan_local_models");
}

export async function listModelDownloads(): Promise<ModelDownloadRecord[]> {
  return invoke("list_model_downloads");
}

export async function inspectModelDownload(url: string): Promise<ModelDownloadInspection> {
  return invoke("inspect_model_download", { url });
}

export async function startModelDownload(request: ModelDownloadRequest): Promise<ModelDownloadRecord> {
  return invoke("start_model_download", { request });
}

export async function resumeModelDownload(id: string): Promise<ModelDownloadRecord> {
  return invoke("resume_model_download", { id });
}

export async function cancelModelDownload(): Promise<void> {
  await invoke("cancel_model_download");
}

export async function onModelDownload(callback: (record: ModelDownloadRecord) => void): Promise<UnlistenFn> {
  return listen("model-download", (event) => callback(event.payload));
}

export async function exportSetupProfile(): Promise<ProfileTransfer> {
  return invoke("export_setup_profile");
}

export async function getSetupProfileText(): Promise<string> {
  return invoke("get_setup_profile_text");
}

export async function getPromptPackText(): Promise<string> {
  return invoke("get_prompt_pack_text");
}

export async function getDefaultPromptPackText(): Promise<string> {
  return invoke("get_default_prompt_pack_text");
}

export async function savePromptPackText(text: string): Promise<string> {
  return invoke("save_prompt_pack_text", { text });
}

export async function resetPromptPack(): Promise<string> {
  return invoke("reset_prompt_pack");
}

export async function exportPromptPackText(text: string): Promise<ProfileTransfer> {
  return invoke("export_prompt_pack_text", { text });
}

export async function importPromptPack(path: string): Promise<string> {
  return invoke("import_prompt_pack", { path });
}

export async function pickPromptPackFile(): Promise<string | undefined> {
  return (await invoke("pick_prompt_pack_file")) ?? undefined;
}

export async function exportSetupProfileText(text: string): Promise<ProfileTransfer> {
  return invoke("export_setup_profile_text", { text });
}

export async function importSetupProfile(path: string): Promise<AppSnapshot> {
  return invoke("import_setup_profile", { path });
}

export async function importSetupProfileText(text: string): Promise<AppSnapshot> {
  return invoke("import_setup_profile_text", { text });
}

export async function saveControlSettings(
  settings: ControlSettings,
): Promise<ControlSnapshot> {
  return invoke("save_control_settings", { settings });
}

export async function startLocalModel(
  modelId: string,
): Promise<ControlSnapshot> {
  return invoke("start_local_model", { modelId });
}

export async function stopLocalModel(): Promise<ControlSnapshot> {
  return invoke("stop_local_model");
}

export async function releaseAiMemory(): Promise<ControlSnapshot> {
  return invoke("release_ai_memory");
}

export async function listChatSessions(): Promise<ChatSessionSummary[]> {
  return invoke("list_chat_sessions");
}

export async function getChatSession(id: string): Promise<ChatSession> {
  return invoke("get_chat_session", { id });
}

export async function deleteChatSession(id: string): Promise<void> {
  await invoke("delete_chat_session", { id });
}

export async function pickContextFiles(): Promise<ContextAttachmentImport> {
  return invoke("pick_context_files");
}

export async function openContextAttachment(id: string): Promise<void> {
  await invoke("open_context_attachment", { id });
}

export async function pickLocalModelFolder(): Promise<string | undefined> {
  return (await invoke("pick_local_model_folder")) ?? undefined;
}

export async function startChatStream(
  request: StartChatRequest,
): Promise<ChatStart> {
  return invoke("start_chat_stream", { request });
}

export async function cancelChatStream(requestId: string): Promise<void> {
  await invoke("cancel_chat_stream", { requestId });
}

export async function onChatStream(
  callback: (event: ChatStreamEvent) => void,
): Promise<UnlistenFn> {
  return listen("chat-stream", (event) =>
      callback(event.payload),
    );
}

export async function listComputerTasks(): Promise<ComputerTaskSummary[]> {
  return invoke("list_computer_tasks");
}

export async function getComputerTask(id: string): Promise<ComputerTaskRun> {
  return invoke("get_computer_task", { id });
}

export async function startComputerTask(
  request: ComputerTaskRequest,
): Promise<ComputerTaskRun> {
  return invoke("start_computer_task", { request });
}

export async function resumeComputerTask(
  request: ResumeComputerTaskRequest,
): Promise<ComputerTaskRun> {
  return invoke("resume_computer_task", { request });
}

export async function stopComputerTask(runId: string): Promise<void> {
  await invoke("stop_computer_task", { runId });
}

export async function onComputerTaskEvent(
  callback: (event: ComputerTaskEvent) => void,
): Promise<UnlistenFn> {
  return listen("computer-task-event", (event) =>
      callback(event.payload),
    );
}

export async function openTaskArtifact(
  runId: string,
  path: string,
): Promise<void> {
  await invoke("open_task_artifact", { runId, path });
}

export async function runNativeDiagnostics(): Promise<string> {
  return invoke("run_native_diagnostics");
}

export async function runCodexRepair(
  issue: string,
): Promise<DeveloperRepairReport> {
  return invoke("run_codex_repair", {
    request: { issue },
  });
}
