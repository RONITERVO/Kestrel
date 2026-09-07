import type { ImageProject, MovieProducerWorkspace, MovieProject, MovieStudioConversation, MusicProject } from "../contracts/index";

const date = "2026-09-07T10:00:00Z";
const draft = { status: "draft", phase: "draft", detail: "Sample project for UI development", error: "", createdAt: date, updatedAt: date };

export const sampleImage = {
  ...draft, schemaVersion: 3, id: "preview-image", title: "Night / Form", idea: "A poster for a night exhibition.",
  highLevelDescription: "A restrained yellow and black exhibition poster with generous margins.",
  style: { mode: "art", aesthetics: "Swiss editorial grid", lighting: "Flat graphic light", photo: "", artStyle: "Geometric illustration", medium: "Risograph print", colorPalette: ["#111111", "#F2C14E"] },
  background: "Matte black paper", elements: [{ id: "preview-title", kind: "text", bbox: [80, 100, 320, 900], text: "NIGHT / FORM", description: "Large upright bold sans-serif title", colorPalette: ["#F2C14E"] }],
  settings: { width: 1024, height: 1024, preset: "standard", seed: 42, batchSize: 1, comfyRoot: "" },
  takes: [], activeTakeId: "", licenseNotice: "Sample composition; no generated media.",
} satisfies ImageProject;

export const sampleMusic = {
  ...draft, schemaVersion: 2, id: "preview-music", title: "Night Signal", idea: "A warm piano song about a light across the bay.",
  caption: "Intimate piano, brushed percussion, warm vocal, gradual chorus lift.", instrumental: false,
  sections: [
    { id: "preview-verse", tag: "Verse", name: "Verse 1", bars: 8, lyrics: "Across the water, one light stays\nA quiet signal through the haze", direction: "Close voice and soft piano" },
    { id: "preview-chorus", tag: "Chorus", name: "Chorus", bars: 8, lyrics: "Keep the light on\nI am coming home", direction: "Open the harmony" },
  ],
  settings: { maxDurationSeconds: 120, steps: 32, cfgScale: 3, topK: 50, seed: 42, tiledDecode: true, modelVariant: "auto", comfyRoot: "" },
  midi: { executablePath: "", modelPath: "", instruments: "piano" }, takes: [], activeTakeId: "",
} satisfies MusicProject;

export const sampleMovie = {
  ...draft, schemaVersion: 8, id: "preview-movie", title: "Tomorrow's Weather", prompt: "A lighthouse keeper hears tomorrow's weather.",
  status: "awaiting-review", phase: "story-draft", model: "Local story collaborator", renderer: "H3",
  settings: { width: 768, height: 448, clipSeconds: 5, steps: 20, maxClips: 12, seed: 42, temperature: .45, topP: .9, topK: 20, thinkingBudget: 32768, maxOutputTokens: 32768, contextWindow: 0, comfyRoot: "", refImageSize: "match" },
  plan: null, references: [], sources: [], clips: [],
  edit: { clips: [], exportTitle: "Tomorrow's Weather", exportPreset: "publish", normalizeAudio: false, targetLufs: -14, markers: [] },
  finalPath: "", exports: [], producerReviewRequired: true, producerApprovedAt: "",
} satisfies MovieProject;

export const sampleConversation = {
  id: "preview-conversation", kind: "story", title: "Story room", createdAt: date, updatedAt: date,
  storyRevisionId: "preview-story", summary: "", archived: false, messages: [],
} satisfies MovieStudioConversation;

export const sampleWorkspace = {
  schemaVersion: 1, projectId: sampleMovie.id, createdAt: date, updatedAt: date,
  activeStoryRevisionId: "preview-story", activeStoryConversationId: sampleConversation.id,
  storyRevisions: [{ id: "preview-story", number: 1, createdAt: date, origin: "producer", instruction: "Sample brief", markdown: "# Tomorrow's Weather\n\nMara hears a storm forecast one day early. She must decide whether to trust the voice on the radio before the fishing fleet leaves the harbour." }],
  conversations: [{ ...sampleConversation, messageCount: 0 }], scenes: [], sceneRevision: 0,
} satisfies MovieProducerWorkspace;
