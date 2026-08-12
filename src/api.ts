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
  MovieEdit,
  MovieImageAssetEvent,
  MovieImageAssetGeneration,
  MovieImageAssetRequest,
  MoviePlan,
  MoviePlanningEvent,
  MoviePlanningSnapshot,
  MovieProject,
  MovieReferenceImport,
  MovieSummary,
  StartMovieRequest,
  StoryDraftEvent,
  StoryDraftRequest,
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

export async function openComfyUi(): Promise<void> {
  if (isTauri()) await invoke("open_comfy_ui");
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

export async function startMovie(request: StartMovieRequest): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Movie production requires the desktop application.");
  return invoke<MovieProject>("start_movie", { request });
}

export async function resumeMovie(id: string): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Movie production requires the desktop application.");
  return invoke<MovieProject>("resume_movie", { id });
}

export async function cancelMovie(id: string): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Movie production requires the desktop application.");
  return invoke<MovieProject>("cancel_movie", { id });
}

export async function startMovieStoryDraft(request: StoryDraftRequest): Promise<string> {
  if (!isTauri()) throw new Error("Local story generation requires the desktop application.");
  return invoke<string>("start_movie_story_draft", { request });
}

export async function cancelMovieStoryDraft(requestId: string): Promise<void> {
  if (isTauri()) await invoke("cancel_movie_story_draft", { requestId });
}

export async function onMovieStoryDraft(callback: (event: StoryDraftEvent) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<StoryDraftEvent>("movie-story-draft", (event) => callback(event.payload));
  return () => undefined;
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
  if (!isTauri()) throw new Error("Bonsai plan revision requires the desktop application.");
  return invoke<MovieProject>("revise_movie_plan", { request: { id, feedback } });
}

export async function approveMoviePlan(id: string): Promise<MovieProject> {
  if (!isTauri()) throw new Error("Producer approval requires the desktop application.");
  return invoke<MovieProject>("approve_movie_plan", { id });
}

export async function askBonsaiMovieClip(id: string, clipId: string, feedback: string): Promise<MovieClipSuggestion> {
  if (!isTauri()) throw new Error("Bonsai scene assistance requires the desktop application.");
  return invoke<MovieClipSuggestion>("ask_bonsai_movie_clip", { request: { id, clipId, feedback } });
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
