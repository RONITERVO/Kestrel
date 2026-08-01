import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { demoReport, demoSnapshot } from "./demo";
import type { AppSnapshot, ChatRequest, ChatResponse, ControlSettings, ControlSnapshot, DeveloperRepairReport, OperationProgress, ResearchProgress, ResearchReport, ResearchSettings, RunResearchRequest, SystemSnapshot } from "./types";

const isTauri = (): boolean => "__TAURI_INTERNALS__" in window;

export async function bootstrap(): Promise<AppSnapshot> {
  if (!isTauri()) return demoSnapshot;
  return invoke<AppSnapshot>("bootstrap");
}

export async function getReport(id: string): Promise<ResearchReport> {
  if (!isTauri()) return demoReport;
  return invoke<ResearchReport>("get_report", { id });
}

export async function runResearch(request: RunResearchRequest): Promise<ResearchReport> {
  if (!isTauri()) {
    await new Promise((resolve) => window.setTimeout(resolve, 900));
    return { ...demoReport, query: request.query, title: request.query, id: `demo-${Date.now()}` };
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

export async function onProgress(callback: (progress: ResearchProgress) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<ResearchProgress>("research-progress", (event) => callback(event.payload));
  return () => undefined;
}

export async function onRuntimeProgress(callback: (progress: OperationProgress) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<OperationProgress>("runtime-progress", (event) => callback(event.payload));
  return () => undefined;
}

export async function onDeveloperProgress(callback: (progress: OperationProgress) => void): Promise<UnlistenFn> {
  if (isTauri()) return listen<OperationProgress>("developer-progress", (event) => callback(event.payload));
  return () => undefined;
}

export async function getSystemSnapshot(): Promise<SystemSnapshot> {
  if (!isTauri()) {
    return {
      status: demoSnapshot.status,
      settings: demoSnapshot.settings,
      runtime: { contextWindow: 98_304, maxOutputTokens: 32_768, parallelSlots: 1, kvCache: "q4_0 / q4_0", modelVramMib: 9_964, modelRoot: "D:\\LocalAI\\Bonsai27B" },
      gpu: { name: "NVIDIA GeForce RTX 5070", totalMib: 12_227, usedMib: 11_128, freeMib: 816, utilizationPercent: 7 },
    };
  }
  return invoke<SystemSnapshot>("get_system_snapshot");
}

export async function saveResearchSettings(settings: ResearchSettings): Promise<ResearchSettings> {
  if (!isTauri()) return settings;
  return invoke<ResearchSettings>("save_research_settings", { settings });
}

export async function applyModelRuntime(settings: ResearchSettings): Promise<SystemSnapshot> {
  if (!isTauri()) return getSystemSnapshot();
  return invoke<SystemSnapshot>("apply_model_runtime", { settings });
}

export async function openBonsaiControlCenter(): Promise<void> {
  if (isTauri()) await invoke("open_bonsai_control_center");
}

export async function getControlSnapshot(): Promise<ControlSnapshot> {
  if (!isTauri()) return demoSnapshot.control;
  return invoke<ControlSnapshot>("get_control_snapshot");
}

export async function scanLocalModels(): Promise<ControlSnapshot> {
  if (!isTauri()) return demoSnapshot.control;
  return invoke<ControlSnapshot>("scan_local_models");
}

export async function saveControlSettings(settings: ControlSettings): Promise<ControlSnapshot> {
  if (!isTauri()) return { ...demoSnapshot.control, settings };
  return invoke<ControlSnapshot>("save_control_settings", { settings });
}

export async function startLocalModel(modelId: string): Promise<ControlSnapshot> {
  if (!isTauri()) return { ...demoSnapshot.control, runtime: { ...demoSnapshot.control.runtime, phase: "ready", modelId, modelName: demoSnapshot.control.models.find(model => model.id === modelId)?.name } };
  return invoke<ControlSnapshot>("start_local_model", { modelId });
}

export async function stopLocalModel(): Promise<ControlSnapshot> {
  if (!isTauri()) return { ...demoSnapshot.control, runtime: { ...demoSnapshot.control.runtime, phase: "stopped", modelId: undefined, modelName: undefined } };
  return invoke<ControlSnapshot>("stop_local_model");
}

export async function sendLocalChat(request: ChatRequest): Promise<ChatResponse> {
  if (!isTauri()) return { content: `Preview response from the local model to: ${request.message}` };
  return invoke<ChatResponse>("send_local_chat", { request });
}

export async function runNativeDiagnostics(): Promise<string> {
  if (!isTauri()) return "## Preview diagnostics: PASS";
  return invoke<string>("run_native_diagnostics");
}

export async function runCodexRepair(issue: string): Promise<DeveloperRepairReport> {
  if (!isTauri()) return { success: true, summary: "Preview repair verified.", diagnosticsBefore: "Preview", diagnosticsAfter: "Preview", reportPath: "preview.json" };
  return invoke<DeveloperRepairReport>("run_codex_repair", { request: { issue } });
}
