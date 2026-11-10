import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { speechPreferencesDefaults, type SpeechPreferences } from "../../contracts/index";
import { useSpeechPreferences } from "./useSpeechPreferences";

const native = vi.hoisted(() => ({ get: vi.fn(), vad: vi.fn(), research: vi.fn(), language: vi.fn() }));
vi.mock("../../platform/api", () => ({ getSpeechPreferences: native.get, saveVadSettings: native.vad, saveResearchSpeechPreferences: native.research, saveConversationLanguage: native.language }));

beforeEach(() => {
  native.get.mockReset().mockResolvedValue({ ...speechPreferencesDefaults, research: { ...speechPreferencesDefaults.research, rate: 1.2 } });
  native.vad.mockReset();
  native.research.mockReset();
  native.language.mockReset();
});

it("serializes edits and ignores an older native reply while a newer draft is pending", async () => {
  let finishFirst!: (value: SpeechPreferences) => void;
  native.vad.mockImplementationOnce(() => new Promise<SpeechPreferences>((resolve) => { finishFirst = resolve; }));
  native.vad.mockImplementationOnce(async (vad) => ({ ...speechPreferencesDefaults, vad, research: { ...speechPreferencesDefaults.research, rate: 1.2 } }));
  const { result } = renderHook(() => useSpeechPreferences());
  await waitFor(() => expect(result.current.researchSpeechPreferences.rate).toBe(1.2));
  act(() => result.current.updateVadSettings({ silenceTimeoutSec: 4 }));
  await waitFor(() => expect(native.vad).toHaveBeenCalledTimes(1));
  act(() => result.current.updateVadSettings({ speechThresholdDb: -50 }));
  expect(result.current.vadSettings.silenceTimeoutSec).toBe(4);
  expect(result.current.vadSettings.speechThresholdDb).toBe(-50);
  await act(async () => finishFirst({ ...speechPreferencesDefaults, vad: { ...speechPreferencesDefaults.vad, silenceTimeoutSec: 4 } }));
  await waitFor(() => expect(native.vad).toHaveBeenCalledTimes(2));
  expect(native.vad.mock.calls[1][0]).toMatchObject({ silenceTimeoutSec: 4, speechThresholdDb: -50 });
  expect(result.current.vadSettings.speechThresholdDb).toBe(-50);
  expect(result.current.researchSpeechPreferences.rate).toBe(1.2);
});

it("shows failed persistence instead of claiming the preference was saved", async () => {
  native.research.mockRejectedValue(new Error("Library is read-only"));
  const { result } = renderHook(() => useSpeechPreferences());
  await waitFor(() => expect(result.current.researchSpeechPreferences.rate).toBe(1.2));
  act(() => result.current.updateResearchSpeechPreferences({ rate: 1.4 }));
  await waitFor(() => expect(result.current.preferenceError).toContain("not saved"));
  expect(result.current.preferenceError).toContain("Library is read-only");
});

it("waits for the native language switch before publishing it to audio consumers", async () => {
  let finish!: (value: SpeechPreferences) => void;
  native.language.mockImplementationOnce(() => new Promise<SpeechPreferences>((resolve) => { finish = resolve; }));
  const { result } = renderHook(() => useSpeechPreferences());
  await waitFor(() => expect(result.current.preferencesReady).toBe(true));
  act(() => { void result.current.updateConversationLanguage("fr"); });
  await waitFor(() => expect(native.language).toHaveBeenCalledWith("fr"));
  expect(result.current.conversationPreferences.language).toBe("en");
  await act(async () => finish({ ...speechPreferencesDefaults, conversation: { ...speechPreferencesDefaults.conversation, language: "fr" } }));
  expect(result.current.conversationPreferences.language).toBe("fr");
});

it("recovers the saved language when warming the replacement model fails", async () => {
  native.language.mockRejectedValueOnce(new Error("Repair voice conversation in Setup"));
  const { result } = renderHook(() => useSpeechPreferences());
  await waitFor(() => expect(result.current.preferencesReady).toBe(true));
  native.get.mockResolvedValue({ ...speechPreferencesDefaults, conversation: { ...speechPreferencesDefaults.conversation, language: "de" } });
  await act(async () => result.current.updateConversationLanguage("de"));
  expect(result.current.conversationPreferences.language).toBe("de");
  expect(result.current.preferenceError).toContain("Repair voice conversation");
});
