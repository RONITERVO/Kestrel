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
  setup: SetupSnapshot;
}

export interface SetupComponent {
  id: "assistant" | "wikipedia" | "media" | "studio" | string;
  label: string;
  status: "ready" | "partial" | "missing" | string;
  detail: string;
  path: string;
  downloadBytes: number;
  optional: boolean;
}

export interface SetupSnapshot {
  ready: boolean;
  installRoot: string;
  availableBytes: number;
  gpuName?: string;
  gpuMemoryBytes: number;
  components: SetupComponent[];
}

export interface SetupLocations {
  installRoot: string;
  bonsaiRoot: string;
  enginePath: string;
  wikipediaZimPath: string;
  kiwixServerPath: string;
  comfyRoot: string;
  ffmpegPath: string;
  ffprobePath: string;
}

export interface SetupInstallRequest {
  component: string;
  installRoot: string;
  wikipediaEdition: "compact" | "complete";
}

export interface SetupProgress {
  component: string;
  stage: string;
  detail: string;
  downloadedBytes: number;
  totalBytes: number;
  bytesPerSecond: number;
}

export interface MovieSettings {
  width: number;
  height: number;
  clipSeconds: number;
  steps: number;
  maxClips: number;
  seed: number;
  temperature: number;
  topP: number;
  topK: number;
  thinkingBudget: number;
  maxOutputTokens: number;
  comfyRoot: string;
  refImageSize: "match" | "max";
}

export interface MovieReferenceAsset {
  id: string;
  name: string;
  kind: "image" | "video" | "audio";
  mimeType: string;
  bytes: number;
  durationSeconds: number;
  width: number;
  height: number;
  hasAudio: boolean;
  path: string;
  createdAt: string;
}

export interface ProducerReferenceRequest {
  assetId: string;
  description: string;
  useEmbeddedAudio: boolean;
  embeddedAudioDescription: string;
}

export interface PendingMovieReference extends MovieReferenceAsset, ProducerReferenceRequest {}

export interface MovieReferenceImport {
  references: MovieReferenceAsset[];
  failures: string[];
}

export interface MovieReference {
  assetId: string;
  tag: string;
  audioTag: string;
  name: string;
  kind: "image" | "video" | "audio";
  mimeType: string;
  bytes: number;
  durationSeconds: number;
  width: number;
  height: number;
  hasAudio: boolean;
  path: string;
  description: string;
  useEmbeddedAudio: boolean;
  embeddedAudioDescription: string;
}

export interface PlannedClip {
  id: string;
  title: string;
  purpose: string;
  durationSeconds: number;
  prompt: string;
  continuityIn: string;
  continuityOut: string;
  transition: string;
  usePreviousFrame: boolean;
  sourceRefs: string[];
  referenceIds: string[];
}

export interface MoviePlan {
  title: string;
  logline: string;
  audience: string;
  creativeDirection: string;
  continuityBible: string[];
  sourceCredits: string[];
  qualityReview: { attempts: number; score: number; verdict: string };
  clips: PlannedClip[];
}

export interface MovieSource {
  id: string;
  title: string;
  reference: string;
  snapshot: string;
  excerpt: string;
}

export interface RenderedClip {
  id: string;
  index: number;
  title: string;
  prompt: string;
  durationSeconds: number;
  seed: number;
  status: "queued" | "rendering" | "complete" | "failed" | string;
  path: string;
  error: string;
  versions: ClipVersion[];
}

export interface ClipVersion {
  id: string;
  createdAt: string;
  title: string;
  prompt: string;
  durationSeconds: number;
  seed: number;
  path: string;
}

export interface ProducerFeedbackRecord {
  createdAt: string;
  scope: string;
  clipId: string;
  feedback: string;
}

export interface MovieClipSuggestion {
  clipId: string;
  summary: string;
  checklist: string[];
  clip: PlannedClip;
}

export interface MovieClipRenderRequest {
  id: string;
  suggestion: MovieClipSuggestion;
  seed: number;
}

export interface ClipEdit {
  id: string;
  clipId: string;
  enabled: boolean;
  order: number;
  trimStart: number;
  trimEnd: number;
  audioGain: number;
  sourceVersionId: string;
  speed: number;
  fadeIn: number;
  fadeOut: number;
  audioFadeIn: number;
  audioFadeOut: number;
}

export interface MovieEdit {
  clips: ClipEdit[];
  exportTitle: string;
  exportPreset: "archive" | "publish" | "review";
  normalizeAudio: boolean;
  targetLufs: number;
}

export interface MovieExport {
  id: string;
  createdAt: string;
  title: string;
  preset: "archive" | "publish" | "review" | string;
  path: string;
  bytes: number;
  sha256: string;
  durationSeconds: number;
  clipCount: number;
}

export interface MovieProject {
  schemaVersion: number;
  id: string;
  prompt: string;
  title: string;
  status: "running" | "complete" | "failed" | "cancelled" | "interrupted" | string;
  phase: string;
  detail: string;
  createdAt: string;
  updatedAt: string;
  model: string;
  renderer: string;
  settings: MovieSettings;
  references: MovieReference[];
  plan?: MoviePlan;
  sources: MovieSource[];
  clips: RenderedClip[];
  edit: MovieEdit;
  finalPath: string;
  exports: MovieExport[];
  error: string;
  producerReviewRequired: boolean;
  producerApprovedAt: string;
  producerFeedback: ProducerFeedbackRecord[];
}

export interface ProducerDirection {
  id: string;
  createdAt: string;
  text: string;
}

export interface MoviePromptDocument {
  id: string;
  title: string;
  category: string;
  content: string;
}

export interface MoviePlanningSnapshot {
  projectId: string;
  checkpointRequested: boolean;
  pendingDirections: ProducerDirection[];
  promptDocuments: MoviePromptDocument[];
  toolSchema: unknown;
  lastRequest: unknown;
  transcript: unknown;
  currentText: string;
}

export interface MoviePlanningEvent {
  projectId: string;
  sequence: number;
  kind: "token" | "advanced-token" | "reasoning" | "turn-start" | "turn-complete" | "activity" | "tool-result" | "direction-queued" | "checkpoint-requested" | "checkpoint-saved" | string;
  stage: string;
  text: string;
  session: number;
  step: number;
  createdAt: string;
}

export interface MovieSummary {
  id: string;
  title: string;
  status: string;
  phase: string;
  updatedAt: string;
  clipCount: number;
  finalPath: string;
}

export interface StartMovieRequest {
  prompt: string;
  settings: MovieSettings;
  references: ProducerReferenceRequest[];
  pauseAfterPlan: boolean;
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
  installRoot: string;
  wikipediaZimPath: string;
  kiwixServerPath: string;
  wikipediaBook: string;
  wikipediaSnapshot: string;
  comfyRoot: string;
  ffmpegPath: string;
  ffprobePath: string;
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
