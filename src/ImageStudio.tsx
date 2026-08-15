import {
  Aperture, ArrowDown, ArrowUp, Bot, Check, ChevronLeft, ChevronRight, CircleStop,
  Copy, Download, Eye, EyeOff, FolderOpen, Frame, Image as ImageIcon, Layers3,
  LayoutTemplate, LoaderCircle, Maximize2, PanelLeft, Plus, Save,
  SlidersHorizontal, Sparkles, Square, Type, WandSparkles, X,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  cancelImageGeneration, cancelMoviePromptDraft, createImageProject, getImageProject,
  imageMediaUrl, listImageProjects, onImageGeneration, onImageProjectUpdated,
  onMoviePromptDraft, revealImageProject, saveImageProject, startImageGeneration,
  startMoviePromptDraft,
} from "./api";
import { appendModelThinking, ModelThinkingStream } from "./ModelThinkingStream";
import type {
  ImageElement, ImageGenerationEvent, ImageProject, ImageSummary, ImageTake, ModelInfo,
  PromptDraftReceipt,
} from "./types";

interface ImageCollaboration {
  id: string;
  text: string;
  reasoning: string;
  status: string;
  modelName: string;
  receipt?: PromptDraftReceipt;
}

type InspectorTab = "compose" | "output" | "advanced";
type DrawKind = ImageElement["kind"] | undefined;

const SIZE_PRESETS = [
  ["Landscape · 3:2", 1536, 1024],
  ["Square · 1:1", 1024, 1024],
  ["Portrait · 2:3", 1024, 1536],
  ["Widescreen · 16:9", 1920, 1088],
  ["Phone · 9:16", 1024, 1792],
  ["Social banner · 4:1", 1600, 400],
  ["Ultra portrait · 9:32", 576, 2048],
  ["Ultra banner · 32:9", 2048, 576],
] as const;

export function ImageStudio({
  initialComfyRoot,
  advancedEnabled,
  models = [],
  selectedModelId,
  onError,
}: {
  initialComfyRoot?: string;
  advancedEnabled: boolean;
  models?: ModelInfo[];
  selectedModelId?: string;
  onError: (message: string) => void;
}) {
  const [summaries, setSummaries] = useState<ImageSummary[]>([]);
  const [project, setProject] = useState<ImageProject>();
  const [loading, setLoading] = useState(true);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [newOpen, setNewOpen] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [newIdea, setNewIdea] = useState("");
  const [creating, setCreating] = useState(false);
  const [showLibrary, setShowLibrary] = useState(true);
  const [showLayout, setShowLayout] = useState(true);
  const [showBackdrop, setShowBackdrop] = useState(true);
  const [backdropOpacity, setBackdropOpacity] = useState(1);
  const [drawKind, setDrawKind] = useState<DrawKind>();
  const [drawPreview, setDrawPreview] = useState<[number, number, number, number]>();
  const [selectedElementId, setSelectedElementId] = useState("");
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>("compose");
  const [progress, setProgress] = useState<ImageGenerationEvent>();
  const [modelId, setModelId] = useState(selectedModelId ?? models[0]?.id ?? "");
  const [collaboration, setCollaboration] = useState<ImageCollaboration>();
  const activeProjectId = useRef("");
  const artboardRef = useRef<HTMLDivElement>(null);

  const refresh = async (preferredId?: string) => {
    const next = await listImageProjects();
    setSummaries(next);
    const id = preferredId || activeProjectId.current || next[0]?.id;
    if (!id) return;
    const loaded = await getImageProject(id);
    activeProjectId.current = loaded.id;
    setProject(loaded);
    setSelectedElementId((current) => loaded.elements.some((element) => element.id === current) ? current : loaded.elements[0]?.id ?? "");
    setDirty(false);
  };

  useEffect(() => {
    refresh().catch((error) => onError(String(error))).finally(() => setLoading(false));
    let disposed = false;
    const cleanups: Array<() => void> = [];
    void onImageProjectUpdated((next) => {
      if (next.id !== activeProjectId.current) return;
      setProject(next);
      setDirty(false);
    }).then((cleanup) => disposed ? cleanup() : cleanups.push(cleanup));
    void onImageGeneration((event) => {
      if (event.projectId !== activeProjectId.current) return;
      setProgress(event);
      if (["complete", "error", "cancelled"].includes(event.kind)) {
        void refresh(event.projectId).catch((error) => onError(String(error)));
      }
    }).then((cleanup) => disposed ? cleanup() : cleanups.push(cleanup));
    void onMoviePromptDraft((event) => {
      if (event.kind === "error") onError(event.content ?? "The local image collaborator stopped.");
      setCollaboration((current) => {
        if (!current || current.id !== event.requestId) return current;
        if (event.kind === "token") return { ...current, text: current.text + (event.content ?? ""), status: "writing", modelName: event.modelName ?? current.modelName };
        if (event.kind === "started") return { ...current, status: "writing", modelName: event.modelName ?? current.modelName, receipt: event.receipt };
        if (event.kind === "reasoning") return { ...current, reasoning: appendModelThinking(current.reasoning, event.content ?? ""), status: "thinking", modelName: event.modelName ?? current.modelName };
        if (event.kind === "complete") return { ...current, status: "ready", modelName: event.modelName ?? current.modelName };
        if (["limited", "cancelled"].includes(event.kind)) return { ...current, status: "checkpoint", modelName: event.modelName ?? current.modelName };
        if (event.kind === "error") return { ...current, status: "error" };
        return current;
      });
    }).then((cleanup) => disposed ? cleanup() : cleanups.push(cleanup));
    return () => { disposed = true; cleanups.forEach((cleanup) => cleanup()); };
    // Event subscriptions intentionally remain stable for the lifetime of the workspace.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!modelId && (selectedModelId || models[0]?.id)) setModelId(selectedModelId ?? models[0]?.id ?? "");
  }, [modelId, models, selectedModelId]);

  const completedTakes = project?.takes.filter((take) => take.status === "complete") ?? [];
  const activeTake = completedTakes.find((take) => take.id === project?.activeTakeId);
  const selectedElement = project?.elements.find((element) => element.id === selectedElementId);
  const generating = project?.status === "generating";
  const assistantBusy = !!collaboration && ["queued", "thinking", "writing"].includes(collaboration.status);
  const busy = saving || creating || generating || assistantBusy;

  const mutate = (change: (current: ImageProject) => ImageProject) => {
    if (generating) return;
    setProject((current) => current ? change(current) : current);
    setDirty(true);
  };

  const save = async (): Promise<ImageProject | undefined> => {
    if (!project || generating) return project;
    setSaving(true);
    try {
      const saved = await saveImageProject(project);
      setProject(saved);
      setDirty(false);
      await refresh(saved.id);
      return saved;
    } catch (error) {
      onError(String(error));
    } finally {
      setSaving(false);
    }
  };

  const create = async () => {
    setCreating(true);
    try {
      const next = await createImageProject({ title: newTitle, idea: newIdea, comfyRoot: initialComfyRoot ?? "" });
      activeProjectId.current = next.id;
      setProject(next);
      setDirty(false);
      setNewOpen(false);
      setNewTitle("");
      setNewIdea("");
      await refresh(next.id);
    } catch (error) {
      onError(String(error));
    } finally {
      setCreating(false);
    }
  };

  const generate = async () => {
    if (!project) return;
    try {
      const saved = dirty ? await save() : project;
      if (!saved) return;
      setProgress({ projectId: saved.id, takeId: "", kind: "queued", phase: "queued", detail: "Freeing GPU memory and preparing Ideogram 4…", at: new Date().toISOString() });
      setProject(await startImageGeneration(saved.id));
    } catch (error) {
      setProgress(undefined);
      onError(String(error));
    }
  };

  const startCollaboration = async () => {
    if (!project || !modelId || assistantBusy) return;
    const id = crypto.randomUUID();
    setCollaboration({ id, text: "", reasoning: "", status: "queued", modelName: models.find((model) => model.id === modelId)?.name ?? "Local model" });
    try {
      await startMoviePromptDraft({
        requestId: id,
        modelId,
        target: "imageComposition",
        mode: "develop",
        storyText: project.idea,
        existingText: JSON.stringify(compiledPrompt(project), null, 2),
        assetName: "",
        assetKind: "",
      });
    } catch (error) {
      setCollaboration(undefined);
      onError(String(error));
    }
  };

  const applyCollaboration = () => {
    if (!project || !collaboration?.text.trim()) return;
    try {
      const proposal = parseImageProposal(collaboration.text);
      mutate((current) => ({ ...current, ...proposal }));
      setSelectedElementId(proposal.elements[0]?.id ?? "");
      setCollaboration(undefined);
    } catch (error) {
      onError(String(error));
    }
  };

  const addElement = (kind: ImageElement["kind"], bbox?: ImageElement["bbox"]) => {
    const element: ImageElement = {
      id: crypto.randomUUID(),
      kind,
      bbox: bbox ?? (kind === "text" ? [80, 100, 230, 900] : [250, 250, 750, 750]),
      text: kind === "text" ? "YOUR TEXT" : "",
      description: kind === "text" ? "Bold upright sans-serif lettering with clear spacing." : "Describe the subject, object, or visual layer.",
      colorPalette: [],
    };
    mutate((current) => ({ ...current, elements: [...current.elements, element] }));
    setSelectedElementId(element.id);
  };

  const duplicateElement = () => {
    if (!project || !selectedElement || project.elements.length >= 64) return;
    const duplicate: ImageElement = {
      ...selectedElement,
      id: crypto.randomUUID(),
      bbox: moveBox(selectedElement.bbox, 20, 20),
    };
    mutate((current) => ({ ...current, elements: [...current.elements, duplicate] }));
    setSelectedElementId(duplicate.id);
  };

  const moveElementLayer = (delta: number) => {
    if (!selectedElement) return;
    mutate((current) => {
      const from = current.elements.findIndex((element) => element.id === selectedElement.id);
      const to = clamp(from + delta, 0, current.elements.length - 1);
      if (from < 0 || from === to) return current;
      const elements = [...current.elements];
      const [element] = elements.splice(from, 1);
      elements.splice(to, 0, element);
      return { ...current, elements };
    });
  };

  const removeElement = () => {
    if (!selectedElement) return;
    mutate((current) => ({ ...current, elements: current.elements.filter((element) => element.id !== selectedElement.id) }));
    setSelectedElementId(project?.elements.find((element) => element.id !== selectedElement.id)?.id ?? "");
  };

  const cycleTake = (delta: number) => {
    if (!project || !completedTakes.length) return;
    const current = Math.max(0, completedTakes.findIndex((take) => take.id === project.activeTakeId));
    const next = completedTakes[(current + delta + completedTakes.length) % completedTakes.length];
    mutate((value) => ({ ...value, activeTakeId: next.id }));
  };

  const startBoxPointer = (event: React.PointerEvent, element: ImageElement, resize: boolean) => {
    if (!project || generating || !artboardRef.current) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.altKey && !resize) {
      const board = artboardRef.current.getBoundingClientRect();
      const x = clamp(Math.round((event.clientX - board.left) / board.width * 1000), 0, 1000);
      const y = clamp(Math.round((event.clientY - board.top) / board.height * 1000), 0, 1000);
      const overlapping = project.elements.filter(({ bbox: [top, left, bottom, right] }) => y >= top && y <= bottom && x >= left && x <= right);
      const current = overlapping.findIndex((item) => item.id === selectedElementId);
      setSelectedElementId(overlapping[(current + 1) % overlapping.length]?.id ?? element.id);
      return;
    }
    setSelectedElementId(element.id);
    const board = artboardRef.current.getBoundingClientRect();
    const startX = event.clientX;
    const startY = event.clientY;
    const original = [...element.bbox] as [number, number, number, number];
    const move = (pointer: PointerEvent) => {
      const dx = Math.round((pointer.clientX - startX) / board.width * 1000);
      const dy = Math.round((pointer.clientY - startY) / board.height * 1000);
      const next: [number, number, number, number] = resize
        ? [original[0], original[1], clamp(original[2] + dy, original[0] + 10, 1000), clamp(original[3] + dx, original[1] + 10, 1000)]
        : moveBox(original, dx, dy);
      setProject((current) => current ? { ...current, elements: current.elements.map((item) => item.id === element.id ? { ...item, bbox: next } : item) } : current);
      setDirty(true);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up, { once: true });
  };

  const startCanvasDraw = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!project || !drawKind || generating || !artboardRef.current || project.elements.length >= 64) return;
    event.preventDefault();
    const board = artboardRef.current.getBoundingClientRect();
    const point = (clientX: number, clientY: number): [number, number] => [
      clamp(Math.round((clientY - board.top) / board.height * 1000), 0, 1000),
      clamp(Math.round((clientX - board.left) / board.width * 1000), 0, 1000),
    ];
    const [startY, startX] = point(event.clientX, event.clientY);
    const move = (pointer: PointerEvent) => {
      const [y, x] = point(pointer.clientX, pointer.clientY);
      setDrawPreview([Math.min(startY, y), Math.min(startX, x), Math.max(startY, y), Math.max(startX, x)]);
    };
    const up = (pointer: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      const [y, x] = point(pointer.clientX, pointer.clientY);
      const bbox: ImageElement["bbox"] = [Math.min(startY, y), Math.min(startX, x), Math.max(startY, y), Math.max(startX, x)];
      setDrawPreview(undefined);
      setDrawKind(undefined);
      if (bbox[2] - bbox[0] >= 10 && bbox[3] - bbox[1] >= 10) addElement(drawKind, bbox);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up, { once: true });
  };

  const handleStudioKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement;
    if (["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName) || target.isContentEditable || busy) return;
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "d") {
      event.preventDefault();
      duplicateElement();
      return;
    }
    if (event.key === "Escape") {
      setDrawKind(undefined);
      setDrawPreview(undefined);
      return;
    }
    if ((event.key === "Delete" || event.key === "Backspace") && selectedElement) {
      event.preventDefault();
      removeElement();
      return;
    }
    const direction: Record<string, [number, number]> = { ArrowUp: [0, -1], ArrowDown: [0, 1], ArrowLeft: [-1, 0], ArrowRight: [1, 0] };
    const nudge = direction[event.key];
    if (selectedElement && nudge) {
      event.preventDefault();
      const distance = event.shiftKey ? 10 : 1;
      patchElement(mutate, selectedElement.id, { bbox: moveBox(selectedElement.bbox, nudge[0] * distance, nudge[1] * distance) });
    }
  };

  if (loading) return <div className="image-studio-loading"><LoaderCircle className="spin" /> Opening private image projects…</div>;
  if (!project) return <ImageStudioEmpty onNew={() => setNewOpen(true)} dialog={newOpen ? <NewImageDialog title={newTitle} idea={newIdea} busy={creating} onTitle={setNewTitle} onIdea={setNewIdea} onClose={() => setNewOpen(false)} onCreate={() => void create()} /> : undefined} />;

  return <div className={`image-studio ${showLibrary ? "library-visible" : ""}`} tabIndex={-1} onKeyDown={handleStudioKeyDown}>
    <header className="image-toolbar">
      <div className="image-toolbar-left"><button aria-label="Toggle image library" onClick={() => setShowLibrary((value) => !value)}><PanelLeft /></button><span className="image-tool-divider" /><Aperture /><input aria-label="Image project title" disabled={busy} value={project.title} onChange={(event) => mutate((current) => ({ ...current, title: event.target.value }))} /><small>{dirty ? "Edited" : "Saved"}</small></div>
      <div className="image-toolbar-center"><button title="Previous take" disabled={busy || !activeTake} onClick={() => cycleTake(-1)}><ChevronLeft /></button><span>{completedTakes.length ? `${Math.max(1, completedTakes.findIndex((take) => take.id === project.activeTakeId) + 1)} / ${completedTakes.length}` : "No takes"}</span><button title="Next take" disabled={busy || !activeTake} onClick={() => cycleTake(1)}><ChevronRight /></button></div>
      <div className="image-toolbar-right"><button disabled={!dirty || busy} onClick={() => void save()}>{saving ? <LoaderCircle className="spin" /> : <Save />} Save</button>{generating ? <button className="image-stop" onClick={() => void cancelImageGeneration(project.id)}><CircleStop /> Stop</button> : <button className="image-render" disabled={assistantBusy} onClick={() => void generate()}><WandSparkles /> Create image</button>}</div>
    </header>

    {showLibrary && <aside className="image-library">
      <div className="image-pane-heading"><span><small>Private library</small><strong>Image projects</strong></span><button aria-label="Create new image project" onClick={() => setNewOpen(true)}><Plus /></button></div>
      <div className="image-project-list">{summaries.map((item) => <button className={item.id === project.id ? "active" : ""} key={item.id} disabled={busy} onClick={() => void refresh(item.id).catch((error) => onError(String(error)))}>{item.activeTakePath ? <img src={imageMediaUrl(item.activeTakePath)} alt="" /> : <ImageIcon />}<span><strong>{item.title}</strong><small>{item.takeCount} take{item.takeCount === 1 ? "" : "s"} · {item.status}</small></span></button>)}</div>
      <div className="image-library-footer"><strong>Lossless and local</strong><small>Every PNG, prompt, seed, graph, and checksum stays with its project.</small></div>
    </aside>}

    <main className="image-workbench">
      <div className="image-viewbar"><span><button className={showLayout ? "active" : ""} onClick={() => setShowLayout((value) => !value)}><Frame /> Layout</button><button className={drawKind === "obj" ? "active" : ""} disabled={busy || project.elements.length >= 64} title="Draw an object box" onClick={() => { setDrawKind(drawKind === "obj" ? undefined : "obj"); setShowLayout(true); }}><Square /> Draw object</button><button className={drawKind === "text" ? "active" : ""} disabled={busy || project.elements.length >= 64} title="Draw an exact-text box" onClick={() => { setDrawKind(drawKind === "text" ? undefined : "text"); setShowLayout(true); }}><Type /> Draw text</button><button className={showBackdrop ? "active" : ""} disabled={!activeTake} title="The active take is a visual alignment backdrop only; it is never sent to Ideogram" onClick={() => setShowBackdrop((value) => !value)}>{showBackdrop ? <Eye /> : <EyeOff />} Backdrop</button>{activeTake && showBackdrop && <label className="image-backdrop-opacity" title="Backdrop opacity"><span>Opacity</span><input aria-label="Backdrop opacity" type="range" min={10} max={100} value={Math.round(backdropOpacity * 100)} onChange={(event) => setBackdropOpacity(event.currentTarget.valueAsNumber / 100)} /></label>}</span><span><strong>{project.settings.width} × {project.settings.height}</strong><small>{project.settings.preset} · seed {project.settings.seed || "random each run"}</small></span></div>
      <section className="image-canvas-stage">
        <div className="image-artboard-wrap">
          <div ref={artboardRef} className={`image-artboard ${activeTake && showBackdrop ? "has-image" : "empty"} ${drawKind ? "drawing" : ""}`} onPointerDown={startCanvasDraw} style={{ aspectRatio: `${project.settings.width} / ${project.settings.height}`, width: `min(100%, calc(100cqh * ${project.settings.width / project.settings.height}))` }}>
            {activeTake && showBackdrop ? <img src={imageMediaUrl(activeTake.path)} alt={project.highLevelDescription} draggable={false} style={{ opacity: backdropOpacity }} /> : <div className="image-artboard-placeholder"><Aperture /><strong>{project.highLevelDescription || "Describe the image in the inspector"}</strong><span>{project.background}</span></div>}
            {showLayout && <div className="image-layout-overlay">{project.elements.map((element, index) => {
              const [top, left, bottom, right] = element.bbox;
              return <button key={element.id} className={`image-layout-box ${element.kind} ${element.id === selectedElementId ? "selected" : ""}`} style={{ top: `${top / 10}%`, left: `${left / 10}%`, width: `${(right - left) / 10}%`, height: `${(bottom - top) / 10}%` }} onPointerDown={(event) => startBoxPointer(event, element, false)}><span>{element.kind === "text" ? element.text : element.description.split(/[,.]/)[0]}<i>{index + 1}</i></span><b onPointerDown={(event) => startBoxPointer(event, element, true)} /></button>;
            })}{drawPreview && <i className={`image-draw-preview ${drawKind}`} style={{ top: `${drawPreview[0] / 10}%`, left: `${drawPreview[1] / 10}%`, width: `${(drawPreview[3] - drawPreview[1]) / 10}%`, height: `${(drawPreview[2] - drawPreview[0]) / 10}%` }} />}</div>}
          </div>
        </div>
        {progress && generating && <div className="image-progress-strip"><LoaderCircle className="spin" /><span><strong>{progress.phase.replaceAll("-", " ")}</strong><small>{progress.detail}</small></span><div><i style={{ width: `${progress.percent ?? 5}%` }} /><small>{progress.step && progress.total ? `${progress.step}/${progress.total}` : "Preparing"}{progress.etaSeconds ? ` · about ${formatEta(progress.etaSeconds)}` : ""}</small></div></div>}
      </section>
      <TakeStrip project={project} activeTake={activeTake} disabled={busy} onSelect={(id) => mutate((current) => ({ ...current, activeTakeId: id }))} />
    </main>

    <aside className="image-inspector">
      <nav><button className={inspectorTab === "compose" ? "active" : ""} onClick={() => setInspectorTab("compose")}><LayoutTemplate /> Compose</button><button className={inspectorTab === "output" ? "active" : ""} onClick={() => setInspectorTab("output")}><ImageIcon /> Takes</button>{advancedEnabled && <button className={inspectorTab === "advanced" ? "active" : ""} onClick={() => setInspectorTab("advanced")}><SlidersHorizontal /> Advanced</button>}</nav>
      {inspectorTab === "compose" && <div className="image-inspector-body">
        <label>Producer brief<textarea disabled={busy} value={project.idea} onChange={(event) => mutate((current) => ({ ...current, idea: event.target.value }))} placeholder="An idea, complete art direction, exact wording, or constraints…" /></label>
        <div className="image-assist"><Bot /><select aria-label="Local image collaborator" disabled={busy} value={modelId} onChange={(event) => setModelId(event.target.value)}><option value="">Choose local collaborator</option>{models.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}</select><button disabled={busy || !modelId} onClick={() => void startCollaboration()}><Sparkles /> Develop design</button></div>
        <label>High-level description<textarea disabled={busy} value={project.highLevelDescription} onChange={(event) => mutate((current) => ({ ...current, highLevelDescription: event.target.value }))} /></label>
        <div className="image-output-controls"><label>Canvas<select disabled={busy} value={`${project.settings.width}x${project.settings.height}`} onChange={(event) => { const [, width, height] = SIZE_PRESETS.find((item) => `${item[1]}x${item[2]}` === event.target.value) ?? SIZE_PRESETS[0]; mutate((current) => ({ ...current, settings: { ...current.settings, width, height } })); }}>{SIZE_PRESETS.map(([label, width, height]) => <option key={label} value={`${width}x${height}`}>{label}</option>)}</select></label><label>Variations<select disabled={busy} value={project.settings.batchSize} onChange={(event) => mutate((current) => ({ ...current, settings: { ...current.settings, batchSize: Number(event.target.value) } }))}><option value={1}>1 image</option><option value={2}>2 images</option><option value={4}>4 images</option></select></label></div>
        <div className="image-style-mode" role="group" aria-label="Image style type"><button className={project.style.mode === "photo" ? "active" : ""} disabled={busy} onClick={() => mutate((current) => ({ ...current, style: { ...current.style, mode: "photo" } }))}>Photography</button><button className={project.style.mode === "art" ? "active" : ""} disabled={busy} onClick={() => mutate((current) => ({ ...current, style: { ...current.style, mode: "art" } }))}>Artwork</button></div>
        <div className="image-field-pair"><label>Medium<input disabled={busy} value={project.style.medium} onChange={(event) => mutate((current) => ({ ...current, style: { ...current.style, medium: event.target.value } }))} /></label><label>Global palette · max 16<input disabled={busy} value={project.style.colorPalette.join(", ")} onChange={(event) => mutate((current) => ({ ...current, style: { ...current.style, colorPalette: parsePalette(event.target.value, 16) } }))} placeholder="#24313A, #D9D2C3" /></label></div>
        <label>Aesthetics<textarea disabled={busy} value={project.style.aesthetics} onChange={(event) => mutate((current) => ({ ...current, style: { ...current.style, aesthetics: event.target.value } }))} /></label>
        <div className="image-field-pair"><label>Lighting<textarea disabled={busy} value={project.style.lighting} onChange={(event) => mutate((current) => ({ ...current, style: { ...current.style, lighting: event.target.value } }))} /></label>{project.style.mode === "photo" ? <label>Photo treatment<textarea disabled={busy} value={project.style.photo} onChange={(event) => mutate((current) => ({ ...current, style: { ...current.style, photo: event.target.value } }))} /></label> : <label>Art style<textarea disabled={busy} value={project.style.artStyle} onChange={(event) => mutate((current) => ({ ...current, style: { ...current.style, artStyle: event.target.value } }))} /></label>}</div>
        <label>Background<textarea disabled={busy} value={project.background} onChange={(event) => mutate((current) => ({ ...current, background: event.target.value }))} /></label>
        <section className="image-element-panel"><header><span><small>Visual layout</small><strong>Objects and exact text</strong></span><div><button disabled={busy || project.elements.length >= 64} onClick={() => addElement("obj")}><Square /> Object</button><button disabled={busy || project.elements.length >= 64} onClick={() => addElement("text")}><Type /> Text</button></div></header>
          <div className="image-element-chips">{project.elements.map((element, index) => <button className={element.id === selectedElementId ? "active" : ""} key={element.id} onClick={() => setSelectedElementId(element.id)}><i>{index + 1}</i>{element.kind === "text" ? element.text || "Text" : element.description || "Object"}</button>)}</div>
          {!project.elements.length && <div className="image-layout-hint"><span>Free-form images work, but a box gives Ideogram stronger placement direction.</span><button disabled={busy} onClick={() => addElement("obj", [0, 0, 1000, 1000])}><Maximize2 /> Full canvas subject</button></div>}
          {selectedElement && <fieldset disabled={busy}><div className="image-element-title"><strong>{selectedElement.kind === "text" ? "Exact text layer" : "Object layer"}</strong><span><button type="button" title="Move layer down" onClick={() => moveElementLayer(-1)}><ArrowDown /></button><button type="button" title="Move layer up" onClick={() => moveElementLayer(1)}><ArrowUp /></button><button type="button" title="Duplicate layer · Ctrl/Cmd+D" onClick={duplicateElement}><Copy /></button><button type="button" className="danger" title="Remove layer · Delete" onClick={removeElement}><X /></button></span></div>{selectedElement.kind === "text" && <label>Visible wording<input value={selectedElement.text} onChange={(event) => patchElement(mutate, selectedElement.id, { text: event.target.value })} /></label>}<label>Description<textarea value={selectedElement.description} onChange={(event) => patchElement(mutate, selectedElement.id, { description: event.target.value })} /></label><label>Layer palette · max 5<input value={selectedElement.colorPalette.join(", ")} onChange={(event) => patchElement(mutate, selectedElement.id, { colorPalette: parsePalette(event.target.value, 5) })} /></label><div className="image-box-fields">{["Top", "Left", "Bottom", "Right"].map((label, index) => <label key={label}>{label}<input type="number" min={0} max={1000} value={selectedElement.bbox[index]} onChange={(event) => patchBox(mutate, selectedElement, index, event.currentTarget.valueAsNumber)} /></label>)}</div><small className="image-key-hint">Alt-click cycles overlapping layers · arrows nudge · Shift+arrows move 10</small></fieldset>}
        </section>
      </div>}
      {inspectorTab === "output" && <div className="image-inspector-body image-output-inspector"><div className="image-output-heading"><span><small>Preserved output</small><strong>{project.takes.length} immutable takes</strong></span><button aria-label="Show image project in File Explorer" onClick={() => void revealImageProject(project.id)}><FolderOpen /></button></div>{[...project.takes].reverse().map((take, index) => <div className={`image-output-take ${take.id === project.activeTakeId ? "active" : ""}`} key={take.id}><button disabled={busy || take.status !== "complete"} onClick={() => mutate((current) => ({ ...current, activeTakeId: take.id }))}>{take.path ? <img src={imageMediaUrl(take.path)} alt="" /> : <ImageIcon />}<span><strong>Take {project.takes.length - index}{take.batchSize > 1 ? ` · variation ${take.batchIndex}/${take.batchSize}` : ""}</strong><small>{take.status} · {take.width}×{take.height} · seed {take.seed}</small><code>{take.sha256 ? `${take.sha256.slice(0, 16)}…` : take.detail}</code></span></button>{take.path && <a href={imageMediaUrl(take.path, true)} download aria-label={`Download take ${project.takes.length - index}`}><Download /></a>}</div>)}<div className="image-license-note"><strong>Non-commercial model</strong><p>{project.licenseNotice}</p><a href="https://github.com/ideogram-oss/ideogram4/blob/main/model_licenses/LICENSE-IDEOGRAM-4-NON-COMMERCIAL" target="_blank" rel="noreferrer">Read model agreement</a></div></div>}
      {inspectorTab === "advanced" && advancedEnabled && <div className="image-inspector-body image-advanced">
        <label>Sampling preset<select disabled={busy} value={project.settings.preset} onChange={(event) => mutate((current) => ({ ...current, settings: { ...current.settings, preset: event.target.value as ImageProject["settings"]["preset"] } }))}><option value="quality">Quality · 48 steps</option><option value="standard">Standard · 20 steps</option><option value="turbo">Turbo · 12 steps</option></select></label>
        <label>Canvas size<select disabled={busy} value={`${project.settings.width}x${project.settings.height}`} onChange={(event) => { const [, width, height] = SIZE_PRESETS.find((item) => `${item[1]}x${item[2]}` === event.target.value) ?? SIZE_PRESETS[0]; mutate((current) => ({ ...current, settings: { ...current.settings, width, height } })); }}>{SIZE_PRESETS.map(([label, width, height]) => <option key={label} value={`${width}x${height}`}>{label} · {width}×{height}</option>)}</select></label>
        <div className="image-field-pair"><label>Width<input disabled={busy} type="number" min={256} max={2048} step={16} value={project.settings.width} onChange={(event) => finiteNumber(event.currentTarget.valueAsNumber, 256, 2048, (width) => mutate((current) => ({ ...current, settings: { ...current.settings, width: round16(width) } })))} /></label><label>Height<input disabled={busy} type="number" min={256} max={2048} step={16} value={project.settings.height} onChange={(event) => finiteNumber(event.currentTarget.valueAsNumber, 256, 2048, (height) => mutate((current) => ({ ...current, settings: { ...current.settings, height: round16(height) } })))} /></label></div>
        <label>Variations in one render<select disabled={busy} value={project.settings.batchSize} onChange={(event) => mutate((current) => ({ ...current, settings: { ...current.settings, batchSize: Number(event.target.value) } }))}><option value={1}>1 variation</option><option value={2}>2 variations</option><option value={4}>4 variations</option></select></label>
        <label>Seed<input disabled={busy} type="number" min={0} max={2147483647} value={project.settings.seed} onChange={(event) => finiteNumber(event.currentTarget.valueAsNumber, 0, 2147483647, (seed) => mutate((current) => ({ ...current, settings: { ...current.settings, seed } })))} /></label>
        <div className="image-seed-actions"><button disabled={busy} onClick={() => mutate((current) => ({ ...current, settings: { ...current.settings, seed: 0 } }))}>Random every render</button><button disabled={busy} onClick={() => mutate((current) => ({ ...current, settings: { ...current.settings, seed: secureSeed() } }))}>New fixed seed</button></div>
        <p className="image-backdrop-note"><Eye /> The visible take is an alignment backdrop only. Ideogram receives the structured caption, seed, and boxes—not the image.</p>
        <details><summary>Compiled structured prompt</summary><pre>{JSON.stringify(compiledPrompt(project), null, 2)}</pre></details>{activeTake && <details><summary>Active take receipt</summary><dl><dt>Model</dt><dd>{activeTake.modelProfile}</dd><dt>Seed</dt><dd>{activeTake.seed}</dd><dt>SHA-256</dt><dd>{activeTake.sha256}</dd><dt>Prompt ID</dt><dd>{activeTake.promptId}</dd><dt>Batch</dt><dd>{activeTake.batchIndex}/{activeTake.batchSize}</dd></dl><pre>{activeTake.exactPromptText}</pre><pre>{JSON.stringify(activeTake.exactGraph, null, 2)}</pre></details>}
      </div>}
    </aside>

    {collaboration && <section className="image-collaboration-sheet"><header><span><Sparkles /><strong>Structured design proposal</strong><small>{collaboration.modelName} · {collaboration.status}</small></span><button aria-label="Close proposal" disabled={assistantBusy} onClick={() => setCollaboration(undefined)}>×</button></header><div className="model-collaboration-streams"><ModelThinkingStream text={collaboration.reasoning} active={assistantBusy} modelName={collaboration.modelName} /><section className="model-result-stream"><strong>Proposed production settings</strong><pre>{collaboration.text || (assistantBusy ? "The structured proposal will stream here when the model begins its answer…" : "No structured proposal was returned.")}</pre></section></div><footer>{assistantBusy ? <button onClick={() => void cancelMoviePromptDraft(collaboration.id)}><CircleStop /> Stop with checkpoint</button> : <><button onClick={() => void navigator.clipboard.writeText(collaboration.text)}><Copy /> Copy JSON</button><button onClick={() => setCollaboration(undefined)}>Discard</button><button className="primary-button" disabled={!collaboration.text.trim()} onClick={applyCollaboration}><Check /> Apply proposal</button></>}{advancedEnabled && collaboration.receipt && <details><summary>Exact model request</summary><pre>{JSON.stringify(collaboration.receipt, null, 2)}</pre></details>}</footer></section>}
    {newOpen && <NewImageDialog title={newTitle} idea={newIdea} busy={creating} onTitle={setNewTitle} onIdea={setNewIdea} onClose={() => setNewOpen(false)} onCreate={() => void create()} />}
  </div>;
}

function TakeStrip({ project, activeTake, disabled, onSelect }: { project: ImageProject; activeTake?: ImageTake; disabled: boolean; onSelect: (id: string) => void }) {
  const completed = project.takes.filter((take) => take.status === "complete");
  return <footer className="image-take-strip"><div><small>Contact sheet</small><strong>{activeTake ? `Active · ${activeTake.width}×${activeTake.height}${activeTake.batchSize > 1 ? ` · ${activeTake.batchIndex}/${activeTake.batchSize}` : ""}` : "No completed takes"}</strong></div><div>{completed.map((take, index) => <button className={take.id === project.activeTakeId ? "active" : ""} key={take.id} disabled={disabled} onClick={() => onSelect(take.id)}><img src={imageMediaUrl(take.path)} alt={`Take ${index + 1}`} /><span>{take.batchSize > 1 ? `${take.batchIndex}/${take.batchSize}` : index + 1}</span></button>)}{!completed.length && <span>New variations appear here without replacing earlier work.</span>}</div></footer>;
}

function ImageStudioEmpty({ onNew, dialog }: { onNew: () => void; dialog?: React.ReactNode }) {
  return <div className="image-studio-empty"><div className="image-empty-frame"><ImageIcon /><span /></div><small>Private visual development</small><h1>A complete image desk, not a node graph.</h1><p>Direct the concept, typography, palette, and layout; collaborate with any local model; then preserve full-resolution Ideogram 4 takes and exact receipts.</p><button className="primary-button" onClick={onNew}><Plus /> New image project</button>{dialog}</div>;
}

function NewImageDialog({ title, idea, busy, onTitle, onIdea, onClose, onCreate }: { title: string; idea: string; busy: boolean; onTitle: (value: string) => void; onIdea: (value: string) => void; onClose: () => void; onCreate: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog || dialog.open) return;
    if (typeof dialog.showModal === "function") dialog.showModal();
    else dialog.setAttribute("open", "");
  }, []);
  return <dialog ref={dialogRef} className="image-new-dialog" aria-labelledby="new-image-dialog-title" onCancel={onClose}><div className="image-dialog-icon"><Aperture /></div><div><small>New private image project</small><h2 id="new-image-dialog-title">Start with as much or as little as you have.</h2><p>A sentence, an A4 brief, exact copy, or an already-developed design all work. The project stays editable without the model.</p></div><label>Project name<input autoFocus disabled={busy} value={title} onChange={(event) => onTitle(event.target.value)} placeholder="Campaign key art" /></label><label>Idea or complete brief<textarea disabled={busy} value={idea} onChange={(event) => onIdea(event.target.value)} placeholder="A quiet editorial portrait… Include the exact headline ‘…’" /></label><footer><button disabled={busy} onClick={onClose}>Cancel</button><button className="primary-button" disabled={busy} onClick={onCreate}>{busy ? <LoaderCircle className="spin" /> : <Plus />} Create project</button></footer></dialog>;
}

export function compiledPrompt(project: ImageProject): Record<string, unknown> {
  const styleDescription = project.style.mode === "art" ? {
    aesthetics: project.style.aesthetics.trim(),
    lighting: project.style.lighting.trim(),
    medium: project.style.medium.trim(),
    art_style: project.style.artStyle.trim(),
    ...(project.style.colorPalette.length ? { color_palette: project.style.colorPalette } : {}),
  } : {
    aesthetics: project.style.aesthetics.trim(),
    lighting: project.style.lighting.trim(),
    photo: project.style.photo.trim(),
    medium: project.style.medium.trim(),
    ...(project.style.colorPalette.length ? { color_palette: project.style.colorPalette } : {}),
  };
  return {
    high_level_description: project.highLevelDescription.trim(),
    style_description: styleDescription,
    compositional_deconstruction: {
      background: project.background.trim(),
      elements: project.elements.map((element) => ({
        type: element.kind,
        bbox: element.bbox,
        ...(element.kind === "text" ? { text: element.text } : {}),
        desc: element.description.trim(),
        ...(element.colorPalette.length ? { color_palette: element.colorPalette } : {}),
      })),
    },
  };
}

export function parseImageProposal(text: string): Pick<ImageProject, "highLevelDescription" | "style" | "background" | "elements"> {
  const cleaned = text.trim().replace(/^```(?:json)?\s*/i, "").replace(/\s*```$/, "");
  const value = JSON.parse(cleaned) as Record<string, unknown>;
  const style = object(value.style_description, "style_description");
  const composition = object(value.compositional_deconstruction, "compositional_deconstruction");
  const rawElements = Array.isArray(composition.elements) ? composition.elements : [];
  if (rawElements.length > 64) throw new Error("The proposal contains more than 64 layout elements.");
  const elements = rawElements.map((entry, index): ImageElement => {
    const element = object(entry, `element ${index + 1}`);
    const kind = element.type === "text" ? "text" : element.type === "obj" ? "obj" : undefined;
    if (!kind) throw new Error(`Element ${index + 1} must be type obj or text.`);
    const bbox = parseBox(element.bbox, index);
    const description = string(element.desc, `element ${index + 1} description`);
    const exactText = kind === "text" ? string(element.text, `element ${index + 1} exact text`) : "";
    return { id: crypto.randomUUID(), kind, bbox, text: exactText, description, colorPalette: proposalPalette(element.color_palette, 5, `element ${index + 1} palette`) };
  });
  const hasPhoto = typeof style.photo === "string" && Boolean(style.photo.trim());
  const hasArtStyle = typeof style.art_style === "string" && Boolean(style.art_style.trim());
  if (hasPhoto === hasArtStyle) throw new Error("style_description must contain exactly one of photo or art_style.");
  return {
    highLevelDescription: string(value.high_level_description, "high_level_description"),
    style: {
      mode: hasArtStyle ? "art" : "photo",
      aesthetics: string(style.aesthetics, "style aesthetics"),
      lighting: string(style.lighting, "style lighting"),
      photo: hasPhoto ? string(style.photo, "photo treatment") : "Clean full-resolution image with restrained texture.",
      artStyle: hasArtStyle ? string(style.art_style, "art style") : "Editorial illustration with purposeful shape language and finished detail.",
      medium: string(style.medium, "medium"),
      colorPalette: proposalPalette(style.color_palette, 16, "global palette"),
    },
    background: string(composition.background, "background"),
    elements,
  };
}

function object(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`The proposal needs a valid ${label} object.`);
  return value as Record<string, unknown>;
}

function string(value: unknown, label: string): string {
  if (typeof value !== "string" || !value.trim()) throw new Error(`The proposal needs ${label} text.`);
  return value;
}

function proposalPalette(value: unknown, maximum: number, label: string): string[] {
  if (value === undefined) return [];
  if (!Array.isArray(value) || value.length > maximum || value.some((color) => typeof color !== "string" || !/^#[0-9a-f]{6}$/i.test(color))) throw new Error(`${label} must use at most ${maximum} #RRGGBB colors.`);
  return (value as string[]).map((color) => color.toUpperCase());
}

function parseBox(value: unknown, index: number): [number, number, number, number] {
  if (!Array.isArray(value) || value.length !== 4 || value.some((part) => !Number.isInteger(part) || Number(part) < 0 || Number(part) > 1000)) throw new Error(`Element ${index + 1} needs a four-integer bbox from 0 to 1000.`);
  const box = value.map(Number) as [number, number, number, number];
  if (box[0] >= box[2] || box[1] >= box[3]) throw new Error(`Element ${index + 1} bbox must be [top, left, bottom, right].`);
  return box;
}

function patchElement(mutate: (change: (current: ImageProject) => ImageProject) => void, id: string, patch: Partial<ImageElement>) {
  mutate((current) => ({ ...current, elements: current.elements.map((element) => element.id === id ? { ...element, ...patch } : element) }));
}

function patchBox(mutate: (change: (current: ImageProject) => ImageProject) => void, element: ImageElement, index: number, value: number) {
  if (!Number.isFinite(value) || value < 0 || value > 1000) return;
  const box = [...element.bbox] as [number, number, number, number];
  box[index] = Math.round(value);
  if (box[0] >= box[2] || box[1] >= box[3]) return;
  patchElement(mutate, element.id, { bbox: box });
}

function parsePalette(value: string, maximum: number): string[] {
  return value.split(/[\s,]+/).map((color) => color.trim().toUpperCase()).filter((color) => /^#[0-9A-F]{6}$/.test(color)).slice(0, maximum);
}

function moveBox(box: [number, number, number, number], dx: number, dy: number): [number, number, number, number] {
  const height = box[2] - box[0];
  const width = box[3] - box[1];
  const top = clamp(box[0] + dy, 0, 1000 - height);
  const left = clamp(box[1] + dx, 0, 1000 - width);
  return [top, left, top + height, left + width];
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}

function round16(value: number): number {
  return clamp(Math.round(value / 16) * 16, 256, 2048);
}

function finiteNumber(value: number, minimum: number, maximum: number, apply: (value: number) => void) {
  if (Number.isFinite(value) && value >= minimum && value <= maximum) apply(Math.round(value));
}

function secureSeed(): number {
  const seed = new Uint32Array(1);
  crypto.getRandomValues(seed);
  return (seed[0] % 2_147_483_647) + 1;
}

function formatEta(seconds: number): string {
  if (seconds < 60) return `${Math.max(1, Math.round(seconds))}s`;
  return `${Math.ceil(seconds / 60)} min`;
}
