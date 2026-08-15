import { LoaderCircle, Mic, Pause, Play, Square, Volume2 } from "lucide-react";
import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  alignLocalSpeech,
  cancelLocalSpeech,
  getLocalSpeechSnapshot,
  localSpeechMediaUrl,
  prepareLocalSpeech,
  releaseLocalSpeechMemory,
  synthesizeLocalSpeech,
  transcribeLocalSpeech,
} from "./api";
import type { LocalSpeechSnapshot, SpeechTiming } from "./types";

type SourceKind = "research" | "chat" | "task" | "copilot";

type SpeechContextValue = {
  snapshot: LocalSpeechSnapshot | null;
  refresh: () => Promise<LocalSpeechSnapshot>;
  prepare: () => Promise<LocalSpeechSnapshot>;
};

const SpeechContext = createContext<SpeechContextValue | null>(null);

let activePlaybackStop: (() => void) | null = null;

function claimPlayback(stop: () => void) {
  activePlaybackStop?.();
  activePlaybackStop = stop;
}

function clearPlayback(stop: () => void) {
  if (activePlaybackStop === stop) activePlaybackStop = null;
}

export function LocalSpeechProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<LocalSpeechSnapshot | null>(null);
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
  useEffect(() => {
    void refresh().catch(() => undefined);
  }, [refresh]);
  const value = useMemo(() => ({ snapshot, refresh, prepare }), [prepare, refresh, snapshot]);
  return <SpeechContext.Provider value={value}>{children}</SpeechContext.Provider>;
}

function useSpeech() {
  const value = useContext(SpeechContext);
  return value ?? {
    snapshot: null,
    refresh: getLocalSpeechSnapshot,
    prepare: prepareLocalSpeech,
  };
}

export function splitSpeechText(text: string, maximum = 1_800): string[] {
  const normalized = text.replace(/```[\s\S]*?```/g, " Code block available on screen. ").replace(/\s+/g, " ").trim();
  if (!normalized) return [];
  const sentences = normalized.match(/[^.!?]+(?:[.!?]+["'’”)]*|$)/g) ?? [normalized];
  const chunks: string[] = [];
  let current = "";
  for (const raw of sentences) {
    const sentence = raw.trim();
    if (!sentence) continue;
    if (sentence.length > maximum) {
      if (current) chunks.push(current);
      const characters = Array.from(sentence);
      for (let offset = 0; offset < characters.length; offset += maximum) {
        chunks.push(characters.slice(offset, offset + maximum).join("").trim());
      }
      current = "";
    } else if (!current) {
      current = sentence;
    } else if (current.length + sentence.length + 1 <= maximum) {
      current += ` ${sentence}`;
    } else {
      chunks.push(current);
      current = sentence;
    }
  }
  if (current) chunks.push(current);
  return chunks;
}

function id(prefix: string): string {
  const value = typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return `${prefix}-${value}`;
}

function wordTimings(text: string, duration: number): SpeechTiming[] {
  const words = text.match(/\S+/g) ?? [];
  const weights = words.map((word) => Math.max(1, word.replace(/[^\p{L}\p{N}]/gu, "").length));
  const total = weights.reduce((sum, value) => sum + value, 0) || 1;
  let cursor = 0;
  return words.map((value, index) => {
    const start = duration * cursor / total;
    cursor += weights[index];
    return { value, start, end: duration * cursor / total };
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

export function SpeechLiveCaption({ text, seconds, duration, timings = [], onSeek }: { text: string; seconds: number; duration: number; timings?: SpeechTiming[]; onSeek?: (seconds: number) => void }) {
  const caption = activeCaption(text, seconds, duration, timings);
  if (!caption.words.length) return null;
  return <div className="speech-live-caption" aria-live="off">
    {caption.words.map((word, index) => onSeek
      ? <button type="button" className={index === caption.active ? "active" : undefined} aria-current={index === caption.active ? "true" : undefined} aria-label={`Start from ${word.value}`} title={`Start from ${word.value}`} key={`${word.start}-${index}`} onClick={() => onSeek(word.start)}>{word.value}</button>
      : index === caption.active
        ? <mark key={`${word.start}-${index}`}>{word.value}</mark>
        : <span key={`${word.start}-${index}`}>{word.value}</span>)}
  </div>;
}

export function SpeechPlaybackButton({ sourceKind, sourceId, passageId, text, label = "Listen" }: {
  sourceKind: SourceKind;
  sourceId: string;
  passageId: string;
  text: string;
  label?: string;
}) {
  const { snapshot, prepare } = useSpeech();
  const [state, setState] = useState<"idle" | "preparing" | "playing" | "paused" | "error">("idle");
  const [detail, setDetail] = useState("");
  const [currentText, setCurrentText] = useState("");
  const [seconds, setSeconds] = useState(0);
  const [duration, setDuration] = useState(0);
  const [timings, setTimings] = useState<SpeechTiming[]>([]);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const jobRef = useRef<string | null>(null);
  const alignmentJobRef = useRef<string | null>(null);
  const activeChunkRef = useRef(-1);
  const generationRef = useRef(0);
  const chunks = useMemo(() => splitSpeechText(text), [text]);

  const stop = useCallback(() => {
    generationRef.current += 1;
    if (jobRef.current) void cancelLocalSpeech(jobRef.current);
    if (alignmentJobRef.current) void cancelLocalSpeech(alignmentJobRef.current);
    jobRef.current = null;
    alignmentJobRef.current = null;
    activeChunkRef.current = -1;
    audioRef.current?.pause();
    if (audioRef.current) audioRef.current.removeAttribute("src");
    setState("idle");
    setSeconds(0);
    setCurrentText("");
  }, []);

  useEffect(() => () => {
    clearPlayback(stop);
    stop();
  }, [stop]);

  const playChunk = useCallback(async (index: number, generation: number, voiceId: string, alignmentModelId?: string) => {
    const chunk = chunks[index];
    if (!chunk || generation !== generationRef.current) {
      clearPlayback(stop);
      stop();
      void releaseLocalSpeechMemory().catch(() => undefined);
      return;
    }
    if (alignmentJobRef.current) void cancelLocalSpeech(alignmentJobRef.current);
    alignmentJobRef.current = null;
    activeChunkRef.current = index;
    const jobId = id("tts");
    jobRef.current = jobId;
    setCurrentText(chunk);
    setTimings([]);
    setSeconds(0);
    setDuration(0);
    setState("preparing");
    setDetail(index ? `Preparing part ${index + 1} of ${chunks.length}` : "Preparing local voice");
    try {
      const clip = await synthesizeLocalSpeech({
        jobId,
        sourceKind,
        sourceId,
        passageId: `${passageId}-${index + 1}`,
        text: chunk,
        modelId: voiceId,
      });
      if (generation !== generationRef.current) return;
      jobRef.current = null;
      const audio = audioRef.current;
      if (!audio) return;
      audio.src = localSpeechMediaUrl(clip.relativePath);
      setTimings(clip.words);
      audio.onended = () => void playChunk(index + 1, generation, voiceId, alignmentModelId);
      await audio.play();
      setState("playing");
      setDetail(`${label} · ${index + 1} / ${chunks.length}`);
      if (!clip.words.length && alignmentModelId) {
        const alignmentJobId = id("speech-alignment");
        alignmentJobRef.current = alignmentJobId;
        void alignLocalSpeech({
          jobId: alignmentJobId,
          sourceKind,
          sourceId,
          passageId: `${passageId}-${index + 1}`,
          text: chunk,
          relativePath: clip.relativePath,
          voiceModelId: voiceId,
          alignmentModelId,
        }).then((aligned) => {
          if (generation === generationRef.current && activeChunkRef.current === index) setTimings(aligned.words);
        }).catch(() => undefined).finally(() => {
          if (alignmentJobRef.current === alignmentJobId) alignmentJobRef.current = null;
        });
      }
    } catch (error) {
      if (generation !== generationRef.current) return;
      jobRef.current = null;
      clearPlayback(stop);
      setState("error");
      setDetail(String(error));
      void releaseLocalSpeechMemory().catch(() => undefined);
    }
  }, [chunks, label, passageId, sourceId, sourceKind, stop]);

  const toggle = () => {
    if (state === "playing") {
      audioRef.current?.pause();
      setState("paused");
      return;
    }
    if (state === "paused" && audioRef.current?.src) {
      void audioRef.current.play().then(() => setState("playing"));
      return;
    }
    const start = async () => {
      const ready = snapshot?.narrationAvailable ? snapshot : await prepare();
      const voice = ready.voices[0];
      if (!ready.narrationAvailable || !voice) throw new Error(ready.detail);
      const generation = generationRef.current + 1;
      generationRef.current = generation;
      claimPlayback(stop);
      await playChunk(0, generation, voice.id, ready.transcriptionAvailable ? ready.transcribers[0]?.id : undefined);
    };
    void start().catch((error) => {
      setState("error");
      setDetail(String(error));
    });
  };

  const caption = currentText ? activeCaption(currentText, seconds, duration, timings) : null;
  const seek = (nextSeconds: number) => {
    const audio = audioRef.current;
    if (!audio || !Number.isFinite(nextSeconds)) return;
    const maximum = Number.isFinite(audio.duration) ? audio.duration : nextSeconds;
    audio.currentTime = Math.max(0, Math.min(nextSeconds, maximum));
    setSeconds(audio.currentTime);
  };
  return <div className={`inline-speech ${state}`}>
    <audio ref={audioRef} preload="auto" onLoadedMetadata={(event) => setDuration(event.currentTarget.duration)} onTimeUpdate={(event) => setSeconds(event.currentTarget.currentTime)} />
    <button type="button" className="inline-speech-button" title={detail || label} aria-label={state === "playing" ? `Pause ${label.toLowerCase()}` : label} disabled={!chunks.length || state === "preparing"} onClick={toggle}>
      {state === "preparing" ? <LoaderCircle className="spin" /> : state === "playing" ? <Pause /> : <Volume2 />}
      <span>{state === "preparing" ? "Preparing…" : state === "paused" ? "Resume" : label}</span>
    </button>
    {state !== "idle" && <button type="button" className="inline-speech-stop" title="Stop speaking" aria-label="Stop speaking" onClick={() => { clearPlayback(stop); stop(); void releaseLocalSpeechMemory().catch(() => undefined); }}><Square /></button>}
    {caption && ["playing", "paused"].includes(state) && <>
      <input className="speech-inline-seek" type="range" aria-label="Speech position" min={0} max={Math.max(duration, 0.01)} step={0.01} value={Math.min(seconds, Math.max(duration, 0.01))} onChange={(event) => seek(event.currentTarget.valueAsNumber)} />
      <SpeechLiveCaption text={currentText} seconds={seconds} duration={duration} timings={timings} onSeek={seek} />
    </>}
    {state === "error" && <small className="speech-inline-error">{detail}</small>}
  </div>;
}

function recorderMimeType(): string {
  const options = ["audio/webm;codecs=opus", "audio/webm", "audio/mp4"];
  return options.find((value) => MediaRecorder.isTypeSupported(value)) ?? "";
}

function blobBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("Could not read microphone audio"));
    reader.onload = () => resolve(String(reader.result).split(",", 2)[1] ?? "");
    reader.readAsDataURL(blob);
  });
}

function utf8Tail(value: string, maximumBytes: number): string {
  const bytes = new TextEncoder().encode(value);
  if (bytes.length <= maximumBytes) return value;
  return new TextDecoder().decode(bytes.slice(bytes.length - maximumBytes)).replace(/^�/, "");
}

export function SpeechDictationButton({ sourceKind, sourceId, value, onChange, onActiveChange, disabled = false, label = "Dictate" }: {
  sourceKind: SourceKind;
  sourceId: string;
  value: string;
  onChange: (value: string) => void;
  onActiveChange?: (active: boolean) => void;
  disabled?: boolean;
  label?: string;
}) {
  const { snapshot, prepare } = useSpeech();
  const [preparing, setPreparing] = useState(false);
  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const [detail, setDetail] = useState("");
  const [failed, setFailed] = useState(false);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const submittedChunkCountRef = useRef(0);
  const provisionalTranscriptRef = useRef("");
  const initialRef = useRef("");
  const recordingIdRef = useRef("");
  const modelIdRef = useRef("");
  const mimeRef = useRef("");
  const pendingRef = useRef<Promise<void> | null>(null);
  const timerRef = useRef<number | null>(null);
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

  const applyTranscript = useCallback((transcript: string) => {
    const spoken = transcript.trim();
    onChange([initialRef.current.trimEnd(), spoken].filter(Boolean).join(initialRef.current.trim() ? " " : ""));
  }, [onChange]);

  const transcribe = useCallback((finalPass: boolean) => {
    if (pendingRef.current) return pendingRef.current;
    const emptyFinal = () => finalPass
      ? releaseLocalSpeechMemory().catch(() => undefined).finally(() => onActiveChange?.(false))
      : Promise.resolve();
    const chunkEnd = chunksRef.current.length;
    const chunks = finalPass
      ? chunksRef.current
      : chunksRef.current.slice(submittedChunkCountRef.current, chunkEnd);
    if (!chunks.length) return emptyFinal();
    const blob = new Blob(chunks, { type: mimeRef.current });
    if (blob.size < 128) return emptyFinal();
    const task = (async () => {
      setTranscribing(true);
      setFailed(false);
      setDetail(finalPass ? "Finalizing words and timestamps…" : "Updating live local transcript…");
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
      if (finalPass) {
        provisionalTranscriptRef.current = result.text.trim();
      } else {
        submittedChunkCountRef.current = chunkEnd;
        provisionalTranscriptRef.current = [provisionalTranscriptRef.current, result.text.trim()]
          .filter(Boolean)
          .join(" ");
      }
      applyTranscript(provisionalTranscriptRef.current);
      setDetail(finalPass ? "Saved locally with word timestamps" : "Listening · live text updated");
    })().catch((error) => {
      setFailed(true);
      setDetail(`Dictation stopped: ${error instanceof Error ? error.message : String(error)}`);
    }).finally(async () => {
      if (finalPass) await releaseLocalSpeechMemory().catch(() => undefined);
      pendingRef.current = null;
      setTranscribing(false);
      if (finalPass) onActiveChange?.(false);
    });
    pendingRef.current = task;
    return task;
  }, [applyTranscript, onActiveChange, sourceId, sourceKind]);

  const stop = useCallback(() => {
    if (timerRef.current !== null) window.clearInterval(timerRef.current);
    timerRef.current = null;
    clearTimeouts();
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
    const stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true } });
    const mime = recorderMimeType();
    const recorder = new MediaRecorder(stream, { ...(mime ? { mimeType: mime } : {}), audioBitsPerSecond: 32_000 });
    chunksRef.current = [];
    submittedChunkCountRef.current = 0;
    provisionalTranscriptRef.current = "";
    initialRef.current = value;
    recordingIdRef.current = id("recording");
    modelIdRef.current = model.id;
    mimeRef.current = recorder.mimeType || mime || "audio/webm";
    recorderRef.current = recorder;
    streamRef.current = stream;
    recorder.ondataavailable = (event) => { if (event.data.size) chunksRef.current.push(event.data); };
    recorder.onerror = () => { setFailed(true); setDetail("Microphone recording failed."); stop(); };
    recorder.onstop = () => {
      stream.getTracks().forEach((track) => track.stop());
      const finish = async () => {
        if (pendingRef.current) await pendingRef.current;
        await transcribe(true);
      };
      scheduleTimeout(() => void finish(), 100);
    };
    recorder.start(500);
    setRecording(true);
    setFailed(false);
    onActiveChange?.(true);
    setDetail("Listening locally…");
    timerRef.current = window.setInterval(() => {
      if (recorder.state === "recording") recorder.requestData();
      scheduleTimeout(() => void transcribe(false), 100);
    }, 4_000);
    scheduleTimeout(() => { if (recorder.state === "recording") stop(); }, 15 * 60 * 1_000);
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
      setDetail(`Dictation unavailable: ${message} Open Setup → Whisper dictation + local voice to install or repair it.`);
    } finally {
      setPreparing(false);
    }
  };

  return <span className={`speech-dictation ${recording ? "recording" : ""}`}>
    <button type="button" title={detail || `${label} with local ComfyUI Whisper`} aria-label={recording ? "Stop dictation" : preparing ? "Preparing dictation" : label} disabled={preparing || disabled && !recording || transcribing && !recording} onClick={() => recording ? stop() : void begin()}>
      {preparing || transcribing && !recording ? <LoaderCircle className="spin" /> : recording ? <Square /> : <Mic />}
      <span>{recording ? "Stop" : preparing ? "Preparing…" : transcribing ? "Saving…" : label}</span>
    </button>
    {recording && <i aria-hidden="true" />}
    {detail && <small className={`speech-dictation-status ${failed ? "error" : ""}`} role="status" aria-live="polite">{detail}</small>}
  </span>;
}
