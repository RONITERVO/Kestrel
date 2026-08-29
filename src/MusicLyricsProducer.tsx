import {
  Captions, ChevronLeft, CircleStop, Clock3, Languages, LoaderCircle, Pause, Play,
  Palette, Plus, Save, Sparkles, Trash2, WandSparkles,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { getLocalSpeechSnapshot } from "./api";
import {
  applyMusicLyricFrameStyles,
  MusicLyricReactivity,
  type MusicLyricBounds,
  type MusicLyricLayout,
} from "./MusicLyricReactivity";
import { createMusicLyricVisualizer, MUSIC_LYRIC_THEMES, type MusicLyricRenderer } from "./MusicLyricVisualizers";
import type {
  MusicLyricSegment, MusicLyricsDocument, MusicProject, MusicTake, SpeechModel,
} from "./types";

interface AudioAnalysis {
  context: AudioContext;
  analyser: AnalyserNode;
  frequency: Uint8Array;
  time: Uint8Array;
}

const analyses = new WeakMap<HTMLMediaElement, AudioAnalysis>();

export function MusicLyricsProducer({
  project,
  take,
  document,
  audio,
  currentTime,
  playing,
  busy,
  status,
  onTogglePlay,
  onSeek,
  onChange,
  onSave,
  onSync,
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
  status: string;
  onTogglePlay: () => void;
  onSeek: (seconds: number) => void;
  onChange: (document: MusicLyricsDocument) => void;
  onSave: (document: MusicLyricsDocument) => Promise<MusicLyricsDocument | undefined>;
  onSync: (modelId: string, language: string) => Promise<void>;
  onCancelSync: () => void;
  onClose: () => void;
}) {
  const producerRef = useRef<HTMLElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const primaryRef = useRef<HTMLDivElement>(null);
  const translationRef = useRef<HTMLDivElement>(null);
  const currentTimeRef = useRef(currentTime);
  const [editing, setEditing] = useState(false);
  const [selectedId, setSelectedId] = useState(document.segments[0]?.id ?? "");
  const [transcribers, setTranscribers] = useState<SpeechModel[]>([]);
  const [modelId, setModelId] = useState("");
  const [language, setLanguage] = useState(document.language || "auto");
  const [speechDetail, setSpeechDetail] = useState("Checking local Whisper…");
  const [savedRevision, setSavedRevision] = useState(document.revision);
  const [savedTheme, setSavedTheme] = useState(document.theme);
  const activeSegment = musicLyricSegmentAt(document.segments, currentTime);
  const displaySegment = musicLyricDisplaySegmentAt(document.segments, currentTime);
  const cueExiting = Boolean(displaySegment && displaySegment !== activeSegment);
  const selectedSegment = document.segments.find((segment) => segment.id === selectedId);
  const takeNumber = project.takes.findIndex((candidate) => candidate.id === take.id) + 1;

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
    setSelectedId((current) => document.segments.some((segment) => segment.id === current)
      ? current
      : document.segments[0]?.id ?? "");
  }, [document]);

  useEffect(() => {
    setSavedRevision(document.revision);
    setSavedTheme(document.theme);
    // updatedAt changes only when the backend returns a durable document; local previews keep it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [document.revision, document.updatedAt]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    let visualizer: MusicLyricRenderer;
    try {
      visualizer = createMusicLyricVisualizer(document.theme, canvas);
    } catch {
      return;
    }
    const analysis = audio ? ensureAudioAnalysis(audio) : undefined;
    const reactivity = new MusicLyricReactivity();
    let frame = 0;
    const draw = () => {
      const visualFrame = reactivity.sample(
        analysis?.analyser,
        analysis?.frequency,
        analysis?.time,
        currentTimeRef.current / Math.max(0.01, take.durationSeconds),
        measureLyricLayout(canvas, primaryRef.current, translationRef.current),
      );
      visualizer.draw(visualFrame);
      if (producerRef.current) applyMusicLyricFrameStyles(producerRef.current, visualFrame);
      frame = requestAnimationFrame(draw);
    };
    frame = requestAnimationFrame(draw);
    return () => {
      cancelAnimationFrame(frame);
      visualizer.destroy?.();
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

  const patchSegment = (id: string, patch: Partial<MusicLyricSegment>) => {
    const timingChanged = "start" in patch || "end" in patch;
    onChange({
      ...document,
      segments: document.segments.map((segment) => segment.id === id ? { ...segment, ...patch, ...(timingChanged ? { words: [] } : {}) } : segment),
    });
  };

  const addCue = () => {
    const start = Math.min(take.durationSeconds - 0.02, Math.max(0, currentTime));
    const segment: MusicLyricSegment = {
      id: stableId(),
      start,
      end: Math.min(take.durationSeconds, start + 3),
      primary: "New lyric cue",
      translation: "",
      words: [],
    };
    onChange({ ...document, segments: [...document.segments, segment].sort((left, right) => left.start - right.start) });
    setSelectedId(segment.id);
  };

  const removeCue = (id: string) => {
    const index = document.segments.findIndex((segment) => segment.id === id);
    const next = document.segments.filter((segment) => segment.id !== id);
    onChange({ ...document, segments: next });
    setSelectedId(next[Math.min(index, next.length - 1)]?.id ?? "");
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

  const saveCurrentDocument = async () => {
    const saved = await onSave(document);
    if (!saved) return;
    setSavedRevision(saved.revision);
    setSavedTheme(saved.theme);
  };

  return (
    <section ref={producerRef} className={`music-lyrics-producer theme-${document.theme} ${editing ? "editing" : ""}`} aria-label="Visual lyric producer">
      <canvas ref={canvasRef} className="music-lyrics-canvas" aria-hidden="true" />
      <div className="music-lyrics-paper" aria-hidden="true" />

      <header className="music-lyrics-header" data-lyric-control>
        <button aria-label="Close visual lyric producer" onClick={onClose}><ChevronLeft /> Arranger</button>
        <div className="music-lyrics-title"><small>Kestrel visual lyrics · Take {takeNumber}</small><strong>{project.title}</strong></div>
        <div className="music-lyrics-header-actions">
          <label className="music-lyrics-theme-picker" title={MUSIC_LYRIC_THEMES.find((theme) => theme.id === document.theme)?.description}>
            <Palette /><span>Visual</span>
            <select aria-label="Lyric visual theme" disabled={busy} value={document.theme} onChange={(event) => onChange({ ...document, theme: event.currentTarget.value as MusicLyricsDocument["theme"] })}>
              {MUSIC_LYRIC_THEMES.map((theme) => <option key={theme.id} value={theme.id}>{theme.name}</option>)}
            </select>
          </label>
          {document.theme !== savedTheme && <button className="music-lyrics-save-look" disabled={busy} onClick={() => void saveCurrentDocument()}><Save /> Save look</button>}
          <span><Captions /> Revision {document.revision} · {document.source === "producer-timing-draft" ? "timing draft" : "local sync"}</span>
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
        <section className="music-lyrics-sync">
          <div><Sparkles /><span><strong>Local word sync</strong><small>{speechDetail}</small></span></div>
          <label>Whisper model<select aria-label="Lyric transcription model" disabled={busy || !transcribers.length} value={modelId} onChange={(event) => setModelId(event.target.value)}><option value="">Not installed</option>{transcribers.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}</select></label>
          <label>Language<input aria-label="Lyric transcription language" disabled={busy} value={language} maxLength={64} onChange={(event) => setLanguage(event.target.value)} placeholder="auto" /></label>
          {busy
            ? <button className="danger" onClick={onCancelSync}><CircleStop /> Stop safely</button>
            : <button disabled={!modelId} onClick={() => void onSync(modelId, language.trim() || "auto")}><WandSparkles /> Sync this take</button>}
          {status && <p role="status">{busy && <LoaderCircle className="spin" />} {status}</p>}
        </section>
        <div className="music-lyrics-editor-actions"><button onClick={addCue}><Plus /> Add cue at playhead</button><label className="music-lyrics-translation-toggle"><input type="checkbox" checked={document.showTranslation} onChange={(event) => onChange({ ...document, showTranslation: event.target.checked })} /><Languages /> Show translations</label></div>
        <div className="music-lyrics-cue-list">
          {document.segments.map((segment, index) => <button key={segment.id} className={`${segment.id === selectedId ? "selected" : ""} ${segment.id === activeSegment?.id ? "active" : ""}`} onClick={() => { setSelectedId(segment.id); onSeek(segment.start); }}><span>{index + 1}</span><strong>{segment.primary}</strong><small>{formatTime(segment.start)} – {formatTime(segment.end)}</small></button>)}
          {!document.segments.length && <p>No vocal cues yet. Add one at the playhead or run local word sync.</p>}
        </div>
        {selectedSegment && <fieldset disabled={busy} className="music-lyrics-cue-editor">
          <legend>Cue {document.segments.findIndex((segment) => segment.id === selectedSegment.id) + 1}</legend>
          <div className="music-lyrics-time-fields"><label>Start<input type="number" min={0} max={take.durationSeconds} step={0.01} value={roundTime(selectedSegment.start)} onChange={(event) => patchSegment(selectedSegment.id, { start: event.currentTarget.valueAsNumber })} /></label><button onClick={() => patchSegment(selectedSegment.id, { start: currentTime })}>Set playhead</button><label>End<input type="number" min={0.01} max={take.durationSeconds} step={0.01} value={roundTime(selectedSegment.end)} onChange={(event) => patchSegment(selectedSegment.id, { end: event.currentTarget.valueAsNumber })} /></label><button onClick={() => patchSegment(selectedSegment.id, { end: currentTime })}>Set playhead</button></div>
          <label>Primary lyric<textarea value={selectedSegment.primary} onChange={(event) => patchSegment(selectedSegment.id, { primary: event.target.value, words: [] })} /></label>
          <label>Translation<textarea value={selectedSegment.translation} onChange={(event) => patchSegment(selectedSegment.id, { translation: event.target.value })} placeholder="Optional second line…" /></label>
          <button className="danger" onClick={() => removeCue(selectedSegment.id)}><Trash2 /> Remove cue</button>
        </fieldset>}
        <footer><span>{document.segments.length} cues · saved revision {savedRevision}</span><button disabled={busy} onClick={() => void saveCurrentDocument()}><Save /> Save revision</button></footer>
      </aside>}
    </section>
  );
}

export function musicLyricSegmentAt(segments: MusicLyricSegment[], seconds: number): MusicLyricSegment | undefined {
  return segments.find((segment) => seconds >= segment.start && seconds <= segment.end);
}

export function musicLyricDisplaySegmentAt(segments: MusicLyricSegment[], seconds: number): MusicLyricSegment | undefined {
  const active = musicLyricSegmentAt(segments, seconds);
  if (active) return active;
  for (let index = segments.length - 1; index >= 0; index -= 1) {
    const segment = segments[index];
    if (seconds > segment.end && seconds <= segment.end + 0.42) return segment;
  }
  return undefined;
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

export function wordProgress(start: number, end: number, currentTime: number): number {
  if (currentTime <= start) return 0;
  if (currentTime >= end || end <= start) return 1;
  return (currentTime - start) / (end - start);
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

function stableId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (character) => {
    const random = Math.floor(Math.random() * 16);
    return (character === "x" ? random : (random & 0x3) | 0x8).toString(16);
  });
}

function roundTime(seconds: number): number {
  return Math.round(seconds * 100) / 100;
}

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds)) return "00:00";
  const safe = Math.max(0, seconds);
  const minutes = Math.floor(safe / 60);
  return `${minutes.toString().padStart(2, "0")}:${Math.floor(safe % 60).toString().padStart(2, "0")}`;
}

function formatPreciseTime(seconds: number): string {
  const safe = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
  const minutes = Math.floor(safe / 60);
  return `${minutes.toString().padStart(2, "0")}:${(safe % 60).toFixed(1).padStart(4, "0")}`;
}
