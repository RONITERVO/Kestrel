import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { demoReport, demoSnapshot } from "./demo";
import type { AppSnapshot, ResearchProgress, ResearchReport, RunResearchRequest } from "./types";

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
