import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LiveH3Preview, MovieStudio, ProducerCopilot, referenceDisplayTags } from "./MovieStudio";
import type { ModelInfo, MovieEdit, MovieProject, MovieRenderPreviewEvent, PendingMovieReference } from "./types";

afterEach(cleanup);

describe("Kestrel Movie Studio", () => {
  it("presents a one-prompt offline production path", async () => {
    render(<MovieStudio advancedEnabled onError={vi.fn()} />);
    expect(screen.getByText(/Shape the production brief together/i)).toBeInTheDocument();
    expect(screen.getByText(/drafts, reviews, and repairs/i)).toBeInTheDocument();
    expect(screen.queryByText(/Wikipedia/i)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Ask Bonsai to plan/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Write plan myself/i })).toBeEnabled();
    fireEvent.change(screen.getByLabelText("Movie brief"), { target: { value: "A tiny film about a lighthouse keeper" } });
    expect(screen.getByRole("button", { name: /Ask Bonsai to plan/i })).toBeEnabled();
  });

  it("keeps full-context and expert production controls discoverable", () => {
    render(<MovieStudio advancedEnabled onError={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /SetupQuality and controls/i }));
    expect(screen.getByText("98,304 context")).toBeInTheDocument();
    expect(screen.getByText("32,768 max thinking")).toBeInTheDocument();
    expect(screen.getByText("32,768 output")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Advanced production controls/i }));
    expect(screen.getByLabelText("Maximum clips")).toHaveAttribute("max", "96");
    expect(screen.getByLabelText("Thinking mode is fixed at maximum")).toHaveValue(
      "Maximum · 32,768",
    );
    expect(screen.getByLabelText("ComfyUI root")).toHaveValue("D:\\AI\\ComfyUI");
    expect(screen.getByLabelText("Reference image fidelity")).toHaveValue("match");
    const checkpoint = screen.getByLabelText(/Review the plan before rendering/i);
    expect(checkpoint).toBeChecked();
    fireEvent.click(checkpoint);
    expect(checkpoint).not.toBeChecked();
    expect(screen.getByText(/before any H3 clip is rendered/i)).toBeInTheDocument();
    expect(screen.queryByLabelText("Research")).not.toBeInTheDocument();
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
    expect(screen.getByText(/22 internal frames · 6 choices/i)).toBeInTheDocument();
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
    expect(screen.getByLabelText("Image sampling steps")).toHaveValue(20);
    expect(screen.getByLabelText("Image seed \(0 = random\)")).toHaveValue(0);
  });

  it("keeps the whole creation process in bounded editor workspaces", () => {
    render(<MovieStudio advancedEnabled onError={vi.fn()} />);
    const rooms = screen.getByRole("navigation", { name: "New production workspaces" });
    expect(rooms).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /ReferencesBind media to the story/i }));
    expect(screen.getByText(/Show and tell H3 what must carry through/i)).toBeInTheDocument();
    expect(screen.queryByLabelText("Movie brief")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /SetupQuality and controls/i }));
    expect(screen.getByText(/Choose the working quality and review boundary/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Ask Bonsai to plan/i })).toBeDisabled();
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
      at: new Date().toISOString(),
    };
    render(<LiveH3Preview event={event} advanced />);
    expect(screen.getByText("Live H3 preview")).toBeInTheDocument();
    expect(screen.getByText("Sample 7 of 20")).toBeInTheDocument();
    expect(screen.getByAltText(/Approximate live MiniMax H3/i)).toHaveAttribute("src", event.dataUrl);
    fireEvent.click(screen.getByText("Preview pipeline details"));
    expect(screen.getByText(/taeh3.safetensors/)).toBeInTheDocument();
    expect(screen.getByText(/Ephemeral preview bytes are not stored/i)).toBeInTheDocument();
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
});
