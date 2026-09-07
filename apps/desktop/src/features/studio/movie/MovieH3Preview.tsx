import { useEffect, useState } from "react";
import { CircleStop, Video } from "lucide-react";
import { getMovieImageAssetRenderState, getMovieRenderState, onMovieRenderPreview } from "../../../platform/api";
import type { MovieRenderPreviewEvent } from "../../../contracts/index";

export function retainPreview(previous: MovieRenderPreviewEvent | undefined, next: MovieRenderPreviewEvent): MovieRenderPreviewEvent {
  return previous?.jobId === next.jobId && !next.dataUrl
    ? { ...previous, ...next, dataUrl: previous.dataUrl, mimeType: previous.mimeType }
    : next;
}

export function MovieH3Preview({ projectId, assetId, active, onStop }: {
  projectId?: string; assetId?: string; active: boolean; onStop?: () => void;
}) {
  const [preview, setPreview] = useState<MovieRenderPreviewEvent>();
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    setPreview(undefined);
    void onMovieRenderPreview((event) => {
      if (!disposed && (assetId ? event.target === "imageAsset" && event.jobId === assetId : event.projectId === projectId && Boolean(projectId))) {
        setPreview((previous) => retainPreview(previous, event));
      }
    }).then((dispose) => { if (disposed) dispose(); else unlisten = dispose; });
    if ((projectId || assetId) && active) void (assetId ? getMovieImageAssetRenderState(assetId) : getMovieRenderState(projectId!)).then((state) => {
      if (!disposed && state.preview) setPreview((current) => current ?? state.preview);
    }).catch(() => undefined);
    return () => { disposed = true; unlisten?.(); };
  }, [projectId, assetId, active]);
  if (!active) return null;
  return <section className="movie-h3-preview" aria-label="Live H3 preview">
    <header><strong><Video size={16} /> H3 live preview</strong>{onStop && <button onClick={onStop}><CircleStop size={15} /> Stop</button>}</header>
    <div className="movie-h3-picture">{preview?.dataUrl ? preview.mimeType === "video/mp4"
      ? <video src={preview.dataUrl} autoPlay muted loop playsInline />
      : <img src={preview.dataUrl} alt="Approximate H3 generation preview" />
      : <span>{preview?.kind === "unavailable" ? "Preview unavailable" : "Preparing the first preview…"}</span>}</div>
    <p role="status">{preview?.detail ?? "The preview appears when H3 begins sampling. The finished take is saved at full quality."}</p>
    {preview?.kind === "unavailable" && <small>Check the H3 preview components in Setup. The saved render continues.</small>}
    {preview?.step !== undefined && <progress max={preview.total || 1} value={preview.step} />}
    <small>Approximate preview · final picture and sound are in the saved master.</small>
  </section>;
}
