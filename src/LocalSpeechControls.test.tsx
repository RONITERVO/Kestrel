import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { advanceLiveTranscriptionCheckpoint, completeRecordingBlob, LIVE_TRANSCRIPTION_CHECKPOINTS_SECONDS, LocalSpeechProvider, mergeProvisionalTranscript, SpeechDictationButton, SpeechLiveCaption, SpeechPlaybackButton, splitSpeechText, VadSettingsModal, type SpeechProgressState } from "./LocalSpeechControls";
import { DEFAULT_VAD_SETTINGS } from "./voiceActivityDetection";

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
  voiceProfiles: [{ id: "voice-default", name: "Chatterbox Default", language: "Auto", tags: ["Built in"], source: "built-in", consentConfirmed: true, performance: "natural", createdAt: "", updatedAt: "" }],
  defaultVoiceProfileId: "voice-default",
  detail: "Ready",
};

class FakeRecorder {
  static isTypeSupported() { return true; }
  state: RecordingState = "inactive";
  mimeType = "audio/webm;codecs=opus";
  ondataavailable: ((event: BlobEvent) => void) | null = null;
  onerror: (() => void) | null = null;
  onstop: (() => void) | null = null;
  constructor(_stream: MediaStream, public options?: MediaRecorderOptions) {}
  start() { this.state = "recording"; }
  requestData() {
    this.ondataavailable?.({
      data: new Blob([new Uint8Array(256)], { type: this.mimeType }),
    } as BlobEvent);
  }
  stop() {
    this.requestData();
    this.state = "inactive";
    this.onstop?.();
  }
}

beforeEach(() => {
  speechApi.snapshot.mockReset().mockResolvedValue(ready);
  speechApi.prepare.mockReset().mockResolvedValue(ready);
  speechApi.synthesize.mockReset().mockImplementation(async (request: { passageId: string; jobId: string; modelId: string }) => ({
    jobId: request.jobId,
    passageId: request.passageId,
    relativePath: `generated/chat/chat-1/${request.passageId}.opus`,
    modelId: request.modelId,
    voiceProfileId: "voice-default",
    cacheHit: false,
    segments: [{ value: "Spoken sentence.", start: 0, end: 1 }],
    words: [
      { value: "Spoken", start: 0, end: .4 },
      { value: "sentence.", start: .4, end: 1 },
    ],
  }));
  speechApi.cancel.mockReset().mockResolvedValue(undefined);
  speechApi.align.mockReset().mockResolvedValue({
    jobId: "alignment-1",
    passageId: "answer-1",
    relativePath: "generated/chat/chat-1/answer.opus",
    modelId: "chatterbox:local",
    voiceProfileId: "voice-default",
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

  it("sanitizes text formatted charts and markdown tables into natural spoken sentences", () => {
    const tableMarkdown = `
Here is the performance overview:
| Model | Latency | Accuracy |
| :--- | :--- | :--- |
| Chatterbox | 400ms | 99% |
| Whisper | 250ms | 98.5% |

+--------------------+
| Architecture Chart |
+--------------------+
`;
    const chunks = splitSpeechText(tableMarkdown);
    const text = chunks.join(" ");
    expect(text).not.toContain("|");
    expect(text).not.toContain("+");
    expect(text).not.toContain(":---");
    expect(text).toContain("Chatterbox: 400ms, 99 percent.");
    expect(text).toContain("Whisper: 250ms, 98 point 5 percent.");
    expect(text).toContain("Architecture Chart.");
  });

  it("speaks dashboard chemistry, thresholds, deltas, arrows, and ratios without glyph noise", () => {
    const text = splitSpeechText("O₂ 19.8% ≥ 20.0%; CO₂ 0.52% ≤ 0.45%; expected Δ O₂ +0.4% → stable; 88/100 ↑").join(" ");
    expect(text).toBe("oxygen 19 point 8 percent at least 20 point 0 percent; carbon dioxide 0 point 52 percent at most 0 point 45 percent; expected change in oxygen plus 0 point 4 percent then stable; 88 out of 100 rising");
  });

  it("does not split decimals or abbreviations across pipelined passages", () => {
    const chunks = splitSpeechText(
      `${"A long setup sentence keeps this passage near its boundary. ".repeat(5)}Oxygen remains at 20.0 percent in a different setting, e.g., a station. Final sentence.`,
      120,
    );
    expect(chunks.join(" ")).toContain("20 point 0 percent");
    expect(chunks.join(" ")).toContain("for example, a station");
    expect(chunks).not.toContain("e.");
    expect(chunks).not.toContain("g., a station. Final sentence.");
  });

  it("removes unpronounceable symbol clusters and stutter triggers like '#¤-_''*+' without breaking natural human words", () => {
    const raw = `The latest update (#¤-_''*+) delivered $50M in savings and +15% performance! Check foo_bar_baz at https://kestrel.local/docs for details.`;
    const chunks = splitSpeechText(raw);
    const text = chunks.join(" ");
    expect(text).not.toContain("#");
    expect(text).not.toContain("¤");
    expect(text).not.toContain("*");
    expect(text).not.toContain("https://");
    expect(text).toContain("50M dollars");
    expect(text).toContain("plus 15 percent");
    expect(text).toContain("foo bar baz");
    expect(text).toContain("kestrel.local");
  });

  it("preserves multilingual sentences, natural contractions, and hyphenated terms", () => {
    const multilingual = `Don't worry, it's a state-of-the-art system. Älä huoli, kaikki toimii loistavasti. Das ist großartig! 東京は晴れです。`;
    const chunks = splitSpeechText(multilingual);
    const text = chunks.join(" ");
    expect(text).toContain("Don't worry, it's a state-of-the-art system.");
    expect(text).toContain("Älä huoli, kaikki toimii loistavasti.");
    expect(text).toContain("Das ist großartig!");
    expect(text).toContain("東京は晴れです。");
  });

  it("speaks ISO dates and signed values coherently while dropping stylesheet fragments", () => {
    const text = splitSpeechText(
      ".mw-parser-output .marriage-display-ws{display:inline} Recorded 1930-08-05 at -12 degrees.",
    ).join(" ");
    expect(text).toContain("1930 08 05");
    expect(text).toContain("minus 12 degrees");
    expect(text).not.toContain("mw-parser-output");
    expect(text).not.toMatch(/[{}]/);
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

  it("keeps the WebM header in every growing provisional recording", async () => {
    const header = new Blob([new Uint8Array([0x1a, 0x45, 0xdf, 0xa3])], { type: "audio/webm" });
    const laterFragment = new Blob([new Uint8Array([0x81, 0x82, 0x83])], { type: "audio/webm" });
    const firstPass = new Uint8Array(await completeRecordingBlob([header], "audio/webm").arrayBuffer());
    const nextPass = new Uint8Array(await completeRecordingBlob([header, laterFragment], "audio/webm").arrayBuffer());

    expect([...firstPass]).toEqual([0x1a, 0x45, 0xdf, 0xa3]);
    expect([...nextPass]).toEqual([0x1a, 0x45, 0xdf, 0xa3, 0x81, 0x82, 0x83]);
  });

  it("bounds live full-recording transcription to eight checkpoints", () => {
    let nextIndex = 0;
    const checkpoints: number[] = [];
    for (let elapsed = 4; elapsed <= 15 * 60; elapsed += 4) {
      const advanced = advanceLiveTranscriptionCheckpoint(elapsed, nextIndex);
      if (advanced !== nextIndex) checkpoints.push(elapsed);
      nextIndex = advanced;
    }
    expect(checkpoints).toEqual([...LIVE_TRANSCRIPTION_CHECKPOINTS_SECONDS]);
    expect(checkpoints.reduce((sum, seconds) => sum + seconds, 0)).toBeLessThan(30 * 60);
  });

  it("merges checkpoint transcripts without repeating the shared WebM header speech", () => {
    expect(mergeProvisionalTranscript(
      "The first spoken thought continues",
      "The first new detail arrives",
    )).toBe("The first spoken thought continues new detail arrives");
  });

  it("keeps VAD settings modal, labeled, focused, and dismissible from the keyboard", () => {
    const onClose = vi.fn();
    render(
      <VadSettingsModal
        settings={DEFAULT_VAD_SETTINGS}
        onChange={vi.fn()}
        onReset={vi.fn()}
        onClose={onClose}
      />,
    );
    expect(screen.getByRole("dialog")).toHaveAttribute("aria-modal", "true");
    expect(screen.getByRole("slider", { name: "Silence duration" })).toBeInTheDocument();
    expect(screen.getByRole("slider", { name: "Microphone speech threshold" })).toBeInTheDocument();
    expect(screen.getByRole("slider", { name: "Minimum speech trigger duration" })).toBeInTheDocument();
    expect(screen.getByRole("slider", { name: "Initial grace period" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Close VAD settings" })).toHaveFocus();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
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

  it("lets a producer cast an individual response without changing the app-wide default", async () => {
    speechApi.snapshot.mockResolvedValueOnce({
      ...ready,
      voiceProfiles: [
        ...ready.voiceProfiles,
        { id: "voice-narrator", name: "Evening Narrator", language: "English", tags: ["Warm"], source: "imported", consentConfirmed: true, performance: "expressive", createdAt: "", updatedAt: "" },
      ],
    });
    render(<LocalSpeechProvider><SpeechPlaybackButton sourceKind="chat" sourceId="chat-1" passageId="answer" text="First sentence." /></LocalSpeechProvider>);

    const selector = await screen.findByRole("combobox", { name: "Voice for listen" });
    fireEvent.change(selector, { target: { value: "voice-narrator" } });
    fireEvent.click(screen.getByRole("button", { name: "Listen" }));

    await waitFor(() => expect(speechApi.synthesize).toHaveBeenCalledWith(expect.objectContaining({
      voiceProfileId: "voice-narrator",
    })));
  });

  it("pipelines and prebuffers multiple passages in inline speech responses", async () => {
    const sentence1 = "First important sentence that explains the initial context in full detail.".repeat(4);
    const sentence2 = "Second crucial paragraph that continues the explanation thoroughly.".repeat(4);
    const longText = `${sentence1}\n\n${sentence2}`;
    const onSpeechProgress = vi.fn();
    const { container } = render(
      <LocalSpeechProvider>
        <SpeechPlaybackButton sourceKind="chat" sourceId="chat-1" passageId="answer" text={longText} onSpeechProgress={onSpeechProgress} />
      </LocalSpeechProvider>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Listen" }));
    await waitFor(() => expect(speechApi.synthesize).toHaveBeenCalled());
    expect(speechApi.synthesize.mock.calls[0][0]).toMatchObject({
      sourceKind: "chat",
      sourceId: "chat-1",
      passageId: "answer-1",
    });

    // Proactive background pre-buffering kicks off for passage 2
    await waitFor(() => expect(speechApi.synthesize.mock.calls.some(([req]) => req.passageId === "answer-2")).toBe(true));
    await waitFor(() => expect(onSpeechProgress).toHaveBeenCalledWith(expect.objectContaining({
      seekablePassages: expect.arrayContaining([
        expect.objectContaining({ passageId: "answer-1" }),
        expect.objectContaining({ passageId: "answer-2" }),
      ]),
      onSeekPassage: expect.any(Function),
    })));

    // When audio completes passage 1, it seamlessly transitions to passage 2
    fireEvent.ended(container.querySelector("audio")!);
    await waitFor(() => expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(2));

    // Any earlier generated passage remains directly seekable while passage 2 is active.
    const cachedProgress = [...onSpeechProgress.mock.calls]
      .reverse()
      .map(([value]) => value as SpeechProgressState | null)
      .find((value) => (value?.seekablePassages?.length ?? 0) >= 2);
    await act(async () => cachedProgress?.onSeekPassage?.("answer-1", 0.25));
    await waitFor(() => expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(3));
    expect(container.querySelector("audio")?.currentTime).toBe(0.25);
    expect(speechApi.synthesize.mock.calls.filter(([request]) => request.passageId === "answer-1")).toHaveLength(1);

    // Stop resets and releases memory
    fireEvent.click(screen.getByRole("button", { name: "Stop speaking" }));
    expect(speechApi.release).toHaveBeenCalled();
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

  it("shows preparation failure instead of leaving the microphone looking inert", async () => {
    speechApi.snapshot.mockResolvedValueOnce({ ...ready, transcriptionAvailable: false, transcribers: [], detail: "Whisper is missing." });
    speechApi.prepare.mockRejectedValueOnce(new Error("Whisper is missing."));
    render(<LocalSpeechProvider><SpeechDictationButton sourceKind="chat" sourceId="chat-1" value="" onChange={vi.fn()} /></LocalSpeechProvider>);

    fireEvent.click(await screen.findByRole("button", { name: "Dictate" }));

    expect(await screen.findByRole("status")).toHaveTextContent(/Dictation unavailable.*Whisper is missing.*Open Setup/i);
    expect(screen.getByRole("button", { name: "Dictate" })).toBeEnabled();
  });

  it("releases an acquired microphone when MediaRecorder startup fails", async () => {
    const stopTrack = vi.fn();
    class FailingRecorder {
      static isTypeSupported() { return true; }
      state: RecordingState = "inactive";
      mimeType = "audio/webm;codecs=opus";
      ondataavailable = null;
      onerror = null;
      onstop = null;
      constructor(_stream: MediaStream, _options?: MediaRecorderOptions) {}
      start() { throw new Error("recorder startup failed"); }
      stop() {}
      requestData() {}
    }
    Object.defineProperty(navigator, "mediaDevices", { configurable: true, value: { getUserMedia: vi.fn(async () => ({ getTracks: () => [{ stop: stopTrack }] })) } });
    Object.defineProperty(globalThis, "MediaRecorder", { configurable: true, value: FailingRecorder });
    render(<LocalSpeechProvider><SpeechDictationButton sourceKind="chat" sourceId="chat-1" value="" onChange={vi.fn()} /></LocalSpeechProvider>);

    fireEvent.click(await screen.findByRole("button", { name: "Dictate" }));

    expect(await screen.findByRole("status")).toHaveTextContent(/Dictation unavailable.*recorder startup failed/i);
    expect(stopTrack).toHaveBeenCalledOnce();
  });

  it("plays back recorded user voice directly with word timestamps without synthesizing TTS", async () => {
    const onSpeechProgress = vi.fn();
    render(
      <LocalSpeechProvider>
        <SpeechPlaybackButton
          sourceKind="chat"
          sourceId="chat-1"
          passageId="msg-user-1"
          text="This is my spoken question."
          recording={{
            audioRelativePath: "recordings/chat/chat-1/voice.webm",
            words: [
              { value: "This", start: 0, end: 0.3 },
              { value: "is", start: 0.3, end: 0.5 },
              { value: "my", start: 0.5, end: 0.8 },
              { value: "spoken", start: 0.8, end: 1.2 },
              { value: "question.", start: 1.2, end: 1.6 },
            ],
          }}
          label="Listen"
          onSpeechProgress={onSpeechProgress}
        />
      </LocalSpeechProvider>,
    );

    const button = await screen.findByRole("button", { name: "Listen" });
    fireEvent.click(button);

    await waitFor(() => {
      expect(HTMLMediaElement.prototype.play).toHaveBeenCalled();
    });
    // TTS synthesis should NOT be called for user voice recording!
    expect(speechApi.synthesize).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(onSpeechProgress).toHaveBeenCalledWith(
        expect.objectContaining({
          active: true,
          sourceKind: "chat",
          timings: expect.arrayContaining([
            expect.objectContaining({ value: "spoken", start: 0.8, end: 1.2 }),
          ]),
        }),
      );
    });
  });

  it("starts a saved recording at a clicked word without requiring Listen first", async () => {
    const onSpeechProgress = vi.fn();
    render(
      <LocalSpeechProvider>
        <SpeechPlaybackButton
          sourceKind="chat"
          sourceId="chat-1"
          passageId="msg-user-1"
          text="This is my spoken question."
          recording={{
            audioRelativePath: "recordings/chat/chat-1/voice.webm",
            words: [
              { value: "This", start: 0, end: 0.3 },
              { value: "is", start: 0.3, end: 0.5 },
              { value: "my", start: 0.5, end: 0.8 },
              { value: "spoken", start: 0.8, end: 1.2 },
              { value: "question", start: 1.2, end: 1.6 },
            ],
          }}
          label="Listen"
          onSpeechProgress={onSpeechProgress}
        />
      </LocalSpeechProvider>,
    );

    await waitFor(() => expect(onSpeechProgress).toHaveBeenCalledWith(expect.objectContaining({
      active: false,
      timings: expect.arrayContaining([expect.objectContaining({ value: "spoken", start: 0.8 })]),
      onSeek: expect.any(Function),
    })));
    const progress = [...onSpeechProgress.mock.calls]
      .reverse()
      .map(([value]) => value as SpeechProgressState | null)
      .find((value) => value?.timings.length);

    await act(async () => progress?.onSeek?.(0.8));

    await waitFor(() => expect(HTMLMediaElement.prototype.play).toHaveBeenCalled());
    expect(document.querySelector("audio")?.currentTime).toBe(0.8);
    expect(speechApi.synthesize).not.toHaveBeenCalled();
  });

  it("emits onRecordingComplete with audio path and whisper timings when dictation finishes", async () => {
    const stopTrack = vi.fn();
    Object.defineProperty(navigator, "mediaDevices", { configurable: true, value: { getUserMedia: vi.fn(async () => ({ getTracks: () => [{ stop: stopTrack }] })) } });
    Object.defineProperty(globalThis, "MediaRecorder", { configurable: true, value: FakeRecorder });
    const onRecordingComplete = vi.fn();

    render(
      <LocalSpeechProvider>
        <SpeechDictationButton
          sourceKind="chat"
          sourceId="chat-1"
          value=""
          onChange={vi.fn()}
          onRecordingComplete={onRecordingComplete}
        />
      </LocalSpeechProvider>,
    );

    fireEvent.click(await screen.findByRole("button", { name: "Dictate" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Stop dictation" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Stop dictation" }));

    await waitFor(() => {
      expect(onRecordingComplete).toHaveBeenCalledWith(
        expect.objectContaining({
          audioRelativePath: "recordings/chat/chat-1/voice.webm",
          words: expect.arrayContaining([
            expect.objectContaining({ value: "clearer", start: 0.2, end: 0.5 }),
          ]),
        }),
      );
    });
  });
});
