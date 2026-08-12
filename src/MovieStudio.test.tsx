import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MovieStudio, referenceDisplayTags } from "./MovieStudio";
import type { ModelInfo, PendingMovieReference } from "./types";

afterEach(cleanup);

describe("Kestrel Movie Studio", () => {
  it("presents a one-prompt offline production path", async () => {
    render(<MovieStudio advancedEnabled onError={vi.fn()} />);
    expect(screen.getByText(/Describe the movie/i)).toBeInTheDocument();
    expect(screen.getByText(/drafts, reviews, and repairs/i)).toBeInTheDocument();
    expect(screen.queryByText(/Wikipedia/i)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Make movie/i })).toBeDisabled();
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "A tiny film about a lighthouse keeper" } });
    expect(screen.getByRole("button", { name: /Make movie/i })).toBeEnabled();
  });

  it("keeps full-context and expert production controls discoverable", () => {
    render(<MovieStudio advancedEnabled onError={vi.fn()} />);
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

  it("offers every discovered local model for inventing or continuing a story", () => {
    const models = [
      { id: "story-small", name: "Small Story Model", quantization: "Q4_K_M" },
      { id: "story-large", name: "Large Story Model", quantization: "Q6_K" },
    ].map((model) => ({
      ...model, path: `${model.id}.gguf`, source: "test", bytes: 1, chatTemplate: true,
      supportsVision: false, supportsAudio: false, recommendation: "Local test model",
    })) as ModelInfo[];
    render(<MovieStudio advancedEnabled models={models} selectedModelId="story-large" onError={vi.fn()} />);
    expect(screen.getByLabelText("Story model")).toHaveValue("story-large");
    expect(screen.getByRole("option", { name: /Small Story Model/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Invent story/i })).toBeEnabled();
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "A botanist finds a singing seed." } });
    expect(screen.getByRole("button", { name: /Continue story/i })).toBeEnabled();
    expect(screen.getByText(/Continue the story already in the box/i)).toBeInTheDocument();
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
});
