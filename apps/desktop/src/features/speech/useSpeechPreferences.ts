import { useCallback, useEffect, useRef, useState } from "react";
import { getSpeechPreferences, saveNarrationSpeechPreferences, saveResearchSpeechPreferences, saveVadSettings, saveConversationVoiceEnabled, saveConversationVad } from "../../platform/api";
import { speechPreferencesDefaults, type NarrationPreferences, type ResearchSpeechPreferences, type SpeechPreferences, type VadSettings } from "../../contracts/index";

/** Editable view of native preferences. Native replies become the accepted saved state. */
export function useSpeechPreferences() {
  const [preferences, setPreferences] = useState<SpeechPreferences>(speechPreferencesDefaults);
  const [preferenceError, setPreferenceError] = useState("");
  const draft = useRef(preferences);
  const ready = useRef(false);
  const revision = useRef(0);
  const pending = useRef(Promise.resolve());

  useEffect(() => {
    let active = true;
    void getSpeechPreferences().then((saved) => {
      if (!active) return;
      draft.current = saved;
      ready.current = true;
      setPreferences(saved);
    }).catch((error: unknown) => { if (active) setPreferenceError(String(error)); });
    return () => { active = false; };
  }, []);

  const save = useCallback((next: SpeechPreferences, submit: () => Promise<SpeechPreferences>) => {
    if (!ready.current) {
      setPreferenceError("Speech preferences have not loaded. Reopen the application before changing saved preferences.");
      return;
    }
    const currentRevision = ++revision.current;
    draft.current = next;
    setPreferences(next);
    pending.current = pending.current.then(submit).then((saved) => {
      if (revision.current !== currentRevision) return;
      draft.current = saved;
      setPreferences(saved);
      setPreferenceError("");
    }).catch((error: unknown) => {
      setPreferenceError(`Speech preferences were not saved: ${String(error)}`);
    });
  }, []);

  const updateVadSettings = useCallback((updater: Partial<VadSettings> | ((previous: VadSettings) => VadSettings)) => {
    const vad = typeof updater === "function" ? updater(draft.current.vad) : { ...draft.current.vad, ...updater };
    save({ ...draft.current, vad }, () => saveVadSettings(vad));
  }, [save]);
  const resetVadSettings = useCallback(() => updateVadSettings(speechPreferencesDefaults.vad), [updateVadSettings]);
  const updateConversationEnabled = useCallback((enabled: boolean) => {
    save({ ...draft.current, conversation: { ...draft.current.conversation, enabled } }, () => saveConversationVoiceEnabled(enabled));
  }, [save]);
  const updateResearchSpeechPreferences = useCallback((patch: Partial<ResearchSpeechPreferences>) => {
    const research = { ...draft.current.research, ...patch };
    save({ ...draft.current, research }, () => saveResearchSpeechPreferences(research));
  }, [save]);
  const updateConversationVad = useCallback((patch: Partial<VadSettings>) => {
    const vad = { ...draft.current.conversation.vad, ...patch };
    save({ ...draft.current, conversation: { ...draft.current.conversation, vad } }, () => saveConversationVad(vad));
  }, [save]);

  const updateNarrationPreferences = useCallback((patch: Partial<NarrationPreferences>) => {
    const narration = { ...draft.current.narration, ...patch };
    save({ ...draft.current, narration }, () => saveNarrationSpeechPreferences(narration));
  }, [save]);

  return { vadSettings: preferences.vad, researchSpeechPreferences: preferences.research,
    narrationPreferences: preferences.narration,
    conversationPreferences: preferences.conversation, preferencesReady: ready.current,
    updateConversationEnabled, updateConversationVad, updateVadSettings, resetVadSettings, updateResearchSpeechPreferences, updateNarrationPreferences, preferenceError };
}
