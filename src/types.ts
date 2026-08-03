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
  control: ControlSnapshot;
}

export interface ModelInfo {
  id: string;
  name: string;
  path: string;
  source: string;
  bytes: number;
  architecture?: string;
  contextLength?: number;
  chatTemplate: boolean;
  quantization?: string;
  mmprojPath?: string;
  supportsVision: boolean;
  supportsAudio: boolean;
  recommendation: string;
}

export interface ContextAttachment {
  id: string;
  name: string;
  kind: "image" | "audio" | "pdf" | "document" | "text" | "binary" | string;
  mimeType: string;
  bytes: number;
  sha256: string;
  storedPath: string;
  extractedChars: number;
  contextMode: "native_media" | "extracted_text" | "text" | "metadata_only" | string;
  note: string;
  createdAt: string;
}

export interface ContextAttachmentImport {
  attachments: ContextAttachment[];
  failures: string[];
}

export interface ControlSettings {
  advancedMode: boolean;
  enginePath: string;
  extraModelRoots: string[];
  selectedModelId?: string;
  contextWindow: number;
  maxOutputTokens: number;
  threads: number;
  projectRoot: string;
  agentWorkspaceRoots: string[];
  allowFullAccessAgent: boolean;
  agentMaxSteps: number;
  agentMaxOutputTokens: number;
}

export interface ManagedRuntimeSnapshot {
  phase: string;
  mode: string;
  modelId?: string;
  modelName?: string;
  endpoint?: string;
  pid?: number;
  contextWindow: number;
  launchArgs: string[];
  detail: string;
  inferenceBusy: boolean;
}

export interface DeveloperStatus {
  codexAvailable: boolean;
  codexAuthenticated: boolean;
  codexVersion?: string;
  projectRoot: string;
  gitRepository: boolean;
  worktreeClean: boolean;
  running: boolean;
  lastReport?: string;
}

export interface ControlSnapshot {
  settings: ControlSettings;
  models: ModelInfo[];
  engineCandidates: EngineCandidate[];
  runtime: ManagedRuntimeSnapshot;
  gpu?: GpuSnapshot;
  developer: DeveloperStatus;
  runtimeLogs: RuntimeLog[];
}

export interface EngineCandidate {
  path: string;
  source: string;
}

export interface ProfileTransfer {
  path: string;
  message: string;
}

export interface RuntimeLog {
  stream: "stdout" | "stderr" | string;
  line: string;
  at: string;
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  reasoning?: string;
  status?: "interrupted" | "limited";
  attachments?: ContextAttachment[];
  createdAt: string;
}

export interface ChatSession {
  id: string;
  title: string;
  modelId: string;
  createdAt: string;
  updatedAt: string;
  messages: ChatMessage[];
}

export interface ChatSessionSummary {
  id: string;
  title: string;
  modelId: string;
  updatedAt: string;
  messageCount: number;
}

export interface StartChatRequest {
  sessionId?: string;
  modelId: string;
  message: string;
  attachmentIds: string[];
  temperature: number;
  topP: number;
  topK: number;
  maxOutputTokens: number;
}

export interface ChatStart {
  requestId: string;
  session: ChatSession;
}

export interface ChatStreamEvent {
  requestId: string;
  sessionId: string;
  kind: "queued" | "started" | "context" | "token" | "reasoning" | "metrics" | "done" | "cancelled" | "error" | "settled";
  content?: string;
  data?: Record<string, unknown>;
  at: string;
}

export interface ComputerTaskRequest {
  modelId: string;
  objective: string;
  attachmentIds: string[];
  access: "workspace" | "full";
  maxSteps: number;
  maxOutputTokens: number;
}

export interface ResumeComputerTaskRequest {
  runId: string;
  answer: string;
}

export interface ComputerTaskEvent {
  runId: string;
  step: number;
  kind: string;
  title: string;
  detail: string;
  data?: { path?: string; question?: string; options?: string[]; recommendedIndex?: number; [key: string]: unknown };
  at: string;
}

export interface ComputerTaskRun {
  id: string;
  objective: string;
  modelId: string;
  access: "workspace" | "full";
  status: string;
  createdAt: string;
  updatedAt: string;
  events: ComputerTaskEvent[];
  artifacts: string[];
  attachments?: ContextAttachment[];
}

export interface ComputerTaskSummary {
  id: string;
  objective: string;
  modelId: string;
  access: "workspace" | "full";
  status: string;
  updatedAt: string;
  eventCount: number;
  artifactCount: number;
}

export interface DeveloperRepairReport {
  success: boolean;
  summary: string;
  diagnosticsBefore: string;
  diagnosticsAfter: string;
  reportPath: string;
}

export interface OperationProgress {
  stage?: string;
  phase?: string;
  detail: string;
  at?: string;
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

export type VideoPreset =
  | "wan-1.3b-gpu-only"
  | "wan-vace-1.3b-reference"
  | "kandinsky-distilled"
  | "kandinsky-sft"
  | "wan-2.2-5b-offload";

export interface VideoSettings {
  comfyRoot: string;
  ffmpegPath: string;
}

export interface VideoBoundarySettings {
  maxClips: number;
  maxRetriesPerClip: number;
  maxFailedClips: number;
  maxRuntimeMinutes: number;
  minFreeDiskGib: number;
  assembleFinalVideo: boolean;
}

export interface VideoPlanRequest {
  prompt: string;
  audience: string;
  useCase: string;
  plannerModelId?: string;
  preset: VideoPreset;
  totalDurationSeconds: number;
  orientation: "landscape" | "portrait" | "square";
  negativePrompt: string;
  boundaries: VideoBoundarySettings;
}

export type VideoClipStatus =
  | "planned"
  | "queued"
  | "generating"
  | "verifying"
  | "complete"
  | "failed";

export type VideoProjectStatus =
  | "planned"
  | "starting"
  | "running"
  | "verifying"
  | "assembling"
  | "completed"
  | "completed-with-warnings"
  | "cancelled"
  | "interrupted"
  | "paused-boundary"
  | "paused-failures"
  | "failed";

export interface VideoChapter {
  index: number;
  title: string;
  narrativeGoal: string;
  promptSeed: string;
  firstClip: number;
  lastClip: number;
  referenceAssetId?: string;
}

export interface VideoClip {
  index: number;
  chapterIndex: number;
  prompt: string;
  seed: number;
  status: VideoClipStatus;
  attempts: number;
  comfyPromptId?: string;
  outputPath?: string;
  bytes?: number;
  sha256?: string;
  error?: string;
  startedAt?: string;
  completedAt?: string;
  referenceAssetId?: string;
  continuityFramePath?: string;
}

export type VideoReferenceKind = "image" | "video";
export type VideoReferenceRole = "subject" | "storyboard" | "motion";
export type VideoContinuityMode = "none" | "anchor" | "previous-frame";

export interface VideoReferenceAsset {
  id: string;
  name: string;
  kind: VideoReferenceKind;
  role: VideoReferenceRole;
  storedPath: string;
  bytes: number;
  sha256: string;
  createdAt: string;
  previewPath?: string;
}

export interface VideoContinuitySettings {
  mode: VideoContinuityMode;
  primaryReferenceId?: string;
}

export interface VideoProject {
  id: string;
  title: string;
  prompt: string;
  audience: string;
  useCase: string;
  preset: VideoPreset;
  status: VideoProjectStatus;
  createdAt: string;
  updatedAt: string;
  totalDurationSeconds: number;
  clipDurationSeconds: number;
  width: number;
  height: number;
  fps: number;
  framesPerClip: number;
  steps: number;
  cfg: number;
  negativePrompt: string;
  continuityBible: string;
  planningNote: string;
  chapters: VideoChapter[];
  clips: VideoClip[];
  boundaries: VideoBoundarySettings;
  outputDirectory: string;
  finalOutputPath?: string;
  errors: string[];
  references: VideoReferenceAsset[];
  continuity: VideoContinuitySettings;
}

export interface VideoProjectSummary {
  id: string;
  title: string;
  preset: VideoPreset;
  status: VideoProjectStatus;
  updatedAt: string;
  totalDurationSeconds: number;
  clipCount: number;
  completedClips: number;
  failedClips: number;
}

export interface VideoPresetStatus {
  id: VideoPreset;
  label: string;
  profile: string;
  offloading: string;
  nativeClipSeconds: number;
  steps: number;
  available: boolean;
  missingFiles: string[];
  supportsImageReference: boolean;
  supportsVideoReference: boolean;
}

export interface VideoBackendSnapshot {
  endpoint: string;
  running: boolean;
  ready: boolean;
  owned: boolean;
  pid?: number;
  profile?: string;
  offloading: string;
  predictable: boolean;
  detail: string;
}

export interface VideoSnapshot {
  settings: VideoSettings;
  backend: VideoBackendSnapshot;
  presets: VideoPresetStatus[];
  projects: VideoProjectSummary[];
  root: string;
}

export interface VideoProjectEvent {
  projectId: string;
  kind: string;
  title: string;
  detail: string;
  clipIndex?: number;
  at: string;
}
