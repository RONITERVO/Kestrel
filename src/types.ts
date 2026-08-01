export type ServiceState = "ready" | "starting" | "stopped" | "unavailable";

export interface ServiceStatus {
  bonsai: ServiceState;
  wikipedia: ServiceState;
  model: string;
  archive: string;
  offlineOnly: boolean;
}

export interface Source {
  id: string;
  kind: "wikipedia" | "research";
  title: string;
  section?: string;
  snapshot?: string;
  reference: string;
  excerpt: string;
}

export interface Finding {
  title: string;
  explanation: string;
  citations: string[];
}

export interface ResearchSection {
  id: string;
  heading: string;
  summary: string;
  body: string[];
  citations: string[];
}

export interface TimelineItem {
  label: string;
  date: string;
  description: string;
  citations: string[];
}

export interface Term {
  term: string;
  meaning: string;
}

export interface ResearchReport {
  id: string;
  title: string;
  dek: string;
  query: string;
  answer: string;
  createdAt: string;
  updatedAt: string;
  edition: number;
  parentId?: string;
  improvement: string;
  model: string;
  archiveSnapshot: string;
  findings: Finding[];
  sections: ResearchSection[];
  timeline: TimelineItem[];
  terms: Term[];
  openQuestions: string[];
  sources: Source[];
  htmlPath: string;
  wordCount: number;
  readingMinutes: number;
  researchProfile: string;
  contextWindow: number;
  outputBudget: number;
  researchLanes: number;
}

export interface ReportSummary {
  id: string;
  title: string;
  query: string;
  dek: string;
  updatedAt: string;
  edition: number;
  sourceCount: number;
  readingMinutes: number;
}

export interface AppSnapshot {
  status: ServiceStatus;
  reports: ReportSummary[];
  libraryRoot: string;
  settings: ResearchSettings;
}

export interface ResearchSettings {
  advancedMode: boolean;
  bonsaiRoot: string;
  contextWindow: number;
  maxOutputTokens: number;
  researchLanes: number;
  resultsPerLane: number;
  sourceTarget: number;
  toolTurns: number;
  thinkingBudget: number;
  maxSourceChars: number;
}

export interface GpuSnapshot {
  name: string;
  totalMib: number;
  usedMib: number;
  freeMib: number;
  utilizationPercent: number;
}

export interface RuntimeSnapshot {
  contextWindow: number;
  maxOutputTokens: number;
  parallelSlots: number;
  kvCache: string;
  modelVramMib: number;
  modelRoot: string;
}

export interface SystemSnapshot {
  status: ServiceStatus;
  gpu?: GpuSnapshot;
  runtime: RuntimeSnapshot;
  settings: ResearchSettings;
}

export type ProgressStage =
  | "preparing"
  | "library"
  | "searching"
  | "reading"
  | "synthesizing"
  | "publishing"
  | "complete"
  | "cancelled"
  | "failed";

export interface ResearchProgress {
  jobId: string;
  stage: ProgressStage;
  title: string;
  detail: string;
  current: number;
  total: number;
  elapsedSeconds: number;
}

export interface RunResearchRequest {
  query: string;
  depth: "focused" | "thorough" | "expedition";
}
