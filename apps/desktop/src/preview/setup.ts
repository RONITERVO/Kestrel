import { installPreviewHandlers, type PreviewHandlers } from "../platform/transport";
import { speechPreferencesDefaults } from "../contracts/index";
import { demoReport, demoSnapshot } from "./fixtures";
import { samplePromptPackText } from "./promptPack";
import { sampleComputerTask } from "./computerTask";
import { sampleChatSession } from "./chatSession";
import {
  sampleFinishedMovie, sampleFinishedMusic, sampleH3CompileResult, sampleH3Environment,
  sampleImage, sampleImageAssets, sampleLyricsDocument, sampleMovie,
} from "./studioFixtures";

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
  list_movie_reference_assets: () => [],
  get_movie_h3_environment: () => sampleH3Environment,
  list_chat_sessions: () => [],
  list_computer_tasks: () => [],
  list_model_downloads: () => [],
  get_speech_preferences: () => speechPreferencesDefaults,
  get_conversation_voice_snapshot: () => ({ installed: false, running: false, processId: null,
    languages: ["en", "fr", "de", "pt", "it", "es", "nl"], detail: "Voice conversation is available in the desktop application." }),
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
    get_chat_session: ({ id }) => ({ ...structuredClone(sampleChatSession), id, title: sampleSessions.find((session) => session.id === id)?.title ?? sampleChatSession.title }),
    list_computer_tasks: () => [{ id: sampleComputerTask.id, objective: sampleComputerTask.objective, modelId: sampleComputerTask.modelId, access: sampleComputerTask.access, status: sampleComputerTask.status, updatedAt: sampleComputerTask.updatedAt, eventCount: sampleComputerTask.events.length, artifactCount: 0 }],
    get_computer_task: () => structuredClone(sampleComputerTask),
    list_movies: () => [
      {
        ...sampleFinishedMovie,
        clipCount: sampleFinishedMovie.clips.length,
        sceneCount: sampleFinishedMovie.scenes.length,
        entityCount: sampleFinishedMovie.entities.length,
        renderJobCount: sampleFinishedMovie.renderJobs.length,
      },
      {
        ...sampleMovie,
        clipCount: 0,
        sceneCount: sampleMovie.scenes.length,
        entityCount: sampleMovie.entities.length,
        renderJobCount: sampleMovie.renderJobs.length,
      },
    ],
    get_movie: ({ id }) => structuredClone(id === sampleMovie.id ? sampleMovie : sampleFinishedMovie),
    compile_movie_h3_window: () => structuredClone(sampleH3CompileResult),
    get_movie_render_state: () => ({ active: false }),
    inspect_movie_h3_take: ({ projectId, jobId }) => {
      const job = sampleFinishedMovie.renderJobs.find((j) => j.id === jobId) ?? sampleFinishedMovie.renderJobs[0];
      return structuredClone({
        jobId: job.id,
        projectId,
        sceneId: job.sceneId,
        label: job.label,
        status: job.status,
        passPreset: job.passPreset,
        taskMode: job.taskMode,
        resolvedModel: job.resolvedModel,
        seed: job.seed,
        compiledPrompt: job.compiledPrompt,
        takeMediaPath: job.takeMediaPath,
        takeSha256: job.takeSha256,
        geometry: job.geometry,
        colorDrift: job.colorDrift,
        quality: job.quality,
        error: job.error,
      });
    },
    list_movie_image_assets: () => structuredClone(sampleImageAssets),
    list_image_projects: () => [{ ...sampleImage, takeCount: 0, activeTakePath: "" }],
    get_image_project: () => structuredClone(sampleImage),
    create_image_project: ({ request }) => ({ ...structuredClone(sampleImage), id: "preview-new-image", title: request.title || "Untitled image", idea: request.idea, highLevelDescription: request.idea, settings: { ...structuredClone(sampleImage.settings), plainPrompt: request.idea } }),
    list_music_projects: () => [{ ...sampleFinishedMusic, takeCount: 1, activeTakePath: "" }],
    get_music_project: () => structuredClone(sampleFinishedMusic),
    get_music_lyrics_document: () => structuredClone({ project: sampleFinishedMusic, document: sampleLyricsDocument }),
  } satisfies PreviewHandlers : {}) });
}
