import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ConversationAudio, ChatStreamEvent } from "../../contracts/index";
import { useConversationVoice } from "./useConversationVoice";
import { pcmBase64 } from "./conversationAudio";

const native = vi.hoisted(() => ({ snapshot: vi.fn(), prepare: vi.fn(), speak: vi.fn(), transcribe: vi.fn(), cancel: vi.fn(), listen: vi.fn(), save: vi.fn(), language: "en" }));
vi.mock("../../platform/api", () => ({ getConversationVoiceSnapshot: native.snapshot, prepareConversationVoice: native.prepare,
  speakConversation: native.speak, transcribeConversation: native.transcribe, cancelConversationVoice: native.cancel,
  onConversationAudio: native.listen }));
vi.mock("./LocalSpeechControls", () => ({ useSpeech: () => ({ conversationEnabled: true, preferencesReady: true,
  conversationLanguage: native.language, updateConversationLanguage: vi.fn(async () => {}),
  updateConversationEnabled: native.save, updateConversationVad: vi.fn(), conversationVad: { enabled: true, speechThresholdDb: -42, silenceTimeoutSec: 0.6, minSpeechDurationMs: 200 } }) }));
vi.mock("./usePipelinedSpeechPlayer", () => ({ claimPlayback: vi.fn(), clearPlayback: vi.fn(), speechJobId: (prefix: string) => `${prefix}-${crypto.randomUUID()}` }));

let audioListener: (audio: ConversationAudio) => void;
let micFrames: (event: MessageEvent<Float32Array>) => void;
let stopTrack: ReturnType<typeof vi.fn>;
const loadWorklet = vi.fn(async (_url: string) => {});
const sources: { stop: ReturnType<typeof vi.fn> }[] = [];
class AudioContextFake {
  state = "running";
  currentTime = 0;
  destination = {};
  audioWorklet = { addModule: loadWorklet };
  resume = vi.fn(async () => {});
  close = vi.fn(async () => { this.state = "closed"; });
  createMediaStreamSource = () => ({ connect: vi.fn(), disconnect: vi.fn() });
  createGain = () => ({ gain: { value: 0 }, connect: vi.fn() });
  createBuffer = (_channels: number, length: number, rate: number) => ({ duration: length / rate, copyToChannel: vi.fn() });
  createBufferSource = () => {
    const source = { stop: vi.fn(), start: vi.fn(), connect: vi.fn(), disconnect: vi.fn(), buffer: null, onended: null };
    sources.push(source); return source;
  };
}
class WorkletFake {
  port = { set onmessage(callback: (event: MessageEvent<Float32Array>) => void) { micFrames = callback; } };
  connect = vi.fn(); disconnect = vi.fn();
}
const token = (content: string, kind: ChatStreamEvent["kind"] = "token"): ChatStreamEvent => ({ requestId: "reply-1", sessionId: "chat-1", kind, content, data: null, at: "2026-11-10T10:00:00Z" });
const frame = (speech: boolean) => new MessageEvent("message", { data: Float32Array.from({ length: 320 }, (_, i) => speech ? Math.sin(i * 0.1) * 0.12 : 0) });

beforeEach(() => {
  vi.clearAllMocks(); sources.length = 0;
  native.language = "en";
  loadWorklet.mockReset().mockResolvedValue(undefined);
  stopTrack = vi.fn();
  vi.stubGlobal("AudioContext", AudioContextFake); vi.stubGlobal("AudioWorkletNode", WorkletFake);
  Object.defineProperty(navigator, "mediaDevices", { configurable: true, value: { getUserMedia: vi.fn(async () => ({ getTracks: () => [{ stop: stopTrack }] })) } });
  native.snapshot.mockResolvedValue({ installed: true }); native.prepare.mockResolvedValue({ running: true });
  native.cancel.mockResolvedValue(undefined); native.speak.mockResolvedValue(undefined);
  native.listen.mockImplementation(async (callback) => { audioListener = callback; return vi.fn(); });
  native.transcribe.mockResolvedValue({ text: "Continue", language: "en", words: [] });
});
afterEach(() => vi.unstubAllGlobals());

it("reports a concise capture failure, releases the microphone, and permits an explicit retry", async () => {
  loadWorklet.mockRejectedValueOnce(new Error(`AbortError: Failed to load worklet module script: data:text/javascript;base64,${"A".repeat(5000)}`));
  const onError = vi.fn();
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn: vi.fn(async () => {}), onError }));
  await waitFor(() => expect(onError).toHaveBeenCalledTimes(1));
  expect(onError.mock.calls[0][0]).toContain("Microphone capture could not start");
  expect(onError.mock.calls[0][0]).not.toContain("base64");
  expect(onError.mock.calls[0][0].length).toBeLessThan(200);
  expect(stopTrack).toHaveBeenCalled();
  expect(native.prepare).not.toHaveBeenCalled();
  expect(result.current.control.listening).toBe(false);
  act(() => result.current.control.toggle());
  await waitFor(() => expect(native.prepare).toHaveBeenCalledTimes(1));
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  unmount();
});

it("uses the selected language for both directions and never switches from detected speech", async () => {
  native.language = "de";
  native.transcribe.mockResolvedValue({ text: "Weiter", language: "fr", words: [] });
  const onTurn = vi.fn(async () => {});
  const { result, rerender, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn, onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.preparing).toBe(false));
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  act(() => { for (let i = 0; i < 20; i++) micFrames(frame(true)); for (let i = 0; i < 35; i++) micFrames(frame(false)); });
  await waitFor(() => expect(onTurn).toHaveBeenCalledWith("Weiter"));
  expect(native.transcribe.mock.calls[0][0].language).toBe("de");
  act(() => result.current.chatEvent(token("Weiter geht es. ")));
  await waitFor(() => expect(native.speak).toHaveBeenCalledTimes(1));
  expect(native.speak.mock.calls[0][0].language).toBe("de");
  native.language = "nl";
  rerender();
  await waitFor(() => expect(native.prepare).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(result.current.control.preparing).toBe(false));
  act(() => result.current.chatEvent({ ...token("We gaan verder. "), requestId: "reply-2" }));
  await waitFor(() => expect(native.speak).toHaveBeenCalledTimes(2));
  expect(native.speak.mock.calls[1][0].language).toBe("nl");
  unmount();
});

it("retains the first utterance while speech models warm, without asking for another click", async () => {
  let warmed!: () => void;
  native.prepare.mockImplementationOnce(() => new Promise<void>((resolve) => { warmed = resolve; }));
  const onTurn = vi.fn(async () => {});
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn, onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  act(() => { for (let i = 0; i < 20; i++) micFrames(frame(true)); for (let i = 0; i < 35; i++) micFrames(frame(false)); });
  expect(native.transcribe).not.toHaveBeenCalled();
  await act(async () => warmed());
  await waitFor(() => expect(onTurn).toHaveBeenCalledExactlyOnceWith("Continue"));
  unmount();
});

it("speaks a task clarification once and accepts consecutive spoken answers", async () => {
  const onTurn = vi.fn(async () => {});
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "task-1", onTurn, onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.preparing).toBe(false));
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  const delta = { runId: "task-1", responseId: "response-1", step: 1, kind: "token", text: "Which folder? " };
  act(() => { result.current.taskDelta(delta); result.current.taskDelta({ ...delta, kind: "done", text: "" }); });
  await waitFor(() => expect(native.speak).toHaveBeenCalledTimes(1));
  act(() => result.current.taskEvent({ runId: "task-1", step: 1, kind: "question", title: "Question", detail: "Which folder? ", at: "2026-11-10T10:00:00Z" }));
  expect(native.speak).toHaveBeenCalledTimes(1);
  native.transcribe.mockResolvedValueOnce({ text: "Use the approved folder", language: "en", words: [] });
  for (let i = 0; i < 2; i++) {
    act(() => { for (let n = 0; n < 20; n++) micFrames(frame(true)); for (let n = 0; n < 35; n++) micFrames(frame(false)); });
    await waitFor(() => expect(onTurn).toHaveBeenCalledTimes(i + 1));
  }
  expect(onTurn.mock.calls).toEqual([["Use the approved folder"], ["Continue"]]);
  expect(native.prepare).toHaveBeenCalledTimes(1);
  expect(native.transcribe.mock.calls.every(([request]) => request.language === "en")).toBe(true);
  unmount();
});

it("listens by default, submits only final speech, and remains ready for another turn", async () => {
  const onTurn = vi.fn(async () => {});
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn, onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  for (let turn = 0; turn < 2; turn++) {
    act(() => { for (let i = 0; i < 20; i++) micFrames(frame(true)); });
    expect(onTurn).toHaveBeenCalledTimes(turn);
    act(() => { for (let i = 0; i < 35; i++) micFrames(frame(false)); });
    await waitFor(() => expect(onTurn).toHaveBeenCalledTimes(turn + 1));
  }
  expect(native.prepare).toHaveBeenCalledTimes(1);
  expect(navigator.mediaDevices.getUserMedia).toHaveBeenCalledWith(expect.objectContaining({ audio: expect.objectContaining({ echoCancellation: true }) }));
  unmount(); expect(stopTrack).toHaveBeenCalled();
});

it("speaks visible deltas before completion and cancels old audio on interruption", async () => {
  let finishSpeech!: () => void;
  native.speak.mockImplementation(() => new Promise<void>((resolve) => { finishSpeech = resolve; }));
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn: vi.fn(async () => {}), onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  act(() => result.current.chatEvent(token("private reasoning", "reasoning")));
  expect(native.speak).not.toHaveBeenCalled();
  act(() => result.current.chatEvent(token("Hello there. ")));
  await waitFor(() => expect(native.speak).toHaveBeenCalledTimes(1));
  const job = native.speak.mock.calls[0][0];
  act(() => audioListener({ ...job, sequence: 0, sampleRate: 24000, pcmBase64: pcmBase64(new Float32Array(2400)) }));
  expect(sources).toHaveLength(1);
  act(() => { for (let i = 0; i < 20; i++) micFrames(frame(true)); });
  expect(sources[0].stop).toHaveBeenCalled();
  expect(native.cancel).toHaveBeenCalledWith(job.jobId);
  act(() => {
    audioListener({ ...job, sequence: 1, sampleRate: 24000, pcmBase64: pcmBase64(new Float32Array(2400)) });
    result.current.chatEvent(token("Obsolete answer. "));
    result.current.chatEvent(token("", "done"));
  });
  await act(async () => finishSpeech());
  expect(sources).toHaveLength(1); expect(native.speak).toHaveBeenCalledTimes(1);
  unmount();
});

it("does not submit a late transcription into a different conversation", async () => {
  let finish!: (value: unknown) => void;
  native.transcribe.mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  const onTurn = vi.fn(async () => {});
  const { result, rerender, unmount } = renderHook(({ scope }) => useConversationVoice({ active: true, scope, onTurn, onError: vi.fn() }), { initialProps: { scope: "chat-1" } });
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  act(() => { for (let i = 0; i < 20; i++) micFrames(frame(true)); for (let i = 0; i < 35; i++) micFrames(frame(false)); });
  await waitFor(() => expect(native.transcribe).toHaveBeenCalled());
  rerender({ scope: "chat-2" });
  await act(async () => finish({ text: "Old direction", language: "en" }));
  expect(onTurn).not.toHaveBeenCalled(); expect(native.cancel).toHaveBeenCalled();
  unmount();
});
