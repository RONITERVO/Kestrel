export type ServiceState = "ready" | "starting" | "stopped" | "unavailable";

export interface ServiceStatus {
  modelRuntime: ServiceState;
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

export interface SetupModelAsset {
  id: string;
  component: string;
  label: string;
  fileName: string;
  bytes: number;
  recognized: boolean;
  installedPath: string;
}

export interface SetupSnapshot {
  ready: boolean;
  installRoot: string;
  availableBytes: number;
  gpuName?: string;
  gpuMemoryBytes: number;
  components: SetupComponent[];
  modelAssets: SetupModelAsset[];
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
  acceptIdeogramNonCommercialLicense?: boolean;
  whisperCheckpointPath?: string;
  muscriptorCheckpointPath?: string;
  acceptMuscriptorNonCommercialLicense?: boolean;
  existingModelPaths?: Record<string, string>;
}

export interface SetupProgress {
  component: string;
  stage: string;
  detail: string;
  downloadedBytes: number;
  totalBytes: number;
  bytesPerSecond: number;
}

export type ThinkingLevel = "off" | "low" | "medium" | "high" | "max";

export function thinkingBudgetForLevel(level: ThinkingLevel, maxOutputTokens = 32768): number {
  switch (level) {
    case "off": return 0;
    case "low": return Math.min(2048, maxOutputTokens);
    case "medium": return Math.min(8192, maxOutputTokens);
    case "high": return Math.min(16384, maxOutputTokens);
    case "max": return maxOutputTokens;
  }
}

export function thinkingLevelFromBudget(budget: number): ThinkingLevel {
  if (budget <= 0) return "off";
  if (budget <= 2048) return "low";
  if (budget <= 8192) return "medium";
  if (budget <= 20000) return "high";
  return "max";
}

export function effectiveThinkingLevelForModel(
  control?: ControlSettings,
  modelId?: string,
): ThinkingLevel {
  if (!control) return "high";
  if (modelId) {
    const override = control.modelOverrides?.find((item) => item.modelId === modelId);
    if (override?.thinkingLevel) {
      return override.thinkingLevel;
    }
  }
  return control.thinkingLevel ?? "high";
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

export interface MovieFrameAnchor {
  editId: string;
  timeSeconds: number;
  label?: string;
}

export interface MovieRenderState {
  active: boolean;
  preview?: MovieRenderPreviewEvent;
}

export interface MovieCapturedFrame {
  anchor: MovieFrameAnchor;
  path: string;
}

export type MovieTransitionPosition = "before" | "between" | "after";
export type MovieTransitionPlacement = "add_to_masters" | "insert_before_right" | "insert_after_left" | "replace_range";

export interface MovieFl2vTransitionRequest {
  id: string;
  position: MovieTransitionPosition;
  firstAnchor?: MovieFrameAnchor;
  lastAnchor?: MovieFrameAnchor;
  prompt: string;
  durationSeconds: number;
  seed?: number;
  placement: MovieTransitionPlacement;
}

export type MovieGenerationTask =
  | { kind: "shotVersion"; clipId: string; direction: string }
  | { kind: "transition"; position: MovieTransitionPosition; firstAnchor?: MovieFrameAnchor; lastAnchor?: MovieFrameAnchor; direction: string; durationSeconds: number };

export type MovieGenerationCandidate =
  | { kind: "shotVersion"; clipId: string; clip: PlannedClip; checklist: string[] }
  | { kind: "transition"; motionPrompt: string; durationSeconds: number; cameraMotion: string; subjectMotion: string; transitionNotes: string; checklist: string[] };

export interface MovieGenerationAgentRequest {
  requestId: string;
  projectId: string;
  task: MovieGenerationTask;
  thinkingLevel?: ThinkingLevel;
  frameAnalystModelId?: string;
}

export interface MovieGenerationProposal {
  summary: string;
  reviewSummary: string;
  candidate: MovieGenerationCandidate;
}

export interface MovieGenerationAgentEvent {
  sequence: number;
  requestId: string;
  projectId: string;
  kind: "turn-start" | "reasoning" | "token" | "activity" | "advanced-token" | "complete" | string;
  modelRole: "director" | "reviewer" | string;
  content: string;
  at: string;
  completionMarkerSeen?: boolean;
  finishReason?: string;
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

export type MovieCopilotWorkspace = "generate" | "edit" | "deliver";

export interface MovieCopilotTurn {
  id: string;
  createdAt: string;
  workspace: MovieCopilotWorkspace;
  producerRequest: string;
  modelId: string;
  response: string;
  status: string;
  proposalSummary: string;
}

export interface MovieCopilotRequest {
  requestId: string;
  projectId: string;
  modelId: string;
  workspace: MovieCopilotWorkspace;
  instruction: string;
  edit: MovieEdit;
  thinkingLevel?: ThinkingLevel;
}

export interface MovieCopilotReceipt {
  systemPrompt: string;
  messages: unknown[];
  toolSchema: unknown;
  exactRequest: unknown;
  lintResult: string;
}

export interface MovieCopilotProposal {
  summary: string;
  changes: string[];
  edit: MovieEdit;
}

export interface MovieCopilotEvent {
  requestId: string;
  projectId: string;
  kind: "queued" | "started" | "reasoning" | "token" | "advanced-token" | "complete" | "cancelled" | "error" | "settled" | string;
  content?: string;
  modelName?: string;
  thinkingLevel?: ThinkingLevel;
  receipt?: MovieCopilotReceipt;
  proposal?: MovieCopilotProposal;
  at: string;
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

export interface MovieModelRoleRequest {
  directorModelId: string;
  reviewerModelId: string;
  directorThinkingLevel?: ThinkingLevel;
  reviewerThinkingLevel?: ThinkingLevel;
}

export interface MovieRuntimePolicyRequest {
  contextWindow: number;
  maxOutputTokens: number;
}

export interface MovieModelBinding {
  modelId: string;
  modelName: string;
  compatibilityTier: string;
  protocolRevision: string;
  boundAt: string;
  thinkingLevel?: ThinkingLevel;
}

export interface MovieModelRoles {
  director: MovieModelBinding;
  reviewer: MovieModelBinding;
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
  modelRoles?: MovieModelRoles;
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
  copilotHistory: MovieCopilotTurn[];
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
  reviewerReview: MovieIndependentReview | null;
}

export interface MovieIndependentReviewIssue {
  clipNumber: number;
  category: string;
  finding: string;
  requiredFix: string;
}

export interface MovieIndependentReview {
  summary: string;
  issues: MovieIndependentReviewIssue[];
}

export interface MoviePlanningEvent {
  projectId: string;
  sequence: number;
  kind: "token" | "advanced-token" | "reasoning" | "turn-start" | "turn-complete" | "activity" | "tool-result" | "direction-queued" | "checkpoint-requested" | "checkpoint-saved";
  stage: "planning" | "thinking" | "producer" | "native-check" | "checkpoint" | "model-text" | "tool-arguments" | "list" | "read" | "read_many" | "write" | "write_batch" | "delete" | "check" | "submit";
  text: string;
  modelRole?: "reviewer";
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
  modelRoles: MovieModelRoleRequest;
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

export interface MovieModelRoleRequest {
  directorModelId: string;
  reviewerModelId: string;
}

export interface MovieModelBinding {
  modelId: string;
  modelName: string;
  compatibilityTier: string;
  protocolRevision: string;
  boundAt: string;
}

export interface MovieModelRoles {
  director: MovieModelBinding;
  reviewer: MovieModelBinding;
}

export interface ModelQualificationReceipt {
  modelId: string;
  modelName: string;
  protocolRevision: string;
  engineSha256: string;
  contextWindow: number;
  maxOutputTokens: number;
  passed: boolean;
  checks: string[];
  detail: string;
  checkedAt: string;
}

export interface ModelCompatibility {
  modelId: string;
  modelName: string;
  tier: "release-validated" | "protocol-ready" | "unverified" | "limited-context" | "incompatible" | string;
  studioReady: boolean;
  requiresQualification: boolean;
  detail: string;
  protocolRevision: string;
  receipt?: ModelQualificationReceipt;
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
  thinkingLevel: ThinkingLevel;
  modelOverrides: ModelRuntimeOverride[];
  projectRoot: string;
  agentWorkspaceRoots: string[];
  allowFullAccessAgent: boolean;
  agentMaxSteps: number;
  agentMaxOutputTokens: number;
}

export interface ModelRuntimeOverride {
  modelId: string;
  contextWindow?: number;
  maxOutputTokens?: number;
  threads?: number;
  thinkingLevel?: ThinkingLevel;
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

export interface ProvenHardwareProfile {
  id: string;
  modelPattern: string;
  quantizationPattern?: string;
  displayName: string;
  minVramMib: number;
  maxVramMib?: number;
  recommendedContextWindow: number;
  recommendedMaxOutputTokens: number;
  recommendedThinkingLevel: ThinkingLevel;
  recommendedThreads: number;
  description: string;
  provenSpeedNotes: string;
}

export function findProvenHardwareProfile(
  profiles: ProvenHardwareProfile[] | undefined,
  modelName: string | undefined,
  vramMib: number | undefined,
): ProvenHardwareProfile | undefined {
  if (!profiles?.length || !modelName) return undefined;
  const lower = modelName.toLowerCase();
  return profiles.find((profile) => {
    if (!lower.includes(profile.modelPattern.toLowerCase())) return false;
    if (profile.quantizationPattern && !lower.includes(profile.quantizationPattern.toLowerCase())) {
      return false;
    }
    if (vramMib !== undefined) {
      if (vramMib < profile.minVramMib) return false;
      if (profile.maxVramMib !== undefined && vramMib > profile.maxVramMib) return false;
    }
    return true;
  });
}

export const STANDARD_CONTEXT_OPTIONS: Array<{ value: number; label: string }> = [
  { value: 4096, label: "4k (4,096 tokens)" },
  { value: 8192, label: "8k (8,192 tokens)" },
  { value: 16384, label: "16k (16,384 tokens)" },
  { value: 24576, label: "24k (24,576 tokens)" },
  { value: 32768, label: "32k (32,768 tokens)" },
  { value: 49152, label: "48k (49,152 tokens)" },
  { value: 65536, label: "64k (65,536 tokens)" },
  { value: 131072, label: "128k (131,072 tokens)" },
  { value: 262144, label: "256k (262,144 tokens)" },
];

export const STANDARD_OUTPUT_OPTIONS: Array<{ value: number; label: string }> = [
  { value: 2048, label: "2k (2,048 tokens)" },
  { value: 4096, label: "4k (4,096 tokens)" },
  { value: 8192, label: "8k (8,192 tokens)" },
  { value: 16384, label: "16k (16,384 tokens)" },
  { value: 32768, label: "32k (32,768 tokens)" },
];

export interface ControlSnapshot {
  settings: ControlSettings;
  models: ModelInfo[];
  engineCandidates: EngineCandidate[];
  runtime: ManagedRuntimeSnapshot;
  gpu?: GpuSnapshot;
  developer: DeveloperStatus;
  runtimeLogs: RuntimeLog[];
  provenHardwareProfiles?: ProvenHardwareProfile[];
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

export interface SpeechRecordingAttachment {
  audioRelativePath: string;
  words: SpeechTiming[];
}

export interface ChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  reasoning?: string;
  status?: "interrupted" | "limited";
  attachments?: ContextAttachment[];
  recording?: SpeechRecordingAttachment;
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
  recording?: SpeechRecordingAttachment;
  temperature: number;
  topP: number;
  topK: number;
  maxOutputTokens: number;
  thinkingLevel?: ThinkingLevel;
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
  thinkingLevel?: ThinkingLevel;
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
  control: ControlSettings;
  models: ModelInfo[];
  managedRuntime: ManagedRuntimeSnapshot;
  provenHardwareProfiles?: ProvenHardwareProfile[];
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
