import { installPreviewHandlers, type PreviewHandlers } from "../platform/transport";
import { speechPreferencesDefaults } from "../contracts/index";
import { demoReport, demoSnapshot } from "./fixtures";
import { sampleConversation, sampleImage, sampleMovie, sampleMusic, sampleWorkspace } from "./studioFixtures";

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
    list_movies: () => [{ ...sampleMovie, clipCount: 0 }],
    get_movie: () => structuredClone(sampleMovie),
    get_movie_producer_workspace: () => structuredClone(sampleWorkspace),
    get_movie_studio_conversation: () => structuredClone(sampleConversation),
    get_movie_editor_state: () => ({ schemaVersion: 1, projectId: sampleMovie.id, editHash: "preview", jobs: [] }),
    list_image_projects: () => [{ ...sampleImage, takeCount: 0, activeTakePath: "" }],
    get_image_project: () => structuredClone(sampleImage),
    list_music_projects: () => [{ ...sampleMusic, takeCount: 0, activeTakePath: "" }],
    get_music_project: () => structuredClone(sampleMusic),
  } satisfies PreviewHandlers : {}) });
}
