import {
  ArrowLeft, ArrowRight, Check, CircleStop, Eye, Film, LoaderCircle, Minus, Play,
  Pause, Plus, Save, ShieldCheck, SkipBack, SkipForward, Sparkles, Video,
} from "lucide-react";
import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import {
  cancelMovieGenerationAgent, cancelMovieRender, captureMovieFrame, generateMovieFl2vTransition,
  getMovieGenerationAgentSnapshot, movieMediaUrl, onMovieGenerationAgent,
  renderMovieClipVersion, runMovieGenerationAgent, saveMovieEdits,
} from "./api";
import { appendModelThinking, ModelThinkingStream } from "./ModelThinkingStream";
import { appendTimelineSource, formatTimecode, orderedMovieEdit, timelineItems, type TimelineItem } from "./MovieTimeline";
import { effectiveThinkingLevelForModel } from "./types";
import type {
  ControlSettings, MovieCapturedFrame, MovieEdit, MovieFrameAnchor,
  MovieGenerationProposal, MovieProject, MovieTransitionPlacement, MovieTransitionPosition,
  ThinkingLevel,
} from "./types";

type GenerationMode = "shot" | "transition";
type AgentRole = "director" | "reviewer";
type AgentRoleStream = { reasoning: string; text: string };
const FPS = 24;
const FRAME_SECONDS = 1 / FPS;

function requestId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function visibleStart(item: TimelineItem): number {
  return item.edit.trimStart;
}

function visibleEnd(item: TimelineItem): number {
  return Math.max(item.edit.trimStart, item.sourceDuration - item.edit.trimEnd - .04);
}

export function parseGenerationTimecode(value: string, fps = FPS): number | undefined {
  const trimmed = value.trim();
  if (/^\d+(?:\.\d+)?$/.test(trimmed)) {
    const seconds = Number(trimmed);
    return Number.isFinite(seconds) ? seconds : undefined;
  }
  const match = /^(\d{1,3}):(\d{1,2}):(\d{1,2}):(\d{1,2})$/.exec(trimmed);
  if (!match) return undefined;
  const [, hoursText, minutesText, secondsText, framesText] = match;
  const hours = Number(hoursText);
  const minutes = Number(minutesText);
  const seconds = Number(secondsText);
  const frames = Number(framesText);
  if (minutes >= 60 || seconds >= 60 || frames >= fps) return undefined;
  return hours * 3600 + minutes * 60 + seconds + frames / fps;
}

export function boundedGenerationFrame(value: number, minimum: number, maximum: number, fps = FPS): number {
  const bounded = Math.max(minimum, Math.min(maximum, Number.isFinite(value) ? value : minimum));
  return Math.max(minimum, Math.min(maximum, Math.round(bounded * fps) / fps));
}

export function replacementRangeAnchors(item: TimelineItem, inSeconds: number, outSeconds: number): {
  firstAnchor: MovieFrameAnchor;
  lastAnchor: MovieFrameAnchor;
} {
  const minimum = visibleStart(item);
  const maximum = visibleEnd(item);
  const firstTime = boundedGenerationFrame(inSeconds, minimum, Math.max(minimum, maximum - FRAME_SECONDS));
  const lastTime = boundedGenerationFrame(outSeconds, Math.min(maximum, firstTime + FRAME_SECONDS), maximum);
  return {
    firstAnchor: { editId: item.edit.id, timeSeconds: firstTime, label: `${item.clip.title} · replacement in` },
    lastAnchor: { editId: item.edit.id, timeSeconds: lastTime, label: `${item.clip.title} · replacement out` },
  };
}

function TimecodeField({ label, value, minimum, maximum, disabled, onCommit }: {
  label: string;
  value: number;
  minimum: number;
  maximum: number;
  disabled: boolean;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState(formatTimecode(value));
  useEffect(() => setDraft(formatTimecode(value)), [value]);
  const commit = () => {
    const parsed = parseGenerationTimecode(draft);
    if (parsed === undefined || parsed < minimum || parsed > maximum) {
      setDraft(formatTimecode(value));
      return;
    }
    onCommit(parsed);
  };
  return <label className="generation-timecode-field">
    <span>{label}</span>
    <input
      aria-label={`${label} timecode`}
      disabled={disabled}
      inputMode="numeric"
      value={draft}
      onBlur={commit}
      onChange={(event) => setDraft(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === "Enter") event.currentTarget.blur();
        else if (event.key === "Escape") {
          setDraft(formatTimecode(value));
          event.currentTarget.blur();
        }
      }}
    />
  </label>;
}

export function transitionAnchorsForPosition(items: TimelineItem[], position: MovieTransitionPosition, selectedIndex: number): {
  firstAnchor?: MovieFrameAnchor;
  lastAnchor?: MovieFrameAnchor;
} | undefined {
  const selected = items[selectedIndex];
  if (!selected) return undefined;
  if (position === "before") return { lastAnchor: {
    editId: selected.edit.id, timeSeconds: visibleStart(selected), label: `${selected.clip.title} · story begins`,
  } };
  if (position === "after") return { firstAnchor: {
    editId: selected.edit.id, timeSeconds: visibleEnd(selected), label: `${selected.clip.title} · story ends`,
  } };
  const next = items[selectedIndex + 1];
  if (!next) return undefined;
  return {
    firstAnchor: { editId: selected.edit.id, timeSeconds: visibleEnd(selected), label: `${selected.clip.title} · cut out` },
    lastAnchor: { editId: next.edit.id, timeSeconds: visibleStart(next), label: `${next.clip.title} · cut in` },
  };
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

export function insertTimelineSourceBefore(edit: MovieEdit, clipId: string, beforeEditId: string, id: string): MovieEdit {
  const clips = [...edit.clips].sort((left, right) => left.order - right.order);
  const before = clips.findIndex((item) => item.id === beforeEditId);
  if (before < 0) return edit;
  clips.splice(before, 0, {
    id, clipId, enabled: true, order: before, trimStart: 0, trimEnd: 0,
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
  const [transitionPosition, setTransitionPosition] = useState<MovieTransitionPosition>("between");
  const [selectedEditId, setSelectedEditId] = useState(items[0]?.edit.id ?? "");
  const selectedIndex = Math.max(0, items.findIndex((item) => item.edit.id === selectedEditId));
  const selected = items[selectedIndex] ?? items[0];
  const [firstAnchor, setFirstAnchor] = useState<MovieFrameAnchor>();
  const [lastAnchor, setLastAnchor] = useState<MovieFrameAnchor>();
  const [firstFrame, setFirstFrame] = useState<MovieCapturedFrame>();
  const [lastFrame, setLastFrame] = useState<MovieCapturedFrame>();
  const rangeVideoRef = useRef<HTMLVideoElement>(null);
  const [rangePlayhead, setRangePlayhead] = useState(selected ? visibleStart(selected) : 0);
  const [rangePlaying, setRangePlaying] = useState(false);
  const [direction, setDirection] = useState("");
  const [duration, setDuration] = useState(5);
  const [shotDuration, setShotDuration] = useState(selected?.clip.durationSeconds ?? 5);
  const [seed, setSeed] = useState(project.settings.seed || project.clips[0]?.seed || 0);
  const [placement, setPlacement] = useState<MovieTransitionPlacement>("add_to_masters");
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

  const ensureCurrentEditSaved = async () => {
    if (JSON.stringify(orderedMovieEdit(edit)) === JSON.stringify(orderedMovieEdit(project.edit))) return;
    const updated = await saveMovieEdits(project.id, edit);
    onProject(updated);
    onEdit(updated.edit);
  };

  useEffect(() => {
    if (selectedEditId && items.some((item) => item.edit.id === selectedEditId)) return;
    setSelectedEditId(items[0]?.edit.id ?? "");
  }, [items, selectedEditId]);

  useEffect(() => {
    setProposal(undefined);
    setRenderPrompt("");
    setGeneratedVersionId("");
    setCheckpointRequestId("");
    if (selected) {
      setShotDuration(selected.clip.durationSeconds);
      setRangePlayhead(visibleStart(selected));
      setRangePlaying(false);
    }
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

  const chooseTransition = async (position: MovieTransitionPosition, index: number) => {
    const anchors = transitionAnchorsForPosition(items, position, index);
    const item = items[index];
    if (!anchors || !item) return;
    const { firstAnchor: first, lastAnchor: last } = anchors;
    setSelectedEditId(item.edit.id);
    setTransitionPosition(position);
    setCheckpointRequestId("");
    setFirstAnchor(first);
    setLastAnchor(last);
    setPlacement(position === "before" ? "insert_before_right" : "insert_after_left");
    setFirstFrame(undefined);
    setLastFrame(undefined);
    try {
      await ensureCurrentEditSaved();
      const [capturedFirst, capturedLast] = await Promise.all([
        first ? captureMovieFrame(project.id, first) : Promise.resolve(undefined),
        last ? captureMovieFrame(project.id, last) : Promise.resolve(undefined),
      ]);
      if (capturedFirst) setFirstFrame(capturedFirst);
      if (capturedLast) setLastFrame(capturedLast);
    } catch (error) {
      onError(String(error));
    }
  };

  const setShotRangeAnchors = async () => {
    if (!selected) return;
    const { firstAnchor: first, lastAnchor: last } = replacementRangeAnchors(
      selected,
      visibleStart(selected),
      visibleEnd(selected),
    );
    setCheckpointRequestId("");
    setTransitionPosition("between");
    setPlacement("replace_range");
    setFirstAnchor(first);
    setLastAnchor(last);
    setRangePlayhead(first.timeSeconds);
    setRangePlaying(false);
    setFirstFrame(undefined);
    setLastFrame(undefined);
    try {
      await ensureCurrentEditSaved();
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

  const adjustEndpoint = async (side: "first" | "last", deltaFrames: number) => {
    const anchor = side === "first" ? firstAnchor : lastAnchor;
    if (!anchor) return;
    const item = items.find((candidate) => candidate.edit.id === anchor.editId);
    if (!item) return;
    const timeSeconds = Math.min(visibleEnd(item), Math.max(visibleStart(item), anchor.timeSeconds + deltaFrames / 24));
    const adjusted = { ...anchor, timeSeconds };
    setCheckpointRequestId("");
    if (side === "first") {
      setFirstAnchor(adjusted);
      setFirstFrame(undefined);
    } else {
      setLastAnchor(adjusted);
      setLastFrame(undefined);
    }
    try {
      const frame = await captureMovieFrame(project.id, adjusted);
      if (side === "first") setFirstFrame(frame);
      else setLastFrame(frame);
    } catch (error) {
      onError(String(error));
    }
  };

  const updateRangeBoundary = async (side: "first" | "last", requestedTime: number, capture: boolean) => {
    if (!selected || !firstAnchor || !lastAnchor || firstAnchor.editId !== selected.edit.id || lastAnchor.editId !== selected.edit.id) return;
    const minimum = visibleStart(selected);
    const maximum = visibleEnd(selected);
    const firstLimit = Math.max(minimum, lastAnchor.timeSeconds - FRAME_SECONDS);
    const lastLimit = Math.min(maximum, firstAnchor.timeSeconds + FRAME_SECONDS);
    const timeSeconds = side === "first"
      ? boundedGenerationFrame(requestedTime, minimum, firstLimit)
      : boundedGenerationFrame(requestedTime, lastLimit, maximum);
    const anchor: MovieFrameAnchor = {
      ...(side === "first" ? firstAnchor : lastAnchor),
      timeSeconds,
    };
    setCheckpointRequestId("");
    if (side === "first") {
      setFirstAnchor(anchor);
      setFirstFrame(undefined);
    } else {
      setLastAnchor(anchor);
      setLastFrame(undefined);
    }
    if (!capture) return;
    try {
      const frame = await captureMovieFrame(project.id, anchor);
      if (side === "first") setFirstFrame(frame);
      else setLastFrame(frame);
    } catch (error) {
      onError(String(error));
    }
  };

  const seekRange = (requestedTime: number) => {
    if (!selected) return;
    const time = boundedGenerationFrame(requestedTime, visibleStart(selected), visibleEnd(selected));
    setRangePlayhead(time);
    const player = rangeVideoRef.current;
    if (player) player.currentTime = time;
  };

  const toggleRangePlayback = () => {
    const player = rangeVideoRef.current;
    if (!player || !firstAnchor || !lastAnchor) return;
    if (!player.paused) {
      player.pause();
      setRangePlaying(false);
      return;
    }
    if (player.currentTime < firstAnchor.timeSeconds || player.currentTime >= lastAnchor.timeSeconds - .01) {
      player.currentTime = firstAnchor.timeSeconds;
      setRangePlayhead(firstAnchor.timeSeconds);
    }
    void player.play().catch(() => setRangePlaying(false));
  };

  const handleRangeTimeUpdate = () => {
    const player = rangeVideoRef.current;
    if (!player || !lastAnchor) return;
    if (player.currentTime >= lastAnchor.timeSeconds - .005) {
      player.pause();
      player.currentTime = lastAnchor.timeSeconds;
      setRangePlaying(false);
      setRangePlayhead(lastAnchor.timeSeconds);
      return;
    }
    setRangePlayhead(player.currentTime);
  };

  const handleRangeKeys = (event: React.KeyboardEvent<HTMLElement>) => {
    if ((event.target as HTMLElement).matches("input, button")) return;
    if (event.key === " ") {
      event.preventDefault();
      toggleRangePlayback();
    } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      seekRange(rangePlayhead + (event.key === "ArrowLeft" ? -FRAME_SECONDS : FRAME_SECONDS));
    } else if (event.key.toLowerCase() === "i") {
      event.preventDefault();
      void updateRangeBoundary("first", rangePlayhead, true);
    } else if (event.key.toLowerCase() === "o") {
      event.preventDefault();
      void updateRangeBoundary("last", rangePlayhead, true);
    }
  };

  const askDirector = async () => {
    if (!selected || direction.trim().length < 3) return;
    if (mode === "transition" && !firstAnchor && !lastAnchor) {
      onError("Choose a story start, cut, or ending first.");
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
      await ensureCurrentEditSaved();
      const result = await runMovieGenerationAgent({
        requestId: id,
        projectId: project.id,
        task: mode === "shot"
          ? { kind: "shotVersion", clipId: selected.clip.id, direction }
          : { kind: "transition", position: transitionPosition, firstAnchor, lastAnchor, direction, durationSeconds: duration },
        thinkingLevel: thinkingLevel === "default" ? undefined : thinkingLevel,
      });
      setProposal(result);
      setRenderPrompt(result.candidate.kind === "shotVersion" ? result.candidate.clip.prompt : result.candidate.motionPrompt);
      if (result.candidate.kind === "transition") setDuration(result.candidate.durationSeconds);
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
      await ensureCurrentEditSaved();
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
        if (!firstAnchor && !lastAnchor) throw new Error("Choose a story transition position first.");
        const previousIds = new Set(project.clips.map((clip) => clip.id));
        const updated = await generateMovieFl2vTransition({
          id: project.id,
          position: transitionPosition,
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

  const placeMaster = async (clipId: string, where: "append" | "before" | "after") => {
    if (!selected) return;
    const id = `edit-${requestId()}`;
    const candidate = where === "append"
      ? appendTimelineSource(edit, clipId, id)
      : where === "before"
        ? insertTimelineSourceBefore(edit, clipId, selected.edit.id, id)
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
  const transitionReady = transitionPosition === "before"
    ? Boolean(lastAnchor) && !firstAnchor
    : transitionPosition === "after"
      ? Boolean(firstAnchor) && !lastAnchor
      : Boolean(firstAnchor && lastAnchor);
  const sameShotRange = Boolean(firstAnchor && lastAnchor && firstAnchor.editId === lastAnchor.editId);
  const rangeMinimum = selected ? visibleStart(selected) : 0;
  const rangeMaximum = selected ? visibleEnd(selected) : 0;
  const rangeSpan = Math.max(FRAME_SECONDS, rangeMaximum - rangeMinimum);
  const selectedRangeDuration = sameShotRange && firstAnchor && lastAnchor && selected
    ? Math.max(0, lastAnchor.timeSeconds - firstAnchor.timeSeconds) / Math.max(.25, selected.edit.speed)
    : 0;
  const generatedDurationDelta = duration - selectedRangeDuration;
  const rangePosition = (time: number) => `${Math.max(0, Math.min(100, (time - rangeMinimum) / rangeSpan * 100))}%`;
  const visibleAgentRoles = (["director", "reviewer"] as AgentRole[]).filter((role) =>
    roleStreams[role].reasoning || roleStreams[role].text || (Boolean(activeRequestId) && activeRole === role));

  return <section className="generation-workspace" aria-label="Generate and auditions workspace">
    <header className="generation-command-bar">
      <div><span className="eyebrow">Producer + local Generative Director + H3</span><strong>Generate, audition, then place</strong><small>Nothing changes the storyline until you explicitly choose a placement.</small></div>
      <div role="tablist" aria-label="Generation task">
        <button role="tab" aria-selected={mode === "shot"} className={mode === "shot" ? "active" : ""} onClick={() => setMode("shot")}><Video /> Shot audition</button>
        <button role="tab" aria-selected={mode === "transition"} className={mode === "transition" ? "active" : ""} onClick={() => setMode("transition")}><ArrowRight /> Story transitions</button>
      </div>
    </header>

    <div className="generation-storyline" aria-label="Story shots and existing cut points">
      {items.map((item, index) => <Fragment key={item.edit.id}>
        {index === 0 && <button type="button" className={`generation-cut-point edge ${mode === "transition" && transitionPosition === "before" && selectedIndex === 0 && Boolean(lastAnchor) ? "active" : ""}`} disabled={disabled || rendering || Boolean(activeRequestId)} onClick={() => { setMode("transition"); void chooseTransition("before", 0); }} aria-label="Generate before the story begins"><ArrowLeft /><strong>Before</strong><small>Story start</small></button>}
        <button type="button" className={`generation-story-shot ${item.edit.id === selected?.edit.id ? "active" : ""}`} onClick={() => setSelectedEditId(item.edit.id)}>
          <span>{index + 1}</span><strong>{item.edit.label || item.clip.title}</strong><small>{item.outputDuration.toFixed(1)}s · {item.versionLabel}</small>
        </button>
        {index < items.length - 1
          ? <button type="button" className={`generation-cut-point ${mode === "transition" && transitionPosition === "between" && selectedIndex === index && Boolean(firstAnchor && lastAnchor) ? "active" : ""}`} disabled={disabled || rendering || Boolean(activeRequestId)} onClick={() => { setMode("transition"); void chooseTransition("between", index); }} aria-label={`Generate at cut ${index + 1} between ${item.clip.title} and ${items[index + 1].clip.title}`}><span>{index + 1}</span><strong>Cut</strong><small>{index + 1} / {index + 2}</small></button>
          : <button type="button" className={`generation-cut-point edge ${mode === "transition" && transitionPosition === "after" && selectedIndex === index && Boolean(firstAnchor) ? "active" : ""}`} disabled={disabled || rendering || Boolean(activeRequestId)} onClick={() => { setMode("transition"); void chooseTransition("after", index); }} aria-label="Generate after the story ends"><ArrowRight /><strong>After</strong><small>Story end</small></button>}
      </Fragment>)}
    </div>

    <div className="generation-room-grid">
      <section className="generation-monitors">
        {preview ? <div className="generation-live-preview">{preview}</div> : mode === "shot" ? <div className="generation-shot-monitor">
          {selected?.sourcePath ? <video controls preload="metadata" src={movieMediaUrl(selected.sourcePath)} /> : <div className="generation-monitor-empty"><Film /><span>Select a preserved master.</span></div>}
          <header><span><strong>Source audition</strong><small>{selected?.clip.title ?? "No shot selected"}</small></span><em>Storyline unchanged</em></header>
        </div> : <div className={`generation-transition-view ${sameShotRange ? "range-active" : ""}`}>
          <div className="generation-anchor-actions"><span><strong>{sameShotRange ? "Inside selected shot" : transitionPosition === "before" ? "Before story" : transitionPosition === "after" ? "After story" : "At existing cut"}</strong><small>{sameShotRange ? "Choose exact In and Out frames; only the selected middle changes" : transitionPosition === "between" ? "Both story endpoints are locked" : "One story endpoint is locked; H3 invents the open side"}</small></span><button disabled={!selected || rendering || Boolean(activeRequestId)} onClick={() => void setShotRangeAnchors()}><Film /> {sameShotRange ? "Reset to full shot" : "Replace selected shot range"}</button></div>
          {sameShotRange && selected && firstAnchor && lastAnchor ? <section className="generation-range-editor" tabIndex={0} onKeyDown={handleRangeKeys} aria-label="Choose the source range to replace">
            <div className="generation-range-source-monitor">
              <video
                ref={rangeVideoRef}
                preload="metadata"
                src={movieMediaUrl(selected.sourcePath)}
                onClick={toggleRangePlayback}
                onLoadedMetadata={() => seekRange(firstAnchor.timeSeconds)}
                onPlay={() => setRangePlaying(true)}
                onPause={() => setRangePlaying(false)}
                onTimeUpdate={handleRangeTimeUpdate}
              />
              <span><strong>{selected.edit.label || selected.clip.title}</strong><small>Source audition · original remains preserved</small></span>
            </div>
            <div className="generation-range-transport">
              <button type="button" onClick={() => seekRange(firstAnchor.timeSeconds)} aria-label="Go to replacement In point"><SkipBack /></button>
              <button type="button" onClick={() => seekRange(rangePlayhead - FRAME_SECONDS)} aria-label="Step one frame backward"><ArrowLeft /></button>
              <button type="button" className="play" onClick={toggleRangePlayback} aria-label={rangePlaying ? "Pause selected range" : "Play selected range"}>{rangePlaying ? <Pause /> : <Play />}</button>
              <button type="button" onClick={() => seekRange(rangePlayhead + FRAME_SECONDS)} aria-label="Step one frame forward"><ArrowRight /></button>
              <button type="button" onClick={() => seekRange(lastAnchor.timeSeconds)} aria-label="Go to replacement Out point"><SkipForward /></button>
              <output aria-label="Source playhead timecode">{formatTimecode(rangePlayhead)}</output>
              <small>Space play · I set In · O set Out · arrows step frames</small>
            </div>
            <button
              type="button"
              className="generation-range-rail"
              aria-label="Seek within the visible source shot"
              onPointerDown={(event) => {
                const rect = event.currentTarget.getBoundingClientRect();
                seekRange(rangeMinimum + Math.max(0, Math.min(1, (event.clientX - rect.left) / rect.width)) * rangeSpan);
              }}
            >
              <span className="source" />
              <span className="selection" style={{ left: rangePosition(firstAnchor.timeSeconds), width: `${Math.max(0, (lastAnchor.timeSeconds - firstAnchor.timeSeconds) / rangeSpan * 100)}%` }} />
              <span className="in" style={{ left: rangePosition(firstAnchor.timeSeconds) }}><i>IN</i></span>
              <span className="out" style={{ left: rangePosition(lastAnchor.timeSeconds) }}><i>OUT</i></span>
              <span className="playhead" style={{ left: rangePosition(rangePlayhead) }} />
            </button>
            <div className="generation-range-summary">
              <span><strong>{formatTimecode(selectedRangeDuration)}</strong><small>source range replaced</small></span>
              <ArrowRight />
              <span><strong>{formatTimecode(duration)}</strong><small>new H3 audition</small></span>
              <em className={generatedDurationDelta > .01 ? "longer" : generatedDurationDelta < -.01 ? "shorter" : "same"}>{generatedDurationDelta > .01 ? "+" : ""}{generatedDurationDelta.toFixed(2)}s in story</em>
            </div>
            <div className="generation-range-boundaries">
              <div>
                <label>In point<input type="range" min={rangeMinimum} max={Math.max(rangeMinimum, lastAnchor.timeSeconds - FRAME_SECONDS)} step={FRAME_SECONDS} value={firstAnchor.timeSeconds} disabled={rendering || Boolean(activeRequestId)} onChange={(event) => void updateRangeBoundary("first", Number(event.target.value), false)} onPointerUp={(event) => void updateRangeBoundary("first", Number(event.currentTarget.value), true)} onKeyUp={(event) => { if (event.key.startsWith("Arrow")) void updateRangeBoundary("first", Number(event.currentTarget.value), true); }} /></label>
                <button type="button" disabled={rendering || Boolean(activeRequestId)} onClick={() => void updateRangeBoundary("first", firstAnchor.timeSeconds - FRAME_SECONDS, true)} aria-label="Move In point one frame earlier"><Minus /></button>
                <TimecodeField label="In" value={firstAnchor.timeSeconds} minimum={rangeMinimum} maximum={lastAnchor.timeSeconds - FRAME_SECONDS} disabled={rendering || Boolean(activeRequestId)} onCommit={(value) => void updateRangeBoundary("first", value, true)} />
                <button type="button" disabled={rendering || Boolean(activeRequestId)} onClick={() => void updateRangeBoundary("first", firstAnchor.timeSeconds + FRAME_SECONDS, true)} aria-label="Move In point one frame later"><Plus /></button>
                <button type="button" className="mark" disabled={rendering || Boolean(activeRequestId) || rangePlayhead >= lastAnchor.timeSeconds - FRAME_SECONDS} onClick={() => void updateRangeBoundary("first", rangePlayhead, true)}>Mark In</button>
              </div>
              <div>
                <label>Out point<input type="range" min={Math.min(rangeMaximum, firstAnchor.timeSeconds + FRAME_SECONDS)} max={rangeMaximum} step={FRAME_SECONDS} value={lastAnchor.timeSeconds} disabled={rendering || Boolean(activeRequestId)} onChange={(event) => void updateRangeBoundary("last", Number(event.target.value), false)} onPointerUp={(event) => void updateRangeBoundary("last", Number(event.currentTarget.value), true)} onKeyUp={(event) => { if (event.key.startsWith("Arrow")) void updateRangeBoundary("last", Number(event.currentTarget.value), true); }} /></label>
                <button type="button" disabled={rendering || Boolean(activeRequestId)} onClick={() => void updateRangeBoundary("last", lastAnchor.timeSeconds - FRAME_SECONDS, true)} aria-label="Move Out point one frame earlier"><Minus /></button>
                <TimecodeField label="Out" value={lastAnchor.timeSeconds} minimum={firstAnchor.timeSeconds + FRAME_SECONDS} maximum={rangeMaximum} disabled={rendering || Boolean(activeRequestId)} onCommit={(value) => void updateRangeBoundary("last", value, true)} />
                <button type="button" disabled={rendering || Boolean(activeRequestId)} onClick={() => void updateRangeBoundary("last", lastAnchor.timeSeconds + FRAME_SECONDS, true)} aria-label="Move Out point one frame later"><Plus /></button>
                <button type="button" className="mark" disabled={rendering || Boolean(activeRequestId) || rangePlayhead <= firstAnchor.timeSeconds + FRAME_SECONDS} onClick={() => void updateRangeBoundary("last", rangePlayhead, true)}>Mark Out</button>
              </div>
            </div>
            <div className="generation-range-endpoints">
              <figure>{firstFrame ? <img src={movieMediaUrl(firstFrame.path)} alt="Exact replacement In frame" /> : <LoaderCircle className="spin" />}<figcaption>IN · {formatTimecode(firstAnchor.timeSeconds)}</figcaption></figure>
              <span><strong>Director preserves both frames</strong><small>Only the selected middle is regenerated</small></span>
              <figure>{lastFrame ? <img src={movieMediaUrl(lastFrame.path)} alt="Exact replacement Out frame" /> : <LoaderCircle className="spin" />}<figcaption>OUT · {formatTimecode(lastAnchor.timeSeconds)}</figcaption></figure>
            </div>
          </section> : <div className="generation-anchor-monitors">
            <figure className={!firstAnchor ? "open-endpoint" : ""}>{firstFrame ? <img src={movieMediaUrl(firstFrame.path)} alt="First transition endpoint" /> : <div><Eye /><span>{transitionPosition === "before" ? "New opening" : "First endpoint"}</span></div>}<figcaption>{firstAnchor?.label ?? "H3 creates the opening frame"}</figcaption>{firstAnchor && <div className="generation-endpoint-adjust"><button type="button" disabled={rendering || Boolean(activeRequestId)} onClick={() => void adjustEndpoint("first", -1)} aria-label="Move first endpoint one frame earlier"><Minus /></button><output>{firstAnchor.timeSeconds.toFixed(2)}s</output><button type="button" disabled={rendering || Boolean(activeRequestId)} onClick={() => void adjustEndpoint("first", 1)} aria-label="Move first endpoint one frame later"><Plus /></button></div>}</figure>
            <ArrowRight />
            <figure className={!lastAnchor ? "open-endpoint" : ""}>{lastFrame ? <img src={movieMediaUrl(lastFrame.path)} alt="Last transition endpoint" /> : <div><Eye /><span>{transitionPosition === "after" ? "New ending" : "Last endpoint"}</span></div>}<figcaption>{lastAnchor?.label ?? "H3 creates the ending frame"}</figcaption>{lastAnchor && <div className="generation-endpoint-adjust"><button type="button" disabled={rendering || Boolean(activeRequestId)} onClick={() => void adjustEndpoint("last", -1)} aria-label="Move last endpoint one frame earlier"><Minus /></button><output>{lastAnchor.timeSeconds.toFixed(2)}s</output><button type="button" disabled={rendering || Boolean(activeRequestId)} onClick={() => void adjustEndpoint("last", 1)} aria-label="Move last endpoint one frame later"><Plus /></button></div>}</figure>
          </div>}
        </div>}
        {selected && <section className="generation-auditions">
          <header><strong>Preserved auditions</strong><small>Active source: {selected.versionLabel}</small></header>
          <div><button className={!selected.edit.sourceVersionId ? "active" : ""} onClick={() => void useVersion("")}><Play /> Active master</button>{selected.clip.versions.map((version) => <button key={version.id} className={selected.edit.sourceVersionId === version.id ? "active" : ""} onClick={() => void useVersion(version.id)}><Play /> {version.title}<small>{version.durationSeconds.toFixed(1)}s · seed {version.seed}</small></button>)}</div>
        </section>}
        {unplacedMasters.length > 0 && <section className="generation-master-shelf" aria-label="Generated masters not yet in the storyline">
          <header><strong>Generated masters</strong><small>Preserved outside the storyline until you place one.</small></header>
          <div>{unplacedMasters.map((clip) => <article key={clip.id} className={clip.id === generatedMasterId ? "new" : ""}>
            <video controls preload="metadata" src={movieMediaUrl(clip.path)} />
            <span><strong>{clip.title}</strong><small>{clip.durationSeconds.toFixed(1)}s · seed {clip.seed}</small></span>
            <button type="button" disabled={disabled} onClick={() => void placeMaster(clip.id, "before")}><ArrowLeft /> Insert before selected</button>
            <button type="button" disabled={disabled} onClick={() => void placeMaster(clip.id, "after")}><ArrowRight /> Insert after selected</button>
            <button type="button" disabled={disabled} onClick={() => void placeMaster(clip.id, "append")}><Film /> Append</button>
          </article>)}</div>
        </section>}
      </section>

      <aside className="generation-inspector">
        <header><span className="eyebrow">{mode === "shot" ? "Shot version" : `${transitionPosition} story transition`}</span><strong>{selected?.clip.title ?? "Select a storyline shot"}</strong></header>
        <label>Producer direction<textarea rows={4} maxLength={8000} disabled={Boolean(activeRequestId)} value={direction} onChange={(event) => { setDirection(event.target.value); setCheckpointRequestId(""); }} placeholder={mode === "shot" ? "What should change, and what must remain identical?" : transitionPosition === "before" ? "What happens before the story, and how should it arrive at the shown first frame?" : transitionPosition === "after" ? "What happens after the shown final frame?" : "Describe the movement and continuity needed between the shown cut frames."} /></label>
        <div className="generation-agent-controls"><label>Thinking<select value={thinkingLevel} disabled={Boolean(activeRequestId)} onChange={(event) => setThinkingLevel(event.target.value as ThinkingLevel | "default")}><option value="default">Default ({effectiveThinkingLevelForModel(controlSettings, project.modelRoles?.director.modelId || project.model)})</option><option value="off">Off</option><option value="low">Low</option><option value="medium">Medium</option><option value="high">High</option><option value="max">Max</option></select></label>{activeRequestId ? <button className="danger" onClick={() => void stopAgent()}><CircleStop /> Stop + checkpoint</button> : <button className="accent" disabled={disabled || direction.trim().length < 3 || (mode === "transition" && !transitionReady)} onClick={() => void askDirector()}><Sparkles /> {checkpointRequestId ? "Resume Director" : "Ask Director"}</button>}</div>

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
        <div className="generation-render-settings"><label>Duration<input type="number" min={1} max={15} step={1} value={mode === "transition" ? duration : shotDuration} onChange={(event) => { const value = Number(event.target.value); if (mode === "transition") { setDuration(value); setCheckpointRequestId(""); } else setShotDuration(value); }} /></label><label>Seed<input type="number" min={0} max={Number.MAX_SAFE_INTEGER} value={seed} onChange={(event) => setSeed(Number(event.target.value))} /></label></div>
        {mode === "transition" && <fieldset><legend>After generation</legend><label><input type="radio" name="transition-placement" checked={placement === "add_to_masters"} onChange={() => setPlacement("add_to_masters")} /> Keep as an audition in Masters</label>{!sameShotRange && <label><input type="radio" name="transition-placement" checked={placement === (transitionPosition === "before" ? "insert_before_right" : "insert_after_left")} onChange={() => setPlacement(transitionPosition === "before" ? "insert_before_right" : "insert_after_left")} /> {transitionPosition === "before" ? "Place before the story" : transitionPosition === "after" ? "Place after the story" : "Place at this cut"}</label>}{transitionPosition === "between" && <label><input type="radio" name="transition-placement" checked={placement === "replace_range"} onChange={() => setPlacement("replace_range")} /> Replace the selected endpoint range</label>}</fieldset>}
        {rendering ? <button className="generation-render-button danger" onClick={() => void stopRender()}><CircleStop /> Stop H3 audition</button> : <button className="generation-render-button" disabled={disabled || Boolean(activeRequestId) || wordCount < 120 || wordCount > 450} onClick={() => void renderCandidate()}><Video /> Generate audition</button>}
        {mode === "shot" && generatedVersionId && <button className="generation-use-button" onClick={() => void useVersion(generatedVersionId)}><Check /> Use new audition in selected storyline edit</button>}
        {advanced && Boolean(snapshot) && <details className="generation-advanced"><summary>Exact agent context, checks, transcript, and reviewer request</summary><pre>{JSON.stringify(snapshot, null, 2)}</pre></details>}
      </aside>
    </div>
    <footer><span><ShieldCheck /> Agent proposals are checked twice and independently reviewed</span><span><Save /> Candidate, transcript, and exact requests survive stop or restart</span><span><Film /> Edit remains focused on picture, sound, timing, and sequence</span></footer>
  </section>;
}
