import type { LegacySpeechPreferences } from "../contracts/index";

/** Read-only migration bridge. Native code validates these old values and imports them once.
 * Keep the original keys intact so an older installation can still read its preferences. */
export function readLegacySpeechPreferences(): LegacySpeechPreferences {
  const read = (key: string): string | null => {
    try { return window.localStorage.getItem(key); } catch { return null; }
  };
  return {
    vadJson: read("kestrel_speech_vad_settings"),
    modelId: read("kestrel.researchSpeech.comfyModel"),
    voiceProfileId: read("kestrel.researchSpeech.voiceProfile"),
    rate: read("kestrel.researchSpeech.rate"),
    scope: read("kestrel.researchSpeech.scope"),
  };
}
