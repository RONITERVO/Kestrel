import { Pause, Play, SkipBack, SkipForward, Square, Volume2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  alignLocalSpeech,
  cancelLocalSpeech,
  getLocalSpeechSnapshot,
  localSpeechMediaUrl,
  onLocalSpeechProgress,
  prepareLocalSpeech,
  releaseLocalSpeechMemory,
  synthesizeLocalSpeech,
} from "./api";
import { buildResearchSpeechPassages, type ResearchSpeechScope } from "./researchSpeechContent";
import { SpeechLiveCaption } from "./LocalSpeechControls";
import type { LocalSpeechSnapshot, ResearchReport, SpeechTiming } from "./types";

type PlayerStatus = "ready" | "preparing" | "playing" | "paused" | "complete" | "error";

const MODEL_KEY = "kestrel.researchSpeech.comfyModel";
const RATE_KEY = "kestrel.researchSpeech.rate";
const SCOPE_KEY = "kestrel.researchSpeech.scope";

function readPreference(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

function savePreference(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Playback remains available when WebView preference storage is unavailable.
  }
}

function initialRate(): number {
  const saved = Number(readPreference(RATE_KEY));
  return Number.isFinite(saved) && saved >= 0.8 && saved <= 1.5 ? saved : 1;
}

function initialScope(): ResearchSpeechScope {
  const saved = readPreference(SCOPE_KEY);
  return saved === "summary" || saved === "all" ? saved : "article";
}

function jobId(): string {
  const random = typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return `speech-${random}`;
}

interface ResearchSpeechPlayerProps {
  report: ResearchReport;
  onPassageChange: (anchorId: string | null) => void;
}

export function ResearchSpeechPlayer({ report, onPassageChange }: ResearchSpeechPlayerProps) {
  const [snapshot, setSnapshot] = useState<LocalSpeechSnapshot | null>(null);
  const [modelId, setModelId] = useState(() => readPreference(MODEL_KEY) ?? "");
  const [rate, setRate] = useState(initialRate);
  const [scope, setScope] = useState<ResearchSpeechScope>(initialScope);
  const [status, setStatus] = useState<PlayerStatus>("ready");
  const [currentIndex, setCurrentIndex] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const [audioProgress, setAudioProgress] = useState(0);
  const [speechSeconds, setSpeechSeconds] = useState(0);
  const [speechDuration, setSpeechDuration] = useState(0);
  const [speechTimings, setSpeechTimings] = useState<SpeechTiming[]>([]);
  const [bufferingIndex, setBufferingIndex] = useState<number | null>(null);
  const [detail, setDetail] = useState("Checking local ComfyUI voice models...");
  const [error, setError] = useState<string | null>(null);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const currentIndexRef = useRef(0);
  const rateRef = useRef(rate);
  const playbackGenerationRef = useRef(0);
  const activeJobsRef = useRef(new Set<string>());
  const clipUrlsRef = useRef(new Map<string, string>());
  const clipTimingsRef = useRef(new Map<string, SpeechTiming[]>());
  const clipPathsRef = useRef(new Map<string, string>());
  const pendingClipsRef = useRef(new Map<string, Promise<string>>());
  const pendingAlignmentsRef = useRef(new Map<string, Promise<void>>());
  const alignmentJobsRef = useRef(new Map<string, string>());
  const mountedRef = useRef(true);
  const startAtRef = useRef<(index: number) => Promise<void>>(async () => undefined);
  const passages = useMemo(() => buildResearchSpeechPassages(report, scope), [report, scope]);
  const selectedModel = snapshot?.voices.find((model) => model.id === modelId) ?? snapshot?.voices[0] ?? null;

  useEffect(() => {
    currentIndexRef.current = currentIndex;
    rateRef.current = rate;
  }, [currentIndex, rate]);

  useEffect(() => {
    let active = true;
    void getLocalSpeechSnapshot()
      .then((next) => {
        if (!active) return;
        setSnapshot(next);
        setDetail(next.detail);
        const selected = next.voices.find((model) => model.id === modelId) ?? next.voices[0];
        setModelId(selected?.id ?? "");
        if (selected) savePreference(MODEL_KEY, selected.id);
        if (next.narrationAvailable && !next.comfyReady) {
          setDetail("Starting the private ComfyUI voice engine in the background...");
          void prepareLocalSpeech().then((ready) => {
            if (!active) return;
            setSnapshot(ready);
            setDetail(ready.detail);
          }).catch((cause) => {
            if (!active) return;
            setDetail(`ComfyUI will retry when Play is pressed: ${String(cause)}`);
          });
        }
      })
      .catch((cause) => {
        if (!active) return;
        setError(String(cause));
        setStatus("error");
      });
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    void onLocalSpeechProgress((progress) => {
      if (!activeJobsRef.current.has(progress.jobId)) return;
      setDetail(progress.detail);
    }).then((unlisten) => {
      dispose = unlisten;
    });
    return () => dispose?.();
  }, []);

  useEffect(() => {
    if (status !== "preparing") return;
    setElapsed(0);
    const timer = window.setInterval(() => setElapsed((value) => value + 1), 1_000);
    return () => window.clearInterval(timer);
  }, [status]);

  const cancelActiveJobs = useCallback(() => {
    for (const activeJob of activeJobsRef.current) void cancelLocalSpeech(activeJob);
    activeJobsRef.current.clear();
    pendingClipsRef.current.clear();
    pendingAlignmentsRef.current.clear();
    alignmentJobsRef.current.clear();
  }, []);

  const resetPlayback = useCallback(() => {
    playbackGenerationRef.current += 1;
    cancelActiveJobs();
    const audio = audioRef.current;
    if (audio) {
      if (!audio.paused) audio.pause();
      audio.removeAttribute("src");
    }
    currentIndexRef.current = 0;
    setCurrentIndex(0);
    setAudioProgress(0);
    setSpeechSeconds(0);
    setSpeechDuration(0);
    setSpeechTimings([]);
    setBufferingIndex(null);
    setStatus("ready");
    setError(null);
    onPassageChange(null);
  }, [cancelActiveJobs, onPassageChange]);

  useEffect(() => {
    resetPlayback();
  }, [report.id, resetPlayback, scope]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      playbackGenerationRef.current += 1;
      cancelActiveJobs();
      if (audioRef.current && !audioRef.current.paused) audioRef.current.pause();
      void releaseLocalSpeechMemory().catch(() => undefined);
    };
  }, [cancelActiveJobs]);

  const clipKey = useCallback((index: number) => {
    const passage = passages[index];
    return passage && selectedModel
      ? `${report.id}:${selectedModel.id}:${passage.id}:${passage.text}`
      : "";
  }, [passages, report.id, selectedModel]);

  const ensureClip = useCallback((index: number, background: boolean): Promise<string> => {
    const passage = passages[index];
    if (!passage || !selectedModel) return Promise.reject(new Error("No local ComfyUI voice model is selected."));
    const key = clipKey(index);
    const cached = clipUrlsRef.current.get(key);
    if (cached) return Promise.resolve(cached);
    const pending = pendingClipsRef.current.get(key);
    if (pending) return pending;

    const activeJob = jobId();
    activeJobsRef.current.add(activeJob);
    if (background) {
      setBufferingIndex(index);
    } else {
      setStatus("preparing");
      setDetail(snapshot?.comfyReady
        ? `Generating ${passage.label} with ${selectedModel.name}...`
        : `Starting ComfyUI and loading ${selectedModel.name}...`);
      setError(null);
    }
    const promise = synthesizeLocalSpeech({
      jobId: activeJob,
      sourceKind: "research",
      sourceId: report.id,
      passageId: passage.id,
      text: passage.text,
      modelId: selectedModel.id,
    }).then((clip) => {
      const url = localSpeechMediaUrl(clip.relativePath);
      if (!url) throw new Error("Kestrel could not create a private URL for the generated passage.");
      clipUrlsRef.current.set(key, url);
      clipTimingsRef.current.set(key, clip.words);
      clipPathsRef.current.set(key, clip.relativePath);
      return url;
    }).finally(() => {
      activeJobsRef.current.delete(activeJob);
      pendingClipsRef.current.delete(key);
      if (background && mountedRef.current) setBufferingIndex((current) => current === index ? null : current);
    });
    pendingClipsRef.current.set(key, promise);
    return promise;
  }, [clipKey, passages, report.id, selectedModel, snapshot?.comfyReady]);

  const alignClip = useCallback((index: number): Promise<void> => {
    const passage = passages[index];
    const alignmentModel = snapshot?.transcriptionAvailable ? snapshot.transcribers[0] : undefined;
    if (!passage || !selectedModel || !alignmentModel) return Promise.resolve();
    const key = clipKey(index);
    if (clipTimingsRef.current.get(key)?.length) return Promise.resolve();
    const pending = pendingAlignmentsRef.current.get(key);
    if (pending) return pending;
    const relativePath = clipPathsRef.current.get(key);
    if (!relativePath) return Promise.resolve();
    const alignmentJob = jobId();
    activeJobsRef.current.add(alignmentJob);
    alignmentJobsRef.current.set(key, alignmentJob);
    const task = alignLocalSpeech({
      jobId: alignmentJob,
      sourceKind: "research",
      sourceId: report.id,
      passageId: passage.id,
      text: passage.text,
      relativePath,
      voiceModelId: selectedModel.id,
      alignmentModelId: alignmentModel.id,
    }).then((aligned) => {
      clipTimingsRef.current.set(key, aligned.words);
      if (mountedRef.current && clipKey(currentIndexRef.current) === key) setSpeechTimings(aligned.words);
    }).catch(() => undefined).finally(() => {
      activeJobsRef.current.delete(alignmentJob);
      alignmentJobsRef.current.delete(key);
      pendingAlignmentsRef.current.delete(key);
    });
    pendingAlignmentsRef.current.set(key, task);
    return task;
  }, [clipKey, passages, report.id, selectedModel, snapshot?.transcribers, snapshot?.transcriptionAvailable]);

  const startAt = useCallback(async (requestedIndex: number) => {
    if (!snapshot?.narrationAvailable || !selectedModel || !passages.length) return;
    const index = Math.max(0, Math.min(requestedIndex, passages.length - 1));
    const generation = playbackGenerationRef.current + 1;
    playbackGenerationRef.current = generation;
    const audio = audioRef.current;
    if (!audio) return;
    audio.pause();
    currentIndexRef.current = index;
    setCurrentIndex(index);
    setAudioProgress(0);
    setSpeechSeconds(0);
    setSpeechDuration(0);
    setSpeechTimings([]);
    setError(null);
    onPassageChange(passages[index].anchorId);
    try {
      const url = await ensureClip(index, false);
      if (!mountedRef.current || generation !== playbackGenerationRef.current) return;
      audio.src = url;
      setSpeechTimings(clipTimingsRef.current.get(clipKey(index)) ?? []);
      audio.playbackRate = rateRef.current;
      await audio.play();
      if (!mountedRef.current || generation !== playbackGenerationRef.current) return;
      setStatus("playing");
      setDetail(`Reading ${passages[index].label} with ${selectedModel.name}.`);
      void alignClip(index).finally(() => {
        if (mountedRef.current && generation === playbackGenerationRef.current && index + 1 < passages.length) {
          void ensureClip(index + 1, true).catch(() => undefined);
        }
      });
    } catch (cause) {
      if (!mountedRef.current || generation !== playbackGenerationRef.current) return;
      const message = String(cause);
      if (/stopped|cancelled/i.test(message)) {
        setStatus("ready");
      } else {
        setStatus("error");
        setError(message);
      }
      onPassageChange(null);
      void releaseLocalSpeechMemory().catch(() => undefined);
    }
  }, [alignClip, clipKey, ensureClip, onPassageChange, passages, selectedModel, snapshot?.narrationAvailable]);
  useEffect(() => {
    startAtRef.current = startAt;
  }, [startAt]);

  const navigateTo = (index: number) => {
    const key = clipKey(currentIndexRef.current);
    const alignmentJob = alignmentJobsRef.current.get(key);
    if (alignmentJob) void cancelLocalSpeech(alignmentJob);
    void startAt(index);
  };

  const togglePlayback = () => {
    const audio = audioRef.current;
    if (!audio || !selectedModel) return;
    if (status === "playing") {
      audio.pause();
      setStatus("paused");
    } else if (status === "paused" && audio.src) {
      audio.playbackRate = rateRef.current;
      void audio.play().then(() => setStatus("playing")).catch((cause) => {
        setStatus("error");
        setError(String(cause));
      });
    } else if (status !== "preparing") {
      void startAt(status === "complete" ? 0 : currentIndexRef.current);
    }
  };

  const stopPlayback = () => {
    resetPlayback();
    void releaseLocalSpeechMemory().catch(() => undefined);
  };

  const seekSpeech = (nextSeconds: number) => {
    const audio = audioRef.current;
    if (!audio || !Number.isFinite(nextSeconds)) return;
    const maximum = Number.isFinite(audio.duration) ? audio.duration : nextSeconds;
    audio.currentTime = Math.max(0, Math.min(nextSeconds, maximum));
    setSpeechSeconds(audio.currentTime);
    setAudioProgress(maximum > 0 ? audio.currentTime / maximum : 0);
  };

  const chooseModel = (nextModelId: string) => {
    resetPlayback();
    clipUrlsRef.current.clear();
    clipPathsRef.current.clear();
    clipTimingsRef.current.clear();
    setModelId(nextModelId);
    savePreference(MODEL_KEY, nextModelId);
  };

  const chooseRate = (nextRate: number) => {
    if (!Number.isFinite(nextRate) || nextRate < 0.8 || nextRate > 1.5) return;
    setRate(nextRate);
    rateRef.current = nextRate;
    if (audioRef.current) audioRef.current.playbackRate = nextRate;
    savePreference(RATE_KEY, String(nextRate));
  };

  const chooseScope = (nextScope: ResearchSpeechScope) => {
    setScope(nextScope);
    savePreference(SCOPE_KEY, nextScope);
  };

  const unavailable = !snapshot?.narrationAvailable || !selectedModel;
  const activePassage = passages[currentIndex];
  const progress = status === "complete"
    ? 100
    : passages.length > 0
      ? ((currentIndex + audioProgress) / passages.length) * 100
      : 0;
  const statusText = !snapshot
    ? "Checking local ComfyUI"
    : unavailable
      ? "ComfyUI TTS unavailable"
      : status === "preparing"
        ? `Preparing ${activePassage?.label ?? "passage"} - ${elapsed}s`
        : status === "playing"
          ? `Reading ${activePassage?.label ?? "report"}`
          : status === "paused"
            ? `Paused at ${activePassage?.label ?? "report"}`
            : status === "complete"
              ? "Finished"
              : "Ready with local ComfyUI";

  return (
    <section className="research-speech-player" aria-label="Listen to report">
      <audio
        ref={audioRef}
        preload="auto"
        onTimeUpdate={(event) => {
          const audio = event.currentTarget;
          setSpeechSeconds(audio.currentTime);
          setSpeechDuration(Number.isFinite(audio.duration) ? audio.duration : 0);
          setAudioProgress(Number.isFinite(audio.duration) && audio.duration > 0 ? audio.currentTime / audio.duration : 0);
        }}
        onEnded={() => {
          if (currentIndexRef.current + 1 < passages.length) {
            const completedIndex = currentIndexRef.current;
            const alignment = pendingAlignmentsRef.current.get(clipKey(completedIndex)) ?? Promise.resolve();
            void alignment.finally(() => startAtRef.current(completedIndex + 1));
          } else {
            setAudioProgress(1);
            setStatus("complete");
            onPassageChange(null);
            void releaseLocalSpeechMemory().catch(() => undefined);
          }
        }}
      />
      <div className="speech-player-heading">
        <span className="speech-player-icon"><Volume2 size={17} /></span>
        <span><strong>Listen</strong><small>{statusText}</small></span>
      </div>
      <div className="speech-transport" aria-label="Speech controls">
        <button type="button" aria-label="Previous passage" title="Previous passage" disabled={unavailable || status === "preparing" || currentIndex === 0} onClick={() => navigateTo(currentIndexRef.current - 1)}><SkipBack size={15} /></button>
        <button type="button" className="speech-play" aria-label={status === "playing" ? "Pause report" : "Play report"} disabled={unavailable || status === "preparing"} onClick={togglePlayback}>{status === "playing" ? <Pause size={16} /> : <Play size={16} />}</button>
        <button type="button" aria-label="Stop report" disabled={unavailable || !["preparing", "playing", "paused"].includes(status)} onClick={stopPlayback}><Square size={13} /></button>
        <button type="button" aria-label="Next passage" title="Next passage" disabled={unavailable || status === "preparing" || currentIndex >= passages.length - 1} onClick={() => navigateTo(currentIndexRef.current + 1)}><SkipForward size={15} /></button>
      </div>
      <div className="speech-options">
        <label>Read
          <select aria-label="Reading length" value={scope} disabled={status === "preparing"} onChange={(event) => chooseScope(event.target.value as ResearchSpeechScope)}>
            <option value="summary">Summary</option>
            <option value="article">Article</option>
            <option value="all">Article + sources</option>
          </select>
        </label>
        <label>ComfyUI voice
          <select aria-label="ComfyUI voice model" value={selectedModel?.id ?? ""} disabled={unavailable || ["preparing", "playing", "paused"].includes(status)} onChange={(event) => chooseModel(event.target.value)}>
            {!snapshot?.voices.length && <option value="">No local TTS model</option>}
            {snapshot?.voices.map((model) => <option value={model.id} key={model.id}>{model.name}</option>)}
          </select>
        </label>
        <label>Playback speed
          <select aria-label="Playback speed" value={rate} disabled={unavailable} onChange={(event) => chooseRate(Number(event.currentTarget.value))}>
            <option value={0.8}>0.8x</option>
            <option value={1}>1x</option>
            <option value={1.15}>1.15x</option>
            <option value={1.3}>1.3x</option>
            <option value={1.5}>1.5x</option>
          </select>
        </label>
      </div>
      <div className="speech-progress" role="progressbar" aria-label="Report speech progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progress)}>
        <span style={{ width: `${progress}%` }} />
      </div>
      {["playing", "paused"].includes(status) && activePassage && <div className="speech-current-passage">
        <input type="range" aria-label="Current passage position" min={0} max={Math.max(speechDuration, 0.01)} step={0.01} value={Math.min(speechSeconds, Math.max(speechDuration, 0.01))} onChange={(event) => seekSpeech(event.currentTarget.valueAsNumber)} />
        <SpeechLiveCaption text={activePassage.text} seconds={speechSeconds} duration={speechDuration} timings={speechTimings} onSeek={seekSpeech} />
      </div>}
      <div className="speech-player-meta">
        <span>{error ?? detail}</span>
        {!unavailable && <span>{bufferingIndex !== null ? `Buffering ${bufferingIndex + 1}` : `${Math.min(currentIndex + 1, passages.length)} / ${passages.length}`}</span>}
      </div>
      {error && <p className="speech-error" role="alert">ComfyUI narration stopped: {error}</p>}
    </section>
  );
}
