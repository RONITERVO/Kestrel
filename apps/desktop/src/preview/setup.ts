import { installPreviewHandlers, type PreviewHandlers } from "../platform/transport";
import { speechPreferencesDefaults } from "../contracts/index";
import { demoReport, demoSnapshot } from "./fixtures";
import { samplePromptPackText } from "./promptPack";
import { sampleComputerTask } from "./computerTask";
import { sampleConversation, sampleFinishedConversation, sampleFinishedMovie, sampleFinishedMusic, sampleFinishedWorkspace, sampleImage, sampleImageAssets, sampleLyricsDocument, sampleMovie, sampleWorkspace } from "./studioFixtures";

const sampleModels = [
  ...demoSnapshot.control.models,
  ...[["Gemma-4-12B-It", "Q5_K_M", true], ["Jan-v2-VL-high", "Q8_0", false], ["Jan-v3.5-4B", "Q4_K_M", false]].map(([name, quantization, audio]) => ({
    ...demoSnapshot.control.models[0], id: `preview-${String(name).toLowerCase()}`, name: String(name), source: "Jan",
    quantization: String(quantization), supportsAudio: Boolean(audio),
  })),
];
const sampleControl = { ...demoSnapshot.control, models: sampleModels };
const sampleSessions = Array.from({ length: 26 }, (_, index) => ({
  id: `preview-chat-${index + 1}`, title: ["Write a story that lets you reveal the quantization", "Why is red associated with heat", "Describe this image for tiny image generation", "Hello, tell me a story using charts"][index % 4],
  modelId: sampleModels[index % sampleModels.length].id, updatedAt: "2026-08-28T12:00:00Z", messageCount: 2 + (index % 9),
}));

// Sample data is deliberately ephemeral. Unsupported actions explain that they need the desktop.
const handlers = {
  bootstrap: () => demoSnapshot,
  get_report: () => demoReport,
  get_setup_snapshot: () => demoSnapshot.setup,
  get_control_snapshot: () => demoSnapshot.control,
  list_movies: () => [],
  list_music_projects: () => [],
  list_image_projects: () => [],
  list_movie_image_assets: () => [],
  list_chat_sessions: () => [],
  list_computer_tasks: () => [],
  list_model_downloads: () => [],
  get_speech_preferences: () => speechPreferencesDefaults,
  get_prompt_pack_text: () => samplePromptPackText,
  get_setup_profile_text: () => JSON.stringify({ schemaVersion: 1, note: "Sample setup profile. The desktop application shows your real setup here." }, null, 2),
  get_default_prompt_pack_text: () => samplePromptPackText,
  get_local_speech_snapshot: () => ({ narrationAvailable: false, transcriptionAvailable: false,
    comfyReady: false, voices: [], transcribers: [], voiceProfiles: [], defaultVoiceProfileId: "",
    detail: "Speech is available in the desktop application. This browser shows sample data." }),
  get_system_snapshot: () => ({ status: demoSnapshot.status, settings: demoSnapshot.settings,
    runtime: { contextWindow: 98304, maxOutputTokens: 32768, parallelSlots: 1, kvCache: "q4_0 / q4_0", modelVramMib: 9964, modelRoot: "Preview" }, gpu: demoSnapshot.control.gpu ?? undefined, control: demoSnapshot.control.settings,
    models: demoSnapshot.control.models, managedRuntime: demoSnapshot.control.runtime,
    provenHardwareProfiles: demoSnapshot.control.provenHardwareProfiles }),
} satisfies PreviewHandlers;

export function setupPreview(studioSamples = true): void {
  installPreviewHandlers({ ...handlers, ...(studioSamples ? {
    bootstrap: () => ({ ...demoSnapshot, control: sampleControl }),
    get_control_snapshot: () => sampleControl,
    list_chat_sessions: () => sampleSessions,
    list_computer_tasks: () => [{ id: sampleComputerTask.id, objective: sampleComputerTask.objective, modelId: sampleComputerTask.modelId, access: sampleComputerTask.access, status: sampleComputerTask.status, updatedAt: sampleComputerTask.updatedAt, eventCount: sampleComputerTask.events.length, artifactCount: 0 }],
    get_computer_task: () => structuredClone(sampleComputerTask),
    list_movies: () => [{ ...sampleMovie, clipCount: 0 }, { ...sampleFinishedMovie, clipCount: sampleFinishedMovie.clips.length }],
    get_movie: ({ id }) => structuredClone(id === sampleFinishedMovie.id ? sampleFinishedMovie : sampleMovie),
    get_movie_producer_workspace: ({ id }) => structuredClone(id === sampleFinishedMovie.id ? sampleFinishedWorkspace : sampleWorkspace),
    get_movie_studio_conversation: ({ projectId }) => structuredClone(projectId === sampleFinishedMovie.id ? sampleFinishedConversation : sampleConversation),
    list_movie_image_assets: () => structuredClone(sampleImageAssets),
    get_movie_editor_state: () => ({ schemaVersion: 1, projectId: sampleMovie.id, editHash: "preview", jobs: [] }),
    list_image_projects: () => [{ ...sampleImage, takeCount: 0, activeTakePath: "" }],
    get_image_project: () => structuredClone(sampleImage),
    list_music_projects: () => [{ ...sampleFinishedMusic, takeCount: 1, activeTakePath: "" }],
    get_music_project: () => structuredClone(sampleFinishedMusic),
    get_music_lyrics_document: () => structuredClone({ project: sampleFinishedMusic, document: sampleLyricsDocument }),
  } satisfies PreviewHandlers : {}) });
}
