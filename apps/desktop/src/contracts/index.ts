// New application contracts are generated from Rust. The declarations below the generated
// imports are a quarantined migration set; do not add durable state or IPC types here.
import type {
  AppSnapshot,
  ChatMessage,
  ChatSession,
  ChatSessionSummary,
  ChatStart,
  ChatStreamEvent,
  ComputerTaskEvent,
  ComputerTaskRequest,
  ComputerTaskRun,
  ComputerTaskSummary,
  ContextAttachment,
  ControlSettings,
  ControlSnapshot,
  DeveloperRepairReport,
  DeveloperStatus,
  EngineCandidate,
  Finding,
  GpuSnapshot,
  ManagedRuntimeSnapshot,
  AttachMovieProducerReferencesRequest,
  CreateMovieProducerProjectRequest,
  MovieProducerProjectSettings,
  MovieProducerReferenceRequest,
  AcceptMovieStoryRevisionRequest,
  MovieProducerWorkspace,
  MovieSceneDraft,
  MovieSceneFrameSource,
  MovieSceneFrameSourceKind,
  MovieSceneReferenceSelection,
  MovieStoryRevision,
  MovieStoryRevisionOrigin,
  MovieStudioChatEvent,
  MovieStudioChatRequest,
  MovieStudioConversation,
  MovieStudioConversationKind,
  MovieStudioConversationMode,
  MovieStudioConversationSummary,
  MovieStudioMessage,
  MovieStudioMessageRole,
  ResetMovieStudioConversationRequest,
  SaveMovieScenesRequest,
  SaveMovieStoryRevisionRequest,
  SummarizeMovieStudioConversationRequest,
  ModelInfo,
  ModelRuntimeOverride,
  ProfileTransfer,
  ProvenHardwareProfile,
  ReportSummary,
  ResearchProgress,
  ResearchReport,
  ResearchSection,
  ResearchSettings,
  ResumeComputerTaskRequest,
  RunResearchRequest,
  RuntimeLog,
  RuntimeSnapshot,
  ServiceStatus,
  SetupComponent,
  SetupInstallRequest,
  SetupLocations,
  SetupModelAsset,
  SetupProgress,
  SetupSnapshot,
  Source,
  SpeechRecordingAttachment,
  StartChatRequest,
  SystemSnapshot,
  Term,
  ThinkingLevel,
  TimelineItem,
} from "@kestrel/generated-bindings";
export type {
  AppSnapshot,
  ChatMessage,
  ChatSession,
  ChatSessionSummary,
  ChatStart,
  ChatStreamEvent,
  ComputerTaskEvent,
  ComputerTaskRequest,
  ComputerTaskRun,
  ComputerTaskSummary,
  ContextAttachment,
  ControlSettings,
  ControlSnapshot,
  DeveloperRepairReport,
  DeveloperStatus,
  EngineCandidate,
  Finding,
  GpuSnapshot,
  ManagedRuntimeSnapshot,
  AttachMovieProducerReferencesRequest,
  CreateMovieProducerProjectRequest,
  MovieProducerProjectSettings,
  MovieProducerReferenceRequest,
  AcceptMovieStoryRevisionRequest,
  MovieProducerWorkspace,
  MovieSceneDraft,
  MovieSceneFrameSource,
  MovieSceneFrameSourceKind,
  MovieSceneReferenceSelection,
  MovieStoryRevision,
  MovieStoryRevisionOrigin,
  MovieStudioChatEvent,
  MovieStudioChatRequest,
  MovieStudioConversation,
  MovieStudioConversationKind,
  MovieStudioConversationMode,
  MovieStudioConversationSummary,
  MovieStudioMessage,
  MovieStudioMessageRole,
  ResetMovieStudioConversationRequest,
  SaveMovieScenesRequest,
  SaveMovieStoryRevisionRequest,
  SummarizeMovieStudioConversationRequest,
  ModelInfo,
  ModelRuntimeOverride,
  ProfileTransfer,
  ProvenHardwareProfile,
  ReportSummary,
  ResearchProgress,
  ResearchReport,
  ResearchSection,
  ResearchSettings,
  ResumeComputerTaskRequest,
  RunResearchRequest,
  RuntimeLog,
  RuntimeSnapshot,
  ServiceStatus,
  SetupComponent,
  SetupInstallRequest,
  SetupLocations,
  SetupModelAsset,
  SetupProgress,
  SetupSnapshot,
  Source,
  SpeechRecordingAttachment,
  StartChatRequest,
  SystemSnapshot,
  Term,
  ThinkingLevel,
  TimelineItem,
} from "@kestrel/generated-bindings";

export type ServiceState = "ready" | "starting" | "stopped" | "unavailable";

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
  thinkingLevel?: ThinkingLevel;
  maxOutputTokens: number;
  contextWindow?: number;
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
  generation?: GeneratedImageProvenance;
}

export interface GeneratedImageProvenance {
  generationId: string;
  workflow: string;
  workflowSource: string;
  workflowRevision: string;
  prompt: string;
  renderedPrompt: string;
  width: number;
  height: number;
  steps: number;
  seed: number;
  requestedLength: number;
  resolvedFrameCount: number;
  frameIndex: number;
  sampler: string;
  scheduler: string;
  diffusionModel: string;
  textEncoder: string;
  vae: string;
  comfyPromptId: string;
  createdAt: string;
  exactGraph: unknown;
}

export interface MovieImageAssetRequest {
  requestId: string;
  prompt: string;
  width: number;
  height: number;
  steps: number;
  seed: number;
  comfyRoot: string;
  stabilize: boolean;
}

export interface MovieImageAssetCandidate {
  frameIndex: number;
  asset: MovieReferenceAsset;
}

export interface MovieImageAssetGeneration {
  id: string;
  status: "running" | "complete" | "failed" | "cancelled" | string;
  stage: string;
  detail: string;
  prompt: string;
  renderedPrompt: string;
  width: number;
  height: number;
  steps: number;
  seed: number;
  stabilize: boolean;
  workflow: string;
  workflowSource: string;
  workflowRevision: string;
  previewNodeRevision: string;
  previewDecoderRevision: string;
  previewDecoderSha256: string;
  requestedLength: number;
  resolvedFrameCount: number;
  candidateStart: number;
  candidateCount: number;
  comfyPromptId: string;
  createdAt: string;
  updatedAt: string;
  completedAt: string;
  error: string;
  candidates: MovieImageAssetCandidate[];
  exactGraph: unknown;
}

export interface MovieImageAssetEvent {
  requestId: string;
  kind: "started" | "progress" | "complete" | "cancelled" | "error" | string;
  stage: string;
  detail: string;
  progress: number;
  at: string;
  generation?: MovieImageAssetGeneration;
}

export interface MovieRenderPreviewEvent {
  kind: "connected" | "frame" | "finished" | "unavailable" | "stopped" | string;
  target: "imageAsset" | "movieClip" | string;
  jobId: string;
  projectId?: string;
  clipId?: string;
  clipIndex?: number;
  detail: string;
  mimeType?: "image/jpeg" | "image/png" | "image/webp" | "video/mp4" | string;
  dataUrl?: string;
  width?: number;
  height?: number;
  step?: number;
  total?: number;
  fps?: number;
  stepMs?: number;
  averageStepMs?: number;
  previewNodeRevision: string;
  previewDecoderRevision: string;
  previewDecoderSha256: string;
  at: string;
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
  generation?: GeneratedImageProvenance;
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
  firstFrameReferenceId: string;
  lastFrameReferenceId: string;
  referenceSelections: MovieSceneReferenceSelection[];
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

export interface MovieRenderState {
  active: boolean;
  preview?: MovieRenderPreviewEvent;
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
  label: string;
  notes: string;
}

export interface TimelineMarker {
  id: string;
  timeSeconds: number;
  label: string;
  kind: "marker" | "todo" | "chapter";
  completed: boolean;
}

export interface MovieEdit {
  clips: ClipEdit[];
  exportTitle: string;
  exportPreset: "archive" | "publish" | "review";
  normalizeAudio: boolean;
  targetLufs: number;
  markers: TimelineMarker[];
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

export interface ImageStyle {
  mode: "photo" | "art";
  aesthetics: string;
  lighting: string;
  photo: string;
  artStyle: string;
  medium: string;
  colorPalette: string[];
}

export interface ImageElement {
  id: string;
  kind: "obj" | "text";
  /** Ideogram coordinates: [top, left, bottom, right], normalized to 0..1000. */
  bbox: [number, number, number, number];
  text: string;
  description: string;
  colorPalette: string[];
}

export interface ImageSettings {
  width: number;
  height: number;
  preset: "quality" | "standard" | "turbo";
  seed: number;
  batchSize: number;
  comfyRoot: string;
}

export interface ImageTake {
  id: string;
  createdAt: string;
  status: string;
  detail: string;
  error: string;
  path: string;
  bytes: number;
  sha256: string;
  width: number;
  height: number;
  preset: ImageSettings["preset"];
  seed: number;
  batchIndex: number;
  batchSize: number;
  promptId: string;
  exactPrompt: unknown;
  exactPromptText: string;
  exactGraph: unknown;
  modelProfile: string;
  licenseNotice: string;
}

export interface ImageProject {
  schemaVersion: number;
  id: string;
  title: string;
  idea: string;
  highLevelDescription: string;
  style: ImageStyle;
  background: string;
  elements: ImageElement[];
  settings: ImageSettings;
  takes: ImageTake[];
  activeTakeId: string;
  status: string;
  phase: string;
  detail: string;
  error: string;
  licenseNotice: string;
  createdAt: string;
  updatedAt: string;
}

export interface ImageSummary {
  id: string;
  title: string;
  status: string;
  updatedAt: string;
  takeCount: number;
  activeTakePath: string;
}

export interface ImageGenerationEvent {
  projectId: string;
  takeId: string;
  kind: "queued" | "progress" | "complete" | "cancelled" | "error" | string;
  phase: string;
  detail: string;
  step?: number;
  total?: number;
  percent?: number;
  etaSeconds?: number;
  at: string;
}

export interface MusicSettings {
  maxDurationSeconds: number;
  steps: number;
  cfgScale: number;
  topK: number;
  seed: number;
  tiledDecode: boolean;
  modelVariant: "auto" | "int8" | "fp16";
  comfyRoot: string;
}

export interface MusicSection {
  id: string;
  tag: "Intro" | "Verse" | "Pre-Chorus" | "Chorus" | "Post-Chorus" | "Bridge" | "Instrumental" | "Solo" | "Break" | "Outro";
  name: string;
  bars: number;
  lyrics: string;
  direction: string;
}

export interface MusicMidiSettings {
  executablePath: string;
  modelPath: string;
  instruments: string;
}

export interface MusicTake {
  id: string;
  createdAt: string;
  status: string;
  detail: string;
  error: string;
  path: string;
  bytes: number;
  sha256: string;
  durationSeconds: number;
  seed: number;
  resolvedModel: string;
  caption: string;
  lyrics: string;
  promptId: string;
  exactGraph: unknown;
  midiPath: string;
  midiReceiptPath: string;
  midiSourcePath: string;
  midiDocumentPath: string;
  midiRevision: number;
  lyricsDocumentPath: string;
  lyricsReceiptPath: string;
  lyricsRevision: number;
}

export interface MusicLyricWord {
  value: string;
  start: number;
  end: number;
}

export interface MusicLyricSegment {
  id: string;
  start: number;
  end: number;
  primary: string;
  translation: string;
  words: MusicLyricWord[];
}

export type MusicLyricTheme = "sketchbook" | "signal-bloom";

export interface MusicLyricsDocument {
  schemaVersion: number;
  takeId: string;
  sourceSha256: string;
  revision: number;
  language: string;
  source: string;
  transcript: string;
  theme: MusicLyricTheme;
  showTranslation: boolean;
  translationLanguage: string;
  translationModelId: string;
  createdAt: string;
  updatedAt: string;
  segments: MusicLyricSegment[];
}

export interface MusicLyricsSaveResult {
  project: MusicProject;
  document: MusicLyricsDocument;
}

export interface RepairMusicLyricsRangeRequest {
  projectId: string;
  takeId: string;
  jobId: string;
  modelId: string;
  language: string;
  startSeconds: number;
  endSeconds: number;
  prompt: string;
}

export interface DraftLyricsFromAudioRangeRequest {
  projectId: string;
  takeId: string;
  modelId: string;
  startSeconds: number;
  endSeconds: number;
}

export interface DraftLyricsFromAudioRangeResult {
  transcription: string;
  modelId: string;
  modelName: string;
}

export interface TranslateMusicLyricsRequest {
  projectId: string;
  takeId: string;
  modelId: string;
  targetLanguage: string;
  lines: string[];
}

export interface TranslateMusicLyricsResult {
  translations: string[];
  modelId: string;
  modelName: string;
}

export interface MusicMidiTempo {
  tick: number;
  microsecondsPerQuarter: number;
}

export interface MusicMidiTimeSignature {
  tick: number;
  numerator: number;
  denominator: number;
}

export interface MusicMidiNote {
  id: string;
  pitch: number;
  startTick: number;
  durationTicks: number;
  velocity: number;
  channel: number;
}

export interface MusicMidiTrack {
  id: string;
  name: string;
  channel: number;
  program: number;
  muted: boolean;
  notes: MusicMidiNote[];
}

export interface MusicMidiDocument {
  schemaVersion: number;
  takeId: string;
  sourceSha256: string;
  revision: number;
  ticksPerQuarter: number;
  durationTicks: number;
  durationSeconds: number;
  tempos: MusicMidiTempo[];
  timeSignatures: MusicMidiTimeSignature[];
  tracks: MusicMidiTrack[];
}

export interface MusicMidiSaveResult {
  project: MusicProject;
  document: MusicMidiDocument;
}

export interface MusicProject {
  schemaVersion: number;
  id: string;
  title: string;
  idea: string;
  caption: string;
  instrumental: boolean;
  sections: MusicSection[];
  settings: MusicSettings;
  midi: MusicMidiSettings;
  takes: MusicTake[];
  activeTakeId: string;
  status: string;
  phase: string;
  detail: string;
  error: string;
  createdAt: string;
  updatedAt: string;
}

export interface MusicSummary {
  id: string;
  title: string;
  status: string;
  updatedAt: string;
  takeCount: number;
  activeTakePath: string;
}

export interface MusicGenerationEvent {
  projectId: string;
  takeId: string;
  kind: "queued" | "progress" | "complete" | "cancelled" | "error" | string;
  phase: string;
  detail: string;
  step?: number;
  total?: number;
  percent?: number;
  etaSeconds?: number;
  at: string;
}

export interface SpeechModel {
  id: string;
  name: string;
  provider: string;
}

export type VoicePerformance = "restrained" | "natural" | "expressive" | "dramatic";

export interface VoiceProfile {
  id: string;
  name: string;
  language: string;
  tags: string[];
  source: "built-in" | "recorded" | "imported" | string;
  consentConfirmed: boolean;
  performance: VoicePerformance;
  referenceRelativePath?: string;
  referenceSha256?: string;
  referenceSeconds?: number;
  originalFileName?: string;
  createdAt: string;
  updatedAt: string;
}

export interface VoiceLibrarySnapshot {
  profiles: VoiceProfile[];
  defaultProfileId: string;
}

export interface CreateVoiceProfileRequest {
  name: string;
  language: string;
  tags: string[];
  source: "recorded" | "imported";
  consentConfirmed: boolean;
  performance: VoicePerformance;
  audioBase64: string;
  mimeType: string;
  originalFileName: string;
  durationSeconds: number;
}

export interface UpdateVoiceProfileRequest {
  id: string;
  name: string;
  language: string;
  tags: string[];
  consentConfirmed: boolean;
  performance: VoicePerformance;
}

export interface LocalSpeechSnapshot {
  narrationAvailable: boolean;
  transcriptionAvailable: boolean;
  comfyReady: boolean;
  voices: SpeechModel[];
  transcribers: SpeechModel[];
  voiceProfiles: VoiceProfile[];
  defaultVoiceProfileId: string;
  detail: string;
}

export interface SpeechSynthesisRequest {
  jobId: string;
  sourceKind: "research" | "chat" | "task" | "copilot";
  sourceId: string;
  passageId: string;
  text: string;
  modelId: string;
  voiceProfileId: string;
}

export interface SpeechAlignmentRequest {
  jobId: string;
  sourceKind: "research" | "chat" | "task" | "copilot";
  sourceId: string;
  passageId: string;
  text: string;
  relativePath: string;
  voiceModelId: string;
  voiceProfileId: string;
  alignmentModelId: string;
}

export interface SpeechTiming {
  value: string;
  start: number;
  end: number;
}

export interface SpeechClip {
  jobId: string;
  passageId: string;
  relativePath: string;
  modelId: string;
  voiceProfileId: string;
  cacheHit: boolean;
  segments: SpeechTiming[];
  words: SpeechTiming[];
}

export interface SpeechTranscriptionRequest {
  jobId: string;
  sourceKind: "research" | "chat" | "task" | "copilot";
  sourceId: string;
  recordingId: string;
  audioBase64: string;
  mimeType: string;
  modelId: string;
  language: string;
  prompt: string;
  finalPass: boolean;
}

export interface SpeechTranscription {
  jobId: string;
  recordingId: string;
  text: string;
  segments: SpeechTiming[];
  words: SpeechTiming[];
  audioRelativePath?: string;
  finalPass: boolean;
}

export interface SpeechProgress {
  jobId: string;
  passageId: string;
  stage: "cached" | "generating" | "transcribing" | "aligning" | "complete" | string;
  detail: string;
}

export type PromptDraftTarget = "story" | "imageAsset" | "imageComposition" | "referenceDescription" | "musicCaption" | "musicLyrics";
export type PromptDraftMode = "develop" | "continue";

export interface PromptDraftRequest {
  requestId: string;
  modelId: string;
  target: PromptDraftTarget;
  mode: PromptDraftMode;
  storyText: string;
  existingText: string;
  assetName: string;
  assetKind: string;
  thinkingLevel?: ThinkingLevel;
  contextWindow?: number;
  maxOutputTokens?: number;
}

export interface PromptDraftReceipt {
  target: PromptDraftTarget;
  mode: PromptDraftMode;
  modelId: string;
  messages: Array<{ role: string; content: string }>;
  temperature: number;
  topP: number;
  topK: number;
  maxTokens: number;
  exactRequest: Record<string, unknown>;
}

export interface PromptDraftEvent {
  requestId: string;
  kind: "queued" | "started" | "token" | "reasoning" | "complete" | "limited" | "cancelled" | "error" | "settled" | string;
  content?: string;
  modelName?: string;
  thinkingLevel?: ThinkingLevel;
  receipt?: PromptDraftReceipt;
  at: string;
}

export interface ModelDownloadRequest {
  url: string;
  expectedSha256?: string;
}

export interface ModelDownloadCandidate {
  filePath: string;
  fileName: string;
  url: string;
  bytes: number;
  sha256?: string;
  kind: "model" | "model-shard" | "projector" | string;
}

export interface ModelDownloadInspection {
  repository: string;
  revision: string;
  candidates: ModelDownloadCandidate[];
  detail: string;
}

export interface ModelDownloadRecord {
  id: string;
  status: "inspecting" | "downloading" | "retrying" | "verifying" | "paused" | "interrupted" | "source-changed" | "failed" | "complete" | string;
  sourceUrl: string;
  repository: string;
  revision: string;
  fileName: string;
  destinationPath: string;
  partialPath: string;
  totalBytes: number;
  downloadedBytes: number;
  bytesPerSecond: number;
  etaSeconds?: number;
  expectedSha256?: string;
  actualSha256?: string;
  sourceEtag?: string;
  checksumSource: string;
  retryCount: number;
  createdAt: string;
  updatedAt: string;
  detail: string;
  error?: string;
}

export interface ContextAttachmentImport {
  attachments: ContextAttachment[];
  failures: string[];
}

export interface OperationProgress {
  stage?: string;
  phase?: string;
  detail: string;
  at?: string;
}

export interface GpuMemoryProcess {
  pid: number;
  name: string;
  executablePath: string;
  memoryMib: number;
  kind: string;
}

export interface VramCleanupPreview {
  gpu?: GpuSnapshot;
  candidates: GpuMemoryProcess[];
  exclusions: GpuMemoryExclusion[];
  candidateMemoryMib: number;
  protectedProcessCount: number;
}

export interface GpuMemoryExclusion {
  process: GpuMemoryProcess;
  reason: string;
  canInclude: boolean;
}

export interface GpuCleanupFailure {
  process: GpuMemoryProcess;
  detail: string;
  canForceClose: boolean;
  powershellCommand?: string;
}

export interface VramCleanupResult {
  attempted: GpuMemoryProcess[];
  terminated: GpuMemoryProcess[];
  failed: GpuCleanupFailure[];
  beforeGpu?: GpuSnapshot;
  afterGpu?: GpuSnapshot;
  freedMib: number;
  message: string;
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
