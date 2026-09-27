import { createContext, useContext, useRef, useState, type ReactNode } from "react";
import { ImagePlus, LoaderCircle, X } from "lucide-react";
import { getImageProject, importImageTakeAsMovieReference, listImageProjects, listMovieImageAssets, movieMediaUrl } from "../../../platform/api";
import type { MovieReferenceAsset } from "../../../contracts/index";
import { PagedList } from "../../../shared/book/PagedList";
import type { ImageHandoff } from "../imageHandoff";

type UseAsset = (asset: MovieReferenceAsset) => void | Promise<void>;
type PictureRequest = { use?: UseAsset; purpose: string; direction?: string };
const AssetContext = createContext<((request: PictureRequest) => void) | undefined>(undefined);

/** Recent Image Studio projects searched for finished takes when the picker opens. */
const RECENT_IMAGE_PROJECTS = 24;

/** Opens the picture picker for one purpose (a first frame, a reference) of the production. */
export function GenerateAssetButton({ onUse, disabled, label = "Create image", direction }: { onUse?: UseAsset; disabled?: boolean; label?: string; direction?: string }) {
  const open = useContext(AssetContext);
  if (!open) return null;
  return <button className="quiet-button compact" disabled={disabled} onClick={() => open({ use: onUse, purpose: label.replace(/^Create /, ""), direction })}><ImagePlus size={15} /> {label}</button>;
}

type Choice = {
  key: string;
  name: string;
  source: string;
  path: string;
  take?: { projectId: string; takeId: string };
  asset?: MovieReferenceAsset;
};

/**
 * Pictures for a production come from Image Studio. The picker shows its finished takes (and stills
 * made earlier by the retired H3 pass); "Make it in Image Studio" opens that chapter with a new
 * project, and the take the producer chooses there comes back to the same frame or reference.
 */
export function MovieAssetCreator({ children, production, onUse, onRequestImage, onError }: {
  children: ReactNode;
  production: string;
  onUse: UseAsset;
  onRequestImage?: (handoff: ImageHandoff) => void;
  onError: (message: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [purpose, setPurpose] = useState("Image");
  const [direction, setDirection] = useState("");
  const [choices, setChoices] = useState<Choice[]>([]);
  const [loading, setLoading] = useState(false);
  const [using, setUsing] = useState(false);
  const choose = useRef<UseAsset>(onUse);
  const callbacks = useRef({ onUse, onError }); callbacks.current = { onUse, onError };

  const loadChoices = async () => {
    setLoading(true);
    try {
      const [summaries, stills] = await Promise.all([listImageProjects(), listMovieImageAssets()]);
      const recent = summaries.filter((summary) => summary.takeCount > 0)
        .sort((left, right) => right.updatedAt.localeCompare(left.updatedAt))
        .slice(0, RECENT_IMAGE_PROJECTS);
      const projects = await Promise.all(recent.map((summary) => getImageProject(summary.id).catch(() => undefined)));
      const takes: Choice[] = projects.flatMap((project) => project ? project.takes.flatMap((take, index) => take.status === "complete"
        ? [{ key: `take-${take.id}`, name: `${project.title} · take ${index + 1}`, source: "Image Studio", path: take.path, take: { projectId: project.id, takeId: take.id } }]
        : []) : []);
      const earlier: Choice[] = stills.filter((generation) => generation.status === "complete").flatMap((generation) => generation.candidates.map((candidate) => ({
        key: `still-${generation.id}-${candidate.frameIndex}`, name: candidate.asset.name || generation.prompt, source: "Earlier H3 still", path: candidate.asset.path, asset: candidate.asset,
      })));
      setChoices([...takes, ...earlier]);
    } catch (error) {
      callbacks.current.onError(String(error));
    } finally {
      setLoading(false);
    }
  };

  const request = (next: PictureRequest) => {
    choose.current = next.use ?? callbacks.current.onUse;
    setPurpose(next.purpose || "Image");
    setDirection(next.direction?.trim() ?? "");
    setOpen(true);
    void loadChoices();
  };

  const useChoice = async (choice: Choice) => {
    setUsing(true);
    try {
      const asset = choice.asset ?? await importImageTakeAsMovieReference(choice.take!.projectId, choice.take!.takeId);
      await choose.current(asset);
      setOpen(false);
    } catch (error) {
      callbacks.current.onError(String(error));
    } finally {
      setUsing(false);
    }
  };

  const makeInImageStudio = () => {
    if (!onRequestImage) return;
    const use = choose.current;
    onRequestImage({
      purpose,
      production,
      direction,
      deliver: async (imageProjectId, takeId) => { await use(await importImageTakeAsMovieReference(imageProjectId, takeId)); },
    });
    setOpen(false);
  };

  return <AssetContext.Provider value={request}>{children}
    {open && <div className="movie-asset-backdrop"><section className="movie-asset-dialog" role="dialog" aria-modal="true" aria-label="Choose a picture" onKeyDown={(event) => {
      if (event.key === "Escape" && !using) { event.preventDefault(); setOpen(false); }
      if (event.key === "Tab") {
        const controls = event.currentTarget.querySelectorAll<HTMLElement>("button:not(:disabled),textarea:not(:disabled),input:not(:disabled),select:not(:disabled)");
        const first = controls[0], last = controls[controls.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    }}>
      <header><span><span className="eyebrow">{purpose} · {production}</span><h2>Choose a picture</h2><p>Use a finished Image Studio take, or make a new picture there. It comes back to this {purpose.toLowerCase()} when you choose it.</p></span><button className="icon-button" aria-label="Close picture picker" disabled={using} onClick={() => setOpen(false)}><X /></button></header>
      <div className="movie-asset-create">
        <label>What should the picture show?<textarea autoFocus aria-label="Picture direction" maxLength={65536} value={direction} onChange={(event) => setDirection(event.target.value)} placeholder="A weathered brass compass on a dark oak desk, soft morning window light…" /></label>
        <button className="primary-button" disabled={using || !onRequestImage} onClick={makeInImageStudio}><ImagePlus size={16} /> Make it in Image Studio</button>
        <p className="movie-asset-note">Image Studio opens a new project with this description. When a take is ready, choose <strong>Use in {production}</strong> there and it returns here.</p>
      </div>
      <div className="movie-asset-library">
        <span className="eyebrow">Finished pictures · {loading ? "…" : choices.length}</span>
        {loading
          ? <p className="movie-asset-note"><LoaderCircle className="spin" size={14} /> Looking through Image Studio…</p>
          : <PagedList
            className="movie-asset-candidates"
            label="Finished pictures"
            items={choices}
            itemKey={(choice) => choice.key}
            empty={<p className="movie-asset-note">No finished pictures yet. Make one in Image Studio.</p>}
            renderItem={(choice) => <button disabled={using} onClick={() => void useChoice(choice)} aria-label={`Use ${choice.name}`}><img src={movieMediaUrl(choice.path) || undefined} alt="" loading="lazy" /><span><strong>{choice.name}</strong><small>{choice.source}</small></span></button>}
          />}
      </div>
    </section></div>}
  </AssetContext.Provider>;
}
