import type {
  H3EnvironmentSnapshot, H3Scene, H3TakePlacementGeometry, H3WindowCompileResult,
  ImageProject, MovieImageAssetGeneration, MovieProject, MusicLyricsDocument,
  MusicProject,
} from "../contracts/index";

const date = "2026-09-07T10:00:00Z";
const draft = { status: "draft", phase: "draft", detail: "Sample project for UI development", error: "", createdAt: date, updatedAt: date };

export const sampleImage = {
  ...draft, schemaVersion: 5, id: "preview-image", title: "Night / Form", idea: "A poster for a night exhibition.",
  highLevelDescription: "A restrained yellow and black exhibition poster with generous margins.",
  style: { mode: "art", aesthetics: "Swiss editorial grid", lighting: "Flat graphic light", photo: "", artStyle: "Geometric illustration", medium: "Risograph print", colorPalette: ["#111111", "#F2C14E"] },
  background: "Matte black paper", elements: [{ id: "preview-title", kind: "text", bbox: [80, 100, 320, 900], text: "NIGHT / FORM", description: "Large upright bold sans-serif title", colorPalette: ["#F2C14E"] }],
  settings: { promptMode: "composition", plainPrompt: "A poster for a night exhibition.", width: 1024, height: 1024, preset: "standard", seed: 42, batchSize: 1, negativePrompt: "worst quality, bad anatomy, watermark, jpeg artifact", comfyRoot: "" },
  takes: [], activeTakeId: "", licenseNotice: "Sample composition; no generated media.",
} satisfies ImageProject;

export const sampleMusic = {
  ...draft, schemaVersion: 2, id: "preview-music", title: "Night Signal", idea: "A warm piano song about a light across the bay.",
  caption: "Intimate piano, brushed percussion, warm vocal, gradual chorus lift.", instrumental: false,
  sections: [
    { id: "preview-verse", tag: "Verse", name: "Verse 1", bars: 8, lyrics: "Across the water, one light stays\nA quiet signal through the haze", direction: "Close voice and soft piano" },
    { id: "preview-chorus", tag: "Chorus", name: "Chorus", bars: 8, lyrics: "Keep the light on\nI am coming home", direction: "Open the harmony" },
  ],
  settings: { backend: "minimaxMusic3", planningMode: "full", abcScore: "", maxDurationSeconds: 120, steps: 32, cfgScale: 3, topK: 50, seed: 42, tiledDecode: true, modelVariant: "auto", comfyRoot: "" },
  midi: { executablePath: "", modelPath: "", instruments: "piano" }, takes: [], activeTakeId: "",
} satisfies MusicProject;

const previewGeometry: H3TakePlacementGeometry = {
  policy: "sonder-h3-v1",
  fps: 24,
  renderStartSec: 0,
  renderEndSec: 5,
  selectionInSec: 0,
  selectionOutSec: 5,
  maskStartFrame: 0,
  maskEndFrame: 120,
  unpaddedFrames: 120,
  paddedFrames: 124,
  frameCountPadding: 4,
  videoLatentTokens: 32,
  videoMaskTokenLo: 0,
  videoMaskTokenHi: 32,
  audioLatentTokens: 125,
  audioMaskTokenLo: 0,
  audioMaskTokenHi: 125,
  trimmedTimelineStartSec: 0,
  trimmedDurationSec: 5,
  trimmedSourceInSec: 0,
  untrimmedTimelineStartSec: 0,
  untrimmedDurationSec: 5,
};

const makePreviewScene = (id: string, name: string, kind: H3Scene["kind"], width: number, height: number): H3Scene => ({
  id,
  name,
  kind,
  width,
  height,
  fps: 24,
  durationSec: 15,
  window: {
    inSec: 0,
    outSec: 5,
    preContextSec: 0,
    postContextSec: 0,
    maskStartOffsetSec: 0,
    maskEndOffsetSec: 0,
    minOverlapRatio: 0.5,
  },
  generation: {
    passPreset: "draft02Mp",
    taskMode: "standard",
    promptFormat: "h3FullReference",
    modelChoice: "auto",
    placementMode: "trimmed",
    customWidth: width,
    customHeight: height,
    steps: 10,
    cfg: 1,
    denoise: 1,
    samplerName: "er_sde",
    scheduler: "sgm_uniform",
    shiftVideo: 3,
    shiftAudio: 1,
    seed: 42,
    randomizeSeed: false,
    refImageSize: "match",
    colorDriftCorrection: true,
    loras: [],
  },
  globalPrompt: {
    summaryLead: "Animated picture-book watercolor short film.",
    detailedLead: "Soft hand-painted grain, warm morning pond light.",
    overallSoundscape: "Gentle pond ripples, reeds rustling in breeze",
    nonDiegeticMusic: "Quiet acoustic harp motif",
    negativePrompt: "worst quality, watermark, text subtitles",
  },
  promptSections: [
    {
      id: `${id}-shot-1`,
      label: "The Pond Inside a Shadow",
      timelineStartSec: 0,
      durationSec: 5,
      summaryBeat: "@Pip counts lily pads at @Pond as @Crow's shadow crosses the water.",
      detailedAction: "Medium close-up on @Pip sitting on a wide lily pad at @Pond. @Crow lands on a bent reed overhead.",
      cameraAndLighting: "35mm storybook lens, soft dawn mist, dappled golden backlight",
      soundscapeOverride: "",
      musicOverride: "",
      vocalEvents: [
        {
          id: "voc-1",
          speakerHandle: "Crow",
          eventType: "dialogue",
          language: "English",
          delivery: "dry, amused",
          text: "The sky is larger than any pond, little counter.",
        },
      ],
      enabled: true,
    },
  ],
  videoLanes: [{ id: `${id}-v1`, name: "V1", muted: false, solo: false, locked: false, clips: [] }],
  audioLanes: [{ id: `${id}-a1`, name: "A1", muted: false, solo: false, locked: false, clips: [] }],
  driverLanes: [{ id: `${id}-d1`, name: "D1", muted: false, locked: false, clips: [] }],
  referenceLanes: [{
    id: `${id}-r1`,
    name: "R1",
    muted: false,
    locked: false,
    items: [
      {
        id: `${id}-ref-pip`,
        entityId: "ent-pip",
        label: "@Pip",
        descriptionOverride: "",
        timelineStartSec: 0,
        durationSec: 15,
        useVisual: true,
        useAudio: true,
        enabled: true,
      },
      {
        id: `${id}-ref-crow`,
        entityId: "ent-crow",
        label: "@Crow",
        descriptionOverride: "",
        timelineStartSec: 0,
        durationSec: 10,
        useVisual: true,
        useAudio: true,
        enabled: true,
      },
    ],
  }],
  guides: [],
  createdAt: date,
  updatedAt: date,
});

export const sampleMovie = {
  ...draft,
  schemaVersion: 9,
  id: "preview-movie",
  title: "Tomorrow's Weather",
  prompt: "A lighthouse keeper hears tomorrow's weather.",
  status: "draft",
  phase: "h3-studio",
  model: "MiniMax H3",
  renderer: "H3",
  settings: { width: 1920, height: 1088, clipSeconds: 5, steps: 10, maxClips: 12, seed: 42, temperature: .45, topP: .9, topK: 20, thinkingBudget: 32768, maxOutputTokens: 32768, contextWindow: 0, comfyRoot: "", refImageSize: "match" },
  plan: null,
  references: [],
  sources: [],
  clips: [],
  edit: { clips: [], exportTitle: "Tomorrow's Weather", exportPreset: "publish", normalizeAudio: false, targetLufs: -14, markers: [] },
  finalPath: "",
  exports: [],
  producerReviewRequired: false,
  producerApprovedAt: date,
  storyMarkdown: "# Tomorrow's Weather\n\n@Mara hears a storm forecast one day early at @HarborLighthouse.",
  storyRevisions: [
    { id: "rev-1", number: 1, createdAt: date, origin: "producer", instruction: "Initial brief", markdown: "# Tomorrow's Weather\n\n@Mara hears a storm forecast one day early at @HarborLighthouse." },
  ],
  entities: [],
  scenes: [makePreviewScene("scene-weather-1", "Main Scene", "standard", 1920, 1088)],
  activeSceneId: "scene-weather-1",
  renderJobs: [],
} satisfies MovieProject;

const frogFolder = String.raw`C:\Kestrel Preview\Movies`;
const frogStory = [
  "# The Frog Who Measured the Sky",
  "**Logline:** @Pip believes @Pond is the entire world until @Crow claims the sky is bigger than the water.",
  "**Tone:** Quiet, lyrical watercolor short with synchronized pond foley and character voices.",
].join("\n\n");

const frogMainScene = makePreviewScene("frog-main", "Main Scene", "standard", 1920, 1088);
frogMainScene.videoLanes[0].clips = [{
  id: "vclip-1",
  sourceKind: "take",
  sourceId: "job-frog-1",
  label: "Main Scene [0.00s..5.00s] (Pass 3 1080p)",
  timelineStartSec: 0,
  durationSec: 5,
  sourceInSec: 0,
  sourceDurationSec: 5,
  opacity: 1,
  audioGain: 1,
  includeEmbeddedAudio: true,
  enabled: true,
}];

/** A finished sample production so every H3 Studio room can be inspected in browser preview. */
export const sampleFinishedMovie = {
  ...sampleMovie,
  id: "preview-frog",
  title: "The Frog Who Measured the Sky",
  prompt: "A frog tries to measure the sky.",
  status: "complete",
  phase: "complete",
  detail: "3-pass Sonder quality ladder complete; master published.",
  storyMarkdown: frogStory,
  storyRevisions: [
    { id: "frog-rev-1", number: 1, createdAt: date, origin: "producer", instruction: "Initial story bible", markdown: frogStory },
  ],
  entities: [
    {
      id: "ent-pip",
      handle: "Pip",
      displayName: "Pip the Pond Frog",
      kind: "character",
      subjectDescription: "a small emerald tree frog with wide amber eyes and a reed measuring stick",
      retentionLevel: "fullyPreserved",
      defaultVisualIntent: "preserve",
      defaultAudioIntent: "voiceIdentity",
      members: [],
      createdAt: date,
      updatedAt: date,
    },
    {
      id: "ent-crow",
      handle: "Crow",
      displayName: "Old Harbor Crow",
      kind: "character",
      subjectDescription: "a weathered charcoal crow with silver-tipped wing feathers",
      retentionLevel: "fullyPreserved",
      defaultVisualIntent: "preserve",
      defaultAudioIntent: "voiceIdentity",
      members: [],
      createdAt: date,
      updatedAt: date,
    },
    {
      id: "ent-pond",
      handle: "Pond",
      displayName: "Shadowed Lily Pond",
      kind: "location",
      subjectDescription: "a quiet circular lily pond ringed by tall cattails and morning mist",
      retentionLevel: "reference",
      defaultVisualIntent: "context",
      defaultAudioIntent: "soundTexture",
      members: [],
      createdAt: date,
      updatedAt: date,
    },
  ],
  scenes: [
    frogMainScene,
    makePreviewScene("frog-audition", "Audition 1", "audition", 960, 544),
    makePreviewScene("frog-score", "Soundtrack 1", "soundtrack", 64, 32),
  ],
  activeSceneId: "frog-main",
  renderJobs: [
    {
      id: "job-frog-1",
      projectId: "preview-frog",
      sceneId: "frog-main",
      sceneName: "Main Scene",
      label: "Main Scene [0.00s..5.00s] (Pass 3 1080p)",
      status: "complete",
      passPreset: "upscale1080p",
      taskMode: "standard",
      placementMode: "trimmed",
      width: 1920,
      height: 1088,
      steps: 4,
      denoise: 0.4,
      seed: 17887344,
      compiledPrompt: "Overall Summary: <Subject 1> counts lily pads as <Subject 2>'s shadow crosses the water.\nSubjects:\n<Subject 1>: a small emerald tree frog with wide amber eyes and a reed measuring stick.\n<Subject 2>: a weathered charcoal crow with silver-tipped wing feathers.\nDetailed Description: [Shot 1] Medium close-up on <Subject 1> sitting on a wide lily pad. <Subject 2> lands on a bent reed overhead.\nVocal Events: <Subject 2> (dry, amused): <d>[English] The sky is larger than any pond, little counter.</d>\nOverall Soundscape: Gentle pond ripples, reeds rustling in breeze.\nNon-Diegetic Music: Quiet acoustic harp motif.",
      resolvedModel: "minimax_h3_ref2va_pruned_int8_convrot.safetensors",
      geometry: previewGeometry,
      outputClipId: "vclip-1",
      takeMediaPath: `${frogFolder}\\frog-1.mp4`,
      takeSha256: "c0d9e0af6be5675a",
      colorDrift: {
        applied: true,
        matchedContextFrames: 8,
        maxBiasAbs: 0.0082,
        maxMatrixDeviation: 0.0145,
        summary: "Ridge affine RGB match applied across 8 context frames.",
      },
      quality: {
        verified: true,
        width: 1920,
        height: 1088,
        fps: 24,
        frameCount: 120,
        durationSec: 5,
        hasAudio: true,
        audioPeakDbfs: -1.4,
        audioRmsDbfs: -18.6,
        audioIsSilent: false,
        contactSheetPaths: [],
        warnings: [],
      },
      error: "",
      createdAt: date,
      updatedAt: date,
    },
  ],
  clips: [
    { id: "frog-1", index: 0, title: "The Pond Inside a Shadow", prompt: "Animated picture-book watercolor.", durationSeconds: 5, seed: 17887344, status: "complete", path: `${frogFolder}\\frog-1.mp4`, error: "", versions: [] },
  ],
  exports: [
    { id: "frog-export-1", createdAt: date, title: "The Frog Who Measured the Sky", preset: "publish", path: `${frogFolder}\\frog-publish.mp4`, bytes: 18400000, sha256: "c0d9e0af6be5675a", durationSeconds: 5, clipCount: 1 },
  ],
  finalPath: `${frogFolder}\\frog-publish.mp4`,
} satisfies MovieProject;

export const sampleH3Environment: H3EnvironmentSnapshot = {
  ready: true,
  comfyRoot: String.raw`C:\Kestrel\ComfyUI`,
  hasRef2va: true,
  hasFl2va: true,
  hasQwen3vl: true,
  hasVideoVae: true,
  hasAudioVae: true,
  hasTaeh3Preview: true,
  hasRef2vTurboLora: true,
  hasFl2vTurboLora: true,
  discoveredLoras: [],
  detail: "Preview H3 environment ready.",
};

export const sampleH3CompileResult: H3WindowCompileResult = {
  sceneId: "frog-main",
  sceneName: "Main Scene",
  resolvedWidth: 608,
  resolvedHeight: 352,
  resolvedModel: "minimax_h3_ref2va_pruned_int8_convrot.safetensors",
  resolvedSteps: 10,
  resolvedDenoise: 1,
  usesSourceAvLatent: false,
  usesVideoNoiseMask: false,
  usesAudioNoiseMask: false,
  resolvedLoras: [{ fileName: "MiniMax-H3-Ref2V-Turbo-LoRA.safetensors", strengthModel: 0.8, enabled: true }],
  promptFormat: "h3FullReference",
  compiledPrompt: sampleFinishedMovie.renderJobs[0].compiledPrompt,
  negativePrompt: "worst quality, watermark, text subtitles",
  channels: {
    subjectDefinitions: "<Subject 1>: a small emerald tree frog with wide amber eyes and a reed measuring stick.\n<Subject 2>: a weathered charcoal crow with silver-tipped wing feathers.",
    summary: "<Subject 1> counts lily pads as <Subject 2>'s shadow crosses the water.",
    retentionAnalysis: "<Subject 1> is fully preserved; <Subject 2> is fully preserved.",
    detailedDescription: "[Shot 1] Medium close-up on <Subject 1> sitting on a wide lily pad. <Subject 2> lands on a bent reed overhead.",
    overallSoundscape: "Gentle pond ripples, reeds rustling in breeze",
    nonDiegeticMusic: "Quiet acoustic harp motif",
  },
  ordinals: {
    subjects: [
      { subjectIndex: 1, subjectTag: "<Subject 1>", speakerCode: "(S1)", handle: "Pip", displayName: "Pip the Pond Frog", entityId: "ent-pip", visualTags: ["<Picture 1>"], audioTags: ["<Audio 1>"], subjectDescription: "a small emerald tree frog with wide amber eyes and a reed measuring stick", retention: "fullyPreserved", shotIndices: [1] },
      { subjectIndex: 2, subjectTag: "<Subject 2>", speakerCode: "(S2)", handle: "Crow", displayName: "Old Harbor Crow", entityId: "ent-crow", visualTags: ["<Picture 2>"], audioTags: ["<Audio 2>"], subjectDescription: "a weathered charcoal crow with silver-tipped wing feathers", retention: "fullyPreserved", shotIndices: [1] },
    ],
    pictures: [],
    videos: [],
    audios: [],
  },
  verdicts: [
    { itemId: "frog-main-ref-pip", laneId: "frog-main-r1", label: "@Pip", handle: "Pip", verdict: "winner", overlapRatio: 1, assignedTags: ["<Subject 1>"], detail: "Active in window" },
    { itemId: "frog-main-ref-crow", laneId: "frog-main-r1", label: "@Crow", handle: "Crow", verdict: "winner", overlapRatio: 1, assignedTags: ["<Subject 2>"], detail: "Active in window" },
  ],
  guides: [],
  geometry: previewGeometry,
  advisories: [],
};

const signalLines = [
  "Across the water, one light stays", "A quiet signal through the haze", "I have been counting every wave",
  "Keep the light on", "I am coming home", "Keep the light on", "Across the bay I hear you call",
  "The harbor sleeps, the stars are small", "Keep the light on", "I am coming home",
];
const signalTake = {
  id: "preview-take", createdAt: date, status: "complete", detail: "", error: "", path: "", bytes: 0, sha256: "preview",
  durationSeconds: 151, seed: 42, resolvedModel: "Preview music model", caption: sampleMusic.caption,
  lyrics: signalLines.join("\n"), promptId: "preview", exactGraph: {}, abcScore: "", midiPath: "", midiReceiptPath: "", midiSourcePath: "",
  midiDocumentPath: "", midiRevision: 0, lyricsDocumentPath: "preview-lyrics.json", lyricsReceiptPath: "", lyricsRevision: 3,
};

/** A song with one finished take and timed lyrics, so the lyric stage and timing editor can be previewed. */
export const sampleFinishedMusic = {
  ...sampleMusic, status: "complete", phase: "complete", takes: [signalTake], activeTakeId: signalTake.id,
} satisfies MusicProject;

export const sampleLyricsDocument = {
  schemaVersion: 1, takeId: signalTake.id, sourceSha256: "preview", revision: 3, language: "en", source: "local-sync",
  transcript: signalLines.join("\n"), theme: "sketchbook", showTranslation: true, translationLanguage: "Spanish",
  translationModelId: "", createdAt: date, updatedAt: date,
  segments: signalLines.map((primary, index) => {
    const start = 8 + index * 13;
    const words = primary.split(" ");
    return {
      id: `preview-cue-${index + 1}`, start, end: start + 9, primary, translation: "",
      words: words.map((value, word) => ({ value, start: start + (word * 9) / words.length, end: start + ((word + 1) * 9) / words.length })),
    };
  }),
} satisfies MusicLyricsDocument;

/** Saved Studio images for the image creator's library page. The pictures themselves need the desktop app. */
export const sampleImageAssets: MovieImageAssetGeneration[] = [{
  id: "preview-assets", status: "complete", stage: "complete", detail: "Saved", prompt: "A beekeeper in a white suit at the edge of a field",
  renderedPrompt: "", width: 768, height: 448, steps: 20, seed: 42, stabilize: true, workflow: "", workflowSource: "", workflowRevision: "",
  previewNodeRevision: "", previewDecoderRevision: "", previewDecoderSha256: "", requestedLength: 12, resolvedFrameCount: 12, candidateStart: 8,
  candidateCount: 7, comfyPromptId: "", createdAt: date, updatedAt: date, completedAt: date, error: "", exactGraph: {},
  candidates: Array.from({ length: 7 }, (_, index) => ({ frameIndex: index + 8, asset: {
    id: `preview-asset-${index}`, name: `Beekeeper ${index + 1}`, kind: "image", mimeType: "image/png", bytes: 0, durationSeconds: 0,
    width: 768, height: 448, hasAudio: false, path: "", createdAt: date,
  } })),
}];
