import {
  Archive, Check, ChevronDown, CircleStop, Clapperboard, Clock3, Download,
  Film, FolderOpen, GripVertical, Library, LoaderCircle, Play, Plus,
  RotateCcw, Save, Settings2, Sparkles, Volume2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  cancelMovie, getMovie, listMovies, movieMediaUrl, onMovieProject,
  renderMovieEdit, resumeMovie, revealMovie, saveMovieEdits, startMovie,
} from "./api";
import type { ClipEdit, MovieProject, MovieSettings, MovieSummary } from "./types";

const defaultSettings: MovieSettings = {
  researchMode: "auto",
  width: 1344,
  height: 768,
  clipSeconds: 5,
  steps: 20,
  maxClips: 12,
  seed: 0,
  temperature: 0.7,
  topP: 0.95,
  topK: 20,
  thinkingBudget: 4096,
  maxOutputTokens: 32768,
  comfyRoot: "D:\\AI\\ComfyUI",
};

export function MovieStudio({ advancedEnabled, onError }: { advancedEnabled: boolean; onError: (message: string) => void }) {
  const [movies, setMovies] = useState<MovieSummary[]>([]);
  const [project, setProject] = useState<MovieProject | null>(null);
  const [creating, setCreating] = useState(true);
  const [prompt, setPrompt] = useState("");
  const [settings, setSettings] = useState(defaultSettings);
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const [edits, setEdits] = useState<ClipEdit[]>([]);

  const refreshList = useCallback(async () => {
    try { setMovies(await listMovies()); } catch (error) { onError(String(error)); }
  }, [onError]);

  useEffect(() => {
    void refreshList();
    let dispose: (() => void) | undefined;
    void onMovieProject((next) => {
      setProject((current) => !current || current.id === next.id ? next : current);
      setEdits(next.edit.clips);
      void refreshList();
    }).then((unlisten) => { dispose = unlisten; });
    return () => dispose?.();
  }, [refreshList]);

  useEffect(() => {
    if (!project || project.status !== "running") return;
    const timer = window.setInterval(() => void getMovie(project.id).then((next) => {
      setProject(next); setEdits(next.edit.clips);
    }).catch(() => undefined), 2500);
    return () => window.clearInterval(timer);
  }, [project?.id, project?.status]);

  const openProject = async (id: string) => {
    try {
      const next = await getMovie(id);
      setProject(next); setEdits(next.edit.clips); setCreating(false);
    } catch (error) { onError(String(error)); }
  };

  const makeMovie = async () => {
    if (!prompt.trim()) return;
    setBusy(true);
    try {
      const next = await startMovie({ prompt, settings });
      setProject(next); setEdits(next.edit.clips); setCreating(false); await refreshList();
    } catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  const updateEdit = (clipId: string, change: Partial<ClipEdit>) => {
    setEdits((items) => items.map((item) => item.clipId === clipId ? { ...item, ...change } : item));
  };

  const saveEdits = async (exportNow: boolean) => {
    if (!project) return;
    setBusy(true);
    try {
      let next = await saveMovieEdits(project.id, { ...project.edit, clips: edits });
      if (exportNow) next = await renderMovieEdit(project.id);
      setProject(next); setEdits(next.edit.clips);
    } catch (error) { onError(String(error)); } finally { setBusy(false); }
  };

  return (
    <div className="movie-studio">
      <aside className="movie-library">
        <div className="movie-library-title"><span>Private movie library</span><button onClick={() => { setCreating(true); setProject(null); }}><Plus size={15} /></button></div>
        <div className="movie-list">
          {movies.map((movie) => <button key={movie.id} className={project?.id === movie.id ? "active" : ""} onClick={() => void openProject(movie.id)}>
            <Film size={15} /><span><strong>{movie.title}</strong><small>{movie.phase} · {movie.clipCount} clips</small></span>
          </button>)}
          {!movies.length && <div className="movie-empty-list"><Library size={18} />Your durable productions will appear here.</div>}
        </div>
      </aside>
      <section className="movie-stage">
        {creating || !project ? (
          <MovieLaunch prompt={prompt} settings={settings} advanced={advanced} advancedEnabled={advancedEnabled} busy={busy}
            onPrompt={setPrompt} onSettings={setSettings} onAdvanced={setAdvanced} onMake={() => void makeMovie()} />
        ) : (
          <MovieProjectView project={project} edits={edits} busy={busy} onEdit={updateEdit}
            onNew={() => { setCreating(true); setProject(null); }}
            onCancel={() => void cancelMovie(project.id).then(setProject).catch((error) => onError(String(error)))}
            onResume={() => void resumeMovie(project.id).then(setProject).catch((error) => onError(String(error)))}
            onReveal={() => void revealMovie(project.id)}
            onSave={() => void saveEdits(false)} onExport={() => void saveEdits(true)} />
        )}
      </section>
    </div>
  );
}

function MovieLaunch({ prompt, settings, advanced, advancedEnabled, busy, onPrompt, onSettings, onAdvanced, onMake }: {
  prompt: string; settings: MovieSettings; advanced: boolean; advancedEnabled: boolean; busy: boolean;
  onPrompt: (value: string) => void; onSettings: (value: MovieSettings) => void; onAdvanced: (value: boolean) => void; onMake: () => void;
}) {
  const quality = settings.width === 1344 ? "master" : settings.width === 864 ? "preview" : "custom";
  return <div className="movie-launch">
    <div className="movie-launch-mark"><Clapperboard /></div>
    <span className="eyebrow">Bonsai director · MiniMax H3 picture & sound</span>
    <h1>Describe the movie.<br />Kestrel runs the studio.</h1>
    <p>One prompt becomes a researched screenplay, continuity bible, native-audio H3 scenes, and an editable first cut—entirely on this computer.</p>
    <div className="movie-prompt-box">
      <textarea autoFocus value={prompt} onChange={(event) => onPrompt(event.target.value)} placeholder="A short educational film explaining why the northern lights happen for a curious ten-year-old…" />
      <div><span><Archive size={14} /> Offline Wikipedia is available when facts matter</span><button disabled={busy || prompt.trim().length < 3} onClick={onMake}>{busy ? <LoaderCircle className="spin" /> : <Sparkles />} Make movie</button></div>
    </div>
    <div className="movie-presets">
      <button className={quality === "master" ? "active" : ""} onClick={() => onSettings({ ...settings, width: 1344, height: 768 })}><strong>Publish master</strong><span>1344 × 768 · highest H3 native canvas</span></button>
      <button className={quality === "preview" ? "active" : ""} onClick={() => onSettings({ ...settings, width: 864, height: 480 })}><strong>Faster draft</strong><span>864 × 480 · proven ~2½ min per clip</span></button>
    </div>
    <button className="movie-advanced-toggle" onClick={() => onAdvanced(!advanced)}><Settings2 size={14} /> Advanced production controls <ChevronDown className={advanced ? "open" : ""} size={14} /></button>
    {advanced && <div className="movie-advanced">
      <SelectField label="Research" value={settings.researchMode} onChange={(value) => onSettings({ ...settings, researchMode: value as MovieSettings["researchMode"] })} options={["auto", "never", "always"]} />
      <NumberField label="Clip seconds" value={settings.clipSeconds} min={5} max={15} step={1} onChange={(value) => onSettings({ ...settings, clipSeconds: value })} />
      <NumberField label="Maximum clips" value={settings.maxClips} min={1} max={advancedEnabled ? 96 : 24} step={1} onChange={(value) => onSettings({ ...settings, maxClips: value })} />
      <NumberField label="Sampling steps" value={settings.steps} min={1} max={advancedEnabled ? 100 : 40} step={1} onChange={(value) => onSettings({ ...settings, steps: value })} />
      <NumberField label="Seed (0 = random)" value={settings.seed} min={0} max={Number.MAX_SAFE_INTEGER} step={1} onChange={(value) => onSettings({ ...settings, seed: value })} />
      <NumberField label="Temperature" value={settings.temperature} min={0} max={2} step={0.05} onChange={(value) => onSettings({ ...settings, temperature: value })} />
      <NumberField label="Top P" value={settings.topP} min={0.05} max={1} step={0.01} onChange={(value) => onSettings({ ...settings, topP: value })} />
      <NumberField label="Top K" value={settings.topK} min={1} max={200} step={1} onChange={(value) => onSettings({ ...settings, topK: value })} />
      <NumberField label="Thinking budget" value={settings.thinkingBudget} min={0} max={32768} step={256} onChange={(value) => onSettings({ ...settings, thinkingBudget: value })} />
      <NumberField label="Output budget" value={settings.maxOutputTokens} min={1024} max={32768} step={1024} onChange={(value) => onSettings({ ...settings, maxOutputTokens: value })} />
      <label className="wide">ComfyUI root<input value={settings.comfyRoot} onChange={(event) => onSettings({ ...settings, comfyRoot: event.target.value })} /></label>
    </div>}
    <div className="movie-capabilities"><span><Check />98,304 context</span><span><Check />32,768 output</span><span><Check />Native stereo audio</span><span><Check />Crash-safe masters</span></div>
  </div>;
}

function MovieProjectView({ project, edits, busy, onEdit, onNew, onCancel, onResume, onReveal, onSave, onExport }: {
  project: MovieProject; edits: ClipEdit[]; busy: boolean; onEdit: (id: string, change: Partial<ClipEdit>) => void;
  onNew: () => void; onCancel: () => void; onResume: () => void; onReveal: () => void; onSave: () => void; onExport: () => void;
}) {
  const complete = project.clips.filter((clip) => clip.status === "complete").length;
  const progress = project.clips.length ? Math.round((complete / project.clips.length) * 100) : project.plan ? 10 : 3;
  const canResume = ["failed", "cancelled", "interrupted"].includes(project.status) && Boolean(project.plan);
  return <div className="movie-project-view">
    <header className="movie-project-header">
      <div><span className="eyebrow">{project.status === "complete" ? "First cut ready" : project.phase}</span><h1>{project.title}</h1><p>{project.plan?.logline ?? project.prompt}</p></div>
      <div className="movie-project-actions"><button onClick={onNew}><Plus /> New</button><button onClick={onReveal}><FolderOpen /> Files</button>{project.status === "running" && <button className="danger" onClick={onCancel}><CircleStop /> Stop safely</button>}{canResume && <button className="accent" onClick={onResume}><RotateCcw /> Resume</button>}</div>
    </header>
    <div className={`movie-status-card ${project.status}`}>
      <div>{project.status === "running" ? <LoaderCircle className="spin" /> : project.status === "complete" ? <Check /> : <Clock3 />}<span><strong>{project.detail}</strong><small>{complete} of {project.clips.length || "—"} masters preserved · {project.renderer}</small></span></div>
      <div className="movie-progress"><i style={{ width: `${progress}%` }} /></div>
      {project.error && <pre>{project.error}</pre>}
    </div>
    {project.finalPath && <section className="movie-final"><div className="movie-section-heading"><div><span className="eyebrow">Current cut</span><h2>Watch the movie</h2></div><a href={movieMediaUrl(project.finalPath)} download><Download /> Open master</a></div><video controls preload="metadata" src={movieMediaUrl(project.finalPath)} /></section>}
    {project.plan && <section className="movie-plan-overview"><article><span className="eyebrow">Creative direction</span><p>{project.plan.creativeDirection}</p></article><article><span className="eyebrow">Continuity bible</span><ul>{project.plan.continuityBible.map((rule) => <li key={rule}>{rule}</li>)}</ul></article>{project.sources.length > 0 && <article><span className="eyebrow">Opened archive evidence</span><ul>{project.sources.map((source) => <li key={source.id}>{source.id} · {source.title} ({source.snapshot})</li>)}</ul></article>}</section>}
    {project.clips.length > 0 && <section className="movie-timeline-section">
      <div className="movie-section-heading"><div><span className="eyebrow">Non-destructive timeline</span><h2>Scenes & sound</h2></div><div><button disabled={busy} onClick={onSave}><Save /> Save edit</button><button className="accent" disabled={busy || complete === 0 || project.status === "running"} onClick={onExport}>{busy ? <LoaderCircle className="spin" /> : <Play />} Export new cut</button></div></div>
      <div className="movie-clip-grid">{project.clips.map((clip) => {
        const edit = edits.find((item) => item.clipId === clip.id) ?? { clipId: clip.id, enabled: true, order: clip.index, trimStart: 0, trimEnd: 0, audioGain: 1 };
        return <article key={clip.id} className={`movie-clip ${clip.status} ${edit.enabled ? "" : "disabled"}`}>
          <div className="clip-preview">{clip.path ? <video controls preload="metadata" src={movieMediaUrl(clip.path)} /> : <div><LoaderCircle className={clip.status === "rendering" ? "spin" : ""} /><span>{clip.status}</span></div>}<span className="clip-number">{clip.index + 1}</span></div>
          <div className="clip-copy"><div><GripVertical /><span><strong>{clip.title}</strong><small>{clip.durationSeconds.toFixed(1)}s · seed {clip.seed}</small></span><label className="clip-enable"><input type="checkbox" checked={edit.enabled} onChange={(event) => onEdit(clip.id, { enabled: event.target.checked })} /> Use</label></div>
            <details><summary>Director prompt</summary><p>{clip.prompt}</p></details>
            <div className="clip-controls"><NumberField label="Order" value={edit.order + 1} min={1} max={project.clips.length} step={1} onChange={(value) => onEdit(clip.id, { order: value - 1 })} /><NumberField label="Trim in" value={edit.trimStart} min={0} max={clip.durationSeconds - 0.1} step={0.1} onChange={(value) => onEdit(clip.id, { trimStart: value })} /><NumberField label="Trim out" value={edit.trimEnd} min={0} max={clip.durationSeconds - 0.1} step={0.1} onChange={(value) => onEdit(clip.id, { trimEnd: value })} /><NumberField label="Audio gain" value={edit.audioGain} min={0} max={4} step={0.05} onChange={(value) => onEdit(clip.id, { audioGain: value })} /></div>
          </div>{clip.error && <pre>{clip.error}</pre>}
        </article>;
      })}</div>
    </section>}
  </div>;
}

function NumberField({ label, value, min, max, step, onChange }: { label: string; value: number; min: number; max: number; step: number; onChange: (value: number) => void }) {
  return <label>{label}<input type="number" value={value} min={min} max={max} step={step} onChange={(event) => onChange(Number(event.target.value))} /></label>;
}

function SelectField({ label, value, options, onChange }: { label: string; value: string; options: string[]; onChange: (value: string) => void }) {
  return <label>{label}<select value={value} onChange={(event) => onChange(event.target.value)}>{options.map((option) => <option key={option}>{option}</option>)}</select></label>;
}
