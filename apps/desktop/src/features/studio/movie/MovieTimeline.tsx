import {
  AudioLines, Compass, Eye, EyeOff, Film, Layers, Lock, Pin, Plus, Redo2,
  Save, Scissors, Sparkles, Trash2, Undo2, Unlock, Video, Volume2, VolumeX,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { GenerateAssetButton } from "./MovieAssetCreator";
import type { H3MonitorSource } from "./MovieH3Preview";
import type {
  H3AudioClip, H3DriverClip, H3GuideItem, H3PromptSection, H3ReferenceItemVerdict,
  H3ReferenceRole, H3RetentionLevel, H3Scene, H3StagedReferenceItem, H3VideoClip,
  H3WindowCompileResult, H3WindowConfig, MovieProject,
} from "../../../contracts/index";

const FPS = 24;

export function snapToFrame(seconds: number, fps = FPS): number {
  const safe = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
  return Math.round(safe * fps) / fps;
}

export function formatTimecode(seconds: number, fps = FPS): string {
  const safe = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
  const totalFrames = Math.round(safe * fps);
  const frames = totalFrames % fps;
  const totalSeconds = Math.floor(totalFrames / fps);
  const secs = totalSeconds % 60;
  const mins = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3600);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(hours)}:${pad(mins)}:${pad(secs)}:${pad(frames)}`;
}

export function clampSceneWindow(
  window: H3WindowConfig,
  patch: Partial<H3WindowConfig>,
  sceneDurationSec: number,
): H3WindowConfig {
  const maxDur = Math.max(1, sceneDurationSec);
  const next = { ...window, ...patch };
  let inSec = snapToFrame(Math.min(Math.max(0, next.inSec), maxDur - 5 / FPS));
  let outSec = snapToFrame(Math.min(Math.max(inSec + 5 / FPS, next.outSec), maxDur));
  if (outSec - inSec > 15) {
    if (patch.inSec !== undefined && patch.outSec === undefined) {
      outSec = snapToFrame(Math.min(maxDur, inSec + 15));
    } else {
      inSec = snapToFrame(Math.max(0, outSec - 15));
    }
  }
  return {
    ...next,
    inSec,
    outSec,
    preContextSec: snapToFrame(Math.min(5, Math.max(0, next.preContextSec))),
    postContextSec: snapToFrame(Math.min(5, Math.max(0, next.postContextSec))),
  };
}

export function splitH3VideoClip(
  scene: H3Scene,
  laneId: string,
  clipId: string,
  splitTimelineSec: number,
  nextId: string,
): H3Scene {
  return {
    ...scene,
    videoLanes: scene.videoLanes.map((lane) => {
      if (lane.id !== laneId || lane.locked) return lane;
      const idx = lane.clips.findIndex((c) => c.id === clipId);
      if (idx < 0) return lane;
      const target = lane.clips[idx];
      const cut = snapToFrame(splitTimelineSec);
      const minCut = target.timelineStartSec + 5 / FPS;
      const maxCut = target.timelineStartSec + target.durationSec - 5 / FPS;
      if (cut < minCut || cut > maxCut) return lane;
      const leftDur = snapToFrame(cut - target.timelineStartSec);
      const rightDur = snapToFrame(target.durationSec - leftDur);
      const first: H3VideoClip = {
        ...target,
        durationSec: leftDur,
      };
      const second: H3VideoClip = {
        ...target,
        id: nextId,
        timelineStartSec: cut,
        sourceInSec: snapToFrame(target.sourceInSec + leftDur),
        durationSec: rightDur,
      };
      const clips = [...lane.clips];
      clips.splice(idx, 1, first, second);
      return { ...lane, clips };
    }),
  };
}

export function activeMonitorSourceAtPlayhead(
  project: MovieProject,
  scene: H3Scene,
  playheadSec: number,
  assetPaths?: Map<string, string>,
): H3MonitorSource | undefined {
  const anySolo = scene.videoLanes.some((l) => l.solo);
  const lanes = [...scene.videoLanes].reverse();
  for (const lane of lanes) {
    if (lane.muted || (anySolo && !lane.solo)) continue;
    const hit = lane.clips.find(
      (c) =>
        c.enabled &&
        playheadSec >= c.timelineStartSec - 1e-4 &&
        playheadSec < c.timelineStartSec + c.durationSec + 1e-4,
    );
    if (!hit) continue;
    let mediaPath = "";
    if (hit.sourceKind === "take") {
      mediaPath =
        project.renderJobs.find((j) => j.id === hit.sourceId)?.takeMediaPath ??
        project.clips.find((c) => c.id === hit.sourceId)?.path ??
        "";
    } else {
      mediaPath =
        assetPaths?.get(hit.sourceId) ??
        project.references.find((r) => r.assetId === hit.sourceId)?.path ??
        "";
    }
    return {
      id: hit.sourceId,
      label: hit.label || lane.name,
      mediaPath,
      sourceOffsetSec: hit.sourceInSec,
      timelineStartSec: hit.timelineStartSec,
      durationSec: hit.durationSec,
      sourceKind: hit.sourceKind === "take" ? "renderedTake" : "referenceAsset",
    };
  }
  return undefined;
}

function makeId(prefix: string): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? `${prefix}-${crypto.randomUUID().slice(0, 8)}`
    : `${prefix}-${Date.now().toString(36)}-${Math.random().toString(16).slice(2, 6)}`;
}

type SelectedItem =
  | { kind: "shot"; id: string }
  | { kind: "guide"; id: string }
  | { kind: "ref"; laneId: string; id: string }
  | { kind: "video"; laneId: string; id: string }
  | { kind: "driver"; laneId: string; id: string }
  | { kind: "audio"; laneId: string; id: string };

export function MovieTimeline({
  project,
  scene,
  savedScene,
  playheadSec,
  compileResult,
  selectedShotId,
  disabled,
  dirty,
  onSceneChange,
  onSaveScene,
  onPlayhead,
  onSelectShot,
}: {
  project: MovieProject;
  scene: H3Scene;
  savedScene?: H3Scene;
  playheadSec: number;
  compileResult?: H3WindowCompileResult | null;
  selectedShotId?: string;
  disabled?: boolean;
  dirty?: boolean;
  onSceneChange: (next: H3Scene) => void;
  onSaveScene?: () => void;
  onPlayhead: (sec: number) => void;
  onSelectShot?: (sectionId: string) => void;
}) {
  const [zoom, setZoom] = useState(56);
  const [tool, setTool] = useState<"select" | "blade">("select");
  const [undo, setUndo] = useState<H3Scene[]>([]);
  const [redo, setRedo] = useState<H3Scene[]>([]);
  const [selected, setSelected] = useState<SelectedItem | null>(null);
  const [viewport, setViewport] = useState({ left: 0, width: 1200 });
  const scrollRef = useRef<HTMLDivElement>(null);

  const priorSaved = useRef(savedScene ?? scene);
  const priorValue = useRef(scene);
  useEffect(() => {
    const nextSaved = savedScene ?? scene;
    if (
      priorSaved.current.id === nextSaved.id &&
      JSON.stringify(priorSaved.current) !== JSON.stringify(nextSaved) &&
      JSON.stringify(priorValue.current) === JSON.stringify(priorSaved.current)
    ) {
      const before = priorSaved.current;
      setUndo((hist) => [...hist.slice(-49), before]);
      setRedo([]);
    }
    priorSaved.current = nextSaved;
    priorValue.current = scene;
  }, [savedScene, scene]);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const obs = new ResizeObserver(() =>
      setViewport({ left: el.scrollLeft, width: el.clientWidth || 1200 }),
    );
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  const commit = (next: H3Scene) => {
    if (disabled) return;
    setUndo((hist) => [...hist.slice(-49), scene]);
    setRedo([]);
    onSceneChange(next);
  };

  const handleUndo = () => {
    if (disabled) return;
    const prev = undo.at(-1);
    if (!prev) return;
    setUndo((h) => h.slice(0, -1));
    setRedo((h) => [...h.slice(-49), scene]);
    onSceneChange(prev);
  };

  const handleRedo = () => {
    if (disabled) return;
    const next = redo.at(-1);
    if (!next) return;
    setRedo((h) => h.slice(0, -1));
    setUndo((h) => [...h.slice(-49), scene]);
    onSceneChange(next);
  };

  const totalSec = Math.max(5, scene.durationSec);
  const trackWidth = Math.max(880, totalSec * zoom);
  const pxPerSec = trackWidth / totalSec;
  const visibleStartSec = Math.max(0, (viewport.left - 500) / pxPerSec);
  const visibleEndSec = Math.min(totalSec + 5, (viewport.left + viewport.width + 500) / pxPerSec);

  const rulerTicks = useMemo(() => {
    const visibleSpan = Math.max(1, visibleEndSec - visibleStartSec);
    const rawStep = visibleSpan / 24;
    const steps = [0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600];
    const step = steps.find((s) => s >= rawStep) ?? 600;
    const first = Math.floor(visibleStartSec / step) * step;
    const out: number[] = [];
    for (let t = first; t <= Math.min(totalSec, visibleEndSec); t += step) {
      out.push(Number(t.toFixed(2)));
      if (out.length >= 36) break;
    }
    return out;
  }, [totalSec, visibleEndSec, visibleStartSec]);

  const verdictMap = useMemo(() => {
    const map = new Map<string, H3ReferenceItemVerdict>();
    for (const v of compileResult?.verdicts ?? []) {
      map.set(v.itemId, v);
    }
    return map;
  }, [compileResult]);

  const entityMap = useMemo(
    () => new Map(project.entities.map((e) => [e.id, e])),
    [project.entities],
  );

  const inPx = scene.window.inSec * pxPerSec;
  const outPx = scene.window.outSec * pxPerSec;
  const prePx = Math.max(0, scene.window.inSec - scene.window.preContextSec) * pxPerSec;
  const postPx = Math.min(totalSec, scene.window.outSec + scene.window.postContextSec) * pxPerSec;
  const playheadPx = Math.min(totalSec, Math.max(0, playheadSec)) * pxPerSec;

  const setWindowPatch = (patch: Partial<H3WindowConfig>) => {
    commit({ ...scene, window: clampSceneWindow(scene.window, patch, totalSec) });
  };

  const addLane = (kind: "video" | "audio" | "ref" | "driver") => {
    if (kind === "video") {
      const idx = scene.videoLanes.length + 1;
      commit({
        ...scene,
        videoLanes: [
          ...scene.videoLanes,
          { id: makeId("vid-lane"), name: `V${idx}`, muted: false, solo: false, locked: false, clips: [] },
        ],
      });
    } else if (kind === "audio") {
      const idx = scene.audioLanes.length + 1;
      commit({
        ...scene,
        audioLanes: [
          ...scene.audioLanes,
          { id: makeId("aud-lane"), name: `A${idx}`, muted: false, solo: false, locked: false, clips: [] },
        ],
      });
    } else if (kind === "driver") {
      const idx = scene.driverLanes.length + 1;
      commit({
        ...scene,
        driverLanes: [
          ...scene.driverLanes,
          { id: makeId("drv-lane"), name: `D${idx}`, muted: false, locked: false, clips: [] },
        ],
      });
    } else {
      const idx = scene.referenceLanes.length + 1;
      commit({
        ...scene,
        referenceLanes: [
          ...scene.referenceLanes,
          { id: makeId("ref-lane"), name: `R${idx}`, muted: false, locked: false, items: [] },
        ],
      });
    }
  };

  const addShotAtWindow = () => {
    const id = makeId("shot");
    const nextSec: H3PromptSection = {
      id,
      label: `Shot ${scene.promptSections.length + 1}`,
      timelineStartSec: scene.window.inSec,
      durationSec: Math.max(0.5, scene.window.outSec - scene.window.inSec),
      summaryBeat: "",
      detailedAction: "",
      cameraAndLighting: "",
      soundscapeOverride: "",
      musicOverride: "",
      vocalEvents: [],
      enabled: true,
    };
    commit({
      ...scene,
      promptSections: [...scene.promptSections, nextSec].sort((a, b) => a.timelineStartSec - b.timelineStartSec),
    });
    setSelected({ kind: "shot", id });
    onSelectShot?.(id);
  };

  const selectedRefItem: { laneId: string; item: H3StagedReferenceItem } | undefined = (() => {
    if (selected?.kind !== "ref") return undefined;
    const lane = scene.referenceLanes.find((l) => l.id === selected.laneId);
    const item = lane?.items.find((i) => i.id === selected.id);
    return lane && item ? { laneId: lane.id, item } : undefined;
  })();

  const selectedVideoClip: { laneId: string; clip: H3VideoClip } | undefined = (() => {
    if (selected?.kind !== "video") return undefined;
    const lane = scene.videoLanes.find((l) => l.id === selected.laneId);
    const clip = lane?.clips.find((c) => c.id === selected.id);
    return lane && clip ? { laneId: lane.id, clip } : undefined;
  })();

  const selectedDriverClip: { laneId: string; clip: H3DriverClip } | undefined = (() => {
    if (selected?.kind !== "driver") return undefined;
    const lane = scene.driverLanes.find((l) => l.id === selected.laneId);
    const clip = lane?.clips.find((c) => c.id === selected.id);
    return lane && clip ? { laneId: lane.id, clip } : undefined;
  })();

  const selectedAudioClip: { laneId: string; clip: H3AudioClip } | undefined = (() => {
    if (selected?.kind !== "audio") return undefined;
    const lane = scene.audioLanes.find((l) => l.id === selected.laneId);
    const clip = lane?.clips.find((c) => c.id === selected.id);
    return lane && clip ? { laneId: lane.id, clip } : undefined;
  })();

  const selectedGuide: H3GuideItem | undefined =
    selected?.kind === "guide" ? scene.guides.find((g) => g.id === selected.id) : undefined;

  const updateRefItem = (laneId: string, itemId: string, patch: Partial<H3StagedReferenceItem>) => {
    commit({
      ...scene,
      referenceLanes: scene.referenceLanes.map((l) =>
        l.id === laneId
          ? { ...l, items: l.items.map((it) => (it.id === itemId ? { ...it, ...patch } : it)) }
          : l,
      ),
    });
  };

  const deleteRefItem = (laneId: string, itemId: string) => {
    commit({
      ...scene,
      referenceLanes: scene.referenceLanes.map((l) =>
        l.id === laneId ? { ...l, items: l.items.filter((it) => it.id !== itemId) } : l,
      ),
    });
    setSelected(null);
  };

  const updateVideoClip = (laneId: string, clipId: string, patch: Partial<H3VideoClip>) => {
    commit({
      ...scene,
      videoLanes: scene.videoLanes.map((l) =>
        l.id === laneId
          ? { ...l, clips: l.clips.map((c) => (c.id === clipId ? { ...c, ...patch } : c)) }
          : l,
      ),
    });
  };

  const deleteVideoClip = (laneId: string, clipId: string) => {
    commit({
      ...scene,
      videoLanes: scene.videoLanes.map((l) =>
        l.id === laneId ? { ...l, clips: l.clips.filter((c) => c.id !== clipId) } : l,
      ),
    });
    setSelected(null);
  };

  const updateDriverClip = (laneId: string, clipId: string, patch: Partial<H3DriverClip>) => {
    commit({
      ...scene,
      driverLanes: scene.driverLanes.map((l) =>
        l.id === laneId
          ? { ...l, clips: l.clips.map((c) => (c.id === clipId ? { ...c, ...patch } : c)) }
          : l,
      ),
    });
  };

  const deleteDriverClip = (laneId: string, clipId: string) => {
    commit({
      ...scene,
      driverLanes: scene.driverLanes.map((l) =>
        l.id === laneId ? { ...l, clips: l.clips.filter((c) => c.id !== clipId) } : l,
      ),
    });
    setSelected(null);
  };

  const updateAudioClip = (laneId: string, clipId: string, patch: Partial<H3AudioClip>) => {
    commit({
      ...scene,
      audioLanes: scene.audioLanes.map((l) =>
        l.id === laneId
          ? { ...l, clips: l.clips.map((c) => (c.id === clipId ? { ...c, ...patch } : c)) }
          : l,
      ),
    });
  };

  const deleteAudioClip = (laneId: string, clipId: string) => {
    commit({
      ...scene,
      audioLanes: scene.audioLanes.map((l) =>
        l.id === laneId ? { ...l, clips: l.clips.filter((c) => c.id !== clipId) } : l,
      ),
    });
    setSelected(null);
  };

  return (
    <section className="h3-timeline-shell" aria-label="MiniMax H3 Multi-Lane Timeline">
      <div className="h3-timeline-toolbar">
        <div className="h3-toolbar-group">
          <button
            type="button"
            aria-label="Undo timeline change"
            disabled={Boolean(disabled || undo.length === 0)}
            onClick={handleUndo}
          >
            <Undo2 size={13} />
          </button>
          <button
            type="button"
            aria-label="Redo timeline change"
            disabled={Boolean(disabled || redo.length === 0)}
            onClick={handleRedo}
          >
            <Redo2 size={13} />
          </button>
          <i />
          <button
            type="button"
            className={tool === "select" ? "active" : ""}
            onClick={() => setTool("select")}
          >
            Select
          </button>
          <button
            type="button"
            className={tool === "blade" ? "active" : ""}
            onClick={() => setTool("blade")}
          >
            <Scissors size={12} /> Blade
          </button>
          <i />
          <button
            type="button"
            disabled={disabled}
            title="Set window In point to current playhead"
            onClick={() => setWindowPatch({ inSec: playheadSec })}
          >
            [ In = {scene.window.inSec.toFixed(2)}s
          </button>
          <button
            type="button"
            disabled={disabled}
            title="Set window Out point to current playhead"
            onClick={() => setWindowPatch({ outSec: Math.max(playheadSec, scene.window.inSec + 0.5) })}
          >
            Out = {scene.window.outSec.toFixed(2)}s ]
          </button>
          <label className="h3-inline-num">
            Pre
            <input
              aria-label="Pre-context seconds"
              type="number"
              min={0}
              max={5}
              step={0.25}
              disabled={disabled}
              value={scene.window.preContextSec}
              onChange={(e) => setWindowPatch({ preContextSec: Number(e.target.value) })}
            />
            s
          </label>
          <label className="h3-inline-num">
            Post
            <input
              aria-label="Post-context seconds"
              type="number"
              min={0}
              max={5}
              step={0.25}
              disabled={disabled}
              value={scene.window.postContextSec}
              onChange={(e) => setWindowPatch({ postContextSec: Number(e.target.value) })}
            />
            s
          </label>
        </div>

        <div className="h3-toolbar-group">
          <button type="button" disabled={disabled} onClick={addShotAtWindow}>
            <Sparkles size={12} /> + Shot
          </button>
          <GenerateAssetButton
            label="Pin Guide"
            disabled={disabled}
            onUse={(asset) => {
              const g: H3GuideItem = {
                id: makeId("guide"),
                assetId: asset.id,
                label: asset.name,
                timelineSec: snapToFrame(playheadSec),
                useImage: asset.kind !== "audio",
                useAudio: asset.kind === "audio",
                enabled: true,
              };
              commit({ ...scene, guides: [...scene.guides, g] });
            }}
          />
          <button type="button" disabled={disabled} onClick={() => addLane("ref")}>
            <Plus size={12} /> R
          </button>
          <button type="button" disabled={disabled} onClick={() => addLane("driver")}>
            <Plus size={12} /> D
          </button>
          <button type="button" disabled={disabled} onClick={() => addLane("video")}>
            <Plus size={12} /> V
          </button>
          <button type="button" disabled={disabled} onClick={() => addLane("audio")}>
            <Plus size={12} /> A
          </button>
          <label className="h3-inline-num">
            Dur
            <input
              aria-label="Scene duration seconds"
              type="number"
              min={2}
              max={600}
              step={1}
              disabled={disabled}
              value={scene.durationSec}
              onChange={(e) => commit({ ...scene, durationSec: Math.max(2, Number(e.target.value)) })}
            />
            s
          </label>
          <input
            type="range"
            aria-label="Timeline zoom"
            min={16}
            max={140}
            value={zoom}
            onChange={(e) => setZoom(Number(e.target.value))}
          />
          {onSaveScene && (
            <button
              type="button"
              className={dirty ? "accent" : ""}
              disabled={Boolean(disabled || !dirty)}
              onClick={onSaveScene}
            >
              <Save size={12} /> Save Scene
            </button>
          )}
        </div>
      </div>

      <div
        ref={scrollRef}
        className="h3-timeline-scroll"
        aria-label="Timeline tracks"
        onScroll={(e) =>
          setViewport({
            left: e.currentTarget.scrollLeft,
            width: e.currentTarget.clientWidth || 1200,
          })
        }
      >
        {/* Ruler + Window [In, Out] Bar */}
        <div className="h3-track-row ruler">
          <div className="h3-track-header">
            <strong>{formatTimecode(playheadSec)}</strong>
            <small>Win {(scene.window.outSec - scene.window.inSec).toFixed(2)}s</small>
          </div>
          <div
            className="h3-ruler-canvas"
            style={{ width: `${trackWidth}px` }}
            onClick={(e) => {
              const rect = e.currentTarget.getBoundingClientRect();
              const x = Math.max(0, e.clientX - rect.left);
              onPlayhead(snapToFrame(Math.min(totalSec, x / pxPerSec)));
            }}
          >
            {scene.window.preContextSec > 0 && (
              <div
                className="h3-context-bar pre"
                style={{ left: `${prePx}px`, width: `${Math.max(0, inPx - prePx)}px` }}
                title={`Pre-context freeze (${scene.window.preContextSec.toFixed(2)}s)`}
              />
            )}
            <div
              className="h3-window-bar"
              style={{ left: `${inPx}px`, width: `${Math.max(4, outPx - inPx)}px` }}
              title={`Active H3 Window [${scene.window.inSec.toFixed(2)}s .. ${scene.window.outSec.toFixed(2)}s]`}
            >
              <span>[In {scene.window.inSec.toFixed(2)}s — Out {scene.window.outSec.toFixed(2)}s]</span>
            </div>
            {scene.window.postContextSec > 0 && (
              <div
                className="h3-context-bar post"
                style={{ left: `${outPx}px`, width: `${Math.max(0, postPx - outPx)}px` }}
                title={`Post-context freeze (${scene.window.postContextSec.toFixed(2)}s)`}
              />
            )}
            {rulerTicks.map((t) => (
              <span key={t} className="h3-ruler-tick" style={{ left: `${t * pxPerSec}px` }}>
                {t}s
              </span>
            ))}
            <div className="h3-playhead" style={{ left: `${playheadPx}px` }} />
          </div>
        </div>

        {/* Prompt Shots Track */}
        <div className="h3-track-row shots">
          <div className="h3-track-header">
            <strong><Sparkles size={11} /> Shots</strong>
            <small>{scene.promptSections.length} beats</small>
          </div>
          <div className="h3-lane-canvas compact" style={{ width: `${trackWidth}px` }}>
            {scene.promptSections
              .filter(
                (s) =>
                  s.timelineStartSec + s.durationSec >= visibleStartSec && s.timelineStartSec <= visibleEndSec,
              )
              .map((sec, idx) => (
                <button
                  key={sec.id}
                  type="button"
                  className={`h3-clip-block shot ${selectedShotId === sec.id || (selected?.kind === "shot" && selected.id === sec.id) ? "selected" : ""}`}
                  style={{
                    left: `${sec.timelineStartSec * pxPerSec}px`,
                    width: `${Math.max(24, sec.durationSec * pxPerSec - 2)}px`,
                  }}
                  onClick={() => {
                    setSelected({ kind: "shot", id: sec.id });
                    onSelectShot?.(sec.id);
                  }}
                  onDoubleClick={() =>
                    setWindowPatch({
                      inSec: sec.timelineStartSec,
                      outSec: sec.timelineStartSec + sec.durationSec,
                    })
                  }
                  title="Click to edit shot prompt; double-click to snap [In, Out] to this shot"
                >
                  <strong>[Shot {idx + 1}] {sec.label}</strong>
                  <small>{sec.summaryBeat || sec.detailedAction || "Click to author beat…"}</small>
                </button>
              ))}
            <div className="h3-playhead" style={{ left: `${playheadPx}px` }} />
          </div>
        </div>

        {/* Guides Track */}
        <div className="h3-track-row guides">
          <div className="h3-track-header">
            <strong><Pin size={11} /> Guides</strong>
            <small>{scene.guides.length} pins</small>
          </div>
          <div className="h3-lane-canvas compact" style={{ width: `${trackWidth}px` }}>
            {scene.guides
              .filter((g) => g.timelineSec >= visibleStartSec && g.timelineSec <= visibleEndSec)
              .map((g) => (
                <button
                  key={g.id}
                  type="button"
                  className={`h3-guide-pin ${selected?.kind === "guide" && selected.id === g.id ? "selected" : ""}`}
                  style={{ left: `${g.timelineSec * pxPerSec}px` }}
                  onClick={() => setSelected({ kind: "guide", id: g.id })}
                  title={`${g.label} @ ${g.timelineSec.toFixed(2)}s`}
                >
                  <Pin size={11} /> {g.label || "Guide"} ({g.timelineSec.toFixed(2)}s)
                </button>
              ))}
            <div className="h3-playhead" style={{ left: `${playheadPx}px` }} />
          </div>
        </div>

        {/* Reference Lanes R1..Rn */}
        {scene.referenceLanes.map((lane) => (
          <div key={lane.id} className={`h3-track-row ref ${lane.muted ? "muted" : ""}`}>
            <div className="h3-track-header">
              <div className="h3-lane-title-row">
                <strong><Layers size={11} /> {lane.name}</strong>
                <div className="h3-lane-toggles">
                  <button
                    type="button"
                    aria-label={`${lane.muted ? "Unmute" : "Mute"} ${lane.name}`}
                    onClick={() =>
                      commit({
                        ...scene,
                        referenceLanes: scene.referenceLanes.map((l) =>
                          l.id === lane.id ? { ...l, muted: !l.muted } : l,
                        ),
                      })
                    }
                  >
                    {lane.muted ? <EyeOff size={11} /> : <Eye size={11} />}
                  </button>
                  <button
                    type="button"
                    aria-label={`${lane.locked ? "Unlock" : "Lock"} ${lane.name}`}
                    onClick={() =>
                      commit({
                        ...scene,
                        referenceLanes: scene.referenceLanes.map((l) =>
                          l.id === lane.id ? { ...l, locked: !l.locked } : l,
                        ),
                      })
                    }
                  >
                    {lane.locked ? <Lock size={11} /> : <Unlock size={11} />}
                  </button>
                </div>
              </div>
              <small>References ({lane.items.length})</small>
            </div>
            <div className="h3-lane-canvas" style={{ width: `${trackWidth}px` }}>
              {lane.items
                .filter(
                  (it) =>
                    it.timelineStartSec + it.durationSec >= visibleStartSec &&
                    it.timelineStartSec <= visibleEndSec,
                )
                .map((it) => {
                  const ent = it.entityId ? entityMap.get(it.entityId) : undefined;
                  const verdict = verdictMap.get(it.id);
                  const vKind = verdict?.verdict ?? "outside";
                  return (
                    <button
                      key={it.id}
                      type="button"
                      className={`h3-clip-block ref verdict-${vKind} ${selected?.kind === "ref" && selected.id === it.id ? "selected" : ""}`}
                      style={{
                        left: `${it.timelineStartSec * pxPerSec}px`,
                        width: `${Math.max(32, it.durationSec * pxPerSec - 2)}px`,
                      }}
                      onClick={() => setSelected({ kind: "ref", laneId: lane.id, id: it.id })}
                      title={verdict?.detail || (ent ? `@${ent.handle}` : it.assetId ?? "Reference")}
                    >
                      <span className="h3-ref-top">
                        <strong>{ent ? `@${ent.handle}` : verdict?.label || "Asset"}</strong>
                        {verdict && <em className={`verdict-pill ${vKind}`}>{vKind}</em>}
                      </span>
                      <small>
                        {verdict && verdict.assignedTags.length > 0
                          ? verdict.assignedTags.join(" · ")
                          : `${it.timelineStartSec.toFixed(1)}s–${(it.timelineStartSec + it.durationSec).toFixed(1)}s`}
                      </small>
                    </button>
                  );
                })}
              <div className="h3-playhead" style={{ left: `${playheadPx}px` }} />
            </div>
          </div>
        ))}

        {/* Driver Lanes D1..Dn */}
        {scene.driverLanes.map((lane) => (
          <div key={lane.id} className={`h3-track-row driver ${lane.muted ? "muted" : ""}`}>
            <div className="h3-track-header">
              <div className="h3-lane-title-row">
                <strong><Compass size={11} /> {lane.name}</strong>
                <div className="h3-lane-toggles">
                  <button
                    type="button"
                    aria-label={`${lane.muted ? "Unmute" : "Mute"} ${lane.name}`}
                    onClick={() =>
                      commit({
                        ...scene,
                        driverLanes: scene.driverLanes.map((l) =>
                          l.id === lane.id ? { ...l, muted: !l.muted } : l,
                        ),
                      })
                    }
                  >
                    {lane.muted ? <EyeOff size={11} /> : <Eye size={11} />}
                  </button>
                  <button
                    type="button"
                    aria-label={`${lane.locked ? "Unlock" : "Lock"} ${lane.name}`}
                    onClick={() =>
                      commit({
                        ...scene,
                        driverLanes: scene.driverLanes.map((l) =>
                          l.id === lane.id ? { ...l, locked: !l.locked } : l,
                        ),
                      })
                    }
                  >
                    {lane.locked ? <Lock size={11} /> : <Unlock size={11} />}
                  </button>
                </div>
              </div>
              <small>Swap / Pose ({lane.clips.length})</small>
            </div>
            <div className="h3-lane-canvas compact" style={{ width: `${trackWidth}px` }}>
              {lane.clips
                .filter(
                  (c) =>
                    c.timelineStartSec + c.durationSec >= visibleStartSec &&
                    c.timelineStartSec <= visibleEndSec,
                )
                .map((c) => (
                  <button
                    key={c.id}
                    type="button"
                    className={`h3-clip-block driver ${selected?.kind === "driver" && selected.id === c.id ? "selected" : ""}`}
                    style={{
                      left: `${c.timelineStartSec * pxPerSec}px`,
                      width: `${Math.max(24, c.durationSec * pxPerSec - 2)}px`,
                    }}
                    onClick={() => setSelected({ kind: "driver", laneId: lane.id, id: c.id })}
                  >
                    <strong>{c.label || "Driver Video"}</strong>
                    <small>{c.durationSec.toFixed(2)}s</small>
                  </button>
                ))}
              <div className="h3-playhead" style={{ left: `${playheadPx}px` }} />
            </div>
          </div>
        ))}

        {/* Video Lanes V1..Vn */}
        {scene.videoLanes.map((lane) => (
          <div key={lane.id} className={`h3-track-row picture ${lane.muted ? "muted" : ""}`}>
            <div className="h3-track-header">
              <div className="h3-lane-title-row">
                <strong><Video size={11} /> {lane.name}</strong>
                <div className="h3-lane-toggles">
                  <button
                    type="button"
                    aria-label={`${lane.muted ? "Show" : "Hide"} ${lane.name}`}
                    onClick={() =>
                      commit({
                        ...scene,
                        videoLanes: scene.videoLanes.map((l) =>
                          l.id === lane.id ? { ...l, muted: !l.muted } : l,
                        ),
                      })
                    }
                  >
                    {lane.muted ? <EyeOff size={11} /> : <Eye size={11} />}
                  </button>
                  <button
                    type="button"
                    className={lane.solo ? "active" : ""}
                    aria-label={`Solo ${lane.name}`}
                    onClick={() =>
                      commit({
                        ...scene,
                        videoLanes: scene.videoLanes.map((l) =>
                          l.id === lane.id ? { ...l, solo: !l.solo } : l,
                        ),
                      })
                    }
                  >
                    S
                  </button>
                  <button
                    type="button"
                    aria-label={`${lane.locked ? "Unlock" : "Lock"} ${lane.name}`}
                    onClick={() =>
                      commit({
                        ...scene,
                        videoLanes: scene.videoLanes.map((l) =>
                          l.id === lane.id ? { ...l, locked: !l.locked } : l,
                        ),
                      })
                    }
                  >
                    {lane.locked ? <Lock size={11} /> : <Unlock size={11} />}
                  </button>
                </div>
              </div>
              <small>Picture ({lane.clips.length})</small>
            </div>
            <div className="h3-lane-canvas picture-canvas" style={{ width: `${trackWidth}px` }}>
              {lane.clips
                .filter(
                  (c) =>
                    c.timelineStartSec + c.durationSec >= visibleStartSec &&
                    c.timelineStartSec <= visibleEndSec,
                )
                .slice(0, 24)
                .map((c) => (
                  <button
                    key={c.id}
                    type="button"
                    className={`h3-clip-block video ${selected?.kind === "video" && selected.id === c.id ? "selected" : ""}`}
                    style={{
                      left: `${c.timelineStartSec * pxPerSec}px`,
                      width: `${Math.max(24, c.durationSec * pxPerSec - 2)}px`,
                    }}
                    onClick={(e) => {
                      if (tool === "blade" && !lane.locked) {
                        const rect = e.currentTarget.parentElement!.getBoundingClientRect();
                        const cutSec = (e.clientX - rect.left) / pxPerSec;
                        commit(splitH3VideoClip(scene, lane.id, c.id, cutSec, makeId("vclip")));
                      } else {
                        setSelected({ kind: "video", laneId: lane.id, id: c.id });
                      }
                    }}
                  >
                    <strong><Film size={11} /> {c.label || "Take"}</strong>
                    <small>{c.durationSec.toFixed(2)}s · in {c.sourceInSec.toFixed(2)}s</small>
                  </button>
                ))}
              <div className="h3-playhead" style={{ left: `${playheadPx}px` }} />
            </div>
          </div>
        ))}

        {/* Audio Lanes A1..An */}
        {scene.audioLanes.map((lane) => (
          <div key={lane.id} className={`h3-track-row audio ${lane.muted ? "muted" : ""}`}>
            <div className="h3-track-header">
              <div className="h3-lane-title-row">
                <strong><AudioLines size={11} /> {lane.name}</strong>
                <div className="h3-lane-toggles">
                  <button
                    type="button"
                    aria-label={`${lane.muted ? "Unmute" : "Mute"} ${lane.name}`}
                    onClick={() =>
                      commit({
                        ...scene,
                        audioLanes: scene.audioLanes.map((l) =>
                          l.id === lane.id ? { ...l, muted: !l.muted } : l,
                        ),
                      })
                    }
                  >
                    {lane.muted ? <VolumeX size={11} /> : <Volume2 size={11} />}
                  </button>
                  <button
                    type="button"
                    className={lane.solo ? "active" : ""}
                    aria-label={`Solo ${lane.name}`}
                    onClick={() =>
                      commit({
                        ...scene,
                        audioLanes: scene.audioLanes.map((l) =>
                          l.id === lane.id ? { ...l, solo: !l.solo } : l,
                        ),
                      })
                    }
                  >
                    S
                  </button>
                  <button
                    type="button"
                    aria-label={`${lane.locked ? "Unlock" : "Lock"} ${lane.name}`}
                    onClick={() =>
                      commit({
                        ...scene,
                        audioLanes: scene.audioLanes.map((l) =>
                          l.id === lane.id ? { ...l, locked: !l.locked } : l,
                        ),
                      })
                    }
                  >
                    {lane.locked ? <Lock size={11} /> : <Unlock size={11} />}
                  </button>
                </div>
              </div>
              <small>Audio ({lane.clips.length})</small>
            </div>
            <div className="h3-lane-canvas compact" style={{ width: `${trackWidth}px` }}>
              {lane.clips
                .filter(
                  (c) =>
                    c.timelineStartSec + c.durationSec >= visibleStartSec &&
                    c.timelineStartSec <= visibleEndSec,
                )
                .slice(0, 24)
                .map((c) => (
                  <button
                    key={c.id}
                    type="button"
                    className={`h3-clip-block audio ${selected?.kind === "audio" && selected.id === c.id ? "selected" : ""}`}
                    style={{
                      left: `${c.timelineStartSec * pxPerSec}px`,
                      width: `${Math.max(24, c.durationSec * pxPerSec - 2)}px`,
                    }}
                    onClick={() => setSelected({ kind: "audio", laneId: lane.id, id: c.id })}
                  >
                    <strong>{c.label || "Audio"}</strong>
                    <small>{c.durationSec.toFixed(2)}s · {(c.gain * 100).toFixed(0)}%</small>
                  </button>
                ))}
              <div className="h3-playhead" style={{ left: `${playheadPx}px` }} />
            </div>
          </div>
        ))}
      </div>

      {/* Selected Timeline Item Inspector Strip */}
      {(selectedRefItem || selectedVideoClip || selectedDriverClip || selectedAudioClip || selectedGuide) && (
        <footer className="h3-selection-strip">
          {selectedRefItem && (
            <>
              <strong>
                Staged Reference:{" "}
                {selectedRefItem.item.entityId
                  ? `@${entityMap.get(selectedRefItem.item.entityId)?.handle ?? "Entity"}`
                  : selectedRefItem.item.assetId}
              </strong>
              <label>
                Start (s)
                <input
                  type="number"
                  min={0}
                  step={0.1}
                  value={selectedRefItem.item.timelineStartSec}
                  onChange={(e) =>
                    updateRefItem(selectedRefItem.laneId, selectedRefItem.item.id, {
                      timelineStartSec: snapToFrame(Number(e.target.value)),
                    })
                  }
                />
              </label>
              <label>
                Span (s)
                <input
                  type="number"
                  min={0.25}
                  step={0.25}
                  value={selectedRefItem.item.durationSec}
                  onChange={(e) =>
                    updateRefItem(selectedRefItem.laneId, selectedRefItem.item.id, {
                      durationSec: snapToFrame(Math.max(0.25, Number(e.target.value))),
                    })
                  }
                />
              </label>
              <label>
                Role
                <select
                  aria-label="Staged reference role override"
                  value={selectedRefItem.item.roleOverride ?? ""}
                  onChange={(e) =>
                    updateRefItem(selectedRefItem.laneId, selectedRefItem.item.id, {
                      roleOverride: (e.target.value || undefined) as H3ReferenceRole | undefined,
                    })
                  }
                >
                  <option value="">Entity Default</option>
                  <option value="subject">Subject</option>
                  <option value="outfit">Outfit</option>
                  <option value="scene">Scene / Location</option>
                  <option value="motion">Motion</option>
                  <option value="camera">Camera</option>
                  <option value="voice">Voice</option>
                  <option value="sound">Sound</option>
                  <option value="general">General</option>
                </select>
              </label>
              <label>
                Retention
                <select
                  aria-label="Staged reference retention override"
                  value={selectedRefItem.item.retentionOverride ?? ""}
                  onChange={(e) =>
                    updateRefItem(selectedRefItem.laneId, selectedRefItem.item.id, {
                      retentionOverride: (e.target.value || undefined) as H3RetentionLevel | undefined,
                    })
                  }
                >
                  <option value="">Entity Default</option>
                  <option value="fullyPreserved">Fully Preserved</option>
                  <option value="partiallyPreserved">Partially Preserved</option>
                  <option value="reference">Reference</option>
                  <option value="attributeTransfer">Attribute Transfer</option>
                  <option value="weakReference">Weak Reference</option>
                  <option value="fullyCopy">Fully Copy</option>
                </select>
              </label>
              <label>
                Desc Override
                <input
                  aria-label="Staged reference description override"
                  value={selectedRefItem.item.descriptionOverride}
                  onChange={(e) =>
                    updateRefItem(selectedRefItem.laneId, selectedRefItem.item.id, {
                      descriptionOverride: e.target.value,
                    })
                  }
                  placeholder="Optional window-specific description…"
                />
              </label>
              <label className="checkbox-inline">
                <input
                  type="checkbox"
                  checked={selectedRefItem.item.useVisual}
                  onChange={(e) =>
                    updateRefItem(selectedRefItem.laneId, selectedRefItem.item.id, {
                      useVisual: e.target.checked,
                    })
                  }
                />
                Visual
              </label>
              <label className="checkbox-inline">
                <input
                  type="checkbox"
                  checked={selectedRefItem.item.useAudio}
                  onChange={(e) =>
                    updateRefItem(selectedRefItem.laneId, selectedRefItem.item.id, {
                      useAudio: e.target.checked,
                    })
                  }
                />
                Audio
              </label>
              <button
                type="button"
                className="danger-icon"
                onClick={() => deleteRefItem(selectedRefItem.laneId, selectedRefItem.item.id)}
              >
                <Trash2 size={13} /> Remove
              </button>
            </>
          )}

          {selectedDriverClip && (
            <>
              <strong>Driver Clip: {selectedDriverClip.clip.label || "Driver Video"}</strong>
              <label>
                Timeline (s)
                <input
                  type="number"
                  min={0}
                  step={0.1}
                  value={selectedDriverClip.clip.timelineStartSec}
                  onChange={(e) =>
                    updateDriverClip(selectedDriverClip.laneId, selectedDriverClip.clip.id, {
                      timelineStartSec: snapToFrame(Number(e.target.value)),
                    })
                  }
                />
              </label>
              <label>
                Duration (s)
                <input
                  type="number"
                  min={0.2}
                  step={0.1}
                  value={selectedDriverClip.clip.durationSec}
                  onChange={(e) =>
                    updateDriverClip(selectedDriverClip.laneId, selectedDriverClip.clip.id, {
                      durationSec: snapToFrame(Math.max(0.2, Number(e.target.value))),
                    })
                  }
                />
              </label>
              <label>
                Source In (s)
                <input
                  type="number"
                  min={0}
                  step={0.1}
                  value={selectedDriverClip.clip.sourceInSec}
                  onChange={(e) =>
                    updateDriverClip(selectedDriverClip.laneId, selectedDriverClip.clip.id, {
                      sourceInSec: snapToFrame(Math.max(0, Number(e.target.value))),
                    })
                  }
                />
              </label>
              <label className="checkbox-inline">
                <input
                  type="checkbox"
                  checked={selectedDriverClip.clip.enabled}
                  onChange={(e) =>
                    updateDriverClip(selectedDriverClip.laneId, selectedDriverClip.clip.id, {
                      enabled: e.target.checked,
                    })
                  }
                />
                Enabled
              </label>
              <button
                type="button"
                className="danger-icon"
                onClick={() => deleteDriverClip(selectedDriverClip.laneId, selectedDriverClip.clip.id)}
              >
                <Trash2 size={13} /> Remove
              </button>
            </>
          )}

          {selectedVideoClip && (
            <>
              <strong>Video Clip: {selectedVideoClip.clip.label}</strong>
              <label>
                Timeline (s)
                <input
                  type="number"
                  min={0}
                  step={0.1}
                  value={selectedVideoClip.clip.timelineStartSec}
                  onChange={(e) =>
                    updateVideoClip(selectedVideoClip.laneId, selectedVideoClip.clip.id, {
                      timelineStartSec: snapToFrame(Number(e.target.value)),
                    })
                  }
                />
              </label>
              <label>
                Duration (s)
                <input
                  type="number"
                  min={0.2}
                  step={0.1}
                  value={selectedVideoClip.clip.durationSec}
                  onChange={(e) =>
                    updateVideoClip(selectedVideoClip.laneId, selectedVideoClip.clip.id, {
                      durationSec: snapToFrame(Math.max(0.2, Number(e.target.value))),
                    })
                  }
                />
              </label>
              <label>
                Opacity
                <input
                  type="number"
                  min={0}
                  max={1}
                  step={0.05}
                  value={selectedVideoClip.clip.opacity}
                  onChange={(e) =>
                    updateVideoClip(selectedVideoClip.laneId, selectedVideoClip.clip.id, {
                      opacity: Math.min(1, Math.max(0, Number(e.target.value))),
                    })
                  }
                />
              </label>
              <label>
                Audio Gain
                <input
                  type="number"
                  min={0}
                  max={2}
                  step={0.05}
                  value={selectedVideoClip.clip.audioGain}
                  onChange={(e) =>
                    updateVideoClip(selectedVideoClip.laneId, selectedVideoClip.clip.id, {
                      audioGain: Math.max(0, Number(e.target.value)),
                    })
                  }
                />
              </label>
              <button
                type="button"
                className="danger-icon"
                onClick={() => deleteVideoClip(selectedVideoClip.laneId, selectedVideoClip.clip.id)}
              >
                <Trash2 size={13} /> Remove
              </button>
            </>
          )}

          {selectedAudioClip && (
            <>
              <strong>Audio Clip: {selectedAudioClip.clip.label}</strong>
              <label>
                Timeline (s)
                <input
                  type="number"
                  min={0}
                  step={0.1}
                  value={selectedAudioClip.clip.timelineStartSec}
                  onChange={(e) =>
                    updateAudioClip(selectedAudioClip.laneId, selectedAudioClip.clip.id, {
                      timelineStartSec: snapToFrame(Number(e.target.value)),
                    })
                  }
                />
              </label>
              <label>
                Duration (s)
                <input
                  type="number"
                  min={0.2}
                  step={0.1}
                  value={selectedAudioClip.clip.durationSec}
                  onChange={(e) =>
                    updateAudioClip(selectedAudioClip.laneId, selectedAudioClip.clip.id, {
                      durationSec: snapToFrame(Math.max(0.2, Number(e.target.value))),
                    })
                  }
                />
              </label>
              <label>
                Source In (s)
                <input
                  type="number"
                  min={0}
                  step={0.1}
                  value={selectedAudioClip.clip.sourceInSec}
                  onChange={(e) =>
                    updateAudioClip(selectedAudioClip.laneId, selectedAudioClip.clip.id, {
                      sourceInSec: snapToFrame(Math.max(0, Number(e.target.value))),
                    })
                  }
                />
              </label>
              <label>
                Gain
                <input
                  type="number"
                  min={0}
                  max={2}
                  step={0.05}
                  value={selectedAudioClip.clip.gain}
                  onChange={(e) =>
                    updateAudioClip(selectedAudioClip.laneId, selectedAudioClip.clip.id, {
                      gain: Math.max(0, Number(e.target.value)),
                    })
                  }
                />
              </label>
              <button
                type="button"
                className="danger-icon"
                onClick={() => deleteAudioClip(selectedAudioClip.laneId, selectedAudioClip.clip.id)}
              >
                <Trash2 size={13} /> Remove
              </button>
            </>
          )}

          {selectedGuide && (
            <>
              <strong>Guide Pin: {selectedGuide.label}</strong>
              <label>
                At (s)
                <input
                  type="number"
                  min={0}
                  step={0.1}
                  value={selectedGuide.timelineSec}
                  onChange={(e) =>
                    commit({
                      ...scene,
                      guides: scene.guides.map((g) =>
                        g.id === selectedGuide.id
                          ? { ...g, timelineSec: snapToFrame(Number(e.target.value)) }
                          : g,
                      ),
                    })
                  }
                />
              </label>
              <label className="checkbox-inline">
                <input
                  type="checkbox"
                  checked={selectedGuide.useImage}
                  onChange={(e) =>
                    commit({
                      ...scene,
                      guides: scene.guides.map((g) =>
                        g.id === selectedGuide.id ? { ...g, useImage: e.target.checked } : g,
                      ),
                    })
                  }
                />
                Image Guide
              </label>
              <label className="checkbox-inline">
                <input
                  type="checkbox"
                  checked={selectedGuide.useAudio}
                  onChange={(e) =>
                    commit({
                      ...scene,
                      guides: scene.guides.map((g) =>
                        g.id === selectedGuide.id ? { ...g, useAudio: e.target.checked } : g,
                      ),
                    })
                  }
                />
                Audio Guide
              </label>
              <button
                type="button"
                className="danger-icon"
                onClick={() => {
                  commit({
                    ...scene,
                    guides: scene.guides.filter((g) => g.id !== selectedGuide.id),
                  });
                  setSelected(null);
                }}
              >
                <Trash2 size={13} /> Remove Guide
              </button>
            </>
          )}
        </footer>
      )}
    </section>
  );
}
