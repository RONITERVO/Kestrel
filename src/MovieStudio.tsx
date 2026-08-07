import {
  AudioLines, Check, ChevronDown, CircleStop, Clapperboard, Clock3, Download,
  Film, FolderOpen, GripVertical, ImageIcon, Library, LoaderCircle, Paperclip, Play, Plus,
  RotateCcw, Save, Settings2, Sparkles, Video, X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  approveMoviePlan, askBonsaiMovieClip, cancelMovie, getMovie, listMovies, movieMediaUrl,
  onMovieProject, pickMovieReferenceFiles, renderMovieClipVersion, renderMovieEdit,
  resumeMovie, revealMovie, reviseMoviePlan, saveMovieEdits, saveMoviePlan, startMovie,
} from "./api";
import type {
  ClipEdit, MovieClipSuggestion, MoviePlan, MovieProject, MovieSettings, MovieSummary,
  PendingMovieReference, PlannedClip, RenderedClip,
} from "./types";

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

export function MovieStudio({ initialComfyRoot, advancedEnabled, onError }: { initialComfyRoot?: string; advancedEnabled: boolean; onError: (message: string) => void }) {
  const [movies, setMovies] = useState<MovieSummary[]>([]);
  const [project, setProject] = useState<MovieProject | null>(null);
  const [creating, setCreating] = useState(true);
  const [prompt, setPrompt] = useState("");
  const [settings, setSettings] = useState(() => ({ ...defaultSettings, comfyRoot: initialComfyRoot || defaultSettings.comfyRoot }));
  const [advanced, setAdvanced] = useState(false);
  const [pauseAfterPlan, setPauseAfterPlan] = useState(false);
  const [busy, setBusy] = useState(false);
  const [edits, setEdits] = useState<ClipEdit[]>([]);
  const [references, setReferences] = useState<PendingMovieReference[]>([]);

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
    let active = true;
    const timer = window.setInterval(() => void getMovie(project.id).then((next) => {
      if (!active) return;
      setProject(next); setEdits(next.edit.clips);
    }).catch(() => undefined), 2500);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [project?.id, project?.status]);

  const openProject = async (id: string) => {
    try {
      const next = await getMovie(id);
      setProject(next); setEdits(next.edit.clips); setCreating(false);
    } catch (error) { onError(String(error)); }
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
        pauseAfterPlan: advancedEnabled && pauseAfterPlan,
      });
      setProject(next); setEdits(next.edit.clips); setCreating(false); await refreshList();
    } catch (error) { onError(String(error)); } finally { setBusy(false); }
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
          <MovieLaunch prompt={prompt} settings={settings} references={references} advanced={advanced} advancedEnabled={advancedEnabled} busy={busy}
            pauseAfterPlan={pauseAfterPlan} onPauseAfterPlan={setPauseAfterPlan}
            onPrompt={setPrompt} onSettings={setSettings} onReferences={setReferences} onAttach={() => void attachReferences()} onAdvanced={setAdvanced} onMake={() => void makeMovie()} />
        ) : (
          <MovieProjectView project={project} edits={edits} busy={busy} advancedEnabled={advancedEnabled} onError={onError} onEdit={updateEdit}
            onProject={(next) => { setProject(next); setEdits(next.edit.clips); void refreshList(); }}
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

function MovieLaunch({ prompt, settings, references, advanced, advancedEnabled, busy, pauseAfterPlan, onPauseAfterPlan, onPrompt, onSettings, onReferences, onAttach, onAdvanced, onMake }: {
  prompt: string; settings: MovieSettings; references: PendingMovieReference[]; advanced: boolean; advancedEnabled: boolean; busy: boolean;
  pauseAfterPlan: boolean; onPauseAfterPlan: (value: boolean) => void;
  onPrompt: (value: string) => void; onSettings: (value: MovieSettings) => void; onReferences: (value: PendingMovieReference[]) => void;
  onAttach: () => void; onAdvanced: (value: boolean) => void; onMake: () => void;
}) {
  const quality = settings.width === 1344 ? "master" : settings.width === 864 ? "preview" : "custom";
  return <div className="movie-launch">
    <div className="movie-launch-mark"><Clapperboard /></div>
    <span className="eyebrow">Bonsai director · MiniMax H3 picture & sound</span>
    <h1>Describe the movie.<br />Kestrel runs the studio.</h1>
    <p>One prompt becomes a reviewed screenplay, continuity bible, native H3 picture-and-sound scenes, and an untouched review cut—entirely on this computer.</p>
    <div className="movie-prompt-box">
      <textarea autoFocus value={prompt} onChange={(event) => onPrompt(event.target.value)} placeholder="A short educational film explaining why the northern lights happen for a curious ten-year-old…" />
      <div><span><Check size={14} /> Bonsai drafts, reviews, and repairs every H3 scene prompt</span><button disabled={busy || prompt.trim().length < 3 || !referencesReady(references)} onClick={onMake}>{busy ? <LoaderCircle className="spin" /> : <Sparkles />} Make movie</button></div>
    </div>
    <section className="movie-reference-builder">
      <div className="movie-reference-heading"><div><span className="eyebrow">Producer references</span><strong>Show and tell H3 what must carry through</strong><small>Attach the actual media, then describe its job. Kestrel binds it natively per shot.</small></div><button disabled={busy} onClick={onAttach}><Paperclip /> Attach image, video, or audio</button></div>
      {references.length > 0 && <div className="movie-reference-grid">{references.map((reference) => {
        const labels = referenceDisplayTags(references, reference.assetId);
        return <article className="movie-reference-card" key={reference.assetId}>
          <ReferencePreview reference={reference} />
          <div className="movie-reference-copy"><div className="movie-reference-meta"><span>{labels.join(" + ")}</span><strong>{reference.name}</strong><button aria-label={`Remove ${reference.name}`} onClick={() => onReferences(references.filter((item) => item.assetId !== reference.assetId))}><X /></button></div>
            <small>{reference.kind}{reference.durationSeconds > 0 ? ` · ${reference.durationSeconds.toFixed(1)}s` : ` · ${reference.width}×${reference.height}`}</small>
            <label>How should Bonsai place this?<textarea aria-label={`Describe ${reference.name}`} value={reference.description} onChange={(event) => onReferences(references.map((item) => item.assetId === reference.assetId ? { ...item, description: event.target.value } : item))} placeholder={reference.kind === "image" ? "Character identity, costume, palette, composition, or style…" : reference.kind === "video" ? "Motion, camera move, pacing, continuation, or temporal structure…" : "Where this exact clip audio belongs: dialogue performance, music, rhythm, ambience, or effects…"} /></label>
            {reference.kind === "video" && reference.hasAudio && <><label className="movie-audio-toggle"><input type="checkbox" checked={reference.useEmbeddedAudio} onChange={(event) => onReferences(references.map((item) => item.assetId === reference.assetId ? { ...item, useEmbeddedAudio: event.target.checked } : item))} /> Use the video's existing audio as native clip audio</label>{reference.useEmbeddedAudio && <label>Where should this audio be used?<input aria-label={`Describe audio from ${reference.name}`} value={reference.embeddedAudioDescription} onChange={(event) => onReferences(references.map((item) => item.assetId === reference.assetId ? { ...item, embeddedAudioDescription: event.target.value } : item))} placeholder="The scenes or beats where this exact audio belongs…" /></label>}</>}
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
      {advancedEnabled && <label className="wide producer-pause-toggle"><span><input type="checkbox" checked={pauseAfterPlan} onChange={(event) => onPauseAfterPlan(event.target.checked)} /> Pause after Bonsai plan</span><small>Review, organize, and send feedback on the structured script before any H3 clip is rendered.</small></label>}
    </div>}
    <div className="movie-capabilities"><span><Check />98,304 context</span><span><Check />32,768 max thinking</span><span><Check />32,768 output</span><span><Check />Untouched H3 audio</span><span><Check />Crash-safe masters</span></div>
  </div>;
}

function MovieProjectView({ project, edits, busy, advancedEnabled, onError, onProject, onEdit, onNew, onCancel, onResume, onReveal, onSave, onExport }: {
  project: MovieProject; edits: ClipEdit[]; busy: boolean; advancedEnabled: boolean; onError: (message: string) => void;
  onProject: (project: MovieProject) => void; onEdit: (id: string, change: Partial<ClipEdit>) => void;
  onNew: () => void; onCancel: () => void; onResume: () => void; onReveal: () => void; onSave: () => void; onExport: () => void;
}) {
  const [draftPlan, setDraftPlan] = useState<MoviePlan | undefined>(project.plan);
  const [working, setWorking] = useState(false);
  useEffect(() => setDraftPlan(project.plan), [project.id, project.plan]);
  const complete = project.clips.filter((clip) => clip.status === "complete").length;
  const progress = project.clips.length ? Math.round((complete / project.clips.length) * 100) : project.plan ? 10 : 3;
  const canResume = ["failed", "cancelled", "interrupted"].includes(project.status) && Boolean(project.plan);
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
      <div className="movie-project-actions"><button onClick={onNew}><Plus /> New</button><button onClick={onReveal}><FolderOpen /> Files</button>{project.status === "running" && <button className="danger" onClick={onCancel}><CircleStop /> Stop safely</button>}{canResume && <button className="accent" onClick={onResume}><RotateCcw /> Resume</button>}</div>
    </header>
    <div className={`movie-status-card ${project.status}`}>
      <div>{project.status === "running" ? <LoaderCircle className="spin" /> : project.status === "complete" ? <Check /> : <Clock3 />}<span><strong>{project.detail}</strong><small>{complete} of {project.clips.length || "—"} H3 masters preserved · {project.renderer}</small></span></div>
      <div className="movie-progress"><i style={{ width: `${progress}%` }} /></div>
      {project.error && <pre>{project.error}</pre>}
    </div>
    {project.finalPath && <section className="movie-final"><div className="movie-section-heading"><div><span className="eyebrow">Assembled file</span><h2>Untouched H3 review cut</h2><small>Native clip duration and audio are preserved. Only an explicit editor export creates an altered cut.</small></div><a href={movieMediaUrl(project.finalPath)} download><Download /> Open file</a></div><video controls preload="metadata" src={movieMediaUrl(project.finalPath)} /></section>}
    {project.references.length > 0 && <section className="movie-project-references"><div className="movie-section-heading"><div><span className="eyebrow">Native H3 inputs</span><h2>Producer references</h2></div><small>Immutable copies preserved with this production</small></div><div>{project.references.map((reference) => <article key={reference.assetId}><ReferencePreview reference={reference} /><span><strong>{reference.tag}{reference.audioTag ? ` + ${reference.audioTag}` : ""} · {reference.name}</strong><small>{reference.description}</small>{reference.audioTag && <small>{reference.audioTag}: {reference.embeddedAudioDescription}</small>}</span></article>)}</div></section>}
    {advancedEnabled && project.status === "awaiting-review" && draftPlan && <ProducerPlanDesk project={project} plan={draftPlan} busy={working} onPlan={setDraftPlan}
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
      <div className="movie-section-heading"><div><span className="eyebrow">Preserved scene masters</span><h2>Scenes & sound</h2>{advancedEnabled && <small>Editor decisions are opt-in. Saving does not touch media; exporting creates a separate cut.</small>}</div>{advancedEnabled && <div><button disabled={busy} onClick={onSave}><Save /> Save decisions</button><button className="accent" disabled={busy || complete === 0 || project.status === "running"} onClick={onExport}>{busy ? <LoaderCircle className="spin" /> : <Play />} Export edited cut</button></div>}</div>
      <div className="movie-clip-grid">{project.clips.map((clip) => {
        const edit = edits.find((item) => item.clipId === clip.id) ?? { clipId: clip.id, enabled: true, order: clip.index, trimStart: 0, trimEnd: 0, audioGain: 1 };
        const planned = project.plan?.clips.find((item) => item.id === clip.id);
        return <article key={clip.id} className={`movie-clip ${clip.status} ${edit.enabled ? "" : "disabled"}`}>
          <div className="clip-preview">{clip.path ? <video controls preload="metadata" src={movieMediaUrl(clip.path)} /> : <div><LoaderCircle className={clip.status === "rendering" ? "spin" : ""} /><span>{clip.status}</span></div>}<span className="clip-number">{clip.index + 1}</span></div>
          <div className="clip-copy"><div><GripVertical /><span><strong>{clip.title}</strong><small>{clip.durationSeconds.toFixed(1)}s · seed {clip.seed}{clip.versions.length ? ` · ${clip.versions.length} preserved versions` : ""}</small></span>{advancedEnabled && <label className="clip-enable"><input type="checkbox" checked={edit.enabled} onChange={(event) => onEdit(clip.id, { enabled: event.target.checked })} /> Use in export</label>}</div>
            {planned && <div className="clip-organization"><span><b>Story job</b>{planned.purpose}</span><span><b>Transition</b>{planned.transition}</span><span><b>Continuity in</b>{planned.continuityIn}</span><span><b>Continuity out</b>{planned.continuityOut}</span>{planned.referenceIds.length > 0 && <span><b>References</b>{planned.referenceIds.map((id) => project.references.find((reference) => reference.assetId === id)?.name ?? id).join(", ")}</span>}</div>}
            <details><summary>H3 renderer direction</summary><p>{clip.prompt}</p></details>
            {advancedEnabled && <><div className="clip-controls"><NumberField label="Export order" value={edit.order + 1} min={1} max={project.clips.length} step={1} onChange={(value) => onEdit(clip.id, { order: value - 1 })} /><NumberField label="Trim in on export" value={edit.trimStart} min={0} max={Math.max(0, clip.durationSeconds - 0.1)} step={0.1} onChange={(value) => onEdit(clip.id, { trimStart: value })} /><NumberField label="Trim out on export" value={edit.trimEnd} min={0} max={Math.max(0, clip.durationSeconds - 0.1)} step={0.1} onChange={(value) => onEdit(clip.id, { trimEnd: value })} /><NumberField label="Export audio gain" value={edit.audioGain} min={0} max={4} step={0.05} onChange={(value) => onEdit(clip.id, { audioGain: value })} /></div>
            {clip.status === "complete" && planned && <SceneAssistant project={project} clip={clip} planned={planned} onProject={onProject} onError={onError} />}</>}
          </div>{clip.error && <pre>{clip.error}</pre>}
        </article>;
      })}</div>
    </section>}
  </div>;
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
