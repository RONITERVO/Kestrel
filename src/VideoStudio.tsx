import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  CircleStop,
  Clock3,
  Cpu,
  Film,
  FolderOpen,
  Gauge,
  HardDrive,
  ImagePlus,
  Layers3,
  LoaderCircle,
  MonitorUp,
  Play,
  RefreshCw,
  Save,
  Settings2,
  ShieldCheck,
  Sparkles,
  WandSparkles,
  Video,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  getVideoProject,
  getVideoReferencePreview,
  getVideoSnapshot,
  importVideoReference,
  onVideoProjectEvent,
  pickComfyRoot,
  planVideoProject,
  revealVideoProject,
  saveVideoSettings,
  startVideoProject,
  stopVideoBackend,
  stopVideoProject,
  setVideoClipReference,
  setVideoChapterReference,
  setVideoContinuity,
  updateVideoClipPrompt,
} from "./api";
import type {
  ControlSnapshot,
  VideoBoundarySettings,
  VideoContinuityMode,
  VideoPlanRequest,
  VideoPreset,
  VideoPresetStatus,
  VideoProject,
  VideoProjectEvent,
  VideoSnapshot,
} from "./types";

const presetCopy: Record<VideoPreset, { kicker: string; description: string; quality: string }> = {
  "wan-1.3b-gpu-only": {
    kicker: "Fast, deterministic",
    description: "No CPU offload. A job that exceeds VRAM fails instead of changing its timing policy.",
    quality: "Medium quality · 30 steps · 2-second native clips",
  },
  "wan-vace-1.3b-reference": {
    kicker: "Continuity and motion studio",
    description: "Native image and control-video conditioning for subject anchors, storyboards, and motion transfer.",
    quality: "Reference-first · 30 steps · 5-second native clips",
  },
  "kandinsky-distilled": {
    kicker: "Recommended daily driver",
    description: "Fast 16-step Kandinsky with declared stage boundaries and a resident sampling model.",
    quality: "High quality · 16 steps · 5-second native clips",
  },
  "kandinsky-sft": {
    kicker: "Maximum quality",
    description: "The 100-step SFT workflow for hero shots and quality-first sequences.",
    quality: "Maximum quality · 100 steps · 5-second native clips",
  },
  "wan-2.2-5b-offload": {
    kicker: "Flexible all-rounder",
    description: "Forced low-VRAM loading with two known asynchronous transfer streams.",
    quality: "High quality · 20 steps · 3-second native clips",
  },
};

const runtimePolicy: Record<VideoPreset, { profile: string; offloading: string }> = {
  "wan-1.3b-gpu-only": { profile: "gpu-only", offloading: "forbidden" },
  "wan-vace-1.3b-reference": { profile: "reference-staged", offloading: "stage-boundary-only" },
  "kandinsky-distilled": { profile: "kandinsky-staged", offloading: "stage-boundary-only" },
  "kandinsky-sft": { profile: "kandinsky-staged", offloading: "stage-boundary-only" },
  "wan-2.2-5b-offload": { profile: "forced-offload", offloading: "forced" },
};

const defaultBoundaries: VideoBoundarySettings = {
  maxClips: 500,
  maxRetriesPerClip: 2,
  maxFailedClips: 3,
  maxRuntimeMinutes: 720,
  minFreeDiskGib: 20,
  assembleFinalVideo: true,
};

export function estimateClipCount(totalSeconds: number, nativeClipSeconds: number): number {
  if (!Number.isFinite(totalSeconds) || totalSeconds <= 0 || nativeClipSeconds <= 0) return 0;
  return Math.ceil(totalSeconds / nativeClipSeconds);
}

export function VideoStudio({ control, onError }: { control: ControlSnapshot; onError: (message: string) => void }) {
  const [snapshot, setSnapshot] = useState<VideoSnapshot | null>(null);
  const [project, setProject] = useState<VideoProject | null>(null);
  const [events, setEvents] = useState<VideoProjectEvent[]>([]);
  const [busy, setBusy] = useState<"loading" | "planning" | "saving" | "starting" | null>("loading");
  const [showSetup, setShowSetup] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [audience, setAudience] = useState("General viewers");
  const [useCase, setUseCase] = useState("Artistic short");
  const [preset, setPreset] = useState<VideoPreset>("kandinsky-distilled");
  const [orientation, setOrientation] = useState<"landscape" | "portrait" | "square">("landscape");
  const [hours, setHours] = useState(0);
  const [minutes, setMinutes] = useState(1);
  const [seconds, setSeconds] = useState(0);
  const [negativePrompt, setNegativePrompt] = useState("blurry, low quality, static, distorted anatomy, text, subtitles, captions, logos, watermark");
  const [plannerModelId, setPlannerModelId] = useState(control.settings.selectedModelId ?? control.models[0]?.id ?? "");
  const [boundaries, setBoundaries] = useState(defaultBoundaries);

  const refresh = useCallback(async () => {
    try {
      const next = await getVideoSnapshot();
      setSnapshot(next);
      if (project) {
        try {
          setProject(await getVideoProject(project.id));
        } catch {
          // Browser preview plans are intentionally not durable.
        }
      }
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  }, [onError, project?.id]);

  useEffect(() => {
    void refresh();
    let dispose: (() => void) | undefined;
    void onVideoProjectEvent((event) => {
      setEvents((items) => [...items, event].slice(-16));
      if (!project || project.id === event.projectId) {
        void getVideoProject(event.projectId).then(setProject).catch(() => undefined);
      }
      void getVideoSnapshot().then(setSnapshot).catch(() => undefined);
    }).then((unlisten) => { dispose = unlisten; });
    return () => dispose?.();
  }, [refresh, project?.id]);

  useEffect(() => {
    if (!plannerModelId && control.models[0]) setPlannerModelId(control.models[0].id);
  }, [control.models, plannerModelId]);

  const selectedPreset = useMemo(
    () => snapshot?.presets.find((item) => item.id === preset),
    [preset, snapshot?.presets],
  );
  const totalSeconds = Math.max(0, hours * 3600 + minutes * 60 + seconds);
  const estimatedClips = estimateClipCount(totalSeconds, selectedPreset?.nativeClipSeconds ?? 5);
  const boundaryExceeded = estimatedClips > boundaries.maxClips;
  const running = project ? ["starting", "running", "verifying", "assembling"].includes(project.status) : false;

  const chooseProject = async (id: string) => {
    try {
      setProject(await getVideoProject(id));
      setEvents([]);
    } catch (cause) {
      onError(String(cause));
    }
  };

  const plan = async () => {
    const request: VideoPlanRequest = {
      prompt,
      audience,
      useCase,
      plannerModelId: plannerModelId || undefined,
      preset,
      totalDurationSeconds: totalSeconds,
      orientation,
      negativePrompt,
      boundaries,
    };
    setBusy("planning");
    setEvents([]);
    try {
      const next = await planVideoProject(request);
      setProject(next);
      await refresh();
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  const start = async () => {
    if (!project) return;
    const policy = snapshot?.presets.find((item) => item.id === project.preset);
    const accepted = window.confirm(
      `Start ${project.clips.length.toLocaleString()} serial generations with ${presetCopy[project.preset].kicker}?\n\nOffloading: ${policy?.offloading ?? "unknown"}\nMemory profile: ${policy?.profile ?? "unknown"}\n\nKestrel will stop the local planning model, claim ComfyUI port 8188, and refuse an unowned backend whose policy cannot be proven.`,
    );
    if (!accepted) return;
    setBusy("starting");
    try {
      await startVideoProject(project.id);
      setProject({ ...project, status: "starting" });
      await refresh();
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  if (!snapshot) {
    return <div className="video-loading"><LoaderCircle className="spin" /><span>Inspecting the offline video backend…</span></div>;
  }

  return (
    <section className="video-studio">
      <header className="video-hero">
        <div>
          <span className="eyebrow">Offline production agent</span>
          <h1>Video Studio</h1>
          <p>Turn one prompt into a reviewed story bible, a bounded serial queue, verified clips, and a recoverable final film—without an online service or hidden runtime decisions.</p>
        </div>
        <div className={`video-backend-card ${snapshot.backend.predictable ? "ready" : ""}`}>
          <div><Cpu size={18} /><span><small>ComfyUI policy</small><strong>{snapshot.backend.profile ?? (snapshot.backend.running ? "Unowned server" : "Starts after review")}</strong></span></div>
          <p>{snapshot.backend.detail}</p>
          <div className="video-policy-line"><ShieldCheck size={14} /> Offloading: <strong>{snapshot.backend.offloading}</strong></div>
          <button className="quiet-button" onClick={() => setShowSetup((value) => !value)}><Settings2 size={15} /> Backend setup</button>
        </div>
      </header>

      {showSetup && <BackendSetup snapshot={snapshot} busy={busy === "saving"} onSnapshot={setSnapshot} onBusy={setBusy} onError={onError} />}

      <div className="video-layout">
        <div className="video-create-column">
          <section className="video-panel prompt-panel">
            <div className="video-panel-heading"><span className="step-badge">1</span><div><span className="eyebrow">Creative intent</span><h2>Describe the finished experience</h2></div></div>
            <label className="video-prompt-field">
              <span>Prompt</span>
              <textarea aria-label="Video prompt" value={prompt} onChange={(event) => setPrompt(event.target.value)} placeholder="A warm, visually precise educational short explaining how a forest recovers after fire…" />
              <small>The local planner turns this into continuity, chapters, and individual shots. You review before any GPU generation.</small>
            </label>
            <div className="video-form-grid">
              <label><span>Audience</span><input aria-label="Video audience" value={audience} onChange={(event) => setAudience(event.target.value)} /></label>
              <label><span>Use case</span><select aria-label="Video use case" value={useCase} onChange={(event) => setUseCase(event.target.value)}><option>Artistic short</option><option>Educational short</option><option>Publisher series</option><option>Documentary sequence</option><option>Ambient installation</option><option>Product visualization</option><option>Storyboard / previsualization</option></select></label>
              <label><span>Format</span><select aria-label="Video orientation" value={orientation} onChange={(event) => setOrientation(event.target.value as typeof orientation)}><option value="landscape">Landscape 16:9</option><option value="portrait">Portrait 9:16</option><option value="square">Near-square</option></select></label>
              <label><span>Planning model</span><select aria-label="Planning model" value={plannerModelId} onChange={(event) => setPlannerModelId(event.target.value)}><option value="">Deterministic plan (no LLM)</option>{control.models.filter((model) => model.chatTemplate).map((model) => <option value={model.id} key={model.id}>{model.name}</option>)}</select></label>
            </div>
            <div className="duration-builder">
              <div><span className="eyebrow">Target runtime</span><strong>{formatDuration(totalSeconds)}</strong></div>
              <label><span>Hours</span><input aria-label="Video hours" type="number" min={0} max={12} value={hours} onChange={(event) => setHours(clampNumber(event.target.value, 0, 12))} /></label>
              <label><span>Minutes</span><input aria-label="Video minutes" type="number" min={0} max={59} value={minutes} onChange={(event) => setMinutes(clampNumber(event.target.value, 0, 59))} /></label>
              <label><span>Seconds</span><input aria-label="Video seconds" type="number" min={0} max={59} value={seconds} onChange={(event) => setSeconds(clampNumber(event.target.value, 0, 59))} /></label>
              <div className={`clip-estimate ${boundaryExceeded ? "warning" : ""}`}><Layers3 size={16} /><span><strong>{estimatedClips.toLocaleString()} clips</strong><small>{selectedPreset?.nativeClipSeconds ?? 5}s native segments · serial execution</small></span></div>
            </div>
          </section>

          <section className="video-panel">
            <div className="video-panel-heading"><span className="step-badge">2</span><div><span className="eyebrow">Quality and timing policy</span><h2>Choose the backend deliberately</h2></div></div>
            <div className="preset-grid">
              {snapshot.presets.map((item) => <PresetCard key={item.id} item={item} selected={preset === item.id} onSelect={() => setPreset(item.id)} />)}
            </div>
          </section>

          <section className="video-panel boundary-panel">
            <button className="advanced-disclosure" onClick={() => setShowAdvanced((value) => !value)} aria-expanded={showAdvanced}><span><Gauge size={17} /><span><strong>Execution boundaries</strong><small>Review retries, disk reserve, runtime, and assembly before a long batch.</small></span></span><ChevronDown className={showAdvanced ? "open" : ""} /></button>
            {showAdvanced && <div className="boundary-content">
              <div className="boundary-grid">
                <NumberField label="Maximum clips" value={boundaries.maxClips} min={1} max={20_000} onChange={(value) => setBoundaries({ ...boundaries, maxClips: value })} />
                <NumberField label="Retries per clip" value={boundaries.maxRetriesPerClip} min={0} max={10} onChange={(value) => setBoundaries({ ...boundaries, maxRetriesPerClip: value })} />
                <NumberField label="Failed clips before pause" value={boundaries.maxFailedClips} min={0} max={boundaries.maxClips} onChange={(value) => setBoundaries({ ...boundaries, maxFailedClips: value })} />
                <NumberField label="Runtime boundary (minutes)" value={boundaries.maxRuntimeMinutes} min={1} max={100_800} onChange={(value) => setBoundaries({ ...boundaries, maxRuntimeMinutes: value })} />
                <NumberField label="Minimum free disk (GiB)" value={boundaries.minFreeDiskGib} min={0} max={10_000} onChange={(value) => setBoundaries({ ...boundaries, minFreeDiskGib: value })} />
                <label className="assembly-toggle"><input type="checkbox" checked={boundaries.assembleFinalVideo} onChange={(event) => setBoundaries({ ...boundaries, assembleFinalVideo: event.target.checked })} /><span><strong>Assemble verified clips</strong><small>Local FFmpeg concat after every clip passes verification.</small></span></label>
              </div>
              <label className="negative-field"><span>Negative prompt</span><textarea value={negativePrompt} onChange={(event) => setNegativePrompt(event.target.value)} /></label>
            </div>}
            {boundaryExceeded && <div className="video-warning"><AlertTriangle size={17} /><span>This plan needs {estimatedClips.toLocaleString()} clips, above the {boundaries.maxClips.toLocaleString()} boundary. Raise it deliberately or shorten the runtime.</span></div>}
            <div className="plan-actions"><div><ShieldCheck size={15} /><span>Planning loads only the local LLM. ComfyUI starts after review.</span></div><button className="primary-button" disabled={busy !== null || !prompt.trim() || totalSeconds < 2 || boundaryExceeded || !selectedPreset?.available} onClick={() => void plan()}>{busy === "planning" ? <LoaderCircle className="spin" /> : <WandSparkles size={16} />} Plan production</button></div>
          </section>
        </div>

        <aside className="video-project-column">
          <ProjectInspector project={project} running={running} busy={busy} events={events} onStart={start} onStop={async () => { if (!project) return; try { await stopVideoProject(project.id); } catch (cause) { onError(String(cause)); } }} onReveal={() => project && void revealVideoProject(project.id)} onEdit={async (clipIndex, prompt) => { if (!project) return; try { setProject(await updateVideoClipPrompt(project.id, clipIndex, prompt)); } catch (cause) { onError(String(cause)); } }} onProject={setProject} onError={onError} />
          <section className="recent-projects">
            <div className="recent-heading"><span><Clock3 size={15} /> Durable projects</span><button className="icon-button" aria-label="Refresh video projects" onClick={() => void refresh()}><RefreshCw size={15} /></button></div>
            {snapshot.projects.length ? snapshot.projects.slice(0, 12).map((item) => <button key={item.id} className={project?.id === item.id ? "selected" : ""} onClick={() => void chooseProject(item.id)}><span><strong>{item.title}</strong><small>{presetCopy[item.preset].kicker} · {formatDuration(item.totalDurationSeconds)}</small></span><span className={`project-status status-${item.status}`}>{humanStatus(item.status)}</span><small>{item.completedClips}/{item.clipCount} verified{item.failedClips ? ` · ${item.failedClips} failed` : ""}</small></button>) : <div className="no-video-projects"><Film size={20} /><span>Your reviewed plans and recoverable runs appear here.</span></div>}
          </section>
        </aside>
      </div>
    </section>
  );
}

function BackendSetup({ snapshot, busy, onSnapshot, onBusy, onError }: { snapshot: VideoSnapshot; busy: boolean; onSnapshot: (value: VideoSnapshot) => void; onBusy: (value: "saving" | null) => void; onError: (message: string) => void }) {
  const [settings, setSettings] = useState(snapshot.settings);
  useEffect(() => setSettings(snapshot.settings), [snapshot.settings]);
  const save = async () => {
    onBusy("saving");
    try { onSnapshot(await saveVideoSettings(settings)); } catch (cause) { onError(String(cause)); } finally { onBusy(null); }
  };
  return <section className="video-setup-panel">
    <div className="setup-heading"><div><Settings2 size={19} /><span><strong>Local backend setup</strong><small>Loopback is fixed to 127.0.0.1:8188. Kestrel never adds a remote fallback.</small></span></div><span className="setup-root"><HardDrive size={14} /> {snapshot.root}</span></div>
    <div className="setup-fields">
      <label><span>ComfyUI root</span><div><input value={settings.comfyRoot} onChange={(event) => setSettings({ ...settings, comfyRoot: event.target.value })} /><button className="quiet-button" onClick={async () => { const root = await pickComfyRoot(); if (root) setSettings({ ...settings, comfyRoot: root }); }}><FolderOpen size={14} /> Choose</button></div></label>
      <label><span>FFmpeg path or PATH command</span><input value={settings.ffmpegPath} onChange={(event) => setSettings({ ...settings, ffmpegPath: event.target.value })} /></label>
    </div>
    <div className="setup-actions"><span>{snapshot.presets.filter((item) => item.available).length}/{snapshot.presets.length} presets ready</span>{snapshot.backend.owned && <button className="quiet-button" onClick={async () => { try { onSnapshot(await stopVideoBackend()); } catch (cause) { onError(String(cause)); } }}><CircleStop size={14} /> Stop owned backend</button>}<button className="primary-button" disabled={busy} onClick={() => void save()}>{busy ? <LoaderCircle className="spin" /> : <Save size={14} />} Save setup</button></div>
  </section>;
}

function PresetCard({ item, selected, onSelect }: { item: VideoPresetStatus; selected: boolean; onSelect: () => void }) {
  const copy = presetCopy[item.id];
  return <button className={`preset-card ${selected ? "selected" : ""} ${!item.available ? "unavailable" : ""}`} onClick={onSelect} disabled={!item.available}>
    <span className="preset-radio">{selected && <span />}</span>
    <span className="preset-copy"><small>{copy.kicker}</small><strong>{item.label}</strong><span>{copy.description}</span><em>{copy.quality}</em></span>
    <span className={`offload-tag offload-${item.offloading}`}>{item.offloading}</span>
    {!item.available && <span className="missing-models">Missing {item.missingFiles.length} local assets</span>}
  </button>;
}

function ProjectInspector({ project, running, busy, events, onStart, onStop, onReveal, onEdit, onProject, onError }: { project: VideoProject | null; running: boolean; busy: string | null; events: VideoProjectEvent[]; onStart: () => void; onStop: () => void; onReveal: () => void; onEdit: (clipIndex: number, prompt: string) => Promise<void>; onProject: (project: VideoProject) => void; onError: (message: string) => void }) {
  const [selectedClipIndex, setSelectedClipIndex] = useState<number | null>(null);
  const [clipPage, setClipPage] = useState(0);
  useEffect(() => { setSelectedClipIndex(null); setClipPage(0); }, [project?.id]);
  if (!project) return <section className="project-inspector empty"><div className="project-empty-icon"><Sparkles /></div><span className="eyebrow">Review before generation</span><h2>Your production plan will appear here.</h2><p>Kestrel saves the story bible, chapter boundaries, seeds, prompts, retries, verified hashes, and assembly result as durable local data.</p><div className="inspector-assurances"><span><ShieldCheck size={14} /> No hidden offload changes</span><span><Layers3 size={14} /> Serial queue</span><span><HardDrive size={14} /> Restart recovery</span></div></section>;
  const completed = project.clips.filter((clip) => clip.status === "complete").length;
  const failed = project.clips.filter((clip) => clip.status === "failed").length;
  const percent = project.clips.length ? Math.round(completed / project.clips.length * 100) : 0;
  const clipPageSize = 120;
  const clipPageCount = Math.max(1, Math.ceil(project.clips.length / clipPageSize));
  const visibleClips = project.clips.slice(clipPage * clipPageSize, (clipPage + 1) * clipPageSize);
  const canStart = !running && !["completed", "completed-with-warnings"].includes(project.status);
  const selectedClip = project.clips.find((clip) => clip.index === selectedClipIndex);
  return <section className="project-inspector">
    <div className="project-inspector-header"><div><span className="eyebrow">Durable production plan</span><h2>{project.title}</h2></div><span className={`project-status status-${project.status}`}>{humanStatus(project.status)}</span></div>
    <p className="planning-note">{project.planningNote}</p>
    <div className="project-facts"><span><Film size={14} /><strong>{project.clips.length.toLocaleString()}</strong> clips</span><span><MonitorUp size={14} /><strong>{project.width}×{project.height}</strong></span><span><Gauge size={14} /><strong>{project.steps}</strong> steps</span><span><Clock3 size={14} /><strong>{formatDuration(project.totalDurationSeconds)}</strong></span></div>
    <div className="project-progress"><div><span>{completed.toLocaleString()} verified</span><span>{failed ? `${failed} failed · ` : ""}{percent}%</span></div><div><span style={{ width: `${percent}%` }} /></div></div>
    <div className="project-runtime-policy"><ShieldCheck size={14} /><span><small>Locked generation policy</small><strong>Offloading {runtimePolicy[project.preset].offloading} · {runtimePolicy[project.preset].profile}</strong></span></div>
    <ReferenceStudio project={project} disabled={running} onProject={onProject} onError={onError} />
    <details className="continuity-card" open={project.status === "planned"}><summary><Sparkles size={15} /> Story bible and continuity</summary><p>{project.continuityBible}</p></details>
    <div className="chapter-list"><span className="eyebrow">Chapter boundaries · {project.chapters.length}</span>{project.chapters.map((chapter) => <div key={chapter.index}><span>{String(chapter.index).padStart(2, "0")}</span><div><strong>{chapter.title}</strong><small>Clips {chapter.firstClip}–{chapter.lastClip} · {chapter.narrativeGoal}</small>{!!project.references.length && <select aria-label={`Chapter ${chapter.index} opening reference`} disabled={running} value={chapter.referenceAssetId ?? ""} onChange={async (event) => { try { onProject(await setVideoChapterReference(project.id, chapter.index, event.target.value || undefined)); } catch (cause) { onError(String(cause)); } }}><option value="">Default continuity at opening</option>{project.references.map((asset) => <option key={asset.id} value={asset.id}>{asset.kind === "video" ? "Motion" : humanStatus(asset.role)} · {asset.name}</option>)}</select>}</div></div>)}</div>
    <div className="clip-ledger"><div className="ledger-heading"><span className="eyebrow">Clip ledger</span><span className="ledger-pages"><button className="icon-button" aria-label="Previous clip page" disabled={clipPage === 0} onClick={() => setClipPage((value) => Math.max(0, value - 1))}>‹</button><small>{clipPage + 1} / {clipPageCount} · clips {visibleClips[0]?.index ?? 0}–{visibleClips.at(-1)?.index ?? 0}</small><button className="icon-button" aria-label="Next clip page" disabled={clipPage + 1 >= clipPageCount} onClick={() => setClipPage((value) => Math.min(clipPageCount - 1, value + 1))}>›</button></span></div><div>{visibleClips.map((clip) => <button type="button" key={clip.index} className={`clip-dot clip-${clip.status} ${selectedClipIndex === clip.index ? "selected" : ""}`} title={`Clip ${clip.index}: ${clip.status}${clip.error ? ` — ${clip.error}` : ""}`} onClick={() => setSelectedClipIndex(clip.index)}>{clip.status === "complete" ? <CheckCircle2 /> : clip.status === "generating" || clip.status === "verifying" ? <LoaderCircle className="spin" /> : clip.status === "failed" ? <AlertTriangle /> : clip.index}</button>)}</div></div>
    {selectedClip && <ClipPromptEditor clip={selectedClip} references={project.references} disabled={running || selectedClip.status === "complete"} onSave={(prompt) => onEdit(selectedClip.index, prompt)} onReference={async (referenceAssetId) => { try { onProject(await setVideoClipReference(project.id, selectedClip.index, referenceAssetId)); } catch (cause) { onError(String(cause)); } }} />}
    {!!events.length && <div className="video-event-log">{events.slice(-6).reverse().map((event, index) => <div key={`${event.at}-${index}`}><span /><p><strong>{event.title}</strong>{event.detail}</p></div>)}</div>}
    {!!project.errors.length && <details className="project-errors"><summary><AlertTriangle size={14} /> {project.errors.length} recorded issue{project.errors.length === 1 ? "" : "s"}</summary>{project.errors.slice(-12).map((error, index) => <p key={index}>{error}</p>)}</details>}
    <div className="project-actions"><button className="quiet-button" onClick={onReveal}><FolderOpen size={15} /> Project files</button><span />{running ? <button className="danger-button" onClick={onStop}><CircleStop size={15} /> Stop safely</button> : <button className="primary-button" disabled={!canStart || busy !== null} onClick={onStart}>{busy === "starting" ? <LoaderCircle className="spin" /> : <Play size={15} />} {project.status === "planned" ? "Start generation" : "Resume unfinished clips"}</button>}</div>
  </section>;
}

function ReferenceStudio({ project, disabled, onProject, onError }: { project: VideoProject; disabled: boolean; onProject: (project: VideoProject) => void; onError: (message: string) => void }) {
  const [importing, setImporting] = useState<"subject" | "storyboard" | "motion" | null>(null);
  const supportsImages = project.preset !== "wan-1.3b-gpu-only";
  const supportsVideo = project.preset === "wan-vace-1.3b-reference";
  const images = project.references.filter((asset) => asset.kind === "image");
  const updateContinuity = async (mode: VideoContinuityMode, primaryReferenceId = project.continuity.primaryReferenceId) => {
    try { onProject(await setVideoContinuity(project.id, mode, primaryReferenceId)); } catch (cause) { onError(String(cause)); }
  };
  const add = async (role: "subject" | "storyboard" | "motion") => {
    setImporting(role);
    try {
      const next = await importVideoReference(project.id, role);
      if (next) onProject(next);
    } catch (cause) { onError(String(cause)); } finally { setImporting(null); }
  };
  return <section className="reference-studio">
    <div className="reference-heading"><div><ImagePlus size={16} /><span><strong>Subject & storyboard references</strong><small>Hashed project copies; source files can move later.</small></span></div><span>{project.references.length} asset{project.references.length === 1 ? "" : "s"}</span></div>
    {!supportsImages ? <p className="reference-unavailable">This strict GPU-only Wan model is text-to-video. Re-plan with Kandinsky, Wan 2.2 TI2V, or Wan VACE to condition shots on durable references.</p> : <>
      <div className="reference-actions"><button className="quiet-button" disabled={disabled || importing !== null} onClick={() => void add("subject")}><ImagePlus size={13} /> {importing === "subject" ? "Importing…" : "Subject image"}</button><button className="quiet-button" disabled={disabled || importing !== null} onClick={() => void add("storyboard")}><Film size={13} /> {importing === "storyboard" ? "Importing…" : "Storyboard frame"}</button><button className="quiet-button" title={supportsVideo ? "Import a control-video reference" : "Wan VACE is required for motion video"} disabled={!supportsVideo || disabled || importing !== null} onClick={() => void add("motion")}><Video size={13} /> {importing === "motion" ? "Importing…" : "Motion video"}</button></div>
      {!!project.references.length && <div className="reference-assets">{project.references.map((asset) => <span key={asset.id} className={`reference-${asset.kind}`}><ReferenceThumbnail projectId={project.id} asset={asset} /><span><strong>{asset.name}</strong><small>{humanStatus(asset.role)} · {(asset.bytes / 1024 / 1024).toFixed(1)} MiB</small></span></span>)}</div>}
      <div className="continuity-controls"><label><span>Continuity policy</span><select disabled={disabled} value={project.continuity.mode} onChange={(event) => void updateContinuity(event.target.value as VideoContinuityMode)}><option value="none">Independent shots</option><option value="anchor">Anchor every shot to primary image</option><option value="previous-frame">Chain previous verified end frame</option></select></label><label><span>Primary subject / look</span><select disabled={disabled || !images.length} value={project.continuity.primaryReferenceId ?? ""} onChange={(event) => void updateContinuity(project.continuity.mode === "none" ? "anchor" : project.continuity.mode, event.target.value || undefined)}><option value="">No primary image</option>{images.map((asset) => <option key={asset.id} value={asset.id}>{asset.name}</option>)}</select></label></div>
      <p className="continuity-note">Anchor mode uses the selected subject/look image on every shot. Chain mode uses it for the first shot, then extracts each verified final frame locally for the next shot; an explicit clip storyboard overrides that frame.</p>
    </>}
  </section>;
}

function ReferenceThumbnail({ projectId, asset }: { projectId: string; asset: VideoProject["references"][number] }) {
  const [preview, setPreview] = useState<string>();
  useEffect(() => {
    let active = true;
    if (!asset.previewPath) return undefined;
    void getVideoReferencePreview(projectId, asset.id).then((value) => {
      if (active) setPreview(value);
    }).catch(() => undefined);
    return () => { active = false; };
  }, [asset.id, asset.previewPath, projectId]);
  if (preview) return <img className="reference-thumbnail" src={preview} alt="" />;
  return <span className="reference-thumbnail-fallback">{asset.kind === "image" ? <ImagePlus size={13} /> : <Video size={13} />}</span>;
}

function ClipPromptEditor({ clip, references, disabled, onSave, onReference }: { clip: VideoProject["clips"][number]; references: VideoProject["references"]; disabled: boolean; onSave: (prompt: string) => Promise<void>; onReference: (referenceAssetId?: string) => Promise<void> }) {
  const [prompt, setPrompt] = useState(clip.prompt);
  const [saving, setSaving] = useState(false);
  useEffect(() => setPrompt(clip.prompt), [clip.index, clip.prompt]);
  const save = async () => {
    setSaving(true);
    try { await onSave(prompt); } finally { setSaving(false); }
  };
  return <div className="clip-prompt-editor"><div><span><strong>Clip {clip.index}</strong><small>Seed {clip.seed} · {humanStatus(clip.status)}</small></span>{clip.error && <em>{clip.error}</em>}</div>{!!references.length && <label className="clip-reference-select"><span>Shot-specific storyboard or motion reference</span><select disabled={disabled} value={clip.referenceAssetId ?? ""} onChange={(event) => void onReference(event.target.value || undefined)}><option value="">Use project continuity policy</option>{references.map((asset) => <option key={asset.id} value={asset.id}>{asset.kind === "video" ? "Motion" : humanStatus(asset.role)} · {asset.name}</option>)}</select></label>}<textarea aria-label={`Clip ${clip.index} prompt`} value={prompt} disabled={disabled} maxLength={14_000} onChange={(event) => setPrompt(event.target.value)} /><div><small>{prompt.length.toLocaleString()} / 14,000 characters</small><button className="quiet-button" disabled={disabled || saving || !prompt.trim() || prompt === clip.prompt} onClick={() => void save()}>{saving ? <LoaderCircle className="spin" size={14} /> : <Save size={14} />} Save clip prompt</button></div></div>;
}

function NumberField({ label, value, min, max, onChange }: { label: string; value: number; min: number; max: number; onChange: (value: number) => void }) {
  return <label><span>{label}</span><input type="number" min={min} max={max} value={value} onChange={(event) => onChange(clampNumber(event.target.value, min, max))} /></label>;
}

function clampNumber(value: string, min: number, max: number): number {
  const number = Number.parseInt(value, 10);
  if (!Number.isFinite(number)) return min;
  return Math.min(max, Math.max(min, number));
}

function formatDuration(total: number): string {
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total % 3600 / 60);
  const seconds = total % 60;
  if (hours) return `${hours}h ${minutes}m ${seconds}s`;
  if (minutes) return `${minutes}m ${seconds}s`;
  return `${seconds}s`;
}

function humanStatus(status: string): string {
  return status.replaceAll("-", " ").replace(/^./, (value) => value.toUpperCase());
}
