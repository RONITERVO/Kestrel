import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { demoReport, demoSnapshot } from "./demo";
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
  OperationProgress,
  ProfileTransfer,
  ResearchProgress,
  ResearchReport,
  LocalSpeechSnapshot,
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
  MovieClipRenderRequest,
  MovieClipSuggestion,
  MovieCopilotEvent,
  MovieCopilotReceipt,
  MovieCopilotRequest,
  MovieEdit,
  MovieImageAssetEvent,
  MovieImageAssetGeneration,
  MovieImageAssetRequest,
  ModelCompatibility,
  MoviePlan,
  MovieModelRoleRequest,
  MoviePlanningEvent,
  MoviePlanningSnapshot,
  MovieProject,
  MovieRenderPreviewEvent,
  MovieReferenceImport,
  ModelDownloadRecord,
  ModelDownloadRequest,
  ModelDownloadInspection,
  MovieSummary,
  MusicGenerationEvent,
  MusicProject,
  MusicSummary,
  ImageGenerationEvent,
  ImageProject,
  ImageSummary,
  StartMovieRequest,
  PromptDraftEvent,
  PromptDraftRequest,
} from "./types";

const isTauri = (): boolean => "__TAURI_INTERNALS__" in window;

export async function bootstrap(): Promise<AppSnapshot> {
  if (!isTauri()) return demoSnapshot;
  return invoke<AppSnapshot>("bootstrap");
}

export async function getReport(id: string): Promise<ResearchReport> {
  if (!isTauri()) return demoReport;
  return invoke<ResearchReport>("get_report", { id });
}

export async function runResearch(
  request: RunResearchRequest,
): Promise<ResearchReport> {
  if (!isTauri()) {
    await new Promise((resolve) => window.setTimeout(resolve, 900));
    return {
      ...demoReport,
      query: request.query,
      title: request.query,
      id: `demo-${Date.now()}`,
    };
  }
  return invoke<ResearchReport>("run_research", { request });
}

export async function cancelResearch(jobId: string): Promise<void> {
  if (isTauri()) await invoke("cancel_research", { jobId });
}

export async function prepareServices(): Promise<AppSnapshot> {
  if (!isTauri()) return demoSnapshot;
  return invoke<AppSnapshot>("prepare_services");
}

export async function getSetupSnapshot(): Promise<SetupSnapshot> {
  if (!isTauri()) return demoSnapshot.setup;
  return invoke<SetupSnapshot>("get_setup_snapshot");
}

export async function openComfyUi(workload: "studio" | "music" | "image"): Promise<void> {
  if (isTauri()) await invoke("open_comfy_ui", { workload });
}

export async function saveSetupLocations(locations: SetupLocations): Promise<AppSnapshot> {
  if (!isTauri()) return demoSnapshot;
  return invoke<AppSnapshot>("save_setup_locations", { locations });
}

export async function pickSetupFolder(): Promise<string> {
  if (!isTauri()) return "";
  return invoke<string>("pick_setup_folder");
}

export async function pickSetupFile(kind: string): Promise<string> {
  if (!isTauri()) return "";
  return invoke<string>("pick_setup_file", { kind });
}

export async function installSetupComponent(request: SetupInstallRequest): Promise<AppSnapshot> {
  if (!isTauri()) return demoSnapshot;
  return invoke<AppSnapshot>("install_setup_component", { request });
}

export async function cancelSetupInstall(): Promise<void> {
  if (isTauri()) await invoke("cancel_setup_install");
}

export async function onSetupProgress(callback: (progress: SetupProgress) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<SetupProgress>("setup-progress", (event) => callback(event.payload));
  return () => undefined;
}

export async function openStandalone(id: string): Promise<void> {
  if (isTauri()) await invoke("open_standalone_report", { id });
}

export async function revealLibrary(): Promise<void> {
  if (isTauri()) await invoke("reveal_library");
}

export async function onProgress(
  callback: (progress: ResearchProgress) => void,
): Promise<UnlistenFn> {
  if (isTauri())
    return listen<ResearchProgress>("research-progress", (event) =>
      callback(event.payload),
    );
  return () => undefined;
}

export async function listMovies(): Promise<MovieSummary[]> {
  if (!isTauri()) return [];
  return invoke<MovieSummary[]>("list_movies");
}

export async function getMovie(id: string): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Movie projects require the desktop application.");
  return invoke<MovieProject>("get_movie", { id });
}

export async function pickMovieReferenceFiles(): Promise<MovieReferenceImport> {
  if (!isTauri()) return { references: [], failures: [] };
  return invoke<MovieReferenceImport>("pick_movie_reference_files");
}

export async function listMovieImageAssets(): Promise<MovieImageAssetGeneration[]> {
  if (!isTauri()) return [];
  return invoke<MovieImageAssetGeneration[]>("list_movie_image_assets");
}

export async function startMovieImageAsset(request: MovieImageAssetRequest): Promise<string> {
  if (!isTauri()) throw new Error("Local H3 image generation requires the desktop application.");
  return invoke<string>("start_movie_image_asset", { request });
}

export async function cancelMovieImageAsset(requestId: string): Promise<void> {
  if (isTauri()) await invoke("cancel_movie_image_asset", { requestId });
}

export async function onMovieImageAsset(callback: (event: MovieImageAssetEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<MovieImageAssetEvent>("movie-image-asset", (event) => callback(event.payload));
  return () => undefined;
}

export async function onMovieRenderPreview(callback: (event: MovieRenderPreviewEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<MovieRenderPreviewEvent>("movie-render-preview", (event) => callback(event.payload));
  return () => undefined;
}

export async function startMovie(request: StartMovieRequest): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Movie production requires the desktop application.");
  return invoke<MovieProject>("start_movie", { request });
}

export async function listMusicProjects(): Promise<MusicSummary[]> {
  if (!isTauri()) return [];
  return invoke<MusicSummary[]>("list_music_projects");
}

export async function getMusicProject(id: string): Promise<MusicProject> {
  if (!isTauri()) throw new Error("Music projects require the desktop application.");
  return invoke<MusicProject>("get_music_project", { id });
}

export async function createMusicProject(request: { title: string; idea: string; comfyRoot: string }): Promise<MusicProject> {
  if (!isTauri()) throw new Error("Music projects require the desktop application.");
  return invoke<MusicProject>("create_music_project", { request });
}

export async function saveMusicProject(project: MusicProject): Promise<MusicProject> {
  if (!isTauri()) throw new Error("Music projects require the desktop application.");
  return invoke<MusicProject>("save_music_project", { project });
}

export async function startMusicGeneration(id: string): Promise<MusicProject> {
  if (!isTauri()) throw new Error("Local music generation requires the desktop application.");
  return invoke<MusicProject>("start_music_generation", { id });
}

export async function cancelMusicGeneration(id: string): Promise<void> {
  if (isTauri()) await invoke("cancel_music_generation", { id });
}

export async function transcribeMusicMidi(projectId: string, takeId: string): Promise<MusicProject> {
  if (!isTauri()) throw new Error("MuScriptor transcription requires the desktop application.");
  return invoke<MusicProject>("transcribe_music_midi", { request: { projectId, takeId } });
}

export async function revealMusicProject(id: string): Promise<void> {
  if (isTauri()) await invoke("reveal_music_project", { id });
}

export async function onMusicGeneration(callback: (event: MusicGenerationEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<MusicGenerationEvent>("music-generation", (event) => callback(event.payload));
  return () => undefined;
}

export async function onMusicProjectUpdated(callback: (project: MusicProject) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<MusicProject>("music-project-updated", (event) => callback(event.payload));
  return () => undefined;
}

export async function listImageProjects(): Promise<ImageSummary[]> {
  if (!isTauri()) return [];
  return invoke<ImageSummary[]>("list_image_projects");
}

export async function getImageProject(id: string): Promise<ImageProject> {
  if (!isTauri()) throw new Error("Image projects require the desktop application.");
  return invoke<ImageProject>("get_image_project", { id });
}

export async function createImageProject(request: { title: string; idea: string; comfyRoot: string }): Promise<ImageProject> {
  if (!isTauri()) throw new Error("Image projects require the desktop application.");
  return invoke<ImageProject>("create_image_project", { request });
}

export async function saveImageProject(project: ImageProject): Promise<ImageProject> {
  if (!isTauri()) throw new Error("Image projects require the desktop application.");
  return invoke<ImageProject>("save_image_project", { project });
}

export async function startImageGeneration(id: string): Promise<ImageProject> {
  if (!isTauri()) throw new Error("Local Ideogram image generation requires the desktop application.");
  return invoke<ImageProject>("start_image_generation", { id });
}

export async function cancelImageGeneration(id: string): Promise<void> {
  if (isTauri()) await invoke("cancel_image_generation", { id });
}

export async function revealImageProject(id: string): Promise<void> {
  if (isTauri()) await invoke("reveal_image_project", { id });
}

export async function onImageGeneration(callback: (event: ImageGenerationEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<ImageGenerationEvent>("image-generation", (event) => callback(event.payload));
  return () => undefined;
}

export async function onImageProjectUpdated(callback: (project: ImageProject) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<ImageProject>("image-project-updated", (event) => callback(event.payload));
  return () => undefined;
}

export async function getLocalSpeechSnapshot(): Promise<LocalSpeechSnapshot> {
  if (!isTauri()) return {
    narrationAvailable: false,
    transcriptionAvailable: false,
    comfyReady: false,
    voices: [],
    transcribers: [],
    detail: "Local speech uses the user's ComfyUI voice and Whisper models in the desktop app.",
  };
  return invoke<LocalSpeechSnapshot>("get_local_speech_snapshot");
}

export async function prepareLocalSpeech(): Promise<LocalSpeechSnapshot> {
  if (!isTauri()) return getLocalSpeechSnapshot();
  return invoke<LocalSpeechSnapshot>("prepare_local_speech");
}

export async function synthesizeLocalSpeech(request: SpeechSynthesisRequest): Promise<SpeechClip> {
  if (!isTauri()) throw new Error("ComfyUI local narration requires the desktop application.");
  return invoke<SpeechClip>("synthesize_local_speech", { request });
}

export async function alignLocalSpeech(request: SpeechAlignmentRequest): Promise<SpeechClip> {
  if (!isTauri()) throw new Error("ComfyUI local speech alignment requires the desktop application.");
  return invoke<SpeechClip>("align_local_speech", { request });
}

export async function transcribeLocalSpeech(request: SpeechTranscriptionRequest): Promise<SpeechTranscription> {
  if (!isTauri()) throw new Error("ComfyUI Whisper dictation requires the desktop application.");
  return invoke<SpeechTranscription>("transcribe_local_speech", { request });
}

export async function cancelLocalSpeech(jobId: string): Promise<void> {
  if (isTauri()) await invoke("cancel_local_speech", { jobId });
}

export async function releaseLocalSpeechMemory(): Promise<void> {
  if (isTauri()) await invoke("release_local_speech_memory");
}

export async function onLocalSpeechProgress(
  callback: (progress: SpeechProgress) => void,
): Promise<UnlistenFn> {
  if (isTauri()) return listen<SpeechProgress>("local-speech-progress", (event) => callback(event.payload));
  return () => undefined;
}

export function localSpeechMediaUrl(relativePath: string): string {
  if (!relativePath || !isTauri()) return "";
  return `http://kestrel-speech.localhost/${relativePath.split("/").map(encodeURIComponent).join("/")}`;
}

export async function listStudioModelCompatibility(): Promise<ModelCompatibility[]> {
  if (!isTauri()) return [];
  return invoke<ModelCompatibility[]>("list_studio_model_compatibility");
}

export async function qualifyStudioModel(modelId: string): Promise<ModelCompatibility> {
  if (!isTauri()) throw new Error("Studio model checks require the desktop application.");
  return invoke<ModelCompatibility>("qualify_studio_model", { modelId });
}

export async function startManualMovie(request: StartMovieRequest): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Movie production requires the desktop application.");
  return invoke<MovieProject>("start_manual_movie", { request });
}

export async function resumeMovie(id: string): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Movie production requires the desktop application.");
  return invoke<MovieProject>("resume_movie", { id });
}

export async function cancelMovie(id: string): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Movie production requires the desktop application.");
  return invoke<MovieProject>("cancel_movie", { id });
}

export async function startMoviePromptDraft(request: PromptDraftRequest): Promise<string> {
  if (!isTauri()) throw new Error("Local prompt collaboration requires the desktop application.");
  return invoke<string>("start_movie_prompt_draft", { request });
}

export async function cancelMoviePromptDraft(requestId: string): Promise<void> {
  if (isTauri()) await invoke("cancel_movie_prompt_draft", { requestId });
}

export async function onMoviePromptDraft(callback: (event: PromptDraftEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<PromptDraftEvent>("movie-prompt-draft", (event) => callback(event.payload));
  return () => undefined;
}

export async function startMovieCopilot(request: MovieCopilotRequest): Promise<string> {
  if (!isTauri()) throw new Error("The producer copilot requires the desktop application.");
  return invoke<string>("start_movie_copilot", { request });
}

export async function cancelMovieCopilot(requestId: string): Promise<void> {
  if (isTauri()) await invoke("cancel_movie_copilot", { requestId });
}

export async function onMovieCopilot(callback: (event: MovieCopilotEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<MovieCopilotEvent>("movie-copilot", (event) => callback(event.payload));
  return () => undefined;
}

export async function getMovieCopilotReceipt(projectId: string, requestId: string): Promise<MovieCopilotReceipt> {
  if (!isTauri()) throw new Error("Copilot audit inspection requires the desktop application.");
  return invoke<MovieCopilotReceipt>("get_movie_copilot_receipt", { projectId, requestId });
}

export async function getMoviePlanning(id: string): Promise<MoviePlanningSnapshot> {
  if (!isTauri()) throw new Error("Movie planning requires the desktop application.");
  return invoke<MoviePlanningSnapshot>("get_movie_planning", { id });
}

export async function directMoviePlanning(id: string, text: string): Promise<MoviePlanningSnapshot> {
  if (!isTauri()) throw new Error("Live producer direction requires the desktop application.");
  return invoke<MoviePlanningSnapshot>("direct_movie_planning", { id, text });
}

export async function checkpointMoviePlanning(id: string): Promise<MoviePlanningSnapshot> {
  if (!isTauri()) throw new Error("Planning checkpoints require the desktop application.");
  return invoke<MoviePlanningSnapshot>("checkpoint_movie_planning", { id });
}

export async function saveMoviePlan(id: string, plan: MoviePlan): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Producer plan editing requires the desktop application.");
  return invoke<MovieProject>("save_movie_plan", { id, plan });
}

export async function reviseMoviePlan(id: string, feedback: string): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Studio plan revision requires the desktop application.");
  return invoke<MovieProject>("revise_movie_plan", { request: { id, feedback } });
}

export async function approveMoviePlan(id: string): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Producer approval requires the desktop application.");
  return invoke<MovieProject>("approve_movie_plan", { id });
}

export async function askMovieDirectorClip(id: string, clipId: string, feedback: string): Promise<MovieClipSuggestion> {
  if (!isTauri()) throw new Error("Studio scene assistance requires the desktop application.");
  return invoke<MovieClipSuggestion>("ask_movie_director_clip", { request: { id, clipId, feedback } });
}

export async function setMovieModelRoles(id: string, modelRoles: MovieModelRoleRequest): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Studio model-role changes require the desktop application.");
  return invoke<MovieProject>("set_movie_model_roles", { id, modelRoles });
}

export async function renderMovieClipVersion(request: MovieClipRenderRequest): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Scene version rendering requires the desktop application.");
  return invoke<MovieProject>("render_movie_clip_version", { request });
}

export async function saveMovieEdits(id: string, edit: MovieEdit): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Movie editing requires the desktop application.");
  return invoke<MovieProject>("save_movie_edits", { id, edit });
}

export async function renderMovieEdit(id: string): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Movie editing requires the desktop application.");
  return invoke<MovieProject>("render_movie_edit", { id });
}

export async function revealMovie(id: string): Promise<void> {
  if (isTauri()) await invoke("reveal_movie", { id });
}

export async function onMovieProject(callback: (project: MovieProject) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<MovieProject>("movie-project", (event) => callback(event.payload));
  return () => undefined;
}

export async function getMoviePlanExchangePrompt(id: string): Promise<string> {
  if (!isTauri()) throw new Error("External plan exchange requires the desktop application.");
  return invoke<string>("get_movie_plan_exchange_prompt", { id });
}

export async function parseMoviePlanExchange(id: string, text: string): Promise<MoviePlan> {
  if (!isTauri()) throw new Error("External plan exchange requires the desktop application.");
  return invoke<MoviePlan>("parse_movie_plan_exchange", { id, text });
}

export async function onMoviePlanning(callback: (event: MoviePlanningEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<MoviePlanningEvent>("movie-planning", (event) => callback(event.payload));
  return () => undefined;
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
  if (isTauri())
    return listen<OperationProgress>("runtime-progress", (event) =>
      callback(event.payload),
    );
  return () => undefined;
}

export async function onDeveloperProgress(
  callback: (progress: OperationProgress) => void,
): Promise<UnlistenFn> {
  if (isTauri())
    return listen<OperationProgress>("developer-progress", (event) =>
      callback(event.payload),
    );
  return () => undefined;
}

export async function getSystemSnapshot(): Promise<SystemSnapshot> {
  if (!isTauri()) {
    return {
      status: demoSnapshot.status,
      settings: demoSnapshot.settings,
      runtime: {
        contextWindow: 98_304,
        maxOutputTokens: 32_768,
        parallelSlots: 1,
        kvCache: "q4_0 / q4_0",
        modelVramMib: 9_964,
        modelRoot: "D:\\LocalAI\\Bonsai27B",
      },
      gpu: {
        name: "NVIDIA GeForce RTX 5070",
        totalMib: 12_227,
        usedMib: 11_128,
        freeMib: 816,
        utilizationPercent: 7,
      },
    };
  }
  return invoke<SystemSnapshot>("get_system_snapshot");
}

export async function saveResearchSettings(
  settings: ResearchSettings,
): Promise<ResearchSettings> {
  if (!isTauri()) return settings;
  return invoke<ResearchSettings>("save_research_settings", { settings });
}

export async function applyModelRuntime(
  settings: ResearchSettings,
): Promise<SystemSnapshot> {
  if (!isTauri()) return getSystemSnapshot();
  return invoke<SystemSnapshot>("apply_model_runtime", { settings });
}

export async function openBonsaiControlCenter(): Promise<void> {
  if (isTauri()) await invoke("open_bonsai_control_center");
}

export async function getControlSnapshot(
  probeDeveloper = true,
): Promise<ControlSnapshot> {
  if (!isTauri()) return demoSnapshot.control;
  return invoke<ControlSnapshot>("get_control_snapshot", { probeDeveloper });
}

export async function scanLocalModels(): Promise<ControlSnapshot> {
  if (!isTauri()) return demoSnapshot.control;
  return invoke<ControlSnapshot>("scan_local_models");
}

export async function listModelDownloads(): Promise<ModelDownloadRecord[]> {
  if (!isTauri()) return [];
  return invoke<ModelDownloadRecord[]>("list_model_downloads");
}

export async function inspectModelDownload(url: string): Promise<ModelDownloadInspection> {
  if (!isTauri()) return { repository: "preview/model", revision: "main", candidates: [], detail: "Desktop inspection only." };
  return invoke<ModelDownloadInspection>("inspect_model_download", { url });
}

export async function startModelDownload(request: ModelDownloadRequest): Promise<ModelDownloadRecord> {
  if (!isTauri()) throw new Error("Model downloads require the desktop application.");
  return invoke<ModelDownloadRecord>("start_model_download", { request });
}

export async function resumeModelDownload(id: string): Promise<ModelDownloadRecord> {
  if (!isTauri()) throw new Error("Model downloads require the desktop application.");
  return invoke<ModelDownloadRecord>("resume_model_download", { id });
}

export async function cancelModelDownload(): Promise<void> {
  if (isTauri()) await invoke("cancel_model_download");
}

export async function onModelDownload(callback: (record: ModelDownloadRecord) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<ModelDownloadRecord>("model-download", (event) => callback(event.payload));
  return () => undefined;
}

export async function exportSetupProfile(): Promise<ProfileTransfer> {
  if (!isTauri())
    return {
      path: "C:\\Users\\Researcher\\Kestrel Research\\setup-profiles\\kestrel-profile-preview.json",
      message: "Preview profile exported safely.",
    };
  return invoke<ProfileTransfer>("export_setup_profile");
}

export async function importSetupProfile(path: string): Promise<AppSnapshot> {
  if (!isTauri()) return demoSnapshot;
  return invoke<AppSnapshot>("import_setup_profile", { path });
}

export async function saveControlSettings(
  settings: ControlSettings,
): Promise<ControlSnapshot> {
  if (!isTauri()) return { ...demoSnapshot.control, settings };
  return invoke<ControlSnapshot>("save_control_settings", { settings });
}

export async function startLocalModel(
  modelId: string,
): Promise<ControlSnapshot> {
  if (!isTauri())
    return {
      ...demoSnapshot.control,
      runtime: {
        ...demoSnapshot.control.runtime,
        phase: "ready",
        modelId,
        modelName: demoSnapshot.control.models.find(
          (model) => model.id === modelId,
        )?.name,
      },
    };
  return invoke<ControlSnapshot>("start_local_model", { modelId });
}

export async function stopLocalModel(): Promise<ControlSnapshot> {
  if (!isTauri())
    return {
      ...demoSnapshot.control,
      runtime: {
        ...demoSnapshot.control.runtime,
        phase: "stopped",
        modelId: undefined,
        modelName: undefined,
      },
    };
  return invoke<ControlSnapshot>("stop_local_model");
}

export async function releaseAiMemory(): Promise<ControlSnapshot> {
  if (!isTauri())
    return {
      ...demoSnapshot.control,
      runtime: {
        ...demoSnapshot.control.runtime,
        phase: "stopped",
        mode: "none",
        modelId: undefined,
        modelName: undefined,
      },
    };
  return invoke<ControlSnapshot>("release_ai_memory");
}

export async function listChatSessions(): Promise<ChatSessionSummary[]> {
  if (!isTauri()) return [];
  return invoke<ChatSessionSummary[]>("list_chat_sessions");
}

export async function getChatSession(id: string): Promise<ChatSession> {
  if (!isTauri()) throw new Error("Preview conversations are not persisted.");
  return invoke<ChatSession>("get_chat_session", { id });
}

export async function deleteChatSession(id: string): Promise<void> {
  if (isTauri()) await invoke("delete_chat_session", { id });
}

export async function pickContextFiles(): Promise<ContextAttachmentImport> {
  if (!isTauri()) return { attachments: [], failures: [] };
  return invoke<ContextAttachmentImport>("pick_context_files");
}

export async function openContextAttachment(id: string): Promise<void> {
  if (isTauri()) await invoke("open_context_attachment", { id });
}

export async function pickLocalModelFolder(): Promise<string | undefined> {
  if (!isTauri()) return undefined;
  return (await invoke<string | null>("pick_local_model_folder")) ?? undefined;
}

export async function startChatStream(
  request: StartChatRequest,
): Promise<ChatStart> {
  if (!isTauri())
    throw new Error("Preview conversations cannot stream or persist.");
  return invoke<ChatStart>("start_chat_stream", { request });
}

export async function cancelChatStream(requestId: string): Promise<void> {
  if (isTauri()) await invoke("cancel_chat_stream", { requestId });
}

export async function onChatStream(
  callback: (event: ChatStreamEvent) => void,
): Promise<UnlistenFn> {
  if (isTauri())
    return listen<ChatStreamEvent>("chat-stream", (event) =>
      callback(event.payload),
    );
  return () => undefined;
}

export async function listComputerTasks(): Promise<ComputerTaskSummary[]> {
  if (!isTauri()) return [];
  return invoke<ComputerTaskSummary[]>("list_computer_tasks");
}

export async function getComputerTask(id: string): Promise<ComputerTaskRun> {
  if (!isTauri()) throw new Error("Preview tasks are not persisted.");
  return invoke<ComputerTaskRun>("get_computer_task", { id });
}

export async function startComputerTask(
  request: ComputerTaskRequest,
): Promise<ComputerTaskRun> {
  if (!isTauri())
    throw new Error("Computer Tasks require the desktop application.");
  return invoke<ComputerTaskRun>("start_computer_task", { request });
}

export async function resumeComputerTask(
  request: ResumeComputerTaskRequest,
): Promise<ComputerTaskRun> {
  if (!isTauri())
    throw new Error("Computer Tasks require the desktop application.");
  return invoke<ComputerTaskRun>("resume_computer_task", { request });
}

export async function stopComputerTask(runId: string): Promise<void> {
  if (isTauri()) await invoke("stop_computer_task", { runId });
}

export async function onComputerTaskEvent(
  callback: (event: ComputerTaskEvent) => void,
): Promise<UnlistenFn> {
  if (isTauri())
    return listen<ComputerTaskEvent>("computer-task-event", (event) =>
      callback(event.payload),
    );
  return () => undefined;
}

export async function openTaskArtifact(
  runId: string,
  path: string,
): Promise<void> {
  if (isTauri()) await invoke("open_task_artifact", { runId, path });
}

export async function runNativeDiagnostics(): Promise<string> {
  if (!isTauri()) return "## Preview diagnostics: PASS";
  return invoke<string>("run_native_diagnostics");
}

export async function runCodexRepair(
  issue: string,
): Promise<DeveloperRepairReport> {
  if (!isTauri())
    return {
      success: true,
      summary: "Preview repair verified.",
      diagnosticsBefore: "Preview",
      diagnosticsAfter: "Preview",
      reportPath: "preview.json",
    };
  return invoke<DeveloperRepairReport>("run_codex_repair", {
    request: { issue },
  });
}
