import { useCallback, useRef, useState } from "react";
import { alignLocalSpeech, cancelLocalSpeech, exportNarration, synthesizeLocalSpeech } from "../../platform/api";
import type { NarrationExportPassage, SpeechModel, VoiceProfile } from "../../contracts/index";
import type { SpeechPassage } from "../../shared/speech/text";
import { speechJobId, type SourceKind } from "./usePipelinedSpeechPlayer";

export type NarrationExportState =
  | { stage: "idle" }
  | { stage: "preparing"; done: number; total: number }
  | { stage: "saving" }
  | { stage: "saved"; files: string[] }
  | { stage: "error"; message: string };

export interface NarrationExportOptions {
  /** Check each passage for voice mistakes first, whatever the saved preference. */
  checkMistakes: boolean;
  /** Also save the words with their times and a word-by-word player page. */
  wordTimings: boolean;
  /** Whisper, to time the words of passages nobody has listened to yet. */
  alignmentModel?: SpeechModel;
}

/**
 * Exports a reply's narration as one audio file. Passages already listened to come from the
 * speech cache at once; the rest are generated first, through the mistake check when the producer
 * chose it, and timed by Whisper when word timings are wanted. Native code then joins them and
 * asks where to save.
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
    { checkMistakes, wordTimings, alignmentModel }: NarrationExportOptions,
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
        if (wordTimings && !clip.words.length && alignmentModel) {
          const alignJob = speechJobId("export-align");
          jobRef.current = alignJob;
          await alignLocalSpeech({
            jobId: alignJob,
            sourceKind,
            sourceId,
            passageId: passage.id,
            text: passage.text,
            relativePath: clip.relativePath,
            voiceModelId: voice.id,
            voiceProfileId: profile.id,
            alignmentModelId: alignmentModel.id,
          });
        }
        clips.push({ text: passage.text, relativePath: clip.relativePath });
      }
      if (stoppedRef.current) return setState({ stage: "idle" });
      setState({ stage: "saving" });
      const jobId = speechJobId("export-join");
      jobRef.current = jobId;
      const exported = await exportNarration({ jobId, title, passages: clips, wordTimings });
      setState(exported ? { stage: "saved", files: exported.files } : { stage: "idle" });
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
