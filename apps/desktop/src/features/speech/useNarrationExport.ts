import { useCallback, useRef, useState } from "react";
import { cancelLocalSpeech, exportNarration, synthesizeLocalSpeech } from "../../platform/api";
import type { NarrationExportPassage, SpeechModel, VoiceProfile } from "../../contracts/index";
import type { SpeechPassage } from "../../shared/speech/text";
import { speechJobId, type SourceKind } from "./usePipelinedSpeechPlayer";

export type NarrationExportState =
  | { stage: "idle" }
  | { stage: "preparing"; done: number; total: number }
  | { stage: "saving" }
  | { stage: "saved"; path: string }
  | { stage: "error"; message: string };

/**
 * Exports a reply's narration as one audio file. Passages already listened to come from the
 * speech cache at once; the rest are generated first, through the mistake check when the producer
 * chose it. Native code then joins them and asks where to save.
 */
export function useNarrationExport({ sourceKind, sourceId, passages, title }: {
  sourceKind: SourceKind;
  sourceId: string;
  passages: SpeechPassage[];
  title: string;
}) {
  const [state, setState] = useState<NarrationExportState>({ stage: "idle" });
  const jobRef = useRef<string | null>(null);
  const stoppedRef = useRef(false);

  const start = useCallback(async (
    readyVoice: () => Promise<{ voice: SpeechModel; profile: VoiceProfile }>,
    checkMistakes: boolean,
  ) => {
    stoppedRef.current = false;
    const clips: NarrationExportPassage[] = [];
    try {
      setState({ stage: "preparing", done: 0, total: passages.length });
      const { voice, profile } = await readyVoice();
      for (let index = 0; index < passages.length; index++) {
        if (stoppedRef.current) return setState({ stage: "idle" });
        setState({ stage: "preparing", done: index, total: passages.length });
        const jobId = speechJobId("export-tts");
        jobRef.current = jobId;
        const passage = passages[index];
        const clip = await synthesizeLocalSpeech({
          jobId,
          sourceKind,
          sourceId,
          passageId: passage.id,
          text: passage.text,
          modelId: voice.id,
          voiceProfileId: profile.id,
          checkMistakes,
        });
        clips.push({ text: passage.text, relativePath: clip.relativePath });
      }
      if (stoppedRef.current) return setState({ stage: "idle" });
      setState({ stage: "saving" });
      const jobId = speechJobId("export-join");
      jobRef.current = jobId;
      const path = await exportNarration({ jobId, title, passages: clips });
      setState(path ? { stage: "saved", path } : { stage: "idle" });
    } catch (error) {
      setState(stoppedRef.current ? { stage: "idle" } : { stage: "error", message: String(error) });
    } finally {
      jobRef.current = null;
    }
  }, [passages, sourceId, sourceKind, title]);

  const stop = useCallback(() => {
    stoppedRef.current = true;
    if (jobRef.current) void cancelLocalSpeech(jobRef.current);
  }, []);

  const reset = useCallback(() => setState({ stage: "idle" }), []);

  return { state, start, stop, reset };
}
