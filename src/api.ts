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
  VideoContinuityMode,
  VideoPlanRequest,
  VideoPreset,
  VideoPresetStatus,
  VideoProject,
  VideoProjectEvent,
  VideoSnapshot,
  VideoSettings,
  VideoReferenceRole,
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

type PreviewPresetConfig = VideoPresetStatus & {
  dimensions: Record<VideoPlanRequest["orientation"], readonly [number, number]>;
  fps: number;
  framesPerClip: number;
  cfg: number;
};

const previewPresetTable: Record<VideoPreset, PreviewPresetConfig> = {
  "wan-1.3b-gpu-only": { id: "wan-1.3b-gpu-only", label: "Wan 2.1 1.3B · GPU only", profile: "gpu-only", offloading: "forbidden", nativeClipSeconds: 2, steps: 30, available: true, missingFiles: [], supportsImageReference: false, supportsVideoReference: false, dimensions: { landscape: [832, 480], portrait: [480, 832], square: [624, 624] }, fps: 16, framesPerClip: 33, cfg: 6 },
  "wan-vace-1.3b-reference": { id: "wan-vace-1.3b-reference", label: "Wan VACE 1.3B · Reference studio", profile: "reference-staged", offloading: "stage-boundary-only", nativeClipSeconds: 5, steps: 30, available: true, missingFiles: [], supportsImageReference: true, supportsVideoReference: true, dimensions: { landscape: [832, 480], portrait: [480, 832], square: [624, 624] }, fps: 16, framesPerClip: 81, cfg: 6 },
  "kandinsky-distilled": { id: "kandinsky-distilled", label: "Kandinsky 5 Lite · Distilled", profile: "kandinsky-staged", offloading: "stage-boundary-only", nativeClipSeconds: 5, steps: 16, available: true, missingFiles: [], supportsImageReference: true, supportsVideoReference: false, dimensions: { landscape: [768, 512], portrait: [512, 768], square: [624, 624] }, fps: 24, framesPerClip: 121, cfg: 1 },
  "kandinsky-sft": { id: "kandinsky-sft", label: "Kandinsky 5 Lite · SFT quality", profile: "kandinsky-staged", offloading: "stage-boundary-only", nativeClipSeconds: 5, steps: 100, available: true, missingFiles: [], supportsImageReference: true, supportsVideoReference: false, dimensions: { landscape: [768, 512], portrait: [512, 768], square: [624, 624] }, fps: 24, framesPerClip: 121, cfg: 5 },
  "wan-2.2-5b-offload": { id: "wan-2.2-5b-offload", label: "Wan 2.2 5B · Predictable offload", profile: "forced-offload", offloading: "forced", nativeClipSeconds: 3, steps: 20, available: true, missingFiles: [], supportsImageReference: true, supportsVideoReference: false, dimensions: { landscape: [832, 480], portrait: [480, 832], square: [640, 608] }, fps: 24, framesPerClip: 81, cfg: 5 },
};

const previewPresetStatuses = Object.values(previewPresetTable).map(({ dimensions: _dimensions, fps: _fps, framesPerClip: _framesPerClip, cfg: _cfg, ...status }) => status);

const previewVideoSnapshot: VideoSnapshot = {
  settings: {
    comfyRoot: "D:\\AI\\ComfyUI",
    ffmpegPath: "ffmpeg.exe",
  },
  backend: {
    endpoint: "http://127.0.0.1:8188",
    running: false,
    ready: false,
    owned: false,
    offloading: "none",
    predictable: false,
    detail: "ComfyUI is stopped. Kestrel will start the selected exact profile after planning.",
  },
  presets: previewPresetStatuses,
  projects: [],
  root: "D:\\Kestrel Research\\video-studio",
};

export async function getVideoSnapshot(): Promise<VideoSnapshot> {
  if (!isTauri()) return previewVideoSnapshot;
  return invoke<VideoSnapshot>("get_video_snapshot");
}

export async function saveVideoSettings(settings: VideoSettings): Promise<VideoSnapshot> {
  if (!isTauri()) return { ...previewVideoSnapshot, settings };
  return invoke<VideoSnapshot>("save_video_settings", { settings });
}

export async function getVideoProject(id: string): Promise<VideoProject> {
  if (!isTauri()) throw new Error(`Preview project is not durable: ${id}`);
  return invoke<VideoProject>("get_video_project", { id });
}

export async function updateVideoClipPrompt(id: string, clipIndex: number, prompt: string): Promise<VideoProject> {
  if (!isTauri()) throw new Error("Clip editing is persisted by the desktop application.");
  return invoke<VideoProject>("update_video_clip_prompt", { id, clipIndex, prompt });
}

export async function importVideoReference(id: string, role: VideoReferenceRole): Promise<VideoProject | undefined> {
  if (!isTauri()) throw new Error("Reference importing is persisted by the desktop application.");
  return (await invoke<VideoProject | null>("import_video_reference", { id, role })) ?? undefined;
}

export async function getVideoReferencePreview(id: string, assetId: string): Promise<string> {
  if (!isTauri()) throw new Error("Reference previews are provided by the desktop application.");
  return invoke<string>("get_video_reference_preview", { id, assetId });
}

export async function setVideoContinuity(id: string, mode: VideoContinuityMode, primaryReferenceId?: string): Promise<VideoProject> {
  if (!isTauri()) throw new Error("Continuity settings are persisted by the desktop application.");
  return invoke<VideoProject>("set_video_continuity", { id, mode, primaryReferenceId });
}

export async function setVideoClipReference(id: string, clipIndex: number, referenceAssetId?: string): Promise<VideoProject> {
  if (!isTauri()) throw new Error("Clip references are persisted by the desktop application.");
  return invoke<VideoProject>("set_video_clip_reference", { id, clipIndex, referenceAssetId });
}

export async function setVideoChapterReference(id: string, chapterIndex: number, referenceAssetId?: string): Promise<VideoProject> {
  if (!isTauri()) throw new Error("Chapter references are persisted by the desktop application.");
  return invoke<VideoProject>("set_video_chapter_reference", { id, chapterIndex, referenceAssetId });
}

export async function planVideoProject(request: VideoPlanRequest): Promise<VideoProject> {
  if (!isTauri()) {
    const preset = previewPresetTable[request.preset];
    const [width, height] = preset.dimensions[request.orientation];
    const clipCount = Math.ceil(request.totalDurationSeconds / preset.nativeClipSeconds);
    const now = new Date().toISOString();
    const id = `preview-${Date.now()}`;
    return {
      id,
      title: request.prompt.split(/[.\n]/)[0] || "Preview video",
      prompt: request.prompt,
      audience: request.audience,
      useCase: request.useCase,
      preset: request.preset,
      status: "planned",
      createdAt: now,
      updatedAt: now,
      totalDurationSeconds: request.totalDurationSeconds,
      clipDurationSeconds: preset.nativeClipSeconds,
      width,
      height,
      fps: preset.fps,
      framesPerClip: preset.framesPerClip,
      steps: preset.steps,
      cfg: preset.cfg,
      negativePrompt: request.negativePrompt,
      continuityBible: `Maintain subject, palette, geography, lighting, and motion continuity for ${request.audience}.`,
      planningNote: "Preview plan; the desktop app uses the selected local model and persists native queue state.",
      chapters: [{ index: 1, title: "Visual arc", narrativeGoal: request.useCase, promptSeed: request.prompt, firstClip: 1, lastClip: clipCount }],
      clips: Array.from({ length: clipCount }, (_, index) => ({
        index: index + 1,
        chapterIndex: 1,
        prompt: `${request.prompt}. Shot ${index + 1} of ${clipCount}.`,
        seed: index + 1,
        status: "planned",
        attempts: 0,
      })),
      boundaries: request.boundaries,
      outputDirectory: `${previewVideoSnapshot.root}\\projects\\${id}`,
      errors: [],
      references: [],
      continuity: { mode: "none" },
    };
  }
  return invoke<VideoProject>("plan_video_project", { request });
}

export async function startVideoProject(id: string): Promise<VideoProject> {
  if (!isTauri()) throw new Error("Video generation requires the desktop application.");
  return invoke<VideoProject>("start_video_project", { id });
}

export async function stopVideoProject(id: string): Promise<void> {
  if (isTauri()) await invoke("stop_video_project", { id });
}

export async function stopVideoBackend(): Promise<VideoSnapshot> {
  if (!isTauri()) return previewVideoSnapshot;
  return invoke<VideoSnapshot>("stop_video_backend");
}

export async function pickComfyRoot(): Promise<string | undefined> {
  if (!isTauri()) return "D:\\AI\\ComfyUI";
  return (await invoke<string | null>("pick_comfy_root")) ?? undefined;
}

export async function revealVideoProject(id: string): Promise<void> {
  if (isTauri()) await invoke("reveal_video_project", { id });
}

export async function onVideoProjectEvent(
  callback: (event: VideoProjectEvent) => void,
): Promise<UnlistenFn> {
  if (isTauri())
    return listen<VideoProjectEvent>("video-project-event", (event) => callback(event.payload));
  return () => undefined;
}
