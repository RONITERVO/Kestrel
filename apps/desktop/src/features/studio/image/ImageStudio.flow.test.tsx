import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ImageProject, PromptDraftEvent } from "../../../contracts/index";
import { sampleImage } from "../../../preview/studioFixtures";
import { demoSnapshot } from "../../../preview/fixtures";
import { ImageStudio, imageCompositionPreview } from "./ImageStudio";

const api = vi.hoisted(() => ({
  save: vi.fn(), start: vi.fn(), draft: vi.fn(), cancel: vi.fn(), cancelDraft: vi.fn(),
  get: vi.fn(), list: vi.fn(),
  onDraft: undefined as ((event: PromptDraftEvent) => void) | undefined,
}));
vi.mock("../../../platform/api", async (original) => ({
  ...await original<typeof import("../../../platform/api")>(),
  saveImageProject: api.save, startImageGeneration: api.start, startStudioPromptDraft: api.draft,
  cancelImageGeneration: api.cancel, cancelStudioPromptDraft: api.cancelDraft,
  getImageProject: api.get, listImageProjects: api.list,
  onImageGeneration: async () => () => {}, onImageProjectUpdated: async () => () => {},
  onStudioPromptDraft: async (callback: (event: PromptDraftEvent) => void) => { api.onDraft = callback; return () => {}; },
}));

let stored: ImageProject;
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
beforeEach(() => {
  vi.clearAllMocks();
  stored = structuredClone(sampleImage);
  api.get.mockImplementation(async () => structuredClone(stored));
  api.list.mockImplementation(async () => [{ id: stored.id, title: stored.title, status: stored.status, updatedAt: stored.updatedAt, takeCount: stored.takes.length, activeTakePath: "" }]);
  api.save.mockImplementation(async (edit: ImageProject) => { stored = structuredClone(edit); return structuredClone(stored); });
  api.start.mockImplementation(async () => ({ ...stored, status: "generating" }));
  api.draft.mockResolvedValue("draft");
});

async function open(onError = vi.fn()) {
  const ui = render(<ImageStudio advancedEnabled models={demoSnapshot.control.models} selectedModelId={demoSnapshot.control.models[0]?.id} onError={onError} />);
  await screen.findByLabelText("Image project title");
  return ui;
}

describe("MOSS producer flow", () => {
  it("keeps a streamed proposal unapplied until the producer accepts it, then saves before rendering", async () => {
    await open();
    fireEvent.change(screen.getByLabelText("Producer brief"), { target: { value: "Generate a poster with exact title CAFÉ." } });
    fireEvent.click(screen.getByRole("button", { name: "Develop design" }));
    await waitFor(() => expect(api.draft).toHaveBeenCalledOnce());
    const request = api.draft.mock.calls[0][0];
    expect(request).toMatchObject({ target: "imageComposition", mode: "develop", storyText: "Generate a poster with exact title CAFÉ." });
    expect(api.save).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled();
    const proposal = imageCompositionPreview({ ...stored, highLevelDescription: "A complete café poster", elements: [{ ...stored.elements[0], text: "CAFÉ" }] });
    const send = (kind: string, content?: string) => api.onDraft?.({ requestId: request.requestId, kind, content: content ?? null, modelName: "Local assistant", thinkingLevel: "high", receipt: null, at: new Date().toISOString() });
    act(() => { send("reasoning", "Plan exact lettering and placement."); send("token", JSON.stringify(proposal)); send("complete"); });
    expect(screen.getByLabelText("Visible wording")).toHaveValue("NIGHT / FORM");
    fireEvent.click(screen.getByRole("button", { name: "Apply proposal" }));
    expect(screen.getByLabelText("Visible wording")).toHaveValue("CAFÉ");
    expect(api.start).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Create image" }));
    await waitFor(() => expect(api.start).toHaveBeenCalledWith(stored.id));
    expect(api.save.mock.calls[0][0].elements[0].text).toBe("CAFÉ");
    expect(api.save.mock.invocationCallOrder[0]).toBeLessThan(api.start.mock.invocationCallOrder[0]);
  });

  it("saves the new negative prompt, preset, batch and full-width seed and locks edits during rendering", async () => {
    await open();
    fireEvent.click(screen.getByRole("button", { name: "Advanced" }));
    fireEvent.change(screen.getByLabelText("Negative prompt"), { target: { value: "watermark, red" } });
    fireEvent.change(screen.getByLabelText("Sampling preset"), { target: { value: "quality" } });
    fireEvent.change(screen.getByLabelText("Variations in one render"), { target: { value: "4" } });
    fireEvent.change(screen.getByLabelText("Seed"), { target: { value: "9223372036854775807" } });
    fireEvent.click(screen.getByRole("button", { name: "Create image" }));
    await waitFor(() => expect(api.start).toHaveBeenCalledOnce());
    expect(stored.settings).toMatchObject({ negativePrompt: "watermark, red", preset: "quality", batchSize: 4, seed: "9223372036854775807" });
    expect(screen.getByLabelText("Negative prompt")).toBeDisabled();
    expect(screen.getByLabelText("Sampling preset")).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(api.cancel).toHaveBeenCalledWith(stored.id);
  });

  it("keeps normalized box edits, duplicate/layer order and keyboard nudges familiar", async () => {
    const { container } = await open();
    fireEvent.change(screen.getByLabelText("Top", { exact: true }), { target: { value: "120" } });
    fireEvent.change(screen.getByLabelText("Left", { exact: true }), { target: { value: "150" } });
    fireEvent.click(screen.getByTitle("Duplicate layer · Ctrl/Cmd+D"));
    fireEvent.keyDown(container.querySelector(".image-studio")!, { key: "ArrowRight", shiftKey: true });
    fireEvent.click(screen.getByTitle("Move layer down"));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.save).toHaveBeenCalledOnce());
    expect(stored.elements).toHaveLength(2);
    expect(stored.elements[0].bbox).toEqual([140, 180, 340, 930]);
    expect(stored.elements[1].bbox).toEqual([120, 150, 320, 900]);
    expect(stored.elements[0].id).not.toBe(stored.elements[1].id);
  });

  it("rejects a truncated assistant checkpoint without changing the design or submitting a render", async () => {
    const onError = vi.fn();
    await open(onError);
    fireEvent.click(screen.getByRole("button", { name: "Develop design" }));
    await waitFor(() => expect(api.draft).toHaveBeenCalledOnce());
    const requestId = api.draft.mock.calls[0][0].requestId;
    fireEvent.click(screen.getByRole("button", { name: "Stop with checkpoint" }));
    expect(api.cancelDraft).toHaveBeenCalledWith(requestId);
    act(() => {
      for (const [kind, content] of [["token", '{"high_level_description":'], ["cancelled", null]] as const) api.onDraft?.({ requestId, kind, content, modelName: null, thinkingLevel: null, receipt: null, at: new Date().toISOString() });
    });
    fireEvent.click(screen.getByRole("button", { name: "Apply proposal" }));
    expect(onError).toHaveBeenCalledOnce();
    expect(api.save).not.toHaveBeenCalled();
    expect(api.start).not.toHaveBeenCalled();
  });

  it("draws, drags and resizes boxes through the canvas pointer handlers", async () => {
    // jsdom has no physical layout or PointerEvent constructor. Supply a measured board and
    // pointer coordinates, then exercise the same handlers used by the desktop WebView.
    vi.stubGlobal("PointerEvent", MouseEvent);
    const { container } = await open();
    const board = container.querySelector(".image-artboard")!;
    vi.spyOn(board, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, width: 1000, height: 1000, right: 1000, bottom: 1000, toJSON: () => ({}) });
    fireEvent.click(screen.getByTitle("Draw an object box"));
    fireEvent.pointerDown(board, { clientX: 200, clientY: 400 });
    fireEvent.pointerMove(window, { clientX: 500, clientY: 700 });
    fireEvent.pointerUp(window, { clientX: 500, clientY: 700 });
    let selected = container.querySelector(".image-layout-box.selected")!;
    fireEvent.pointerDown(selected, { clientX: 300, clientY: 500 });
    fireEvent.pointerMove(window, { clientX: 400, clientY: 550 });
    fireEvent.pointerUp(window, { clientX: 400, clientY: 550 });
    selected = container.querySelector(".image-layout-box.selected")!;
    fireEvent.pointerDown(selected.querySelector("b")!, { clientX: 600, clientY: 750 });
    fireEvent.pointerMove(window, { clientX: 650, clientY: 800 });
    fireEvent.pointerUp(window, { clientX: 650, clientY: 800 });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(api.save).toHaveBeenCalledOnce());
    expect(stored.elements[1]).toMatchObject({ kind: "obj", bbox: [450, 300, 800, 650] });
    expect(stored.elements[0].bbox).toEqual([80, 100, 320, 900]);
  });

  it("shows historical take terms without Advanced and delivers the selected preserved take", async () => {
    stored.takes = [{
      id: "legacy-take", createdAt: stored.createdAt, status: "complete", detail: "Preserved", error: "",
      path: "D:/acceptance/legacy.png", bytes: 512, sha256: "a".repeat(64), width: 1024, height: 1024,
      preset: "standard", seed: "42621", batchIndex: 1, batchSize: 1, promptId: "legacy-prompt",
      exactPrompt: {}, exactPromptText: "legacy caption", exactGraph: {}, modelProfile: "Ideogram 4 original profile",
      licenseNotice: "Original non-commercial model terms remain applicable.",
    }];
    stored.activeTakeId = "legacy-take";
    stored.licenseNotice = "New renders use MOSS-Image Base under Apache 2.0.";
    const deliver = vi.fn().mockResolvedValue(undefined);
    const done = vi.fn();
    render(<ImageStudio advancedEnabled={false} onError={vi.fn()} handoff={{ production: "Test movie", purpose: "poster", direction: "Use the preserved poster", deliver }} onHandoffDone={done} />);
    await screen.findByLabelText("Image project title");
    fireEvent.click(screen.getByRole("button", { name: "Takes" }));
    expect(screen.getByText("Ideogram 4 original profile")).toBeTruthy();
    expect(screen.getByText("Original non-commercial model terms remain applicable.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Use in Test movie" }));
    await waitFor(() => expect(done).toHaveBeenCalledWith(true));
    expect(deliver).toHaveBeenCalledWith(stored.id, "legacy-take");
    expect(api.start).not.toHaveBeenCalled();
    expect(api.save).not.toHaveBeenCalled();
  });
});
