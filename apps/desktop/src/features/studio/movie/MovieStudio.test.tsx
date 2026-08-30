import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { emptyPlannedClip, IndependentReviewerResult, LiveH3Preview, moviePlanningLive, MovieStudio, previewProvenanceAvailable, ProducerCopilot, ProducerPlanDesk, referenceDisplayTags } from "./MovieStudio";
import * as api from "../../../platform/api";
import type { ModelInfo, MovieEdit, MoviePlan, MoviePlanningSnapshot, MovieProject, MovieRenderPreviewEvent, MovieSummary, PendingMovieReference } from "../../../contracts/index";

vi.mock("../../../platform/api", async () => {
  const actual = await vi.importActual<typeof import("../../../platform/api")>("../../../platform/api");
  return {
    ...actual,
    listMovies: vi.fn(async () => []),
    getMovie: vi.fn(async () => {
      throw new Error("Movie projects require the desktop application.");
    }),
    getMoviePlanning: vi.fn(async () => ({
      projectId: "",
      checkpointRequested: false,
      pendingDirections: [],
      promptDocuments: [],
      toolSchema: {},
      lastRequest: {},
      transcript: {},
      currentText: "",
      reviewerReview: null,
    } satisfies MoviePlanningSnapshot)),
  };
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const baselineModel = {
  id: "bonsai-local",
  name: "Ternary Bonsai 27B",
  path: "bonsai.gguf",
  source: "test",
  bytes: 1,
  chatTemplate: true,
  supportsVision: false,
  supportsAudio: false,
  recommendation: "Release baseline",
} satisfies ModelInfo;

const defaultMovieSettings = {
  width: 1344,
  height: 768,
  clipSeconds: 5,
  steps: 20,
  maxClips: 12,
  seed: 0,
  temperature: 0.7,
  topP: 0.95,
  topK: 20,
  thinkingBudget: 32768,
  maxOutputTokens: 32768,
  comfyRoot: "",
  refImageSize: "match",
} as const;

const makeMovieSummary = (id: string, title: string): MovieSummary => ({
  id,
  title,
  status: "running",
  phase: "agent-submitted",
  updatedAt: "2026-08-16T00:00:00Z",
  clipCount: 0,
  finalPath: "",
});

const makeRunningProject = (id: string, title: string): MovieProject => ({
  schemaVersion: 6,
  id,
  title,
  prompt: `Prompt for ${title}.`,
  status: "running",
  phase: "agent-submitted",
  detail: "Planning",
  createdAt: "2026-08-16T00:00:00Z",
  updatedAt: "2026-08-16T00:00:00Z",
  model: "Local Director",
  renderer: "H3",
  settings: { ...defaultMovieSettings },
  references: [],
  sources: [],
  plan: undefined,
  clips: [],
  edit: { clips: [], exportTitle: title, exportPreset: "publish", normalizeAudio: false, targetLufs: -14, markers: [] },
  finalPath: "",
  exports: [],
  error: "",
  producerReviewRequired: false,
  producerApprovedAt: "",
  producerFeedback: [],
  copilotHistory: [],
});

const makePlanningSnapshot = (projectId: string): MoviePlanningSnapshot => ({
  projectId,
  checkpointRequested: false,
  pendingDirections: [],
  promptDocuments: [],
  toolSchema: {},
  lastRequest: {},
  transcript: {},
  currentText: "",
  reviewerReview: null,
});

describe("Kestrel Movie Studio", () => {
  it("keeps the planning room mounted and presents the fresh-context review", () => {
    expect(moviePlanningLive({ status: "running", phase: "agent-submitted" })).toBe(true);
    render(<IndependentReviewerResult review={{
      summary: "The ending loses the producer's requested red suitcase.",
      issues: [{ clipNumber: 8, category: "continuity", finding: "The suitcase disappears.", requiredFix: "Restore it in the final frame." }],
    }} />);
    expect(screen.getByRole("region", { name: "Latest fresh-context review" })).toBeInTheDocument();
    expect(screen.getByText("1 blocking issue")).toBeInTheDocument();
    expect(screen.getByText("Scene 8 · continuity")).toBeInTheDocument();
    expect(screen.getByText(/Restore it in the final frame/)).toBeInTheDocument();
  });

  it("remounts the planning room when switching between planning-enabled projects", async () => {
    const onError = vi.fn();
    const first = makeRunningProject("movie-one", "Project One");
    const second = makeRunningProject("movie-two", "Project Two");

    vi.mocked(api.listMovies).mockResolvedValue([makeMovieSummary(first.id, first.title), makeMovieSummary(second.id, second.title)]);
    vi.mocked(api.getMovie).mockImplementation(async (id: string) => {
      if (id === first.id) return first;
      if (id === second.id) return second;
      throw new Error(`Unknown movie project: ${id}`);
    });
    vi.mocked(api.getMoviePlanning).mockImplementation(async (id: string) => makePlanningSnapshot(id));

    render(<MovieStudio advancedEnabled onError={onError} />);
    fireEvent.click(await screen.findByRole("button", { name: /Project One/i }));
    const firstDirection = await screen.findByPlaceholderText(/opening warmer and more intimate/i);
    fireEvent.change(firstDirection, { target: { value: "Keep the first project focused." } });
    await waitFor(() => expect(firstDirection).toHaveValue("Keep the first project focused."));

    fireEvent.click(await screen.findByRole("button", { name: /Project Two/i }));
    await waitFor(() => {
      const directions = screen.getAllByPlaceholderText(/opening warmer and more intimate/i);
      expect(directions).toHaveLength(1);
      expect(directions[0]).toHaveValue("");
    });
    expect(onError).not.toHaveBeenCalled();
  });

  it("keeps the Generate workspace attached while another production stage is visible", async () => {
    const project: MovieProject = {
      ...makeRunningProject("movie-retained", "Retained Director"),
      status: "complete",
      phase: "complete",
      detail: "Review cut ready",
      plan: {
        title: "Retained Director",
        logline: "A single completed shot.",
        audience: "Producers",
        creativeDirection: "Keep every approved frame available.",
        continuityBible: ["Preserve the subject."],
        sourceCredits: [],
        qualityReview: { attempts: 1, score: 100, verdict: "Ready" },
        clips: [{ id: "clip-1", title: "The Shot", purpose: "Open", durationSeconds: 5, prompt: "A precise cinematic shot.", continuityIn: "Start", continuityOut: "End", transition: "cut", usePreviousFrame: false, sourceRefs: [], referenceIds: [] }],
      },
      clips: [{ id: "clip-1", index: 1, title: "The Shot", prompt: "A precise cinematic shot.", durationSeconds: 5, seed: 7, status: "complete", path: "C:\\Movies\\clip-1.mp4", error: "", versions: [] }],
      edit: {
        clips: [{ id: "edit-1", clipId: "clip-1", enabled: true, order: 0, trimStart: 0, trimEnd: 0, audioGain: 1, sourceVersionId: "", speed: 1, fadeIn: 0, fadeOut: 0, audioFadeIn: 0, audioFadeOut: 0, label: "", notes: "" }],
        exportTitle: "Retained Director", exportPreset: "publish", normalizeAudio: false, targetLufs: -14, markers: [],
      },
    };
    vi.mocked(api.listMovies).mockResolvedValue([{ ...makeMovieSummary(project.id, project.title), status: "complete", phase: "complete", clipCount: 1 }]);
    vi.mocked(api.getMovie).mockResolvedValue(project);

    render(<MovieStudio advancedEnabled models={[baselineModel]} selectedModelId={baselineModel.id} onError={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: /Retained Director/i }));
    fireEvent.click(await screen.findByText("Model limits"));
    expect(screen.getByLabelText("This production context window")).toHaveValue(32_768);
    expect(screen.getByLabelText("This production maximum output")).toHaveValue(32_768);
    fireEvent.click(await screen.findByRole("button", { name: /GenerateH3 picture and sound/i }));
    const direction = await screen.findByLabelText("Producer direction");
    fireEvent.change(direction, { target: { value: "Preserve this unfinished producer direction." } });

    const generateWorkspace = direction.closest("section.retained-studio-workspace");
    expect(generateWorkspace).not.toHaveAttribute("hidden");
    fireEvent.click(screen.getByRole("button", { name: /EditStoryline and native mix/i }));
    expect(generateWorkspace).toHaveAttribute("hidden");
    const editWorkspace = document.querySelector("section.project-edit-room.retained-studio-workspace");
    expect(editWorkspace).not.toHaveAttribute("hidden");
    fireEvent.click(screen.getByRole("button", { name: /GenerateH3 picture and sound/i }));
    expect(generateWorkspace).not.toHaveAttribute("hidden");
    expect(editWorkspace).toHaveAttribute("hidden");

    expect(screen.getByLabelText("Producer direction")).toBe(direction);
    expect(direction).toHaveValue("Preserve this unfinished producer direction.");
    expect(screen.getByLabelText("This production context window")).toHaveValue(32_768);
  });

  it("presents a one-prompt offline production path", async () => {
    render(<MovieStudio advancedEnabled models={[baselineModel]} selectedModelId={baselineModel.id} onError={vi.fn()} />);
    expect(screen.getByText(/Shape the production brief together/i)).toBeInTheDocument();
    expect(screen.getByText(/drafts, reviews, and repairs/i)).toBeInTheDocument();
    expect(screen.queryByText(/Wikipedia/i)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Ask Director to plan/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Write plan myself/i })).toBeEnabled();
    fireEvent.change(screen.getByLabelText("Movie brief"), { target: { value: "A tiny film about a lighthouse keeper" } });
    await waitFor(() => expect(screen.getByRole("button", { name: /Ask Director to plan/i })).toBeEnabled());
  });

  it("keeps full-context and expert production controls discoverable", () => {
    render(
      <MovieStudio
        initialComfyRoot={"C:\\Configured\\ComfyUI"}
        advancedEnabled
        onError={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /SetupQuality and controls/i }));
    expect(screen.getByText("System model policy")).toBeInTheDocument();
    expect(screen.getByText("Per-model exceptions")).toBeInTheDocument();
    expect(screen.getByText("Live reasoning stream")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Advanced production controls/i }));
    expect(screen.getByLabelText("Maximum clips")).toHaveAttribute("max", "96");
    expect(screen.getByLabelText("Thinking mode")).toHaveValue("max");
    expect(screen.getByLabelText("ComfyUI root")).toHaveValue("C:\\Configured\\ComfyUI");
    expect(screen.getByLabelText("Reference image fidelity")).toHaveValue("match");
    expect(screen.getByLabelText("This production context window")).toHaveValue(32_768);
    expect(screen.getByLabelText("This production maximum output")).toHaveValue(32_768);
    const checkpoint = screen.getByLabelText(/Review the plan before rendering/i);
    expect(checkpoint).toBeChecked();
    fireEvent.click(checkpoint);
    expect(checkpoint).not.toBeChecked();
    expect(screen.getByText(/before any H3 clip is rendered/i)).toBeInTheDocument();
    expect(screen.queryByLabelText("Research")).not.toBeInTheDocument();
  });

  it("does not invent a machine-specific ComfyUI location", () => {
    render(<MovieStudio advancedEnabled onError={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /SetupQuality and controls/i }));
    fireEvent.click(screen.getByRole("button", { name: /Advanced production controls/i }));
    expect(screen.getByLabelText("ComfyUI root")).toHaveValue("");
  });

  it("lets producers explicitly develop notes or continue exact text with any local model", () => {
    const models = [
      { id: "story-small", name: "Small Story Model", quantization: "Q4_K_M" },
      { id: "story-large", name: "Large Story Model", quantization: "Q6_K" },
    ].map((model) => ({
      ...model, path: `${model.id}.gguf`, source: "test", bytes: 1, chatTemplate: true,
      supportsVision: false, supportsAudio: false, recommendation: "Local test model",
    })) as ModelInfo[];
    render(<MovieStudio advancedEnabled models={models} selectedModelId="story-large" onError={vi.fn()} />);
    expect(screen.getByLabelText("Movie brief model")).toHaveValue("story-large");
    expect(screen.getAllByRole("option", { name: /Small Story Model/ })).toHaveLength(1);
    expect(screen.getByRole("button", { name: /Invent story/i })).toBeEnabled();
    fireEvent.change(screen.getByLabelText("Movie brief"), { target: { value: "A botanist finds a singing seed." } });
    expect(screen.getByRole("button", { name: /Develop idea \/ notes/i })).toBeEnabled();
    const meaning = screen.getByLabelText("Movie brief existing text meaning");
    expect(meaning).toHaveValue("develop");
    fireEvent.change(meaning, { target: { value: "continue" } });
    expect(screen.getByRole("button", { name: /Continue exact draft/i })).toBeEnabled();
    expect(screen.getByText(/Nothing is inferred/i)).toBeInTheDocument();
    expect(screen.getByLabelText("Movie brief")).toHaveAttribute("maxlength", "65536");
    fireEvent.click(screen.getByRole("button", { name: /ImagesGenerate visual assets/i }));
    expect(screen.getByLabelText("Image description model")).toHaveValue("story-large");
  });

  it("offers a producer-friendly durable H3 image asset workflow", () => {
    render(<MovieStudio advancedEnabled onError={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /ImagesGenerate visual assets/i }));
    expect(screen.getByText(/Offline image asset lab/i)).toBeInTheDocument();
    expect(screen.getByText(/stable-frame candidate pass/i)).toBeInTheDocument();
    const generate = screen.getByRole("button", { name: /Generate candidates/i });
    expect(generate).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Image asset prompt"), { target: { value: "A precise portrait of the recurring lead character" } });
    expect(generate).toBeEnabled();
    expect(screen.getByLabelText("Image canvas")).toHaveValue("768x1344");
    fireEvent.change(screen.getByLabelText("Image canvas"), { target: { value: "1024x1024" } });
    expect(screen.getByLabelText("Image canvas")).toHaveValue("1024x1024");
    fireEvent.click(screen.getByRole("button", { name: /SetupQuality and controls/i }));
    fireEvent.click(screen.getByRole("button", { name: /Advanced production controls/i }));
    fireEvent.click(screen.getByRole("button", { name: /ImagesGenerate visual assets/i }));
    const steps = screen.getByLabelText("Image sampling steps");
    const seed = screen.getByLabelText("Image seed \(0 = random\)");
    expect(steps).toHaveValue(20);
    expect(seed).toHaveValue(0);
    fireEvent.change(steps, { target: { value: "" } });
    expect(steps).toHaveValue(20);
    fireEvent.change(steps, { target: { value: "101" } });
    expect(steps).toHaveValue(20);
    fireEvent.change(steps, { target: { value: "21" } });
    expect(steps).toHaveValue(21);
    fireEvent.click(screen.getByRole("button", { name: /Generate candidates/i }));
    expect(steps).toBeDisabled();
    expect(seed).toBeDisabled();
  });

  it("treats blank and legacy image-preview provenance as unavailable", () => {
    expect(previewProvenanceAvailable({
      previewNodeRevision: "kj-revision",
      previewDecoderRevision: "taehv-revision",
      previewDecoderSha256: "decoder-sha256",
    })).toBe(true);
    expect(previewProvenanceAvailable({
      previewNodeRevision: "   ",
      previewDecoderRevision: "taehv-revision",
      previewDecoderSha256: "decoder-sha256",
    })).toBe(false);
    expect(previewProvenanceAvailable({
      previewNodeRevision: "kj-revision",
      previewDecoderRevision: " unavailable (legacy generation)",
      previewDecoderSha256: "decoder-sha256",
    })).toBe(false);
  });

  it("keeps the whole creation process in bounded editor workspaces", () => {
    render(<MovieStudio advancedEnabled models={[baselineModel]} selectedModelId={baselineModel.id} onError={vi.fn()} />);
    const rooms = screen.getByRole("navigation", { name: "New production workspaces" });
    expect(rooms).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /ReferencesBind media to the story/i }));
    expect(screen.getByText(/Show and tell H3 what must carry through/i)).toBeInTheDocument();
    expect(screen.queryByLabelText("Movie brief")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /SetupQuality and controls/i }));
    expect(screen.getByText(/Choose the working quality and review boundary/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Ask Director to plan/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Write plan myself/i })).toBeEnabled();
  });

  it("shows a plain-language live H3 monitor with advanced offline provenance", () => {
    const event: MovieRenderPreviewEvent = {
      kind: "frame",
      target: "movieClip",
      jobId: "clip-1",
      projectId: "movie-1",
      clipId: "scene-1",
      clipIndex: 0,
      detail: "Approximate live preview · sample 7 of 20",
      mimeType: "image/jpeg",
      dataUrl: "data:image/jpeg;base64,AQID",
      width: 512,
      height: 288,
      step: 7,
      total: 20,
      averageStepMs: 1250,
      previewNodeRevision: "kj-revision",
      previewDecoderRevision: "taehv-revision",
      previewDecoderSha256: "decoder-sha256",
      at: new Date().toISOString(),
    };
    render(<LiveH3Preview event={event} advanced />);
    expect(screen.getByText("Live H3 estimate")).toBeInTheDocument();
    expect(screen.getByText("Sample 7 of 20")).toBeInTheDocument();
    expect(screen.getByAltText(/Approximate live MiniMax H3/i)).toHaveAttribute("src", event.dataUrl);
    fireEvent.click(screen.getByText("Preview pipeline details"));
    expect(screen.getByText(/taeh3.safetensors/)).toBeInTheDocument();
    expect(screen.getByText(/KJNodes@kj-revision/)).toBeInTheDocument();
    expect(screen.getByText(/taehv-revision/)).toBeInTheDocument();
    expect(screen.getByText(/decoder-sha256/)).toBeInTheDocument();
    expect(screen.getByText(/Ephemeral preview bytes are not stored/i)).toBeInTheDocument();
  });

  it("shows an explicit safe terminal state when an H3 estimate stops before its first frame", () => {
    render(<LiveH3Preview event={{
      kind: "stopped", target: "movieClip", jobId: "job-1", projectId: "movie-1",
      detail: "The H3 live estimate stopped before a full master was preserved.",
      previewNodeRevision: "kj", previewDecoderRevision: "taehv", previewDecoderSha256: "sha", at: new Date().toISOString(),
    }} advanced={false} />);
    expect(screen.getByText("Stopped safely")).toBeInTheDocument();
    expect(screen.getByText(/source master and storyline remain unchanged/i)).toBeInTheDocument();
  });

  it("numbers native H3 labels by type and puts embedded video audio first", () => {
    const makeReference = (assetId: string, kind: "image" | "video" | "audio", useEmbeddedAudio = false) => ({
      id: assetId, assetId, kind, useEmbeddedAudio, name: `${assetId}.dat`, mimeType: "application/octet-stream",
      bytes: 1, durationSeconds: 1, width: 1, height: 1, hasAudio: useEmbeddedAudio, path: "",
      createdAt: "", description: "Producer job", embeddedAudioDescription: useEmbeddedAudio ? "Voice job" : "",
    }) as PendingMovieReference;
    const references = [
      makeReference("image-a", "image"),
      makeReference("video-a", "video", true),
      makeReference("audio-a", "audio"),
      makeReference("image-b", "image"),
    ];
    expect(referenceDisplayTags(references, "image-b")).toEqual(["<Picture 2>"]);
    expect(referenceDisplayTags(references, "video-a")).toEqual(["<Video 1>", "<Audio 1>"]);
    expect(referenceDisplayTags(references, "audio-a")).toEqual(["<Audio 2>"]);
  });

  it("keeps model cooperation inside the editor and producer-approved", () => {
    const edit: MovieEdit = { clips: [], exportTitle: "Reunion", exportPreset: "publish", normalizeAudio: false, targetLufs: -14, markers: [] };
    const project = {
      schemaVersion: 6, id: "movie-1", title: "Reunion", prompt: "Two friends meet beside the sea.", status: "complete", phase: "review", detail: "Review cut ready",
      createdAt: "2026-08-12T00:00:00Z", updatedAt: "2026-08-12T00:00:00Z", model: "Local Director", renderer: "H3",
      settings: { width: 1344, height: 768, clipSeconds: 5, steps: 20, maxClips: 12, seed: 0, temperature: .7, topP: .95, topK: 20, thinkingBudget: 32768, maxOutputTokens: 32768, comfyRoot: "D:\\AI\\ComfyUI", refImageSize: "match" },
      copilotHistory: [{ id: "turn-1", createdAt: new Date().toISOString(), workspace: "edit", producerRequest: "Protect the ending.", modelId: "local-1", response: "Keep the final hold and tighten the entrance.", status: "complete", proposalSummary: "" }],
      clips: [], references: [], exports: [], sources: [], edit, finalPath: "", error: "", producerReviewRequired: false, producerApprovedAt: "", producerFeedback: [],
    } satisfies MovieProject;
    const models = [{ id: "local-1", name: "Local Director" }] as ModelInfo[];
    render(<ProducerCopilot project={project} edit={edit} workspace="edit" models={models} selectedModelId="local-1" advancedEnabled onEdit={vi.fn()} onClose={vi.fn()} onError={vi.fn()} />);

    expect(screen.getByRole("complementary", { name: "Producer copilot" })).toBeInTheDocument();
    expect(screen.getByText(/model cannot watch media or change the project/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Collaborate/i })).toBeDisabled();
    fireEvent.change(screen.getByPlaceholderText(/Make the middle move faster/i), { target: { value: "Tighten the entrance but preserve the last reaction." } });
    expect(screen.getByRole("button", { name: /Collaborate/i })).toBeEnabled();
    fireEvent.click(screen.getByText(/Recent durable conversations/i));
    expect(screen.getByText("Protect the ending.")).toBeInTheDocument();
  });

  it("locks the complete producer plan desk while an action is busy", () => {
    const plan: MoviePlan = {
      title: "Night Crossing", logline: "A courier crosses a flooded city before dawn.", audience: "Film buyers",
      creativeDirection: "Tactile live action with restrained camera movement.", continuityBible: ["The courier keeps the same red coat."], sourceCredits: [],
      qualityReview: { score: 100, attempts: 1, verdict: "Ready" },
      clips: [{ id: "clip-stable", title: "Departure", purpose: "Begin the journey", durationSeconds: 5, prompt: "A timed cinematic departure.", continuityIn: "Night", continuityOut: "Street", transition: "hard cut", usePreviousFrame: false, sourceRefs: [], referenceIds: [] }],
    };
    const project = {
      schemaVersion: 6, id: "movie-review", title: plan.title, prompt: plan.logline, status: "awaiting-review", phase: "awaiting-producer", detail: "Review",
      createdAt: "2026-08-12T00:00:00Z", updatedAt: "2026-08-12T00:00:00Z", model: "Local Director", renderer: "H3",
      settings: { width: 1344, height: 768, clipSeconds: 5, steps: 20, maxClips: 12, seed: 0, temperature: .7, topP: .95, topK: 20, thinkingBudget: 32768, maxOutputTokens: 32768, comfyRoot: "D:\\AI\\ComfyUI", refImageSize: "match" },
      clips: [], references: [], exports: [], sources: [], edit: { clips: [], exportTitle: plan.title, exportPreset: "publish", normalizeAudio: false, targetLufs: -14, markers: [] }, finalPath: "", error: "", plan,
      producerReviewRequired: true, producerApprovedAt: "", producerFeedback: [], copilotHistory: [],
    } as MovieProject;
    render(<ProducerPlanDesk project={project} plan={plan} busy onPlan={vi.fn()} onSave={vi.fn()} onRevise={vi.fn()} onApprove={vi.fn()} />);

    expect(screen.getByLabelText("Title")).toBeDisabled();
    expect(screen.getByLabelText("Scene title")).toBeDisabled();
    expect(screen.getByRole("button", { name: /Insert before/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Add scene at end/i })).toBeDisabled();
    expect(screen.getByPlaceholderText(/Keep the flashback isolated/i)).toBeDisabled();
    fireEvent.click(screen.getByText(/Use another chat or agent/i));
    expect(screen.getByRole("button", { name: /Copy request/i })).toBeDisabled();
    expect(screen.getByLabelText("Choose Use another chat or agent response")).toBeDisabled();
    expect(screen.getByLabelText("Use another chat or agent JSON response")).toBeDisabled();
  });

  it("creates scene IDs independently of insertion index and retries collisions", () => {
    const collision = "00000000-0000-4000-8000-000000000001";
    const unique = "00000000-0000-4000-8000-000000000002";
    const random = vi.spyOn(crypto, "randomUUID")
      .mockReturnValueOnce(collision)
      .mockReturnValueOnce(unique);

    expect(emptyPlannedClip(3, new Set([`producer-scene-${collision}`])).id)
      .toBe(`producer-scene-${unique}`);
    random.mockRestore();
  });
});
