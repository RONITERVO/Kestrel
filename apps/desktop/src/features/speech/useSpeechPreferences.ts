import { useCallback, useEffect, useRef, useState } from "react";
import { getSpeechPreferences, saveResearchSpeechPreferences, saveVadSettings } from "../../platform/api";
import { speechPreferencesDefaults, type ResearchSpeechPreferences, type SpeechPreferences, type VadSettings } from "../../contracts/index";

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
  const updateResearchSpeechPreferences = useCallback((patch: Partial<ResearchSpeechPreferences>) => {
    const research = { ...draft.current.research, ...patch };
    save({ ...draft.current, research }, () => saveResearchSpeechPreferences(research));
  }, [save]);

  return { vadSettings: preferences.vad, researchSpeechPreferences: preferences.research,
    updateVadSettings, resetVadSettings, updateResearchSpeechPreferences, preferenceError };
}
