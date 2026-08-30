import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoReport } from "../../app/demo";
import { ResearchSpeechPlayer } from "./ResearchSpeech";
import { buildResearchSpeechPassages } from "./researchSpeechContent";
import { LocalSpeechProvider } from "../speech/LocalSpeechControls";

const speechApi = vi.hoisted(() => ({
  snapshot: vi.fn(),
  synthesize: vi.fn(),
  align: vi.fn(),
  cancel: vi.fn(),
  prepare: vi.fn(),
  release: vi.fn(),
}));

vi.mock("../../platform/api", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../platform/api")>(),
  getLocalSpeechSnapshot: speechApi.snapshot,
  synthesizeLocalSpeech: speechApi.synthesize,
  alignLocalSpeech: speechApi.align,
  cancelLocalSpeech: speechApi.cancel,
  prepareLocalSpeech: speechApi.prepare,
  releaseLocalSpeechMemory: speechApi.release,
  onLocalSpeechProgress: vi.fn(async () => () => undefined),
  localSpeechMediaUrl: (path: string) => `http://kestrel-speech.localhost/${path}`,
}));

beforeEach(() => {
  speechApi.snapshot.mockReset().mockResolvedValue({
    narrationAvailable: true,
    transcriptionAvailable: true,
    comfyReady: true,
    voices: [{ id: "chatterbox:local_narrator", name: "Local Narrator", provider: "ComfyUI Chatterbox" }],
    transcribers: [{ id: "whisper:large-v3-turbo", name: "Whisper Large V3 Turbo", provider: "ComfyUI Whisper" }],
    voiceProfiles: [{ id: "voice-default", name: "Chatterbox Default", language: "Auto", tags: ["Built in"], source: "built-in", consentConfirmed: true, performance: "natural", createdAt: "", updatedAt: "" }],
    defaultVoiceProfileId: "voice-default",
    detail: "ComfyUI TTS is ready.",
  });
  speechApi.synthesize.mockReset().mockImplementation(async (request: { passageId: string; jobId: string; modelId: string }) => ({
    jobId: request.jobId,
    passageId: request.passageId,
    relativePath: `${demoReport.id}/${request.passageId}.flac`,
    modelId: request.modelId,
    voiceProfileId: "voice-default",
    cacheHit: false,
    segments: [],
    words: [],
  }));
  speechApi.cancel.mockReset().mockResolvedValue(undefined);
  speechApi.align.mockReset().mockImplementation(async (request: { jobId: string; passageId: string; relativePath: string; voiceModelId: string }) => ({
    jobId: request.jobId,
    passageId: request.passageId,
    relativePath: request.relativePath,
    modelId: request.voiceModelId,
    voiceProfileId: "voice-default",
    cacheHit: false,
    segments: [{ value: "Aligned passage.", start: 0, end: 1 }],
    words: [{ value: "Aligned", start: 0, end: .5 }, { value: "passage.", start: .5, end: 1 }],
  }));
  speechApi.release.mockReset().mockResolvedValue(undefined);
  speechApi.prepare.mockReset().mockResolvedValue({
    narrationAvailable: true,
    transcriptionAvailable: true,
    comfyReady: true,
    voices: [{ id: "chatterbox:local_narrator", name: "Local Narrator", provider: "ComfyUI Chatterbox" }],
    transcribers: [{ id: "whisper:large-v3-turbo", name: "Whisper Large V3 Turbo", provider: "ComfyUI Whisper" }],
    voiceProfiles: [{ id: "voice-default", name: "Chatterbox Default", language: "Auto", tags: ["Built in"], source: "built-in", consentConfirmed: true, performance: "natural", createdAt: "", updatedAt: "" }],
    defaultVoiceProfileId: "voice-default",
    detail: "ComfyUI TTS is ready.",
  });
  vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => undefined);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Research speech", () => {
  const renderPlayer = (onPassageChange: (anchorId: string | null, passageId?: string | null) => void) => render(
    <LocalSpeechProvider><ResearchSpeechPlayer report={demoReport} onPassageChange={onPassageChange} /></LocalSpeechProvider>,
  );

  it("builds short buffered passages and offers explicit source narration", () => {
    const longReport = {
      ...demoReport,
      sections: [{
        ...demoReport.sections[0],
        body: [`${"A realistic long sentence. ".repeat(80)} ${"continuation ".repeat(180)}`],
      }],
    };
    const summary = buildResearchSpeechPassages(longReport, "summary");
    const article = buildResearchSpeechPassages(longReport, "article");
    const all = buildResearchSpeechPassages(longReport, "all");

    expect(summary.map((passage) => passage.label)).toContain("Short answer");
    expect(summary.some((passage) => passage.label === "What it was")).toBe(false);
    expect(article.some((passage) => passage.label === "What it was")).toBe(true);
    expect(article.findIndex((passage) => passage.label === "What changed")).toBeLessThan(article.findIndex((passage) => passage.label === "Key findings"));
    expect(article.every((passage) => passage.text.length <= 320)).toBe(true);
    expect(new Set(article.map((passage) => passage.id)).size).toBe(article.length);
    expect(article.map((passage) => passage.text).join(" ")).not.toContain(demoReport.sources[0].excerpt);
    expect(all.map((passage) => passage.text).join(" ")).toContain(demoReport.sources[0].excerpt);
  });

  it("generates with the selected ComfyUI model, plays local audio, and buffers ahead", async () => {
    const onPassageChange = vi.fn();
    const { container } = renderPlayer(onPassageChange);

    const play = await screen.findByRole("button", { name: "Play report" });
    await waitFor(() => expect(play).toBeEnabled());
    expect(screen.getByRole("combobox", { name: "Narration voice" })).toHaveValue("voice-default");
    fireEvent.click(play);

    await waitFor(() => expect(speechApi.synthesize).toHaveBeenCalled());
    expect(speechApi.synthesize.mock.calls[0][0]).toMatchObject({
      sourceKind: "research",
      sourceId: demoReport.id,
      passageId: "overview",
      modelId: "chatterbox:local_narrator",
      voiceProfileId: "voice-default",
    });
    await waitFor(() => expect(HTMLMediaElement.prototype.play).toHaveBeenCalled());
    await waitFor(() => expect(speechApi.align).toHaveBeenCalledWith(expect.objectContaining({
      sourceKind: "research",
      sourceId: demoReport.id,
      alignmentModelId: "whisper:large-v3-turbo",
    })));
    expect(screen.getByRole("slider", { name: "Current passage position" })).toBeInTheDocument();
    expect(onPassageChange).toHaveBeenCalledWith("report-overview", "overview");
    await waitFor(() => expect(speechApi.synthesize.mock.calls.some(([request]) => request.passageId === "short-answer")).toBe(true));

    fireEvent.click(await screen.findByRole("button", { name: "Pause report" }));
    expect(HTMLMediaElement.prototype.pause).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Play report" }));
    await waitFor(() => expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(2));

    fireEvent.ended(container.querySelector("audio")!);
    await waitFor(() => expect(onPassageChange).toHaveBeenLastCalledWith("short-answer", "short-answer"));
    fireEvent.click(screen.getByRole("button", { name: "Stop report" }));
    expect(onPassageChange).toHaveBeenLastCalledWith(null, null);
    expect(speechApi.release).toHaveBeenCalled();
  });

  it("cancels pending alignment instead of delaying producer navigation", async () => {
    let finishAlignment: ((value: unknown) => void) | undefined;
    speechApi.align.mockImplementationOnce((request: { jobId: string; passageId: string; relativePath: string; voiceModelId: string }) => new Promise((resolve) => {
      finishAlignment = resolve;
    }));
    const onPassageChange = vi.fn();
    renderPlayer(onPassageChange);
    fireEvent.click(await screen.findByRole("button", { name: "Play report" }));
    await waitFor(() => expect(speechApi.align).toHaveBeenCalled());
    const alignmentJob = speechApi.align.mock.calls[0][0].jobId;

    fireEvent.click(screen.getByRole("button", { name: "Next passage" }));

    expect(speechApi.cancel).toHaveBeenCalledWith(alignmentJob);
    await waitFor(() => expect(onPassageChange).toHaveBeenLastCalledWith("short-answer", "short-answer"));
    finishAlignment?.({
      jobId: alignmentJob,
      passageId: "overview",
      relativePath: "generated/research/overview.flac",
      modelId: "chatterbox:local_narrator",
      cacheHit: false,
      segments: [],
      words: [],
    });
  });

  it("never offers a browser or system voice fallback", async () => {
    speechApi.snapshot.mockResolvedValue({
      narrationAvailable: false,
      transcriptionAvailable: false,
      comfyReady: false,
      voices: [],
      transcribers: [],
      voiceProfiles: [],
      defaultVoiceProfileId: "voice-default",
      detail: "No complete local ComfyUI voice pack was found.",
    });
    renderPlayer(() => undefined);

    expect(await screen.findByText("ComfyUI TTS unavailable")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Play report" })).toBeDisabled();
    expect(screen.getByRole("combobox", { name: "Narration voice" })).toHaveTextContent("No local voice");
    expect(screen.queryByLabelText(/system voice|offline voice/i)).not.toBeInTheDocument();
  });
});
