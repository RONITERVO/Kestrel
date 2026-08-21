import {
  Check,
  LoaderCircle,
  Mic,
  Pause,
  Play,
  RotateCcw,
  Sliders,
  Square,
  Volume2,
  X,
} from "lucide-react";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  getLocalSpeechSnapshot,
  prepareLocalSpeech,
  releaseLocalSpeechMemory,
  transcribeLocalSpeech,
} from "./api";
import {
  buildSpeechPassages,
  MAX_PASSAGE_CHARS,
  splitForSpeech,
} from "./researchSpeechContent";
import type { LocalSpeechSnapshot, SpeechTiming } from "./types";
import { type SpeechProgressState } from "./spokenHighlight";
import {
  claimPlayback,
  clearPlayback,
  usePipelinedSpeechPlayer,
  type SourceKind,
} from "./usePipelinedSpeechPlayer";
import {
  DEFAULT_VAD_SETTINGS,
  loadVadSettings,
  saveVadSettings,
  VoiceActivityDetector,
  type VadSettings,
} from "./voiceActivityDetection";

export { claimPlayback, clearPlayback };
export type { SpeechProgressState };

type SpeechContextValue = {
  snapshot: LocalSpeechSnapshot | null;
  refresh: () => Promise<LocalSpeechSnapshot>;
  prepare: () => Promise<LocalSpeechSnapshot>;
  vadSettings: VadSettings;
  updateVadSettings: (updater: Partial<VadSettings> | ((prev: VadSettings) => VadSettings)) => void;
  resetVadSettings: () => void;
};

const SpeechContext = createContext<SpeechContextValue | null>(null);

export function LocalSpeechProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<LocalSpeechSnapshot | null>(null);
  const [vadSettings, setVadSettingsState] = useState<VadSettings>(() => loadVadSettings());

  const refresh = useCallback(async () => {
    const next = await getLocalSpeechSnapshot();
    setSnapshot(next);
    return next;
  }, []);

  const prepare = useCallback(async () => {
    const next = await prepareLocalSpeech();
    setSnapshot(next);
    return next;
  }, []);

  const updateVadSettings = useCallback(
    (updater: Partial<VadSettings> | ((prev: VadSettings) => VadSettings)) => {
      setVadSettingsState((prev) => {
        const next = typeof updater === "function" ? updater(prev) : { ...prev, ...updater };
        saveVadSettings(next);
        return next;
      });
    },
    [],
  );

  const resetVadSettings = useCallback(() => {
    setVadSettingsState(DEFAULT_VAD_SETTINGS);
    saveVadSettings(DEFAULT_VAD_SETTINGS);
  }, []);

  useEffect(() => {
    void refresh().catch(() => undefined);
  }, [refresh]);

  const value = useMemo(
    () => ({
      snapshot,
      refresh,
      prepare,
      vadSettings,
      updateVadSettings,
      resetVadSettings,
    }),
    [prepare, refresh, resetVadSettings, snapshot, updateVadSettings, vadSettings],
  );

  return <SpeechContext.Provider value={value}>{children}</SpeechContext.Provider>;
}

export function useSpeech() {
  const value = useContext(SpeechContext);
  return (
    value ?? {
      snapshot: null,
      refresh: getLocalSpeechSnapshot,
      prepare: prepareLocalSpeech,
      vadSettings: DEFAULT_VAD_SETTINGS,
      updateVadSettings: () => undefined,
      resetVadSettings: () => undefined,
    }
  );
}

export function splitSpeechText(text: string, maximum = MAX_PASSAGE_CHARS): string[] {
  return splitForSpeech(text, maximum, true);
}

function id(prefix: string): string {
  const value =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return `${prefix}-${value}`;
}

function wordTimings(text: string, duration: number): SpeechTiming[] {
  const words = text.match(/\S+/g) ?? [];
  const weights = words.map((word) =>
    Math.max(1, word.replace(/[^\p{L}\p{N}]/gu, "").length),
  );
  const total = weights.reduce((sum, value) => sum + value, 0) || 1;
  let cursor = 0;
  return words.map((value, index) => {
    const start = (duration * cursor) / total;
    cursor += weights[index];
    return { value, start, end: (duration * cursor) / total };
  });
}

function activeCaption(text: string, seconds: number, duration: number, exact: SpeechTiming[]) {
  const words = exact.length ? exact : wordTimings(text, duration || Math.max(1, text.length / 15));
  const found = words.findIndex((word) => seconds >= word.start && seconds < word.end);
  const index = found >= 0 ? found : Math.max(0, words.length - 1);
  let start = index;
  let end = index;
  while (start > 0 && !/[.!?]["'’”)]*$/.test(words[start - 1].value)) start -= 1;
  while (end + 1 < words.length && !/[.!?]["'’”)]*$/.test(words[end].value)) end += 1;
  return { words: words.slice(start, end + 1), active: index - start };
}

export function SpeechLiveCaption({
  text,
  seconds,
  duration,
  timings = [],
  onSeek,
}: {
  text: string;
  seconds: number;
  duration: number;
  timings?: SpeechTiming[];
  onSeek?: (seconds: number) => void;
}) {
  const caption = activeCaption(text, seconds, duration, timings);
  if (!caption.words.length) return null;
  return (
    <div className="speech-live-caption" aria-live="off">
      {caption.words.map((word, index) =>
        onSeek ? (
          <button
            type="button"
            className={index === caption.active ? "active" : undefined}
            aria-current={index === caption.active ? "true" : undefined}
            aria-label={`Start from ${word.value}`}
            title={`Start from ${word.value}`}
            key={`${word.start}-${index}`}
            onClick={() => onSeek(word.start)}
          >
            {word.value}
          </button>
        ) : index === caption.active ? (
          <mark key={`${word.start}-${index}`}>{word.value}</mark>
        ) : (
          <span key={`${word.start}-${index}`}>{word.value}</span>
        ),
      )}
    </div>
  );
}

export function SpeechPlaybackButton({
  sourceKind,
  sourceId,
  passageId,
  text,
  label = "Listen",
  recording,
  onActiveChange,
  onSpeechProgress,
}: {
  sourceKind: SourceKind;
  sourceId: string;
  passageId: string;
  text: string;
  label?: string;
  recording?: {
    audioRelativePath: string;
    words: SpeechTiming[];
  };
  onActiveChange?: (active: boolean) => void;
  onSpeechProgress?: (progress: SpeechProgressState | null) => void;
}) {
  const { snapshot, prepare } = useSpeech();
  const passages = useMemo(
    () => buildSpeechPassages(text, { stripCodeBlocks: true, basePassageId: passageId, label }),
    [label, passageId, text],
  );
  const voice = snapshot?.voices[0] ?? null;
  const alignmentModel = snapshot?.transcriptionAvailable ? snapshot.transcribers[0] : undefined;

  const player = usePipelinedSpeechPlayer({
    sourceKind,
    sourceId,
    passages,
    selectedVoiceModel: voice,
    alignmentModel,
    playbackRate: 1,
    initialDetail: label,
    recording,
  });

  const state = player.status;
  const currentPassage = player.currentPassage;

  useEffect(() => {
    const active = state === "playing" || state === "paused";
    onActiveChange?.(active);
    if (active) {
      onSpeechProgress?.({
        active: true,
        sourceKind,
        sourceId,
        passageId: currentPassage?.id ?? passageId,
        text: currentPassage?.text ?? text,
        seconds: player.speechSeconds,
        duration: player.speechDuration,
        timings: player.speechTimings,
      });
    } else {
      onSpeechProgress?.(null);
    }
  }, [
    currentPassage?.id,
    currentPassage?.text,
    onActiveChange,
    onSpeechProgress,
    passageId,
    player.speechDuration,
    player.speechSeconds,
    player.speechTimings,
    sourceId,
    sourceKind,
    state,
    text,
  ]);

  const toggle = () => {
    if (player.status === "playing" || (player.status === "paused" && player.audioRef.current?.src)) {
      player.togglePlayback();
      return;
    }
    const start = async () => {
      if (recording) {
        await player.startAt(player.status === "complete" ? 0 : player.currentIndex, null);
        return;
      }
      const ready = snapshot?.narrationAvailable ? snapshot : await prepare();
      const readyVoice = ready.voices[0];
      if (!ready.narrationAvailable || !readyVoice) throw new Error(ready.detail);
      await player.startAt(player.status === "complete" ? 0 : player.currentIndex, readyVoice);
    };
    void start().catch((error) => {
      player.setStatus("error");
      player.setDetail(String(error));
    });
  };

  return (
    <div className={`inline-speech ${state === "complete" ? "idle" : state}`}>
      <audio ref={player.audioRef} {...player.audioProps} />
      <button
        type="button"
        className="inline-speech-button"
        title={player.detail || label}
        aria-label={state === "playing" ? `Pause ${label.toLowerCase()}` : label}
        disabled={!passages.length || state === "preparing"}
        onClick={toggle}
      >
        {state === "preparing" ? (
          <LoaderCircle className="spin" />
        ) : state === "playing" ? (
          <Pause />
        ) : (
          <Volume2 />
        )}
        <span>
          {state === "preparing" ? "Preparing…" : state === "paused" ? "Resume" : label}
        </span>
      </button>
      {state !== "ready" && state !== "complete" && (
        <button
          type="button"
          className="inline-speech-stop"
          title="Stop speaking"
          aria-label="Stop speaking"
          onClick={player.stopPlayback}
        >
          <Square />
        </button>
      )}
      {currentPassage && ["playing", "paused"].includes(state) && (
        <input
          className="speech-inline-seek"
          type="range"
          aria-label="Speech position"
          min={0}
          max={Math.max(player.speechDuration, 0.01)}
          step={0.01}
          value={Math.min(player.speechSeconds, Math.max(player.speechDuration, 0.01))}
          onChange={(event) => player.seekSpeech(event.currentTarget.valueAsNumber)}
        />
      )}
      {state === "error" && (
        <small className="speech-inline-error">{player.error ?? player.detail}</small>
      )}
    </div>
  );
}

function recorderMimeType(): string {
  const options = ["audio/webm;codecs=opus", "audio/webm", "audio/mp4"];
  return options.find((value) => MediaRecorder.isTypeSupported(value)) ?? "";
}

export const LIVE_TRANSCRIPTION_CHECKPOINTS_SECONDS = [4, 8, 16, 24, 32, 48, 60, 92] as const;

export function advanceLiveTranscriptionCheckpoint(
  elapsedSeconds: number,
  currentIndex: number,
): number {
  let next = currentIndex;
  while (
    next < LIVE_TRANSCRIPTION_CHECKPOINTS_SECONDS.length &&
    elapsedSeconds >= LIVE_TRANSCRIPTION_CHECKPOINTS_SECONDS[next]
  ) {
    next += 1;
  }
  return next;
}

function blobBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onloadend = () => {
      const dataUrl = reader.result as string;
      const base64 = dataUrl.split(",")[1];
      if (base64) resolve(base64);
      else reject(new Error("Empty recorded audio payload."));
    };
    reader.onerror = () => reject(new Error("Could not read microphone recording data."));
    reader.readAsDataURL(blob);
  });
}

export function completeRecordingBlob(chunks: Blob[], mimeType: string): Blob {
  return new Blob(chunks, { type: mimeType || "audio/webm" });
}

function utf8Tail(text: string, maxBytes: number): string {
  const bytes = new TextEncoder().encode(text);
  if (bytes.length <= maxBytes) return text;
  let start = bytes.length - maxBytes;
  while (start < bytes.length && (bytes[start] & 0xc0) === 0x80) {
    start += 1;
  }
  return new TextDecoder().decode(bytes.slice(start));
}

export function VadSettingsModal({
  settings,
  onChange,
  onReset,
  onClose,
}: {
  settings: VadSettings;
  onChange: (updater: Partial<VadSettings>) => void;
  onReset: () => void;
  onClose: () => void;
}) {
  return (
    <div className="vad-settings-overlay" onClick={onClose}>
      <div
        className="vad-settings-modal"
        role="dialog"
        aria-labelledby="vad-settings-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="vad-settings-header">
          <div className="vad-settings-title-wrap">
            <Sliders size={16} />
            <h3 id="vad-settings-title">Voice Activity Detection (VAD)</h3>
          </div>
          <button
            type="button"
            className="vad-settings-close"
            onClick={onClose}
            aria-label="Close VAD settings"
          >
            <X size={15} />
          </button>
        </div>

        <p className="vad-settings-desc">
          Automatically stops microphone dictation when you finish speaking so you don&apos;t need to
          manually press stop.
        </p>

        <div className="vad-settings-body">
          <label className="vad-setting-toggle">
            <input
              type="checkbox"
              checked={settings.enabled}
              onChange={(e) => onChange({ enabled: e.target.checked })}
            />
            <span className="vad-toggle-text">
              <strong>Auto-stop on silence</strong>
              <small>Detect when speech ends and automatically finalize transcript</small>
            </span>
          </label>

          <div className={`vad-settings-sliders ${settings.enabled ? "" : "disabled"}`}>
            <div className="vad-slider-row">
              <div className="vad-slider-label">
                <span>Silence Duration</span>
                <strong>{settings.silenceTimeoutSec.toFixed(1)}s</strong>
              </div>
              <input
                type="range"
                min={0.5}
                max={10.0}
                step={0.1}
                disabled={!settings.enabled}
                value={settings.silenceTimeoutSec}
                onChange={(e) => onChange({ silenceTimeoutSec: parseFloat(e.target.value) })}
              />
              <small className="vad-slider-help">
                Time of silence required after speaking before auto-stopping (default: 2.0s).
              </small>
            </div>

            <div className="vad-slider-row">
              <div className="vad-slider-label">
                <span>Mic Speech Threshold</span>
                <strong>{settings.speechThresholdDb} dB</strong>
              </div>
              <input
                type="range"
                min={-60}
                max={-20}
                step={1}
                disabled={!settings.enabled}
                value={settings.speechThresholdDb}
                onChange={(e) => onChange({ speechThresholdDb: parseInt(e.target.value, 10) })}
              />
              <small className="vad-slider-help">
                Audio sensitivity (-60 dB for quiet room / whisper, -20 dB for noisy environment).
              </small>
            </div>

            <div className="vad-slider-row">
              <div className="vad-slider-label">
                <span>Minimum Speech Trigger</span>
                <strong>{settings.minSpeechDurationMs}ms</strong>
              </div>
              <input
                type="range"
                min={100}
                max={2000}
                step={50}
                disabled={!settings.enabled}
                value={settings.minSpeechDurationMs}
                onChange={(e) => onChange({ minSpeechDurationMs: parseInt(e.target.value, 10) })}
              />
              <small className="vad-slider-help">
                Speech duration required before silence detection arms, preventing brief clicks from triggering auto-stop.
              </small>
            </div>

            <div className="vad-slider-row">
              <div className="vad-slider-label">
                <span>Initial Grace Period</span>
                <strong>{settings.initialGraceTimeoutSec.toFixed(0)}s</strong>
              </div>
              <input
                type="range"
                min={3}
                max={60}
                step={1}
                disabled={!settings.enabled}
                value={settings.initialGraceTimeoutSec}
                onChange={(e) => onChange({ initialGraceTimeoutSec: parseFloat(e.target.value) })}
              />
              <small className="vad-slider-help">
                Maximum time allowed before auto-stopping if you haven&apos;t started speaking yet.
              </small>
            </div>
          </div>
        </div>

        <div className="vad-settings-footer">
          <button type="button" className="vad-reset-button" onClick={onReset}>
            <RotateCcw size={13} />
            <span>Reset to Defaults</span>
          </button>
          <button type="button" className="vad-done-button" onClick={onClose}>
            <Check size={14} />
            <span>Done</span>
          </button>
        </div>
      </div>
    </div>
  );
}

export function SpeechDictationButton({
  sourceKind,
  sourceId,
  value,
  onChange,
  onRecordingComplete,
  onActiveChange,
  disabled = false,
  label = "Dictate",
}: {
  sourceKind: SourceKind;
  sourceId: string;
  value: string;
  onChange: (value: string) => void;
  onRecordingComplete?: (recording: { audioRelativePath: string; words: SpeechTiming[] }) => void;
  onActiveChange?: (active: boolean) => void;
  disabled?: boolean;
  label?: string;
}) {
  const { snapshot, prepare, vadSettings, updateVadSettings, resetVadSettings } = useSpeech();
  const [preparing, setPreparing] = useState(false);
  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const [detail, setDetail] = useState("");
  const [failed, setFailed] = useState(false);
  const [showVadSettings, setShowVadSettings] = useState(false);
  const [isSpeakingLive, setIsSpeakingLive] = useState(false);
  const [silenceProgress, setSilenceProgress] = useState(0);

  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const vadDetectorRef = useRef<VoiceActivityDetector | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const provisionalTranscriptRef = useRef("");
  const initialRef = useRef("");
  const recordingIdRef = useRef("");
  const modelIdRef = useRef("");
  const mimeRef = useRef("");
  const pendingRef = useRef<Promise<void> | null>(null);
  const timerRef = useRef<number | null>(null);
  const liveElapsedSecondsRef = useRef(0);
  const liveCheckpointIndexRef = useRef(0);
  const timeoutRefs = useRef(new Set<number>());
  const mountedRef = useRef(true);

  const clearTimeouts = useCallback(() => {
    for (const timeout of timeoutRefs.current) window.clearTimeout(timeout);
    timeoutRefs.current.clear();
  }, []);

  const scheduleTimeout = useCallback((callback: () => void, milliseconds: number) => {
    if (!mountedRef.current) return;
    const timeout = window.setTimeout(() => {
      timeoutRefs.current.delete(timeout);
      if (mountedRef.current) callback();
    }, milliseconds);
    timeoutRefs.current.add(timeout);
  }, []);

  const applyTranscript = useCallback(
    (transcript: string) => {
      const spoken = transcript.trim();
      onChange(
        [initialRef.current.trimEnd(), spoken]
          .filter(Boolean)
          .join(initialRef.current.trim() ? " " : ""),
      );
    },
    [onChange],
  );

  const transcribe = useCallback(
    (finalPass: boolean) => {
      if (pendingRef.current) return pendingRef.current;
      const emptyFinal = () =>
        finalPass
          ? releaseLocalSpeechMemory()
              .catch(() => undefined)
              .finally(() => onActiveChange?.(false))
          : Promise.resolve();

      const chunks = chunksRef.current;
      if (!chunks.length) return emptyFinal();
      const blob = completeRecordingBlob(chunks, mimeRef.current);
      if (blob.size < 128) return emptyFinal();

      const task = (async () => {
        setTranscribing(true);
        setFailed(false);
        setDetail(
          finalPass ? "Finalizing words and timestamps…" : "Updating live local transcript…",
        );
        const result = await transcribeLocalSpeech({
          jobId: id(finalPass ? "stt-final" : "stt-live"),
          sourceKind,
          sourceId,
          recordingId: recordingIdRef.current,
          audioBase64: await blobBase64(blob),
          mimeType: mimeRef.current || blob.type,
          modelId: modelIdRef.current,
          language: "auto",
          prompt: utf8Tail(initialRef.current, 4_000),
          finalPass,
        });
        provisionalTranscriptRef.current = result.text.trim();
        applyTranscript(provisionalTranscriptRef.current);
        if (finalPass && result.audioRelativePath) {
          onRecordingComplete?.({
            audioRelativePath: result.audioRelativePath,
            words: result.words,
          });
        }
        setDetail(
          finalPass ? "Saved locally with word timestamps" : "Listening · live text updated",
        );
      })()
        .catch((error) => {
          const message = error instanceof Error ? error.message : String(error);
          setFailed(finalPass);
          setDetail(
            finalPass
              ? `Dictation stopped: ${message}`
              : `Still listening · live update will retry: ${message}`,
          );
        })
        .finally(async () => {
          if (finalPass) await releaseLocalSpeechMemory().catch(() => undefined);
          pendingRef.current = null;
          setTranscribing(false);
          if (finalPass) onActiveChange?.(false);
        });
      pendingRef.current = task;
      return task;
    },
    [applyTranscript, onActiveChange, sourceId, sourceKind],
  );

  const stop = useCallback(() => {
    if (timerRef.current !== null) window.clearInterval(timerRef.current);
    timerRef.current = null;
    clearTimeouts();

    if (vadDetectorRef.current) {
      vadDetectorRef.current.destroy();
      vadDetectorRef.current = null;
    }

    setIsSpeakingLive(false);
    setSilenceProgress(0);

    const recorder = recorderRef.current;
    if (recorder && recorder.state !== "inactive") recorder.stop();
    else streamRef.current?.getTracks().forEach((track) => track.stop());
    setRecording(false);
  }, [clearTimeouts]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      stop();
      clearTimeouts();
      onActiveChange?.(false);
    };
  }, [clearTimeouts, onActiveChange, stop]);

  const start = async () => {
    const ready = snapshot?.transcriptionAvailable ? snapshot : await prepare();
    const model = ready.transcribers[0];
    if (!ready.transcriptionAvailable || !model) throw new Error(ready.detail);
    if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") {
      throw new Error("This desktop WebView cannot capture microphone audio.");
    }
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
    });
    streamRef.current = stream;

    try {
      const mime = recorderMimeType();
      const recorder = new MediaRecorder(stream, {
        ...(mime ? { mimeType: mime } : {}),
        audioBitsPerSecond: 32_000,
      });
      chunksRef.current = [];
      provisionalTranscriptRef.current = "";
      initialRef.current = value;
      recordingIdRef.current = id("recording");
      modelIdRef.current = model.id;
      mimeRef.current = recorder.mimeType || mime || "audio/webm";
      liveElapsedSecondsRef.current = 0;
      liveCheckpointIndexRef.current = 0;
      recorderRef.current = recorder;

      recorder.ondataavailable = (event) => {
        if (event.data.size) chunksRef.current.push(event.data);
      };
      recorder.onerror = () => {
        setFailed(true);
        setDetail("Microphone recording failed.");
        stop();
      };
      recorder.onstop = () => {
        stream.getTracks().forEach((track) => track.stop());
        streamRef.current = null;
        recorderRef.current = null;
        const finish = async () => {
          if (pendingRef.current) await pendingRef.current;
          await transcribe(true);
        };
        scheduleTimeout(() => void finish(), 100);
      };

      // Initialize VAD for auto-shutoff on silence
      if (vadSettings.enabled) {
        vadDetectorRef.current = new VoiceActivityDetector(stream, vadSettings, {
          onSilenceTimeout: () => {
            if (mountedRef.current) {
              setDetail("Auto-stopped after silence");
              stop();
            }
          },
          onEnergyUpdate: (_db, isSpeaking, ratio) => {
            if (mountedRef.current) {
              setIsSpeakingLive(isSpeaking);
              setSilenceProgress(ratio);
            }
          },
        });
      }

      recorder.start(500);
      setRecording(true);
      setFailed(false);
      onActiveChange?.(true);
      setDetail(
        vadSettings.enabled
          ? `Listening locally · auto-stops after ${vadSettings.silenceTimeoutSec.toFixed(1)}s silence`
          : "Listening locally…",
      );

      timerRef.current = window.setInterval(() => {
        liveElapsedSecondsRef.current += 4;
        const previousIndex = liveCheckpointIndexRef.current;
        const nextIndex = advanceLiveTranscriptionCheckpoint(
          liveElapsedSecondsRef.current,
          previousIndex,
        );
        if (nextIndex === previousIndex) {
          const finalCheckpoint = LIVE_TRANSCRIPTION_CHECKPOINTS_SECONDS.at(-1) ?? 0;
          if (
            previousIndex === LIVE_TRANSCRIPTION_CHECKPOINTS_SECONDS.length &&
            liveElapsedSecondsRef.current === finalCheckpoint + 4
          ) {
            setDetail("Listening locally · full recording will finalize when you stop");
          }
          return;
        }
        liveCheckpointIndexRef.current = nextIndex;
        if (recorder.state === "recording") recorder.requestData();
        scheduleTimeout(() => void transcribe(false), 100);
      }, 4_000);

      scheduleTimeout(() => {
        if (recorder.state === "recording") stop();
      }, 15 * 60 * 1_000);
    } catch (error) {
      if (vadDetectorRef.current) {
        vadDetectorRef.current.destroy();
        vadDetectorRef.current = null;
      }
      recorderRef.current = null;
      streamRef.current = null;
      stream.getTracks().forEach((track) => track.stop());
      throw error;
    }
  };

  const begin = async () => {
    setPreparing(true);
    setFailed(false);
    setDetail("Preparing private Whisper dictation…");
    try {
      await start();
    } catch (error) {
      setFailed(true);
      const message = error instanceof Error ? error.message : String(error);
      setDetail(
        `Dictation unavailable: ${message} Open Setup → Whisper dictation + local voice to install or repair it.`,
      );
    } finally {
      setPreparing(false);
    }
  };

  return (
    <span
      className={`speech-dictation ${recording ? "recording" : ""} ${isSpeakingLive ? "speaking-live" : ""}`}
    >
      <button
        type="button"
        title={detail || `${label} with local ComfyUI Whisper`}
        aria-label={
          recording ? "Stop dictation" : preparing ? "Preparing dictation" : label
        }
        disabled={
          preparing || (disabled && !recording) || (transcribing && !recording)
        }
        onClick={() => (recording ? stop() : void begin())}
      >
        {preparing || (transcribing && !recording) ? (
          <LoaderCircle className="spin" />
        ) : recording ? (
          <Square />
        ) : (
          <Mic />
        )}
        <span>
          {recording ? "Stop" : preparing ? "Preparing…" : transcribing ? "Saving…" : label}
        </span>
      </button>

      {recording && (
        <i
          className="speech-recording-vad-indicator"
          style={{ opacity: isSpeakingLive ? 1 : Math.max(0.2, 1 - silenceProgress) }}
          title={
            isSpeakingLive
              ? "Speech detected"
              : `Silence (${Math.round((1 - silenceProgress) * vadSettings.silenceTimeoutSec * 10) / 10}s remaining)`
          }
          aria-hidden="true"
        />
      )}

      <button
        type="button"
        className="speech-vad-settings-btn"
        title="Voice Activity Detection (VAD) Settings"
        aria-label="Voice Activity Detection Settings"
        onClick={() => setShowVadSettings(true)}
      >
        <Sliders size={12} />
      </button>

      {detail && (
        <small
          className={`speech-dictation-status ${failed ? "error" : ""}`}
          role="status"
          aria-live="polite"
        >
          {detail}
        </small>
      )}

      {showVadSettings && (
        <VadSettingsModal
          settings={vadSettings}
          onChange={updateVadSettings}
          onReset={resetVadSettings}
          onClose={() => setShowVadSettings(false)}
        />
      )}
    </span>
  );
}
