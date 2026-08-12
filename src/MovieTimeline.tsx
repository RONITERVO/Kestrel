import {
  Archive, Copy, Gauge, Pause, Play, Redo2, RotateCcw, Scissors, Trash2,
  Undo2, Volume2, ZoomIn,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { movieMediaUrl } from "./api";
import type { ClipEdit, MovieEdit, MovieProject, RenderedClip } from "./types";

export interface TimelineItem {
  edit: ClipEdit;
  clip: RenderedClip;
  sourcePath: string;
  sourceDuration: number;
  outputDuration: number;
  versionLabel: string;
}

export function timelineItems(project: MovieProject, edit: MovieEdit): TimelineItem[] {
  return [...edit.clips]
    .sort((left, right) => left.order - right.order)
    .flatMap((decision) => {
      const clip = project.clips.find((candidate) => candidate.id === decision.clipId);
      if (!clip) return [];
      const version = decision.sourceVersionId
        ? clip.versions.find((candidate) => candidate.id === decision.sourceVersionId)
        : undefined;
      const sourceDuration = version?.durationSeconds ?? clip.durationSeconds;
      return [{
        edit: decision,
        clip,
        sourcePath: version?.path ?? clip.path,
        sourceDuration,
        outputDuration: Math.max(0, sourceDuration - decision.trimStart - decision.trimEnd) / Math.max(0.25, decision.speed),
        versionLabel: version ? version.id === "original" ? "Original master" : `Version ${version.id}` : "Active master",
      }];
    });
}

export function orderedMovieEdit(edit: MovieEdit): MovieEdit {
  return {
    ...edit,
    clips: [...edit.clips]
      .sort((left, right) => left.order - right.order)
      .map((item, order) => ({ ...item, order })),
  };
}

export function moveTimelineItem(edit: MovieEdit, sourceId: string, targetId: string): MovieEdit {
  const clips = [...edit.clips].sort((left, right) => left.order - right.order);
  const source = clips.findIndex((item) => item.id === sourceId);
  const target = clips.findIndex((item) => item.id === targetId);
  if (source < 0 || target < 0 || source === target) return orderedMovieEdit(edit);
  const [moved] = clips.splice(source, 1);
  clips.splice(target, 0, moved);
  return { ...edit, clips: clips.map((item, order) => ({ ...item, order })) };
}

export function splitTimelineItem(
  project: MovieProject,
  edit: MovieEdit,
  itemId: string,
  sourceTime: number,
  nextId: string,
): MovieEdit {
  const items = timelineItems(project, edit);
  const selected = items.find((item) => item.edit.id === itemId);
  if (!selected) return edit;
  const minimum = selected.edit.trimStart + 0.1;
  const maximum = selected.sourceDuration - selected.edit.trimEnd - 0.1;
  if (sourceTime < minimum || sourceTime > maximum) return edit;
  const clips = [...edit.clips].sort((left, right) => left.order - right.order);
  const index = clips.findIndex((item) => item.id === itemId);
  const first = { ...clips[index], trimEnd: selected.sourceDuration - sourceTime };
  const second = { ...clips[index], id: nextId, trimStart: sourceTime };
  clips.splice(index, 1, first, second);
  return { ...edit, clips: clips.map((item, order) => ({ ...item, order })) };
}

function editId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? `edit-${crypto.randomUUID()}`
    : `edit-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export function MovieTimeline({ project, value, disabled, onChange }: {
  project: MovieProject;
  value: MovieEdit;
  disabled: boolean;
  onChange: (edit: MovieEdit) => void;
}) {
  const items = useMemo(() => timelineItems(project, value), [project, value]);
  const enabledItems = items.filter((item) => item.edit.enabled && item.sourcePath);
  const [selectedId, setSelectedId] = useState(items[0]?.edit.id ?? "");
  const [draggingId, setDraggingId] = useState("");
  const [zoom, setZoom] = useState(58);
  const [undo, setUndo] = useState<MovieEdit[]>([]);
  const [redo, setRedo] = useState<MovieEdit[]>([]);
  const [previewId, setPreviewId] = useState(enabledItems[0]?.edit.id ?? "");
  const [previewTime, setPreviewTime] = useState(0);
  const [sequencePlaying, setSequencePlaying] = useState(false);
  const videoRef = useRef<HTMLVideoElement>(null);
  const selected = items.find((item) => item.edit.id === selectedId) ?? items[0];
  const preview = enabledItems.find((item) => item.edit.id === previewId) ?? enabledItems[0];
  const totalDuration = enabledItems.reduce((sum, item) => sum + item.outputDuration, 0);
  const elapsed = preview ? enabledItems.slice(0, enabledItems.indexOf(preview)).reduce((sum, item) => sum + item.outputDuration, 0)
    + Math.max(0, previewTime - preview.edit.trimStart) / preview.edit.speed : 0;

  useEffect(() => {
    if (selectedId && items.some((item) => item.edit.id === selectedId)) return;
    setSelectedId(items[0]?.edit.id ?? "");
  }, [items, selectedId]);

  const commit = (next: MovieEdit) => {
    if (disabled) return;
    setUndo((history) => [...history.slice(-49), value]);
    setRedo([]);
    onChange(orderedMovieEdit(next));
  };
  const undoEdit = () => {
    const previous = undo.at(-1);
    if (!previous) return;
    setUndo((history) => history.slice(0, -1));
    setRedo((history) => [...history.slice(-49), value]);
    onChange(previous);
  };
  const redoEdit = () => {
    const next = redo.at(-1);
    if (!next) return;
    setRedo((history) => history.slice(0, -1));
    setUndo((history) => [...history.slice(-49), value]);
    onChange(next);
  };
  const patchSelected = (change: Partial<ClipEdit>) => {
    if (!selected) return;
    const next = { ...selected.edit, ...change };
    const version = next.sourceVersionId ? selected.clip.versions.find((item) => item.id === next.sourceVersionId) : undefined;
    const sourceDuration = version?.durationSeconds ?? selected.clip.durationSeconds;
    const duration = Math.max(0, sourceDuration - next.trimStart - next.trimEnd) / Math.max(.25, next.speed);
    const fitFades = (incoming: number, outgoing: number): [number, number] => {
      const total = incoming + outgoing;
      const scale = total > duration && total > 0 ? duration / total : 1;
      return [incoming * scale, outgoing * scale];
    };
    [next.fadeIn, next.fadeOut] = fitFades(next.fadeIn, next.fadeOut);
    [next.audioFadeIn, next.audioFadeOut] = fitFades(next.audioFadeIn, next.audioFadeOut);
    commit({ ...value, clips: value.clips.map((item) => item.id === selected.edit.id ? next : item) });
  };
  const previewSequence = () => {
    if (!preview) return;
    const player = videoRef.current;
    if (player && !player.paused) {
      player.pause();
      setSequencePlaying(false);
      return;
    }
    setSequencePlaying(true);
    void player?.play().catch(() => setSequencePlaying(false));
  };
  const advancePreview = () => {
    if (!preview) return;
    const index = enabledItems.findIndex((item) => item.edit.id === preview.edit.id);
    const next = enabledItems[index + 1];
    if (!next) {
      setSequencePlaying(false);
      return;
    }
    setSequencePlaying(true);
    setPreviewId(next.edit.id);
    setSelectedId(next.edit.id);
    setPreviewTime(next.edit.trimStart);
  };
  const split = () => {
    if (!selected) return;
    const currentSourceTime = preview?.edit.id === selected.edit.id
      ? previewTime
      : selected.edit.trimStart + (selected.sourceDuration - selected.edit.trimStart - selected.edit.trimEnd) / 2;
    const next = splitTimelineItem(project, value, selected.edit.id, currentSourceTime, editId());
    if (next !== value) commit(next);
  };
  const duplicate = () => {
    if (!selected) return;
    const clips = [...value.clips].sort((left, right) => left.order - right.order);
    const index = clips.findIndex((item) => item.id === selected.edit.id);
    const copy = { ...clips[index], id: editId() };
    clips.splice(index + 1, 0, copy);
    commit({ ...value, clips: clips.map((item, order) => ({ ...item, order })) });
    setSelectedId(copy.id);
  };
  const remove = () => {
    if (!selected) return;
    const index = items.findIndex((item) => item.edit.id === selected.edit.id);
    commit({ ...value, clips: value.clips.filter((item) => item.id !== selected.edit.id) });
    setSelectedId(items[index + 1]?.edit.id ?? items[index - 1]?.edit.id ?? "");
  };
  const handleKey = (event: React.KeyboardEvent<HTMLElement>) => {
    if ((event.target as HTMLElement).matches("input, textarea, select, button")) return;
    if (event.ctrlKey && event.key.toLowerCase() === "z") {
      event.preventDefault();
      event.shiftKey ? redoEdit() : undoEdit();
    } else if (event.key === " ") {
      event.preventDefault();
      previewSequence();
    } else if ((event.key === "Delete" || event.key === "Backspace") && selected) {
      event.preventDefault();
      remove();
    } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      const index = items.findIndex((item) => item.edit.id === selected?.edit.id);
      const next = items[index + (event.key === "ArrowLeft" ? -1 : 1)];
      if (next) setSelectedId(next.edit.id);
    }
  };

  return <div className="movie-editor" tabIndex={0} onKeyDown={handleKey}>
    <div className="movie-editor-workspace">
      <section className="movie-program-monitor">
        <div className="movie-monitor-screen">
          {preview ? <video key={preview.sourcePath} ref={videoRef} src={movieMediaUrl(preview.sourcePath)} preload="metadata"
            onLoadedMetadata={(event) => {
              event.currentTarget.currentTime = preview.edit.trimStart;
              event.currentTarget.playbackRate = preview.edit.speed;
              event.currentTarget.volume = Math.min(1, preview.edit.audioGain);
              setPreviewTime(preview.edit.trimStart);
              if (sequencePlaying) void event.currentTarget.play().catch(() => setSequencePlaying(false));
            }}
            onPlay={() => setSequencePlaying(true)}
            onTimeUpdate={(event) => {
              const time = event.currentTarget.currentTime;
              setPreviewTime(time);
              if (time >= preview.sourceDuration - preview.edit.trimEnd - 0.03) {
                advancePreview();
              }
            }} /> : <div className="movie-monitor-empty"><Play />Enable a rendered timeline item to preview the sequence.</div>}
        </div>
        <div className="movie-transport">
          <button aria-label="Restart preview" disabled={!preview} onClick={() => {
            if (!preview || !videoRef.current) return;
            videoRef.current.currentTime = preview.edit.trimStart;
            setPreviewTime(preview.edit.trimStart);
          }}><RotateCcw /></button>
          <button className="transport-play" aria-label={sequencePlaying ? "Pause sequence" : "Play sequence"} disabled={!preview} onClick={previewSequence}>{sequencePlaying ? <Pause /> : <Play />}</button>
          <time>{formatTime(elapsed)} / {formatTime(totalDuration)}</time>
          <span>{preview ? `${preview.clip.title} · ${preview.versionLabel} · ${preview.edit.speed}×` : "No preview source"}</span>
        </div>
      </section>
      <aside className="movie-inspector">
        <header><span>Inspector</span><strong>{selected?.clip.title ?? "No selection"}</strong></header>
        {selected ? <div className="movie-inspector-fields">
          <label className="wide">Preserved source<select value={selected.edit.sourceVersionId} onChange={(event) => patchSelected({ sourceVersionId: event.target.value, trimStart: 0, trimEnd: 0 })}>
            <option value="">Active master</option>
            {selected.clip.versions.map((version) => <option key={version.id} value={version.id}>{version.id === "original" ? "Original master" : `Version ${version.id}`} · {version.durationSeconds.toFixed(1)}s</option>)}
          </select></label>
          <TimelineNumber label="Trim in" value={selected.edit.trimStart} min={0} max={Math.max(0, selected.sourceDuration - selected.edit.trimEnd - 0.1)} step={0.05} suffix="s" onChange={(trimStart) => patchSelected({ trimStart })} />
          <TimelineNumber label="Trim out" value={selected.edit.trimEnd} min={0} max={Math.max(0, selected.sourceDuration - selected.edit.trimStart - 0.1)} step={0.05} suffix="s" onChange={(trimEnd) => patchSelected({ trimEnd })} />
          <TimelineNumber label="Speed" value={selected.edit.speed} min={0.25} max={4} step={0.05} suffix="×" onChange={(speed) => patchSelected({ speed })} />
          <TimelineNumber label="Audio gain" value={selected.edit.audioGain} min={0} max={4} step={0.05} suffix="×" onChange={(audioGain) => patchSelected({ audioGain })} />
          <TimelineNumber label="Picture fade in" value={selected.edit.fadeIn} min={0} max={Math.max(0, selected.outputDuration - selected.edit.fadeOut)} step={0.05} suffix="s" onChange={(fadeIn) => patchSelected({ fadeIn })} />
          <TimelineNumber label="Picture fade out" value={selected.edit.fadeOut} min={0} max={Math.max(0, selected.outputDuration - selected.edit.fadeIn)} step={0.05} suffix="s" onChange={(fadeOut) => patchSelected({ fadeOut })} />
          <TimelineNumber label="Audio fade in" value={selected.edit.audioFadeIn} min={0} max={Math.max(0, selected.outputDuration - selected.edit.audioFadeOut)} step={0.05} suffix="s" onChange={(audioFadeIn) => patchSelected({ audioFadeIn })} />
          <TimelineNumber label="Audio fade out" value={selected.edit.audioFadeOut} min={0} max={Math.max(0, selected.outputDuration - selected.edit.audioFadeIn)} step={0.05} suffix="s" onChange={(audioFadeOut) => patchSelected({ audioFadeOut })} />
          <label className="wide movie-enable-item"><input type="checkbox" checked={selected.edit.enabled} onChange={(event) => patchSelected({ enabled: event.target.checked })} /> Include this item in preview and export</label>
        </div> : <p>Select a timeline item to edit it.</p>}
      </aside>
    </div>
    <section className="movie-timeline-panel">
      <div className="movie-timeline-toolbar">
        <div>
          <button aria-label="Undo timeline change" title="Undo · Ctrl+Z" disabled={!undo.length} onClick={undoEdit}><Undo2 /></button>
          <button aria-label="Redo timeline change" title="Redo · Ctrl+Shift+Z" disabled={!redo.length} onClick={redoEdit}><Redo2 /></button>
          <i />
          <button aria-label="Split timeline item" title="Split at playhead" disabled={!selected} onClick={split}><Scissors /></button>
          <button aria-label="Duplicate timeline item" title="Duplicate" disabled={!selected || items.length >= 512} onClick={duplicate}><Copy /></button>
          <button aria-label="Remove timeline item" title="Remove from timeline" disabled={!selected} onClick={remove}><Trash2 /></button>
        </div>
        <div><span>{items.length} items · {formatTime(totalDuration)}</span><ZoomIn /><input aria-label="Timeline zoom" type="range" min={24} max={140} value={zoom} onChange={(event) => setZoom(Number(event.target.value))} /></div>
      </div>
      <div className="movie-time-ruler" style={{ width: Math.max(720, totalDuration * zoom) }}><span>00:00</span><span>{formatTime(totalDuration / 2)}</span><span>{formatTime(totalDuration)}</span></div>
      <div className="movie-track-scroll">
        <div className="movie-track-row"><div className="movie-track-label"><strong>V1</strong><small>Picture</small></div><div className="movie-track" style={{ width: Math.max(720, totalDuration * zoom) }}>
          {items.map((item, index) => <button key={item.edit.id} draggable={!disabled} onDragStart={() => setDraggingId(item.edit.id)} onDragOver={(event) => event.preventDefault()} onDrop={() => {
            if (draggingId) commit(moveTimelineItem(value, draggingId, item.edit.id));
            setDraggingId("");
          }} onClick={() => { setSelectedId(item.edit.id); if (item.edit.enabled) setPreviewId(item.edit.id); }}
            className={`${selected?.edit.id === item.edit.id ? "selected" : ""} ${item.edit.enabled ? "" : "disabled"}`}
            style={{ width: Math.max(72, item.outputDuration * zoom) }}>
            <span>{index + 1}</span><strong>{item.clip.title}</strong><small>{item.outputDuration.toFixed(2)}s · {item.edit.speed}×</small>
            {(item.edit.fadeIn > 0 || item.edit.fadeOut > 0) && <i className="timeline-fade">fade</i>}
          </button>)}
        </div></div>
        <div className="movie-track-row audio"><div className="movie-track-label"><strong>A1</strong><small>Native sound</small></div><div className="movie-track" style={{ width: Math.max(720, totalDuration * zoom) }}>
          {items.map((item) => <div key={item.edit.id} className={item.edit.enabled ? "" : "disabled"} style={{ width: Math.max(72, item.outputDuration * zoom) }}><Volume2 /><span style={{ opacity: Math.min(1, .25 + item.edit.audioGain / 2) }} /><small>{item.edit.audioGain === 0 ? "muted" : `${item.edit.audioGain}×`}</small></div>)}
        </div></div>
      </div>
      <small className="movie-shortcuts">Space play/pause · ←/→ select · Delete removes a decision · Ctrl+Z undo · drag items to reorder. Source masters are never changed.</small>
    </section>
    <section className="movie-export-settings">
      <div><Archive /><span><strong>Immutable export</strong><small>Every render gets a unique file, SHA-256 record, and JSON decision-list sidecar.</small></span></div>
      <label>Title<input value={value.exportTitle} maxLength={120} onChange={(event) => commit({ ...value, exportTitle: event.target.value })} /></label>
      <label>Quality<select value={value.exportPreset} onChange={(event) => commit({ ...value, exportPreset: event.target.value as MovieEdit["exportPreset"] })}>
        <option value="archive">Archive · CRF 14 / 320 kbps</option><option value="publish">Publish · CRF 18 / 192 kbps</option><option value="review">Review · CRF 24 / 128 kbps</option>
      </select></label>
      <label className="movie-normalize"><input type="checkbox" checked={value.normalizeAudio} onChange={(event) => commit({ ...value, normalizeAudio: event.target.checked })} /> Normalize finished audio</label>
      {value.normalizeAudio && <TimelineNumber label="Target" value={value.targetLufs} min={-24} max={-9} step={1} suffix="LUFS" onChange={(targetLufs) => commit({ ...value, targetLufs })} />}
      <span className="movie-export-summary"><Gauge /> {formatTime(totalDuration)} · {enabledItems.length} active items</span>
    </section>
  </div>;
}

function TimelineNumber({ label, value, min, max, step, suffix, onChange }: {
  label: string; value: number; min: number; max: number; step: number; suffix: string; onChange: (value: number) => void;
}) {
  return <label>{label}<span><input type="number" value={Number.isFinite(value) ? value : 0} min={min} max={max} step={step} onChange={(event) => {
    const parsed = Number(event.target.value);
    if (Number.isFinite(parsed)) onChange(Math.min(max, Math.max(min, parsed)));
  }} /><b>{suffix}</b></span></label>;
}

export function formatTime(seconds: number): string {
  const safe = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
  const minutes = Math.floor(safe / 60);
  const remainder = safe - minutes * 60;
  return `${String(minutes).padStart(2, "0")}:${remainder.toFixed(2).padStart(5, "0")}`;
}
