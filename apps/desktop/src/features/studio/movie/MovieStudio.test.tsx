import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MovieStudio } from "./MovieStudio";
import * as api from "../../../platform/api";
import type { ModelInfo, MovieProducerWorkspace, MovieProject, MovieStudioConversation, MovieSummary } from "../../../contracts/index";

vi.mock("../../../platform/api", async () => {
  const actual = await vi.importActual<typeof import("../../../platform/api")>("../../../platform/api");
  return {
    ...actual,
    listMovies: vi.fn(async () => []),
    listMovieImageAssets: vi.fn(async () => []),
    getMovie: vi.fn(),
    getMovieProducerWorkspace: vi.fn(),
    getMovieStudioConversation: vi.fn(),
    saveMovieStoryRevision: vi.fn(),
    acceptMovieStoryRevision: vi.fn(),
    saveMovieScenes: vi.fn(),
  };
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const model = {
  id: "bonsai-local", name: "Ternary Bonsai 27B", path: "bonsai.gguf", source: "test",
  bytes: 1, chatTemplate: true, supportsVision: false, supportsAudio: false,
  recommendation: "Local collaborator",
} satisfies ModelInfo;

const settings = {
  width: 1344, height: 768, clipSeconds: 5, steps: 20, maxClips: 12, seed: 0,
  temperature: .45, topP: .9, topK: 20, thinkingBudget: 32768,
  maxOutputTokens: 32768, comfyRoot: "", refImageSize: "match" as const,
};

function project(overrides: Partial<MovieProject> = {}): MovieProject {
  return {
    schemaVersion: 7, id: "movie-one", prompt: "A lighthouse keeper hears tomorrow's weather.",
    title: "Tomorrow's Weather", status: "awaiting-review", phase: "story-draft",
    detail: "Story revision 1 is ready.", createdAt: "2026-08-31T10:00:00Z",
    updatedAt: "2026-08-31T10:00:00Z", model: "Ternary Bonsai 27B", renderer: "H3",
    settings, references: [], sources: [], clips: [],
    edit: { clips: [], exportTitle: "Tomorrow's Weather", exportPreset: "publish", normalizeAudio: false, targetLufs: -14, markers: [] },
    finalPath: "", exports: [], error: "", producerReviewRequired: true,
    producerApprovedAt: "", ...overrides,
  };
}

function workspace(overrides: Partial<MovieProducerWorkspace> = {}): MovieProducerWorkspace {
  return {
    schemaVersion: 1, projectId: "movie-one", createdAt: "2026-08-31T10:00:00Z",
    updatedAt: "2026-08-31T10:00:00Z", activeStoryRevisionId: "story-one",
    activeStoryConversationId: "conversation-story", storyRevisions: [{
      id: "story-one", number: 1, createdAt: "2026-08-31T10:00:00Z", origin: "collaborator",
      instruction: "First sketch", markdown: "# Tomorrow's Weather\n\nMara hears a storm one day early.",
    }], conversations: [{ id: "conversation-story", kind: "story", title: "Story room", createdAt: "2026-08-31T10:00:00Z", updatedAt: "2026-08-31T10:00:00Z", storyRevisionId: "", summary: "", archived: false, messageCount: 0 }],
    scenes: [], sceneRevision: 0, ...overrides,
  };
}

const conversation: MovieStudioConversation = {
  id: "conversation-story", kind: "story", createdAt: "2026-08-31T10:00:00Z",
  updatedAt: "2026-08-31T10:00:00Z", storyRevisionId: "story-one",
  title: "Story room", summary: "", archived: false, messages: [],
};

function summary(): MovieSummary {
  return { id: "movie-one", title: "Tomorrow's Weather", status: "awaiting-review", phase: "story-draft", updatedAt: "2026-08-31T10:00:00Z", clipCount: 0, finalPath: "" };
}

async function openFixture(
  nextProject = project(),
  nextWorkspace = workspace(),
  nextConversation = conversation,
) {
  vi.mocked(api.listMovies).mockResolvedValue([summary()]);
  vi.mocked(api.getMovie).mockResolvedValue(nextProject);
  vi.mocked(api.getMovieProducerWorkspace).mockResolvedValue(nextWorkspace);
  vi.mocked(api.getMovieStudioConversation).mockResolvedValue(nextConversation);
  render(<MovieStudio advancedEnabled models={[model]} selectedModelId={model.id} onError={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: /Tomorrow's Weather/i }));
  await screen.findByText("Producer-owned story, scenes, media choices, and H3 masters");
}

describe("producer-owned Movie Studio", () => {
  it("starts from loose material with one local story collaborator", async () => {
    render(<MovieStudio advancedEnabled models={[model]} selectedModelId={model.id} onError={vi.fn()} />);
    expect(screen.getByText("Start with the movie, not a workflow.")).toBeInTheDocument();
    expect(screen.getByLabelText(/story collaborator/i)).toBeInTheDocument();
    const create = screen.getByRole("button", { name: /Create story sketch/i });
    expect(create).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Starting material"), { target: { value: "A fox waits for the last train." } });
    expect(create).toBeEnabled();
    expect(screen.getByText(/TypeScript never owns application truth/i)).toBeInTheDocument();
  });

  it("shows a readable Markdown story and saves direct edits as a new revision", async () => {
    const saved = workspace({
      activeStoryRevisionId: "story-two",
      storyRevisions: [...workspace().storyRevisions, { id: "story-two", number: 2, parentRevisionId: "story-one", createdAt: "2026-08-31T10:10:00Z", origin: "producer", instruction: "Direct producer edit", markdown: "# A quieter title\n\nMara listens." }],
    });
    vi.mocked(api.saveMovieStoryRevision).mockResolvedValue(saved);
    await openFixture();
    expect(screen.getByRole("heading", { name: "Tomorrow's Weather" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^Edit$/ }));
    const document = screen.getByLabelText("Story document");
    fireEvent.change(document, { target: { value: "# A quieter title\n\nMara listens." } });
    fireEvent.click(screen.getByRole("button", { name: /Save revision/i }));
    await waitFor(() => expect(api.saveMovieStoryRevision).toHaveBeenCalledWith(expect.objectContaining({
      projectId: "movie-one", parentRevisionId: "story-one", markdown: "# A quieter title\n\nMara listens.",
    })));
    expect(await screen.findByRole("option", { name: /Revision 2/ })).toBeInTheDocument();
  });

  it("keeps scene context selection separate from native reference bindings", async () => {
    const reference = {
      assetId: "asset-image", tag: "<Image 1>", audioTag: "", name: "Mara portrait", kind: "image" as const,
      mimeType: "image/png", bytes: 100, durationSeconds: 0, width: 768, height: 1024,
      hasAudio: false, path: "C:\\Movies\\mara.png", description: "Mara's identity", useEmbeddedAudio: false,
      embeddedAudioDescription: "",
    };
    const scene = {
      id: "scene-one", revision: 1, title: "The forecast", purpose: "Inciting event", durationSeconds: 5,
      h3Prompt: "At 0 seconds Mara enters the tower. At 5 seconds she faces the radio. No dialogue.",
      continuityIn: "Night", continuityOut: "Mara at radio", transition: "Cut", references: [],
      storyRevisionId: "story-one", createdAt: "2026-08-31T10:20:00Z", updatedAt: "2026-08-31T10:20:00Z",
    };
    const nextWorkspace = workspace({ acceptedStoryRevisionId: "story-one", activeSceneConversationId: "conversation-scenes", scenes: [scene], sceneRevision: 1 });
    const nextProject = project({ references: [reference], plan: { title: "Tomorrow's Weather", logline: "", audience: "", creativeDirection: "", continuityBible: [], sourceCredits: [], qualityReview: { attempts: 0, score: 0, verdict: "Producer-owned" }, clips: [{ id: scene.id, title: scene.title, purpose: scene.purpose, durationSeconds: 5, prompt: scene.h3Prompt, continuityIn: scene.continuityIn, continuityOut: scene.continuityOut, transition: "Cut", usePreviousFrame: false, sourceRefs: [], referenceIds: [], firstFrameReferenceId: "", lastFrameReferenceId: "", referenceSelections: [] }] } });
    await openFixture(nextProject, nextWorkspace, { ...conversation, id: "conversation-scenes", kind: "scenes" });
    expect(screen.getByText(/You choose the model context/i)).toBeInTheDocument();
    const include = screen.getByLabelText(/Include The forecast in scene chat context/i);
    const visual = screen.getByLabelText(/Visual \/ motion/i);
    fireEvent.click(include);
    expect(screen.getByText(/1 scene card in full context/i)).toBeInTheDocument();
    fireEvent.click(visual);
    expect(visual).toBeChecked();
    fireEvent.change(screen.getByLabelText(/First frame/i), { target: { value: "image:asset-image" } });
    expect(visual).not.toBeChecked();
    expect(visual).toBeDisabled();
  });
});
