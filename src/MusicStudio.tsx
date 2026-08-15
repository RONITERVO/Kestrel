import {
  AudioLines, Bot, ChevronDown, ChevronLeft, ChevronRight, CircleStop, Copy,
  Disc3, Download, FileMusic, FolderOpen, Gauge, ListMusic, LoaderCircle, Music,
  PanelLeft, Pause, Play, Plus, Save, SlidersHorizontal, Sparkles, Square, Trash2,
  WandSparkles,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  cancelMoviePromptDraft, cancelMusicGeneration, createMusicProject, getMusicProject,
  listMusicProjects, musicMediaUrl, onMoviePromptDraft, onMusicGeneration,
  onMusicProjectUpdated, pickSetupFile, revealMusicProject, saveMusicProject,
  startMoviePromptDraft, startMusicGeneration, transcribeMusicMidi,
} from "./api";
import type {
  ModelInfo, MusicGenerationEvent, MusicProject, MusicSection, MusicSummary, MusicTake,
  PromptDraftMode, PromptDraftReceipt, PromptDraftTarget,
} from "./types";

const SECTION_TAGS: MusicSection["tag"][] = [
  "Intro", "Verse", "Pre-Chorus", "Chorus", "Post-Chorus", "Bridge",
  "Instrumental", "Solo", "Break", "Outro",
];

interface CollaborationDraft {
  id: string;
  target: "musicCaption" | "musicLyrics";
  mode: PromptDraftMode;
  base: string;
  text: string;
  status: string;
  modelName: string;
  receipt?: PromptDraftReceipt;
}

export function MusicStudio({
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
  const [summaries, setSummaries] = useState<MusicSummary[]>([]);
  const [project, setProject] = useState<MusicProject>();
  const [dirty, setDirty] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [creating, setCreating] = useState(false);
  const [newOpen, setNewOpen] = useState(false);
  const [newTitle, setNewTitle] = useState("");
  const [newIdea, setNewIdea] = useState("");
  const [selectedSectionId, setSelectedSectionId] = useState("");
  const [showLibrary, setShowLibrary] = useState(true);
  const [progress, setProgress] = useState<MusicGenerationEvent>();
  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [modelId, setModelId] = useState(selectedModelId ?? models[0]?.id ?? "");
  const [draftMode, setDraftMode] = useState<PromptDraftMode>("develop");
  const [collaboration, setCollaboration] = useState<CollaborationDraft>();
  const [midiBusy, setMidiBusy] = useState(false);
  const audioRef = useRef<HTMLAudioElement>(null);
  const activeProjectId = useRef("");

  const refresh = async (preferredId?: string) => {
    const next = await listMusicProjects();
    setSummaries(next);
    const id = preferredId || activeProjectId.current || next[0]?.id;
    if (id) {
      const loaded = await getMusicProject(id);
      activeProjectId.current = loaded.id;
      setProject(loaded);
      setSelectedSectionId((current) => loaded.sections.some((section) => section.id === current) ? current : loaded.sections[0]?.id ?? "");
      setDirty(false);
    }
  };

  useEffect(() => {
    refresh().catch((error) => onError(String(error))).finally(() => setLoading(false));
    let disposed = false;
    const cleanups: Array<() => void> = [];
    void onMusicProjectUpdated((next) => {
      if (next.id !== activeProjectId.current) return;
      setProject(next);
      setDirty(false);
    }).then((cleanup) => disposed ? cleanup() : cleanups.push(cleanup));
    void onMusicGeneration((event) => {
      if (event.projectId !== activeProjectId.current) return;
      setProgress(event);
      if (["complete", "error", "cancelled"].includes(event.kind)) {
        void refresh(event.projectId).catch((error) => onError(String(error)));
      }
    }).then((cleanup) => disposed ? cleanup() : cleanups.push(cleanup));
    void onMoviePromptDraft((event) => {
      if (event.kind === "error") onError(event.content ?? "The local music collaborator stopped.");
      setCollaboration((current) => {
        if (!current || current.id !== event.requestId) return current;
        if (event.kind === "token") return { ...current, text: current.text + (event.content ?? ""), status: "writing", modelName: event.modelName ?? current.modelName };
        if (event.kind === "started") return { ...current, status: "writing", modelName: event.modelName ?? current.modelName, receipt: event.receipt };
        if (event.kind === "reasoning") return { ...current, status: "thinking", modelName: event.modelName ?? current.modelName };
        if (event.kind === "complete") return { ...current, status: "ready", modelName: event.modelName ?? current.modelName };
        if (event.kind === "limited") return { ...current, status: "checkpoint", modelName: event.modelName ?? current.modelName };
        if (event.kind === "cancelled") return { ...current, status: "checkpoint", modelName: event.modelName ?? current.modelName };
        if (event.kind === "error") return { ...current, status: "error" };
        return current;
      });
    }).then((cleanup) => disposed ? cleanup() : cleanups.push(cleanup));
    return () => { disposed = true; cleanups.forEach((cleanup) => cleanup()); };
  // The event subscriptions are intentionally stable for the lifetime of the workspace.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!modelId && (selectedModelId || models[0]?.id)) setModelId(selectedModelId ?? models[0]?.id ?? "");
  }, [modelId, models, selectedModelId]);

  const selectedSection = project?.sections.find((section) => section.id === selectedSectionId) ?? project?.sections[0];
  const activeTake = project?.takes.find((take) => take.id === project.activeTakeId && take.status === "complete")
    ?? [...(project?.takes ?? [])].reverse().find((take) => take.status === "complete");
  const busy = project?.status === "generating" || saving || creating || midiBusy;
  const assistantBusy = !!collaboration && ["queued", "thinking", "writing"].includes(collaboration.status);
  const totalBars = Math.max(1, project?.sections.reduce((sum, section) => sum + section.bars, 0) ?? 1);

  const mutate = (change: (current: MusicProject) => MusicProject) => {
    setProject((current) => current ? change(current) : current);
    setDirty(true);
  };

  const save = async (): Promise<MusicProject | undefined> => {
    if (!project) return undefined;
    setSaving(true);
    try {
      const next = await saveMusicProject(project);
      setProject(next);
      setDirty(false);
      await refresh(next.id);
      return next;
    } catch (error) {
      onError(String(error));
      return undefined;
    } finally {
      setSaving(false);
    }
  };

  const create = async () => {
    setCreating(true);
    try {
      const next = await createMusicProject({ title: newTitle, idea: newIdea, comfyRoot: initialComfyRoot ?? "" });
      activeProjectId.current = next.id;
      setProject(next);
      setSelectedSectionId(next.sections[0]?.id ?? "");
      setNewOpen(false);
      setNewTitle("");
      setNewIdea("");
      setDirty(false);
      await refresh(next.id);
    } catch (error) {
      onError(String(error));
    } finally {
      setCreating(false);
    }
  };

  const generate = async () => {
    if (!project) return;
    const saved = dirty ? await save() : project;
    if (!saved) return;
    try {
      const next = await startMusicGeneration(saved.id);
      setProject(next);
      setProgress({ projectId: next.id, takeId: next.activeTakeId, kind: "queued", phase: "queued", detail: next.detail, at: new Date().toISOString() });
    } catch (error) {
      onError(String(error));
    }
  };

  const chooseProject = async (id: string) => {
    if (id === project?.id) return;
    if (dirty && await save() === undefined) return;
    try {
      const next = await getMusicProject(id);
      activeProjectId.current = next.id;
      setProject(next);
      setSelectedSectionId(next.sections[0]?.id ?? "");
      setProgress(undefined);
      setDirty(false);
    } catch (error) {
      onError(String(error));
    }
  };

  const moveSection = (offset: number) => {
    if (!project || !selectedSection) return;
    const index = project.sections.findIndex((section) => section.id === selectedSection.id);
    const target = index + offset;
    if (target < 0 || target >= project.sections.length) return;
    mutate((current) => {
      const sections = [...current.sections];
      const [moved] = sections.splice(index, 1);
      sections.splice(target, 0, moved);
      return { ...current, sections };
    });
  };

  const addSection = () => {
    if (!project) return;
    const index = Math.max(0, project.sections.findIndex((section) => section.id === selectedSection?.id));
    const section: MusicSection = { id: stableId(), tag: "Verse", name: `Section ${project.sections.length + 1}`, bars: 8, lyrics: "", direction: "" };
    mutate((current) => {
      const sections = [...current.sections];
      sections.splice(index + 1, 0, section);
      return { ...current, sections };
    });
    setSelectedSectionId(section.id);
  };

  const duplicateSection = () => {
    if (!project || !selectedSection) return;
    const duplicate = { ...selectedSection, id: stableId(), name: `${selectedSection.name} copy` };
    const index = project.sections.findIndex((section) => section.id === selectedSection.id);
    mutate((current) => {
      const sections = [...current.sections];
      sections.splice(index + 1, 0, duplicate);
      return { ...current, sections };
    });
    setSelectedSectionId(duplicate.id);
  };

  const removeSection = () => {
    if (!project || !selectedSection || project.sections.length <= 1) return;
    const index = project.sections.findIndex((section) => section.id === selectedSection.id);
    const nextId = project.sections[index + 1]?.id ?? project.sections[index - 1]?.id ?? "";
    mutate((current) => ({ ...current, sections: current.sections.filter((section) => section.id !== selectedSection.id) }));
    setSelectedSectionId(nextId);
  };

  const seekSection = (section: MusicSection) => {
    setSelectedSectionId(section.id);
    if (!audioRef.current || !activeTake) return;
    const preceding = project?.sections.slice(0, project.sections.findIndex((item) => item.id === section.id)).reduce((sum, item) => sum + item.bars, 0) ?? 0;
    audioRef.current.currentTime = activeTake.durationSeconds * preceding / totalBars;
    setCurrentTime(audioRef.current.currentTime);
  };

  const togglePlay = () => {
    const audio = audioRef.current;
    if (!audio || !activeTake) return;
    if (audio.paused) void audio.play().catch(() => setPlaying(false));
    else audio.pause();
  };

  const startCollaboration = async (target: "musicCaption" | "musicLyrics") => {
    if (!project || !modelId) {
      onError("Choose a local model in Control before asking the music collaborator.");
      return;
    }
    const base = target === "musicCaption" ? project.caption : compiledLyrics(project);
    const id = stableId();
    const sectionPlan = project.sections.map((section) => `${section.name} [${section.tag}], ${section.bars} bars${section.direction ? `: ${section.direction}` : ""}`).join("\n");
    const context = target === "musicCaption"
      ? `Song idea:\n${project.idea}\n\nProducer section plan:\n${sectionPlan}\n\nCurrent lyrics:\n${compiledLyrics(project)}`
      : `Song idea:\n${project.idea}\n\nMusic description:\n${project.caption}\n\nProducer section plan:\n${sectionPlan}`;
    setCollaboration({ id, target, mode: draftMode, base, text: "", status: "queued", modelName: models.find((model) => model.id === modelId)?.name ?? "Local model" });
    try {
      await startMoviePromptDraft({ requestId: id, modelId, target, mode: draftMode, storyText: context, existingText: base, assetName: "", assetKind: "" });
    } catch (error) {
      setCollaboration(undefined);
      onError(String(error));
    }
  };

  const applyCollaboration = () => {
    if (!collaboration || !project || !collaboration.text.trim()) return;
    const value = collaboration.mode === "continue" && collaboration.base.trim()
      ? `${collaboration.base.trimEnd()}\n\n${collaboration.text.trimStart()}`
      : collaboration.text.trim();
    if (collaboration.target === "musicCaption") {
      mutate((current) => ({ ...current, caption: value }));
    } else {
      mutate((current) => ({ ...current, sections: applyTaggedLyrics(current.sections, value) }));
    }
    setCollaboration(undefined);
  };

  const transcribe = async (take: MusicTake) => {
    if (!project) return;
    const saved = dirty ? await save() : project;
    if (!saved) return;
    setMidiBusy(true);
    try {
      const next = await transcribeMusicMidi(saved.id, take.id);
      setProject(next);
      setDirty(false);
    } catch (error) {
      onError(String(error));
    } finally {
      setMidiBusy(false);
    }
  };

  if (loading) return <div className="music-studio-loading"><LoaderCircle className="spin" /><span>Opening private music projects…</span></div>;

  if (!project) return (
    <div className="music-studio-empty">
      <div className="music-empty-record"><Disc3 /><span /></div>
      <span className="eyebrow">Kestrel Music</span>
      <h1>Start with a feeling, hook, lyric, or full arrangement.</h1>
      <p>You own every section. A local model can help write; MiniMax Music 3 creates private stereo takes through your ComfyUI.</p>
      <button className="primary-button" onClick={() => setNewOpen(true)}><Plus /> New song</button>
      {newOpen && <NewSongDialog title={newTitle} idea={newIdea} busy={creating} onTitle={setNewTitle} onIdea={setNewIdea} onClose={() => setNewOpen(false)} onCreate={() => void create()} />}
    </div>
  );

  return (
    <div className={`music-studio ${showLibrary ? "library-visible" : ""}`}>
      <header className="music-transport">
        <div className="music-transport-left">
          <button aria-label="Toggle music library" className={showLibrary ? "active" : ""} onClick={() => setShowLibrary((value) => !value)}><PanelLeft /></button>
          <span className="music-app-badge"><Music /></span>
          <input aria-label="Song title" disabled={busy} value={project.title} onChange={(event) => mutate((current) => ({ ...current, title: event.target.value }))} />
          <span className={`music-save-state ${dirty ? "dirty" : ""}`}>{dirty ? "Edited" : "Saved"}</span>
        </div>
        <div className="music-transport-center">
          <button aria-label="Return to start" disabled={!activeTake} onClick={() => { if (audioRef.current) audioRef.current.currentTime = 0; }}><Square /></button>
          <button className="music-play" aria-label={playing ? "Pause" : "Play"} disabled={!activeTake} onClick={togglePlay}>{playing ? <Pause /> : <Play />}</button>
          <div className="music-time"><strong>{formatTime(currentTime)}</strong><small>{activeTake ? formatTime(activeTake.durationSeconds) : "--:--"}</small></div>
          <div className="music-tempo"><strong>{readBpm(project.caption) ?? "—"}</strong><small>BPM</small></div>
          <div className="music-key"><strong>{readKey(project.caption) ?? "—"}</strong><small>Key</small></div>
        </div>
        <div className="music-transport-right">
          <button disabled={!dirty || busy} onClick={() => void save()}>{saving ? <LoaderCircle className="spin" /> : <Save />} Save</button>
          {project.status === "generating"
            ? <button className="danger-button" onClick={() => void cancelMusicGeneration(project.id)}><CircleStop /> Stop safely</button>
            : <button className="primary-button" disabled={busy || assistantBusy} onClick={() => void generate()}><WandSparkles /> Create take</button>}
        </div>
      </header>

      {showLibrary && <aside className="music-library">
        <div className="music-pane-heading"><span><small>Library</small><strong>Projects</strong></span><button aria-label="New song" onClick={() => setNewOpen(true)}><Plus /></button></div>
        <div className="music-project-list">
          {summaries.map((summary) => <button key={summary.id} className={summary.id === project.id ? "active" : ""} onClick={() => void chooseProject(summary.id)}><Disc3 /><span><strong>{summary.title}</strong><small>{summary.takeCount} {summary.takeCount === 1 ? "take" : "takes"} · {summary.status}</small></span></button>)}
        </div>
        <div className="music-pane-heading takes"><span><small>Project audio</small><strong>Preserved takes</strong></span><button aria-label="Reveal project files" onClick={() => void revealMusicProject(project.id)}><FolderOpen /></button></div>
        <div className="music-take-list">
          {[...project.takes].reverse().map((take, reverseIndex) => <button key={take.id} className={take.id === project.activeTakeId ? "active" : ""} disabled={take.status !== "complete"} onClick={() => mutate((current) => ({ ...current, activeTakeId: take.id }))}><FileMusic /><span><strong>Take {project.takes.length - reverseIndex}</strong><small>{take.status === "complete" ? `${formatTime(take.durationSeconds)} · seed ${take.seed}` : take.status}</small></span>{take.status === "complete" && <Play />}</button>)}
          {!project.takes.length && <div className="music-list-empty"><AudioLines /><span>Your generated takes will stay here.</span></div>}
        </div>
        <div className="music-library-footer"><span>Offline project</span><small>Masters and receipts stay in your private library.</small></div>
      </aside>}

      <main className="music-arranger">
        <section className="music-monitor">
          <div className="music-now-playing">
            <span className={`music-record-art ${playing ? "playing" : ""}`}><Disc3 /></span>
            <div><small>{activeTake ? "Now playing" : "Generated master"}</small><strong>{activeTake ? `Take ${project.takes.findIndex((take) => take.id === activeTake.id) + 1}` : "No take yet"}</strong><span>{activeTake?.resolvedModel || "MiniMax Music 3 · local ComfyUI"}</span></div>
          </div>
          <div className="music-waveform" aria-hidden="true">{Array.from({ length: 92 }, (_, index) => <i key={index} style={{ height: `${18 + ((index * 29) % 67)}%` }} />)}<span style={{ width: activeTake ? `${Math.min(100, currentTime / Math.max(.01, activeTake.durationSeconds) * 100)}%` : "0%" }} /></div>
          <audio ref={audioRef} src={activeTake ? musicMediaUrl(activeTake.path) : undefined} preload="metadata" onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onEnded={() => setPlaying(false)} onTimeUpdate={(event) => setCurrentTime(event.currentTarget.currentTime)} />
        </section>

        {(project.status === "generating" || progress) && <section className={`music-render-strip ${progress?.kind ?? ""}`}>
          <LoaderCircle className={project.status === "generating" ? "spin" : ""} />
          <div><strong>{friendlyPhase(progress?.phase ?? project.phase)}</strong><span>{progress?.detail ?? project.detail}</span></div>
          {progress?.percent !== undefined && <div className="music-render-meter"><span style={{ width: `${progress.percent}%` }} /><small>{progress.step} / {progress.total}{progress.etaSeconds !== undefined ? ` · about ${formatEta(progress.etaSeconds)}` : ""}</small></div>}
        </section>}

        <section className="music-timeline">
          <div className="music-ruler"><span>1</span><span>9</span><span>17</span><span>25</span><span>33</span><span>41</span><span>49</span><span>57</span></div>
          <div className="music-track-row arrangement"><div className="music-track-label"><SlidersHorizontal /><span><strong>ARR</strong><small>Structure</small></span></div><div className="music-track-canvas">{project.sections.map((section, index) => <button key={section.id} className={`${section.id === selectedSection?.id ? "selected" : ""} tag-${section.tag.toLowerCase().replaceAll("-", "")}`} style={{ flexGrow: section.bars, flexBasis: `${section.bars * 18}px` }} onClick={() => seekSection(section)}><strong>{section.name}</strong><small>{section.bars} bars · [{section.tag}]</small><span>{section.direction || "Producer direction"}</span><i>{index + 1}</i></button>)}</div></div>
          <div className="music-track-row lyrics"><div className="music-track-label"><ListMusic /><span><strong>LYR</strong><small>{project.instrumental ? "Instrumental" : "Lyrics"}</small></span></div><div className="music-track-canvas">{project.sections.map((section) => <button key={section.id} className={section.id === selectedSection?.id ? "selected" : ""} style={{ flexGrow: section.bars, flexBasis: `${section.bars * 18}px` }} onClick={() => seekSection(section)}><span>{project.instrumental ? "Instrumental passage" : section.lyrics.trim().split("\n")[0] || "No lyric yet"}</span></button>)}</div></div>
          <div className="music-track-row master"><div className="music-track-label"><AudioLines /><span><strong>MIX</strong><small>Stereo master</small></span></div><div className="music-track-canvas"><div className={activeTake ? "master-region ready" : "master-region"}><span>{activeTake ? `Preserved take · ${formatTime(activeTake.durationSeconds)}` : "Generate a stereo take — no fake stems"}</span>{Array.from({ length: 64 }, (_, index) => <i key={index} style={{ height: `${12 + ((index * 17) % 75)}%` }} />)}</div></div></div>
        </section>

        <section className="music-writing-desk">
          <div className="music-writing-heading"><span><small>Music description</small><strong>Sound, performance, and production</strong></span><span className="music-structure-check"><i className={structuredCaption(project.caption) ? "ready" : ""} />{structuredCaption(project.caption) ? "Structured for Music 3" : "Describe freely or ask a local model"}</span></div>
          <textarea aria-label="Music description" disabled={busy || assistantBusy} value={project.caption} onChange={(event) => mutate((current) => ({ ...current, caption: event.target.value }))} placeholder={`Global Metadata: genre, BPM, key, emotion, production profile…\n\nVocal Details: timbre, performance, harmonies, effects…\n\nArrangement: instruments, groove, section evolution, textures, space…`} />
          <div className="music-assist-bar">
            <Bot /><select aria-label="Music collaborator model" disabled={assistantBusy || busy} value={modelId} onChange={(event) => setModelId(event.target.value)}><option value="">Choose local model</option>{models.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}</select>
            <select aria-label="Music collaborator mode" disabled={assistantBusy || busy} value={draftMode} onChange={(event) => setDraftMode(event.target.value as PromptDraftMode)}><option value="develop">Develop idea / notes</option><option value="continue">Continue exact draft</option></select>
            <button disabled={assistantBusy || busy || !modelId} onClick={() => void startCollaboration("musicCaption")}><Sparkles /> Develop description</button>
            <button disabled={assistantBusy || busy || !modelId} onClick={() => void startCollaboration("musicLyrics")}><ListMusic /> Write full lyrics</button>
          </div>
        </section>
      </main>

      <aside className="music-inspector">
        <div className="music-pane-heading inspector"><span><small>Inspector</small><strong>{selectedSection?.name ?? "Song"}</strong></span><Gauge /></div>
        {selectedSection && <div className="music-inspector-body">
          <div className="music-section-actions"><button aria-label="Move section left" disabled={busy} onClick={() => moveSection(-1)}><ChevronLeft /></button><button disabled={busy} onClick={addSection}><Plus /> Add</button><button disabled={busy} onClick={duplicateSection}><Copy /> Duplicate</button><button aria-label="Move section right" disabled={busy} onClick={() => moveSection(1)}><ChevronRight /></button><button aria-label="Remove section" disabled={busy || project.sections.length <= 1} onClick={removeSection}><Trash2 /></button></div>
          <fieldset disabled={busy}>
            <label>Section name<input value={selectedSection.name} onChange={(event) => patchSection(mutate, selectedSection.id, { name: event.target.value })} /></label>
            <div className="music-field-row"><label>Type<select value={selectedSection.tag} onChange={(event) => patchSection(mutate, selectedSection.id, { tag: event.target.value as MusicSection["tag"] })}>{SECTION_TAGS.map((tag) => <option key={tag}>{tag}</option>)}</select></label><label>Bars<input type="number" min={1} max={128} value={selectedSection.bars} onChange={(event) => { const value = event.currentTarget.valueAsNumber; if (Number.isFinite(value) && value >= 1 && value <= 128) patchSection(mutate, selectedSection.id, { bars: value }); }} /></label></div>
            <label>Section direction<textarea value={selectedSection.direction} onChange={(event) => patchSection(mutate, selectedSection.id, { direction: event.target.value })} placeholder="What enters, drops out, changes, or should be performed here…" /></label>
            <label className="lyrics-field">Lyrics<textarea disabled={project.instrumental} value={selectedSection.lyrics} onChange={(event) => patchSection(mutate, selectedSection.id, { lyrics: event.target.value })} placeholder={project.instrumental ? "Instrumental mode is on" : "Write only the words sung in this section…"} /></label>
            <label className="music-toggle"><input type="checkbox" checked={project.instrumental} onChange={(event) => mutate((current) => ({ ...current, instrumental: event.target.checked }))} /><span><strong>Instrumental</strong><small>Keep section structure, generate without sung lyrics</small></span></label>
          </fieldset>

          <details className="music-generation-settings" open>
            <summary><span><SlidersHorizontal /> Generation</span><ChevronDown /></summary>
            <fieldset disabled={busy}>
              <label>Maximum length <span>{formatTime(project.settings.maxDurationSeconds)}</span><input aria-label="Maximum song duration" type="range" min={15} max={300} step={1} value={project.settings.maxDurationSeconds} onChange={(event) => mutate((current) => ({ ...current, settings: { ...current.settings, maxDurationSeconds: Number(event.target.value) } }))} /></label>
              {advancedEnabled && <><div className="music-field-row"><label>Steps<input type="number" min={1} max={100} value={project.settings.steps} onChange={(event) => finiteSetting(event.currentTarget.valueAsNumber, 1, 100, (steps) => mutate((current) => ({ ...current, settings: { ...current.settings, steps } })))} /></label><label>Seed<input type="number" min={0} max={2147483647} value={project.settings.seed} onChange={(event) => finiteSetting(event.currentTarget.valueAsNumber, 0, 2147483647, (seed) => mutate((current) => ({ ...current, settings: { ...current.settings, seed } })))} /></label></div><div className="music-field-row"><label>CFG<input type="number" min={0} max={100} step={.1} value={project.settings.cfgScale} onChange={(event) => finiteSetting(event.currentTarget.valueAsNumber, 0, 100, (cfgScale) => mutate((current) => ({ ...current, settings: { ...current.settings, cfgScale } })))} /></label><label>Top K<input type="number" min={1} max={16384} value={project.settings.topK} onChange={(event) => finiteSetting(event.currentTarget.valueAsNumber, 1, 16384, (topK) => mutate((current) => ({ ...current, settings: { ...current.settings, topK } })))} /></label></div><label>Model<select value={project.settings.modelVariant} onChange={(event) => mutate((current) => ({ ...current, settings: { ...current.settings, modelVariant: event.target.value as MusicProject["settings"]["modelVariant"] } }))}><option value="auto">Auto · best installed</option><option value="int8">INT8 · lower VRAM</option><option value="fp16">FP16 · maximum fidelity</option></select></label><label className="music-toggle"><input type="checkbox" checked={project.settings.tiledDecode} onChange={(event) => mutate((current) => ({ ...current, settings: { ...current.settings, tiledDecode: event.target.checked } }))} /><span><strong>Tiled full-quality decode</strong><small>Lower VRAM; never changes the preserved source format</small></span></label></>}
            </fieldset>
          </details>

          {advancedEnabled && <details className="music-midi-panel">
            <summary><span><FileMusic /> Audio → editable MIDI</span><ChevronDown /></summary>
            <p>Optional MuScriptor pass. Its gated CC-BY-NC weights are not bundled and may not suit commercial delivery. Choose files you accepted and installed locally.</p>
            <fieldset disabled={busy}>
              <div className="music-path-field"><label>muscriptor.exe<input value={project.midi.executablePath} onChange={(event) => mutate((current) => ({ ...current, midi: { ...current.midi, executablePath: event.target.value } }))} /></label><button aria-label="Browse for muscriptor executable" onClick={() => void pickSetupFile("muscriptor").then((value) => value && mutate((current) => ({ ...current, midi: { ...current.midi, executablePath: value } }))).catch((error) => onError(String(error)))}><FolderOpen /></button></div>
              <div className="music-path-field"><label>Accepted checkpoint<input value={project.midi.modelPath} onChange={(event) => mutate((current) => ({ ...current, midi: { ...current.midi, modelPath: event.target.value } }))} /></label><button aria-label="Browse for MuScriptor checkpoint" onClick={() => void pickSetupFile("muscriptorModel").then((value) => value && mutate((current) => ({ ...current, midi: { ...current.midi, modelPath: value } }))).catch((error) => onError(String(error)))}><FolderOpen /></button></div>
              <label>Expected instruments<input value={project.midi.instruments} onChange={(event) => mutate((current) => ({ ...current, midi: { ...current.midi, instruments: event.target.value } }))} placeholder="acoustic_piano,acoustic_guitar,acoustic_bass" /></label>
              <button disabled={!activeTake || midiBusy} onClick={() => activeTake && void transcribe(activeTake)}>{midiBusy ? <LoaderCircle className="spin" /> : <FileMusic />} Transcribe active take</button>
              {activeTake?.midiPath && <span className="music-midi-ready"><Download /> MIDI preserved beside the take</span>}
            </fieldset>
          </details>}

          {advancedEnabled && activeTake && <details className="music-receipt"><summary><span><Gauge /> Exact generation receipt</span><ChevronDown /></summary><dl><dt>Model</dt><dd>{activeTake.resolvedModel}</dd><dt>Seed</dt><dd>{activeTake.seed}</dd><dt>Prompt ID</dt><dd>{activeTake.promptId}</dd><dt>SHA-256</dt><dd>{activeTake.sha256}</dd></dl><pre>{JSON.stringify(activeTake.exactGraph, null, 2)}</pre></details>}
        </div>}
      </aside>

      {collaboration && <section className="music-collaboration-sheet" aria-live="polite">
        <header><span><Sparkles /><strong>{collaboration.target === "musicCaption" ? "Description proposal" : "Lyrics proposal"}</strong><small>{collaboration.modelName} · {collaboration.status === "thinking" ? "thinking privately" : collaboration.status}</small></span><button aria-label="Close proposal" disabled={assistantBusy} onClick={() => setCollaboration(undefined)}>×</button></header>
        <pre>{collaboration.text || (collaboration.status === "thinking" ? "The local model is thinking before it writes…" : "Waiting for the first words…")}</pre>
        <footer>{assistantBusy ? <button onClick={() => void cancelMoviePromptDraft(collaboration.id)}><CircleStop /> Stop and keep checkpoint</button> : <><button onClick={() => setCollaboration(undefined)}>Discard</button><button className="primary-button" disabled={!collaboration.text.trim()} onClick={applyCollaboration}><Save /> Apply to project</button></>}{advancedEnabled && collaboration.receipt && <details><summary>Exact model request</summary><pre>{JSON.stringify(collaboration.receipt.exactRequest, null, 2)}</pre></details>}</footer>
      </section>}

      {newOpen && <NewSongDialog title={newTitle} idea={newIdea} busy={creating} onTitle={setNewTitle} onIdea={setNewIdea} onClose={() => setNewOpen(false)} onCreate={() => void create()} />}
    </div>
  );
}

function NewSongDialog({ title, idea, busy, onTitle, onIdea, onClose, onCreate }: { title: string; idea: string; busy: boolean; onTitle: (value: string) => void; onIdea: (value: string) => void; onClose: () => void; onCreate: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const element = dialog.current;
    if (element && !element.open) element.showModal();
    return () => { if (element?.open) element.close(); };
  }, []);
  return <dialog ref={dialog} className="music-new-dialog" aria-label="New song" onCancel={(event) => { if (busy) event.preventDefault(); }} onClose={onClose}><span className="music-dialog-icon"><Disc3 /></span><div><span className="eyebrow">New private project</span><h2>What are you hearing?</h2><p>A sentence is enough. A full A4 brief also fits. You can write every part yourself or invite any local model after the project opens.</p></div><label>Working title<input autoFocus maxLength={120} value={title} onChange={(event) => onTitle(event.target.value)} placeholder="Untitled song" /></label><label>Idea, story, hook, references, or production notes<textarea maxLength={65536} value={idea} onChange={(event) => onIdea(event.target.value)} placeholder="A slow-burning northern soul song about…" /></label><footer><button disabled={busy} onClick={() => dialog.current?.close()}>Cancel</button><button className="primary-button" disabled={busy} onClick={onCreate}>{busy ? <LoaderCircle className="spin" /> : <Plus />} Create project</button></footer></dialog>;
}

function patchSection(mutate: (change: (current: MusicProject) => MusicProject) => void, id: string, patch: Partial<MusicSection>) {
  mutate((current) => ({ ...current, sections: current.sections.map((section) => section.id === id ? { ...section, ...patch } : section) }));
}

function compiledLyrics(project: MusicProject): string {
  return project.sections.map((section) => `[${section.tag}]${section.lyrics.trim() ? `\n${section.lyrics.trim()}` : ""}`).join("\n\n");
}

export function applyTaggedLyrics(sections: MusicSection[], value: string): MusicSection[] {
  const matches = [...value.matchAll(/^\s*\[([^\]]+)\]\s*$/gm)];
  if (!matches.length) {
    if (!sections.length) return sections;
    return sections.map((section, index) => index === 0 ? { ...section, lyrics: value.trim() } : section);
  }
  const counts = new Map<string, number>();
  const merged = sections.map((section) => ({ ...section }));
  const added: MusicSection[] = [];
  let recognized = false;
  matches.forEach((match, index) => {
    const tag = normalizeTag(match[1]);
    if (!tag) return;
    recognized = true;
    const occurrence = counts.get(tag) ?? 0;
    counts.set(tag, occurrence + 1);
    const sameTagIndexes = sections.flatMap((section, sectionIndex) => section.tag === tag ? [sectionIndex] : []);
    const existingIndex = sameTagIndexes[occurrence];
    const start = (match.index ?? 0) + match[0].length;
    const end = matches[index + 1]?.index ?? value.length;
    const lyrics = value.slice(start, end).trim();
    if (existingIndex !== undefined) {
      merged[existingIndex] = { ...merged[existingIndex], lyrics };
    } else {
      added.push({ id: stableId(), tag, name: `${tag}${occurrence ? ` ${occurrence + 1}` : ""}`, bars: tag === "Intro" || tag === "Outro" ? 4 : 8, direction: "", lyrics });
    }
  });
  return recognized ? [...merged, ...added] : sections;
}

function normalizeTag(value: string): MusicSection["tag"] | undefined {
  const normalized = value.trim().toLocaleLowerCase().replaceAll("_", "-").replaceAll(" ", "-");
  return SECTION_TAGS.find((tag) => tag.toLocaleLowerCase() === normalized || tag.toLocaleLowerCase().replaceAll("-", "") === normalized.replaceAll("-", ""));
}

function stableId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (character) => {
    const random = Math.floor(Math.random() * 16);
    return (character === "x" ? random : (random & 0x3) | 0x8).toString(16);
  });
}

function structuredCaption(value: string): boolean {
  const lower = value.toLocaleLowerCase();
  return lower.includes("global metadata:") && lower.includes("vocal details:") && lower.includes("arrangement:");
}

function readBpm(value: string): string | undefined {
  return value.match(/\b(\d{2,3})\s*bpm\b/i)?.[1];
}

function readKey(value: string): string | undefined {
  return value.match(/\b([A-G](?:\s*(?:flat|sharp)|[#b])?\s+(?:major|minor))\b/i)?.[1];
}

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds)) return "00:00";
  const rounded = Math.max(0, Math.floor(seconds));
  return `${Math.floor(rounded / 60).toString().padStart(2, "0")}:${(rounded % 60).toString().padStart(2, "0")}`;
}

function formatEta(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  return minutes < 60 ? `${minutes} min` : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

function friendlyPhase(value: string): string {
  return ({ queued: "Waiting for renderer", "starting-renderer": "Loading local music studio", composing: "Composing song", sampling: "Rendering acoustic detail", decoding: "Full-quality decode", saving: "Saving output", preserving: "Preserving take", "take-ready": "Take ready" } as Record<string, string>)[value] ?? value.replaceAll("-", " ");
}

function finiteSetting(value: number, min: number, max: number, apply: (value: number) => void) {
  if (Number.isFinite(value) && value >= min && value <= max) apply(value);
}
