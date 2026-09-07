import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MovieGenerationPanel, parseTimelineSeconds } from "./MovieGenerationPanel";
import { retainPreview } from "./MovieH3Preview";
import * as api from "../../../platform/api";
import type { ModelInfo, MovieEditorJob, MovieProject, MovieRenderPreviewEvent } from "../../../contracts/index";

vi.mock("../../../platform/api", async () => ({
  ...await vi.importActual<typeof import("../../../platform/api")>("../../../platform/api"),
  getMovieEditorState: vi.fn(async () => ({ editHash: "saved-hash", jobs: [] })),
  prepareMovieEditorRange: vi.fn(), startMovieEditorGeneration: vi.fn(async () => "job"),
  onMovieEditorJob: vi.fn(async () => () => undefined), getMovie: vi.fn(),
}));
afterEach(() => { cleanup(); vi.clearAllMocks(); });
const project = { id: "movie", edit: { clips: [], markers: [], exportTitle: "Movie", exportPreset: "publish", normalizeAudio: false, targetLufs: -14 } } as unknown as MovieProject;
const frame = { editId: "edit", clipId: "source", versionId: "original", sourcePath: "source.mp4", sourceSha256: "sha", sourceSeconds: 1, imagePath: "", imageSha256: "image", scenePrompt: "The keeper." };
const job: MovieEditorJob = { id: "job", projectId: "movie", status: "ready", detail: "Frames ready", createdAt: "", updatedAt: "", editHash: "saved-hash", startSeconds: 16.78, endSeconds: 70.01, durationSeconds: 5, placement: "replaceRange", direction: "Lifts his head", renderPrompt: "", modelId: "", seed: 42, first: frame, last: frame, clipId: "editor-job", resultPath: "", resultSha256: "", rawPath: "", rawSha256: "", placed: false };
function setup() {
  vi.mocked(api.getMovie).mockResolvedValue(project);
  vi.mocked(api.prepareMovieEditorRange).mockResolvedValue(job);
  render(<MovieGenerationPanel project={project} edit={project.edit} playhead={16.78} duration={120} models={[{ id: "qwen", name: "Qwen local" } as ModelInfo]} modelId="qwen" disabled={false} onProject={vi.fn()} onActive={vi.fn()} onError={vi.fn()} />);
}

describe("producer endpoint generation", () => {
  it("accepts decimal commas and time notation without accepting malformed numbers", () => {
    expect(parseTimelineSeconds("16,78")).toBe(16.78);
    expect(parseTimelineSeconds("1:10,01")).toBe(70.01);
    expect(parseTimelineSeconds("2:00:00")).toBe(7200);
    for (const invalid of ["", "-1", "Infinity", "1e3", "1:80", "1.5:20"]) expect(Number.isNaN(parseTimelineSeconds(invalid))).toBe(true);
  });
  it("submits the selected range and placement once, then lets native code run the take", async () => {
    setup();
    await waitFor(() => expect(api.getMovieEditorState).toHaveBeenCalled());
    fireEvent.change(screen.getByLabelText("Generation range start"), { target: { value: "16,78" } });
    fireEvent.change(screen.getByLabelText("Generation range end"), { target: { value: "70,01" } });
    fireEvent.change(screen.getByLabelText("New take direction"), { target: { value: "Lifts his head" } });
    fireEvent.change(screen.getByLabelText("Generated take placement"), { target: { value: "replaceRange" } });
    fireEvent.click(screen.getByRole("button", { name: "Generate take" }));
    await waitFor(() => expect(api.startMovieEditorGeneration).toHaveBeenCalledTimes(1));
    expect(api.prepareMovieEditorRange).toHaveBeenCalledWith({ projectId: "movie", expectedEditHash: "saved-hash", startSeconds: 16.78, endSeconds: 70.01, durationSeconds: 5, direction: "Lifts his head", placement: "replaceRange" }, project.edit);
    expect(api.startMovieEditorGeneration).toHaveBeenCalledWith({ projectId: "movie", jobId: "job", modelId: "qwen", renderPrompt: "" });
    const callback = vi.mocked(api.onMovieEditorJob).mock.calls[0][0];
    await act(async () => callback({ ...job, status: "complete", detail: "Take preserved" }));
    expect(screen.getByRole("button", { name: "Generate take" })).toBeEnabled();
    expect(screen.getByText("Take preserved")).toBeInTheDocument();
  });
  it("offers explicit generation of prepared endpoints without automatically starting it", async () => {
    setup();
    fireEvent.change(screen.getByLabelText("New take direction"), { target: { value: "Lifts his head" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview endpoints" }));
    await screen.findByText("Frames ready");
    expect(api.startMovieEditorGeneration).not.toHaveBeenCalled();
    expect(screen.getByText("16.78–70.01s → 5s · Replace saved range")).toBeInTheDocument();
  });
  it("retains the latest preview through status events but clears it for a different take", () => {
    const preview = { jobId: "one", dataUrl: "data:image/png;base64,a", mimeType: "image/png", kind: "frame" } as MovieRenderPreviewEvent;
    expect(retainPreview(preview, { ...preview, kind: "finished", dataUrl: undefined }).dataUrl).toBe(preview.dataUrl);
    expect(retainPreview(preview, { ...preview, jobId: "two", kind: "connected", dataUrl: undefined }).dataUrl).toBeUndefined();
  });
});
