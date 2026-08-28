import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  alignLocalSpeech,
  cancelLocalSpeech,
  localSpeechMediaUrl,
  onLocalSpeechProgress,
  releaseLocalSpeechMemory,
  synthesizeLocalSpeech,
} from "./api";
import type { SpeechPassage } from "./researchSpeechContent";
import { speechPlaybackEnd, type SpeechSeekPassage } from "./spokenHighlight";
import type { SpeechModel, SpeechTiming, VoiceProfile } from "./types";

export type PlayerStatus = "ready" | "preparing" | "playing" | "paused" | "complete" | "error";
export type SourceKind = "research" | "chat" | "task" | "copilot";

let activePlaybackStop: (() => void) | null = null;

export function claimPlayback(stop: () => void) {
  if (activePlaybackStop && activePlaybackStop !== stop) {
    activePlaybackStop();
  }
  activePlaybackStop = stop;
}

export function clearPlayback(stop: () => void) {
  if (activePlaybackStop === stop) activePlaybackStop = null;
}

function speechJobId(prefix = "speech"): string {
  const random = typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
  return `${prefix}-${random}`;
}

export interface UsePipelinedSpeechPlayerProps {
  sourceKind: SourceKind;
  sourceId: string;
  passages: SpeechPassage[];
  selectedVoiceModel: SpeechModel | null;
  selectedVoiceProfile: VoiceProfile | null;
  alignmentModel?: SpeechModel | null;
  playbackRate?: number;
  initialDetail?: string;
  recording?: {
    audioRelativePath: string;
    words: SpeechTiming[];
  };
  onPassageChange?: (passage: SpeechPassage | null, index: number) => void;
  onEnded?: () => void;
}

export function usePipelinedSpeechPlayer({
  sourceKind,
  sourceId,
  passages,
  selectedVoiceModel,
  selectedVoiceProfile,
  alignmentModel,
  playbackRate = 1,
  initialDetail = "Ready with local ComfyUI",
  recording,
  onPassageChange,
  onEnded,
}: UsePipelinedSpeechPlayerProps) {
  const [status, setStatus] = useState<PlayerStatus>("ready");
  const [currentIndex, setCurrentIndex] = useState(0);
  const [elapsed, setElapsed] = useState(0);
  const [audioProgress, setAudioProgress] = useState(0);
  const [speechSeconds, setSpeechSeconds] = useState(0);
  const [speechDuration, setSpeechDuration] = useState(0);
  const [speechTimings, setSpeechTimings] = useState<SpeechTiming[]>([]);
  const [timingsRevision, setTimingsRevision] = useState(0);
  const [bufferingIndex, setBufferingIndex] = useState<number | null>(null);
  const [detail, setDetail] = useState(initialDetail);
  const [error, setError] = useState<string | null>(null);

  const audioRef = useRef<HTMLAudioElement | null>(null);
  const currentIndexRef = useRef(0);
  const rateRef = useRef(playbackRate);
  const playbackGenerationRef = useRef(0);
  const activeJobsRef = useRef(new Set<string>());
  const clipUrlsRef = useRef(new Map<string, string>());
  const clipTimingsRef = useRef(new Map<string, SpeechTiming[]>());
  const clipPathsRef = useRef(new Map<string, string>());
  const pendingClipsRef = useRef(new Map<string, Promise<string>>());
  const pendingAlignmentsRef = useRef(new Map<string, Promise<void>>());
  const alignmentJobsRef = useRef(new Map<string, string>());
  const mountedRef = useRef(true);
  const ownsPlaybackRef = useRef(false);
  const startAtRef = useRef<(index: number, voiceOverride?: SpeechModel | null, profileOverride?: VoiceProfile | null) => Promise<void>>(async () => undefined);
  const onPassageChangeRef = useRef(onPassageChange);
  const onEndedRef = useRef(onEnded);
  const recordingRef = useRef(recording);
  const speechTimingsRef = useRef<SpeechTiming[]>([]);
  const metadataRef = useRef({
    sourceKind,
    sourceId,
    passages,
    selectedVoiceModel,
    selectedVoiceProfile,
    alignmentModel,
  });
  const recordingPath = recording?.audioRelativePath ?? null;

  recordingRef.current = recording;
  metadataRef.current = {
    sourceKind,
    sourceId,
    passages,
    selectedVoiceModel,
    selectedVoiceProfile,
    alignmentModel,
  };

  useEffect(() => {
    onPassageChangeRef.current = onPassageChange;
    onEndedRef.current = onEnded;
  }, [onEnded, onPassageChange]);

  useEffect(() => {
    speechTimingsRef.current = speechTimings;
  }, [speechTimings]);

  useEffect(() => {
    currentIndexRef.current = currentIndex;
    rateRef.current = playbackRate;
    if (audioRef.current && Number.isFinite(playbackRate)) {
      audioRef.current.playbackRate = playbackRate;
    }
  }, [currentIndex, playbackRate]);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    let active = true;
    void onLocalSpeechProgress((progress) => {
      if (!activeJobsRef.current.has(progress.jobId)) return;
      setDetail(progress.detail);
    }).then((unlisten) => {
      if (!active) {
        unlisten();
      } else {
        dispose = unlisten;
      }
    });
    return () => {
      active = false;
      dispose?.();
    };
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
    setSpeechTimings(recordingRef.current?.words ?? []);
    setBufferingIndex(null);
    setStatus("ready");
    setError(null);
    onPassageChangeRef.current?.(null, -1);
  }, [cancelActiveJobs]);

  const stopPlayback = useCallback(() => {
    const wasOwner = ownsPlaybackRef.current;
    ownsPlaybackRef.current = false;
    resetPlayback();
    clearPlayback(stopPlayback);
    if (wasOwner) {
      void releaseLocalSpeechMemory().catch(() => undefined);
    }
  }, [resetPlayback]);

  useEffect(() => {
    resetPlayback();
  }, [passages, recordingPath, resetPlayback, sourceId]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      playbackGenerationRef.current += 1;
      cancelActiveJobs();
      const wasOwner = ownsPlaybackRef.current;
      ownsPlaybackRef.current = false;
      clearPlayback(stopPlayback);
      if (audioRef.current && !audioRef.current.paused) audioRef.current.pause();
      if (wasOwner) {
        void releaseLocalSpeechMemory().catch(() => undefined);
      }
    };
  }, [cancelActiveJobs, stopPlayback]);

  const clipKey = useCallback((index: number, voice?: SpeechModel | null, profile?: VoiceProfile | null) => {
    const current = metadataRef.current;
    const currentRecording = recordingRef.current;
    const passage = current.passages[index];
    if (!passage) return "";
    if (currentRecording) {
      return `${current.sourceKind}:${current.sourceId}:voice-recording:${currentRecording.audioRelativePath}:${passage.id}:${passage.text}`;
    }
    const activeVoice = voice ?? current.selectedVoiceModel;
    const activeProfile = profile ?? current.selectedVoiceProfile;
    return activeVoice && activeProfile
      ? `${current.sourceKind}:${current.sourceId}:${activeVoice.id}:${activeProfile.id}:${activeProfile.performance}:${passage.id}:${passage.text}`
      : "";
  }, []);

  const publishClipTimings = useCallback((key: string, timings: SpeechTiming[]) => {
    clipTimingsRef.current.set(key, timings);
    if (mountedRef.current) setTimingsRevision((revision) => revision + 1);
  }, []);

  const ensureClip = useCallback((index: number, background: boolean, voiceOverride?: SpeechModel | null, profileOverride?: VoiceProfile | null): Promise<string> => {
    const current = metadataRef.current;
    const currentRecording = recordingRef.current;
    const passage = current.passages[index];
    if (!passage) {
      return Promise.reject(new Error("Passage not found."));
    }
    if (currentRecording) {
      const url = localSpeechMediaUrl(currentRecording.audioRelativePath);
      if (!url) throw new Error("Kestrel could not create a private URL for your voice recording.");
      const key = clipKey(index);
      clipUrlsRef.current.set(key, url);
      publishClipTimings(key, currentRecording.words);
      clipPathsRef.current.set(key, currentRecording.audioRelativePath);
      return Promise.resolve(url);
    }
    const activeVoice = voiceOverride ?? current.selectedVoiceModel;
    const activeProfile = profileOverride ?? current.selectedVoiceProfile;
    if (!activeVoice || !activeProfile) {
      return Promise.reject(new Error("No local ComfyUI speech engine and voice are selected."));
    }
    const key = clipKey(index, activeVoice, activeProfile);
    const cached = clipUrlsRef.current.get(key);
    if (cached) return Promise.resolve(cached);
    const pending = pendingClipsRef.current.get(key);
    if (pending) return pending;

    const activeJob = speechJobId("tts");
    activeJobsRef.current.add(activeJob);
    if (background) {
      setBufferingIndex(index);
    } else {
      setStatus("preparing");
      setDetail(`Generating ${passage.label} with ${activeVoice.name}...`);
      setError(null);
    }

    const promise = synthesizeLocalSpeech({
      jobId: activeJob,
      sourceKind: current.sourceKind,
      sourceId: current.sourceId,
      passageId: passage.id,
      text: passage.text,
      modelId: activeVoice.id,
      voiceProfileId: activeProfile.id,
    }).then((clip) => {
      const url = localSpeechMediaUrl(clip.relativePath);
      if (!url) throw new Error("Kestrel could not create a private URL for the generated passage.");
      clipUrlsRef.current.set(key, url);
      clipPathsRef.current.set(key, clip.relativePath);
      if (clip.words && clip.words.length > 0) {
        publishClipTimings(key, clip.words);
      }
      return url;
    }).finally(() => {
      activeJobsRef.current.delete(activeJob);
      if (pendingClipsRef.current.get(key) === promise) {
        pendingClipsRef.current.delete(key);
      }
      if (background && mountedRef.current) {
        setBufferingIndex((current) => current === index ? null : current);
      }
    });

    pendingClipsRef.current.set(key, promise);
    return promise;
  }, [clipKey, publishClipTimings]);

  const alignClip = useCallback((index: number, voiceOverride?: SpeechModel | null, profileOverride?: VoiceProfile | null): Promise<void> => {
    const current = metadataRef.current;
    const currentRecording = recordingRef.current;
    const passage = current.passages[index];
    if (!passage) return Promise.resolve();
    if (currentRecording) {
      const key = clipKey(index);
      publishClipTimings(key, currentRecording.words);
      if (mountedRef.current && clipKey(currentIndexRef.current) === key) {
        setSpeechTimings(currentRecording.words);
      }
      return Promise.resolve();
    }
    const activeVoice = voiceOverride ?? current.selectedVoiceModel;
    const activeProfile = profileOverride ?? current.selectedVoiceProfile;
    if (!activeVoice || !current.alignmentModel || !activeProfile) return Promise.resolve();
    const key = clipKey(index, activeVoice, activeProfile);
    if (clipTimingsRef.current.get(key)?.length) return Promise.resolve();
    const pending = pendingAlignmentsRef.current.get(key);
    if (pending) return pending;
    const relativePath = clipPathsRef.current.get(key);
    if (!relativePath) return Promise.resolve();

    const alignmentJob = speechJobId("speech-align");
    activeJobsRef.current.add(alignmentJob);
    alignmentJobsRef.current.set(key, alignmentJob);

    const task = alignLocalSpeech({
      jobId: alignmentJob,
      sourceKind: current.sourceKind,
      sourceId: current.sourceId,
      passageId: passage.id,
      text: passage.text,
      relativePath,
      voiceModelId: activeVoice.id,
      voiceProfileId: activeProfile.id,
      alignmentModelId: current.alignmentModel.id,
    }).then((aligned) => {
      publishClipTimings(key, aligned.words);
      if (mountedRef.current && clipKey(currentIndexRef.current, activeVoice, activeProfile) === key) {
        setSpeechTimings(aligned.words);
      }
    }).catch(() => undefined).finally(() => {
      activeJobsRef.current.delete(alignmentJob);
      if (alignmentJobsRef.current.get(key) === alignmentJob) {
        alignmentJobsRef.current.delete(key);
      }
      if (pendingAlignmentsRef.current.get(key) === task) {
        pendingAlignmentsRef.current.delete(key);
      }
    });

    pendingAlignmentsRef.current.set(key, task);
    return task;
  }, [clipKey, publishClipTimings]);

  const startAt = useCallback(async (
    index: number,
    voiceOverride?: SpeechModel | null,
    profileOverride?: VoiceProfile | null,
    startSeconds = 0,
  ) => {
    const current = metadataRef.current;
    const currentRecording = recordingRef.current;
    if (!current.passages[index]) return;
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
    setSpeechTimings(currentRecording?.words ?? []);
    setError(null);
    onPassageChangeRef.current?.(current.passages[index], index);
    ownsPlaybackRef.current = true;
    claimPlayback(stopPlayback);
    try {
      const url = await ensureClip(index, false, voiceOverride, profileOverride);
      if (!mountedRef.current || generation !== playbackGenerationRef.current) return;
      audio.src = url;
      const exactTimings = clipTimingsRef.current.get(clipKey(index, voiceOverride, profileOverride)) ?? currentRecording?.words ?? [];
      setSpeechTimings(exactTimings);
      if (Number.isFinite(startSeconds) && startSeconds > 0) {
        audio.currentTime = startSeconds;
        setSpeechSeconds(startSeconds);
      }
      audio.playbackRate = rateRef.current;
      await audio.play();
      if (!mountedRef.current || generation !== playbackGenerationRef.current) return;
      setStatus("playing");
      setDetail(
        currentRecording
          ? "Playing your voice recording."
          : `Reading ${current.passages[index].label} with ${(profileOverride ?? current.selectedVoiceProfile)?.name ?? "voice"}.`,
      );
      void alignClip(index, voiceOverride, profileOverride);
      if (!currentRecording && mountedRef.current && generation === playbackGenerationRef.current && index + 1 < current.passages.length) {
        void ensureClip(index + 1, true, voiceOverride, profileOverride).then(() => {
          void alignClip(index + 1, voiceOverride, profileOverride);
        }).catch(() => undefined);
      }
    } catch (cause) {
      if (!mountedRef.current || generation !== playbackGenerationRef.current) return;
      const message = String(cause);
      if (/stopped|cancelled/i.test(message)) {
        setStatus("ready");
      } else {
        setStatus("error");
        setError(message);
      }
      onPassageChangeRef.current?.(null, -1);
      ownsPlaybackRef.current = false;
      clearPlayback(stopPlayback);
      void releaseLocalSpeechMemory().catch(() => undefined);
    }
  }, [alignClip, clipKey, ensureClip, stopPlayback]);

  useEffect(() => {
    startAtRef.current = startAt;
  }, [startAt]);

  const navigateTo = useCallback((index: number) => {
    const key = clipKey(currentIndexRef.current);
    const alignmentJob = alignmentJobsRef.current.get(key);
    if (alignmentJob) void cancelLocalSpeech(alignmentJob);
    void startAt(index);
  }, [clipKey, startAt]);

  const togglePlayback = useCallback(() => {
    const audio = audioRef.current;
    const current = metadataRef.current;
    if (!audio || ((!current.selectedVoiceModel || !current.selectedVoiceProfile) && !recordingRef.current)) return;
    if (status === "playing") {
      audio.pause();
      setStatus("paused");
    } else if (status === "paused" && audio.src) {
      audio.playbackRate = rateRef.current;
      ownsPlaybackRef.current = true;
      claimPlayback(stopPlayback);
      void audio.play().then(() => setStatus("playing")).catch((cause) => {
        setStatus("error");
        setError(String(cause));
      });
    } else if (status !== "preparing") {
      void startAt(status === "complete" ? 0 : currentIndexRef.current);
    }
  }, [startAt, status, stopPlayback]);

  const seekSpeech = useCallback((nextSeconds: number) => {
    const audio = audioRef.current;
    if (!audio || !Number.isFinite(nextSeconds)) return;
    const maximum = Number.isFinite(audio.duration) ? audio.duration : nextSeconds;
    audio.currentTime = Math.max(0, Math.min(nextSeconds, maximum));
    setSpeechSeconds(audio.currentTime);
    setAudioProgress(maximum > 0 ? audio.currentTime / maximum : 0);
  }, []);

  const playSpeechFrom = useCallback((nextSeconds: number) => {
    const audio = audioRef.current;
    if (!audio || !audio.src || !Number.isFinite(nextSeconds)) return;
    seekSpeech(nextSeconds);
    audio.playbackRate = rateRef.current;
    ownsPlaybackRef.current = true;
    claimPlayback(stopPlayback);
    void audio.play().then(() => {
      if (mountedRef.current) setStatus("playing");
    }).catch((cause) => {
      if (!mountedRef.current) return;
      setStatus("error");
      setError(String(cause));
    });
  }, [seekSpeech, stopPlayback]);

  const playPassageFrom = useCallback((passageId: string, nextSeconds: number) => {
    if (!Number.isFinite(nextSeconds)) return;
    const index = metadataRef.current.passages.findIndex((passage) => passage.id === passageId);
    if (index < 0) return;
    if (currentIndexRef.current === index && audioRef.current?.src) {
      playSpeechFrom(nextSeconds);
      return;
    }
    void startAt(index, undefined, undefined, nextSeconds);
  }, [playSpeechFrom, startAt]);

  const seekablePassages = useMemo<SpeechSeekPassage[]>(() => {
    if (recordingPath) return [];
    return passages.flatMap((passage, index) => {
      const timings = clipTimingsRef.current.get(clipKey(index, selectedVoiceModel, selectedVoiceProfile));
      return timings?.length ? [{ passageId: passage.id, text: passage.text, timings }] : [];
    });
  }, [clipKey, passages, recordingPath, selectedVoiceModel, selectedVoiceProfile, timingsRevision]);

  const clearModelCache = useCallback(() => {
    resetPlayback();
    clipUrlsRef.current.clear();
    clipPathsRef.current.clear();
    clipTimingsRef.current.clear();
    setTimingsRevision((revision) => revision + 1);
  }, [resetPlayback]);

  const progress = useMemo(() => {
    if (status === "complete") return 100;
    if (!passages.length) return 0;
    return ((currentIndex + audioProgress) / passages.length) * 100;
  }, [audioProgress, currentIndex, passages.length, status]);

  const activePassage = passages[currentIndex] ?? null;

  const completeCurrentPassage = useCallback(() => {
    if (currentIndexRef.current + 1 < metadataRef.current.passages.length) {
      startAtRef.current(currentIndexRef.current + 1);
    } else {
      setAudioProgress(1);
      setStatus("complete");
      onPassageChangeRef.current?.(null, -1);
      onEndedRef.current?.();
      ownsPlaybackRef.current = false;
      clearPlayback(stopPlayback);
      void releaseLocalSpeechMemory().catch(() => undefined);
    }
  }, [stopPlayback]);

  const audioProps = useMemo(() => ({
    preload: "auto" as const,
    onLoadedMetadata: (event: React.SyntheticEvent<HTMLAudioElement>) => {
      const audio = event.currentTarget;
      setSpeechDuration(Number.isFinite(audio.duration) ? audio.duration : 0);
    },
    onTimeUpdate: (event: React.SyntheticEvent<HTMLAudioElement>) => {
      const audio = event.currentTarget;
      const duration = Number.isFinite(audio.duration) ? audio.duration : 0;
      const passage = metadataRef.current.passages[currentIndexRef.current];
      const playbackEnd = passage
        ? speechPlaybackEnd(passage.text, speechTimingsRef.current, duration)
        : duration;
      if (playbackEnd > 0 && playbackEnd < duration - 0.1 && audio.currentTime >= playbackEnd) {
        audio.pause();
        audio.currentTime = playbackEnd;
        setSpeechSeconds(playbackEnd);
        setSpeechDuration(playbackEnd);
        setAudioProgress(1);
        completeCurrentPassage();
        return;
      }
      setSpeechSeconds(audio.currentTime);
      setSpeechDuration(playbackEnd);
      setAudioProgress(playbackEnd > 0 ? audio.currentTime / playbackEnd : 0);
    },
    onError: (event: React.SyntheticEvent<HTMLAudioElement>) => {
      const audio = event.currentTarget;
      const mediaError = audio.error?.message || "Audio playback decoding failed";
      setStatus("error");
      setError(mediaError);
      onPassageChangeRef.current?.(null, -1);
      ownsPlaybackRef.current = false;
      clearPlayback(stopPlayback);
      void releaseLocalSpeechMemory().catch(() => undefined);
    },
    onEnded: completeCurrentPassage,
  }), [completeCurrentPassage, stopPlayback]);

  return {
    audioRef,
    audioProps,
    status,
    setStatus,
    detail,
    setDetail,
    error,
    setError,
    elapsed,
    currentIndex,
    currentPassage: activePassage,
    speechSeconds,
    speechDuration,
    audioProgress,
    progress,
    speechTimings,
    setSpeechTimings,
    bufferingIndex,
    startAt,
    togglePlayback,
    stopPlayback,
    seekSpeech,
    playPassageFrom,
    seekablePassages,
    navigateTo,
    resetPlayback,
    clearModelCache,
  };
}
