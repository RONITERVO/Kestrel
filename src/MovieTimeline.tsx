import {
  Archive, AudioLines, Check, ChevronLeft, ChevronRight, CircleHelp, Copy,
  Eye, EyeOff, Film, Flag, Gauge, Images, Info, List, Magnet, Maximize2,
  Minimize2, MousePointer2, PanelLeft, PanelRight, Pause, Play, Plus, Redo2,
  RotateCcw, Save, ScanLine, Scissors, Search, SkipBack, SkipForward,
  Trash2, Undo2, Video, Volume2, X, ZoomIn,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { movieMediaUrl } from "./api";
import type {
  ClipEdit, MovieEdit, MovieProject, MovieReference, RenderedClip, TimelineMarker,
} from "./types";

const FPS = 24;

export interface TimelineItem {
  edit: ClipEdit;
  clip: RenderedClip;
  sourcePath: string;
  sourceDuration: number;
  outputDuration: number;
  versionLabel: string;
}

type EditorTool = "select" | "trim" | "blade";
type BrowserTab = "masters" | "references" | "index";
type InspectorTab = "video" | "audio" | "info";
type ViewerMode = "program" | "source";

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
    markers: [...(edit.markers ?? [])].sort((left, right) => left.timeSeconds - right.timeSeconds),
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
  const first = {
    ...clips[index],
    trimEnd: selected.sourceDuration - sourceTime,
    fadeOut: 0,
    audioFadeOut: 0,
  };
  const second = {
    ...clips[index],
    id: nextId,
    trimStart: sourceTime,
    fadeIn: 0,
    audioFadeIn: 0,
  };
  clips.splice(index, 1, first, second);
  return { ...edit, clips: clips.map((item, order) => ({ ...item, order })) };
}

export function appendTimelineSource(edit: MovieEdit, clipId: string, id: string): MovieEdit {
  const clips = [...edit.clips].sort((left, right) => left.order - right.order);
  clips.push({
    id, clipId, enabled: true, order: clips.length, trimStart: 0, trimEnd: 0,
    audioGain: 1, sourceVersionId: "", speed: 1, fadeIn: 0, fadeOut: 0,
    audioFadeIn: 0, audioFadeOut: 0, label: "", notes: "",
  });
  return { ...edit, clips };
}

function editId(prefix = "edit"): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? `${prefix}-${crypto.randomUUID()}`
    : `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export function MovieTimeline({ project, value, disabled, onChange, onRequestSave }: {
  project: MovieProject;
  value: MovieEdit;
  disabled: boolean;
  onChange: (edit: MovieEdit) => void;
  onRequestSave?: () => void;
}) {
  const normalizedValue = useMemo(() => ({ ...value, markers: value.markers ?? [] }), [value]);
  const items = useMemo(() => timelineItems(project, normalizedValue), [project, normalizedValue]);
  const enabledItems = items.filter((item) => item.edit.enabled && item.sourcePath);
  const [selectedId, setSelectedId] = useState(items[0]?.edit.id ?? "");
  const [draggingId, setDraggingId] = useState("");
  const [zoom, setZoom] = useState(68);
  const [undo, setUndo] = useState<MovieEdit[]>([]);
  const [redo, setRedo] = useState<MovieEdit[]>([]);
  const [previewId, setPreviewId] = useState(enabledItems[0]?.edit.id ?? "");
  const [previewTime, setPreviewTime] = useState(0);
  const [sequencePlaying, setSequencePlaying] = useState(false);
  const [viewerMode, setViewerMode] = useState<ViewerMode>("program");
  const [browserTab, setBrowserTab] = useState<BrowserTab>("masters");
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>("video");
  const [tool, setTool] = useState<EditorTool>("select");
  const [snapping, setSnapping] = useState(true);
  const [skimming, setSkimming] = useState(true);
  const [showSafeAreas, setShowSafeAreas] = useState(false);
  const [cinemaViewer, setCinemaViewer] = useState(false);
  const [showBrowser, setShowBrowser] = useState(true);
  const [showInspector, setShowInspector] = useState(true);
  const [showShortcuts, setShowShortcuts] = useState(false);
  const [search, setSearch] = useState("");
  const [sourceClipId, setSourceClipId] = useState(project.clips[0]?.id ?? "");
  const [sourceVersionId, setSourceVersionId] = useState("");
  const [sourceTime, setSourceTime] = useState(0);
  const [sourcePlaying, setSourcePlaying] = useState(false);
  const [sourceReferenceId, setSourceReferenceId] = useState("");
  const [skimmer, setSkimmer] = useState<{ id: string; fraction: number }>();
  const [markerComposer, setMarkerComposer] = useState(false);
  const [markerLabel, setMarkerLabel] = useState("");
  const [markerKind, setMarkerKind] = useState<TimelineMarker["kind"]>("marker");
  const videoRef = useRef<HTMLVideoElement>(null);
  const sourceVideoRef = useRef<HTMLVideoElement>(null);
  const skimThrottle = useRef(0);
  const selected = items.find((item) => item.edit.id === selectedId) ?? items[0];
  const preview = enabledItems.find((item) => item.edit.id === previewId) ?? enabledItems[0];
  const sourceClip = project.clips.find((clip) => clip.id === sourceClipId) ?? project.clips[0];
  const sourceVersion = sourceVersionId ? sourceClip?.versions.find((version) => version.id === sourceVersionId) : undefined;
  const sourceReference = project.references.find((reference) => reference.assetId === sourceReferenceId);
  const sourcePath = sourceReference?.path ?? sourceVersion?.path ?? sourceClip?.path ?? "";
  const programMediaUrl = preview ? movieMediaUrl(preview.sourcePath) : "";
  const sourceDuration = sourceReference?.durationSeconds ?? sourceVersion?.durationSeconds ?? sourceClip?.durationSeconds ?? 0;
  const totalDuration = enabledItems.reduce((sum, item) => sum + item.outputDuration, 0);
  const elapsed = preview ? enabledItems.slice(0, enabledItems.indexOf(preview)).reduce((sum, item) => sum + item.outputDuration, 0)
    + Math.max(0, previewTime - preview.edit.trimStart) / preview.edit.speed : 0;
  const trackWidth = Math.max(900, totalDuration * zoom);
  const timelineScale = totalDuration > 0 ? trackWidth / totalDuration : zoom;
  const dirty = JSON.stringify(orderedMovieEdit(normalizedValue)) !== JSON.stringify(orderedMovieEdit({ ...project.edit, markers: project.edit.markers ?? [] }));
  const query = search.trim().toLocaleLowerCase();
  const filteredClips = project.clips.filter((clip) => !query || `${clip.title} ${clip.prompt}`.toLocaleLowerCase().includes(query));
  const filteredReferences = project.references.filter((reference) => !query || `${reference.name} ${reference.description}`.toLocaleLowerCase().includes(query));

  useEffect(() => {
    if (selectedId && items.some((item) => item.edit.id === selectedId)) return;
    setSelectedId(items[0]?.edit.id ?? "");
  }, [items, selectedId]);

  const commit = (next: MovieEdit) => {
    if (disabled) return;
    setUndo((history) => [...history.slice(-49), normalizedValue]);
    setRedo([]);
    onChange(orderedMovieEdit({ ...next, markers: next.markers ?? [] }));
  };
  const undoEdit = () => {
    if (disabled) return;
    const previous = undo.at(-1);
    if (!previous) return;
    setUndo((history) => history.slice(0, -1));
    setRedo((history) => [...history.slice(-49), normalizedValue]);
    onChange(previous);
  };
  const redoEdit = () => {
    if (disabled) return;
    const next = redo.at(-1);
    if (!next) return;
    setRedo((history) => history.slice(0, -1));
    setUndo((history) => [...history.slice(-49), normalizedValue]);
    onChange(next);
  };
  const patchSelected = (change: Partial<ClipEdit>) => {
    if (!selected) return;
    const next = { ...selected.edit, ...change };
    const version = next.sourceVersionId ? selected.clip.versions.find((item) => item.id === next.sourceVersionId) : undefined;
    const nextSourceDuration = version?.durationSeconds ?? selected.clip.durationSeconds;
    const duration = Math.max(0, nextSourceDuration - next.trimStart - next.trimEnd) / Math.max(.25, next.speed);
    const fitFades = (incoming: number, outgoing: number): [number, number] => {
      const total = incoming + outgoing;
      const scale = total > duration && total > 0 ? duration / total : 1;
      return [incoming * scale, outgoing * scale];
    };
    [next.fadeIn, next.fadeOut] = fitFades(next.fadeIn, next.fadeOut);
    [next.audioFadeIn, next.audioFadeOut] = fitFades(next.audioFadeIn, next.audioFadeOut);
    commit({ ...normalizedValue, clips: normalizedValue.clips.map((item) => item.id === selected.edit.id ? next : item) });
  };
  const setProgramPosition = (item: TimelineItem, time: number, play = false) => {
    const bounded = Math.max(item.edit.trimStart, Math.min(item.sourceDuration - item.edit.trimEnd, time));
    setViewerMode("program");
    setPreviewId(item.edit.id);
    setSelectedId(item.edit.id);
    setPreviewTime(bounded);
    requestAnimationFrame(() => {
      const player = videoRef.current;
      if (!player) return;
      player.currentTime = bounded;
      if (play) void player.play().catch(() => setSequencePlaying(false));
    });
  };
  const seekGlobal = (time: number, play = false) => {
    const bounded = Math.max(0, Math.min(totalDuration, time));
    let cursor = 0;
    for (const item of enabledItems) {
      if (bounded <= cursor + item.outputDuration || item === enabledItems.at(-1)) {
        setProgramPosition(item, item.edit.trimStart + (bounded - cursor) * item.edit.speed, play);
        return;
      }
      cursor += item.outputDuration;
    }
  };
  const previewSequence = () => {
    if (viewerMode === "source") {
      const player = sourceVideoRef.current;
      if (!player) return;
      if (player.paused) void player.play().catch(() => setSourcePlaying(false));
      else player.pause();
      return;
    }
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
  const advancePreview = (direction = 1) => {
    if (!preview) return;
    const index = enabledItems.findIndex((item) => item.edit.id === preview.edit.id);
    const next = enabledItems[index + direction];
    if (!next) {
      setSequencePlaying(false);
      return;
    }
    setProgramPosition(next, direction > 0 ? next.edit.trimStart : next.sourceDuration - next.edit.trimEnd, sequencePlaying);
  };
  const stepFrame = (direction: number) => {
    if (viewerMode === "source") {
      const player = sourceVideoRef.current;
      if (!player) return;
      player.pause();
      player.currentTime = Math.max(0, Math.min(sourceDuration, player.currentTime + direction / FPS));
      setSourceTime(player.currentTime);
      return;
    }
    if (!preview) return;
    const next = previewTime + direction / FPS;
    if (next < preview.edit.trimStart) return advancePreview(-1);
    if (next > preview.sourceDuration - preview.edit.trimEnd) return advancePreview(1);
    videoRef.current?.pause();
    setProgramPosition(preview, next);
  };
  const splitAt = (item: TimelineItem, sourceTime: number) => {
    const next = splitTimelineItem(project, normalizedValue, item.edit.id, sourceTime, editId());
    if (next !== normalizedValue) commit(next);
  };
  const split = () => {
    if (!selected) return;
    const currentSourceTime = preview?.edit.id === selected.edit.id
      ? previewTime
      : selected.edit.trimStart + (selected.sourceDuration - selected.edit.trimStart - selected.edit.trimEnd) / 2;
    splitAt(selected, currentSourceTime);
  };
  const duplicate = () => {
    if (!selected) return;
    const clips = [...normalizedValue.clips].sort((left, right) => left.order - right.order);
    const index = clips.findIndex((item) => item.id === selected.edit.id);
    const copy = { ...clips[index], id: editId(), label: clips[index].label ? `${clips[index].label} copy` : "" };
    clips.splice(index + 1, 0, copy);
    commit({ ...normalizedValue, clips: clips.map((item, order) => ({ ...item, order })) });
    setSelectedId(copy.id);
  };
  const remove = () => {
    if (!selected) return;
    const index = items.findIndex((item) => item.edit.id === selected.edit.id);
    commit({ ...normalizedValue, clips: normalizedValue.clips.filter((item) => item.id !== selected.edit.id) });
    setSelectedId(items[index + 1]?.edit.id ?? items[index - 1]?.edit.id ?? "");
  };
  const appendClip = (clip: RenderedClip) => {
    if (disabled || normalizedValue.clips.length >= 512 || !clip.path) return;
    const id = editId();
    commit(appendTimelineSource(normalizedValue, clip.id, id));
    setSelectedId(id);
  };
  const addMarker = (label?: string, kind: TimelineMarker["kind"] = "marker") => {
    const marker: TimelineMarker = {
      id: editId("marker"), timeSeconds: Math.max(0, elapsed),
      label: label?.trim() || `${kind === "todo" ? "To-do" : kind === "chapter" ? "Chapter" : "Marker"} ${(normalizedValue.markers?.length ?? 0) + 1}`,
      kind, completed: false,
    };
    commit({ ...normalizedValue, markers: [...(normalizedValue.markers ?? []), marker] });
    setBrowserTab("index");
    setMarkerLabel("");
    setMarkerComposer(false);
  };
  const patchMarker = (id: string, change: Partial<TimelineMarker>) => commit({
    ...normalizedValue,
    markers: normalizedValue.markers.map((marker) => marker.id === id ? { ...marker, ...change } : marker),
  });
  const handleTimelinePointer = (event: React.MouseEvent<HTMLButtonElement>, item: TimelineItem) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const fraction = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
    const sourceTimeAtPointer = item.edit.trimStart + fraction * (item.sourceDuration - item.edit.trimStart - item.edit.trimEnd);
    if (tool === "blade") splitAt(item, sourceTimeAtPointer);
    else setProgramPosition(item, sourceTimeAtPointer);
  };
  const handleSkim = (event: React.MouseEvent<HTMLButtonElement>, item: TimelineItem) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const fraction = Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width));
    setSkimmer({ id: item.edit.id, fraction });
    if (!skimming || sequencePlaying || Date.now() - skimThrottle.current < 80) return;
    skimThrottle.current = Date.now();
    const time = item.edit.trimStart + fraction * (item.sourceDuration - item.edit.trimStart - item.edit.trimEnd);
    setProgramPosition(item, time);
  };
  const handleKey = (event: React.KeyboardEvent<HTMLElement>) => {
    if ((event.target as HTMLElement).matches("input, textarea, select, button")) return;
    const command = event.ctrlKey || event.metaKey;
    const key = event.key.toLowerCase();
    if (command && key === "z") {
      event.preventDefault();
      event.shiftKey ? redoEdit() : undoEdit();
    } else if (command && key === "s") {
      event.preventDefault();
      onRequestSave?.();
    } else if (event.key === " ") {
      event.preventDefault();
      previewSequence();
    } else if (key === "j") {
      event.preventDefault();
      stepFrame(-FPS);
    } else if (key === "k") {
      event.preventDefault();
      videoRef.current?.pause();
      sourceVideoRef.current?.pause();
    } else if (key === "l") {
      event.preventDefault();
      previewSequence();
    } else if (key === "a") setTool("select");
    else if (key === "t") setTool("trim");
    else if (key === "b") setTool("blade");
    else if (key === "n") setSnapping((active) => !active);
    else if (key === "s") setSkimming((active) => !active);
    else if (key === "m") addMarker();
    else if (key === "i" && selected && preview?.edit.id === selected.edit.id) patchSelected({ trimStart: Math.min(previewTime, selected.sourceDuration - selected.edit.trimEnd - .1) });
    else if (key === "o" && selected && preview?.edit.id === selected.edit.id) patchSelected({ trimEnd: Math.max(0, selected.sourceDuration - previewTime) });
    else if ((event.key === "Delete" || event.key === "Backspace") && selected) {
      event.preventDefault();
      remove();
    } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      stepFrame(event.key === "ArrowLeft" ? -1 : 1);
    } else if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      event.preventDefault();
      advancePreview(event.key === "ArrowUp" ? -1 : 1);
    } else if (event.key === "?") setShowShortcuts((visible) => !visible);
  };

  const viewerTitle = viewerMode === "program"
    ? preview ? preview.edit.label || preview.clip.title : "Program"
    : sourceReference?.name ?? sourceVersion?.title ?? sourceClip?.title ?? "Source";
  const viewerCurrentTime = viewerMode === "program" ? elapsed : sourceTime;
  const viewerDuration = viewerMode === "program" ? totalDuration : sourceDuration;

  return <div className={`movie-editor-pro ${cinemaViewer ? "cinema-viewer" : ""}`} tabIndex={0} onKeyDown={handleKey}>
    <header className="editor-command-bar">
      <div className="editor-project-identity"><Film /><span><strong>{project.title}</strong><small>{dirty ? "Unsaved timeline changes" : "Timeline saved"}</small></span>{dirty && <i />}</div>
      <div className="editor-workspace-switch"><button className={showBrowser ? "active" : ""} aria-label="Toggle media browser" onClick={() => setShowBrowser((shown) => !shown)}><PanelLeft /> Media</button><button className={showInspector ? "active" : ""} aria-label="Toggle inspector" onClick={() => setShowInspector((shown) => !shown)}><PanelRight /> Inspector</button></div>
      <div className="editor-save-state"><span>{formatTimecode(totalDuration)} · {enabledItems.length} edits</span><button disabled={disabled || !dirty} onClick={onRequestSave}><Save /> Save <kbd>⌘S</kbd></button></div>
    </header>

    <div className={`editor-upper ${showBrowser ? "with-browser" : ""} ${showInspector ? "with-inspector" : ""}`}>
      {showBrowser && <aside className="editor-media-browser">
        <div className="editor-panel-tabs"><button className={browserTab === "masters" ? "active" : ""} onClick={() => setBrowserTab("masters")}><Film /> Masters</button><button className={browserTab === "references" ? "active" : ""} onClick={() => setBrowserTab("references")}><Images /> References</button><button className={browserTab === "index" ? "active" : ""} onClick={() => setBrowserTab("index")}><List /> Index</button></div>
        <label className="editor-search"><Search /><input aria-label="Search editor media" value={search} onChange={(event) => setSearch(event.target.value)} placeholder={browserTab === "index" ? "Clips, markers, notes" : "Search this production"} />{search && <button aria-label="Clear media search" onClick={() => setSearch("")}><X /></button>}</label>
        <div className="editor-browser-body">
          {browserTab === "masters" && <>{filteredClips.map((clip) => <button key={clip.id} className={`editor-media-row ${sourceClip?.id === clip.id && !sourceReference ? "selected" : ""}`} onClick={() => { setSourceReferenceId(""); setSourceClipId(clip.id); setSourceVersionId(""); setViewerMode("source"); setSourceTime(0); }} onDoubleClick={() => appendClip(clip)}>
            <span className="editor-media-thumb"><Video /><b>{clip.index + 1}</b></span><span><strong>{clip.title}</strong><small>{clip.durationSeconds.toFixed(1)}s · seed {clip.seed}</small><em>{clip.versions.length ? `${clip.versions.length} preserved versions` : "Active master"}</em></span><i className={normalizedValue.clips.some((item) => item.clipId === clip.id) ? "used" : ""} />
          </button>)}{!filteredClips.length && <EditorEmpty text="No preserved masters match this search." />}</>}
          {browserTab === "references" && <>{filteredReferences.map((reference) => <button key={reference.assetId} className={`editor-media-row ${sourceReference?.assetId === reference.assetId ? "selected" : ""}`} onClick={() => { setSourceReferenceId(reference.assetId); setViewerMode("source"); setSourceTime(0); }}>
            <ReferenceThumb reference={reference} /><span><strong>{reference.name}</strong><small>{reference.kind} · {reference.durationSeconds ? `${reference.durationSeconds.toFixed(1)}s` : `${reference.width}×${reference.height}`}</small><em>{reference.description || "Producer reference"}</em></span>
          </button>)}{!filteredReferences.length && <EditorEmpty text="No producer references match this search." />}</>}
          {browserTab === "index" && <TimelineIndex items={items} markers={normalizedValue.markers} query={query} selectedId={selected?.edit.id} onSelect={(item) => setProgramPosition(item, item.edit.trimStart)} onSeek={seekGlobal} onPatchMarker={patchMarker} onDeleteMarker={(id) => commit({ ...normalizedValue, markers: normalizedValue.markers.filter((marker) => marker.id !== id) })} />}
        </div>
        <footer>{browserTab === "masters" ? <><span>{filteredClips.length} masters · double-click to append</span>{sourceClip && <button disabled={disabled || !sourceClip.path} onClick={() => appendClip(sourceClip)}><Plus /> Append</button>}</> : browserTab === "references" ? <span>Reference media informs generation; it is never substituted into the cut.</span> : <><span>{items.length} edits · {normalizedValue.markers.length} markers</span><button onClick={() => setMarkerComposer(true)}><Flag /> Marker</button></>}</footer>
      </aside>}

      <main className="editor-viewer">
        <header className="viewer-header"><div className="viewer-mode-tabs"><button className={viewerMode === "source" ? "active" : ""} onClick={() => setViewerMode("source")}>Source</button><button className={viewerMode === "program" ? "active" : ""} onClick={() => setViewerMode("program")}>Program</button></div><strong>{viewerTitle}</strong><div><button className={showSafeAreas ? "active" : ""} aria-label="Toggle title safe guides" title="Title safe guides" onClick={() => setShowSafeAreas((shown) => !shown)}><ScanLine /></button><button aria-label={cinemaViewer ? "Exit cinema viewer" : "Cinema viewer"} onClick={() => setCinemaViewer((active) => !active)}>{cinemaViewer ? <Minimize2 /> : <Maximize2 />}</button></div></header>
        <div className="editor-monitor">
          {viewerMode === "program" ? preview && programMediaUrl ? <video key={preview.sourcePath} ref={videoRef} src={programMediaUrl} preload="metadata"
            onLoadedMetadata={(event) => { event.currentTarget.currentTime = previewTime || preview.edit.trimStart; event.currentTarget.playbackRate = preview.edit.speed; event.currentTarget.volume = Math.min(1, preview.edit.audioGain); if (sequencePlaying) void event.currentTarget.play().catch(() => setSequencePlaying(false)); }}
            onPlay={() => setSequencePlaying(true)} onPause={() => setSequencePlaying(false)} onTimeUpdate={(event) => { const time = event.currentTarget.currentTime; setPreviewTime(time); if (time >= preview.sourceDuration - preview.edit.trimEnd - .03) advancePreview(); }} /> : <EditorMonitorEmpty />
            : <SourceViewer reference={sourceReference} path={sourcePath} time={sourceTime} duration={sourceDuration} videoRef={sourceVideoRef} playing={sourcePlaying} onPlaying={setSourcePlaying} onTime={setSourceTime} />}
          {showSafeAreas && <div className="viewer-safe-areas"><i /><i /></div>}
          <span className="viewer-resolution">{project.settings.width} × {project.settings.height} · 24p</span>
        </div>
        <input className="viewer-scrubber" aria-label="Viewer playhead" type="range" min={0} max={Math.max(.01, viewerDuration)} step={1 / FPS} value={Math.min(viewerDuration, viewerCurrentTime)} onChange={(event) => viewerMode === "program" ? seekGlobal(Number(event.target.value)) : (() => { const time = Number(event.target.value); setSourceTime(time); if (sourceVideoRef.current) sourceVideoRef.current.currentTime = time; })()} />
        <div className="editor-transport">
          <div><button aria-label="Previous edit" title="Previous edit · ↑" onClick={() => advancePreview(-1)}><SkipBack /></button><button aria-label="Previous frame" title="Previous frame · ←" onClick={() => stepFrame(-1)}><ChevronLeft /></button><button className="play" aria-label={(viewerMode === "program" ? sequencePlaying : sourcePlaying) ? "Pause" : "Play"} title="Play/Pause · Space" onClick={previewSequence}>{(viewerMode === "program" ? sequencePlaying : sourcePlaying) ? <Pause /> : <Play />}</button><button aria-label="Next frame" title="Next frame · →" onClick={() => stepFrame(1)}><ChevronRight /></button><button aria-label="Next edit" title="Next edit · ↓" onClick={() => advancePreview(1)}><SkipForward /></button></div>
          <time>{formatTimecode(viewerCurrentTime)}</time>
          <div>{viewerMode === "program" && <><button title="Mark In · I" onClick={() => selected && patchSelected({ trimStart: Math.min(previewTime, selected.sourceDuration - selected.edit.trimEnd - .1) })}>I</button><button title="Mark Out · O" onClick={() => selected && patchSelected({ trimEnd: Math.max(0, selected.sourceDuration - previewTime) })}>O</button><button title="Add marker · M" onClick={() => addMarker()}><Flag /></button></>}</div>
        </div>
        <div className="viewer-status"><span>{viewerMode === "program" ? "Program monitor · edited sequence" : sourceReference ? "Reference viewer · generation input" : "Source viewer · preserved master"}</span><span>{formatTimecode(viewerDuration)} total</span></div>
      </main>

      {showInspector && <aside className="editor-inspector">
        <div className="editor-panel-tabs"><button className={inspectorTab === "video" ? "active" : ""} onClick={() => setInspectorTab("video")}><Video /> Video</button><button className={inspectorTab === "audio" ? "active" : ""} onClick={() => setInspectorTab("audio")}><AudioLines /> Audio</button><button className={inspectorTab === "info" ? "active" : ""} onClick={() => setInspectorTab("info")}><Info /> Info</button></div>
        <header><span>Timeline selection</span><strong>{selected?.edit.label || selected?.clip.title || "No selection"}</strong><small>{selected ? `${formatTimecode(selected.outputDuration)} · ${selected.versionLabel}` : "Select an edit in the primary storyline"}</small></header>
        {selected ? <div className="editor-inspector-body">
          {inspectorTab === "video" && <>
            <InspectorSection title="Source" defaultOpen><label className="wide">Preserved version<select value={selected.edit.sourceVersionId} onChange={(event) => patchSelected({ sourceVersionId: event.target.value, trimStart: 0, trimEnd: 0 })}><option value="">Active master</option>{selected.clip.versions.map((version) => <option key={version.id} value={version.id}>{version.id === "original" ? "Original master" : `Version ${version.id}`} · {version.durationSeconds.toFixed(1)}s</option>)}</select></label></InspectorSection>
            <InspectorSection title="Timing" defaultOpen><TimelineNumber label="Trim start" value={selected.edit.trimStart} min={0} max={Math.max(0, selected.sourceDuration - selected.edit.trimEnd - .1)} step={1 / FPS} suffix="s" onChange={(trimStart) => patchSelected({ trimStart })} /><TimelineNumber label="Trim end" value={selected.edit.trimEnd} min={0} max={Math.max(0, selected.sourceDuration - selected.edit.trimStart - .1)} step={1 / FPS} suffix="s" onChange={(trimEnd) => patchSelected({ trimEnd })} /><TimelineNumber label="Speed" value={selected.edit.speed} min={.25} max={4} step={.05} suffix="×" onChange={(speed) => patchSelected({ speed })} /><div className="inspector-nudge wide"><button onClick={() => patchSelected({ trimStart: Math.max(0, selected.edit.trimStart - 1 / FPS) })}>Start −1f</button><button onClick={() => patchSelected({ trimStart: Math.min(selected.sourceDuration - selected.edit.trimEnd - .1, selected.edit.trimStart + 1 / FPS) })}>Start +1f</button><button onClick={() => patchSelected({ trimEnd: Math.min(selected.sourceDuration - selected.edit.trimStart - .1, selected.edit.trimEnd + 1 / FPS) })}>End −1f</button><button onClick={() => patchSelected({ trimEnd: Math.max(0, selected.edit.trimEnd - 1 / FPS) })}>End +1f</button></div></InspectorSection>
            <InspectorSection title="Picture fades"><TimelineNumber label="Fade in" value={selected.edit.fadeIn} min={0} max={Math.max(0, selected.outputDuration - selected.edit.fadeOut)} step={.05} suffix="s" onChange={(fadeIn) => patchSelected({ fadeIn })} /><TimelineNumber label="Fade out" value={selected.edit.fadeOut} min={0} max={Math.max(0, selected.outputDuration - selected.edit.fadeIn)} step={.05} suffix="s" onChange={(fadeOut) => patchSelected({ fadeOut })} /><button className="inspector-reset wide" onClick={() => patchSelected({ speed: 1, fadeIn: 0, fadeOut: 0 })}><RotateCcw /> Reset picture timing</button></InspectorSection>
          </>}
          {inspectorTab === "audio" && <>
            <div className="native-mix-role"><span><Volume2 /><b>Native Mix</b></span><small>H3 picture and sound stay synchronized as one preserved source. Dialogue, music, ambience, and effects are not falsely presented as separate stems.</small></div>
            <InspectorSection title="Level" defaultOpen><TimelineNumber label="Gain" value={selected.edit.audioGain} min={0} max={4} step={.05} suffix="×" onChange={(audioGain) => patchSelected({ audioGain })} /><label className="wide inspector-range">Clip level<input aria-label="Selected clip audio level" type="range" min={0} max={2} step={.01} value={Math.min(2, selected.edit.audioGain)} onChange={(event) => patchSelected({ audioGain: Number(event.target.value) })} /></label></InspectorSection>
            <InspectorSection title="Audio fades" defaultOpen><TimelineNumber label="Fade in" value={selected.edit.audioFadeIn} min={0} max={Math.max(0, selected.outputDuration - selected.edit.audioFadeOut)} step={.05} suffix="s" onChange={(audioFadeIn) => patchSelected({ audioFadeIn })} /><TimelineNumber label="Fade out" value={selected.edit.audioFadeOut} min={0} max={Math.max(0, selected.outputDuration - selected.edit.audioFadeIn)} step={.05} suffix="s" onChange={(audioFadeOut) => patchSelected({ audioFadeOut })} /><button className="inspector-reset wide" onClick={() => patchSelected({ audioGain: 1, audioFadeIn: 0, audioFadeOut: 0 })}><RotateCcw /> Reset audio</button></InspectorSection>
          </>}
          {inspectorTab === "info" && <>
            <InspectorSection title="Producer metadata" defaultOpen><label className="wide">Timeline label<input maxLength={120} value={selected.edit.label} onChange={(event) => patchSelected({ label: event.target.value })} placeholder={selected.clip.title} /></label><label className="wide">Producer notes<textarea maxLength={4000} value={selected.edit.notes} onChange={(event) => patchSelected({ notes: event.target.value })} placeholder="Performance, pacing, continuity, review notes, or handoff details…" /></label></InspectorSection>
            <InspectorSection title="Source facts" defaultOpen><dl className="inspector-facts wide"><div><dt>Master</dt><dd>{selected.clip.title}</dd></div><div><dt>Seed</dt><dd>{selected.clip.seed}</dd></div><div><dt>Source</dt><dd>{formatTimecode(selected.sourceDuration)}</dd></div><div><dt>Output</dt><dd>{formatTimecode(selected.outputDuration)}</dd></div></dl></InspectorSection>
            <label className="inspector-enable"><input type="checkbox" checked={selected.edit.enabled} onChange={(event) => patchSelected({ enabled: event.target.checked })} /> <span>{selected.edit.enabled ? <Eye /> : <EyeOff />}<b>Include in program and export</b><small>Disabling removes this decision from playback; the preserved source remains in Masters.</small></span></label>
          </>}
        </div> : <EditorEmpty text="Select a timeline edit to inspect its preserved source, timing, audio, and notes." />}
      </aside>}
    </div>

    <section className="editor-timeline-panel">
      <div className="editor-tool-bar">
        <div><button aria-label="Undo timeline change" title="Undo · ⌘Z" disabled={disabled || !undo.length} onClick={undoEdit}><Undo2 /></button><button aria-label="Redo timeline change" title="Redo · ⇧⌘Z" disabled={disabled || !redo.length} onClick={redoEdit}><Redo2 /></button><i /><button className={tool === "select" ? "active" : ""} title="Select tool · A" onClick={() => setTool("select")}><MousePointer2 /><kbd>A</kbd></button><button className={tool === "trim" ? "active" : ""} title="Trim tool · T" onClick={() => setTool("trim")}><ScanLine /><kbd>T</kbd></button><button className={tool === "blade" ? "active" : ""} title="Blade tool · B" onClick={() => setTool("blade")}><Scissors /><kbd>B</kbd></button><i /><button aria-label="Split at playhead" title="Split at playhead" disabled={!selected} onClick={split}><Scissors /></button><button aria-label="Duplicate timeline item" title="Duplicate" disabled={!selected || items.length >= 512} onClick={duplicate}><Copy /></button><button aria-label="Remove timeline item" title="Remove decision" disabled={!selected} onClick={remove}><Trash2 /></button></div>
        <div><button className={snapping ? "active" : ""} title="Snapping · N" onClick={() => setSnapping((active) => !active)}><Magnet /><kbd>N</kbd></button><button className={skimming ? "active" : ""} title="Skimming · S" onClick={() => setSkimming((active) => !active)}><Eye /><kbd>S</kbd></button><button title="Keyboard shortcuts · ?" onClick={() => setShowShortcuts((visible) => !visible)}><CircleHelp /></button><span>{items.length} edits · {formatTimecode(totalDuration)}</span><ZoomIn /><input aria-label="Timeline zoom" type="range" min={28} max={180} value={zoom} onChange={(event) => setZoom(Number(event.target.value))} /><button className="zoom-fit" onClick={() => setZoom(Math.max(28, Math.min(180, 900 / Math.max(1, totalDuration))))}>Fit</button></div>
      </div>
      {markerComposer && <div className="editor-marker-composer"><Flag /><select aria-label="Marker type" value={markerKind} onChange={(event) => setMarkerKind(event.target.value as TimelineMarker["kind"])}><option value="marker">Marker</option><option value="todo">To-do</option><option value="chapter">Chapter</option></select><input autoFocus maxLength={120} value={markerLabel} onChange={(event) => setMarkerLabel(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") addMarker(markerLabel, markerKind); if (event.key === "Escape") setMarkerComposer(false); }} placeholder={`Note at ${formatTimecode(elapsed)}`} /><button onClick={() => addMarker(markerLabel, markerKind)}>Add at playhead</button><button aria-label="Close marker composer" onClick={() => setMarkerComposer(false)}><X /></button></div>}
      {showShortcuts && <div className="editor-shortcut-map"><span><kbd>Space</kbd> Play / pause</span><span><kbd>J K L</kbd> Shuttle</span><span><kbd>← →</kbd> One frame</span><span><kbd>↑ ↓</kbd> Previous / next edit</span><span><kbd>I O</kbd> Mark in / out</span><span><kbd>M</kbd> Marker</span><span><kbd>A T B</kbd> Select / trim / blade</span><span><kbd>N S</kbd> Snapping / skimming</span><span><kbd>⌘Z</kbd> Undo</span><span><kbd>⌘S</kbd> Save</span></div>}
      <div className="editor-timeline-scroll">
        <div className="editor-ruler-row"><div className="editor-track-label"><strong>INDEX</strong><small>{normalizedValue.markers.length} markers</small></div><div className="editor-ruler" style={{ width: trackWidth }} onClick={(event) => { const rect = event.currentTarget.getBoundingClientRect(); seekGlobal(((event.clientX - rect.left) / rect.width) * totalDuration); }}>{rulerLabels(totalDuration, trackWidth).map((tick) => <span key={tick.time} style={{ left: tick.left }}>{formatTimecode(tick.time)}</span>)}{normalizedValue.markers.filter((marker) => marker.timeSeconds <= totalDuration).map((marker) => <button key={marker.id} className={marker.kind} title={`${formatTimecode(marker.timeSeconds)} · ${marker.label}`} style={{ left: marker.timeSeconds * timelineScale }} onClick={(event) => { event.stopPropagation(); seekGlobal(marker.timeSeconds); }}><Flag /></button>)}<i className="editor-playhead" style={{ left: Math.min(trackWidth, elapsed * timelineScale) }} /></div></div>
        <div className="editor-track-row picture"><div className="editor-track-label"><strong>V1</strong><small>Primary Storyline</small></div><div className="editor-track-canvas" style={{ width: trackWidth }}>{enabledItems.map((item, index) => <button key={item.edit.id} draggable={!disabled && tool === "select"} onDragStart={() => setDraggingId(item.edit.id)} onDragOver={(event) => event.preventDefault()} onDrop={() => { if (draggingId) commit(moveTimelineItem(normalizedValue, draggingId, item.edit.id)); setDraggingId(""); }} onMouseMove={(event) => handleSkim(event, item)} onMouseLeave={() => setSkimmer(undefined)} onClick={(event) => handleTimelinePointer(event, item)} className={`${selected?.edit.id === item.edit.id ? "selected" : ""} tool-${tool}`} style={{ width: Math.max(24, item.outputDuration * timelineScale) }}>
          <i className="clip-color" /><span className="clip-order">{index + 1}</span><strong>{item.edit.label || item.clip.title}</strong><small>{formatTimecode(item.outputDuration)} · {item.edit.speed}×</small><em>{item.versionLabel}</em>{item.edit.notes && <Flag className="clip-note" />}{(item.edit.fadeIn > 0 || item.edit.fadeOut > 0) && <span className="clip-fade-in" />}{(item.edit.fadeIn > 0 || item.edit.fadeOut > 0) && <span className="clip-fade-out" />}{tool === "trim" && <><i className="trim-handle start" /><i className="trim-handle end" /></>}{skimmer?.id === item.edit.id && <i className="timeline-skimmer" style={{ left: `${skimmer.fraction * 100}%` }} />}
        </button>)}<i className="editor-playhead" style={{ left: Math.min(trackWidth, elapsed * timelineScale) }} /></div></div>
        <div className="editor-track-row audio"><div className="editor-track-label"><strong>A1</strong><small>Native Mix</small></div><div className="editor-track-canvas" style={{ width: trackWidth }}>{enabledItems.map((item) => <div key={item.edit.id} className={selected?.edit.id === item.edit.id ? "selected" : ""} style={{ width: Math.max(24, item.outputDuration * timelineScale) }}><Volume2 /><span className="audio-waveform" style={{ opacity: Math.min(1, .22 + item.edit.audioGain / 2) }} /><small>{item.edit.audioGain === 0 ? "Muted" : `${item.edit.audioGain.toFixed(2)}×`}</small></div>)}<i className="editor-playhead" style={{ left: Math.min(trackWidth, elapsed * timelineScale) }} /></div></div>
      </div>
      <footer className="editor-timeline-footer"><span><Magnet /> Magnetic storyline closes gaps automatically</span><span><Volume2 /> H3 native mix remains frame-locked</span><span><Archive /> Every source master is immutable</span></footer>
    </section>

    <section className="editor-delivery-strip">
      <div><Archive /><span><strong>Delivery settings</strong><small>Exports are new immutable files with SHA-256 and a JSON decision list.</small></span></div>
      <label>File title<input value={normalizedValue.exportTitle} maxLength={120} onChange={(event) => commit({ ...normalizedValue, exportTitle: event.target.value })} /></label>
      <label>Preset<select value={normalizedValue.exportPreset} onChange={(event) => commit({ ...normalizedValue, exportPreset: event.target.value as MovieEdit["exportPreset"] })}><option value="archive">Archive · CRF 14 / 320 kbps</option><option value="publish">Publish · CRF 18 / 192 kbps</option><option value="review">Review · CRF 24 / 128 kbps</option></select></label>
      <label className="delivery-normalize"><input type="checkbox" checked={normalizedValue.normalizeAudio} onChange={(event) => commit({ ...normalizedValue, normalizeAudio: event.target.checked })} /> Normalize program</label>
      {normalizedValue.normalizeAudio && <TimelineNumber label="Target" value={normalizedValue.targetLufs} min={-24} max={-9} step={1} suffix="LUFS" onChange={(targetLufs) => commit({ ...normalizedValue, targetLufs })} />}
      <span className="delivery-summary"><Gauge /> {formatTimecode(totalDuration)} · {enabledItems.length} active edits</span>
    </section>
  </div>;
}

function TimelineIndex({ items, markers, query, selectedId, onSelect, onSeek, onPatchMarker, onDeleteMarker }: {
  items: TimelineItem[]; markers: TimelineMarker[]; query: string; selectedId?: string;
  onSelect: (item: TimelineItem) => void; onSeek: (time: number) => void;
  onPatchMarker: (id: string, change: Partial<TimelineMarker>) => void; onDeleteMarker: (id: string) => void;
}) {
  const visibleItems = items.filter((item) => !query || `${item.edit.label} ${item.clip.title} ${item.edit.notes}`.toLocaleLowerCase().includes(query));
  const visibleMarkers = markers.filter((marker) => !query || marker.label.toLocaleLowerCase().includes(query));
  let cursor = 0;
  return <div className="editor-index"><h4>Storyline</h4>{visibleItems.map((item, index) => { const start = cursor; cursor += item.edit.enabled ? item.outputDuration : 0; return <button key={item.edit.id} className={selectedId === item.edit.id ? "selected" : ""} onClick={() => onSelect(item)}><b>{index + 1}</b><span><strong>{item.edit.label || item.clip.title}</strong><small>{formatTimecode(start)} · {formatTimecode(item.outputDuration)}</small>{item.edit.notes && <em>{item.edit.notes}</em>}</span></button>; })}<h4>Markers & to-dos</h4>{visibleMarkers.map((marker) => <article key={marker.id} className={`${marker.kind} ${marker.completed ? "complete" : ""}`}><button aria-label={`Go to ${marker.label}`} onClick={() => onSeek(marker.timeSeconds)}><Flag /></button><span><input aria-label={`Marker label at ${formatTimecode(marker.timeSeconds)}`} maxLength={120} value={marker.label} onChange={(event) => onPatchMarker(marker.id, { label: event.target.value })} /><small>{formatTimecode(marker.timeSeconds)} · {marker.kind}</small></span>{marker.kind === "todo" && <button aria-label={`Mark ${marker.label} complete`} onClick={() => onPatchMarker(marker.id, { completed: !marker.completed })}><Check /></button>}<button aria-label={`Delete ${marker.label}`} onClick={() => onDeleteMarker(marker.id)}><Trash2 /></button></article>)}{!visibleItems.length && !visibleMarkers.length && <EditorEmpty text="No timeline items match this search." />}</div>;
}

function SourceViewer({ reference, path, time, duration, videoRef, playing, onPlaying, onTime }: {
  reference?: MovieReference; path: string; time: number; duration: number; videoRef: React.RefObject<HTMLVideoElement | null>; playing: boolean;
  onPlaying: (playing: boolean) => void; onTime: (time: number) => void;
}) {
  const mediaUrl = movieMediaUrl(path);
  if (!mediaUrl) return <EditorMonitorEmpty source />;
  if (reference?.kind === "image") return <img src={mediaUrl} alt={reference.name} />;
  if (reference?.kind === "audio") return <div className="source-audio-view"><AudioLines /><strong>{reference.name}</strong><audio ref={videoRef as unknown as React.RefObject<HTMLAudioElement>} controls src={mediaUrl} onTimeUpdate={(event) => onTime(event.currentTarget.currentTime)} /></div>;
  return <video key={path} ref={videoRef} src={mediaUrl} preload="metadata" onLoadedMetadata={(event) => { event.currentTarget.currentTime = Math.min(time, duration); if (playing) void event.currentTarget.play().catch(() => onPlaying(false)); }} onPlay={() => onPlaying(true)} onPause={() => onPlaying(false)} onTimeUpdate={(event) => onTime(event.currentTarget.currentTime)} />;
}

function ReferenceThumb({ reference }: { reference: MovieReference }) {
  const mediaUrl = movieMediaUrl(reference.path);
  return <span className={`editor-media-thumb reference ${reference.kind}`}>{reference.kind === "image" && mediaUrl ? <img src={mediaUrl} alt="" /> : reference.kind === "video" ? <Video /> : reference.kind === "audio" ? <AudioLines /> : <Images />}</span>;
}

function InspectorSection({ title, defaultOpen = false, children }: { title: string; defaultOpen?: boolean; children: React.ReactNode }) {
  return <details className="inspector-section" open={defaultOpen}><summary>{title}<ChevronRight /></summary><div>{children}</div></details>;
}

function EditorEmpty({ text }: { text: string }) {
  return <div className="editor-empty"><Film /><span>{text}</span></div>;
}

function EditorMonitorEmpty({ source = false }: { source?: boolean }) {
  return <div className="editor-monitor-empty"><Play /><strong>{source ? "Select a master or reference" : "No enabled timeline media"}</strong><span>{source ? "Choose media in the browser to audition it here." : "Append a preserved master to the primary storyline."}</span></div>;
}

function TimelineNumber({ label, value, min, max, step, suffix, onChange }: {
  label: string; value: number; min: number; max: number; step: number; suffix: string; onChange: (value: number) => void;
}) {
  return <label>{label}<span className="timeline-number"><input type="number" value={Number.isFinite(value) ? Number(value.toFixed(3)) : 0} min={min} max={max} step={step} onChange={(event) => { if (event.target.value === "") return; const parsed = Number(event.target.value); if (Number.isFinite(parsed)) onChange(Math.min(max, Math.max(min, parsed))); }} /><b>{suffix}</b></span></label>;
}

function rulerLabels(duration: number, width: number): Array<{ time: number; left: number }> {
  if (duration <= 0) return [{ time: 0, left: 0 }];
  const targetTicks = Math.max(2, Math.floor(width / 150));
  const rough = duration / targetTicks;
  const steps = [.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600];
  const step = steps.find((candidate) => candidate >= rough) ?? 600;
  const ticks = [];
  for (let time = 0; time <= duration + .001; time += step) ticks.push({ time, left: (time / duration) * width });
  return ticks;
}

export function formatTime(seconds: number): string {
  const safe = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
  const minutes = Math.floor(safe / 60);
  const remainder = safe - minutes * 60;
  return `${String(minutes).padStart(2, "0")}:${remainder.toFixed(2).padStart(5, "0")}`;
}

export function formatTimecode(seconds: number, fps = FPS): string {
  const safe = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
  const frames = Math.round(safe * fps);
  const frame = frames % fps;
  const wholeSeconds = Math.floor(frames / fps);
  const second = wholeSeconds % 60;
  const minutes = Math.floor(wholeSeconds / 60);
  const minute = minutes % 60;
  const hour = Math.floor(minutes / 60);
  return `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}:${String(second).padStart(2, "0")}:${String(frame).padStart(2, "0")}`;
}
