import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { Sparkles, X } from "lucide-react";
import { cancelMovieImageAsset, listMovieImageAssets, movieMediaUrl, onMovieImageAsset, startMovieImageAsset } from "../../../platform/api";
import type { MovieImageAssetGeneration, MovieReferenceAsset } from "../../../contracts/index";
import { MovieH3Preview } from "./MovieH3Preview";

type UseAsset = (asset: MovieReferenceAsset) => void | Promise<void>;
const AssetContext = createContext<((use?: UseAsset) => void) | undefined>(undefined);

export function GenerateAssetButton({ onUse, disabled, label = "Create image" }: { onUse?: UseAsset; disabled?: boolean; label?: string }) {
  const open = useContext(AssetContext);
  if (!open) return null;
  return <button disabled={disabled} onClick={() => open(onUse)}><Sparkles size={15} /> {label}</button>;
}

export function MovieAssetCreator({ children, comfyRoot, onUse, onError }: { children: ReactNode; comfyRoot: string; onUse: UseAsset; onError: (message: string) => void }) {
  const [open, setOpen] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [history, setHistory] = useState<MovieImageAssetGeneration[]>([]);
  const [requestId, setRequestId] = useState("");
  const [detail, setDetail] = useState("");
  const [busy, setBusy] = useState(false);
  const [using, setUsing] = useState(false);
  const choose = useRef<UseAsset>(onUse);
  const callbacks = useRef({ onUse, onError }); callbacks.current = { onUse, onError };
  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | undefined;
    void listMovieImageAssets().then((items) => {
      if (!alive) return;
      setHistory(items);
      const running = items.find((item) => item.status === "running");
      if (running) { setRequestId(running.id); setBusy(true); setOpen(true); setDetail(running.detail); }
    }).catch((error) => callbacks.current.onError(String(error)));
    void onMovieImageAsset((event) => {
      if (!alive) return;
      setRequestId(event.requestId); setDetail(event.detail);
      setBusy(!["complete", "cancelled", "error"].includes(event.kind));
      if (event.generation) { const generation = event.generation; setHistory((items) => [generation, ...items.filter((item) => item.id !== generation.id)].slice(0, 50)); }
    }).then((dispose) => { if (alive) unlisten = dispose; else dispose(); });
    return () => { alive = false; unlisten?.(); };
  }, []);
  const generate = async () => {
    const id = crypto.randomUUID(); setRequestId(id); setBusy(true); setDetail("Starting local H3 image generation…");
    try { await startMovieImageAsset({ requestId: id, prompt, width: 768, height: 448, steps: 20, seed: Math.floor(Math.random() * 2 ** 32), comfyRoot, stabilize: true }); }
    catch (error) { setBusy(false); setDetail(String(error)); callbacks.current.onError(String(error)); }
  };
  const useAsset = async (asset: MovieReferenceAsset) => {
    setUsing(true);
    try { await choose.current(asset); setOpen(false); }
    catch (error) { setDetail(String(error)); callbacks.current.onError(String(error)); }
    finally { setUsing(false); }
  };
  return <AssetContext.Provider value={(use) => { choose.current = use ?? callbacks.current.onUse; setOpen(true); }}>{children}
    {open && <div className="movie-asset-backdrop"><section className="movie-asset-dialog" role="dialog" aria-modal="true" aria-label="Create or choose an image asset" onKeyDown={(event) => {
      if (event.key === "Escape" && !busy && !using) { event.preventDefault(); setOpen(false); }
      if (event.key === "Tab") {
        const controls = event.currentTarget.querySelectorAll<HTMLElement>("button:not(:disabled),textarea:not(:disabled),input:not(:disabled),select:not(:disabled)");
        const first = controls[0], last = controls[controls.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    }}>
      <header><span><small>Available throughout your production</small><h2>Create or choose an image</h2></span><button aria-label="Close image creator" disabled={busy || using} onClick={() => setOpen(false)}><X /></button></header>
      <p>Describe a character, prop, location or visual style. Choose a saved image to attach it here.</p>
      <label>Image direction<textarea autoFocus aria-label="Image asset direction" maxLength={65536} value={prompt} onChange={(event) => setPrompt(event.target.value)} placeholder="A weathered brass compass on a dark oak desk, soft morning window light…" /></label>
      <button className="primary-button" disabled={busy || using || !comfyRoot || prompt.trim().length < 3} onClick={() => void generate()}><Sparkles size={16} /> Generate images · 768 × 448</button>
      {!comfyRoot && <p role="alert">Choose the installed H3 renderer in Setup before generating.</p>}
      {detail && <p role="status">{detail}</p>}
      <MovieH3Preview assetId={requestId} active={busy} onStop={() => void cancelMovieImageAsset(requestId).catch((error) => onError(String(error)))} />
      <div className="movie-asset-candidates">{history.filter((item) => item.status === "complete").flatMap((item) => item.candidates.map((candidate) => <button disabled={busy || using} key={`${item.id}-${candidate.frameIndex}`} onClick={() => void useAsset(candidate.asset)}><img src={movieMediaUrl(candidate.asset.path) || undefined} alt={item.prompt} loading="lazy" /><span>Use image · {candidate.frameIndex + 1}</span></button>))}</div>
      {!history.some((item) => item.candidates.length) && <p>Your completed images will remain available here for later scenes.</p>}
    </section></div>}
  </AssetContext.Provider>;
}
