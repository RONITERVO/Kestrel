import {
  Bot, Captions, ChevronLeft, CircleStop, Clock3, FileText, Languages, ListMusic, LoaderCircle, Pause, Play,
  Palette, Plus, Save, Sparkles, Trash2, Wand2, WandSparkles,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { getLocalSpeechSnapshot } from "../../../platform/api";
import {
  applyMusicLyricFrameStyles,
  MusicLyricReactivity,
  type MusicLyricBounds,
  type MusicLyricLayout,
} from "./MusicLyricReactivity";
import {
  clampFiniteMusicLyricTime as clampFinite,
  estimatedMusicLyricWords as estimatedWords,
  extractLyricsForRange,
  formatMusicLyricTime as formatTime,
  formatPreciseMusicLyricTime as formatPreciseTime,
  musicLyricDisplaySegmentAt,
  musicLyricSegmentAt,
  newMusicLyricId as stableId,
  reconcileMusicLyricWords as reconcileTimedWords,
  roundMusicLyricTime as roundTime,
  truncateUtf8,
  utf8ByteLength,
  wordProgress,
} from "./MusicLyricsTiming";
import { createMusicLyricVisualizer, MUSIC_LYRIC_THEMES, type MusicLyricRenderer } from "./MusicLyricVisualizers";
import type {
  ModelInfo, MusicLyricSegment, MusicLyricsDocument, MusicLyricWord, MusicProject, MusicTake, SpeechModel,
} from "../../../contracts/index";

export { musicLyricDisplaySegmentAt, musicLyricSegmentAt, wordProgress } from "./MusicLyricsTiming";

interface AudioAnalysis {
  context: AudioContext;
  analyser: AnalyserNode;
  frequency: Uint8Array;
  time: Uint8Array;
}

const analyses = new WeakMap<HTMLMediaElement, AudioAnalysis>();

const LANGUAGE_PRESETS = [
  "English", "Spanish", "French", "German", "Italian", "Portuguese",
  "Japanese", "Chinese", "Korean", "Arabic", "Russian", "Hindi", "Dutch", "Swedish",
];

export function MusicLyricsProducer({
  project,
  take,
  document,
  audio,
  currentTime,
  playing,
  busy,
  speechBusy = false,
  status,
  models,
  activeModelId,
  onTogglePlay,
  onSeek,
  onChange,
  onSave,
  onSync,
  onRepairRange,
  onDraftAudioPrompt,
  onTranslateLyrics,
  onCancelSync,
  onClose,
}: {
  project: MusicProject;
  take: MusicTake;
  document: MusicLyricsDocument;
  audio: HTMLAudioElement | null;
  currentTime: number;
  playing: boolean;
  busy: boolean;
  speechBusy?: boolean;
  status: string;
  models?: ModelInfo[];
  activeModelId?: string;
  onTogglePlay: () => void;
  onSeek: (seconds: number) => void;
  onChange: (document: MusicLyricsDocument) => void;
  onSave: (document: MusicLyricsDocument) => Promise<MusicLyricsDocument | undefined>;
  onSync: (modelId: string, language: string) => Promise<void>;
  onRepairRange?: (modelId: string, language: string, startSeconds: number, endSeconds: number, prompt: string) => Promise<void>;
  onDraftAudioPrompt?: (startSeconds: number, endSeconds: number) => Promise<{ transcription: string; modelId: string; modelName: string }>;
  onTranslateLyrics?: (targetLanguage: string, lines: string[]) => Promise<{ translations: string[]; modelId: string; modelName: string }>;
  onCancelSync: () => void;
  onClose: () => void;
}) {
  const producerRef = useRef<HTMLElement>(null);
  const documentRef = useRef(document);
  documentRef.current = document;
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const primaryRef = useRef<HTMLDivElement>(null);
  const translationRef = useRef<HTMLDivElement>(null);
  const currentTimeRef = useRef(currentTime);
  const [editing, setEditing] = useState(false);
  const [editorTab, setEditorTab] = useState<"cues" | "repair">("cues");
  const [selectedId, setSelectedId] = useState(document.segments[0]?.id ?? "");
  const [transcribers, setTranscribers] = useState<SpeechModel[]>([]);
  const [modelId, setModelId] = useState("");
  const [language, setLanguage] = useState(document.language || "auto");
  const [speechDetail, setSpeechDetail] = useState("Checking local Whisper…");
  const [savedRevision, setSavedRevision] = useState(document.revision);
  const [savedTheme, setSavedTheme] = useState(document.theme);
  const [repairStart, setRepairStart] = useState(0);
  const [repairEnd, setRepairEnd] = useState(Math.min(take.durationSeconds, 10));
  const [repairPrompt, setRepairPrompt] = useState("");
  const [audioDraftBusy, setAudioDraftBusy] = useState(false);
  const [audioDraftStatus, setAudioDraftStatus] = useState("");
  const [targetLanguage, setTargetLanguage] = useState(document.translationLanguage || "Spanish");
  const [translationBusy, setTranslationBusy] = useState(false);
  const [translationStatus, setTranslationStatus] = useState("");
  const [dirty, setDirty] = useState(false);
  const activeSegment = musicLyricSegmentAt(document.segments, currentTime);
  const displaySegment = musicLyricDisplaySegmentAt(document.segments, currentTime);
  const cueExiting = Boolean(displaySegment && displaySegment !== activeSegment);
  const selectedSegment = document.segments.find((segment) => segment.id === selectedId);
  const takeNumber = project.takes.findIndex((candidate) => candidate.id === take.id) + 1;
  const audioModel = useMemo(() => {
    if (activeModelId && models) {
      const active = models.find((m) => m.id === activeModelId);
      if (active?.supportsAudio) return active;
    }
    return models?.find((m) => m.supportsAudio);
  }, [models, activeModelId]);

  useEffect(() => {
    currentTimeRef.current = currentTime;
  }, [currentTime]);

  useEffect(() => {
    let disposed = false;
    void getLocalSpeechSnapshot()
      .then((snapshot) => {
        if (disposed) return;
        const available = snapshot.transcriptionAvailable ? snapshot.transcribers : [];
        setTranscribers(available);
        setModelId((current) => current || available[0]?.id || "");
        setSpeechDetail(snapshot.transcriptionAvailable
          ? "Whisper is installed for private, word-level lyric sync."
          : snapshot.detail || "Install Local voice and dictation in Setup to sync sung words automatically.");
      })
      .catch((error) => !disposed && setSpeechDetail(String(error)));
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    setLanguage(document.language || "auto");
    if (document.translationLanguage) setTargetLanguage(document.translationLanguage);
    setSelectedId((current) => document.segments.some((segment) => segment.id === current)
      ? current
      : document.segments[0]?.id ?? "");
  }, [document]);

  useEffect(() => {
    setSavedRevision(document.revision);
    setSavedTheme(document.theme);
    setDirty(false);
    // updatedAt changes only when the backend returns a durable document; local previews keep it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [document.revision, document.updatedAt]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    let visualizer: MusicLyricRenderer | undefined;
    let disposed = false;
    let frame = 0;
    void createMusicLyricVisualizer(document.theme, canvas)
      .then((created) => {
        if (disposed) {
          created.destroy?.();
          return;
        }
        visualizer = created;
        const analysis = audio ? ensureAudioAnalysis(audio) : undefined;
        const reactivity = new MusicLyricReactivity();
        let layout = measureLyricLayout(canvas, primaryRef.current, translationRef.current);
        let lastLayoutAt = 0;
        const reducedMotion = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
        const draw = (now = performance.now()) => {
          if (disposed || !visualizer) return;
          if (now - lastLayoutAt >= 80) {
            layout = measureLyricLayout(canvas, primaryRef.current, translationRef.current);
            lastLayoutAt = now;
          }
          const visualFrame = reactivity.sample(
            analysis?.analyser,
            analysis?.frequency,
            analysis?.time,
            currentTimeRef.current / Math.max(0.01, take.durationSeconds),
            layout,
            now,
          );
          visualizer.draw(visualFrame);
          if (producerRef.current) applyMusicLyricFrameStyles(producerRef.current, visualFrame);
          if (!reducedMotion) frame = requestAnimationFrame(draw);
        };
        draw();
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      visualizer?.destroy?.();
    };
  }, [audio, document.theme, take.durationSeconds]);

  useEffect(() => {
    if (!playing || !audio) return;
    const analysis = ensureAudioAnalysis(audio);
    if (analysis?.context.state === "suspended") void analysis.context.resume().catch(() => undefined);
  }, [audio, playing]);

  const upcoming = useMemo(
    () => document.segments.find((segment) => segment.start > currentTime),
    [currentTime, document.segments],
  );

  const changeDocument = (next: MusicLyricsDocument) => {
    documentRef.current = next;
    setDirty(true);
    onChange(next);
  };

  const patchSegment = (id: string, patch: Partial<MusicLyricSegment>) => {
    const current = documentRef.current;
    const targetIndex = current.segments.findIndex((segment) => segment.id === id);
    if (targetIndex < 0) return;
    const minimumStart = current.segments[targetIndex - 1]?.end ?? 0;
    const maximumEnd = current.segments[targetIndex + 1]?.start ?? take.durationSeconds;
    const segments = current.segments.map((segment) => {
      if (segment.id !== id) return segment;
      const next = { ...segment, ...patch };
      const maximumStart = Math.max(minimumStart, Math.min(maximumEnd - 0.01, segment.end - 0.01));
      next.start = clampFinite(next.start, minimumStart, maximumStart, segment.start);
      next.end = clampFinite(next.end, next.start + 0.01, maximumEnd, segment.end);
      const primaryChanged = "primary" in patch && next.primary !== segment.primary;
      if (primaryChanged) next.translation = "";
      if (primaryChanged && !("words" in patch) && segment.words.length > 0) {
        next.words = reconcileTimedWords(next.primary, next.start, next.end, segment.words);
      }
      if (("start" in patch || "end" in patch) && !("words" in patch) && segment.words.length > 0) {
        next.words = segment.words.map((word) => ({
          ...word,
          start: Math.max(next.start, Math.min(next.end, word.start)),
          end: Math.max(next.start, Math.min(next.end, word.end)),
        }));
      }
      return next;
    });
    const hasTranslation = segments.some((segment) => segment.translation.trim());
    changeDocument({
      ...current,
      translationLanguage: hasTranslation ? current.translationLanguage : "",
      translationModelId: hasTranslation && !("translation" in patch) ? current.translationModelId : "",
      segments,
    });
  };

  const patchWord = (segmentId: string, wordIndex: number, wordPatch: Partial<MusicLyricWord>) => {
    const targetSegment = documentRef.current.segments.find((s) => s.id === segmentId);
    if (!targetSegment) return;
    const nextWords = targetSegment.words.map((w, idx) => (idx === wordIndex ? { ...w, ...wordPatch } : w));
    const timingChanged = "start" in wordPatch || "end" in wordPatch;
    let nextStart = targetSegment.start;
    let nextEnd = targetSegment.end;
    if (timingChanged) {
      nextWords.sort((a, b) => a.start - b.start);
      if (nextWords[0]) nextStart = Math.min(nextStart, nextWords[0].start);
      const lastWord = nextWords[nextWords.length - 1];
      if (lastWord) nextEnd = Math.max(nextEnd, lastWord.end);
    }
    const nextPrimary = "value" in wordPatch ? nextWords.map((w) => w.value).join(" ") : targetSegment.primary;
    patchSegment(segmentId, {
      start: nextStart,
      end: nextEnd,
      primary: nextPrimary,
      words: nextWords,
    });
  };

  const setWordStart = (segmentId: string, wordIndex: number) => {
    const targetSegment = documentRef.current.segments.find((s) => s.id === segmentId);
    if (!targetSegment || !targetSegment.words[wordIndex]) return;
    const currentWord = targetSegment.words[wordIndex];
    const previousEnd = targetSegment.words[wordIndex - 1]?.end ?? targetSegment.start;
    const newStart = roundTime(Math.max(previousEnd, Math.min(currentWord.end - 0.01, currentTime)));
    const newEnd = Math.min(targetSegment.end, Math.max(newStart + 0.01, currentWord.end));
    patchWord(segmentId, wordIndex, { start: newStart, end: newEnd });
  };

  const setWordEnd = (segmentId: string, wordIndex: number) => {
    const targetSegment = documentRef.current.segments.find((s) => s.id === segmentId);
    if (!targetSegment || !targetSegment.words[wordIndex]) return;
    const currentWord = targetSegment.words[wordIndex];
    const nextStart = targetSegment.words[wordIndex + 1]?.start ?? targetSegment.end;
    const newEnd = roundTime(Math.max(currentWord.start + 0.01, Math.min(nextStart, currentTime)));
    const newStart = Math.max(targetSegment.start, Math.min(newEnd - 0.01, currentWord.start));
    patchWord(segmentId, wordIndex, { start: newStart, end: newEnd });
  };

  const addWordToSegment = (segmentId: string) => {
    const targetSegment = documentRef.current.segments.find((s) => s.id === segmentId);
    if (!targetSegment) return;
    const wordStart = roundTime(Math.max(targetSegment.start, Math.min(targetSegment.end, currentTime)));
    const wordEnd = roundTime(Math.min(targetSegment.end, wordStart + 0.5));
    const newWord: MusicLyricWord = {
      value: "word",
      start: wordStart,
      end: wordEnd,
    };
    const nextWords = [...targetSegment.words, newWord].sort((a, b) => a.start - b.start);
    patchSegment(segmentId, {
      words: nextWords,
      primary: nextWords.map((w) => w.value).join(" "),
    });
  };

  const removeWordFromSegment = (segmentId: string, wordIndex: number) => {
    const targetSegment = documentRef.current.segments.find((s) => s.id === segmentId);
    if (!targetSegment) return;
    const nextWords = targetSegment.words.filter((_, idx) => idx !== wordIndex);
    patchSegment(segmentId, {
      words: nextWords,
      primary: nextWords.length > 0 ? nextWords.map((w) => w.value).join(" ") : targetSegment.primary,
    });
  };

  const splitWordsFromPrimary = (segmentId: string) => {
    const targetSegment = documentRef.current.segments.find((s) => s.id === segmentId);
    if (!targetSegment) return;
    const words = estimatedWords(targetSegment.primary, targetSegment.start, targetSegment.end);
    patchSegment(segmentId, { words });
  };

  const addCue = () => {
    const start = Math.max(0, Math.min(Math.max(0, take.durationSeconds - 0.02), currentTime));
    const segment: MusicLyricSegment = {
      id: stableId(),
      start,
      end: Math.min(take.durationSeconds, start + 3),
      primary: "New lyric cue",
      translation: "",
      words: [],
    };
    const current = documentRef.current;
    changeDocument({ ...current, segments: [...current.segments, segment].sort((left, right) => left.start - right.start) });
    setSelectedId(segment.id);
  };

  const removeCue = (id: string) => {
    const current = documentRef.current;
    const index = current.segments.findIndex((segment) => segment.id === id);
    const next = current.segments.filter((segment) => segment.id !== id);
    changeDocument({ ...current, segments: next });
    setSelectedId(next[Math.min(index, next.length - 1)]?.id ?? "");
  };

  const handleTranslateAll = async () => {
    const requestedDocument = documentRef.current;
    if (!onTranslateLyrics || !requestedDocument.segments.length) return;
    const target = targetLanguage.trim() || "Spanish";
    const requested = requestedDocument.segments.map((segment) => ({ id: segment.id, primary: segment.primary }));
    setTranslationBusy(true);
    setTranslationStatus(`Translating ${requested.length} cues into ${target}…`);
    try {
      const lines = requested.map((segment) => segment.primary);
      const res = await onTranslateLyrics(target, lines);
      const translations = new Map(requested.map((segment, index) => [segment.id, {
        primary: segment.primary,
        translation: res.translations[index]?.trim() || "",
      }]));
      const latest = documentRef.current;
      let applied = 0;
      const updatedSegments = latest.segments.map((segment) => {
        const candidate = translations.get(segment.id);
        if (!candidate || candidate.primary !== segment.primary || !candidate.translation) return segment;
        applied += 1;
        return { ...segment, translation: candidate.translation };
      });
      if (applied > 0) {
        changeDocument({
          ...latest,
          showTranslation: true,
          translationLanguage: target,
          translationModelId: res.modelId,
          segments: updatedSegments,
        });
      }
      const skipped = requested.length - applied;
      setTranslationStatus(`Translated ${applied} cue${applied === 1 ? "" : "s"} into ${target} with ${res.modelName}.${skipped ? ` Preserved ${skipped} cue${skipped === 1 ? "" : "s"} edited while the model was working.` : ""}`);
    } catch (err) {
      setTranslationStatus(`Translation error: ${String(err)}`);
    } finally {
      setTranslationBusy(false);
    }
  };

  const handleTranslateCue = async (segmentId: string, primaryText: string) => {
    if (!onTranslateLyrics || !primaryText.trim()) return;
    const target = targetLanguage.trim() || "Spanish";
    setTranslationBusy(true);
    setTranslationStatus(`Translating cue into ${target}…`);
    try {
      const res = await onTranslateLyrics(target, [primaryText]);
      const trans = res.translations[0]?.trim() || "";
      if (trans) {
        const latest = documentRef.current;
        const matchingCue = latest.segments.find((segment) => segment.id === segmentId);
        if (!matchingCue || matchingCue.primary !== primaryText) {
          setTranslationStatus("The cue changed while the model was working, so its newer text was preserved.");
          return;
        }
        changeDocument({
          ...latest,
          showTranslation: true,
          translationLanguage: target,
          translationModelId: res.modelId,
          segments: latest.segments.map((segment) => segment.id === segmentId
            ? { ...segment, translation: trans }
            : segment),
        });
        setTranslationStatus(`Cue translated into ${target} with ${res.modelName}.`);
      }
    } catch (err) {
      setTranslationStatus(`Translation error: ${String(err)}`);
    } finally {
      setTranslationBusy(false);
    }
  };

  const handleTogglePlay = () => {
    const analysis = audio ? ensureAudioAnalysis(audio) : undefined;
    if (analysis?.context.state === "suspended") void analysis.context.resume();
    onTogglePlay();
  };

  const handleStageClick = (event: React.MouseEvent) => {
    if ((event.target as HTMLElement).closest("[data-lyric-control]")) return;
    handleTogglePlay();
  };

  const saveCurrentDocument = async (): Promise<boolean> => {
    const saved = await onSave(documentRef.current);
    if (!saved) return false;
    documentRef.current = saved;
    setSavedRevision(saved.revision);
    setSavedTheme(saved.theme);
    setDirty(false);
    return true;
  };

  const closeProducer = async () => {
    if (busy) return;
    if (dirty && !(await saveCurrentDocument())) return;
    onClose();
  };

  return (
    <section ref={producerRef} className={`music-lyrics-producer theme-${document.theme} ${editing ? "editing" : ""}`} aria-label="Visual lyric producer">
      <canvas ref={canvasRef} className="music-lyrics-canvas" aria-hidden="true" />
      <div className="music-lyrics-paper" aria-hidden="true" />

      <header className="music-lyrics-header" data-lyric-control>
        <button aria-label="Close visual lyric producer" disabled={busy} onClick={() => void closeProducer()}><ChevronLeft /> {dirty ? "Save & return" : "Back to arranger"}</button>
        <div className="music-lyrics-title"><small>Kestrel visual lyrics · Take {takeNumber}</small><strong>{project.title}</strong></div>
        <div className="music-lyrics-header-actions">
          <label className="music-lyrics-theme-picker" title={MUSIC_LYRIC_THEMES.find((theme) => theme.id === document.theme)?.description}>
            <Palette /><span>Visual</span>
            <select aria-label="Lyric visual theme" disabled={busy} value={document.theme} onChange={(event) => changeDocument({ ...documentRef.current, theme: event.currentTarget.value as MusicLyricsDocument["theme"] })}>
              {MUSIC_LYRIC_THEMES.map((theme) => <option key={theme.id} value={theme.id}>{theme.name}</option>)}
            </select>
          </label>
          {document.theme !== savedTheme && <button className="music-lyrics-save-look" disabled={busy} onClick={() => void saveCurrentDocument()}><Save /> Save look</button>}
              <span><Captions /> Revision {document.revision}{dirty ? " · unsaved" : ""} · {document.source === "producer-timing-draft" ? "timing draft" : "local sync"}</span>
        </div>
      </header>

      <div className="music-lyrics-stage" onClick={handleStageClick}>
        <div className="music-lyrics-stage-meta"><span>{take.resolvedModel || "Kestrel Music"}</span><strong>{formatPreciseTime(currentTime)}</strong></div>
        <div className="music-lyrics-copy" data-lyric-control data-cue-state={cueExiting ? "exiting" : displaySegment ? "active" : "instrumental"}>
          <div key={displaySegment?.id ?? (upcoming ? "instrumental" : "ending")} className={`music-lyrics-cue ${cueExiting ? "exiting" : "entering"}`}>
            <div ref={primaryRef} className="music-lyrics-primary">
              {displaySegment
                ? renderTimedWords(displaySegment, currentTime, (seconds) => {
                  onSeek(seconds);
                  if (!playing) handleTogglePlay();
                })
                : <span className="music-lyrics-placeholder">{document.segments.length ? (upcoming ? "( Instrumental )" : "End of page") : "( Instrumental )"}</span>}
            </div>
            {document.showTranslation && displaySegment?.translation && <div ref={translationRef} className="music-lyrics-translation">{renderProgressiveText(displaySegment.translation, displaySegment.start, displaySegment.end, currentTime)}</div>}
          </div>
        </div>
      </div>

      <footer className="music-lyrics-transport" data-lyric-control>
        <button className="music-lyrics-play" aria-label={playing ? "Pause lyric stage" : "Play lyric stage"} onClick={handleTogglePlay}>{playing ? <Pause /> : <Play />}</button>
        <span className="music-lyrics-clock">{formatTime(currentTime)}</span>
        <input aria-label="Visual lyric timeline" type="range" min={0} max={Math.max(0.01, take.durationSeconds)} step={0.01} value={Math.min(currentTime, take.durationSeconds)} onChange={(event) => onSeek(event.currentTarget.valueAsNumber)} />
        <span className="music-lyrics-clock">{formatTime(take.durationSeconds)}</span>
        <button className={editing ? "active" : ""} onClick={() => setEditing((value) => !value)}><Clock3 /> Edit timing</button>
      </footer>

      {editing && <aside className="music-lyrics-editor" data-lyric-control>
        <header><div><small>Durable take document</small><strong>Lyrics & timing</strong></div><button aria-label="Close lyric timing editor" onClick={() => setEditing(false)}>×</button></header>
        <div className="music-lyrics-tab-bar">
          <button type="button" className={editorTab === "cues" ? "active" : ""} onClick={() => setEditorTab("cues")}><ListMusic /> Cues & Words</button>
          <button type="button" className={editorTab === "repair" ? "active" : ""} onClick={() => {
            setEditorTab("repair");
            if (selectedSegment) {
              setRepairStart(roundTime(selectedSegment.start));
              setRepairEnd(roundTime(selectedSegment.end));
              setRepairPrompt(truncateUtf8(selectedSegment.primary, 512));
            }
          }}><Wand2 /> Repair with Whisper</button>
        </div>

        {editorTab === "repair" ? (
          <section className="music-lyrics-range-repair">
            <div className="music-lyrics-repair-guide">
              <Sparkles />
              <span>
                <strong>Targeted, prompt-guided Whisper transcription</strong>
                <small>Select a time range and prompt Whisper with expected words. 1.5s audio buffers prevent boundary clipping.</small>
              </span>
            </div>

            <div className="music-lyrics-time-fields">
              <div className="music-lyrics-time-card">
                <label>
                  <span>Range Start ({formatTime(repairStart)})</span>
                  <input
                    type="number"
                    min={0}
                    max={take.durationSeconds}
                    step={0.01}
                    value={repairStart}
                    onChange={(e) => setRepairStart(clampFinite(e.currentTarget.valueAsNumber, 0, take.durationSeconds, repairStart))}
                  />
                </label>
                <button
                  type="button"
                  className="music-lyrics-set-btn"
                  title="Set range start to playhead position"
                  onClick={() => setRepairStart(roundTime(currentTime))}
                >
                  <Clock3 /> Set Start ({formatPreciseTime(currentTime)})
                </button>
              </div>

              <div className="music-lyrics-time-card">
                <label>
                  <span>Range End ({formatTime(repairEnd)})</span>
                  <input
                    type="number"
                    min={0.01}
                    max={take.durationSeconds}
                    step={0.01}
                    value={repairEnd}
                    onChange={(e) => setRepairEnd(clampFinite(e.currentTarget.valueAsNumber, 0.01, take.durationSeconds, repairEnd))}
                  />
                </label>
                <button
                  type="button"
                  className="music-lyrics-set-btn"
                  title="Set range end to playhead position"
                  onClick={() => setRepairEnd(roundTime(currentTime))}
                >
                  <Clock3 /> Set End ({formatPreciseTime(currentTime)})
                </button>
              </div>
            </div>

            {selectedSegment && (
              <button
                type="button"
                className="music-lyrics-btn-sm music-lyrics-use-cue-btn"
                onClick={() => {
                  setRepairStart(roundTime(selectedSegment.start));
                  setRepairEnd(roundTime(selectedSegment.end));
                  setRepairPrompt(truncateUtf8(selectedSegment.primary, 512));
                }}
              >
                <Clock3 /> Use Cue #{document.segments.findIndex((s) => s.id === selectedSegment.id) + 1} ({formatTime(selectedSegment.start)} – {formatTime(selectedSegment.end)})
              </button>
            )}

            <label className="music-lyrics-field">
              <div className="music-lyrics-prompt-header">
                <span>Start prompt / Expected lyrics</span>
                <small>{utf8ByteLength(repairPrompt)} / 512 bytes</small>
              </div>
              <textarea
                value={repairPrompt}
                rows={3}
                onChange={(e) => setRepairPrompt(truncateUtf8(e.target.value, 512))}
                placeholder="Type or paste the exact expected sung words for this section…"
              />
            </label>

            <div className="music-lyrics-prompt-fill-actions">
              <span>Fill prompt from:</span>
              <button
                type="button"
                className="music-lyrics-btn-sm"
                title="Extract matching lines from generated take lyrics"
                onClick={() => {
                  const extracted = extractLyricsForRange(take.lyrics || project.caption, repairStart, repairEnd, take.durationSeconds);
                  if (extracted) setRepairPrompt(truncateUtf8(extracted, 512));
                }}
              >
                <FileText /> Take lyrics
              </button>
              {selectedSegment && (
                <button
                  type="button"
                  className="music-lyrics-btn-sm"
                  title="Use selected cue text"
                  onClick={() => setRepairPrompt(truncateUtf8(selectedSegment.primary, 512))}
                >
                  <Sparkles /> Current cue
                </button>
              )}
              {onDraftAudioPrompt && audioModel && (
                <button
                  type="button"
                  className="music-lyrics-btn-sm music-lyrics-copilot-btn"
                  disabled={audioDraftBusy || busy}
                  title={`Listen to audio slice using ${audioModel.name} (native audio model)`}
                  onClick={async () => {
                    setAudioDraftBusy(true);
                    setAudioDraftStatus("Audio model is listening to the slice…");
                    try {
                      const res = await onDraftAudioPrompt(repairStart, repairEnd);
                      if (res.transcription) {
                        setRepairPrompt(truncateUtf8(res.transcription, 512));
                        setAudioDraftStatus(`Transcribed by ${res.modelName}: "${res.transcription}"`);
                      } else {
                        setAudioDraftStatus(`${res.modelName} finished without detected vocal words.`);
                      }
                    } catch (err) {
                      setAudioDraftStatus(`Audio Copilot error: ${String(err)}`);
                    } finally {
                      setAudioDraftBusy(false);
                    }
                  }}
                >
                  {audioDraftBusy ? <LoaderCircle className="spin" /> : <Bot />} Audio Copilot ({audioModel.name})
                </button>
              )}
              {onDraftAudioPrompt && !audioModel && <small>Audio Copilot needs a local model with native audio input.</small>}
            </div>

            {audioDraftStatus && (
              <div className="music-lyrics-copilot-status">
                {audioDraftBusy && <LoaderCircle className="spin" />} {audioDraftStatus}
              </div>
            )}

            <div className="music-lyrics-repair-model-row">
              <label>
                <span>Whisper Model</span>
                <select disabled={busy || !transcribers.length} value={modelId} onChange={(e) => setModelId(e.target.value)}>
                  <option value="">Not installed</option>
                  {transcribers.map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
                </select>
              </label>
              <label>
                <span>Language</span>
                <input disabled={busy} value={language} maxLength={64} onChange={(e) => setLanguage(e.target.value)} placeholder="auto" />
              </label>
            </div>

            <div className="music-lyrics-range-repair-actions">
              {speechBusy ? (
                <button type="button" className="danger" onClick={onCancelSync}>
                  <CircleStop /> Stop safely
                </button>
              ) : (
                <button
                  type="button"
                  className="primary-button music-lyrics-repair-submit-btn"
                  disabled={busy || !modelId || !Number.isFinite(repairStart) || !Number.isFinite(repairEnd) || repairEnd <= repairStart || utf8ByteLength(repairPrompt) > 512}
                  onClick={() => {
                    if (onRepairRange && modelId) {
                      void onRepairRange(modelId, language.trim() || "auto", repairStart, repairEnd, repairPrompt.trim());
                    }
                  }}
                >
                  <WandSparkles /> Re-sync range with Whisper
                </button>
              )}
              {status && <p role="status">{busy && <LoaderCircle className="spin" />} {status}</p>}
            </div>
          </section>
        ) : (
          <>
            <section className="music-lyrics-sync">
              <div><Sparkles /><span><strong>Local word sync</strong><small>{speechDetail}</small></span></div>
              <label>Whisper model<select aria-label="Lyric transcription model" disabled={busy || !transcribers.length} value={modelId} onChange={(event) => setModelId(event.target.value)}><option value="">Not installed</option>{transcribers.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}</select></label>
              <label>Language<input aria-label="Lyric transcription language" disabled={busy} value={language} maxLength={64} onChange={(event) => setLanguage(event.target.value)} placeholder="auto" /></label>
              {speechBusy
                ? <button className="danger" onClick={onCancelSync}><CircleStop /> Stop safely</button>
                : <button disabled={busy || !modelId} onClick={() => void onSync(modelId, language.trim() || "auto")}><WandSparkles /> Sync this take</button>}
              {status && <p role="status">{busy && <LoaderCircle className="spin" />} {status}</p>}
            </section>
            <div className="music-lyrics-editor-actions">
              <button onClick={addCue}><Plus /> Add cue</button>
              <label className="music-lyrics-translation-toggle">
                <input type="checkbox" checked={document.showTranslation} onChange={(event) => changeDocument({ ...documentRef.current, showTranslation: event.target.checked })} />
                <Languages /> Show subtitles
              </label>
              {onTranslateLyrics && (
                <div className="music-lyrics-translation-bar">
                  <select
                    aria-label="Target translation language"
                    disabled={translationBusy || busy}
                    value={targetLanguage}
                    onChange={(e) => setTargetLanguage(e.target.value)}
                    className="music-lyrics-lang-select"
                    title="Select target language for local AI translation"
                  >
                    {LANGUAGE_PRESETS.map((l) => <option key={l} value={l}>{l}</option>)}
                  </select>
                  <button
                    type="button"
                    className="music-lyrics-btn-sm music-lyrics-translate-all-btn"
                    disabled={translationBusy || busy || !document.segments.length}
                    title={`Translate all ${document.segments.length} cues into ${targetLanguage} using local AI`}
                    onClick={handleTranslateAll}
                  >
                    {translationBusy ? <LoaderCircle className="spin" /> : <Languages />} Translate all
                  </button>
                </div>
              )}
            </div>
            {translationStatus && (
              <div className="music-lyrics-translation-status">
                {translationBusy && <LoaderCircle className="spin" />} {translationStatus}
              </div>
            )}
            <div className="music-lyrics-cue-list">
              {document.segments.map((segment, index) => <button key={segment.id} className={`${segment.id === selectedId ? "selected" : ""} ${segment.id === activeSegment?.id ? "active" : ""}`} onClick={() => { setSelectedId(segment.id); onSeek(segment.start); }}><span>{index + 1}</span><strong>{segment.primary}</strong><small>{formatTime(segment.start)} – {formatTime(segment.end)}</small></button>)}
              {!document.segments.length && <p>No vocal cues yet. Add one at the playhead or run local word sync.</p>}
            </div>
            {selectedSegment && (
              <fieldset disabled={busy} className="music-lyrics-cue-editor">
                <div className="music-lyrics-cue-editor-header">
                  <legend>
                    Cue {document.segments.findIndex((s) => s.id === selectedSegment.id) + 1} of {document.segments.length}
                  </legend>
                  <div className="music-lyrics-cue-quick-actions">
                    <button
                      type="button"
                      className="music-lyrics-preview-btn"
                      title="Preview cue from start"
                      onClick={() => {
                        onSeek(selectedSegment.start);
                        if (!playing) handleTogglePlay();
                      }}
                    >
                      <Play /> Play cue
                    </button>
                    <button
                      type="button"
                      className="danger music-lyrics-remove-cue-btn"
                      title="Remove this cue"
                      onClick={() => removeCue(selectedSegment.id)}
                    >
                      <Trash2 />
                    </button>
                  </div>
                </div>

                <div className="music-lyrics-time-fields">
                  <div className="music-lyrics-time-card">
                    <label>
                      <span>Start ({formatTime(selectedSegment.start)})</span>
                      <input
                        type="number"
                        min={0}
                        max={take.durationSeconds}
                        step={0.01}
                        value={roundTime(selectedSegment.start)}
                        onChange={(e) => patchSegment(selectedSegment.id, { start: e.currentTarget.valueAsNumber })}
                      />
                    </label>
                    <button
                      type="button"
                      className="music-lyrics-set-btn"
                      title="Set cue start to playhead position"
                      onClick={() => patchSegment(selectedSegment.id, { start: roundTime(currentTime) })}
                    >
                      <Clock3 /> Set Start ({formatPreciseTime(currentTime)})
                    </button>
                  </div>

                  <div className="music-lyrics-time-card">
                    <label>
                      <span>End ({formatTime(selectedSegment.end)})</span>
                      <input
                        type="number"
                        min={0.01}
                        max={take.durationSeconds}
                        step={0.01}
                        value={roundTime(selectedSegment.end)}
                        onChange={(e) => patchSegment(selectedSegment.id, { end: e.currentTarget.valueAsNumber })}
                      />
                    </label>
                    <button
                      type="button"
                      className="music-lyrics-set-btn"
                      title="Set cue end to playhead position"
                      onClick={() => patchSegment(selectedSegment.id, { end: roundTime(currentTime) })}
                    >
                      <Clock3 /> Set End ({formatPreciseTime(currentTime)})
                    </button>
                  </div>
                </div>

                <label className="music-lyrics-field">
                  <span>Primary lyric</span>
                  <textarea
                    value={selectedSegment.primary}
                    rows={2}
                    onChange={(e) => patchSegment(selectedSegment.id, { primary: e.target.value })}
                    placeholder="Lyric line text…"
                  />
                </label>

                {document.showTranslation && (
                  <div className="music-lyrics-field">
                    <div className="music-lyrics-field-header">
                      <span>Translation ({targetLanguage})</span>
                      {onTranslateLyrics && (
                        <button
                          type="button"
                          className="music-lyrics-btn-xs"
                          disabled={translationBusy || busy || !selectedSegment.primary.trim()}
                          title={`Translate this cue into ${targetLanguage} using local AI`}
                          onClick={() => handleTranslateCue(selectedSegment.id, selectedSegment.primary)}
                        >
                          {translationBusy ? <LoaderCircle className="spin" /> : <Languages />} Translate cue
                        </button>
                      )}
                    </div>
                    <textarea
                      value={selectedSegment.translation}
                      rows={2}
                      onChange={(e) => patchSegment(selectedSegment.id, { translation: e.target.value })}
                      placeholder={`Translated lyric in ${targetLanguage}…`}
                    />
                  </div>
                )}

                <div className="music-lyrics-words-panel">
                  <div className="music-lyrics-words-panel-header">
                    <strong>Word Timings ({selectedSegment.words.length})</strong>
                    <div className="music-lyrics-words-actions">
                      <button
                        type="button"
                        className="music-lyrics-btn-sm"
                        title="Add word at current playhead"
                        onClick={() => addWordToSegment(selectedSegment.id)}
                      >
                        <Plus /> Add word
                      </button>
                      {selectedSegment.words.length === 0 && selectedSegment.primary.trim().length > 0 && (
                        <button
                          type="button"
                          className="music-lyrics-btn-sm"
                          title="Generate word timings from primary lyric"
                          onClick={() => splitWordsFromPrimary(selectedSegment.id)}
                        >
                          <Sparkles /> Split words
                        </button>
                      )}
                    </div>
                  </div>

                  {selectedSegment.words.length === 0 ? (
                    <div className="music-lyrics-no-words">
                      <span>No individual word timings. Add words or click Split words above.</span>
                    </div>
                  ) : (
                    <div className="music-lyrics-word-list">
                      {selectedSegment.words.map((word, wordIndex) => (
                        <div key={`${wordIndex}-${word.start}`} className="music-lyrics-word-item">
                          <div className="music-lyrics-word-main">
                            <input
                              className="music-lyrics-word-text-input"
                              value={word.value}
                              onChange={(e) => patchWord(selectedSegment.id, wordIndex, { value: e.target.value })}
                              placeholder="word"
                            />
                            <button
                              type="button"
                              className="music-lyrics-word-play-btn"
                              title={`Seek and play from "${word.value}" (${formatPreciseTime(word.start)})`}
                              onClick={() => {
                                onSeek(word.start);
                                if (!playing) handleTogglePlay();
                              }}
                            >
                              <Play /> {formatPreciseTime(word.start)} – {formatPreciseTime(word.end)}
                            </button>
                            <button
                              type="button"
                              className="music-lyrics-word-del-btn"
                              title="Remove word"
                              onClick={() => removeWordFromSegment(selectedSegment.id, wordIndex)}
                            >
                              <Trash2 />
                            </button>
                          </div>
                          <div className="music-lyrics-word-timing-bar">
                            <button
                              type="button"
                              className="music-lyrics-word-set-btn"
                              title={`Set start of "${word.value}" to playhead (${formatPreciseTime(currentTime)})`}
                              onClick={() => setWordStart(selectedSegment.id, wordIndex)}
                            >
                              Set start ({formatPreciseTime(currentTime)})
                            </button>
                            <button
                              type="button"
                              className="music-lyrics-word-set-btn"
                              title={`Set end of "${word.value}" to playhead (${formatPreciseTime(currentTime)})`}
                              onClick={() => setWordEnd(selectedSegment.id, wordIndex)}
                            >
                              Set end ({formatPreciseTime(currentTime)})
                            </button>
                          </div>
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              </fieldset>
            )}
          </>
        )}
        <footer><span>{document.segments.length} cues · saved revision {savedRevision}{dirty ? " · unsaved edits" : ""}</span><button disabled={busy || !dirty} onClick={() => void saveCurrentDocument()}><Save /> {dirty ? "Save revision" : "Saved"}</button></footer>
      </aside>}
    </section>
  );
}

function renderTimedWords(segment: MusicLyricSegment, currentTime: number, onSeek: (seconds: number) => void) {
  if (!segment.words.length) return renderProgressiveText(segment.primary, segment.start, segment.end, currentTime);
  return segment.words.map((word, index) => {
    const written = currentTime >= word.start;
    const active = currentTime >= word.start && currentTime < word.end;
    const progress = wordProgress(word.start, word.end, currentTime);
    return <span key={`${word.start}-${index}`} className="music-lyrics-word-wrap"><button className={`${written ? "written" : "waiting"} ${active ? "active" : ""}`} aria-label={`Play from ${word.value}`} onClick={(event) => { event.stopPropagation(); onSeek(word.start); }} style={{ "--word-progress": `${progress * 100}%`, "--word-hide": `${(1 - progress) * 100}%`, "--word-index": index } as React.CSSProperties}><span className="music-lyrics-word-ghost">{word.value}</span><span className="music-lyrics-word-ink" aria-hidden="true">{word.value}</span></button>{index < segment.words.length - 1 ? " " : ""}</span>;
  });
}

function renderProgressiveText(text: string, start: number, end: number, currentTime: number) {
  const words = text.trim().split(/\s+/u).filter(Boolean);
  if (!words.length) return null;
  const duration = Math.max(0.05, end - start);
  return words.map((word, index) => {
    const wordStart = start + duration * index / words.length;
    const wordEnd = start + duration * (index + 1) / words.length;
    const progress = wordProgress(wordStart, wordEnd, currentTime);
    return <span key={`${index}-${word}`} className={`music-lyrics-progressive-word ${progress > 0 ? "written" : "waiting"}`} style={{ "--word-hide": `${(1 - progress) * 100}%`, "--word-index": index } as React.CSSProperties}><span className="music-lyrics-word-ghost">{word}</span><span className="music-lyrics-word-ink" aria-hidden="true">{word}</span>{index < words.length - 1 ? " " : ""}</span>;
  });
}

function ensureAudioAnalysis(audio: HTMLMediaElement): AudioAnalysis | undefined {
  const existing = analyses.get(audio);
  if (existing) return existing;
  try {
    const AudioContextClass = window.AudioContext || (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AudioContextClass) return undefined;
    const context = new AudioContextClass();
    const analyser = context.createAnalyser();
    analyser.fftSize = 1_024;
    analyser.smoothingTimeConstant = 0.68;
    analyser.minDecibels = -92;
    analyser.maxDecibels = -12;
    const source = context.createMediaElementSource(audio);
    source.connect(analyser);
    analyser.connect(context.destination);
    const analysis = {
      context,
      analyser,
      frequency: new Uint8Array(analyser.frequencyBinCount),
      time: new Uint8Array(analyser.fftSize),
    };
    analyses.set(audio, analysis);
    return analysis;
  } catch {
    return undefined;
  }
}

function measureLyricLayout(
  canvas: HTMLCanvasElement,
  primary: HTMLElement | null,
  translation: HTMLElement | null,
): MusicLyricLayout {
  const canvasBounds = canvas.getBoundingClientRect();
  const primaryBounds = relativeBounds(primary?.getBoundingClientRect(), canvasBounds);
  const translationBounds = relativeBounds(translation?.getBoundingClientRect(), canvasBounds);
  const activeWordBounds = relativeBounds(primary?.querySelector("button.active")?.getBoundingClientRect(), canvasBounds);
  return {
    horizon: primaryBounds?.bottom ? primaryBounds.bottom + 7 : canvasBounds.height * 0.56,
    primary: primaryBounds,
    translation: translationBounds,
    activeWord: activeWordBounds,
  };
}

function relativeBounds(
  bounds: DOMRect | undefined,
  canvasBounds: DOMRect,
): MusicLyricBounds | undefined {
  if (!bounds || bounds.width <= 0 || bounds.height <= 0) return undefined;
  return {
    left: bounds.left - canvasBounds.left,
    right: bounds.right - canvasBounds.left,
    top: bounds.top - canvasBounds.top,
    bottom: bounds.bottom - canvasBounds.top,
  };
}
