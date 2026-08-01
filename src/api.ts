import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { demoReport, demoSnapshot } from "./demo";
import type { AppSnapshot, ResearchProgress, ResearchReport, ResearchSettings, RunResearchRequest, SystemSnapshot } from "./types";

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
