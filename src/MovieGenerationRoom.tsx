import {
  ArrowRight, Check, CircleStop, Eye, Film, LoaderCircle, Play, Save, ShieldCheck,
  Sparkles, Video,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  cancelMovieGenerationAgent, cancelMovieRender, captureMovieFrame, generateMovieFl2vBridge,
  getMovieGenerationAgentSnapshot, movieMediaUrl, onMovieGenerationAgent,
  renderMovieClipVersion, runMovieGenerationAgent, saveMovieEdits,
} from "./api";
import { appendModelThinking, ModelThinkingStream } from "./ModelThinkingStream";
import { appendTimelineSource, orderedMovieEdit, timelineItems, type TimelineItem } from "./MovieTimeline";
import { effectiveThinkingLevelForModel } from "./types";
import type {
  ControlSettings, MovieCapturedFrame, MovieEdit, MovieFrameAnchor,
  MovieGenerationProposal, MovieProject, ThinkingLevel,
} from "./types";

type GenerationMode = "shot" | "bridge";
type BridgePlacement = "add_to_masters" | "insert_after_left" | "replace_range";
type AgentRole = "director" | "reviewer";
type AgentRoleStream = { reasoning: string; text: string };

function requestId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export function bridgeAnchorsForCut(items: TimelineItem[], selectedIndex: number): [MovieFrameAnchor, MovieFrameAnchor] | undefined {
  const selected = items[selectedIndex];
  const next = items[selectedIndex + 1];
  if (!selected || !next) return undefined;
  return [{
    editId: selected.edit.id,
    timeSeconds: Math.max(selected.edit.trimStart, selected.sourceDuration - selected.edit.trimEnd - .04),
    label: `${selected.clip.title} · cut out`,
  }, {
    editId: next.edit.id,
    timeSeconds: next.edit.trimStart,
    label: `${next.clip.title} · cut in`,
  }];
}

export function editWithSourceVersion(edit: MovieEdit, editId: string, versionId: string): MovieEdit {
  return {
    ...edit,
    clips: edit.clips.map((decision) => decision.id === editId
      ? { ...decision, sourceVersionId: versionId, trimStart: 0, trimEnd: 0 }
      : decision),
  };
}

export function insertTimelineSourceAfter(edit: MovieEdit, clipId: string, afterEditId: string, id: string): MovieEdit {
  const clips = [...edit.clips].sort((left, right) => left.order - right.order);
  const after = clips.findIndex((item) => item.id === afterEditId);
  if (after < 0) return edit;
  clips.splice(after + 1, 0, {
    id, clipId, enabled: true, order: after + 1, trimStart: 0, trimEnd: 0,
    audioGain: 1, sourceVersionId: "", speed: 1, fadeIn: 0, fadeOut: 0,
    audioFadeIn: 0, audioFadeOut: 0, label: "", notes: "",
  });
  return orderedMovieEdit({ ...edit, clips });
}

export function MovieGenerationRoom({ project, edit, disabled, advanced, controlSettings, preview, onProject, onEdit, onError }: {
  project: MovieProject;
  edit: MovieEdit;
  disabled: boolean;
  advanced: boolean;
  controlSettings?: ControlSettings;
  preview?: React.ReactNode;
  onProject: (project: MovieProject) => void;
  onEdit: (edit: MovieEdit) => void;
  onError: (message: string) => void;
}) {
  const items = useMemo(() => timelineItems(project, edit), [edit, project]);
  const [mode, setMode] = useState<GenerationMode>("shot");
  const [selectedEditId, setSelectedEditId] = useState(items[0]?.edit.id ?? "");
  const selectedIndex = Math.max(0, items.findIndex((item) => item.edit.id === selectedEditId));
  const selected = items[selectedIndex] ?? items[0];
  const next = items[selectedIndex + 1];
  const [firstAnchor, setFirstAnchor] = useState<MovieFrameAnchor>();
  const [lastAnchor, setLastAnchor] = useState<MovieFrameAnchor>();
  const [firstFrame, setFirstFrame] = useState<MovieCapturedFrame>();
  const [lastFrame, setLastFrame] = useState<MovieCapturedFrame>();
  const [direction, setDirection] = useState("");
  const [duration, setDuration] = useState(5);
  const [shotDuration, setShotDuration] = useState(selected?.clip.durationSeconds ?? 5);
  const [seed, setSeed] = useState(project.settings.seed || project.clips[0]?.seed || 0);
  const [placement, setPlacement] = useState<BridgePlacement>("add_to_masters");
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel | "default">("default");
  const [activeRequestId, setActiveRequestId] = useState("");
  const [checkpointRequestId, setCheckpointRequestId] = useState("");
  const activeRequestIdRef = useRef("");
  const [roleStreams, setRoleStreams] = useState<Record<AgentRole, AgentRoleStream>>({
    director: { reasoning: "", text: "" }, reviewer: { reasoning: "", text: "" },
  });
  const [activities, setActivities] = useState<string[]>([]);
  const [activeRole, setActiveRole] = useState("director");
  const [proposal, setProposal] = useState<MovieGenerationProposal>();
  const [renderPrompt, setRenderPrompt] = useState("");
  const [rendering, setRendering] = useState(false);
  const [snapshot, setSnapshot] = useState<unknown>();
  const [generatedVersionId, setGeneratedVersionId] = useState("");
  const [generatedMasterId, setGeneratedMasterId] = useState("");
  const unplacedMasters = useMemo(() => {
    const placed = new Set(edit.clips.map((item) => item.clipId));
    return project.clips.filter((clip) => clip.status === "complete" && clip.path && !placed.has(clip.id));
  }, [edit.clips, project.clips]);

  useEffect(() => {
    if (selectedEditId && items.some((item) => item.edit.id === selectedEditId)) return;
    setSelectedEditId(items[0]?.edit.id ?? "");
  }, [items, selectedEditId]);

  useEffect(() => {
    setProposal(undefined);
    setRenderPrompt("");
    setGeneratedVersionId("");
    setCheckpointRequestId("");
    if (selected) setShotDuration(selected.clip.durationSeconds);
  }, [mode, selectedEditId]);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    void onMovieGenerationAgent((event) => {
      if (event.projectId !== project.id || event.requestId !== activeRequestIdRef.current) return;
      const role: AgentRole = event.modelRole === "reviewer" ? "reviewer" : "director";
      setActiveRole(role);
      if (event.kind === "reasoning") setRoleStreams((value) => ({ ...value, [role]: { ...value[role], reasoning: appendModelThinking(value[role].reasoning, event.content) } }));
      else if (event.kind === "token") setRoleStreams((value) => ({ ...value, [role]: { ...value[role], text: value[role].text + event.content } }));
      else if (event.kind === "turn-start") {
        setRoleStreams((value) => ({ ...value, [role]: { reasoning: "", text: "" } }));
        setActivities((value) => [...value.slice(-7), event.content]);
      } else if (event.kind === "activity" || event.kind === "complete") {
        setActivities((value) => [...value.slice(-7), event.content]);
      }
    }).then((unlisten) => { dispose = unlisten; });
    return () => dispose?.();
  }, [project.id]);

  const setCutAnchors = async () => {
    const anchors = bridgeAnchorsForCut(items, selectedIndex);
    if (!anchors) {
      onError("Select a storyline shot that has a following shot.");
      return;
    }
    const [first, last] = anchors;
    setCheckpointRequestId("");
    setFirstAnchor(first);
    setLastAnchor(last);
    setFirstFrame(undefined);
    setLastFrame(undefined);
    try {
      const [capturedFirst, capturedLast] = await Promise.all([
        captureMovieFrame(project.id, first),
        captureMovieFrame(project.id, last),
      ]);
      setFirstFrame(capturedFirst);
      setLastFrame(capturedLast);
    } catch (error) {
      onError(String(error));
    }
  };

  const setShotRangeAnchors = async () => {
    if (!selected) return;
    const first: MovieFrameAnchor = {
      editId: selected.edit.id,
      timeSeconds: selected.edit.trimStart,
      label: `${selected.clip.title} · range in`,
    };
    const last: MovieFrameAnchor = {
      editId: selected.edit.id,
      timeSeconds: Math.max(selected.edit.trimStart + .04, selected.sourceDuration - selected.edit.trimEnd - .04),
      label: `${selected.clip.title} · range out`,
    };
    setCheckpointRequestId("");
    setPlacement("replace_range");
    setFirstAnchor(first);
    setLastAnchor(last);
    setFirstFrame(undefined);
    setLastFrame(undefined);
    try {
      const [capturedFirst, capturedLast] = await Promise.all([
        captureMovieFrame(project.id, first),
        captureMovieFrame(project.id, last),
      ]);
      setFirstFrame(capturedFirst);
      setLastFrame(capturedLast);
    } catch (error) {
      onError(String(error));
    }
  };

  const askDirector = async () => {
    if (!selected || direction.trim().length < 3) return;
    if (mode === "bridge" && (!firstAnchor || !lastAnchor)) {
      onError("Choose the bridge endpoint frames first.");
      return;
    }
    const id = checkpointRequestId || requestId();
    activeRequestIdRef.current = id;
    setActiveRequestId(id);
    setRoleStreams({ director: { reasoning: "", text: "" }, reviewer: { reasoning: "", text: "" } });
    setActivities(["Opening the durable generative-edit workspace"]);
    setProposal(undefined);
    setSnapshot(undefined);
    try {
      const result = await runMovieGenerationAgent({
        requestId: id,
        projectId: project.id,
        task: mode === "shot"
          ? { kind: "shotVersion", clipId: selected.clip.id, direction }
          : { kind: "bridge", firstAnchor: firstAnchor!, lastAnchor: lastAnchor!, direction, durationSeconds: duration },
        thinkingLevel: thinkingLevel === "default" ? undefined : thinkingLevel,
      });
      setProposal(result);
      setRenderPrompt(result.candidate.kind === "shotVersion" ? result.candidate.clip.prompt : result.candidate.motionPrompt);
      if (result.candidate.kind === "bridge") setDuration(result.candidate.durationSeconds);
      else setShotDuration(result.candidate.clip.durationSeconds);
      if (advanced) setSnapshot(await getMovieGenerationAgentSnapshot(project.id, id));
      setCheckpointRequestId("");
      activeRequestIdRef.current = "";
      setActiveRequestId("");
    } catch (error) {
      setCheckpointRequestId(id);
      activeRequestIdRef.current = "";
      setActiveRequestId("");
      onError(String(error));
    }
  };

  const stopAgent = async () => {
    if (!activeRequestId) return;
    setCheckpointRequestId(activeRequestId);
    await cancelMovieGenerationAgent(activeRequestId);
    setActivities((value) => [...value.slice(-7), "Stop requested; durable candidate and transcript retained"]);
  };

  const renderCandidate = async () => {
    if (!selected || renderPrompt.trim().split(/\s+/).length < 120) {
      onError("H3 render direction needs at least 120 words. Ask the Director or complete the direction before rendering.");
      return;
    }
    setRendering(true);
    try {
      if (mode === "shot") {
        const planned = proposal?.candidate.kind === "shotVersion"
          ? proposal.candidate.clip
          : project.plan?.clips.find((clip) => clip.id === selected.clip.id);
        if (!planned) throw new Error("The selected shot is absent from the approved plan.");
        const updated = await renderMovieClipVersion({
          id: project.id,
          seed: seed || selected.clip.seed,
          suggestion: {
            clipId: selected.clip.id,
            summary: proposal?.summary ?? "Producer-authored scene audition",
            checklist: proposal?.candidate.checklist ?? [],
            clip: { ...planned, prompt: renderPrompt, durationSeconds: shotDuration },
          },
        });
        const source = updated.clips.find((clip) => clip.id === selected.clip.id);
        setGeneratedVersionId(source?.versions.at(-1)?.id ?? "");
        onProject(updated);
      } else {
        if (!firstAnchor || !lastAnchor) throw new Error("Choose both bridge endpoint frames.");
        const previousIds = new Set(project.clips.map((clip) => clip.id));
        const updated = await generateMovieFl2vBridge({
          id: project.id,
          firstAnchor,
          lastAnchor,
          prompt: renderPrompt,
          durationSeconds: duration,
          seed: seed || undefined,
          placement,
        });
        setGeneratedMasterId(updated.clips.find((clip) => !previousIds.has(clip.id))?.id ?? "");
        onProject(updated);
        onEdit(updated.edit);
      }
    } catch (error) {
      onError(String(error));
    } finally {
      setRendering(false);
    }
  };

  const stopRender = async () => {
    await cancelMovieRender(project.id);
    setActivities((value) => [...value.slice(-7), "H3 stop requested; completed masters and the current storyline remain unchanged"]);
  };

  const placeMaster = async (clipId: string, where: "append" | "after") => {
    if (!selected) return;
    const id = `edit-${requestId()}`;
    const candidate = where === "append"
      ? appendTimelineSource(edit, clipId, id)
      : insertTimelineSourceAfter(edit, clipId, selected.edit.id, id);
    try {
      const updated = await saveMovieEdits(project.id, candidate);
      onEdit(updated.edit);
      onProject(updated);
      setGeneratedMasterId("");
    } catch (error) {
      onError(String(error));
    }
  };

  const useVersion = async (versionId: string) => {
    if (!selected) return;
    const nextEdit = editWithSourceVersion(edit, selected.edit.id, versionId);
    try {
      const updated = await saveMovieEdits(project.id, nextEdit);
      onEdit(updated.edit);
      onProject(updated);
      setGeneratedVersionId("");
    } catch (error) {
      onError(String(error));
    }
  };

  const wordCount = renderPrompt.trim() ? renderPrompt.trim().split(/\s+/).length : 0;
  const visibleAgentRoles = (["director", "reviewer"] as AgentRole[]).filter((role) =>
    roleStreams[role].reasoning || roleStreams[role].text || (Boolean(activeRequestId) && activeRole === role));

  return <section className="generation-workspace" aria-label="Generate and auditions workspace">
    <header className="generation-command-bar">
      <div><span className="eyebrow">Producer + local Generative Director + H3</span><strong>Generate, audition, then place</strong><small>Nothing changes the storyline until you explicitly choose a placement.</small></div>
      <div role="tablist" aria-label="Generation task">
        <button role="tab" aria-selected={mode === "shot"} className={mode === "shot" ? "active" : ""} onClick={() => setMode("shot")}><Video /> Shot audition</button>
        <button role="tab" aria-selected={mode === "bridge"} className={mode === "bridge" ? "active" : ""} onClick={() => setMode("bridge")}><ArrowRight /> In-between</button>
      </div>
    </header>

    <div className="generation-storyline" aria-label="Primary storyline sources">
      {items.map((item, index) => <button key={item.edit.id} className={item.edit.id === selected?.edit.id ? "active" : ""} onClick={() => setSelectedEditId(item.edit.id)}>
        <span>{index + 1}</span><strong>{item.edit.label || item.clip.title}</strong><small>{item.outputDuration.toFixed(1)}s · {item.versionLabel}</small>
      </button>)}
    </div>

    <div className="generation-room-grid">
      <section className="generation-monitors">
        {preview ? <div className="generation-live-preview">{preview}</div> : mode === "shot" ? <div className="generation-shot-monitor">
          {selected?.sourcePath ? <video controls preload="metadata" src={movieMediaUrl(selected.sourcePath)} /> : <div className="generation-monitor-empty"><Film /><span>Select a preserved master.</span></div>}
          <header><span><strong>Source audition</strong><small>{selected?.clip.title ?? "No shot selected"}</small></span><em>Storyline unchanged</em></header>
        </div> : <>
          <div className="generation-anchor-actions"><button disabled={!next} onClick={() => void setCutAnchors()}><ArrowRight /> Use selected cut</button><button onClick={() => void setShotRangeAnchors()}><Film /> Use selected range</button></div>
          <div className="generation-anchor-monitors">
            <figure>{firstFrame ? <img src={movieMediaUrl(firstFrame.path)} alt="Bridge first frame" /> : <div><Eye /><span>First endpoint</span></div>}<figcaption>{firstAnchor?.label ?? "Choose a cut or range"}</figcaption></figure>
            <ArrowRight />
            <figure>{lastFrame ? <img src={movieMediaUrl(lastFrame.path)} alt="Bridge last frame" /> : <div><Eye /><span>Last endpoint</span></div>}<figcaption>{lastAnchor?.label ?? "Choose a cut or range"}</figcaption></figure>
          </div>
        </>}
        {selected && <section className="generation-auditions">
          <header><strong>Preserved auditions</strong><small>Active source: {selected.versionLabel}</small></header>
          <div><button className={!selected.edit.sourceVersionId ? "active" : ""} onClick={() => void useVersion("")}><Play /> Active master</button>{selected.clip.versions.map((version) => <button key={version.id} className={selected.edit.sourceVersionId === version.id ? "active" : ""} onClick={() => void useVersion(version.id)}><Play /> {version.title}<small>{version.durationSeconds.toFixed(1)}s · seed {version.seed}</small></button>)}</div>
        </section>}
        {unplacedMasters.length > 0 && <section className="generation-master-shelf" aria-label="Generated masters not yet in the storyline">
          <header><strong>Generated masters</strong><small>Preserved outside the storyline until you place one.</small></header>
          <div>{unplacedMasters.map((clip) => <article key={clip.id} className={clip.id === generatedMasterId ? "new" : ""}>
            <video controls preload="metadata" src={movieMediaUrl(clip.path)} />
            <span><strong>{clip.title}</strong><small>{clip.durationSeconds.toFixed(1)}s · seed {clip.seed}</small></span>
            <button type="button" disabled={disabled} onClick={() => void placeMaster(clip.id, "after")}><ArrowRight /> Insert after selected</button>
            <button type="button" disabled={disabled} onClick={() => void placeMaster(clip.id, "append")}><Film /> Append</button>
          </article>)}</div>
        </section>}
      </section>

      <aside className="generation-inspector">
        <header><span className="eyebrow">{mode === "shot" ? "Shot version" : "First / last frame video"}</span><strong>{selected?.clip.title ?? "Select a storyline shot"}</strong></header>
        <label>Producer direction<textarea rows={4} maxLength={8000} disabled={Boolean(activeRequestId)} value={direction} onChange={(event) => { setDirection(event.target.value); setCheckpointRequestId(""); }} placeholder={mode === "shot" ? "What should change, and what must remain identical?" : "Describe the movement and continuity needed between these endpoint frames."} /></label>
        <div className="generation-agent-controls"><label>Thinking<select value={thinkingLevel} disabled={Boolean(activeRequestId)} onChange={(event) => setThinkingLevel(event.target.value as ThinkingLevel | "default")}><option value="default">Default ({effectiveThinkingLevelForModel(controlSettings, project.modelRoles?.director.modelId || project.model)})</option><option value="off">Off</option><option value="low">Low</option><option value="medium">Medium</option><option value="high">High</option><option value="max">Max</option></select></label>{activeRequestId ? <button className="danger" onClick={() => void stopAgent()}><CircleStop /> Stop + checkpoint</button> : <button className="accent" disabled={disabled || direction.trim().length < 3 || (mode === "bridge" && (!firstAnchor || !lastAnchor))} onClick={() => void askDirector()}><Sparkles /> {checkpointRequestId ? "Resume Director" : "Ask Director"}</button>}</div>

        {visibleAgentRoles.map((role) => {
          const roleModelId = role === "reviewer"
            ? project.modelRoles?.reviewer.modelId || project.modelRoles?.director.modelId || project.model
            : project.modelRoles?.director.modelId || project.model;
          const roleModelName = role === "reviewer"
            ? project.modelRoles?.reviewer.modelName || project.modelRoles?.director.modelName || project.model
            : project.modelRoles?.director.modelName || project.model;
          const roleActive = Boolean(activeRequestId) && activeRole === role;
          return <section className="generation-agent-live" key={role}><header><strong>{role === "reviewer" ? "Fresh-context Reviewer" : "Generative Director"}</strong><small>{roleActive ? "Working live" : "Turn retained"}</small></header><ModelThinkingStream text={roleStreams[role].reasoning} active={roleActive} modelName={roleModelName} thinkingLevel={thinkingLevel === "default" ? effectiveThinkingLevelForModel(controlSettings, roleModelId) : thinkingLevel} /><div className="generation-agent-text">{roleStreams[role].text || (roleActive ? "The local model is preparing its next checked workspace action…" : "No visible prose in this typed tool turn.")}</div>{role === activeRole && <ol>{activities.map((activity, index) => <li key={`${index}-${activity}`}>{activity}</li>)}</ol>}</section>;
        })}

        {proposal && <section className="generation-review-result"><ShieldCheck /><span><strong>Fresh reviewer passed this candidate</strong><small>{proposal.reviewSummary}</small></span></section>}
        <label>H3 renderer direction <span className={wordCount >= 120 && wordCount <= 450 ? "valid" : ""}>{wordCount} / 120–450 words</span><textarea rows={10} maxLength={65536} disabled={rendering || Boolean(activeRequestId)} value={renderPrompt} onChange={(event) => setRenderPrompt(event.target.value)} placeholder="You may write the complete H3 direction yourself, or ask the Director above." /></label>
        <div className="generation-render-settings"><label>Duration<input type="number" min={1} max={15} step={1} value={mode === "bridge" ? duration : shotDuration} onChange={(event) => { const value = Number(event.target.value); if (mode === "bridge") { setDuration(value); setCheckpointRequestId(""); } else setShotDuration(value); }} /></label><label>Seed<input type="number" min={0} max={Number.MAX_SAFE_INTEGER} value={seed} onChange={(event) => setSeed(Number(event.target.value))} /></label></div>
        {mode === "bridge" && <fieldset><legend>After generation</legend><label><input type="radio" name="bridge-placement" checked={placement === "add_to_masters"} onChange={() => setPlacement("add_to_masters")} /> Keep as an audition in Masters</label><label><input type="radio" name="bridge-placement" checked={placement === "insert_after_left"} onChange={() => setPlacement("insert_after_left")} /> Insert after the first endpoint</label><label><input type="radio" name="bridge-placement" checked={placement === "replace_range"} onChange={() => setPlacement("replace_range")} /> Replace the selected range</label></fieldset>}
        {rendering ? <button className="generation-render-button danger" onClick={() => void stopRender()}><CircleStop /> Stop H3 audition</button> : <button className="generation-render-button" disabled={disabled || Boolean(activeRequestId) || wordCount < 120 || wordCount > 450} onClick={() => void renderCandidate()}><Video /> Generate audition</button>}
        {mode === "shot" && generatedVersionId && <button className="generation-use-button" onClick={() => void useVersion(generatedVersionId)}><Check /> Use new audition in selected storyline edit</button>}
        {advanced && Boolean(snapshot) && <details className="generation-advanced"><summary>Exact agent context, checks, transcript, and reviewer request</summary><pre>{JSON.stringify(snapshot, null, 2)}</pre></details>}
      </aside>
    </div>
    <footer><span><ShieldCheck /> Agent proposals are checked twice and independently reviewed</span><span><Save /> Candidate, transcript, and exact requests survive stop or restart</span><span><Film /> Edit remains focused on picture, sound, timing, and sequence</span></footer>
  </section>;
}
