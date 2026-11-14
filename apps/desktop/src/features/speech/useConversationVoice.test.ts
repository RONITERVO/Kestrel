import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import type { ConversationCaptureEvent, ChatStreamEvent } from "../../contracts/index";
import { useConversationVoice } from "./useConversationVoice";

const native = vi.hoisted(() => ({ snapshot: vi.fn(), prepare: vi.fn(), speak: vi.fn(), capture: vi.fn(), cancel: vi.fn(), listen: vi.fn(), save: vi.fn(), language: "en" }));
vi.mock("../../platform/api", () => ({ getConversationVoiceSnapshot: native.snapshot, prepareConversationVoice: native.prepare,
  speakConversation: native.speak, startConversationCapture: native.capture, cancelConversationVoice: native.cancel,
  onConversationCapture: native.listen }));
vi.mock("./LocalSpeechControls", () => ({ useSpeech: () => ({ conversationEnabled: true, preferencesReady: true,
  conversationLanguage: native.language, updateConversationLanguage: vi.fn(async () => {}),
  updateConversationEnabled: native.save, updateConversationVad: vi.fn(), conversationVad: { enabled: true, speechThresholdDb: -42, silenceTimeoutSec: 0.6, minSpeechDurationMs: 200 } }) }));
vi.mock("./usePipelinedSpeechPlayer", () => ({ claimPlayback: vi.fn(), clearPlayback: vi.fn(), observePlayback: (callback: (owner: (() => void) | null) => void) => { playbackChanged = callback; return vi.fn(); }, speechJobId: (prefix: string) => `${prefix}-${crypto.randomUUID()}` }));

let listener: (event: ConversationCaptureEvent) => void;
let playbackChanged: (owner: (() => void) | null) => void;
const captures = new Map<string, () => void>();
let session = "";
const token = (content: string, kind: ChatStreamEvent["kind"] = "token"): ChatStreamEvent => ({ requestId: "reply-1", sessionId: "chat-1", kind, content, data: null, at: "2026-11-14T10:00:00Z" });
const receive = (kind: string, text = "", sessionId = session) => listener({ sessionId, kind, text, detail: "" });
beforeEach(() => {
  vi.clearAllMocks(); captures.clear(); native.language = "en";
  native.snapshot.mockResolvedValue({ installed: true }); native.prepare.mockResolvedValue({ running: true });
  native.speak.mockResolvedValue(undefined);
  native.cancel.mockImplementation(async (id) => { captures.get(id)?.(); captures.delete(id); });
  native.listen.mockImplementation(async (callback) => { listener = callback; return vi.fn(); });
  native.capture.mockImplementation((id) => { session = id; receive("listening"); return new Promise<void>((resolve) => captures.set(id, resolve)); });
});

it("reports native capture failure and permits an explicit retry", async () => {
  native.capture.mockRejectedValueOnce(new Error("Microphone access denied. Allow desktop microphone access in Windows settings."));
  const onError = vi.fn();
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn: vi.fn(async () => {}), onError }));
  await waitFor(() => expect(onError).toHaveBeenCalledTimes(1));
  expect(onError.mock.calls[0][0]).toContain("Microphone access denied");
  expect(result.current.control.listening).toBe(false);
  act(() => result.current.control.toggle());
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  unmount(); expect(native.cancel).toHaveBeenCalledWith(session);
});

it("uses the selected language and cancels the old session on language changes", async () => {
  native.language = "de";
  const onTurn = vi.fn(async () => {});
  const { result, rerender, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn, onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.preparing).toBe(false));
  act(() => receive("transcript", "Weiter")); await waitFor(() => expect(onTurn).toHaveBeenCalledWith("Weiter"));
  act(() => result.current.chatEvent(token("Weiter geht es. "))); await waitFor(() => expect(native.speak).toHaveBeenCalledTimes(1));
  expect(native.speak.mock.calls[0][0].language).toBe("de");
  const old = session; native.language = "nl"; rerender();
  await waitFor(() => expect(native.prepare).toHaveBeenCalledTimes(2));
  expect(native.cancel).toHaveBeenCalledWith(old);
  act(() => result.current.chatEvent({ ...token("We gaan verder. "), requestId: "reply-2" }));
  await waitFor(() => expect(native.speak).toHaveBeenCalledTimes(2));
  expect(native.speak.mock.calls[1][0]).toMatchObject({ language: "nl", audioSessionId: session }); unmount();
});

it("retains the first final utterance while the resident models warm", async () => {
  let warmed!: () => void;
  native.prepare.mockImplementationOnce(() => new Promise<void>((resolve) => { warmed = resolve; }));
  const onTurn = vi.fn(async () => {});
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn, onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  act(() => receive("transcript", "Continue")); expect(onTurn).not.toHaveBeenCalled();
  await act(async () => warmed()); await waitFor(() => expect(onTurn).toHaveBeenCalledExactlyOnceWith("Continue")); unmount();
});

it("speaks a task clarification once and accepts consecutive spoken answers", async () => {
  const onTurn = vi.fn(async () => {});
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "task-1", onTurn, onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.preparing).toBe(false));
  const delta = { runId: "task-1", responseId: "response-1", step: 1, kind: "token", text: "Which folder? " };
  act(() => { result.current.taskDelta(delta); result.current.taskDelta({ ...delta, kind: "done", text: "" }); });
  await waitFor(() => expect(native.speak).toHaveBeenCalledTimes(1));
  act(() => result.current.taskEvent({ runId: "task-1", step: 1, kind: "question", title: "Question", detail: "Which folder? ", at: "2026-11-14T10:00:00Z" }));
  expect(native.speak).toHaveBeenCalledTimes(1);
  for (const text of ["Use the approved folder", "Continue"]) { act(() => receive("transcript", text)); await waitFor(() => expect(onTurn).toHaveBeenCalledWith(text)); }
  expect(onTurn.mock.calls).toEqual([["Use the approved folder"], ["Continue"]]); expect(native.prepare).toHaveBeenCalledTimes(1); unmount();
});

it("listens by default and submits only final native transcriptions", async () => {
  const onTurn = vi.fn(async () => {});
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn, onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  for (let turn = 0; turn < 2; turn++) {
    act(() => receive("speech_start")); expect(onTurn).toHaveBeenCalledTimes(turn);
    act(() => receive("transcript", "Continue")); await waitFor(() => expect(onTurn).toHaveBeenCalledTimes(turn + 1));
  }
  expect(native.prepare).toHaveBeenCalledTimes(1); unmount(); expect(native.cancel).toHaveBeenCalledWith(session);
});

it("speaks visible deltas before completion and cancels old audio on interruption", async () => {
  let finishSpeech!: () => void;
  native.speak.mockImplementation(() => new Promise<void>((resolve) => { finishSpeech = resolve; }));
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn: vi.fn(async () => {}), onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  act(() => result.current.chatEvent(token("private reasoning", "reasoning"))); expect(native.speak).not.toHaveBeenCalled();
  act(() => result.current.chatEvent(token("Hello there. "))); await waitFor(() => expect(native.speak).toHaveBeenCalledTimes(1));
  const job = native.speak.mock.calls[0][0]; expect(job.audioSessionId).toBe(session);
  act(() => receive("speech_start")); expect(native.cancel).toHaveBeenCalledWith(job.jobId);
  act(() => { result.current.chatEvent(token("Obsolete answer. ")); result.current.chatEvent(token("", "done")); });
  await act(async () => finishSpeech()); expect(native.speak).toHaveBeenCalledTimes(1); unmount();
});

it("does not submit late transcription into a different conversation", async () => {
  const onTurn = vi.fn(async () => {});
  const { result, rerender, unmount } = renderHook(({ scope }) => useConversationVoice({ active: true, scope, onTurn, onError: vi.fn() }), { initialProps: { scope: "chat-1" } });
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  const old = session; rerender({ scope: "chat-2" }); await waitFor(() => expect(native.capture).toHaveBeenCalledTimes(2));
  act(() => receive("transcript", "Old direction", old)); expect(onTurn).not.toHaveBeenCalled(); expect(native.cancel).toHaveBeenCalledWith(old); unmount();
});

it("discards final speech after mute immediately", async () => {
  const onTurn = vi.fn(async () => {});
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn, onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  act(() => result.current.control.mute()); act(() => receive("transcript", "Do not send"));
  expect(onTurn).not.toHaveBeenCalled(); expect(native.save).toHaveBeenCalledWith(false); unmount();
});

it("pauses native capture during manual Listen and resumes without changing preferences", async () => {
  const onTurn = vi.fn(async () => {});
  const { result, unmount } = renderHook(() => useConversationVoice({ active: true, scope: "chat-1", onTurn, onError: vi.fn() }));
  await waitFor(() => expect(result.current.control.listening).toBe(true));
  const old = session;
  act(() => playbackChanged(() => {}));
  await waitFor(() => expect(native.cancel).toHaveBeenCalledWith(old));
  act(() => receive("transcript", "Manual playback echo", old));
  expect(onTurn).not.toHaveBeenCalled();
  act(() => playbackChanged(null));
  await waitFor(() => expect(native.capture).toHaveBeenCalledTimes(2));
  expect(native.save).not.toHaveBeenCalled(); unmount();
});
