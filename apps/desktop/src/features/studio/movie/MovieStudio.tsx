import {
  ArrowDown, ArrowUp, AudioLines, Check, ChevronDown, CircleStop, Clapperboard,
  Download, FilePenLine, Film, FolderOpen, Library, LoaderCircle, MessageSquare,
  Paperclip, Play, Plus, RotateCcw, Save, Send, Settings2, ShieldCheck, Sparkles,
  Trash2, Video, X,
} from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  acceptMovieStoryRevision, attachMovieProducerReferences, cancelMovieRender,
  cancelMovieStudioChat, createMovieProducerProject, getMovie,
  getMovieProducerWorkspace, getMovieStudioConversation, listMovieImageAssets,
  listMovies, movieMediaUrl, onMovieProducerWorkspace, onMovieProject,
  onMovieStudioChat, pickMovieReferenceFiles, renderMovieEdit, renderMovieScenes,
  resetMovieStudioConversation, revealMovie, saveMovieEdits, saveMovieScenes,
  saveMovieStoryRevision, startMovieStudioChat, summarizeMovieStudioConversation,
} from "../../../platform/api";
import { MarkdownContent } from "../../../shared/components/MarkdownContent";
import { appendModelThinking, ModelThinkingStream } from "../../control/ModelThinkingStream";
import { MovieTimeline } from "./MovieTimeline";
import type {
  ControlSettings, ModelInfo, MovieEdit, MovieProducerWorkspace, MovieReference,
  MovieReferenceAsset, MovieSceneDraft, MovieSceneFrameSource,
  MovieStudioConversation, MovieStudioConversationKind, MovieStudioConversationMode,
  MovieSummary, PendingMovieReference, ThinkingLevel,
} from "../../../contracts/index";

type ProjectWorkspace = "story" | "scenes" | "edit" | "deliver";
type ChatState = { requestId?: string; kind?: MovieStudioConversationKind; text: string; reasoning: string; status: string };

const defaultSettings = {
  width: 1344, height: 768, clipSeconds: 5, steps: 20, maxClips: 12, seed: 0,
  temperature: 0.45, topP: 0.9, topK: 20, thinkingBudget: 32768,
  maxOutputTokens: 32768, comfyRoot: "", refImageSize: "match" as const,
};
const emptyChat: ChatState = { text: "", reasoning: "", status: "" };

function requestId(prefix: string): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? `${prefix}-${crypto.randomUUID()}`
    : `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function emptyScene(storyRevisionId: string, durationSeconds: number, index: number): MovieSceneDraft {
  const now = new Date().toISOString();
  return {
    id: requestId("scene"), revision: 0, title: `Scene ${index + 1}`,
    purpose: "", durationSeconds, h3Prompt: "", continuityIn: "",
    continuityOut: "", transition: "Cut", references: [], storyRevisionId,
    createdAt: now, updatedAt: now,
  };
}

function useStableCallback<T extends (...args: never[]) => unknown>(callback: T): T {
  const callbackRef = useRef(callback);
  useEffect(() => { callbackRef.current = callback; }, [callback]);
  return useCallback(((...args: Parameters<T>) => callbackRef.current(...args)) as T, []);
}

export function MovieStudio({
  initialComfyRoot, advancedEnabled, models = [], selectedModelId, onError: onErrorProp,
}: {
  initialComfyRoot?: string; advancedEnabled: boolean; models?: ModelInfo[];
  selectedModelId?: string; controlSettings?: ControlSettings; onError: (message: string) => void;
}) {
  const onError = useStableCallback(onErrorProp);
  const [movies, setMovies] = useState<MovieSummary[]>([]);
  const [project, setProject] = useState<Awaited<ReturnType<typeof getMovie>> | null>(null);
  const [producer, setProducer] = useState<MovieProducerWorkspace | null>(null);
  const [creating, setCreating] = useState(true);
  const [startingMaterial, setStartingMaterial] = useState("");
  const [modelId, setModelId] = useState(selectedModelId ?? models[0]?.id ?? "");
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel | "default">("default");
  const [settings, setSettings] = useState(() => ({ ...defaultSettings, comfyRoot: initialComfyRoot || "" }));
  const [advanced, setAdvanced] = useState(false);
  const [pendingReferences, setPendingReferences] = useState<PendingMovieReference[]>([]);
  const [workspace, setWorkspace] = useState<ProjectWorkspace>("story");
  const [conversation, setConversation] = useState<MovieStudioConversation | null>(null);
  const [storyDraft, setStoryDraft] = useState("");
  const [storyRevisionId, setStoryRevisionId] = useState("");
  const [storyEditing, setStoryEditing] = useState(false);
  const [scenes, setScenes] = useState<MovieSceneDraft[]>([]);
  const [selectedSceneIds, setSelectedSceneIds] = useState<string[]>([]);
  const [chat, setChat] = useState<ChatState>(emptyChat);
  const [chatInstruction, setChatInstruction] = useState("");
  const [busy, setBusy] = useState(false);
  const [edit, setEdit] = useState<MovieEdit>({ clips: [], exportTitle: "Kestrel Movie", exportPreset: "publish", normalizeAudio: false, targetLufs: -14, markers: [] });
  const [generatedImages, setGeneratedImages] = useState<MovieReferenceAsset[]>([]);
  const activeProjectId = useRef("");
  const activeChatRequest = useRef("");
  const activeChatKind = useRef<MovieStudioConversationKind>("story");
  const modelIds = models.map((model) => model.id).join("\u0000");

  useEffect(() => {
    if (models.some((model) => model.id === modelId)) return;
    setModelId(models.find((model) => model.id === selectedModelId)?.id ?? models[0]?.id ?? "");
  }, [modelId, modelIds, models, selectedModelId]);

  const refreshList = useCallback(async () => setMovies(await listMovies()), []);
  const loadConversation = useCallback(async (projectId: string, next: MovieProducerWorkspace, kind: MovieStudioConversationKind) => {
    const id = kind === "story" ? next.activeStoryConversationId : next.activeSceneConversationId;
    setConversation(id ? await getMovieStudioConversation(projectId, id) : null);
  }, []);
  const applyProducer = useCallback((next: MovieProducerWorkspace, preserveSceneDraft = false) => {
    setProducer(next);
    if (!preserveSceneDraft) setScenes(next.scenes);
    const active = next.storyRevisions.find((item) => item.id === next.activeStoryRevisionId) ?? next.storyRevisions.at(-1);
    setStoryRevisionId(active?.id ?? "");
    setStoryDraft(active?.markdown ?? "");
  }, []);

  const openProject = useCallback(async (id: string) => {
    setBusy(true);
    try {
      const [nextProject, nextProducer] = await Promise.all([getMovie(id), getMovieProducerWorkspace(id)]);
      activeProjectId.current = id;
      setProject(nextProject); setEdit(nextProject.edit); applyProducer(nextProducer); setCreating(false);
      const nextWorkspace: ProjectWorkspace = nextProducer.acceptedStoryRevisionId ? "scenes" : "story";
      setWorkspace(nextWorkspace);
      await loadConversation(id, nextProducer, nextWorkspace);
    } catch (error) { onError(String(error)); } finally { setBusy(false); }
  }, [applyProducer, loadConversation, onError]);

  useEffect(() => {
    let alive = true;
    void listMovies().then((items) => { if (alive) setMovies(items); }).catch((error) => onError(String(error)));
    void listMovieImageAssets().then((items) => {
      if (alive) setGeneratedImages(items.flatMap((item) => item.status === "complete" ? item.candidates.map((candidate) => candidate.asset) : []));
    }).catch(() => undefined);
    return () => { alive = false; };
  }, [onError]);

  useEffect(() => {
    let disposeProject: (() => void) | undefined;
    let disposeProducer: (() => void) | undefined;
    let disposeChat: (() => void) | undefined;
    void onMovieProject((next) => {
      if (next.id !== activeProjectId.current) return;
      setProject(next);
      if (next.status !== "running") setEdit(next.edit);
      void refreshList().catch(() => undefined);
    }).then((dispose) => { disposeProject = dispose; });
    void onMovieProducerWorkspace((next) => {
      if (next.projectId === activeProjectId.current) applyProducer(next, Boolean(activeChatRequest.current && activeChatKind.current === "scenes"));
    }).then((dispose) => { disposeProducer = dispose; });
    void onMovieStudioChat((event) => {
      if (event.requestId !== activeChatRequest.current || event.projectId !== activeProjectId.current) return;
      if (event.event === "queued") setChat((value) => ({ ...value, status: `Loading ${event.modelName ?? "local collaborator"}…` }));
      else if (event.event === "started") setChat((value) => ({ ...value, status: "Writing locally…" }));
      else if (event.event === "token" && event.content) setChat((value) => ({ ...value, text: value.text + event.content }));
      else if (event.event === "reasoning" && event.content) setChat((value) => ({ ...value, reasoning: appendModelThinking(value.reasoning, event.content ?? "") }));
      else if (event.event === "cancelled") setChat((value) => ({ ...value, status: "Stopped. Partial text was preserved but not applied." }));
      else if (event.event === "error") {
        setChat((value) => ({ ...value, status: event.content || "The local collaborator stopped with an error." }));
        onError(event.content || "Movie Studio collaborator failed.");
      } else if (event.event === "complete") setChat((value) => ({ ...value, status: event.kind === "story" ? "A new story revision is ready." : "Scene changes are saved." }));
      else if (event.event === "settled") {
        const kind = event.kind;
        const projectId = event.projectId;
        activeChatRequest.current = "";
        setChat((value) => ({ ...value, requestId: undefined }));
        void Promise.all([getMovie(projectId), getMovieProducerWorkspace(projectId)]).then(async ([nextProject, nextProducer]) => {
          setProject(nextProject); setEdit(nextProject.edit); applyProducer(nextProducer);
          await loadConversation(projectId, nextProducer, kind); await refreshList();
        }).catch((error) => onError(String(error)));
      }
    }).then((dispose) => { disposeChat = dispose; });
    return () => { disposeProject?.(); disposeProducer?.(); disposeChat?.(); };
  }, [applyProducer, loadConversation, onError, refreshList]);

  const run = async <T,>(action: () => Promise<T>, apply?: (value: T) => void) => {
    setBusy(true);
    try { const result = await action(); apply?.(result); return result; }
    catch (error) { onError(String(error)); return undefined; }
    finally { setBusy(false); }
  };

  const attachReferences = async (toProject = false) => {
    setBusy(true);
    try {
      const imported = await pickMovieReferenceFiles();
      if (imported.failures.length) onError(imported.failures.join("\n"));
      const known = new Set(toProject ? project?.references.map((item) => item.assetId) : pendingReferences.map((item) => item.assetId));
      const added = imported.references.filter((item) => !known.has(item.id)).map(toPendingReference);
      if (!added.length) return;
      if (toProject && project) {
        const next = await attachMovieProducerReferences({
          projectId: project.id,
          references: added.map((item) => ({ assetId: item.assetId, description: item.description, includeEmbeddedAudio: item.useEmbeddedAudio, embeddedAudioDescription: item.embeddedAudioDescription })),
        });
        setProject(next);
      } else setPendingReferences((current) => [...current, ...added]);
    } catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const useGeneratedImage = async (asset: MovieReferenceAsset, toProject = false) => {
    const pending = toPendingReference(asset);
    if (toProject && project) {
      await run(() => attachMovieProducerReferences({ projectId: project.id, references: [{ assetId: pending.assetId, description: pending.description, includeEmbeddedAudio: false, embeddedAudioDescription: "" }] }), setProject);
    } else if (!pendingReferences.some((item) => item.assetId === pending.assetId)) setPendingReferences((current) => [...current, pending]);
  };

  const beginNew = () => {
    if (chat.requestId) void cancelMovieStudioChat(chat.requestId).catch(() => undefined);
    activeProjectId.current = "";
    activeChatRequest.current = "";
    setProject(null); setProducer(null); setConversation(null); setCreating(true);
    setStartingMaterial(""); setPendingReferences([]); setWorkspace("story");
    setStoryDraft(""); setStoryRevisionId(""); setScenes([]); setSelectedSceneIds([]); setChat(emptyChat);
  };

  const sendChat = async (kind: MovieStudioConversationKind, instruction = chatInstruction, projectId = project?.id, currentProducer = producer) => {
    if (!projectId || !currentProducer || !modelId || instruction.trim().length < 2 || activeChatRequest.current) return;
    const id = requestId("studio-chat");
    const conversationId = kind === "story" ? currentProducer.activeStoryConversationId : currentProducer.activeSceneConversationId;
    activeChatRequest.current = id;
    activeChatKind.current = kind;
    setChat({ requestId: id, kind, text: "", reasoning: "", status: "Preparing the local collaborator…" });
    setChatInstruction("");
    try {
      await startMovieStudioChat({
        requestId: id, projectId, kind, mode: "continue", conversationId, modelId, instruction,
        storyRevisionId: kind === "story" ? storyRevisionId || undefined : currentProducer.acceptedStoryRevisionId,
        selectedSceneIds: kind === "scenes" ? selectedSceneIds : [],
        thinkingLevel: thinkingLevel === "default" ? undefined : thinkingLevel,
      });
    } catch (error) { activeChatRequest.current = ""; setChat(emptyChat); onError(String(error)); }
  };

  const createProject = async () => {
    if (!modelId || startingMaterial.trim().length < 3) return;
    setBusy(true);
    try {
      const nextProject = await createMovieProducerProject({
        startingMaterial, collaboratorModelId: modelId,
        thinkingLevel: thinkingLevel === "default" ? undefined : thinkingLevel,
        settings: {
          width: settings.width, height: settings.height, clipSeconds: settings.clipSeconds,
          steps: settings.steps, maxClips: settings.maxClips, seed: settings.seed,
          thinkingBudget: settings.thinkingBudget, maxOutputTokens: settings.maxOutputTokens,
          comfyRoot: settings.comfyRoot, refImageSize: settings.refImageSize,
        },
        references: pendingReferences.map((item) => ({ assetId: item.assetId, description: item.description, includeEmbeddedAudio: item.useEmbeddedAudio, embeddedAudioDescription: item.embeddedAudioDescription })),
      });
      const nextProducer = await getMovieProducerWorkspace(nextProject.id);
      activeProjectId.current = nextProject.id;
      setProject(nextProject); setEdit(nextProject.edit); applyProducer(nextProducer); setCreating(false); setWorkspace("story");
      await loadConversation(nextProject.id, nextProducer, "story"); await refreshList();
      await sendChat("story", "Write the first complete story sketch from my starting material. Make confident creative choices while preserving its intent.", nextProject.id, nextProducer);
    } catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const showWorkspace = async (next: ProjectWorkspace) => {
    setWorkspace(next);
    if (project && producer && (next === "story" || next === "scenes")) await loadConversation(project.id, producer, next).catch((error) => onError(String(error)));
  };
  const chooseStoryRevision = (id: string) => {
    const revision = producer?.storyRevisions.find((item) => item.id === id);
    if (revision) { setStoryRevisionId(id); setStoryDraft(revision.markdown); setStoryEditing(false); }
  };
  const saveStory = async () => {
    if (!project || storyDraft.trim().length < 3) return;
    const next = await run(() => saveMovieStoryRevision({ projectId: project.id, parentRevisionId: storyRevisionId || undefined, instruction: "Direct producer edit", markdown: storyDraft }));
    if (next) { applyProducer(next); setStoryEditing(false); }
  };
  const acceptStory = async (mode: MovieStudioConversationMode) => {
    if (!project || !storyRevisionId) return;
    const next = await run(() => acceptMovieStoryRevision({ projectId: project.id, revisionId: storyRevisionId, conversationMode: mode }));
    if (next) { applyProducer(next); setWorkspace("scenes"); await loadConversation(project.id, next, "scenes").catch((error) => onError(String(error))); }
  };
  const saveScenes = async () => {
    if (!project || !producer) return;
    const next = await run(() => saveMovieScenes({ projectId: project.id, expectedRevision: producer.sceneRevision, scenes }));
    if (next) applyProducer(next);
  };
  const startSceneChat = async () => {
    if (!project || !producer) return;
    const saved = await run(() => saveMovieScenes({ projectId: project.id, expectedRevision: producer.sceneRevision, scenes }));
    if (saved) { applyProducer(saved); await sendChat("scenes", chatInstruction, project.id, saved); }
  };
  const resetConversation = async (keepSummary: boolean) => {
    if (!project || !conversation) return;
    const next = await run(() => resetMovieStudioConversation({ projectId: project.id, conversationId: conversation.id, keepSummary }));
    if (next) { setConversation(next); applyProducer(await getMovieProducerWorkspace(project.id)); }
  };
  const summarizeConversation = async () => {
    if (!project || !conversation || !modelId) return;
    const next = await run(() => summarizeMovieStudioConversation({
      projectId: project.id, conversationId: conversation.id, modelId,
      thinkingLevel: thinkingLevel === "default" ? undefined : thinkingLevel,
    }));
    if (next) setConversation(next);
  };
  const renderScenes = async () => {
    if (!project || !producer) return;
    const saved = await run(() => saveMovieScenes({ projectId: project.id, expectedRevision: producer.sceneRevision, scenes }));
    if (!saved) return;
    applyProducer(saved);
    await run(() => renderMovieScenes(project.id), (next) => { setProject(next); setEdit(next.edit); });
  };
  const saveEdits = async (exportNow: boolean) => {
    if (!project) return;
    await run(async () => { let next = await saveMovieEdits(project.id, edit); if (exportNow) next = await renderMovieEdit(project.id); return next; }, (next) => { setProject(next); setEdit(next.edit); });
  };

  const activeStory = producer?.storyRevisions.find((item) => item.id === storyRevisionId);
  const storyDirty = Boolean(activeStory && activeStory.markdown !== storyDraft);
  const sceneDirty = Boolean(producer && JSON.stringify(scenes) !== JSON.stringify(producer.scenes));
  const complete = project?.clips.filter((clip) => clip.status === "complete").length ?? 0;

  return <div className="movie-studio producer-movie-studio">
    <aside className="movie-library"><div className="movie-library-title"><span>Private movie library</span><button aria-label="New production" onClick={beginNew}><Plus size={15} /></button></div><div className="movie-list">
      {movies.map((movie) => <button key={movie.id} className={project?.id === movie.id ? "active" : ""} onClick={() => void openProject(movie.id)}><Film size={15} /><span><strong>{movie.title}</strong><small>{movie.phase} · {movie.clipCount} scenes</small></span></button>)}
      {!movies.length && <div className="movie-empty-list"><Library size={18} />Your durable productions will appear here.</div>}
    </div></aside>
    <section className="movie-stage">
      {creating || !project || !producer ? <ProducerLaunch material={startingMaterial} onMaterial={setStartingMaterial} modelId={modelId} onModel={setModelId} models={models} thinkingLevel={thinkingLevel} onThinkingLevel={setThinkingLevel} settings={settings} onSettings={setSettings} advanced={advanced} onAdvanced={setAdvanced} advancedEnabled={advancedEnabled} references={pendingReferences} onReferences={setPendingReferences} generatedImages={generatedImages} busy={busy || Boolean(chat.requestId)} onAttach={() => void attachReferences(false)} onUseGenerated={(asset) => void useGeneratedImage(asset, false)} onCreate={() => void createProject()} /> : <div className="movie-project-view movie-production-shell producer-project-shell">
        <header className="studio-project-bar"><div><span className={`studio-project-state ${project.status}`}>{project.status === "running" ? <LoaderCircle className="spin" /> : <ShieldCheck />}{project.phase}</span><span><strong>{project.title}</strong><small>Producer-owned story, scenes, media choices, and H3 masters</small></span></div><div className="movie-project-actions"><button onClick={beginNew}><Plus /> New</button><button onClick={() => void revealMovie(project.id)}><FolderOpen /> Files</button>{project.status === "running" && <button className="danger" onClick={() => void cancelMovieRender(project.id).catch((error) => onError(String(error)))}><CircleStop /> Stop H3 safely</button>}</div></header>
        <div className={`studio-production-strip ${project.status}`}><span>{project.status === "running" ? <LoaderCircle className="spin" /> : <ShieldCheck />}<strong>{project.detail}</strong><small>{producer.storyRevisions.length} story revisions · {producer.scenes.length} scene cards · {complete} H3 masters</small></span><div className="movie-progress"><i style={{ width: `${project.clips.length ? complete / project.clips.length * 100 : 0}%` }} /></div>{project.error && <button title={project.error} onClick={() => onError(project.error)}>Production issue</button>}</div>
        <nav className="studio-workspace-tabs project-tabs" aria-label="Production workspaces">
          <button className={workspace === "story" ? "active" : ""} onClick={() => void showWorkspace("story")}><FilePenLine /><span><strong>Story</strong><small>Draft and revise</small></span>{producer.activeStoryRevisionId && <Check />}</button>
          <button className={workspace === "scenes" ? "active" : ""} disabled={!producer.acceptedStoryRevisionId} onClick={() => void showWorkspace("scenes")}><Video /><span><strong>Scenes</strong><small>Cards and H3 bindings</small></span>{producer.scenes.length > 0 && <b>{producer.scenes.length}</b>}</button>
          <button className={workspace === "edit" ? "active" : ""} disabled={!project.clips.length} onClick={() => void showWorkspace("edit")}><Film /><span><strong>Edit</strong><small>Timeline and native mix</small></span>{edit.clips.length > 0 && <b>{edit.clips.filter((item) => item.enabled).length}</b>}</button>
          <button className={workspace === "deliver" ? "active" : ""} disabled={!project.clips.length} onClick={() => void showWorkspace("deliver")}><Download /><span><strong>Deliver</strong><small>Immutable exports</small></span>{project.exports.length > 0 && <b>{project.exports.length}</b>}</button>
        </nav>
        <div className={`studio-workspace-body producer-workspace producer-${workspace}`}>
          {workspace === "story" && <StoryRoom producer={producer} revisionId={storyRevisionId} draft={storyDraft} editing={storyEditing} dirty={storyDirty} acceptedId={producer.acceptedStoryRevisionId} conversation={conversation} chat={chat} instruction={chatInstruction} modelId={modelId} models={models} disabled={busy} onRevision={chooseStoryRevision} onDraft={setStoryDraft} onEditing={setStoryEditing} onInstruction={setChatInstruction} onSend={() => void sendChat("story")} onStop={() => chat.requestId && void cancelMovieStudioChat(chat.requestId)} onSave={() => void saveStory()} onAccept={(mode) => void acceptStory(mode)} onSummarize={() => void summarizeConversation()} onReset={(keep) => void resetConversation(keep)} />}
          {workspace === "scenes" && <SceneRoom project={project} producer={producer} scenes={scenes} selectedIds={selectedSceneIds} dirty={sceneDirty} conversation={conversation} chat={chat} instruction={chatInstruction} modelId={modelId} models={models} disabled={busy || project.status === "running"} generatedImages={generatedImages} onScenes={setScenes} onSelected={setSelectedSceneIds} onInstruction={setChatInstruction} onAttach={() => void attachReferences(true)} onUseGenerated={(asset) => void useGeneratedImage(asset, true)} onSave={() => void saveScenes()} onSend={() => void startSceneChat()} onStop={() => chat.requestId && void cancelMovieStudioChat(chat.requestId)} onSummarize={() => void summarizeConversation()} onReset={(keep) => void resetConversation(keep)} onRender={() => void renderScenes()} />}
          {workspace === "edit" && project.clips.length > 0 && <section className="project-edit-room"><MovieTimeline key={project.id} project={project} value={edit} disabled={busy || project.status === "running"} onChange={setEdit} onRequestSave={() => void saveEdits(false)} /></section>}
          {workspace === "deliver" && <DeliveryRoom project={project} edit={edit} busy={busy} complete={complete} onExport={() => void saveEdits(true)} />}
        </div>
      </div>}
    </section>
  </div>;
}

function ProducerLaunch({ material, onMaterial, modelId, onModel, models, thinkingLevel, onThinkingLevel, settings, onSettings, advanced, onAdvanced, advancedEnabled, references, onReferences, generatedImages, busy, onAttach, onUseGenerated, onCreate }: {
  material: string; onMaterial: (value: string) => void; modelId: string; onModel: (value: string) => void;
  models: ModelInfo[]; thinkingLevel: ThinkingLevel | "default"; onThinkingLevel: (value: ThinkingLevel | "default") => void;
  settings: typeof defaultSettings; onSettings: (value: typeof defaultSettings) => void; advanced: boolean; onAdvanced: (value: boolean) => void; advancedEnabled: boolean;
  references: PendingMovieReference[]; onReferences: (value: PendingMovieReference[]) => void; generatedImages: MovieReferenceAsset[];
  busy: boolean; onAttach: () => void; onUseGenerated: (asset: MovieReferenceAsset) => void; onCreate: () => void;
}) {
  return <div className="movie-launch producer-launch"><div className="movie-launch-mark"><Clapperboard /></div><span className="eyebrow">Producer-led · private local collaborator · MiniMax H3</span><h1>Start with the movie, not a workflow.</h1><p>Paste an idea, loose notes, a story, or a script. Your local model writes one complete creative sketch. You revise it together, accept it, then shape scene cards and choose every media reference yourself.</p>
    <div className="movie-prompt-box"><textarea aria-label="Starting material" maxLength={262144} value={material} onChange={(event) => onMaterial(event.target.value)} placeholder="A woman keeps receiving postcards from a city that disappeared…" /><div><span><ShieldCheck /> Tool-free, offline story collaboration</span><button disabled={busy || !modelId || material.trim().length < 3} onClick={onCreate}>{busy ? <LoaderCircle className="spin" /> : <Sparkles />} Create story sketch</button></div></div>
    <section className="producer-launch-controls"><label>Local collaborator<select aria-label="Story collaborator" value={modelId} onChange={(event) => onModel(event.target.value)}><option value="">Choose a local model</option>{models.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}</select></label><label>Thinking<select aria-label="Story thinking" value={thinkingLevel} onChange={(event) => onThinkingLevel(event.target.value as ThinkingLevel | "default")}><option value="default">Use model default</option><option value="off">Direct</option><option value="low">Low</option><option value="medium">Medium</option><option value="high">High</option><option value="max">Maximum</option></select></label><button onClick={onAttach}><Paperclip /> Add references</button></section>
    <ReferenceShelf references={references} generatedImages={generatedImages} editable onReferences={onReferences} onUseGenerated={onUseGenerated} />
    <button className="movie-advanced-toggle" onClick={() => onAdvanced(!advanced)}><Settings2 /> Production settings <ChevronDown className={advanced ? "open" : ""} /></button>
    {advanced && <div className="movie-advanced producer-settings"><NumberField label="Width" value={settings.width} min={256} max={2048} onChange={(width) => onSettings({ ...settings, width })} /><NumberField label="Height" value={settings.height} min={256} max={2048} onChange={(height) => onSettings({ ...settings, height })} /><NumberField label="Default scene seconds" value={settings.clipSeconds} min={5} max={15} onChange={(clipSeconds) => onSettings({ ...settings, clipSeconds })} /><NumberField label="Maximum scenes" value={settings.maxClips} min={1} max={advancedEnabled ? 96 : 24} onChange={(maxClips) => onSettings({ ...settings, maxClips })} /><NumberField label="H3 steps" value={settings.steps} min={1} max={advancedEnabled ? 100 : 40} onChange={(steps) => onSettings({ ...settings, steps })} /><NumberField label="Seed (0 = random)" value={settings.seed} min={0} max={Number.MAX_SAFE_INTEGER} onChange={(seed) => onSettings({ ...settings, seed })} /><label className="wide">ComfyUI root<input value={settings.comfyRoot} onChange={(event) => onSettings({ ...settings, comfyRoot: event.target.value })} /></label></div>}
    <div className="studio-room-assurance"><ShieldCheck /><span><strong>TypeScript never owns application truth.</strong><small>Every story revision, conversation, scene snapshot, media binding, render decision, and export is validated and persisted by Rust.</small></span></div>
  </div>;
}

function StoryRoom({ producer, revisionId, draft, editing, dirty, acceptedId, conversation, chat, instruction, modelId, models, disabled, onRevision, onDraft, onEditing, onInstruction, onSend, onStop, onSave, onAccept, onSummarize, onReset }: {
  producer: MovieProducerWorkspace; revisionId: string; draft: string; editing: boolean; dirty: boolean; acceptedId?: string;
  conversation: MovieStudioConversation | null; chat: ChatState; instruction: string; modelId: string; models: ModelInfo[]; disabled: boolean;
  onRevision: (id: string) => void; onDraft: (value: string) => void; onEditing: (value: boolean) => void; onInstruction: (value: string) => void;
  onSend: () => void; onStop: () => void; onSave: () => void; onAccept: (mode: MovieStudioConversationMode) => void; onSummarize: () => void; onReset: (keepSummary: boolean) => void;
}) {
  const revision = producer.storyRevisions.find((item) => item.id === revisionId);
  return <div className="producer-split-room"><main className="producer-document-room">
    <header className="producer-room-header"><span><small>Story document</small><strong>One complete creative source of truth</strong></span><div>
      <select aria-label="Story revision" value={revisionId} onChange={(event) => onRevision(event.target.value)}>{producer.storyRevisions.map((item) => <option value={item.id} key={item.id}>Revision {item.number}{item.id === acceptedId ? " · accepted" : ""}</option>)}</select>
      <button className={editing ? "active" : ""} onClick={() => onEditing(!editing)}><FilePenLine /> {editing ? "Preview" : "Edit"}</button>
      {editing && <button disabled={disabled || !dirty || draft.trim().length < 3} onClick={onSave}><Save /> Save revision</button>}
    </div></header>
    <article className={`producer-story-paper ${editing ? "editing" : ""}`}>
      {editing ? <textarea aria-label="Story document" maxLength={262144} value={draft} onChange={(event) => onDraft(event.target.value)} /> : draft ? <MarkdownContent value={draft} /> : <div className="studio-room-empty"><LoaderCircle className={chat.requestId ? "spin" : ""} /><strong>{chat.requestId ? "Writing the first story sketch" : "No story revision yet"}</strong><span>The complete Markdown response will become revision 1.</span></div>}
    </article>
    <footer className="producer-document-footer"><span>{revision ? `Revision ${revision.number} · ${new Date(revision.createdAt).toLocaleString()}` : "Waiting for the first revision"}{dirty ? " · unsaved edits" : ""}</span><div>{revisionId === acceptedId && <span className="accepted-badge"><Check /> Accepted</span>}<button disabled={disabled || Boolean(chat.requestId) || !revisionId || dirty} onClick={() => onAccept("continue")}><Check /> Accept and continue scenes</button><button disabled={disabled || Boolean(chat.requestId) || !revisionId || dirty} onClick={() => onAccept("fresh")}><RotateCcw /> Accept with fresh scene chat</button></div></footer>
  </main><ConversationPanel kind="story" conversation={conversation} chat={chat} instruction={instruction} modelId={modelId} models={models} disabled={disabled} selectedCount={0} onInstruction={onInstruction} onSend={onSend} onStop={onStop} onSummarize={onSummarize} onReset={onReset} /></div>;
}

function SceneRoom({ project, producer, scenes, selectedIds, dirty, conversation, chat, instruction, modelId, models, disabled, generatedImages, onScenes, onSelected, onInstruction, onAttach, onUseGenerated, onSave, onSend, onStop, onSummarize, onReset, onRender }: {
  project: Awaited<ReturnType<typeof getMovie>>; producer: MovieProducerWorkspace; scenes: MovieSceneDraft[]; selectedIds: string[]; dirty: boolean;
  conversation: MovieStudioConversation | null; chat: ChatState; instruction: string; modelId: string; models: ModelInfo[]; disabled: boolean; generatedImages: MovieReferenceAsset[];
  onScenes: (scenes: MovieSceneDraft[]) => void; onSelected: (ids: string[]) => void; onInstruction: (value: string) => void;
  onAttach: () => void; onUseGenerated: (asset: MovieReferenceAsset) => void; onSave: () => void; onSend: () => void; onStop: () => void; onSummarize: () => void; onReset: (keepSummary: boolean) => void; onRender: () => void;
}) {
  const accepted = producer.storyRevisions.find((item) => item.id === producer.acceptedStoryRevisionId);
  const patchScene = (id: string, patch: Partial<MovieSceneDraft>) => onScenes(scenes.map((scene) => scene.id === id ? { ...scene, ...patch } : scene));
  const move = (index: number, change: number) => {
    const target = index + change;
    if (target < 0 || target >= scenes.length) return;
    const next = [...scenes]; [next[index], next[target]] = [next[target], next[index]]; onScenes(next);
  };
  const remove = (id: string) => { onScenes(scenes.filter((scene) => scene.id !== id)); onSelected(selectedIds.filter((item) => item !== id)); };
  return <div className="producer-scene-layout"><main className="producer-scene-room">
    <header className="producer-room-header"><span><small>Scene cards · accepted story revision {accepted?.number ?? "—"}</small><strong>Producer-controlled H3 picture and sound</strong></span><div><button onClick={onAttach} disabled={disabled}><Paperclip /> Add media</button><button disabled={disabled || !dirty} onClick={onSave}><Save /> Save cards</button><button className="accent" disabled={disabled || !scenes.length || dirty} onClick={onRender}>{project.status === "running" ? <LoaderCircle className="spin" /> : <Play />} Render H3 masters</button></div></header>
    <div className="producer-context-note"><ShieldCheck /><span><strong>You choose the model context.</strong><small>The full accepted story is always included. Only checked scene cards are supplied in full. References, paths, frame bindings, and render controls never enter model context.</small></span></div>
    <ReferenceShelf references={project.references} generatedImages={generatedImages.filter((asset) => !project.references.some((item) => item.assetId === asset.id))} onUseGenerated={onUseGenerated} />
    <div className="producer-scene-list">{scenes.map((scene, index) => <SceneCard key={scene.id} scene={scene} index={index} first={index === 0} last={index === scenes.length - 1} references={project.references} selected={selectedIds.includes(scene.id)} disabled={disabled} rendered={project.clips.find((clip) => clip.id === scene.id)} onSelected={(selected) => onSelected(selected ? [...selectedIds, scene.id] : selectedIds.filter((id) => id !== scene.id))} onPatch={(patch) => patchScene(scene.id, patch)} onMoveUp={() => move(index, -1)} onMoveDown={() => move(index, 1)} onRemove={() => remove(scene.id)} />)}
      {!scenes.length && <div className="studio-room-empty"><Video /><strong>Turn the accepted story into scenes</strong><span>Ask the scene collaborator for a first pass, or add the first card yourself. Nothing renders until you approve the cards.</span></div>}
    </div>
    <button className="producer-add-scene" disabled={disabled || scenes.length >= project.settings.maxClips} onClick={() => onScenes([...scenes, emptyScene(producer.acceptedStoryRevisionId ?? "", project.settings.clipSeconds, scenes.length)])}><Plus /> Add scene card</button>
  </main><ConversationPanel kind="scenes" conversation={conversation} chat={chat} instruction={instruction} modelId={modelId} models={models} disabled={disabled} selectedCount={selectedIds.length} onInstruction={onInstruction} onSend={onSend} onStop={onStop} onSummarize={onSummarize} onReset={onReset} /></div>;
}

function SceneCard({ scene, index, first, last, references, selected, disabled, rendered, onSelected, onPatch, onMoveUp, onMoveDown, onRemove }: {
  scene: MovieSceneDraft; index: number; first: boolean; last: boolean; references: MovieReference[]; selected: boolean; disabled: boolean;
  rendered?: Awaited<ReturnType<typeof getMovie>>["clips"][number];
  onSelected: (value: boolean) => void; onPatch: (patch: Partial<MovieSceneDraft>) => void; onMoveUp: () => void; onMoveDown: () => void; onRemove: () => void;
}) {
  const images = references.filter((item) => item.kind === "image");
  const hasFrames = Boolean(scene.firstFrame || scene.lastFrame);
  const frameValue = (frame?: MovieSceneFrameSource) => !frame ? "" : frame.kind === "previousScene" ? "previous" : `image:${frame.assetId}`;
  const parseFrame = (value: string): MovieSceneFrameSource | undefined => value === "previous" ? { kind: "previousScene" } : value.startsWith("image:") ? { kind: "referenceImage", assetId: value.slice(6) } : undefined;
  const updateReference = (reference: MovieReference, signal: "visual" | "audio", checked: boolean) => {
    const current = scene.references.find((item) => item.assetId === reference.assetId);
    const next = current ?? { assetId: reference.assetId, useVisual: false, useAudio: false, guidance: "" };
    const updated = signal === "visual" ? { ...next, useVisual: checked } : { ...next, useAudio: checked };
    onPatch({ references: updated.useVisual || updated.useAudio ? [...scene.references.filter((item) => item.assetId !== reference.assetId), updated] : scene.references.filter((item) => item.assetId !== reference.assetId) });
  };
  return <article className={`producer-scene-card ${selected ? "selected" : ""}`}>
    <header><label><input type="checkbox" aria-label={`Include ${scene.title} in scene chat context`} checked={selected} onChange={(event) => onSelected(event.target.checked)} /> <span>Scene {index + 1}</span></label><div><button aria-label={`Move ${scene.title} up`} disabled={disabled || first} onClick={onMoveUp}><ArrowUp /></button><button aria-label={`Move ${scene.title} down`} disabled={disabled || last} onClick={onMoveDown}><ArrowDown /></button><button aria-label={`Remove ${scene.title}`} disabled={disabled} onClick={onRemove}><Trash2 /></button></div></header>
    <div className="producer-scene-fields"><label>Title<input value={scene.title} maxLength={160} onChange={(event) => onPatch({ title: event.target.value })} /></label><label>Purpose<input value={scene.purpose} maxLength={4000} onChange={(event) => onPatch({ purpose: event.target.value })} placeholder="What changes in the movie here?" /></label><label className="short">Seconds<input type="number" min={5} max={15} step={1} value={scene.durationSeconds} onChange={(event) => onPatch({ durationSeconds: Number(event.target.value) })} /></label><label className="wide">H3 prompt<textarea aria-label={`${scene.title} H3 prompt`} maxLength={65536} value={scene.h3Prompt} onChange={(event) => onPatch({ h3Prompt: event.target.value })} placeholder="Concrete subject, setting, framing, camera, light, timed action beats, exact sound/dialogue, exclusions, and visible final frame…" /></label><label>Continuity in<textarea value={scene.continuityIn} onChange={(event) => onPatch({ continuityIn: event.target.value })} /></label><label>Continuity out<textarea value={scene.continuityOut} onChange={(event) => onPatch({ continuityOut: event.target.value })} /></label><label>Transition<input value={scene.transition} onChange={(event) => onPatch({ transition: event.target.value })} /></label></div>
    <section className="producer-frame-controls"><strong>Frame conditioning</strong><small>Choose first/last frames, or native references below — H3 does not combine both.</small><div><label>First frame<select value={frameValue(scene.firstFrame)} onChange={(event) => onPatch({ firstFrame: parseFrame(event.target.value), references: event.target.value ? [] : scene.references })}><option value="">None</option>{!first && <option value="previous">Previous scene final frame</option>}{images.map((item) => <option key={item.assetId} value={`image:${item.assetId}`}>{item.name}</option>)}</select></label><label>Last frame<select value={frameValue(scene.lastFrame)} onChange={(event) => onPatch({ lastFrame: parseFrame(event.target.value), references: event.target.value ? [] : scene.references })}><option value="">None</option>{images.map((item) => <option key={item.assetId} value={`image:${item.assetId}`}>{item.name}</option>)}</select></label></div></section>
    <section className={`producer-reference-bindings ${hasFrames ? "disabled" : ""}`}><strong>Native H3 references</strong>{hasFrames && <small>Clear frame conditioning to enable references.</small>}{references.map((reference) => {
      const value = scene.references.find((item) => item.assetId === reference.assetId);
      const canVisual = reference.kind !== "audio"; const canAudio = reference.kind === "audio" || reference.hasAudio;
      return <div key={reference.assetId}><span><b>{reference.name}</b><small>{reference.tag}{reference.audioTag ? ` + ${reference.audioTag}` : ""}</small></span>{canVisual && <label><input type="checkbox" disabled={hasFrames} checked={value?.useVisual ?? false} onChange={(event) => updateReference(reference, "visual", event.target.checked)} /> Visual / motion</label>}{canAudio && <label><input type="checkbox" disabled={hasFrames || (reference.kind === "video" && !(value?.useVisual ?? false))} checked={value?.useAudio ?? false} onChange={(event) => updateReference(reference, "audio", event.target.checked)} /> Exact audio</label>}{value && <input aria-label={`${reference.name} scene guidance`} disabled={hasFrames} value={value.guidance} onChange={(event) => onPatch({ references: scene.references.map((item) => item.assetId === reference.assetId ? { ...item, guidance: event.target.value } : item) })} placeholder="Optional placement guidance from you…" />}</div>;
    })}</section>
    {rendered?.path && <details className="producer-rendered-scene"><summary><Check /> H3 master available</summary><video controls preload="metadata" src={movieMediaUrl(rendered.path)} /></details>}
  </article>;
}

function ConversationPanel({ kind, conversation, chat, instruction, modelId, models, disabled, selectedCount, onInstruction, onSend, onStop, onSummarize, onReset }: {
  kind: MovieStudioConversationKind; conversation: MovieStudioConversation | null; chat: ChatState; instruction: string;
  modelId: string; models: ModelInfo[]; disabled: boolean; selectedCount: number;
  onInstruction: (value: string) => void; onSend: () => void; onStop: () => void; onSummarize: () => void; onReset: (keepSummary: boolean) => void;
}) {
  return <aside className="producer-chat-panel"><header><span><MessageSquare /><strong>{kind === "story" ? "Story collaborator" : "Scene collaborator"}</strong><small>{kind === "story" ? "Every response is a full Markdown revision" : `${selectedCount} scene card${selectedCount === 1 ? "" : "s"} in full context`}</small></span><div><button disabled={disabled || Boolean(chat.requestId) || !conversation?.messages.length} title="Summarize this conversation" onClick={onSummarize}><Sparkles /></button><button disabled={disabled || Boolean(chat.requestId) || !conversation} title="Clear and start a blank conversation" onClick={() => onReset(false)}><Trash2 /></button><button disabled={disabled || Boolean(chat.requestId) || !conversation?.summary} title="Start a new conversation carrying the saved summary" onClick={() => onReset(true)}><RotateCcw /></button></div></header>
    {conversation?.summary && <section className="producer-chat-summary"><strong>Carried summary</strong><MarkdownContent value={conversation.summary} /></section>}
    <div className="producer-chat-history">{conversation?.messages.map((message) => <article className={message.role} key={message.id}><small>{message.role === "producer" ? "You" : message.role === "collaborator" ? "Local collaborator" : "Kestrel"}{message.selectedSceneIds.length ? ` · ${message.selectedSceneIds.length} selected scenes` : ""}</small><MarkdownContent value={message.markdown} /></article>)}
      {chat.text && <article className="collaborator live"><small>Local collaborator · live</small><MarkdownContent value={chat.text} streaming /></article>}
      {!conversation?.messages.length && !chat.text && <div className="studio-room-empty"><MessageSquare /><strong>{kind === "story" ? "Revise by talking" : "Shape scenes by talking"}</strong><span>{kind === "story" ? "Ask for tone, structure, character, pacing, or a full rewrite. Every answer is saved as a new revision." : "Check only the cards the collaborator may change. It can add cards around the outline without seeing your media choices."}</span></div>}
    </div>
    {chat.reasoning && <ModelThinkingStream text={chat.reasoning} active={Boolean(chat.requestId)} />}
    {chat.status && <div className="producer-chat-status">{chat.requestId && <LoaderCircle className="spin" />}{chat.status}</div>}
    <div className="producer-chat-compose"><textarea aria-label={`${kind} collaborator direction`} maxLength={16000} value={instruction} onChange={(event) => onInstruction(event.target.value)} placeholder={kind === "story" ? "Make the ending quieter and let Mara choose to stay…" : selectedCount ? "Split the selected scene and make the second beat more intimate…" : "Create a first scene pass from the accepted story…"} /><div><select aria-label={`${kind} collaborator model`} value={modelId} disabled>{models.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}</select>{chat.requestId ? <button className="danger" onClick={onStop}><CircleStop /> Stop</button> : <button disabled={disabled || !modelId || instruction.trim().length < 2} onClick={onSend}><Send /> Send</button>}</div></div>
    <footer><ShieldCheck /> Tool-free local inference. Conversation and revisions are durable.</footer>
  </aside>;
}

function ReferenceShelf({ references, generatedImages = [], editable = false, onReferences, onUseGenerated }: {
  references: Array<PendingMovieReference | MovieReference>; generatedImages?: MovieReferenceAsset[]; editable?: boolean;
  onReferences?: (value: PendingMovieReference[]) => void; onUseGenerated?: (asset: MovieReferenceAsset) => void;
}) {
  if (!references.length && !generatedImages.length) return null;
  return <details className="producer-reference-shelf"><summary><Paperclip /> {references.length} attached reference{references.length === 1 ? "" : "s"}{generatedImages.length ? ` · ${generatedImages.length} generated image${generatedImages.length === 1 ? "" : "s"} available` : ""}</summary><div>{references.map((reference) => {
    const id = reference.assetId;
    return <article key={id}><ReferencePreview reference={reference} /><span><strong>{reference.name}</strong><small>{reference.kind}{reference.hasAudio ? " · audio available" : ""}</small>{editable && onReferences && <textarea value={reference.description} onChange={(event) => onReferences(references.map((item) => ({ ...item, description: item.assetId === id ? event.target.value : item.description })) as PendingMovieReference[])} placeholder="Optional identity, style, motion, or use note…" />}</span>{editable && onReferences && <button aria-label={`Remove ${reference.name}`} onClick={() => onReferences(references.filter((item) => item.assetId !== id) as PendingMovieReference[])}><X /></button>}</article>;
  })}{generatedImages.map((asset) => <article key={asset.id}><ReferencePreview reference={asset} /><span><strong>{asset.name}</strong><small>Generated image</small></span><button className="use-generated" onClick={() => onUseGenerated?.(asset)}><Plus /> Use</button></article>)}</div></details>;
}

function ReferencePreview({ reference }: { reference: MovieReferenceAsset | MovieReference | PendingMovieReference }) {
  const source = movieMediaUrl(reference.path);
  if (reference.kind === "image") return <div className="movie-reference-preview">{source && <img src={source} alt="" />}</div>;
  if (reference.kind === "video") return <div className="movie-reference-preview">{source && <video controls preload="metadata" src={source} />}</div>;
  return <div className="movie-reference-preview audio"><AudioLines />{source && <audio controls preload="metadata" src={source} />}</div>;
}

function DeliveryRoom({ project, edit, busy, complete, onExport }: {
  project: Awaited<ReturnType<typeof getMovie>>; edit: MovieEdit; busy: boolean; complete: number; onExport: () => void;
}) {
  const latest = project.exports.at(-1);
  return <section className="project-room-scroll delivery-room"><div className="studio-room-heading"><span><small>Producer delivery room</small><strong>Review and preserve every explicit export</strong></span><button disabled={busy || !complete || project.status === "running" || !edit.clips.some((item) => item.enabled)} onClick={onExport}>{busy ? <LoaderCircle className="spin" /> : <Play />} Export current cut</button></div>
    {project.finalPath ? <section className="movie-final"><div className="movie-section-heading"><div><span className="eyebrow">{latest ? "Latest immutable export" : "Assembled file"}</span><h2>{latest?.title ?? project.title}</h2><small>Native masters and earlier exports remain addressable.</small></div><a href={movieMediaUrl(project.finalPath)} download><Download /> Open file</a></div><video controls preload="metadata" src={movieMediaUrl(project.finalPath)} /></section> : <div className="studio-room-empty"><Download /><strong>No deliverable yet</strong><span>Render scenes, arrange the timeline, then explicitly export a cut.</span></div>}
    {project.exports.length > 0 && <section className="movie-export-history"><div className="movie-section-heading"><div><span className="eyebrow">Immutable deliverables</span><h2>Export history</h2></div></div><div>{[...project.exports].reverse().map((item) => <article key={item.id}><span><strong>{item.title}</strong><small>{new Date(item.createdAt).toLocaleString()} · {item.preset} · {item.clipCount} items · {item.durationSeconds.toFixed(2)}s</small><code title={item.sha256}>{item.sha256.slice(0, 16)}…</code></span><a href={movieMediaUrl(item.path)} download><Download /> Open</a></article>)}</div></section>}
  </section>;
}

function NumberField({ label, value, min, max, onChange }: { label: string; value: number; min: number; max: number; onChange: (value: number) => void }) {
  return <label>{label}<input type="number" min={min} max={max} value={value} onChange={(event) => { const next = Number(event.target.value); if (Number.isFinite(next) && next >= min && next <= max) onChange(next); }} /></label>;
}

function toPendingReference(asset: MovieReferenceAsset): PendingMovieReference {
  const description = asset.kind === "image"
    ? `Use ${asset.name} as a producer-selected visual identity and appearance reference.`
    : asset.kind === "video"
      ? `Use ${asset.name} as a producer-selected motion and visual reference.`
      : `Use the exact audio from ${asset.name} when the producer selects it for a scene.`;
  return {
    ...asset,
    assetId: asset.id,
    description,
    useEmbeddedAudio: asset.kind === "video" && asset.hasAudio,
    embeddedAudioDescription: asset.kind === "video" && asset.hasAudio
      ? `Use the exact embedded audio from ${asset.name} when selected for a scene.`
      : "",
  };
}
