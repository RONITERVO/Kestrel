import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LocalSpeechProvider, SpeechDictationButton, SpeechLiveCaption, SpeechPlaybackButton, splitSpeechText } from "./LocalSpeechControls";

const speechApi = vi.hoisted(() => ({
  snapshot: vi.fn(),
  prepare: vi.fn(),
  synthesize: vi.fn(),
  align: vi.fn(),
  transcribe: vi.fn(),
  cancel: vi.fn(),
  release: vi.fn(),
}));

vi.mock("./api", async (importOriginal) => ({
  ...await importOriginal<typeof import("./api")>(),
  getLocalSpeechSnapshot: speechApi.snapshot,
  prepareLocalSpeech: speechApi.prepare,
  synthesizeLocalSpeech: speechApi.synthesize,
  alignLocalSpeech: speechApi.align,
  transcribeLocalSpeech: speechApi.transcribe,
  cancelLocalSpeech: speechApi.cancel,
  releaseLocalSpeechMemory: speechApi.release,
  localSpeechMediaUrl: (path: string) => `http://kestrel-speech.localhost/${path}`,
}));

const ready = {
  narrationAvailable: true,
  transcriptionAvailable: true,
  comfyReady: true,
  voices: [{ id: "chatterbox:local", name: "Local", provider: "ComfyUI Chatterbox" }],
  transcribers: [{ id: "whisper:turbo", name: "Whisper Turbo", provider: "ComfyUI Whisper" }],
  detail: "Ready",
};

beforeEach(() => {
  speechApi.snapshot.mockReset().mockResolvedValue(ready);
  speechApi.prepare.mockReset().mockResolvedValue(ready);
  speechApi.synthesize.mockReset().mockResolvedValue({
    jobId: "tts-1",
    passageId: "answer-1",
    relativePath: "generated/chat/chat-1/answer.opus",
    modelId: "chatterbox:local",
    cacheHit: false,
    segments: [{ value: "First sentence.", start: 0, end: 1 }],
    words: [
      { value: "First", start: 0, end: .4 },
      { value: "sentence.", start: .4, end: 1 },
    ],
  });
  speechApi.cancel.mockReset().mockResolvedValue(undefined);
  speechApi.align.mockReset().mockResolvedValue({
    jobId: "alignment-1",
    passageId: "answer-1",
    relativePath: "generated/chat/chat-1/answer.opus",
    modelId: "chatterbox:local",
    cacheHit: false,
    segments: [{ value: "First sentence.", start: 0, end: 1 }],
    words: [{ value: "First", start: 0, end: .4 }, { value: "sentence.", start: .4, end: 1 }],
  });
  speechApi.release.mockReset().mockResolvedValue(undefined);
  speechApi.transcribe.mockReset().mockImplementation(async (request: { jobId: string; recordingId: string; finalPass: boolean }) => ({
    jobId: request.jobId,
    recordingId: request.recordingId,
    text: "a clearer spoken direction",
    segments: [{ value: "a clearer spoken direction", start: 0, end: 1 }],
    words: [{ value: "clearer", start: .2, end: .5 }],
    audioRelativePath: request.finalPass ? "recordings/chat/chat-1/voice.webm" : undefined,
    finalPass: request.finalPass,
  }));
  vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => undefined);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("shared local speech controls", () => {
  it("splits long public responses without narrating fenced code", () => {
    const chunks = splitSpeechText(`A useful sentence.\n\n\`\`\`json\n{"secret":"tool"}\n\`\`\`\n${"Another sentence. ".repeat(150)}`, 160);
    expect(chunks.length).toBeGreaterThan(2);
    expect(chunks.every((chunk) => chunk.length <= 160)).toBe(true);
    expect(chunks.join(" ")).not.toContain("secret");
  });

  it("marks the currently spoken word inside its sentence", () => {
    render(<SpeechLiveCaption text="First sentence. Second sentence." seconds={1.4} duration={2} timings={[
      { value: "First", start: 0, end: .4 },
      { value: "sentence.", start: .4, end: 1 },
      { value: "Second", start: 1, end: 1.5 },
      { value: "sentence.", start: 1.5, end: 2 },
    ]} />);
    expect(screen.getByText("Second").tagName).toBe("MARK");
    expect(screen.queryByText("First")).not.toBeInTheDocument();
  });

  it("lets the user start from an aligned word", () => {
    const onSeek = vi.fn();
    render(<SpeechLiveCaption text="First sentence." seconds={0} duration={1} onSeek={onSeek} timings={[
      { value: "First", start: 0, end: .4 },
      { value: "sentence.", start: .4, end: 1 },
    ]} />);
    fireEvent.click(screen.getByRole("button", { name: "Start from sentence." }));
    expect(onSeek).toHaveBeenCalledWith(.4);
  });

  it("speaks only after the user requests a saved model response", async () => {
    const { container } = render(<LocalSpeechProvider><SpeechPlaybackButton sourceKind="chat" sourceId="chat-1" passageId="answer" text="First sentence." /></LocalSpeechProvider>);
    expect(speechApi.synthesize).not.toHaveBeenCalled();
    fireEvent.click(await screen.findByRole("button", { name: "Listen" }));
    await waitFor(() => expect(speechApi.synthesize).toHaveBeenCalledWith(expect.objectContaining({
      sourceKind: "chat",
      sourceId: "chat-1",
      text: "First sentence.",
    })));
    await waitFor(() => expect(HTMLMediaElement.prototype.play).toHaveBeenCalled());
    expect(container.querySelector("audio")?.src).toContain("kestrel-speech.localhost");
  });

  it("aligns the unchanged cached recording in the background when timings are missing", async () => {
    speechApi.synthesize.mockResolvedValueOnce({
      jobId: "tts-1",
      passageId: "answer-1",
      relativePath: "generated/chat/chat-1/answer.opus",
      modelId: "chatterbox:local",
      cacheHit: false,
      segments: [],
      words: [],
    });
    render(<LocalSpeechProvider><SpeechPlaybackButton sourceKind="chat" sourceId="chat-1" passageId="answer" text="First sentence." /></LocalSpeechProvider>);
    fireEvent.click(await screen.findByRole("button", { name: "Listen" }));
    await waitFor(() => expect(speechApi.align).toHaveBeenCalledWith(expect.objectContaining({
      relativePath: "generated/chat/chat-1/answer.opus",
      voiceModelId: "chatterbox:local",
      alignmentModelId: "whisper:turbo",
    })));
    expect(await screen.findByRole("slider", { name: "Speech position" })).toBeInTheDocument();
  });

  it("streams a provisional draft and saves a final low-bitrate microphone pass", async () => {
    class FakeRecorder {
      static isTypeSupported() { return true; }
      state: RecordingState = "inactive";
      mimeType = "audio/webm;codecs=opus";
      ondataavailable: ((event: BlobEvent) => void) | null = null;
      onerror: (() => void) | null = null;
      onstop: (() => void) | null = null;
      constructor(_stream: MediaStream, public options?: MediaRecorderOptions) {}
      start() { this.state = "recording"; }
      requestData() { this.ondataavailable?.({ data: new Blob([new Uint8Array(256)], { type: this.mimeType }) } as BlobEvent); }
      stop() {
        this.requestData();
        this.state = "inactive";
        this.onstop?.();
      }
    }
    const stopTrack = vi.fn();
    Object.defineProperty(navigator, "mediaDevices", { configurable: true, value: { getUserMedia: vi.fn(async () => ({ getTracks: () => [{ stop: stopTrack }] })) } });
    Object.defineProperty(globalThis, "MediaRecorder", { configurable: true, value: FakeRecorder });
    const onChange = vi.fn();
    const onActiveChange = vi.fn();
    render(<LocalSpeechProvider><SpeechDictationButton sourceKind="chat" sourceId="chat-1" value="Existing idea." onChange={onChange} onActiveChange={onActiveChange} /></LocalSpeechProvider>);

    fireEvent.click(await screen.findByRole("button", { name: "Dictate" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Stop dictation" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Stop dictation" }));
    await waitFor(() => expect(speechApi.transcribe).toHaveBeenCalledWith(expect.objectContaining({ finalPass: true, mimeType: "audio/webm;codecs=opus" })), { timeout: 2_000 });
    await waitFor(() => expect(onChange).toHaveBeenCalledWith("Existing idea. a clearer spoken direction"));
    await waitFor(() => expect(onActiveChange).toHaveBeenLastCalledWith(false));
    expect(onActiveChange).toHaveBeenCalledWith(true);
    expect(speechApi.release).toHaveBeenCalled();
    expect(stopTrack).toHaveBeenCalled();
  });
});
