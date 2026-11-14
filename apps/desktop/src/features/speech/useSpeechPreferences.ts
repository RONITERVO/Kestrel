import { useCallback, useEffect, useRef, useState } from "react";
import { getSpeechPreferences, saveNarrationSpeechPreferences, saveResearchSpeechPreferences, saveVadSettings, saveConversationVoiceEnabled, saveConversationVad, saveConversationLanguage } from "../../platform/api";
import { speechPreferencesDefaults, type ConversationLanguage, type NarrationPreferences, type ResearchSpeechPreferences, type SpeechPreferences, type VadSettings } from "../../contracts/index";

/** Editable view of native preferences. Native replies become the accepted saved state. */
export function useSpeechPreferences() {
  const [preferences, setPreferences] = useState<SpeechPreferences>(speechPreferencesDefaults);
  const [preferenceError, setPreferenceError] = useState("");
  const draft = useRef(preferences);
  const acceptedLanguage = useRef(preferences.conversation.language);
  const ready = useRef(false);
  const revision = useRef(0);
  const pending = useRef(Promise.resolve());

  useEffect(() => {
    let active = true;
    void getSpeechPreferences().then((saved) => {
      if (!active) return;
      draft.current = saved;
      acceptedLanguage.current = saved.conversation.language;
      ready.current = true;
      setPreferences(saved);
    }).catch((error: unknown) => { if (active) setPreferenceError(String(error)); });
    return () => { active = false; };
  }, []);

  const save = useCallback((next: SpeechPreferences, submit: () => Promise<SpeechPreferences>) => {
    if (!ready.current) {
      setPreferenceError("Speech preferences have not loaded. Reopen the application before changing saved preferences.");
      return Promise.resolve();
    }
    const currentRevision = ++revision.current;
    draft.current = next;
    // Do not start a new language until native storage and the resident worker accept it.
    setPreferences({ ...next, conversation: { ...next.conversation, language: acceptedLanguage.current } });
    pending.current = pending.current.then(submit).then((saved) => {
      acceptedLanguage.current = saved.conversation.language;
      if (revision.current !== currentRevision) return;
      draft.current = saved;
      setPreferences(saved);
      setPreferenceError("");
    }).catch(async (error: unknown) => {
      setPreferenceError(`Speech preferences were not saved: ${String(error)}`);
      // A worker preparation error can follow a successful durable language save.
      // Reload the native truth so the next microphone retry uses that selection.
      try {
        const saved = await getSpeechPreferences();
        if (revision.current !== currentRevision) return;
        acceptedLanguage.current = saved.conversation.language;
        draft.current = saved;
        setPreferences(saved);
      } catch { /* retain the actionable persistence error */ }
    });
    return pending.current;
  }, []);

  const updateVadSettings = useCallback((updater: Partial<VadSettings> | ((previous: VadSettings) => VadSettings)) => {
    const vad = typeof updater === "function" ? updater(draft.current.vad) : { ...draft.current.vad, ...updater };
    save({ ...draft.current, vad }, () => saveVadSettings(vad));
  }, [save]);
  const resetVadSettings = useCallback(() => updateVadSettings(speechPreferencesDefaults.vad), [updateVadSettings]);
  const updateConversationEnabled = useCallback((enabled: boolean) => {
    save({ ...draft.current, conversation: { ...draft.current.conversation, enabled } }, () => saveConversationVoiceEnabled(enabled));
  }, [save]);
  const updateConversationLanguage = useCallback((language: ConversationLanguage) => {
    return save({ ...draft.current, conversation: { ...draft.current.conversation, language } }, () => saveConversationLanguage(language));
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
    updateConversationEnabled, updateConversationLanguage, updateConversationVad, updateVadSettings, resetVadSettings, updateResearchSpeechPreferences, updateNarrationPreferences, preferenceError };
}
