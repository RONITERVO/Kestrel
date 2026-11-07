import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle, BookOpen, CheckCircle2, CircleStop, Clapperboard, Cpu, Download,
  Film, FolderOpen, Layers, LoaderCircle, PanelLeft, Plus, Save, Sparkles,
  Users, WandSparkles, X,
} from "lucide-react";
import {
  cancelMovieRender, cancelStudioPromptDraft, captureMovieH3FrameGuide,
  compileMovieH3Window, createMovieH3Project, createMovieH3Scene, getMovie,
  getMovieH3Environment, inspectMovieH3Take, installMovieH3TurboLoras,
  listMovieReferenceAssets, listMovies, movieMediaUrl, onMovieProject,
  onStudioPromptDraft, placeMovieH3Take, renderMovieEdit, revealMovie,
  saveMovieH3Scene, saveMovieH3Story, startMovieH3Generation,
  startStudioPromptDraft,
} from "../../../platform/api";
import type {
  ControlSettings, H3EnvironmentSnapshot, H3RenderQueueJob, H3Scene, H3SceneKind,
  H3TakeInspection, H3TakePlacementMode, H3WindowCompileResult, ModelInfo,
  MovieProject, MovieReferenceAsset, MovieSummary, PromptDraftMode,
  PromptDraftReceipt, PromptDraftTarget, ThinkingLevel,
} from "../../../contracts/index";
import { appendModelThinking } from "../../control/ModelThinkingStream";
import { ModelReplyPages } from "../../control/ModelReplyPages";
import { effectiveModelRuntimePolicy, ModelRuntimePolicyControls, type RuntimePolicyValue } from "../../control/ModelRuntimePolicy";
import { effectiveThinkingLevelForModel } from "../../control/modelPolicy";
import { PagedList } from "../../../shared/book/PagedList";
import { ExternalCollaborationExchange } from "../../../shared/collaboration/ExternalCollaborationExchange";
import { buildExternalCollaborationRequest, parseExternalTextResult } from "../../../shared/collaboration/externalCollaboration";
import type { ImageHandoff } from "../imageHandoff";
import { H3ProgramMonitor, type H3MonitorSource } from "./MovieH3Preview";
import { H3EntityLibrary, MovieAssetCreator } from "./MovieAssetCreator";
import { MovieGenerationPanel } from "./MovieGenerationPanel";
import { activeMonitorSourceAtPlayhead, MovieTimeline, snapToFrame } from "./MovieTimeline";
import "./movie.css";

type StudioRoom = "studio" | "references" | "story" | "takes" | "deliver";

interface CollaboratorDraftSession {
  id: string;
  label: string;
  target: PromptDraftTarget;
  mode: PromptDraftMode;
  text: string;
  reasoning: string;
  status: string;
  modelName: string;
  thinkingLevel?: ThinkingLevel;
  receipt?: PromptDraftReceipt;
  apply: (text: string) => void;
}

const CANVAS_PRESETS: { key: string; label: string; width: number; height: number }[] = [
  { key: "1920x1088", label: "1920×1088 · Standard 16:9 Cinema", width: 1920, height: 1088 },
  { key: "960x544", label: "960×544 · Fast Half-Res Audition", width: 960, height: 544 },
  { key: "1088x1920", label: "1088×1920 · Vertical 9:16", width: 1088, height: 1920 },
  { key: "1440x1088", label: "1440×1088 · Academy 4:3", width: 1440, height: 1088 },
];

const EXPORT_PRESETS: { value: string; label: string; hint: string }[] = [
  { value: "publish", label: "Publish Master (MP4 · H.264 BT.709 + AAC 48kHz)", hint: "Web & screening ready with -1 dBFS peak headroom" },
  { value: "review", label: "Fast Review Cut (MP4 · Quick H.264)", hint: "Rapid editorial check" },
  { value: "archive", label: "Archive Master (MP4 · High-Bitrate H.264)", hint: "Near-master archival quality" },
  { value: "lossless", label: "Lossless Mezzanine (MKV · FFV1 + FLAC)", hint: "Bit-exact archival master" },
  { value: "audio_flac", label: "Soundtrack Stem Only (FLAC · 48kHz Stereo)", hint: "Lossless audio bounce" },
];

function makeRequestId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? `draft-${crypto.randomUUID().slice(0, 8)}`
    : `draft-${Date.now().toString(36)}`;
}

export function MovieStudio({
  initialComfyRoot,
  advancedEnabled,
  models = [],
  selectedModelId,
  controlSettings,
  onError,
  onRequestImage,
}: {
  initialComfyRoot?: string;
  advancedEnabled: boolean;
  models?: ModelInfo[];
  selectedModelId?: string;
  controlSettings?: ControlSettings;
  onError: (message: string) => void;
  onRequestImage?: (handoff: ImageHandoff) => void;
}) {
  const [summaries, setSummaries] = useState<MovieSummary[]>([]);
  const [project, setProject] = useState<MovieProject>();
  const [sceneDraft, setSceneDraft] = useState<H3Scene>();
  const [sceneDirty, setSceneDirty] = useState(false);
  const [storyDraft, setStoryDraft] = useState("");
  const [storyTitleDraft, setStoryTitleDraft] = useState("");
  const [storyInstruction, setStoryInstruction] = useState("Updated story brief");
  const [storyDirty, setStoryDirty] = useState(false);

  const [environment, setEnvironment] = useState<H3EnvironmentSnapshot>();
  const [installingTurbo, setInstallingTurbo] = useState(false);
  const [referenceAssets, setReferenceAssets] = useState<MovieReferenceAsset[]>([]);
  const [compileResult, setCompileResult] = useState<H3WindowCompileResult | null>(null);

  const [room, setRoom] = useState<StudioRoom>("studio");
  const [showLibrary, setShowLibrary] = useState(() => window.matchMedia?.("(max-width: 1199px)").matches !== true);
  const [loading, setLoading] = useState(true);
  const [creating, setCreating] = useState(false);
  const [busyAction, setBusyAction] = useState("");
  const [notice, setNotice] = useState("");

  // New production form
  const [newTitle, setNewTitle] = useState("");
  const [newStory, setNewStory] = useState("");
  const [newCanvasKey, setNewCanvasKey] = useState("1920x1088");
  const [newDurationSec, setNewDurationSec] = useState(15);

  // Playback & timeline selection
  const [playheadSec, setPlayheadSec] = useState(0);
  const [selectedShotId, setSelectedShotId] = useState<string>();

  // Takes & Telemetry room state
  const [selectedTakeJobId, setSelectedTakeJobId] = useState("");
  const [takeInspection, setTakeInspection] = useState<H3TakeInspection>();
  const [takeInspecting, setTakeInspecting] = useState(false);
  const [placeMode, setPlaceMode] = useState<H3TakePlacementMode>("trimmed");
  const [placeSceneId, setPlaceSceneId] = useState("");
  const [placeVideoLaneId, setPlaceVideoLaneId] = useState("");
  const [placeAudioLaneId, setPlaceAudioLaneId] = useState("");
  const [placeDriverLaneId, setPlaceDriverLaneId] = useState("");

  // Deliver / Export room state
  const [exportSceneId, setExportSceneId] = useState("");
  const [exportTitle, setExportTitle] = useState("");
  const [exportPreset, setExportPreset] = useState("publish");
  const [exportHeadroomDb, setExportHeadroomDb] = useState(-1.0);
  const [exporting, setExporting] = useState(false);

  // Local LLM Collaborator state
  const [modelId, setModelId] = useState(selectedModelId ?? models[0]?.id ?? "");
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel | "default">("default");
  const [draftMode, setDraftMode] = useState<PromptDraftMode>("develop");
  const [runtimePolicyOverride, setRuntimePolicyOverride] = useState<RuntimePolicyValue>();
  const [draftSession, setDraftSession] = useState<CollaboratorDraftSession>();

  const activeProjectIdRef = useRef("");
  const sceneDirtyRef = useRef(false);
  const storyDirtyRef = useRef(false);
  sceneDirtyRef.current = sceneDirty;
  storyDirtyRef.current = storyDirty;

  useEffect(() => {
    if (!modelId && (selectedModelId || models[0]?.id)) {
      setModelId(selectedModelId ?? models[0]?.id ?? "");
    }
  }, [modelId, models, selectedModelId]);

  const applyLoadedProject = useCallback((loaded: MovieProject, preserveDrafts = false) => {
    activeProjectIdRef.current = loaded.id;
    setProject(loaded);
    const activeScene = loaded.scenes.find((s) => s.id === loaded.activeSceneId) ?? loaded.scenes[0];
    if (activeScene && (!preserveDrafts || !sceneDirtyRef.current)) {
      setSceneDraft(structuredClone(activeScene));
      setSceneDirty(false);
      setSelectedShotId((prev) =>
        activeScene.promptSections.some((s) => s.id === prev) ? prev : activeScene.promptSections[0]?.id,
      );
    }
    if (!preserveDrafts || !storyDirtyRef.current) {
      setStoryDraft(loaded.storyMarkdown ?? "");
      setStoryTitleDraft(loaded.title);
      setStoryDirty(false);
    }
    setExportSceneId((prev) =>
      loaded.scenes.some((s) => s.id === prev) ? prev : activeScene?.id ?? "",
    );
    setPlaceSceneId((prev) =>
      loaded.scenes.some((s) => s.id === prev) ? prev : activeScene?.id ?? "",
    );
    setExportTitle((prev) => prev || loaded.edit?.exportTitle || loaded.title);
    setSelectedTakeJobId((prev) =>
      loaded.renderJobs.some((j) => j.id === prev)
        ? prev
        : [...loaded.renderJobs].reverse()[0]?.id ?? "",
    );
  }, []);

  const refreshSummariesAndAssets = useCallback(async () => {
    const [nextSummaries, nextEnv, nextAssets] = await Promise.all([
      listMovies().catch(() => [] as MovieSummary[]),
      getMovieH3Environment().catch(() => undefined),
      listMovieReferenceAssets().catch(() => [] as MovieReferenceAsset[]),
    ]);
    setSummaries(nextSummaries);
    if (nextEnv) setEnvironment(nextEnv);
    setReferenceAssets(nextAssets);
    return nextSummaries;
  }, []);

  const refreshActiveProject = useCallback(async (preferredId?: string) => {
    const nextSummaries = await listMovies();
    setSummaries(nextSummaries);
    const targetId = preferredId || activeProjectIdRef.current || nextSummaries[0]?.id;
    if (targetId) {
      const loaded = await getMovie(targetId);
      applyLoadedProject(loaded, false);
      setCreating(false);
    } else {
      setCreating(true);
    }
  }, [applyLoadedProject]);

  useEffect(() => {
    let disposed = false;
    const cleanups: Array<() => void> = [];

    (async () => {
      try {
        const nextSummaries = await refreshSummariesAndAssets();
        if (disposed) return;
        if (nextSummaries[0]?.id) {
          const loaded = await getMovie(nextSummaries[0].id);
          if (!disposed) applyLoadedProject(loaded, false);
        } else {
          setCreating(true);
        }
      } catch (err) {
        if (!disposed) onError(String(err));
      } finally {
        if (!disposed) setLoading(false);
      }
    })();

    void onMovieProject((next) => {
      if (disposed) return;
      void listMovies().then((items) => { if (!disposed) setSummaries(items); }).catch(() => undefined);
      void listMovieReferenceAssets().then((assets) => { if (!disposed) setReferenceAssets(assets); }).catch(() => undefined);
      if (next.id === activeProjectIdRef.current) {
        applyLoadedProject(next, true);
      } else if (!activeProjectIdRef.current) {
        applyLoadedProject(next, false);
        setCreating(false);
      }
    }).then((dispose) => (disposed ? dispose() : cleanups.push(dispose)));

    void onStudioPromptDraft((event) => {
      if (disposed) return;
      if (event.kind === "error") onError(event.content ?? "Local Studio collaborator stopped.");
      setDraftSession((current) => {
        if (!current || current.id !== event.requestId) return current;
        const nextThinking = event.thinkingLevel ?? current.thinkingLevel;
        if (event.kind === "token") {
          return {
            ...current,
            text: current.text + (event.content ?? ""),
            status: "writing",
            modelName: event.modelName ?? current.modelName,
            thinkingLevel: nextThinking,
          };
        }
        if (event.kind === "started") {
          return {
            ...current,
            status: "writing",
            modelName: event.modelName ?? current.modelName,
            receipt: event.receipt ?? undefined,
            thinkingLevel: nextThinking,
          };
        }
        if (event.kind === "reasoning") {
          return {
            ...current,
            reasoning: appendModelThinking(current.reasoning, event.content ?? ""),
            status: "thinking",
            modelName: event.modelName ?? current.modelName,
            thinkingLevel: nextThinking,
          };
        }
        if (event.kind === "complete") {
          return { ...current, status: "ready", modelName: event.modelName ?? current.modelName, thinkingLevel: nextThinking };
        }
        if (event.kind === "limited" || event.kind === "cancelled") {
          return { ...current, status: "checkpoint", modelName: event.modelName ?? current.modelName, thinkingLevel: nextThinking };
        }
        if (event.kind === "error") {
          return { ...current, status: "error" };
        }
        return current;
      });
    }).then((dispose) => (disposed ? dispose() : cleanups.push(dispose)));

    return () => {
      disposed = true;
      cleanups.forEach((fn) => fn());
    };
    // Subscriptions remain stable across the MovieStudio session.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Recompile active H3 window whenever project/scene/window/generation updates
  useEffect(() => {
    if (!project?.id || !sceneDraft?.id) {
      setCompileResult(null);
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void compileMovieH3Window({
        projectId: project.id,
        sceneId: sceneDraft.id,
        windowOverride: sceneDraft.window,
        generationOverride: sceneDraft.generation,
      })
        .then((res) => { if (!cancelled) setCompileResult(res); })
        .catch(() => { if (!cancelled) setCompileResult(null); });
    }, 100);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [
    project?.id,
    project?.updatedAt,
    sceneDraft?.id,
    sceneDraft?.window,
    sceneDraft?.generation,
  ]);

  // Inspect selected take when in Takes room
  useEffect(() => {
    if (room !== "takes" || !project?.id || !selectedTakeJobId) {
      setTakeInspection(undefined);
      return;
    }
    let cancelled = false;
    setTakeInspecting(true);
    void inspectMovieH3Take(project.id, selectedTakeJobId)
      .then((res) => {
        if (!cancelled) {
          setTakeInspection(res);
          setPlaceSceneId((prev) => prev || res.sceneId);
        }
      })
      .catch((err) => { if (!cancelled) onError(String(err)); })
      .finally(() => { if (!cancelled) setTakeInspecting(false); });
    return () => { cancelled = true; };
  }, [room, project?.id, project?.updatedAt, selectedTakeJobId, onError]);

  const rendering = useMemo(() => {
    if (!project) return false;
    return (
      project.status === "rendering" ||
      project.renderJobs.some(
        (j) =>
          j.status === "queued" ||
          j.status === "preparing" ||
          j.status === "rendering" ||
          j.status === "postProcessing",
      )
    );
  }, [project]);

  const completedTakes = useMemo<H3RenderQueueJob[]>(() => {
    if (!project) return [];
    return [...project.renderJobs].filter((j) => j.status === "complete" && Boolean(j.takeMediaPath)).reverse();
  }, [project]);

  const assetPathMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const asset of referenceAssets) {
      if (asset.path) map.set(asset.id, asset.path);
    }
    return map;
  }, [referenceAssets]);

  const savedScene = useMemo(() => {
    if (!project || !sceneDraft) return undefined;
    return project.scenes.find((s) => s.id === sceneDraft.id);
  }, [project, sceneDraft]);

  const activeMonitorClip: H3MonitorSource | undefined = useMemo(() => {
    if (!sceneDraft || !project) return undefined;
    return activeMonitorSourceAtPlayhead(project, sceneDraft, playheadSec, assetPathMap);
  }, [sceneDraft, playheadSec, project, assetPathMap]);

  const selectedModel = useMemo(
    () => models.find((m) => m.id === modelId) ?? models[0],
    [models, modelId],
  );
  const inheritedPolicy = useMemo(
    () => effectiveModelRuntimePolicy(controlSettings, selectedModel?.id, selectedModel?.fixedContextWindow),
    [controlSettings, selectedModel],
  );
  const activeRuntimePolicy = runtimePolicyOverride ?? inheritedPolicy;

  const startLocalDraft = async (
    target: PromptDraftTarget,
    label: string,
    existingText: string,
    assetName: string,
    assetKind: string,
    apply: (text: string) => void,
  ) => {
    const chosenModel = selectedModel?.id || modelId;
    if (!chosenModel) {
      onError("Select a local GGUF model in Control before starting the local Studio collaborator, or use External AI Copy/Paste.");
      return;
    }
    const requestId = makeRequestId();
    const resolvedThinking = thinkingLevel === "default"
      ? effectiveThinkingLevelForModel(controlSettings, selectedModel?.id)
      : thinkingLevel;
    setDraftSession({
      id: requestId,
      label,
      target,
      mode: draftMode,
      text: "",
      reasoning: "",
      status: "starting",
      modelName: selectedModel?.name ?? chosenModel,
      thinkingLevel: resolvedThinking,
      apply,
    });
    try {
      await startStudioPromptDraft({
        requestId,
        modelId: chosenModel,
        target,
        mode: draftMode,
        storyText: storyDraft || project?.storyMarkdown || newStory || "",
        existingText,
        assetName,
        assetKind,
        thinkingLevel: resolvedThinking,
        contextWindow: activeRuntimePolicy.contextWindow,
        maxOutputTokens: activeRuntimePolicy.maxOutputTokens,
      });
    } catch (err) {
      setDraftSession(undefined);
      onError(String(err));
    }
  };

  const persistSceneIfNeeded = async (force = false): Promise<H3Scene | undefined> => {
    if (!project || !sceneDraft) return sceneDraft;
    if (!force && !sceneDirty && project.activeSceneId === sceneDraft.id) return sceneDraft;
    setBusyAction("saving-scene");
    try {
      const updated = await saveMovieH3Scene({
        projectId: project.id,
        scene: sceneDraft,
        setActive: true,
        replaceEmptyCollections: true,
      });
      applyLoadedProject(updated, false);
      const refreshed = updated.scenes.find((s) => s.id === sceneDraft.id) ?? updated.scenes[0];
      return refreshed;
    } finally {
      setBusyAction("");
    }
  };

  const handleSceneChange = (next: H3Scene) => {
    setSceneDraft(next);
    setSceneDirty(true);
  };

  const handleSaveScene = async () => {
    try {
      await persistSceneIfNeeded(true);
      setNotice("Saved H3 scene timeline & refreshed prompt compiler.");
    } catch (err) {
      onError(String(err));
    }
  };

  const handleSelectProject = async (id: string) => {
    if (id === project?.id && !creating) return;
    try {
      if (sceneDirty && project) {
        await persistSceneIfNeeded(true);
      }
      const loaded = await getMovie(id);
      applyLoadedProject(loaded, false);
      setPlayheadSec(0);
      setCreating(false);
    } catch (err) {
      onError(String(err));
    }
  };

  const handleCreateProject = async () => {
    const preset = CANVAS_PRESETS.find((p) => p.key === newCanvasKey) ?? CANVAS_PRESETS[0];
    setBusyAction("creating-project");
    try {
      const created = await createMovieH3Project({
        title: newTitle.trim() || "Untitled H3 Production",
        storyMarkdown: newStory.trim(),
        width: preset.width,
        height: preset.height,
        durationSec: Math.max(2, newDurationSec),
      });
      await refreshSummariesAndAssets();
      applyLoadedProject(created, false);
      setCreating(false);
      setRoom("studio");
      setNewTitle("");
      setNewStory("");
      setNotice(`Created MiniMax H3 production "${created.title}".`);
    } catch (err) {
      onError(String(err));
    } finally {
      setBusyAction("");
    }
  };

  const handleSwitchScene = async (targetSceneId: string) => {
    if (!project || targetSceneId === sceneDraft?.id) return;
    try {
      let currentProject = project;
      if (sceneDirty && sceneDraft) {
        currentProject = await saveMovieH3Scene({
          projectId: project.id,
          scene: sceneDraft,
          setActive: false,
          replaceEmptyCollections: true,
        });
      }
      const target = currentProject.scenes.find((s) => s.id === targetSceneId);
      if (!target) return;
      const updated = await saveMovieH3Scene({
        projectId: currentProject.id,
        scene: target,
        setActive: true,
        replaceEmptyCollections: true,
      });
      applyLoadedProject(updated, false);
      setPlayheadSec(snapToFrame(target.window.inSec));
    } catch (err) {
      onError(String(err));
    }
  };

  const handleAddScene = async (kind: H3SceneKind) => {
    if (!project) return;
    setBusyAction("adding-scene");
    try {
      if (sceneDirty) await persistSceneIfNeeded(true);
      const count = project.scenes.filter((s) => s.kind === kind).length + 1;
      const spec =
        kind === "audition"
          ? { name: `Audition ${count}`, width: 960, height: 544, durationSec: 10 }
          : kind === "soundtrack"
            ? { name: `Soundtrack ${count}`, width: 64, height: 32, durationSec: 15 }
            : { name: `Scene ${project.scenes.length + 1}`, width: 1920, height: 1088, durationSec: 15 };
      const updated = await createMovieH3Scene({
        projectId: project.id,
        name: spec.name,
        kind,
        width: spec.width,
        height: spec.height,
        durationSec: spec.durationSec,
      });
      applyLoadedProject(updated, false);
      setPlayheadSec(0);
      setRoom("studio");
      setNotice(`Created ${spec.name} (${spec.width}×${spec.height}).`);
    } catch (err) {
      onError(String(err));
    } finally {
      setBusyAction("");
    }
  };

  const handleInstallTurboLoras = async () => {
    setInstallingTurbo(true);
    try {
      const nextEnv = await installMovieH3TurboLoras();
      setEnvironment(nextEnv);
      setNotice("MiniMax H3 Ref2V + FL2V Turbo LoRAs verified and ready.");
    } catch (err) {
      onError(String(err));
    } finally {
      setInstallingTurbo(false);
    }
  };

  const handleStartGeneration = async (expandQualityLadder: boolean) => {
    if (!project || !sceneDraft) return;
    setBusyAction("starting-render");
    try {
      const persistedScene = await persistSceneIfNeeded(sceneDirty || project.activeSceneId !== sceneDraft.id);
      const active = persistedScene ?? sceneDraft;
      const receipt = await startMovieH3Generation({
        projectId: project.id,
        windows: [
          {
            sceneId: active.id,
            label: `${active.name} [${active.window.inSec.toFixed(2)}s..${active.window.outSec.toFixed(2)}s]`,
            windowOverride: null,
            generationOverride: null,
          },
        ],
        expandQualityLadder,
      });
      const reloaded = await getMovie(project.id);
      applyLoadedProject(reloaded, false);
      setNotice(receipt.detail);
    } catch (err) {
      onError(String(err));
    } finally {
      setBusyAction("");
    }
  };

  const handleStopGeneration = async () => {
    if (!project) return;
    try {
      const updated = await cancelMovieRender(project.id);
      applyLoadedProject(updated, true);
      setNotice("Requested H3 queue stop.");
    } catch (err) {
      onError(String(err));
    }
  };

  const handleCaptureGuide = async (source: H3MonitorSource, sourceTimeSec: number, timelineSec: number) => {
    if (!project || !sceneDraft) return;
    setBusyAction("capturing-guide");
    try {
      if (sceneDirty) await persistSceneIfNeeded(true);
      const asset = await captureMovieH3FrameGuide({
        projectId: project.id,
        sceneId: sceneDraft.id,
        sourceJobId: source.sourceKind === "renderedTake" ? source.id : null,
        sourceAssetId: source.sourceKind === "referenceAsset" ? source.id : null,
        timeSec: sourceTimeSec,
        addGuideAtSec: timelineSec,
        label: `Guide @ ${timelineSec.toFixed(2)}s`,
      });
      const [reloaded, assets] = await Promise.all([
        getMovie(project.id),
        listMovieReferenceAssets().catch(() => referenceAssets),
      ]);
      setReferenceAssets(assets);
      applyLoadedProject(reloaded, false);
      setNotice(`Pinned guide frame "${asset.name}" at ${timelineSec.toFixed(2)}s.`);
    } catch (err) {
      onError(String(err));
    } finally {
      setBusyAction("");
    }
  };

  const handleSaveStory = async () => {
    if (!project) return;
    setBusyAction("saving-story");
    try {
      const updated = await saveMovieH3Story({
        projectId: project.id,
        title: storyTitleDraft.trim() || project.title,
        storyMarkdown: storyDraft,
        instruction: storyInstruction.trim() || "Producer revision",
      });
      await refreshSummariesAndAssets();
      applyLoadedProject(updated, false);
      setNotice(`Saved Story Brief revision #${updated.storyRevisions.length}.`);
    } catch (err) {
      onError(String(err));
    } finally {
      setBusyAction("");
    }
  };

  const handlePlaceTake = async () => {
    if (!project || !selectedTakeJobId) return;
    const targetScene = project.scenes.find((s) => s.id === placeSceneId) ?? sceneDraft ?? project.scenes[0];
    if (!targetScene) return;
    setBusyAction("placing-take");
    try {
      if (sceneDirty) await persistSceneIfNeeded(true);
      const updated = await placeMovieH3Take({
        projectId: project.id,
        jobId: selectedTakeJobId,
        assetId: null,
        targetSceneId: targetScene.id,
        placementMode: placeMode,
        targetVideoLaneId: placeDriverLaneId ? null : placeVideoLaneId || null,
        targetAudioLaneId: placeDriverLaneId ? null : placeAudioLaneId || null,
        targetDriverLaneId: placeDriverLaneId || null,
        timelineStartOverrideSec: null,
        durationOverrideSec: null,
      });
      applyLoadedProject(updated, false);
      setNotice(`Placed take onto "${targetScene.name}" (${placeMode}).`);
    } catch (err) {
      onError(String(err));
    } finally {
      setBusyAction("");
    }
  };

  const handleRenderExport = async () => {
    if (!project) return;
    setExporting(true);
    try {
      if (sceneDirty) await persistSceneIfNeeded(true);
      const receipt = await renderMovieEdit({
        projectId: project.id,
        sceneId: exportSceneId || sceneDraft?.id || null,
        preset: exportPreset,
        title: exportTitle.trim() || project.title,
        peakHeadroomDb: exportHeadroomDb,
        destinationPath: null,
      });
      const reloaded = await getMovie(project.id);
      applyLoadedProject(reloaded, false);
      setNotice(`Exported master "${receipt.export.title}" (${receipt.export.durationSeconds.toFixed(1)}s).`);
    } catch (err) {
      onError(String(err));
    } finally {
      setExporting(false);
    }
  };

  const placeTargetScene = project?.scenes.find((s) => s.id === placeSceneId) ?? sceneDraft;

  return (
    <MovieAssetCreator
      production={project?.title ?? (newTitle || "MiniMax H3 Production")}
      onUse={async (asset) => {
        setReferenceAssets((prev) => [asset, ...prev.filter((a) => a.id !== asset.id)]);
      }}
      onRequestImage={onRequestImage}
      onError={onError}
    >
      <div className="movie-studio">
        {/* Left Project Library Column */}
        {showLibrary && (
          <aside className="movie-library" aria-label="Movie productions">
            <header className="movie-library-head">
              <div>
                <span className="eyebrow">MiniMax H3 NLE</span>
                <h2>Productions</h2>
              </div>
              <button
                type="button"
                className="primary-action compact"
                onClick={() => setCreating(true)}
              >
                <Plus size={14} /> New
              </button>
            </header>

            {loading ? (
              <p className="muted"><LoaderCircle size={14} className="spin" /> Loading studio…</p>
            ) : summaries.length === 0 ? (
              <p className="muted">No H3 productions yet. Create your first multi-scene production.</p>
            ) : (
              <PagedList
                items={summaries}
                itemKey={(item) => item.id}
                label="Production list"
                selectedKey={!creating ? project?.id : null}
                renderItem={(item) => (
                  <button
                    key={item.id}
                    type="button"
                    className={`movie-library-item ${!creating && project?.id === item.id ? "active" : ""}`}
                    onClick={() => void handleSelectProject(item.id)}
                  >
                    <strong>{item.title}</strong>
                    <small>{item.status} · {new Date(item.updatedAt).toLocaleDateString()}</small>
                  </button>
                )}
              />
            )}
          </aside>
        )}

        {/* Main Workspace Column */}
        <div className="movie-workspace">
          {/* Top Bar */}
          <header className="movie-topbar">
            <div className="movie-topbar-title">
              <button
                type="button"
                className="quiet-button compact"
                aria-label="Toggle production library"
                onClick={() => setShowLibrary((v) => !v)}
              >
                <PanelLeft size={15} />
              </button>
              <div>
                <h1>{creating || !project ? "New MiniMax H3 Production" : project.title}</h1>
                {project && !creating && (
                  <span className="movie-status-line">
                    <span className={`h3-status-pill status-${project.status}`}>{project.status}</span>
                    {sceneDirty && <span className="h3-dirty-pill">Unsaved scene edits</span>}
                    <small>{project.detail}</small>
                  </span>
                )}
              </div>
            </div>

            {/* H3 Environment & Turbo LoRA Status */}
            <div className="movie-env-bar">
              {environment ? (
                <>
                  <span
                    className={`h3-env-badge ${environment.ready ? "ready" : "missing"}`}
                    title={`ComfyUI: ${environment.comfyRoot || initialComfyRoot || "Unset"} — ${environment.detail}`}
                  >
                    <Cpu size={13} />
                    {environment.ready ? "H3 Weights Ready" : "H3 Weights Incomplete"}
                    {environment.hasTaeh3Preview ? " · TAEH3" : ""}
                  </span>
                  {environment.hasRef2vTurboLora && environment.hasFl2vTurboLora ? (
                    <span className="h3-env-badge ready" title="Ref2V + FL2V Turbo LoRAs installed">
                      <CheckCircle2 size={13} /> Turbo LoRAs Ready
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="quiet-button compact"
                      disabled={installingTurbo}
                      onClick={() => void handleInstallTurboLoras()}
                    >
                      {installingTurbo ? <LoaderCircle size={13} className="spin" /> : <Download size={13} />}
                      Install H3 Turbo LoRAs
                    </button>
                  )}
                </>
              ) : null}

              {project && !creating && (
                <button
                  type="button"
                  className="quiet-button compact"
                  onClick={() => void revealMovie(project.id).catch((e) => onError(String(e)))}
                >
                  <FolderOpen size={14} /> Folder
                </button>
              )}
            </div>
          </header>

          {notice && (
            <div className="movie-notice-banner" role="status">
              <span>{notice}</span>
              <button type="button" className="quiet-button compact" aria-label="Dismiss notice" onClick={() => setNotice("")}>
                <X size={13} />
              </button>
            </div>
          )}

          {/* Active Local LLM Collaborator Sheet */}
          {draftSession && (
            <section className="movie-collaborator-sheet" aria-label="Local Studio Collaborator">
              <header>
                <strong><WandSparkles size={15} /> Local Collaborator · {draftSession.label} ({draftSession.modelName})</strong>
                <div className="h3-inline-actions">
                  {["starting", "thinking", "writing"].includes(draftSession.status) && (
                    <button
                      type="button"
                      className="quiet-button compact"
                      onClick={() => void cancelStudioPromptDraft(draftSession.id)}
                    >
                      <CircleStop size={13} /> Stop
                    </button>
                  )}
                  <button
                    type="button"
                    className="primary-action compact"
                    disabled={!draftSession.text.trim()}
                    onClick={() => {
                      draftSession.apply(draftSession.text.trim());
                      setDraftSession(undefined);
                    }}
                  >
                    <CheckCircle2 size={13} /> Apply Draft
                  </button>
                  <button
                    type="button"
                    className="quiet-button compact"
                    onClick={() => setDraftSession(undefined)}
                  >
                    Dismiss
                  </button>
                </div>
              </header>
              <div className="movie-collaborator-pages">
                <ModelReplyPages
                  label={draftSession.label}
                  text={draftSession.text}
                  reasoning={draftSession.reasoning}
                  live={["starting", "thinking", "writing"].includes(draftSession.status)}
                  modelName={draftSession.modelName}
                  thinkingLevel={draftSession.thinkingLevel}
                  request={advancedEnabled && draftSession.receipt ? JSON.stringify(draftSession.receipt.exactRequest, null, 2) : undefined}
                />
              </div>
            </section>
          )}

          {/* New Production Launcher OR Active Multi-Scene Studio */}
          {creating || !project ? (
            <section className="movie-create-card" aria-label="Create MiniMax H3 production">
              <header>
                <h2><Clapperboard size={18} /> Start a MiniMax H3 Reference Production</h2>
                <p className="muted">
                  Build reusable <code>@Handle</code> characters, locations, props, voices, and sounds; stage them across timeline windows; and run Sonder’s 3-pass quality ladder (<code>0.2 MP</code> → <code>1 MP</code> → <code>1080p</code>) with joint 48 kHz audio.
                </p>
              </header>

              <div className="h3-form-grid">
                <label>
                  Production Title
                  <input
                    aria-label="Production title"
                    value={newTitle}
                    onChange={(e) => setNewTitle(e.target.value)}
                    placeholder="The Frog Who Measured the Sky"
                  />
                </label>

                <div className="h3-row-2">
                  <label>
                    Master Scene Canvas
                    <select
                      aria-label="Master scene canvas"
                      value={newCanvasKey}
                      onChange={(e) => setNewCanvasKey(e.target.value)}
                    >
                      {CANVAS_PRESETS.map((preset) => (
                        <option key={preset.key} value={preset.key}>{preset.label}</option>
                      ))}
                    </select>
                  </label>
                  <label>
                    Initial Scene Duration (s)
                    <input
                      aria-label="Initial scene duration"
                      type="number"
                      min={2}
                      max={600}
                      value={newDurationSec}
                      onChange={(e) => setNewDurationSec(Math.max(2, Number(e.target.value) || 15))}
                    />
                  </label>
                </div>

                <label>
                  Story Brief &amp; Cast Notes (Markdown)
                  <textarea
                    aria-label="Production story brief"
                    rows={7}
                    value={newStory}
                    onChange={(e) => setNewStory(e.target.value)}
                    placeholder="Describe the story arc, visual world, @Characters, @Locations, and sonic atmosphere…"
                  />
                </label>

                <div className="h3-inline-actions">
                  <button
                    type="button"
                    className="quiet-button compact"
                    onClick={() => void startLocalDraft(
                      "story",
                      "Production Story Brief",
                      newStory,
                      newTitle || "New H3 Production",
                      "story",
                      (text) => setNewStory(text),
                    )}
                  >
                    <Sparkles size={13} /> Draft Brief with Local LLM
                  </button>

                  <ExternalCollaborationExchange
                    title="External AI Story Brief"
                    summary="Copy/paste structured story brief request"
                    buildRequest={() =>
                      buildExternalCollaborationRequest({
                        target: "movie-brief",
                        role: "MiniMax H3 Story Producer",
                        instructions: [
                          "Draft a concise multi-shot story brief with clear @Character and @Location handles for MiniMax H3.",
                          "Highlight sonic atmosphere and character voices.",
                        ],
                        context: {
                          title: newTitle || "New H3 Production",
                          currentDraft: newStory,
                        },
                        resultTemplate: { text: "Complete Markdown story brief with @Handle definitions." },
                      })
                    }
                    parseResponse={(raw) => parseExternalTextResult(raw, "movie-brief")}
                    onApply={(text) => setNewStory(text)}
                  />
                </div>
              </div>

              <footer className="h3-inline-actions">
                <button
                  type="button"
                  className="primary-action"
                  disabled={busyAction === "creating-project"}
                  onClick={() => void handleCreateProject()}
                >
                  {busyAction === "creating-project" ? <LoaderCircle size={15} className="spin" /> : <Clapperboard size={15} />}
                  Create H3 Production
                </button>
                {project && (
                  <button type="button" className="quiet-button" onClick={() => setCreating(false)}>
                    Cancel
                  </button>
                )}
              </footer>
            </section>
          ) : (
            <>
              {/* Multi-Scene Switcher + Room Navigation */}
              <div className="movie-nav-strip">
                <div className="movie-scene-tabs" role="tablist" aria-label="Production scenes">
                  {project.scenes.map((sc) => (
                    <button
                      key={sc.id}
                      type="button"
                      role="tab"
                      aria-selected={sceneDraft?.id === sc.id}
                      className={`movie-scene-tab ${sceneDraft?.id === sc.id ? "active" : ""}`}
                      onClick={() => void handleSwitchScene(sc.id)}
                    >
                      <span>{sc.name}</span>
                      <small>{sc.width}×{sc.height}</small>
                    </button>
                  ))}
                  <button
                    type="button"
                    className="quiet-button compact"
                    disabled={busyAction === "adding-scene"}
                    title="Add a full 1920×1088 cinema scene"
                    onClick={() => void handleAddScene("standard")}
                  >
                    <Plus size={12} /> Scene
                  </button>
                  <button
                    type="button"
                    className="quiet-button compact"
                    disabled={busyAction === "adding-scene"}
                    title="Add a fast 960×544 character & blocking audition scene"
                    onClick={() => void handleAddScene("audition")}
                  >
                    <Plus size={12} /> Audition
                  </button>
                  <button
                    type="button"
                    className="quiet-button compact"
                    disabled={busyAction === "adding-scene"}
                    title="Add a tiny 64×32 soundtrack-only scene for fast standalone H3 score/ambience"
                    onClick={() => void handleAddScene("soundtrack")}
                  >
                    <Plus size={12} /> 64×32 Soundtrack
                  </button>
                </div>

                <nav className="movie-room-tabs" aria-label="Studio rooms">
                  <button
                    type="button"
                    className={room === "studio" ? "active" : ""}
                    onClick={() => setRoom("studio")}
                  >
                    <Film size={14} /> Timeline &amp; Compiler
                  </button>
                  <button
                    type="button"
                    className={room === "references" ? "active" : ""}
                    onClick={() => setRoom("references")}
                  >
                    <Users size={14} /> @Handle Library ({project.entities.length})
                  </button>
                  <button
                    type="button"
                    className={room === "story" ? "active" : ""}
                    onClick={() => setRoom("story")}
                  >
                    <BookOpen size={14} /> Story Brief
                  </button>
                  <button
                    type="button"
                    className={room === "takes" ? "active" : ""}
                    onClick={() => setRoom("takes")}
                  >
                    <Layers size={14} /> Takes &amp; Telemetry ({project.renderJobs.length})
                  </button>
                  <button
                    type="button"
                    className={room === "deliver" ? "active" : ""}
                    onClick={() => setRoom("deliver")}
                  >
                    <Download size={14} /> Deliver ({project.exports.length})
                  </button>
                </nav>
              </div>

              {/* Room 1: Timeline & Compiler */}
              {room === "studio" && sceneDraft && (
                <div className="h3-nle-workbench">
                  <div className="h3-upper-deck">
                    <H3ProgramMonitor
                      sceneName={sceneDraft.name}
                      sceneWidth={sceneDraft.width}
                      sceneHeight={sceneDraft.height}
                      playheadSec={playheadSec}
                      durationSec={sceneDraft.durationSec}
                      activeClip={activeMonitorClip}
                      completedTakes={completedTakes}
                      rendering={rendering}
                      projectId={project.id}
                      disabled={Boolean(busyAction)}
                      onPlayhead={setPlayheadSec}
                      onCaptureGuide={(source, srcTime, tlSec) => void handleCaptureGuide(source, srcTime, tlSec)}
                      onStopRender={() => void handleStopGeneration()}
                    />

                    <MovieGenerationPanel
                      project={project}
                      scene={sceneDraft}
                      environment={environment}
                      selectedShotId={selectedShotId}
                      compileResult={compileResult}
                      rendering={rendering}
                      disabled={Boolean(busyAction)}
                      onSceneChange={handleSceneChange}
                      onSelectShot={setSelectedShotId}
                      onStartGeneration={handleStartGeneration}
                      onStopGeneration={() => void handleStopGeneration()}
                      onDraftDirection={(currentText, shotLabel, apply) =>
                        void startLocalDraft("imageAsset", `Shot Action · ${shotLabel}`, currentText, shotLabel, "shot", apply)
                      }
                      onPersistSceneIfNeeded={async () => {
                        await persistSceneIfNeeded(false);
                      }}
                      onProjectRefresh={async () => {
                        const reloaded = await getMovie(project.id);
                        applyLoadedProject(reloaded, false);
                      }}
                      onError={onError}
                      onNotice={(msg) => setNotice(msg)}
                    />
                  </div>

                  <MovieTimeline
                    key={`${project.id}:${sceneDraft.id}`}
                    project={project}
                    scene={sceneDraft}
                    savedScene={savedScene}
                    playheadSec={playheadSec}
                    compileResult={compileResult}
                    selectedShotId={selectedShotId}
                    disabled={rendering || Boolean(busyAction)}
                    dirty={sceneDirty}
                    onSceneChange={handleSceneChange}
                    onSaveScene={() => void handleSaveScene()}
                    onPlayhead={setPlayheadSec}
                    onSelectShot={setSelectedShotId}
                  />
                </div>
              )}

              {/* Room 2: @Handle Entity Reference Library */}
              {room === "references" && sceneDraft && (
                <H3EntityLibrary
                  project={project}
                  scene={sceneDraft}
                  disabled={rendering}
                  onPersistSceneIfNeeded={async () => {
                    await persistSceneIfNeeded(false);
                  }}
                  onProject={(next) => {
                    applyLoadedProject(next, false);
                    void refreshSummariesAndAssets();
                  }}
                  onDraftDescription={(currentText, handle, apply) =>
                    void startLocalDraft(
                      "referenceDescription",
                      `@${handle} Subject Description`,
                      currentText,
                      `@${handle}`,
                      "character",
                      apply,
                    )
                  }
                  onError={onError}
                  onNotice={(msg) => setNotice(msg)}
                />
              )}

              {/* Room 3: Story Brief & Local Collaborator Controls */}
              {room === "story" && (
                <section className="h3-story-room" aria-label="Story brief">
                  <div className="h3-story-editor">
                    <header className="h3-section-head">
                      <div>
                        <h3>Story Brief &amp; Cast Bible</h3>
                        <small>Durable Markdown brief shared with both Local Collaborator and Computer Use Producer Agent.</small>
                      </div>
                      <button
                        type="button"
                        className="primary-action compact"
                        disabled={busyAction === "saving-story"}
                        onClick={() => void handleSaveStory()}
                      >
                        <Save size={14} /> Save Story Revision
                      </button>
                    </header>

                    <div className="h3-row-2">
                      <label>
                        Production Title
                        <input
                          aria-label="Edit production title"
                          value={storyTitleDraft}
                          onChange={(e) => { setStoryTitleDraft(e.target.value); setStoryDirty(true); }}
                        />
                      </label>
                      <label>
                        Revision Note
                        <input
                          aria-label="Story revision note"
                          value={storyInstruction}
                          onChange={(e) => setStoryInstruction(e.target.value)}
                          placeholder="Refined @Pip character arc and scene beats"
                        />
                      </label>
                    </div>

                    <label>
                      Story Markdown
                      <textarea
                        aria-label="Story brief markdown"
                        rows={14}
                        value={storyDraft}
                        onChange={(e) => { setStoryDraft(e.target.value); setStoryDirty(true); }}
                      />
                    </label>

                    <div className="h3-inline-actions">
                      <button
                        type="button"
                        className="quiet-button compact"
                        onClick={() =>
                          void startLocalDraft(
                            "story",
                            "Story Brief",
                            storyDraft,
                            storyTitleDraft || project.title,
                            "story",
                            (text) => { setStoryDraft(text); setStoryDirty(true); },
                          )
                        }
                      >
                        <Sparkles size={13} /> {draftMode === "continue" ? "Continue with Local LLM" : "Develop with Local LLM"}
                      </button>

                      <ExternalCollaborationExchange
                        title="External AI Story Brief"
                        summary="Copy/paste structured story brief revision"
                        buildRequest={() =>
                          buildExternalCollaborationRequest({
                            target: "movie-brief",
                            role: "MiniMax H3 Story Producer",
                            instructions: [
                              "Refine this MiniMax H3 story brief with clear @Character and @Location handles and vivid sonic cues.",
                            ],
                            context: {
                              title: storyTitleDraft || project.title,
                              currentDraft: storyDraft,
                            },
                            resultTemplate: { text: "Revised Markdown story brief." },
                          })
                        }
                        parseResponse={(raw) => parseExternalTextResult(raw, "movie-brief")}
                        onApply={(text) => {
                          setStoryDraft(text);
                          setStoryDirty(true);
                        }}
                      />
                    </div>
                  </div>

                  <aside className="h3-story-sidebar">
                    <div className="h3-card">
                      <h4>Local LLM Collaborator Policy</h4>
                      <label>
                        Model
                        <select
                          aria-label="Collaborator model"
                          value={modelId}
                          onChange={(e) => setModelId(e.target.value)}
                        >
                          {models.length === 0 && <option value="">No GGUF models scanned</option>}
                          {models.map((m) => (
                            <option key={m.id} value={m.id}>{m.name} ({m.quantization})</option>
                          ))}
                        </select>
                      </label>
                      <div className="h3-row-2">
                        <label>
                          Mode
                          <select
                            aria-label="Collaborator draft mode"
                            value={draftMode}
                            onChange={(e) => setDraftMode(e.target.value as PromptDraftMode)}
                          >
                            <option value="develop">Develop / Rewrite</option>
                            <option value="continue">Continue Writing</option>
                          </select>
                        </label>
                        <label>
                          Thinking
                          <select
                            aria-label="Collaborator thinking level"
                            value={thinkingLevel}
                            onChange={(e) => setThinkingLevel(e.target.value as ThinkingLevel | "default")}
                          >
                            <option value="default">Model Default</option>
                            <option value="off">Off</option>
                            <option value="low">Low</option>
                            <option value="medium">Medium</option>
                            <option value="high">High</option>
                          </select>
                        </label>
                      </div>
                      <ModelRuntimePolicyControls
                        value={activeRuntimePolicy}
                        inherited={inheritedPolicy}
                        disabled={false}
                        expert={advancedEnabled}
                        fixedContextWindow={selectedModel?.fixedContextWindow}
                        scope="Movie collaborator"
                        onChange={setRuntimePolicyOverride}
                        onReset={() => setRuntimePolicyOverride(undefined)}
                      />
                    </div>

                    <div className="h3-card">
                      <h4>Story Revisions ({project.storyRevisions.length})</h4>
                      {project.storyRevisions.length === 0 ? (
                        <p className="muted">No saved revisions yet.</p>
                      ) : (
                        <ul className="h3-revision-list">
                          {[...project.storyRevisions].reverse().map((rev) => (
                            <li key={rev.id}>
                              <div>
                                <strong>#{rev.number} · {rev.origin}</strong>
                                <small>{rev.instruction || "Initial brief"}</small>
                              </div>
                              <button
                                type="button"
                                className="quiet-button compact"
                                onClick={() => {
                                  setStoryDraft(rev.markdown);
                                  setStoryDirty(true);
                                }}
                              >
                                Load
                              </button>
                            </li>
                          ))}
                        </ul>
                      )}
                    </div>
                  </aside>
                </section>
              )}

              {/* Room 4: Takes & Telemetry */}
              {room === "takes" && (
                <section className="h3-takes-room" aria-label="H3 takes and telemetry">
                  <aside className="h3-takes-list">
                    <header className="h3-section-head">
                      <h3>Render Queue &amp; Takes</h3>
                      <small>{project.renderJobs.length} total</small>
                    </header>
                    {project.renderJobs.length === 0 ? (
                      <p className="muted">Queue a window or 3-pass ladder from Timeline &amp; Compiler to inspect takes here.</p>
                    ) : (
                      <ul className="h3-job-list">
                        {[...project.renderJobs].reverse().map((job) => (
                          <li key={job.id}>
                            <button
                              type="button"
                              className={`h3-job-card ${selectedTakeJobId === job.id ? "active" : ""}`}
                              onClick={() => setSelectedTakeJobId(job.id)}
                            >
                              <div className="h3-job-row">
                                <strong>{job.label}</strong>
                                <span className={`h3-status-pill status-${job.status}`}>{job.status}</span>
                              </div>
                              <small>
                                {job.passPreset} · {job.width}×{job.height} · {job.steps} steps · denoise {job.denoise}
                              </small>
                            </button>
                          </li>
                        ))}
                      </ul>
                    )}
                  </aside>

                  <div className="h3-take-inspector">
                    {takeInspecting ? (
                      <p className="muted"><LoaderCircle size={15} className="spin" /> Inspecting take telemetry…</p>
                    ) : !takeInspection ? (
                      <p className="muted">Select a render job on the left to inspect its media, 3 contact-sheet frames, audio dBFS, color-drift matrix, and compiled prompt.</p>
                    ) : (
                      <>
                        <header className="h3-section-head">
                          <div>
                            <h3>{takeInspection.label}</h3>
                            <small>
                              Model: <code>{takeInspection.resolvedModel || "pending"}</code> · Task: <code>{takeInspection.taskMode}</code> · Seed: <code>{String(takeInspection.seed)}</code>
                            </small>
                          </div>
                          <span className={`h3-status-pill status-${takeInspection.status}`}>{takeInspection.status}</span>
                        </header>

                        {takeInspection.error && (
                          <div className="h3-advisory-box" role="alert">
                            <AlertTriangle size={14} /> {takeInspection.error}
                          </div>
                        )}

                        {takeInspection.takeMediaPath && movieMediaUrl(takeInspection.takeMediaPath) && (
                          <div className="h3-take-player">
                            <video
                              controls
                              playsInline
                              src={movieMediaUrl(takeInspection.takeMediaPath)}
                            />
                          </div>
                        )}

                        {/* 3 Contact-Sheet Stills */}
                        {takeInspection.quality && takeInspection.quality.contactSheetPaths.length > 0 && (
                          <div className="h3-contact-sheet">
                            {takeInspection.quality.contactSheetPaths.map((stillPath, idx) => {
                              const url = movieMediaUrl(stillPath);
                              return (
                                <figure key={stillPath}>
                                  {url ? <img src={url} alt={`Take frame ${idx + 1}`} /> : <div className="h3-still-placeholder">Still {idx + 1}</div>}
                                  <figcaption>{["20% Early", "50% Mid", "85% Late"][idx] ?? `Frame ${idx + 1}`}</figcaption>
                                </figure>
                              );
                            })}
                          </div>
                        )}

                        {/* Telemetry Cards */}
                        <div className="h3-telemetry-grid">
                          {takeInspection.quality && (
                            <div className="h3-card">
                              <h4>Media &amp; Audio Quality</h4>
                              <p>
                                <strong>{takeInspection.quality.width}×{takeInspection.quality.height}</strong> @ {takeInspection.quality.fps} fps ·{" "}
                                {takeInspection.quality.frameCount} frames ({takeInspection.quality.durationSec.toFixed(2)}s)
                              </p>
                              <p>
                                Audio: {takeInspection.quality.hasAudio
                                  ? `Peak ${takeInspection.quality.audioPeakDbfs.toFixed(1)} dBFS · RMS ${takeInspection.quality.audioRmsDbfs.toFixed(1)} dBFS${takeInspection.quality.audioIsSilent ? " (Silent)" : ""}`
                                  : "No audio stream"}
                              </p>
                              {takeInspection.quality.warnings.length > 0 && (
                                <ul className="h3-warning-list">
                                  {takeInspection.quality.warnings.map((w, idx) => <li key={idx}><AlertTriangle size={12} /> {w}</li>)}
                                </ul>
                              )}
                            </div>
                          )}

                          {takeInspection.colorDrift && (
                            <div className="h3-card">
                              <h4>Ridge Color-Drift Cancellation</h4>
                              <p>
                                Status: <strong>{takeInspection.colorDrift.applied ? "Applied" : "Bypassed"}</strong> ·{" "}
                                Matched context frames: {takeInspection.colorDrift.matchedContextFrames}
                              </p>
                              <p>
                                Max bias: {takeInspection.colorDrift.maxBiasAbs.toFixed(4)} · Matrix dev: {takeInspection.colorDrift.maxMatrixDeviation.toFixed(4)}
                              </p>
                              <small>{takeInspection.colorDrift.summary}</small>
                            </div>
                          )}
                        </div>

                        {/* Place Take onto Any Scene / Lane */}
                        {takeInspection.status === "complete" && (
                          <div className="h3-card">
                            <h4>Place Take onto Timeline</h4>
                            <div className="h3-row-4">
                              <label>
                                Target Scene
                                <select
                                  aria-label="Place take target scene"
                                  value={placeSceneId}
                                  onChange={(e) => setPlaceSceneId(e.target.value)}
                                >
                                  {project.scenes.map((s) => <option key={s.id} value={s.id}>{s.name}</option>)}
                                </select>
                              </label>
                              <label>
                                Trim Mode
                                <select
                                  aria-label="Place take trim mode"
                                  value={placeMode}
                                  onChange={(e) => setPlaceMode(e.target.value as H3TakePlacementMode)}
                                >
                                  <option value="trimmed">Trimmed [In, Out] Only</option>
                                  <option value="untrimmed">Untrimmed (Include Context)</option>
                                </select>
                              </label>
                              <label>
                                Video Lane
                                <select
                                  aria-label="Place take video lane"
                                  value={placeVideoLaneId}
                                  onChange={(e) => setPlaceVideoLaneId(e.target.value)}
                                >
                                  <option value="">Top Video Lane (Default)</option>
                                  {placeTargetScene?.videoLanes.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}
                                </select>
                              </label>
                              <label>
                                Audio Lane
                                <select
                                  aria-label="Place take audio lane"
                                  value={placeAudioLaneId}
                                  disabled={Boolean(placeDriverLaneId)}
                                  onChange={(e) => setPlaceAudioLaneId(e.target.value)}
                                >
                                  <option value="">Top Audio Lane (Default)</option>
                                  {placeTargetScene?.audioLanes.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}
                                </select>
                              </label>
                              <label>
                                Driver Lane (D1..Dn)
                                <select
                                  aria-label="Place take driver lane"
                                  value={placeDriverLaneId}
                                  onChange={(e) => setPlaceDriverLaneId(e.target.value)}
                                >
                                  <option value="">None (Use Video/Audio)</option>
                                  {placeTargetScene?.driverLanes.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}
                                </select>
                              </label>
                            </div>
                            <button
                              type="button"
                              className="primary-action compact"
                              disabled={busyAction === "placing-take"}
                              onClick={() => void handlePlaceTake()}
                            >
                              <Layers size={14} /> Place Take on Timeline
                            </button>
                          </div>
                        )}

                        <details className="h3-card">
                          <summary>Compiled 6-Channel Prompt</summary>
                          <pre className="h3-compiled-pre">{takeInspection.compiledPrompt}</pre>
                        </details>
                      </>
                    )}
                  </div>
                </section>
              )}

              {/* Room 5: Deliver / Export */}
              {room === "deliver" && (
                <section className="h3-deliver-room" aria-label="Deliver and export">
                  <div className="h3-card">
                    <header className="h3-section-head">
                      <div>
                        <h3>Render Non-Destructive Master</h3>
                        <small>Composites enabled video lanes, mixes multi-track 48 kHz audio with peak headroom protection, and tags BT.709 color.</small>
                      </div>
                      <button
                        type="button"
                        className="quiet-button compact"
                        onClick={() => void revealMovie(project.id).catch((e) => onError(String(e)))}
                      >
                        <FolderOpen size={14} /> Open Production Folder
                      </button>
                    </header>

                    <div className="h3-row-2">
                      <label>
                        Source Scene
                        <select
                          aria-label="Export source scene"
                          value={exportSceneId}
                          onChange={(e) => setExportSceneId(e.target.value)}
                        >
                          {project.scenes.map((sc) => (
                            <option key={sc.id} value={sc.id}>{sc.name} ({sc.width}×{sc.height})</option>
                          ))}
                        </select>
                      </label>
                      <label>
                        Export Title
                        <input
                          aria-label="Export title"
                          value={exportTitle}
                          onChange={(e) => setExportTitle(e.target.value)}
                        />
                      </label>
                    </div>

                    <div className="h3-row-2">
                      <label>
                        Master Preset
                        <select
                          aria-label="Export preset"
                          value={exportPreset}
                          onChange={(e) => setExportPreset(e.target.value)}
                        >
                          {EXPORT_PRESETS.map((p) => (
                            <option key={p.value} value={p.value}>{p.label}</option>
                          ))}
                        </select>
                      </label>
                      <label>
                        Audio Peak Ceiling (dBFS)
                        <input
                          aria-label="Audio peak headroom dBFS"
                          type="number"
                          min={-20}
                          max={0}
                          step={0.5}
                          value={exportHeadroomDb}
                          onChange={(e) => setExportHeadroomDb(Math.min(0, Math.max(-20, Number(e.target.value) || -1)))}
                        />
                      </label>
                    </div>

                    <button
                      type="button"
                      className="primary-action"
                      disabled={exporting}
                      onClick={() => void handleRenderExport()}
                    >
                      {exporting ? <LoaderCircle size={15} className="spin" /> : <Download size={15} />}
                      Render Master
                    </button>
                  </div>

                  <div className="h3-card">
                    <h3>Published Masters ({project.exports.length})</h3>
                    {project.exports.length === 0 ? (
                      <p className="muted">No masters exported yet.</p>
                    ) : (
                      <ul className="h3-export-list">
                        {[...project.exports].reverse().map((exp) => {
                          const url = movieMediaUrl(exp.path);
                          return (
                            <li key={exp.id} className="h3-export-item">
                              <div>
                                <strong>{exp.title}</strong>
                                <small>
                                  {exp.preset} · {exp.durationSeconds.toFixed(1)}s · {(exp.bytes / (1024 * 1024)).toFixed(2)} MiB · SHA-256 {exp.sha256.slice(0, 12)}…
                                </small>
                              </div>
                              {url && (
                                exp.preset === "audio_flac"
                                  ? <audio controls src={url} />
                                  : <video controls playsInline src={url} className="h3-export-video" />
                              )}
                            </li>
                          );
                        })}
                      </ul>
                    )}
                  </div>
                </section>
              )}
            </>
          )}
        </div>
      </div>
    </MovieAssetCreator>
  );
}
