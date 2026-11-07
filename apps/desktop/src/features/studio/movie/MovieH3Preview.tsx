import { useEffect, useRef, useState } from "react";
import { Camera, CircleStop, Columns2, Film, Pause, Play, Pin, SkipBack, Video } from "lucide-react";
import { getMovieRenderState, movieMediaUrl, onMovieRenderPreview } from "../../../platform/api";
import type { H3RenderQueueJob, MovieRenderPreviewEvent } from "../../../contracts/index";

export function retainPreview(previous: MovieRenderPreviewEvent | undefined, next: MovieRenderPreviewEvent): MovieRenderPreviewEvent {
  return previous?.jobId === next.jobId && !next.dataUrl
    ? { ...previous, ...next, dataUrl: previous.dataUrl, mimeType: previous.mimeType }
    : next;
}

export function MovieH3Preview({ projectId, active, onStop }: {
  projectId?: string; active: boolean; onStop?: () => void;
}) {
  const [preview, setPreview] = useState<MovieRenderPreviewEvent>();
  const [failedPreview, setFailedPreview] = useState<MovieRenderPreviewEvent>();
  const failed = Boolean(preview?.dataUrl && failedPreview?.jobId === preview.jobId && failedPreview?.dataUrl === preview.dataUrl);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    setPreview(undefined);
    setFailedPreview(undefined);
    void onMovieRenderPreview((event) => {
      if (!disposed && event.projectId === projectId && Boolean(projectId)) {
        setPreview((previous) => retainPreview(previous, event));
      }
    }).then((dispose) => { if (disposed) dispose(); else unlisten = dispose; });
    if (projectId && active) void getMovieRenderState(projectId).then((state) => {
      if (!disposed && state.preview) setPreview((current) => current ?? state.preview);
    }).catch(() => undefined);
    return () => { disposed = true; unlisten?.(); };
  }, [projectId, active]);
  if (!active) return null;
  return <section className="movie-h3-preview" aria-label="Live H3 preview">
    <header><strong><Video size={16} /> H3 live preview</strong>{onStop && <button onClick={onStop}><CircleStop size={15} /> Stop</button>}</header>
    <div className="movie-h3-picture">{preview?.dataUrl ? <>{preview.mimeType === "video/mp4"
      ? <video src={preview.dataUrl} autoPlay muted loop playsInline hidden={failed} onError={() => setFailedPreview(preview)} onLoadedData={() => setFailedPreview(undefined)} />
      : <img src={preview.dataUrl} alt="Approximate H3 generation preview" hidden={failed} onError={() => setFailedPreview(preview)} onLoad={() => setFailedPreview(undefined)} />}
      {failed && <span role="alert">Preview couldn’t play. Rendering continues.</span>}</>
      : <span>{preview?.kind === "unavailable" ? "Preview unavailable" : "Preparing the first preview…"}</span>}</div>
    <p role="status">{preview?.detail ?? "The preview appears when H3 begins sampling. The finished take is saved at full quality."}</p>
    {preview?.kind === "unavailable" && <small>Check the H3 preview components in Setup. The saved render continues.</small>}
    {preview?.step !== undefined && <progress max={preview.total || 1} value={preview.step} />}
    <small>Approximate preview · final picture and sound are in the saved master.</small>
  </section>;
}

export interface H3MonitorSource {
  id: string;
  label: string;
  mediaPath: string;
  sourceOffsetSec: number;
  timelineStartSec: number;
  durationSec: number;
  sourceKind: "renderedTake" | "referenceAsset";
}

export function H3ProgramMonitor({
  sceneName,
  sceneWidth,
  sceneHeight,
  playheadSec,
  durationSec,
  activeClip,
  completedTakes,
  rendering,
  projectId,
  disabled,
  onPlayhead,
  onCaptureGuide,
  onStopRender,
}: {
  sceneName: string;
  sceneWidth: number;
  sceneHeight: number;
  playheadSec: number;
  durationSec: number;
  activeClip?: H3MonitorSource;
  completedTakes: H3RenderQueueJob[];
  rendering: boolean;
  projectId: string;
  disabled?: boolean;
  onPlayhead: (sec: number) => void;
  onCaptureGuide?: (source: H3MonitorSource, sourceTimeSec: number, timelineSec: number) => void;
  onStopRender?: () => void;
}) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const compareRef = useRef<HTMLVideoElement>(null);
  const [playing, setPlaying] = useState(false);
  const [abMode, setAbMode] = useState(false);
  const [takeAId, setTakeAId] = useState("");
  const [takeBId, setTakeBId] = useState("");

  useEffect(() => {
    if (completedTakes.length > 0 && !completedTakes.some((t) => t.id === takeAId)) {
      setTakeAId(completedTakes[0].id);
    }
    if (completedTakes.length > 1 && !completedTakes.some((t) => t.id === takeBId)) {
      setTakeBId(completedTakes[1].id);
    } else if (completedTakes.length === 1 && !takeBId) {
      setTakeBId(completedTakes[0].id);
    }
  }, [completedTakes, takeAId, takeBId]);

  const clipLocalSec = activeClip
    ? Math.max(0, activeClip.sourceOffsetSec + (playheadSec - activeClip.timelineStartSec))
    : 0;
  const primaryUrl = activeClip ? movieMediaUrl(activeClip.mediaPath) : "";

  useEffect(() => {
    const el = videoRef.current;
    if (!el || playing || !primaryUrl) return;
    if (Math.abs(el.currentTime - clipLocalSec) > 0.08) {
      try { el.currentTime = clipLocalSec; } catch { /* ignore unready metadata */ }
    }
  }, [clipLocalSec, playing, primaryUrl]);

  const togglePlay = () => {
    const el = videoRef.current;
    if (!el || !primaryUrl) return;
    if (el.paused) {
      void el.play().then(() => setPlaying(true)).catch(() => setPlaying(false));
      if (abMode && compareRef.current) void compareRef.current.play().catch(() => undefined);
    } else {
      el.pause();
      compareRef.current?.pause();
      setPlaying(false);
    }
  };

  const takeA = completedTakes.find((t) => t.id === takeAId) ?? completedTakes[0];
  const takeB = completedTakes.find((t) => t.id === takeBId) ?? completedTakes[1] ?? completedTakes[0];

  return (
    <section className="h3-program-monitor" aria-label="Program and A/B monitor">
      <header className="h3-monitor-header">
        <div className="h3-monitor-title">
          <strong><Film size={14} /> {sceneName}</strong>
          <small>{sceneWidth}×{sceneHeight} · 24 fps</small>
        </div>
        <div className="h3-monitor-actions">
          {completedTakes.length >= 1 && (
            <button
              type="button"
              className={abMode ? "active" : ""}
              onClick={() => setAbMode((v) => !v)}
              aria-pressed={abMode}
            >
              <Columns2 size={13} /> A/B Compare ({completedTakes.length})
            </button>
          )}
          {activeClip && onCaptureGuide && (
            <button
              type="button"
              disabled={disabled}
              title="Extract this exact frame as a PNG and pin it to Guides at the playhead"
              onClick={() => onCaptureGuide(activeClip, clipLocalSec, playheadSec)}
            >
              <Camera size={13} /> <Pin size={11} /> Capture Frame to Guides
            </button>
          )}
        </div>
      </header>

      {rendering ? (
        <MovieH3Preview projectId={projectId} active={rendering} onStop={onStopRender} />
      ) : abMode && takeA ? (
        <div className="h3-ab-stage">
          <div className="h3-ab-pane">
            <header>
              <label>
                Take A
                <select aria-label="Compare Take A" value={takeA.id} onChange={(e) => setTakeAId(e.target.value)}>
                  {completedTakes.map((t) => (
                    <option key={t.id} value={t.id}>
                      {t.label} · {t.passPreset} ({t.quality ? `${t.quality.width}×${t.quality.height}` : "done"})
                    </option>
                  ))}
                </select>
              </label>
              {takeA.quality && (
                <small>
                  {takeA.quality.durationSec.toFixed(2)}s · Peak {takeA.quality.audioPeakDbfs.toFixed(1)} dBFS
                </small>
              )}
            </header>
            {movieMediaUrl(takeA.takeMediaPath) ? (
              <video ref={videoRef} src={movieMediaUrl(takeA.takeMediaPath)} controls preload="metadata" />
            ) : (
              <div className="h3-monitor-placeholder"><span>Take A saved at {takeA.takeMediaPath || "disk"}</span></div>
            )}
          </div>
          <div className="h3-ab-pane">
            <header>
              <label>
                Take B
                <select aria-label="Compare Take B" value={takeB?.id ?? ""} onChange={(e) => setTakeBId(e.target.value)}>
                  {completedTakes.map((t) => (
                    <option key={t.id} value={t.id}>
                      {t.label} · {t.passPreset} ({t.quality ? `${t.quality.width}×${t.quality.height}` : "done"})
                    </option>
                  ))}
                </select>
              </label>
              {takeB?.quality && (
                <small>
                  {takeB.quality.durationSec.toFixed(2)}s · Peak {takeB.quality.audioPeakDbfs.toFixed(1)} dBFS
                </small>
              )}
            </header>
            {takeB && movieMediaUrl(takeB.takeMediaPath) ? (
              <video ref={compareRef} src={movieMediaUrl(takeB.takeMediaPath)} controls preload="metadata" />
            ) : (
              <div className="h3-monitor-placeholder"><span>Take B saved at {takeB?.takeMediaPath || "disk"}</span></div>
            )}
          </div>
        </div>
      ) : (
        <div className="h3-monitor-stage">
          {primaryUrl ? (
            <video
              ref={videoRef}
              src={primaryUrl}
              playsInline
              preload="metadata"
              onPause={() => setPlaying(false)}
              onPlay={() => setPlaying(true)}
              onTimeUpdate={(e) => {
                if (!playing || !activeClip) return;
                const nextSec = activeClip.timelineStartSec + (e.currentTarget.currentTime - activeClip.sourceOffsetSec);
                onPlayhead(Math.min(durationSec, Math.max(0, nextSec)));
              }}
            />
          ) : (
            <div className="h3-monitor-placeholder">
              <Video size={26} />
              <strong>{activeClip ? activeClip.label : "No video clip under playhead"}</strong>
              <span>
                {activeClip
                  ? `Clip span ${activeClip.timelineStartSec.toFixed(2)}s – ${(activeClip.timelineStartSec + activeClip.durationSec).toFixed(2)}s`
                  : "Stage @Handle references and generate an [In, Out] window, or scrub over a placed take."}
              </span>
            </div>
          )}
        </div>
      )}

      <footer className="h3-monitor-transport">
        <div>
          <button type="button" aria-label="Jump to scene start" onClick={() => onPlayhead(0)}>
            <SkipBack size={13} />
          </button>
          <button
            type="button"
            className="play-btn"
            aria-label={playing ? "Pause program monitor" : "Play program monitor"}
            disabled={!primaryUrl}
            onClick={togglePlay}
          >
            {playing ? <Pause size={13} /> : <Play size={13} />}
          </button>
        </div>
        <input
          type="range"
          aria-label="Program playhead scrubber"
          min={0}
          max={Math.max(1, durationSec)}
          step={1 / 24}
          value={Math.min(playheadSec, Math.max(1, durationSec))}
          onChange={(e) => onPlayhead(Number(e.target.value))}
        />
        <time>{playheadSec.toFixed(2)}s / {durationSec.toFixed(2)}s</time>
      </footer>
    </section>
  );
}
