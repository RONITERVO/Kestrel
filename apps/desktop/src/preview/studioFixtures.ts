import type { ImageProject, MovieImageAssetGeneration, MovieProducerWorkspace, MovieProject, MovieStudioConversation, MusicLyricsDocument, MusicProject } from "../contracts/index";

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


const frogScenes = [
  ["The Pond Inside a Shadow", "Establish Pip's small pond and the crow shadow that begins his journey."],
  ["The First Leap", "Pip leaves the only water he has known."],
  ["The Crow Lands", "The crow claims the sky is larger than any pond."],
  ["The Proof Begins", "Pip starts measuring, one lily pad at a time."],
];
const frogFolder = String.raw`C:\Kestrel Preview\Movies`;
const frogStory = [
  "# The Frog Who Measured the Sky",
  "## First Story Sketch",
  "**Logline:** A small frog who believes his pond is the entire world follows a crow's impossible claim that the sky is bigger than the water.",
  "**Format:** Animated short / picture book.",
  "**Tone:** Quiet, lyrical, gently funny.",
  "## Main Character",
  "Pip is a small frog with a voice like a wet stone. He is curious, anxious, and quick to count the lily pads around him.",
].join("\n\n");

/** A finished sample production so every Studio room can be designed in the browser. */
export const sampleFinishedMovie = {
  ...sampleMovie, id: "preview-frog", title: "The Frog Who Measured the Sky", prompt: "A frog tries to measure the sky.",
  status: "complete", phase: "complete", detail: "A new immutable, non-destructive timeline export is ready.",
  clips: frogScenes.map(([title], index) => ({ id: `frog-${index + 1}`, index, title, prompt: `Animated picture-book watercolor. ${title}.`, durationSeconds: 5, seed: 17887344 + index, status: "complete", path: `${frogFolder}\\frog-${index + 1}.mp4`, error: "", versions: [] })),
  edit: { clips: frogScenes.map((_, index) => ({ id: `edit-${index + 1}`, clipId: `frog-${index + 1}`, enabled: true, order: index, trimStart: 0, trimEnd: 0, audioGain: 1, sourceVersionId: "", speed: 1, fadeIn: 0, fadeOut: 0, audioFadeIn: 0, audioFadeOut: 0, label: "", notes: "" })), exportTitle: "The Frog Who Measured the Sky", exportPreset: "publish", normalizeAudio: false, targetLufs: -14, markers: [] },
  exports: [{ id: "frog-export-1", createdAt: date, title: "The Frog Who Measured the Sky", preset: "publish", path: `${frogFolder}\\frog-publish.mp4`, bytes: 18400000, sha256: "c0d9e0af6be5675a", durationSeconds: 20, clipCount: 4 }],
  finalPath: `${frogFolder}\\frog-publish.mp4`, producerReviewRequired: false, producerApprovedAt: date,
} satisfies MovieProject;

export const sampleFinishedWorkspace = {
  ...sampleWorkspace, projectId: sampleFinishedMovie.id, acceptedStoryRevisionId: "frog-story", activeStoryRevisionId: "frog-story",
  storyRevisions: [{ id: "frog-story", number: 1, createdAt: date, origin: "collaborator", instruction: "First sketch", markdown: frogStory }],
  scenes: frogScenes.map(([title, purpose], index) => ({ id: `frog-${index + 1}`, revision: 1, title, purpose, durationSeconds: 5, h3Prompt: `Animated short / picture book, soft watercolor texture. ${purpose}`, continuityIn: index ? "Continues from the previous scene." : "Opening image.", continuityOut: "Pip looks up.", transition: "Cut", references: [], storyRevisionId: "frog-story", createdAt: date, updatedAt: date })),
  sceneRevision: 1,
} satisfies MovieProducerWorkspace;

const signalLines = [
  "Across the water, one light stays", "A quiet signal through the haze", "I have been counting every wave",
  "Keep the light on", "I am coming home", "Keep the light on", "Across the bay I hear you call",
  "The harbor sleeps, the stars are small", "Keep the light on", "I am coming home",
];
const signalTake = {
  id: "preview-take", createdAt: date, status: "complete", detail: "", error: "", path: "", bytes: 0, sha256: "preview",
  durationSeconds: 151, seed: 42, resolvedModel: "Preview music model", caption: sampleMusic.caption,
  lyrics: signalLines.join("\n"), promptId: "preview", exactGraph: {}, midiPath: "", midiReceiptPath: "", midiSourcePath: "",
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

const collaboratorReply = [
  "## Revised Story Sketch",
  "**Logline:** A small frog who believes his pond is the entire world follows a crow's impossible claim that the sky is bigger than the water, and learns that the sky is not a place to reach but a distance you become by leaping.",
  "**Premise:** In a pond so small it fits inside a single shadow, a young frog named Pip believes every frog should live where the water ends. When a crow tells him the sky stretches beyond every pond, Pip sets out to prove the crow wrong. Along the way, he meets creatures whose definitions of home are larger than his own, and a storm that teaches him the sky is not a wall but a door. At the end, Pip returns to his pond, but it is no longer the edge of the world. It is the first place he chose.",
  "## Main Character",
  "**Pip** is a small frog with a voice like a wet stone. He is curious, anxious, and quick to count the lily pads around him. His flaw is that he mistakes the edge of the known world for the edge of the possible world.",
  "## Supporting Characters",
  "- **Crow:** A grizzled, sardonic traveler who has seen more world than he can explain.\n- **The Heron:** Patient and still, she measures depth by waiting rather than by diving.\n- **The Snail Choir:** Three snails who sing the weather a day late, which makes them oddly reassuring.",
  "## Scene Beats",
  "1. Pip counts every lily pad at dawn and announces the total to nobody in particular.\n2. The crow lands on the reed, laughs at the count, and claims the sky is bigger than the water.\n3. Pip measures the sky with a stick, a leaf and finally a very long jump.\n4. The storm arrives; Pip shelters under the heron's wing and hears the snails sing yesterday's rain.\n5. Morning: the pond looks smaller, and Pip is not afraid of that.",
  "## Tone Notes",
  "Keep the humor in the gaps between what Pip says and what the camera shows. The storm should feel dangerous for two beats only, then turn beautiful. Let silence carry the ending; no voice-over explains the lesson, and the final image is Pip choosing where to sit.",
].join("\n\n");

/** A long story-room exchange so the collaborator's page breaks can be checked. */
export const sampleFinishedConversation = {
  id: "frog-conversation", kind: "story", title: "Story room", createdAt: date, updatedAt: date,
  storyRevisionId: "frog-story", summary: "", archived: false,
  messages: [
    { id: "frog-message-1", createdAt: date, role: "producer", markdown: "Make the crow more sardonic and give the storm a clearer turn.", selectedSceneIds: [] },
    { id: "frog-message-2", createdAt: date, role: "collaborator", markdown: collaboratorReply, storyRevisionId: "frog-story", selectedSceneIds: [] },
    { id: "frog-message-3", createdAt: date, role: "producer", markdown: "Good. Keep the ending silent and let Pip choose where to sit.", selectedSceneIds: [] },
    { id: "frog-message-4", createdAt: date, role: "collaborator", markdown: collaboratorReply.replace("## Revised Story Sketch", "## Quieter Ending"), storyRevisionId: "frog-story", selectedSceneIds: [] },
  ],
} satisfies MovieStudioConversation;
