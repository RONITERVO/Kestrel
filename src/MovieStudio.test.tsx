import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MovieStudio, referenceDisplayTags } from "./MovieStudio";
import type { PendingMovieReference } from "./types";

afterEach(cleanup);

describe("Kestrel Movie Studio", () => {
  it("presents a one-prompt offline production path", async () => {
    render(<MovieStudio advancedEnabled onError={vi.fn()} />);
    expect(screen.getByText(/Describe the movie/i)).toBeInTheDocument();
    expect(screen.getByText(/Offline Wikipedia is available/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Make movie/i })).toBeDisabled();
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "A tiny film about a lighthouse keeper" } });
    expect(screen.getByRole("button", { name: /Make movie/i })).toBeEnabled();
  });

  it("keeps full-context and expert production controls discoverable", () => {
    render(<MovieStudio advancedEnabled onError={vi.fn()} />);
    expect(screen.getByText("98,304 context")).toBeInTheDocument();
    expect(screen.getByText("32,768 output")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Advanced production controls/i }));
    expect(screen.getByLabelText("Maximum clips")).toHaveAttribute("max", "96");
    expect(screen.getByLabelText("Thinking budget")).toHaveValue(4096);
    expect(screen.getByLabelText("ComfyUI root")).toHaveValue("D:\\AI\\ComfyUI");
    expect(screen.getByLabelText("Reference image fidelity")).toHaveValue("match");
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
