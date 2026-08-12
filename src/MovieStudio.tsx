import {
  AudioLines, Check, ChevronDown, CircleStop, Clapperboard, Clock3, Download,
  Film, FolderOpen, ImageIcon, Library, LoaderCircle, Paperclip, Play, Plus,
  RotateCcw, Save, Send, Settings2, ShieldCheck, Sparkles, Video, X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  approveMoviePlan, askBonsaiMovieClip, cancelMovie, cancelMovieImageAsset, checkpointMoviePlanning,
  cancelMoviePromptDraft, directMoviePlanning, getMovie, getMoviePlanning, listMovieImageAssets, listMovies, movieMediaUrl,
  onMovieImageAsset, onMoviePlanning, onMovieProject, onMoviePromptDraft, pickMovieReferenceFiles, renderMovieClipVersion, renderMovieEdit,
  resumeMovie, revealMovie, reviseMoviePlan, saveMovieEdits, saveMoviePlan, startMovie,
  startMovieImageAsset, startMoviePromptDraft,
} from "./api";
import { MovieTimeline } from "./MovieTimeline";
import type {
  MovieClipSuggestion, MovieEdit, MoviePlan, MoviePlanningEvent, MoviePlanningSnapshot,
  ModelInfo, MovieImageAssetGeneration, MovieProject, MovieReferenceAsset, MovieSettings,
  MovieSummary, PendingMovieReference, PlannedClip, PromptDraftMode, PromptDraftReceipt,
  RenderedClip,
} from "./types";

type PromptField = { kind: "story" } | { kind: "imageAsset" } | {
  kind: "referenceDescription";
  assetId: string;
  part: "description" | "embeddedAudioDescription";
};
type ActivePromptDraft = { requestId: string; field: PromptField; mode: PromptDraftMode; originalText: string };

const defaultSettings: MovieSettings = {
  width: 1344,
  height: 768,
  clipSeconds: 5,
  steps: 20,
  maxClips: 12,
  seed: 0,
  temperature: 0.7,
  topP: 0.95,
  topK: 20,
  thinkingBudget: 32768,
  maxOutputTokens: 32768,
  comfyRoot: "D:\\AI\\ComfyUI",
  refImageSize: "match",
};

export function MovieStudio({ initialComfyRoot, advancedEnabled, models = [], selectedModelId, onError }: { initialComfyRoot?: string; advancedEnabled: boolean; models?: ModelInfo[]; selectedModelId?: string; onError: (message: string) => void }) {
  const [movies, setMovies] = useState<MovieSummary[]>([]);
  const [project, setProject] = useState<MovieProject | null>(null);
  const [creating, setCreating] = useState(true);
  const [prompt, setPrompt] = useState("");
  const [settings, setSettings] = useState(() => ({ ...defaultSettings, comfyRoot: initialComfyRoot || defaultSettings.comfyRoot }));
  const [advanced, setAdvanced] = useState(false);
  const [pauseAfterPlan, setPauseAfterPlan] = useState(true);
  const [promptModelId, setPromptModelId] = useState(() => selectedModelId ?? models[0]?.id ?? "");
  const [storyDraftMode, setStoryDraftMode] = useState<PromptDraftMode>("develop");
  const [imageDraftMode, setImageDraftMode] = useState<PromptDraftMode>("develop");
  const [referenceDraftModes, setReferenceDraftModes] = useState<Record<string, PromptDraftMode>>({});
  const [promptDraftActive, setPromptDraftActive] = useState<ActivePromptDraft>();
  const [promptDraftLastField, setPromptDraftLastField] = useState<PromptField>();
  const [promptDraftStatus, setPromptDraftStatus] = useState("");
  const [promptDraftReceipt, setPromptDraftReceipt] = useState<PromptDraftReceipt>();
  const [imagePrompt, setImagePrompt] = useState("");
  const [imageWidth, setImageWidth] = useState(768);
  const [imageHeight, setImageHeight] = useState(1344);
  const [imageSteps, setImageSteps] = useState(20);
  const [imageSeed, setImageSeed] = useState(0);
  const [imageStabilize, setImageStabilize] = useState(true);
  const [imageGenerating, setImageGenerating] = useState(false);
  const [imageStatus, setImageStatus] = useState("");
  const [imageGenerations, setImageGenerations] = useState<MovieImageAssetGeneration[]>([]);
  const [busy, setBusy] = useState(false);
  const [edit, setEdit] = useState<MovieEdit>({ clips: [], exportTitle: "Kestrel Movie", exportPreset: "publish", normalizeAudio: false, targetLufs: -14 });
  const [references, setReferences] = useState<PendingMovieReference[]>([]);
  const activeProjectId = useRef<string | undefined>(undefined);
  const promptDraftActiveRef = useRef<ActivePromptDraft | undefined>(undefined);
  const imageRequestId = useRef<string | undefined>(undefined);

  useEffect(() => {
    if (models.some((model) => model.id === promptModelId)) return;
    const selected = selectedModelId && models.some((model) => model.id === selectedModelId)
      ? selectedModelId
      : models[0]?.id ?? "";
    setPromptModelId(selected);
  }, [models, selectedModelId, promptModelId]);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    void onMoviePromptDraft((event) => {
      const active = promptDraftActiveRef.current;
      if (!active || event.requestId !== active.requestId) return;
      if (event.kind === "token" && event.content) {
        if (active.field.kind === "story") setPrompt((value) => value + event.content);
        if (active.field.kind === "imageAsset") setImagePrompt((value) => value + event.content);
        if (active.field.kind === "referenceDescription") {
          const { assetId, part } = active.field;
          setReferences((known) => known.map((item) => item.assetId === assetId
            ? { ...item, [part]: item[part] + event.content }
            : item));
        }
      } else if (event.kind === "queued") {
        setPromptDraftStatus(`Loading ${event.modelName ?? "local model"}…`);
      } else if (event.kind === "started") {
        setPromptDraftStatus("Writing locally… tokens appear as they are produced.");
        if (event.receipt) setPromptDraftReceipt(event.receipt);
      } else if (event.kind === "reasoning") {
        setPromptDraftStatus("The local model is thinking before it writes…");
      } else if (event.kind === "limited") {
        setPromptDraftStatus("Stopped at this field’s safe size limit. The partial text is preserved.");
      } else if (event.kind === "complete") {
        setPromptDraftStatus("Draft ready — review or edit anything before continuing.");
      } else if (event.kind === "cancelled") {
        setPromptDraftStatus("Stopped at a safe checkpoint. The text produced so far is preserved.");
      } else if (event.kind === "error") {
        setPromptDraftStatus("Local writing stopped. Any generated text is preserved.");
        onError(event.content ?? "Local prompt collaboration failed.");
      } else if (event.kind === "settled") {
        promptDraftActiveRef.current = undefined;
        setPromptDraftActive(undefined);
      }
    }).then((unlisten) => { dispose = unlisten; });
    return () => dispose?.();
  }, [onError]);

  const refreshImageAssets = useCallback(async () => {
    try { setImageGenerations(await listMovieImageAssets()); } catch (error) { onError(String(error)); }
  }, [onError]);

  useEffect(() => {
    void refreshImageAssets();
    let dispose: (() => void) | undefined;
    void onMovieImageAsset((event) => {
      if (event.requestId !== imageRequestId.current) return;
      setImageStatus(event.detail);
      if (event.kind === "complete" && event.generation) {
        setImageGenerations((known) => [event.generation!, ...known.filter((item) => item.id !== event.generation!.id)]);
        setImageGenerating(false);
        imageRequestId.current = undefined;
      } else if (event.kind === "cancelled") {
        setImageGenerating(false);
        imageRequestId.current = undefined;
        void refreshImageAssets();
      } else if (event.kind === "error") {
        setImageGenerating(false);
        imageRequestId.current = undefined;
        onError(event.detail || "Local H3 image generation failed.");
        void refreshImageAssets();
      }
    }).then((unlisten) => { dispose = unlisten; });
    return () => dispose?.();
  }, [onError, refreshImageAssets]);

  const refreshList = useCallback(async () => {
    try { setMovies(await listMovies()); } catch (error) { onError(String(error)); }
  }, [onError]);

  useEffect(() => {
    void refreshList();
    let dispose: (() => void) | undefined;
    void onMovieProject((next) => {
      if (activeProjectId.current && activeProjectId.current !== next.id) return;
      activeProjectId.current = next.id;
      setProject(next);
      setEdit(next.edit);
      void refreshList();
    }).then((unlisten) => { dispose = unlisten; });
    return () => dispose?.();
  }, [refreshList]);

  useEffect(() => {
    if (!project || project.status !== "running") return;
    let active = true;
    const timer = window.setInterval(() => void getMovie(project.id).then((next) => {
      if (!active) return;
      setProject(next); setEdit(next.edit);
    }).catch(() => undefined), 2500);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [project?.id, project?.status]);

  const openProject = async (id: string) => {
    const previousId = activeProjectId.current;
    activeProjectId.current = id;
    try {
      const next = await getMovie(id);
      setProject(next); setEdit(next.edit); setCreating(false);
    } catch (error) { activeProjectId.current = previousId; onError(String(error)); }
  };

  const makeMovie = async () => {
    if (!prompt.trim() || !referencesReady(references)) return;
    setBusy(true);
    try {
      const next = await startMovie({
        prompt,
        settings,
        references: references.map(({ assetId, description, useEmbeddedAudio, embeddedAudioDescription }) => ({
          assetId, description, useEmbeddedAudio, embeddedAudioDescription,
        })),
        pauseAfterPlan,
      });
      activeProjectId.current = next.id;
      setProject(next); setEdit(next.edit); setCreating(false); await refreshList();
    } catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const setPromptField = (field: PromptField, value: string) => {
    if (field.kind === "story") setPrompt(value);
    if (field.kind === "imageAsset") setImagePrompt(value);
    if (field.kind === "referenceDescription") {
      setReferences((known) => known.map((item) => item.assetId === field.assetId ? { ...item, [field.part]: value } : item));
    }
  };

  const generatePromptDraft = async (field: PromptField, requestedMode: PromptDraftMode) => {
    if (!promptModelId || promptDraftActiveRef.current) return;
    const reference = field.kind === "referenceDescription" ? references.find((item) => item.assetId === field.assetId) : undefined;
    if (field.kind === "referenceDescription" && !reference) return;
    const originalText = field.kind === "story" ? prompt : field.kind === "imageAsset" ? imagePrompt : reference![field.part];
    const existingText = originalText.trimEnd();
    const mode: PromptDraftMode = existingText ? requestedMode : "develop";
    const requestId = crypto.randomUUID();
    const active = { requestId, field, mode, originalText } satisfies ActivePromptDraft;
    promptDraftActiveRef.current = active;
    setPromptDraftActive(active);
    setPromptDraftLastField(field);
    setPromptDraftStatus(mode === "continue" ? "Preparing to continue the exact draft…" : existingText ? "Preparing to develop the idea and replace this field…" : "Preparing an original draft…");
    setPromptField(field, mode === "continue" && existingText ? `${existingText}\n\n` : "");
    try {
      await startMoviePromptDraft({
        requestId,
        modelId: promptModelId,
        target: field.kind,
        mode,
        storyText: field.kind === "story" ? "" : prompt,
        existingText,
        assetName: reference ? (field.kind === "referenceDescription" && field.part === "embeddedAudioDescription" ? `embedded audio from ${reference.name}` : reference.name) : "",
        assetKind: reference ? (field.kind === "referenceDescription" && field.part === "embeddedAudioDescription" ? "audio" : reference.kind) : "",
      });
    } catch (error) {
      promptDraftActiveRef.current = undefined;
      setPromptDraftActive(undefined);
      setPromptField(field, originalText);
      setPromptDraftStatus("");
      onError(String(error));
    }
  };

  const stopPromptDraft = async () => {
    const active = promptDraftActiveRef.current;
    if (!active) return;
    setPromptDraftStatus("Stopping after the current local token…");
    try {
      await cancelMoviePromptDraft(active.requestId);
    } catch (error) {
      onError(String(error));
    }
  };

  const generateImageAsset = async () => {
    if (imageGenerating || imagePrompt.trim().length < 3) return;
    const requestId = crypto.randomUUID();
    imageRequestId.current = requestId;
    setImageGenerating(true);
    setImageStatus("Preparing the private H3 image workflow…");
    try {
      await startMovieImageAsset({
        requestId,
        prompt: imagePrompt,
        width: imageWidth,
        height: imageHeight,
        steps: imageSteps,
        seed: imageSeed,
        comfyRoot: settings.comfyRoot,
        stabilize: imageStabilize,
      });
    } catch (error) {
      imageRequestId.current = undefined;
      setImageGenerating(false);
      setImageStatus("");
      onError(String(error));
    }
  };

  const stopImageAsset = async () => {
    const requestId = imageRequestId.current;
    if (!requestId) return;
    setImageStatus("Stopping the local image pass…");
    try { await cancelMovieImageAsset(requestId); } catch (error) { onError(String(error)); }
  };

  const useGeneratedImage = (asset: MovieReferenceAsset) => {
    if (references.some((reference) => reference.assetId === asset.id)) return;
    if (references.filter((reference) => reference.kind === "image").length >= 9) {
      onError("MiniMax H3 accepts at most 9 picture references. Remove one before adding this candidate.");
      return;
    }
    const sourcePrompt = asset.generation?.prompt.trim().replace(/\s+/g, " ") ?? "";
    const description = `Use this generated image as a native visual identity and art-direction reference.${sourcePrompt ? ` Generation brief: ${sourcePrompt.slice(0, 1_000)}` : ""}`;
    setReferences((known) => [...known, {
      ...asset,
      assetId: asset.id,
      description,
      useEmbeddedAudio: false,
      embeddedAudioDescription: "",
    }]);
  };

  const attachReferences = async () => {
    setBusy(true);
    try {
      const imported = await pickMovieReferenceFiles();
      if (imported.failures.length) onError(imported.failures.join("\n"));
      const additions = imported.references
        .filter((asset) => !references.some((known) => known.assetId === asset.id))
        .map((asset) => ({ ...asset, assetId: asset.id, description: "", useEmbeddedAudio: false, embeddedAudioDescription: "" }));
      const next = [...references, ...additions];
      const pictures = next.filter((reference) => reference.kind === "image").length;
      const videos = next.filter((reference) => reference.kind === "video").length;
      const audios = next.filter((reference) => reference.kind === "audio" || reference.useEmbeddedAudio).length;
      if (pictures > 9 || videos > 3 || audios > 3) {
        onError("MiniMax H3 accepts at most 9 pictures, 3 videos, and 3 audio signals. Remove a reference before adding another.");
        return;
      }
      setReferences(next);
    } catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const saveEdits = async (exportNow: boolean) => {
    if (!project) return;
    setBusy(true);
    try {
      let next = await saveMovieEdits(project.id, edit);
      if (exportNow) next = await renderMovieEdit(project.id);
      setProject(next); setEdit(next.edit);
    } catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  return (
    <div className="movie-studio">
      <aside className="movie-library">
        <div className="movie-library-title"><span>Private movie library</span><button onClick={() => { activeProjectId.current = undefined; setCreating(true); setProject(null); }}><Plus size={15} /></button></div>
        <div className="movie-list">
          {movies.map((movie) => <button key={movie.id} className={project?.id === movie.id ? "active" : ""} onClick={() => void openProject(movie.id)}>
            <Film size={15} /><span><strong>{movie.title}</strong><small>{movie.phase} · {movie.clipCount} clips</small></span>
          </button>)}
          {!movies.length && <div className="movie-empty-list"><Library size={18} />Your durable productions will appear here.</div>}
        </div>
      </aside>
      <section className="movie-stage">
        {creating || !project ? (
          <MovieLaunch prompt={prompt} settings={settings} references={references} advanced={advanced} advancedEnabled={advancedEnabled} busy={busy}
            pauseAfterPlan={pauseAfterPlan} onPauseAfterPlan={setPauseAfterPlan}
            models={models} promptModelId={promptModelId} promptDraftActive={promptDraftActive} promptDraftLastField={promptDraftLastField} promptDraftStatus={promptDraftStatus}
            promptDraftReceipt={promptDraftReceipt} storyDraftMode={storyDraftMode} imageDraftMode={imageDraftMode} referenceDraftModes={referenceDraftModes}
            onPromptModel={setPromptModelId} onStoryDraftMode={setStoryDraftMode} onImageDraftMode={setImageDraftMode}
            onReferenceDraftMode={(assetId, mode) => setReferenceDraftModes((known) => ({ ...known, [assetId]: mode }))}
            onGeneratePrompt={(field, mode) => void generatePromptDraft(field, mode)} onStopPrompt={() => void stopPromptDraft()}
            imagePrompt={imagePrompt} imageWidth={imageWidth} imageHeight={imageHeight} imageSteps={imageSteps} imageSeed={imageSeed}
            imageStabilize={imageStabilize} imageGenerating={imageGenerating} imageStatus={imageStatus} imageGenerations={imageGenerations}
            onImagePrompt={setImagePrompt} onImageCanvas={(width, height) => { setImageWidth(width); setImageHeight(height); }}
            onImageSteps={setImageSteps} onImageSeed={setImageSeed} onImageStabilize={setImageStabilize}
            onGenerateImage={() => void generateImageAsset()} onStopImage={() => void stopImageAsset()} onUseGeneratedImage={useGeneratedImage}
            onPrompt={setPrompt} onSettings={setSettings} onReferences={setReferences} onAttach={() => void attachReferences()} onAdvanced={setAdvanced} onMake={() => void makeMovie()} />
        ) : (
          <MovieProjectView project={project} edit={edit} busy={busy} advancedEnabled={advancedEnabled} onError={onError} onEdit={setEdit}
            onProject={(next) => { activeProjectId.current = next.id; setProject(next); setEdit(next.edit); void refreshList(); }}
            onNew={() => { activeProjectId.current = undefined; setCreating(true); setProject(null); }}
            onCancel={() => void cancelMovie(project.id).then(setProject).catch((error) => onError(String(error)))}
            onResume={() => void resumeMovie(project.id).then(setProject).catch((error) => onError(String(error)))}
            onReveal={() => void revealMovie(project.id)}
            onSave={() => void saveEdits(false)} onExport={() => void saveEdits(true)} />
        )}
      </section>
    </div>
  );
}

function MovieLaunch({ prompt, settings, references, advanced, advancedEnabled, busy, pauseAfterPlan, onPauseAfterPlan, models, promptModelId, promptDraftActive, promptDraftLastField, promptDraftStatus, promptDraftReceipt, storyDraftMode, imageDraftMode, referenceDraftModes, onPromptModel, onStoryDraftMode, onImageDraftMode, onReferenceDraftMode, onGeneratePrompt, onStopPrompt, imagePrompt, imageWidth, imageHeight, imageSteps, imageSeed, imageStabilize, imageGenerating, imageStatus, imageGenerations, onImagePrompt, onImageCanvas, onImageSteps, onImageSeed, onImageStabilize, onGenerateImage, onStopImage, onUseGeneratedImage, onPrompt, onSettings, onReferences, onAttach, onAdvanced, onMake }: {
  prompt: string; settings: MovieSettings; references: PendingMovieReference[]; advanced: boolean; advancedEnabled: boolean; busy: boolean;
  pauseAfterPlan: boolean; onPauseAfterPlan: (value: boolean) => void;
  models: ModelInfo[]; promptModelId: string; promptDraftActive?: ActivePromptDraft; promptDraftLastField?: PromptField; promptDraftStatus: string; promptDraftReceipt?: PromptDraftReceipt;
  storyDraftMode: PromptDraftMode; imageDraftMode: PromptDraftMode; referenceDraftModes: Record<string, PromptDraftMode>;
  onPromptModel: (value: string) => void; onStoryDraftMode: (value: PromptDraftMode) => void; onImageDraftMode: (value: PromptDraftMode) => void;
  onReferenceDraftMode: (assetId: string, value: PromptDraftMode) => void;
  onGeneratePrompt: (field: PromptField, mode: PromptDraftMode) => void; onStopPrompt: () => void;
  imagePrompt: string; imageWidth: number; imageHeight: number; imageSteps: number; imageSeed: number; imageStabilize: boolean;
  imageGenerating: boolean; imageStatus: string; imageGenerations: MovieImageAssetGeneration[];
  onImagePrompt: (value: string) => void; onImageCanvas: (width: number, height: number) => void;
  onImageSteps: (value: number) => void; onImageSeed: (value: number) => void; onImageStabilize: (value: boolean) => void;
  onGenerateImage: () => void; onStopImage: () => void; onUseGeneratedImage: (asset: MovieReferenceAsset) => void;
  onPrompt: (value: string) => void; onSettings: (value: MovieSettings) => void; onReferences: (value: PendingMovieReference[]) => void;
  onAttach: () => void; onAdvanced: (value: boolean) => void; onMake: () => void;
}) {
  const quality = settings.width === 1344 ? "master" : settings.width === 864 ? "preview" : "custom";
  const storyWriting = promptFieldMatches(promptDraftActive?.field, { kind: "story" });
  const imageWriting = promptFieldMatches(promptDraftActive?.field, { kind: "imageAsset" });
  const promptBusy = Boolean(promptDraftActive);
  const statusField = promptDraftActive?.field ?? promptDraftLastField;
  return <div className="movie-launch">
    <div className="movie-launch-mark"><Clapperboard /></div>
    <span className="eyebrow">Bonsai director · MiniMax H3 picture & sound</span>
    <h1>Describe the movie.<br />Kestrel runs the studio.</h1>
    <p>One prompt becomes a reviewed screenplay, continuity bible, native H3 picture-and-sound scenes, and an untouched review cut—entirely on this computer.</p>
    <div className="movie-prompt-box">
      <textarea aria-label="Movie brief" autoFocus maxLength={65536} value={prompt} readOnly={storyWriting} onChange={(event) => onPrompt(event.target.value)} placeholder="Write or paste your story here—even an A4-length brief—or ask any local model to develop an idea…" />
      <div><span><Check size={14} /> Bonsai drafts, reviews, and repairs every H3 scene prompt</span><button disabled={busy || promptBusy || imageGenerating || prompt.trim().length < 3 || !referencesReady(references)} onClick={onMake}>{busy ? <LoaderCircle className="spin" /> : <Sparkles />} Make movie</button></div>
    </div>
    <PromptAssistBar label="Movie brief" existing={prompt} mode={storyDraftMode} models={models} modelId={promptModelId}
      active={storyWriting} disabled={busy || imageGenerating || (promptBusy && !storyWriting)} status={promptFieldMatches(statusField, { kind: "story" }) ? promptDraftStatus : ""}
      onModel={onPromptModel} onMode={onStoryDraftMode} onGenerate={() => onGeneratePrompt({ kind: "story" }, storyDraftMode)} onStop={onStopPrompt} />
    <ImageAssetLab
      prompt={imagePrompt} width={imageWidth} height={imageHeight} steps={imageSteps} seed={imageSeed}
      stabilize={imageStabilize} generating={imageGenerating} status={imageStatus} generations={imageGenerations}
      references={references} advanced={advanced} expertEnabled={advancedEnabled} disabled={busy || promptBusy}
      models={models} modelId={promptModelId} draftMode={imageDraftMode} draftActive={imageWriting} draftStatus={promptFieldMatches(statusField, { kind: "imageAsset" }) ? promptDraftStatus : ""}
      onModel={onPromptModel} onDraftMode={onImageDraftMode} onDraft={() => onGeneratePrompt({ kind: "imageAsset" }, imageDraftMode)} onStopDraft={onStopPrompt}
      onPrompt={onImagePrompt} onCanvas={onImageCanvas} onSteps={onImageSteps} onSeed={onImageSeed}
      onStabilize={onImageStabilize} onGenerate={onGenerateImage} onStop={onStopImage} onUse={onUseGeneratedImage}
    />
    <section className="movie-reference-builder">
      <div className="movie-reference-heading"><div><span className="eyebrow">Producer references</span><strong>Show and tell H3 what must carry through</strong><small>Attach the actual media, then describe its job. Kestrel binds it natively per shot.</small></div><button disabled={busy || promptBusy} onClick={onAttach}><Paperclip /> Attach image, video, or audio</button></div>
      {references.length > 0 && <div className="movie-reference-grid">{references.map((reference) => {
        const labels = referenceDisplayTags(references, reference.assetId);
        return <article className="movie-reference-card" key={reference.assetId}>
          <ReferencePreview reference={reference} />
          <div className="movie-reference-copy"><div className="movie-reference-meta"><span>{labels.join(" + ")}</span><strong>{reference.name}</strong><button aria-label={`Remove ${reference.name}`} disabled={promptBusy} onClick={() => onReferences(references.filter((item) => item.assetId !== reference.assetId))}><X /></button></div>
            <small>{reference.kind}{reference.durationSeconds > 0 ? ` · ${reference.durationSeconds.toFixed(1)}s` : ` · ${reference.width}×${reference.height}`}</small>
            <label>How should Bonsai place this?<textarea aria-label={`Describe ${reference.name}`} maxLength={4000} readOnly={promptFieldMatches(promptDraftActive?.field, { kind: "referenceDescription", assetId: reference.assetId, part: "description" })} value={reference.description} onChange={(event) => onReferences(references.map((item) => item.assetId === reference.assetId ? { ...item, description: event.target.value } : item))} placeholder={reference.kind === "image" ? "Character identity, costume, palette, composition, or style…" : reference.kind === "video" ? "Motion, camera move, pacing, continuation, or temporal structure…" : "Where this exact clip audio belongs: dialogue performance, music, rhythm, ambience, or effects…"} /></label>
            <PromptAssistBar compact label={`${reference.kind} reference`} existing={reference.description} mode={referenceDraftModes[referenceDraftKey(reference.assetId, "description")] ?? "develop"} models={models} modelId={promptModelId}
              active={promptFieldMatches(promptDraftActive?.field, { kind: "referenceDescription", assetId: reference.assetId, part: "description" })}
              disabled={busy || imageGenerating || (promptBusy && !promptFieldMatches(promptDraftActive?.field, { kind: "referenceDescription", assetId: reference.assetId, part: "description" }))}
              status={promptFieldMatches(statusField, { kind: "referenceDescription", assetId: reference.assetId, part: "description" }) ? promptDraftStatus : ""}
              onModel={onPromptModel} onMode={(mode) => onReferenceDraftMode(referenceDraftKey(reference.assetId, "description"), mode)}
              onGenerate={() => onGeneratePrompt({ kind: "referenceDescription", assetId: reference.assetId, part: "description" }, referenceDraftModes[referenceDraftKey(reference.assetId, "description")] ?? "develop")} onStop={onStopPrompt} />
            {reference.kind === "video" && reference.hasAudio && <><label className="movie-audio-toggle"><input type="checkbox" disabled={promptBusy} checked={reference.useEmbeddedAudio} onChange={(event) => onReferences(references.map((item) => item.assetId === reference.assetId ? { ...item, useEmbeddedAudio: event.target.checked } : item))} /> Use the video's existing audio as native clip audio</label>{reference.useEmbeddedAudio && <><label>Where should this audio be used?<input aria-label={`Describe audio from ${reference.name}`} maxLength={4000} readOnly={promptFieldMatches(promptDraftActive?.field, { kind: "referenceDescription", assetId: reference.assetId, part: "embeddedAudioDescription" })} value={reference.embeddedAudioDescription} onChange={(event) => onReferences(references.map((item) => item.assetId === reference.assetId ? { ...item, embeddedAudioDescription: event.target.value } : item))} placeholder="The scenes or beats where this exact audio belongs…" /></label><PromptAssistBar compact label={`audio from ${reference.name}`} existing={reference.embeddedAudioDescription} mode={referenceDraftModes[referenceDraftKey(reference.assetId, "embeddedAudioDescription")] ?? "develop"} models={models} modelId={promptModelId}
              active={promptFieldMatches(promptDraftActive?.field, { kind: "referenceDescription", assetId: reference.assetId, part: "embeddedAudioDescription" })}
              disabled={busy || imageGenerating || (promptBusy && !promptFieldMatches(promptDraftActive?.field, { kind: "referenceDescription", assetId: reference.assetId, part: "embeddedAudioDescription" }))}
              status={promptFieldMatches(statusField, { kind: "referenceDescription", assetId: reference.assetId, part: "embeddedAudioDescription" }) ? promptDraftStatus : ""}
              onModel={onPromptModel} onMode={(mode) => onReferenceDraftMode(referenceDraftKey(reference.assetId, "embeddedAudioDescription"), mode)}
              onGenerate={() => onGeneratePrompt({ kind: "referenceDescription", assetId: reference.assetId, part: "embeddedAudioDescription" }, referenceDraftModes[referenceDraftKey(reference.assetId, "embeddedAudioDescription")] ?? "develop")} onStop={onStopPrompt} /></>}</>}
          </div>
        </article>;
      })}</div>}
      {!references.length && <div className="movie-reference-empty"><ImageIcon /><Video /><AudioLines /><span>Optional. Use references when identity, motion, camera, exact clip audio, or a visual language matters.</span></div>}
    </section>
    <div className="movie-presets">
      <button className={quality === "master" ? "active" : ""} onClick={() => onSettings({ ...settings, width: 1344, height: 768 })}><strong>Publish master</strong><span>1344 × 768 · highest H3 native canvas</span></button>
      <button className={quality === "preview" ? "active" : ""} onClick={() => onSettings({ ...settings, width: 864, height: 480 })}><strong>Faster draft</strong><span>864 × 480 · proven ~2½ min per clip</span></button>
    </div>
    <button className="movie-advanced-toggle" onClick={() => onAdvanced(!advanced)}><Settings2 size={14} /> Advanced production controls <ChevronDown className={advanced ? "open" : ""} size={14} /></button>
    {advanced && <div className="movie-advanced">
      <NumberField label="Clip seconds" value={settings.clipSeconds} min={5} max={15} step={1} onChange={(value) => onSettings({ ...settings, clipSeconds: value })} />
      <NumberField label="Maximum clips" value={settings.maxClips} min={1} max={advancedEnabled ? 96 : 24} step={1} onChange={(value) => onSettings({ ...settings, maxClips: value })} />
      <NumberField label="Sampling steps" value={settings.steps} min={1} max={advancedEnabled ? 100 : 40} step={1} onChange={(value) => onSettings({ ...settings, steps: value })} />
      <NumberField label="Seed (0 = random)" value={settings.seed} min={0} max={Number.MAX_SAFE_INTEGER} step={1} onChange={(value) => onSettings({ ...settings, seed: value })} />
      <NumberField label="Temperature" value={settings.temperature} min={0} max={2} step={0.05} onChange={(value) => onSettings({ ...settings, temperature: value })} />
      <NumberField label="Top P" value={settings.topP} min={0.05} max={1} step={0.01} onChange={(value) => onSettings({ ...settings, topP: value })} />
      <NumberField label="Top K" value={settings.topK} min={1} max={200} step={1} onChange={(value) => onSettings({ ...settings, topK: value })} />
      <label>Thinking mode<input value="Maximum · 32,768" disabled aria-label="Thinking mode is fixed at maximum" /></label>
      <NumberField label="Output budget" value={settings.maxOutputTokens} min={1024} max={32768} step={1024} onChange={(value) => onSettings({ ...settings, maxOutputTokens: value })} />
      <SelectField label="Reference image fidelity" value={settings.refImageSize} onChange={(value) => onSettings({ ...settings, refImageSize: value as MovieSettings["refImageSize"] })} options={["match", "max"]} />
      <label className="wide">ComfyUI root<input value={settings.comfyRoot} onChange={(event) => onSettings({ ...settings, comfyRoot: event.target.value })} /></label>
      {promptDraftReceipt && <details className="prompt-draft-receipt wide"><summary>Last prompt collaborator request — everything the model received</summary><div><span>Target / behavior</span><code>{promptDraftReceipt.target} · {promptDraftReceipt.mode}</code><span>Exact local API request</span><pre>{JSON.stringify(promptDraftReceipt.exactRequest, null, 2)}</pre></div></details>}
    </div>}
    <label className="wide producer-pause-toggle"><span><input type="checkbox" checked={pauseAfterPlan} onChange={(event) => onPauseAfterPlan(event.target.checked)} /> Review the plan before rendering</span><small>Recommended. Edit scenes or redirect Bonsai before any H3 clip is rendered.</small></label>
    <div className="movie-capabilities"><span><Check />98,304 context</span><span><Check />32,768 max thinking</span><span><Check />32,768 output</span><span><Check />Untouched H3 audio</span><span><Check />Crash-safe masters</span></div>
  </div>;
}

function PromptAssistBar({ label, existing, mode, models, modelId, active, disabled, status, compact = false, onModel, onMode, onGenerate, onStop }: {
  label: string; existing: string; mode: PromptDraftMode; models: ModelInfo[]; modelId: string; active: boolean; disabled: boolean; status: string; compact?: boolean;
  onModel: (value: string) => void; onMode: (value: PromptDraftMode) => void; onGenerate: () => void; onStop: () => void;
}) {
  const hasText = Boolean(existing.trim());
  const effectiveMode = hasText ? mode : "develop";
  const action = !hasText ? (label === "Movie brief" ? "Invent story" : "Generate description") : effectiveMode === "develop" ? "Develop idea / notes" : "Continue exact draft";
  return <div className={`prompt-assist-bar ${compact ? "compact" : ""} ${active ? "active" : ""}`}>
    <div className="prompt-assist-copy"><span className="eyebrow">Offline prompt collaborator</span><strong>{label}</strong><small>{status || (hasText ? "Tell the model whether this text is source material or an exact draft. Nothing is inferred." : "No text yet: the model will create a complete draft from the movie context or invent a useful direction.")}</small></div>
    <label>Local writing model<select aria-label={`${label} model`} value={modelId} disabled={active || disabled || !models.length} onChange={(event) => onModel(event.target.value)}>{!models.length && <option value="">No local models discovered</option>}{models.map((model) => <option key={model.id} value={model.id}>{model.name}{model.quantization ? ` · ${model.quantization}` : ""}</option>)}</select></label>
    <label>Existing text means<select aria-label={`${label} existing text meaning`} value={effectiveMode} disabled={active || disabled || !hasText} onChange={(event) => onMode(event.target.value as PromptDraftMode)}><option value="develop">Idea or notes — replace with a complete draft</option><option value="continue">Exact draft — keep it and continue</option></select></label>
    {active
      ? <button className="prompt-stop" onClick={onStop}><CircleStop /> Stop & keep text</button>
      : <button disabled={disabled || !modelId} onClick={onGenerate}><Sparkles /> {action}</button>}
  </div>;
}

function ImageAssetLab({ prompt, width, height, steps, seed, stabilize, generating, status, generations, references, advanced, expertEnabled, disabled, models, modelId, draftMode, draftActive, draftStatus, onModel, onDraftMode, onDraft, onStopDraft, onPrompt, onCanvas, onSteps, onSeed, onStabilize, onGenerate, onStop, onUse }: {
  prompt: string; width: number; height: number; steps: number; seed: number; stabilize: boolean;
  generating: boolean; status: string; generations: MovieImageAssetGeneration[]; references: PendingMovieReference[];
  advanced: boolean; expertEnabled: boolean; disabled: boolean; models: ModelInfo[]; modelId: string; draftMode: PromptDraftMode; draftActive: boolean; draftStatus: string;
  onModel: (value: string) => void; onDraftMode: (value: PromptDraftMode) => void; onDraft: () => void; onStopDraft: () => void;
  onPrompt: (value: string) => void; onCanvas: (width: number, height: number) => void;
  onSteps: (value: number) => void; onSeed: (value: number) => void; onStabilize: (value: boolean) => void;
  onGenerate: () => void; onStop: () => void; onUse: (asset: MovieReferenceAsset) => void;
}) {
  const canvas = `${width}x${height}`;
  const ready = generations.filter((generation) => generation.status === "complete" && generation.candidates.length).slice(0, 2);
  const recentIssue = generations[0] && generations[0].status !== "complete" ? generations[0] : undefined;
  return <section className="image-asset-lab">
    <div className="image-asset-heading">
      <div><span className="eyebrow">Offline image asset lab</span><strong>Generate characters, locations, props, posters, and style frames</strong><small>H3 renders one short private frame pass, then Kestrel preserves several stable candidates so you can choose the best image.</small></div>
      <span className="image-workflow-badge">H3 · 22 internal frames · 6 choices</span>
    </div>
    <div className="image-asset-compose">
      <label>Describe the exact image asset<textarea aria-label="Image asset prompt" maxLength={65536} value={prompt} readOnly={draftActive} disabled={generating} onChange={(event) => onPrompt(event.target.value)} placeholder="A precise character identity portrait, recurring location, hero prop, title poster, texture plate, or visual style frame… Include composition, lighting, palette, materials, and any exact text." /></label>
      <PromptAssistBar compact label="Image description" existing={prompt} mode={draftMode} models={models} modelId={modelId}
        active={draftActive} disabled={disabled || generating} status={draftStatus} onModel={onModel} onMode={onDraftMode} onGenerate={onDraft} onStop={onStopDraft} />
      <div className="image-asset-controls">
        <label>Canvas<select aria-label="Image canvas" value={canvas} disabled={generating} onChange={(event) => {
          const [nextWidth, nextHeight] = event.target.value.split("x").map(Number);
          onCanvas(nextWidth, nextHeight);
        }}><option value="768x1344">Portrait · 768 × 1344</option><option value="1344x768">Landscape · 1344 × 768</option><option value="1024x1024">Square · 1024 × 1024</option></select></label>
        <label className="image-stabilize"><input type="checkbox" checked={stabilize} disabled={generating} onChange={(event) => onStabilize(event.target.checked)} /><span><strong>Stabilize as a still image</strong><small>Recommended for consistent faces, geometry, and lettering.</small></span></label>
        {generating ? <button className="image-stop" onClick={onStop}><CircleStop /> Stop image pass</button> : <button disabled={disabled || prompt.trim().length < 3} onClick={onGenerate}><ImageIcon /> Generate candidates</button>}
      </div>
      {advanced && <div className="image-asset-advanced"><NumberField label="Image sampling steps" value={steps} min={1} max={expertEnabled ? 100 : 40} step={1} onChange={onSteps} /><NumberField label="Image seed (0 = random)" value={seed} min={0} max={Number.MAX_SAFE_INTEGER} step={1} onChange={onSeed} /></div>}
      {(generating || status) && <div className={`image-asset-status ${generating ? "running" : ""}`}>{generating && <LoaderCircle className="spin" />}<span>{status}</span></div>}
    </div>
    {ready.map((generation) => <article className="image-generation" key={generation.id}>
      <header><span><strong>{generation.width} × {generation.height} candidate strip</strong><small>{generation.candidates.length} preserved choices · seed {generation.seed} · {generation.steps} steps</small></span><small>{new Date(generation.completedAt || generation.createdAt).toLocaleString()}</small></header>
      <div className="image-candidate-grid">{generation.candidates.map(({ frameIndex, asset }) => {
        const selected = references.some((reference) => reference.assetId === asset.id);
        return <figure key={`${generation.id}-${frameIndex}`}>
          <img src={movieMediaUrl(asset.path)} alt={`Generated image candidate frame ${frameIndex}`} />
          <figcaption><span>Frame {frameIndex}{frameIndex === 8 ? " · workflow pick" : ""}</span><button disabled={selected} onClick={() => onUse(asset)}>{selected ? <Check /> : <Plus />}{selected ? "Added" : "Use image"}</button></figcaption>
        </figure>;
      })}</div>
      {advanced && <details className="image-generation-receipt"><summary>Exact H3 prompt, models, seed, and ComfyUI graph</summary><div><span>Workflow</span><code>{generation.workflow}</code><span>Fixed source</span><code>{generation.workflowSource}@{generation.workflowRevision}</code><span>Rendered prompt</span><pre>{generation.renderedPrompt}</pre><span>Exact API graph</span><pre>{JSON.stringify(generation.exactGraph, null, 2)}</pre></div></details>}
    </article>)}
    {recentIssue && !generating && <div className="image-generation-issue"><strong>{recentIssue.status === "interrupted" ? "Previous image pass was interrupted" : "Previous image pass did not finish"}</strong><span>{recentIssue.detail}</span>{recentIssue.error && <small>{recentIssue.error}</small>}</div>}
    {!ready.length && !generating && <div className="image-asset-empty"><ImageIcon /><span>Your generated candidates will stay in this private library across restarts. Only the image you choose is attached to the movie.</span></div>}
  </section>;
}

function MovieProjectView({ project, edit, busy, advancedEnabled, onError, onProject, onEdit, onNew, onCancel, onResume, onReveal, onSave, onExport }: {
  project: MovieProject; edit: MovieEdit; busy: boolean; advancedEnabled: boolean; onError: (message: string) => void;
  onProject: (project: MovieProject) => void; onEdit: (edit: MovieEdit) => void;
  onNew: () => void; onCancel: () => void; onResume: () => void; onReveal: () => void; onSave: () => void; onExport: () => void;
}) {
  const [draftPlan, setDraftPlan] = useState<MoviePlan | undefined>(project.plan);
  const [working, setWorking] = useState(false);
  useEffect(() => setDraftPlan(project.plan), [project.id, project.plan]);
  const complete = project.clips.filter((clip) => clip.status === "complete").length;
  const progress = project.clips.length ? Math.round((complete / project.clips.length) * 100) : project.plan ? 10 : 3;
  const canResume = project.status === "planning-checkpoint" || ["failed", "cancelled", "interrupted"].includes(project.status);
  const resumeLabel = project.plan && project.status !== "planning-checkpoint" ? "Resume production" : "Resume planning";
  const latestExport = project.exports?.at(-1);
  const runProjectAction = async (action: () => Promise<MovieProject>): Promise<boolean> => {
    setWorking(true);
    try {
      onProject(await action());
      return true;
    } catch (error) {
      onError(String(error));
      return false;
    } finally {
      setWorking(false);
    }
  };
  return <div className="movie-project-view">
    <header className="movie-project-header">
      <div><span className="eyebrow">{project.status === "complete" ? "Review cut ready" : project.phase}</span><h1>{project.title}</h1><p>{project.plan?.logline ?? project.prompt}</p></div>
      <div className="movie-project-actions"><button onClick={onNew}><Plus /> New</button><button onClick={onReveal}><FolderOpen /> Files</button>{project.status === "running" && <button className="danger" onClick={onCancel}><CircleStop /> Cancel production</button>}{canResume && <button className="accent" onClick={onResume}><RotateCcw /> {resumeLabel}</button>}</div>
    </header>
    <div className={`movie-status-card ${project.status}`}>
      <div>{project.status === "running" ? <LoaderCircle className="spin" /> : project.status === "complete" ? <Check /> : <Clock3 />}<span><strong>{project.detail}</strong><small>{complete} of {project.clips.length || "—"} H3 masters preserved · {project.renderer}</small></span></div>
      <div className="movie-progress"><i style={{ width: `${progress}%` }} /></div>
      {project.error && <pre>{project.error}</pre>}
    </div>
    {(project.status === "planning-checkpoint" || (project.status === "running" && ["writing", "agent-workspace", "resuming", "producer-revision"].includes(project.phase))) && <ProducerPlanningRoom project={project} advancedEnabled={advancedEnabled} onError={onError} />}
    {project.finalPath && <section className="movie-final"><div className="movie-section-heading"><div><span className="eyebrow">{latestExport ? "Latest immutable timeline export" : "Assembled file"}</span><h2>{latestExport?.title ?? "Untouched H3 review cut"}</h2><small>{latestExport ? `${latestExport.preset} preset · ${latestExport.clipCount} timeline items · SHA-256 recorded` : "Native clip duration and audio are preserved. Only an explicit editor export creates an altered cut."}</small></div><a href={movieMediaUrl(project.finalPath)} download><Download /> Open file</a></div><video controls preload="metadata" src={movieMediaUrl(project.finalPath)} /></section>}
    {project.references.length > 0 && <section className="movie-project-references"><div className="movie-section-heading"><div><span className="eyebrow">Native H3 inputs</span><h2>Producer references</h2></div><small>Immutable copies preserved with this production</small></div><div>{project.references.map((reference) => <article key={reference.assetId}><ReferencePreview reference={reference} /><span><strong>{reference.tag}{reference.audioTag ? ` + ${reference.audioTag}` : ""} · {reference.name}</strong><small>{reference.description}</small>{reference.audioTag && <small>{reference.audioTag}: {reference.embeddedAudioDescription}</small>}{reference.generation && <details><summary>Generated-image provenance</summary><small>Frame {reference.generation.frameIndex} · seed {reference.generation.seed} · {reference.generation.steps} steps · {reference.generation.width} × {reference.generation.height}</small><pre>{reference.generation.renderedPrompt}</pre><pre>{JSON.stringify(reference.generation.exactGraph, null, 2)}</pre></details>}</span></article>)}</div></section>}
    {project.status === "awaiting-review" && draftPlan && <ProducerPlanDesk project={project} plan={draftPlan} busy={working} onPlan={setDraftPlan}
      onSave={() => void runProjectAction(() => saveMoviePlan(project.id, draftPlan))}
      onRevise={(feedback) => runProjectAction(async () => {
        await saveMoviePlan(project.id, draftPlan);
        return reviseMoviePlan(project.id, feedback);
      })}
      onApprove={() => void runProjectAction(async () => {
        await saveMoviePlan(project.id, draftPlan);
        return approveMoviePlan(project.id);
      })} />}
    {project.plan && project.status !== "awaiting-review" && <section className="movie-plan-overview"><article><span className="eyebrow">Creative direction</span><p>{project.plan.creativeDirection}</p></article><article><span className="eyebrow">Continuity bible</span><ul>{project.plan.continuityBible.map((rule) => <li key={rule}>{rule}</li>)}</ul></article><article><span className="eyebrow">Bonsai acceptance</span><p>{project.plan.qualityReview.score}/100 after {project.plan.qualityReview.attempts} {project.plan.qualityReview.attempts === 1 ? "attempt" : "attempts"}. {project.plan.qualityReview.verdict}</p></article></section>}
    {project.clips.length > 0 && <section className="movie-timeline-section">
      <div className="movie-section-heading"><div><span className="eyebrow">Non-destructive edit</span><h2>Timeline & program monitor</h2><small>Split, repeat, reorder, retime, fade, and audition any preserved scene version. Masters remain immutable.</small></div><div><button disabled={busy} onClick={onSave}><Save /> Save timeline</button><button className="accent" disabled={busy || complete === 0 || project.status === "running" || !edit.clips.some((item) => item.enabled)} onClick={onExport}>{busy ? <LoaderCircle className="spin" /> : <Play />} Export new cut</button></div></div>
      <MovieTimeline key={project.id} project={project} value={edit} disabled={busy || project.status === "running"} onChange={onEdit} />
      <details className="movie-master-bin"><summary><span><Film /> Preserved master bin</span><small>{project.clips.length} original scenes · open for story notes, prompts, and Bonsai versioning</small></summary>
        <div className="movie-clip-grid">{project.clips.map((clip) => {
          const planned = project.plan?.clips.find((item) => item.id === clip.id);
          return <article key={clip.id} className={`movie-clip ${clip.status}`}>
            <div className="clip-preview">{clip.path ? <video controls preload="metadata" src={movieMediaUrl(clip.path)} /> : <div><LoaderCircle className={clip.status === "rendering" ? "spin" : ""} /><span>{clip.status}</span></div>}<span className="clip-number">{clip.index + 1}</span></div>
            <div className="clip-copy"><div><span><strong>{clip.title}</strong><small>{clip.durationSeconds.toFixed(1)}s · seed {clip.seed}{clip.versions.length ? ` · ${clip.versions.length} preserved versions` : ""}</small></span></div>
              {planned && <div className="clip-organization"><span><b>Story job</b>{planned.purpose}</span><span><b>Transition</b>{planned.transition}</span><span><b>Continuity in</b>{planned.continuityIn}</span><span><b>Continuity out</b>{planned.continuityOut}</span>{planned.referenceIds.length > 0 && <span><b>References</b>{planned.referenceIds.map((id) => project.references.find((reference) => reference.assetId === id)?.name ?? id).join(", ")}</span>}</div>}
              <details><summary>H3 renderer direction</summary><p>{clip.prompt}</p></details>
              {advancedEnabled && clip.status === "complete" && planned && <SceneAssistant project={project} clip={clip} planned={planned} onProject={onProject} onError={onError} />}
            </div>{clip.error && <pre>{clip.error}</pre>}
          </article>;
        })}</div>
      </details>
    </section>}
    {project.exports?.length > 0 && <section className="movie-export-history"><div className="movie-section-heading"><div><span className="eyebrow">Immutable deliverables</span><h2>Export history</h2><small>Every cut remains addressable with its decision-list sidecar and SHA-256 identity.</small></div></div><div>{[...project.exports].reverse().map((item) => <article key={item.id}><span><strong>{item.title}</strong><small>{new Date(item.createdAt).toLocaleString()} · {item.preset} · {item.clipCount} items · {item.durationSeconds.toFixed(2)}s · {readableSize(item.bytes)}</small><code title={item.sha256}>{item.sha256.slice(0, 16)}…</code></span><a href={movieMediaUrl(item.path)} download><Download /> Open</a></article>)}</div></section>}
  </div>;
}

function ProducerPlanningRoom({ project, advancedEnabled, onError }: {
  project: MovieProject;
  advancedEnabled: boolean;
  onError: (message: string) => void;
}) {
  const [snapshot, setSnapshot] = useState<MoviePlanningSnapshot>();
  const [currentText, setCurrentText] = useState("");
  const [advancedStream, setAdvancedStream] = useState("");
  const [activities, setActivities] = useState<MoviePlanningEvent[]>([]);
  const [direction, setDirection] = useState("");
  const [sending, setSending] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const planning = project.status === "running";

  const refresh = useCallback(async () => {
    try {
      const next = await getMoviePlanning(project.id);
      setSnapshot(next);
      setCurrentText((value) => value || next.currentText);
    } catch (error) {
      onError(String(error));
    }
  }, [onError, project.id]);

  useEffect(() => {
    void refresh();
    let dispose: (() => void) | undefined;
    let refreshTimer: number | undefined;
    void onMoviePlanning((event) => {
      if (event.projectId !== project.id) return;
      if (event.kind === "turn-start") {
        setCurrentText("");
        setAdvancedStream("");
      } else if (event.kind === "token") {
        setCurrentText((value) => value + event.text);
      } else if (event.kind === "advanced-token") {
        setAdvancedStream((value) => (value + event.text).slice(-120_000));
      } else {
        setActivities((value) => [...value.slice(-11), event]);
      }
      if (["turn-complete", "tool-result", "direction-queued", "checkpoint-saved"].includes(event.kind)) {
        if (refreshTimer) window.clearTimeout(refreshTimer);
        refreshTimer = window.setTimeout(() => void refresh(), 350);
      }
    }).then((unlisten) => { dispose = unlisten; });
    return () => {
      dispose?.();
      if (refreshTimer) window.clearTimeout(refreshTimer);
    };
  }, [project.id, refresh]);

  const sendDirection = async () => {
    if (direction.trim().length < 3) return;
    setSending(true);
    try {
      setSnapshot(await directMoviePlanning(project.id, direction));
      setDirection("");
    } catch (error) {
      onError(String(error));
    } finally {
      setSending(false);
    }
  };

  const checkpoint = async () => {
    setSending(true);
    try {
      setSnapshot(await checkpointMoviePlanning(project.id));
    } catch (error) {
      onError(String(error));
    } finally {
      setSending(false);
    }
  };

  return <section className="producer-planning-room">
    <div className="movie-section-heading"><div><span className="eyebrow">Live planning room</span><h2>Direct Bonsai while it works</h2><small>Directions enter the durable workspace at the next safe model-turn boundary. Nothing is sent to the public network.</small></div><span className={`planning-room-state ${planning ? "live" : "saved"}`}>{planning ? <LoaderCircle className="spin" /> : <ShieldCheck />}{planning ? "Planning live" : "Checkpoint saved"}</span></div>
    <div className="planning-room-grid">
      <article className="planning-current-copy">
        <header><strong>What Bonsai is saying now</strong><small>Streamed as the local model produces it</small></header>
        <div className="planning-stream-text">{currentText.trim() || (planning ? "Bonsai is preparing its next structured production action…" : "No unfinished model text. The durable workspace is ready to resume.")}</div>
        <div className="planning-activity-feed">{activities.length ? activities.map((event) => <div key={`${event.sequence}-${event.kind}`}><span>{event.kind === "reasoning" ? <Sparkles /> : event.kind.includes("checkpoint") ? <ShieldCheck /> : <Check />}</span><p><b>{friendlyPlanningStage(event.stage)}</b>{event.text}</p></div>) : <small>Production actions will appear here as Bonsai reads, edits, and checks scenes.</small>}</div>
      </article>
      <article className="planning-direction-card">
        <header><strong>Change direction</strong><small>Write naturally—no JSON, prompts, or code required</small></header>
        <textarea value={direction} disabled={!planning || sending} onChange={(event) => setDirection(event.target.value)} placeholder="Example: Make the opening warmer and more intimate. Keep the train-station ending, but reveal the red suitcase two scenes earlier." />
        {snapshot?.pendingDirections.length ? <small>{snapshot.pendingDirections.length} direction{snapshot.pendingDirections.length === 1 ? "" : "s"} queued for the next safe turn.</small> : <small>Bonsai preserves compatible work and revises only affected scenes.</small>}
        <div><button className="accent" disabled={!planning || sending || direction.trim().length < 3} onClick={() => void sendDirection()}>{sending ? <LoaderCircle className="spin" /> : <Send />} Send direction</button><button disabled={!planning || sending || snapshot?.checkpointRequested} onClick={() => void checkpoint()}><ShieldCheck /> {snapshot?.checkpointRequested ? "Checkpoint queued" : "Save checkpoint"}</button></div>
        <p><b>Checkpoint, don’t cancel</b> waits for the current model/tool turn to finish, then preserves the exact transcript, producer notes, screenplay, and scene files. “Cancel production” remains available for an immediate stop.</p>
      </article>
    </div>
    {advancedEnabled && <div className="planning-advanced">
      <button onClick={() => setShowAdvanced((value) => !value)}><Settings2 /> {showAdvanced ? "Hide" : "Inspect"} exact model context <ChevronDown className={showAdvanced ? "open" : ""} /></button>
      {showAdvanced && <div className="planning-advanced-content">
        <p>These are the exact sanitized messages, tool definition, workspace contract, lint policy, brief, references, and live tool-call arguments available to Bonsai. Private reasoning tokens are intentionally not presented as producer text.</p>
        {advancedStream && <details open><summary>Current streamed tool-call arguments</summary><pre>{advancedStream}</pre></details>}
        {snapshot?.promptDocuments.map((document) => <details key={document.id}><summary>{document.title} <small>{document.category}</small></summary><pre>{document.content}</pre></details>)}
        <details><summary>movie_workspace tool schema</summary><pre>{JSON.stringify(snapshot?.toolSchema ?? {}, null, 2)}</pre></details>
        <details><summary>Exact last request envelope sent to Bonsai</summary><pre>{JSON.stringify(snapshot?.lastRequest ?? {}, null, 2)}</pre></details>
        <details><summary>Exact accepted model transcript</summary><pre>{JSON.stringify(snapshot?.transcript ?? {}, null, 2)}</pre></details>
        <button onClick={() => void refresh()}><RotateCcw /> Refresh exact context</button>
      </div>}
    </div>}
  </section>;
}

function friendlyPlanningStage(stage: string): string {
  const names: Record<string, string> = {
    planning: "Model turn",
    thinking: "Local reasoning",
    producer: "Producer control",
    "native-check": "Production check",
    checkpoint: "Safe checkpoint",
    list: "Workspace review",
    read: "Scene review",
    read_many: "Scene review",
    write: "Scene edit",
    write_batch: "Scene edit",
    check: "Native checks",
    submit: "Plan submission",
  };
  return names[stage] ?? "Planning";
}

function ProducerPlanDesk({ project, plan, busy, onPlan, onSave, onRevise, onApprove }: {
  project: MovieProject; plan: MoviePlan; busy: boolean; onPlan: (plan: MoviePlan) => void;
  onSave: () => void; onRevise: (feedback: string) => Promise<boolean>; onApprove: () => void;
}) {
  const [feedback, setFeedback] = useState("");
  const updateClip = (index: number, clip: PlannedClip) => onPlan({ ...plan, clips: plan.clips.map((item, itemIndex) => itemIndex === index ? clip : item) });
  const moveClip = (index: number, direction: number) => {
    const target = index + direction;
    if (target < 0 || target >= plan.clips.length) return;
    const clips = [...plan.clips];
    [clips[index], clips[target]] = [clips[target], clips[index]];
    onPlan({ ...plan, clips });
  };
  const sendFeedback = async () => {
    if (feedback.trim().length < 3) return;
    if (await onRevise(feedback)) setFeedback("");
  };
  return <section className="producer-plan-desk">
    <div className="movie-section-heading"><div><span className="eyebrow">Producer checkpoint · no H3 render has started</span><h2>Organize the Bonsai plan</h2><small>Edit fields directly, or send the whole structured plan back to Bonsai with production notes.</small></div><div><button disabled={busy} onClick={onSave}><Save /> Save structured draft</button><button className="accent" disabled={busy || plan.clips.length === 0} onClick={onApprove}>{busy ? <LoaderCircle className="spin" /> : <Play />} Approve & render H3</button></div></div>
    <div className="producer-plan-basics">
      <label>Title<input value={plan.title} onChange={(event) => onPlan({ ...plan, title: event.target.value })} /></label>
      <label>Audience<input value={plan.audience} onChange={(event) => onPlan({ ...plan, audience: event.target.value })} /></label>
      <label className="wide">Logline<textarea value={plan.logline} onChange={(event) => onPlan({ ...plan, logline: event.target.value })} /></label>
      <label className="wide">Creative direction<textarea value={plan.creativeDirection} onChange={(event) => onPlan({ ...plan, creativeDirection: event.target.value })} /></label>
      <label className="wide">Continuity bible · one rule per line<textarea value={plan.continuityBible.join("\n")} onChange={(event) => onPlan({ ...plan, continuityBible: event.target.value.split("\n").map((item) => item.trim()).filter(Boolean) })} /></label>
    </div>
    <div className="producer-scene-list">{plan.clips.map((clip, index) => <article key={`${clip.id}-${index}`} className="producer-scene-card">
      <header><span><b>Scene {index + 1}</b><small>{clip.durationSeconds}s planned · {clip.usePreviousFrame ? "continuous frame handoff" : "independent visual start"}</small></span><div><button disabled={index === 0} onClick={() => moveClip(index, -1)}>Move up</button><button disabled={index === plan.clips.length - 1} onClick={() => moveClip(index, 1)}>Move down</button><button onClick={() => onPlan({ ...plan, clips: plan.clips.filter((_, itemIndex) => itemIndex !== index) })}>Remove</button></div></header>
      <PlannedClipFields clip={clip} references={project.references} onClip={(next) => updateClip(index, next)} />
    </article>)}</div>
    <button className="producer-add-scene" disabled={plan.clips.length >= project.settings.maxClips} onClick={() => onPlan({ ...plan, clips: [...plan.clips, emptyPlannedClip(plan.clips.length)] })}><Plus /> Add scene</button>
    <div className="producer-feedback"><label>Notes for Bonsai<textarea value={feedback} onChange={(event) => setFeedback(event.target.value)} placeholder="Keep the flashback isolated to scene 5; strengthen the visual bridge between scenes 2 and 3; rewrite scene 8's H3 direction with more precise camera and audio beats…" /></label><button disabled={busy || feedback.trim().length < 3} onClick={() => void sendFeedback()}>{busy ? <LoaderCircle className="spin" /> : <Sparkles />} Send full plan back to Bonsai</button></div>
  </section>;
}

function PlannedClipFields({ clip, references, onClip }: { clip: PlannedClip; references: MovieProject["references"]; onClip: (clip: PlannedClip) => void }) {
  const field = <K extends keyof PlannedClip>(name: K, value: PlannedClip[K]) => onClip({ ...clip, [name]: value });
  return <div className="planned-clip-fields">
    <label>Scene title<input value={clip.title} onChange={(event) => field("title", event.target.value)} /></label>
    <NumberField label="Planned H3 seconds" value={clip.durationSeconds} min={5} max={15} step={1} onChange={(value) => field("durationSeconds", value)} />
    <label className="wide">Story purpose<textarea value={clip.purpose} onChange={(event) => field("purpose", event.target.value)} /></label>
    <label>Transition<input value={clip.transition} onChange={(event) => field("transition", event.target.value)} /></label>
    <label>Continuity in<input value={clip.continuityIn} onChange={(event) => field("continuityIn", event.target.value)} /></label>
    <label>Continuity out<input value={clip.continuityOut} onChange={(event) => field("continuityOut", event.target.value)} /></label>
    <label className="previous-frame-toggle"><span><input type="checkbox" checked={clip.usePreviousFrame} onChange={(event) => field("usePreviousFrame", event.target.checked)} /> Continue from previous scene’s last frame</span></label>
    {references.length > 0 && <fieldset className="wide"><legend>Native references for this scene</legend>{references.map((reference) => <label key={reference.assetId}><input type="checkbox" checked={clip.referenceIds.includes(reference.assetId)} disabled={clip.usePreviousFrame && !clip.referenceIds.includes(reference.assetId)} onChange={(event) => field("referenceIds", event.target.checked ? [...clip.referenceIds, reference.assetId] : clip.referenceIds.filter((id) => id !== reference.assetId))} /><span>{reference.tag}{reference.audioTag ? ` + ${reference.audioTag}` : ""} · {reference.name}</span></label>)}</fieldset>}
    <label className="wide renderer-direction">H3 renderer direction<textarea value={clip.prompt} onChange={(event) => field("prompt", event.target.value)} /></label>
  </div>;
}

function SceneAssistant({ project, clip, planned: _planned, onProject, onError }: { project: MovieProject; clip: RenderedClip; planned: PlannedClip; onProject: (project: MovieProject) => void; onError: (message: string) => void }) {
  const [open, setOpen] = useState(false);
  const [feedback, setFeedback] = useState("");
  const [suggestion, setSuggestion] = useState<MovieClipSuggestion | null>(null);
  const [seed, setSeed] = useState(clip.seed + 1);
  const [busy, setBusy] = useState(false);
  const ask = async () => {
    setBusy(true);
    try { setSuggestion(await askBonsaiMovieClip(project.id, clip.id, feedback)); } catch (error) { onError(String(error)); } finally { setBusy(false); }
  };
  const renderVersion = async () => {
    if (!suggestion) return;
    setBusy(true);
    try { onProject(await renderMovieClipVersion({ id: project.id, suggestion, seed })); } catch (error) { onError(String(error)); } finally { setBusy(false); }
  };
  return <div className="scene-assistant"><button className="scene-assistant-toggle" onClick={() => setOpen(!open)}><Sparkles /> Bonsai scene assistant <ChevronDown className={open ? "open" : ""} /></button>{open && <div className="scene-assistant-body">
    <p>Give Bonsai a focused fix request. It receives this organized scene, its neighbors, continuity bible, and reference manifest—not an unstructured text dump.</p>
    <label>Producer fix request<textarea value={feedback} onChange={(event) => setFeedback(event.target.value)} placeholder="Preserve the performance and story beat, but make the camera blocking legible and specify the sound transition into the next scene…" /></label>
    <button disabled={busy || feedback.trim().length < 3} onClick={() => void ask()}>{busy ? <LoaderCircle className="spin" /> : <Sparkles />} Ask Bonsai for a structured fix</button>
    {suggestion && <div className="scene-suggestion"><h4>{suggestion.summary}</h4><ul>{suggestion.checklist.map((item) => <li key={item}>{item}</li>)}</ul><PlannedClipFields clip={suggestion.clip} references={project.references} onClip={(next) => setSuggestion({ ...suggestion, clip: { ...next, id: clip.id } })} /><div className="scene-version-action"><NumberField label="New version seed" value={seed} min={0} max={Number.MAX_SAFE_INTEGER} step={1} onChange={setSeed} /><span>The current master and assembled review cut remain preserved. This explicit action renders a separate H3 master.</span><button disabled={busy} onClick={() => void renderVersion()}>{busy ? <LoaderCircle className="spin" /> : <Video />} Render new scene version</button></div></div>}
  </div>}</div>;
}

function emptyPlannedClip(index: number): PlannedClip {
  return { id: `producer-scene-${Date.now()}-${index}`, title: `Scene ${index + 1}`, purpose: "", durationSeconds: 5, prompt: "", continuityIn: "", continuityOut: "", transition: "hard cut", usePreviousFrame: false, sourceRefs: [], referenceIds: [] };
}

function ReferencePreview({ reference }: { reference: { kind: string; path: string; name: string } }) {
  const source = movieMediaUrl(reference.path);
  if (reference.kind === "image") return <div className="movie-reference-preview"><img src={source} alt={reference.name} /></div>;
  if (reference.kind === "video") return <div className="movie-reference-preview"><video controls muted preload="metadata" src={source} /></div>;
  return <div className="movie-reference-preview audio"><AudioLines /><audio controls preload="metadata" src={source} /></div>;
}

function promptFieldMatches(left: PromptField | undefined, right: PromptField): boolean {
  if (!left || left.kind !== right.kind) return false;
  return left.kind !== "referenceDescription"
    || (right.kind === "referenceDescription" && left.assetId === right.assetId && left.part === right.part);
}

function referenceDraftKey(assetId: string, part: "description" | "embeddedAudioDescription"): string {
  return `${assetId}:${part}`;
}

export function referenceDisplayTags(references: PendingMovieReference[], id: string): string[] {
  const reference = references.find((item) => item.assetId === id);
  if (!reference) return [];
  if (reference.kind === "image") {
    return [`<Picture ${references.filter((item) => item.kind === "image").findIndex((item) => item.assetId === id) + 1}>`];
  }
  const embeddedVideos = references.filter((item) => item.kind === "video" && item.useEmbeddedAudio);
  if (reference.kind === "video") {
    const video = references.filter((item) => item.kind === "video").findIndex((item) => item.assetId === id) + 1;
    const labels = [`<Video ${video}>`];
    if (reference.useEmbeddedAudio) labels.push(`<Audio ${embeddedVideos.findIndex((item) => item.assetId === id) + 1}>`);
    return labels;
  }
  const standalone = references.filter((item) => item.kind === "audio").findIndex((item) => item.assetId === id) + 1;
  return [`<Audio ${embeddedVideos.length + standalone}>`];
}

function referencesReady(references: PendingMovieReference[]): boolean {
  const reserved = /<(picture|video|audio|subject)\b/i;
  return references.every((reference) => reference.description.trim().length >= 3
    && !reserved.test(reference.description)
    && (!reference.useEmbeddedAudio || (reference.embeddedAudioDescription.trim().length >= 3
      && !reserved.test(reference.embeddedAudioDescription))));
}

function NumberField({ label, value, min, max, step, onChange }: { label: string; value: number; min: number; max: number; step: number; onChange: (value: number) => void }) {
  return <label>{label}<input type="number" value={value} min={min} max={max} step={step} onChange={(event) => onChange(Number(event.target.value))} /></label>;
}

function SelectField({ label, value, options, onChange }: { label: string; value: string; options: string[]; onChange: (value: string) => void }) {
  return <label>{label}<select value={value} onChange={(event) => onChange(event.target.value)}>{options.map((option) => <option key={option}>{option}</option>)}</select></label>;
}

function readableSize(bytes: number): string {
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KiB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
}
