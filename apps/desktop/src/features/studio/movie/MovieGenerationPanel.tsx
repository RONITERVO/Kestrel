import { useEffect, useRef, useState } from "react";
import { CircleStop, Sparkles } from "lucide-react";
import type { ModelInfo, MovieEdit, MovieEditorJob, MovieEditorPlacement, MovieProject } from "../../../contracts/index";
import { cancelMovieRender, getMovie, getMovieEditorState, movieMediaUrl, onMovieEditorJob, prepareMovieEditorRange, startMovieEditorGeneration } from "../../../platform/api";

export function parseTimelineSeconds(text: string): number {
  const normalized = text.trim().replace(",", ".");
  if (!/^\d+(?:\.\d+)?(?::\d{1,2}(?:\.\d+)?){0,2}$/.test(normalized)) return NaN;
  const parts = normalized.split(":").map(Number);
  if (parts.slice(0, -1).some((part) => !Number.isInteger(part))) return NaN;
  if (parts.slice(1).some((part) => part >= 60)) return NaN;
  return parts.reduce((seconds, part) => seconds * 60 + part, 0);
}

export function MovieGenerationPanel({ project, edit, playhead, duration, models, modelId, disabled, onProject, onActive, onError }: {
  project: MovieProject; edit: MovieEdit; playhead: number; duration: number; models: ModelInfo[]; modelId: string;
  disabled: boolean; onProject: (project: MovieProject) => void; onActive: (active: boolean) => void; onError: (message: string) => void;
}) {
  const [start, setStart] = useState("0");
  const [end, setEnd] = useState(Math.min(duration, 1).toFixed(2));
  const [seconds, setSeconds] = useState(5);
  const [direction, setDirection] = useState("");
  const [placement, setPlacement] = useState<MovieEditorPlacement>("masters");
  const [writer, setWriter] = useState(modelId);
  const [manual, setManual] = useState(false);
  const [prompt, setPrompt] = useState("");
  const [jobs, setJobs] = useState<MovieEditorJob[]>([]);
  const [selected, setSelected] = useState("");
  const [working, setWorking] = useState(false);
  const hash = useRef("");
  const callbacks = useRef({ onProject, onActive, onError });
  callbacks.current = { onProject, onActive, onError };
  const job = jobs.find((item) => item.id === selected) ?? jobs[0];
  const active = working || jobs.some((item) => ["preparing", "writing", "rendering"].includes(item.status));
  const rangeStart = parseTimelineSeconds(start);
  const rangeEnd = parseTimelineSeconds(end);
  const valid = Number.isFinite(rangeStart) && Number.isFinite(rangeEnd) && rangeEnd > rangeStart && rangeEnd <= duration + .0001;
  useEffect(() => { callbacks.current.onActive(active); }, [active]);
  useEffect(() => {
    let alive = true;
    let dispose: (() => void) | undefined;
    void getMovieEditorState(project.id).then((state) => {
      if (alive) { hash.current = state.editHash; setJobs(state.jobs); }
    }).catch((error) => callbacks.current.onError(String(error)));
    void onMovieEditorJob((next) => {
      if (!alive || next.projectId !== project.id) return;
      setJobs((current) => [next, ...current.filter((item) => item.id !== next.id)].slice(0, 50));
      if (["complete", "stopped", "failed"].includes(next.status)) {
        setWorking(false);
        void getMovie(project.id).then((current) => { if (alive) callbacks.current.onProject(current); }).catch((error) => callbacks.current.onError(String(error)));
      }
    }).then((unlisten) => { if (alive) dispose = unlisten; else unlisten(); });
    return () => { alive = false; dispose?.(); };
  }, [project.id]);
  useEffect(() => { void getMovieEditorState(project.id).then((state) => { hash.current = state.editHash; }).catch(() => undefined); }, [project.id, project.edit]);

  const prepare = async (generate: boolean) => {
    setWorking(true);
    try {
      const next = await prepareMovieEditorRange({ projectId: project.id, expectedEditHash: hash.current, startSeconds: rangeStart, endSeconds: rangeEnd, durationSeconds: seconds, direction, placement }, edit);
      setJobs((current) => [next, ...current].slice(0, 50)); setSelected(next.id);
      callbacks.current.onProject(await getMovie(project.id));
      hash.current = next.editHash;
      if (generate) await startMovieEditorGeneration({ projectId: project.id, jobId: next.id, modelId: manual ? "" : writer, renderPrompt: manual ? prompt : "" });
      else setWorking(false);
    } catch (error) { setWorking(false); callbacks.current.onError(String(error)); }
  };
  const startPrepared = async () => {
    if (!job) return;
    setWorking(true);
    try { await startMovieEditorGeneration({ projectId: project.id, jobId: job.id, modelId: manual ? "" : writer, renderPrompt: manual ? prompt : "" }); }
    catch (error) { setWorking(false); callbacks.current.onError(String(error)); }
  };
  return <section className="movie-generation-panel" aria-label="Generate between frames">
    <header><small>Range → direction → new take</small><h3>Generate between frames</h3><p>Keep the endpoints. Describe what happens between them.</p></header>
    <fieldset disabled={disabled || active}>
      <div className="movie-range-inputs"><div><label>Start<input aria-label="Generation range start" inputMode="decimal" value={start} onChange={(event) => setStart(event.target.value)} /></label><button aria-label="Mark generation start at playhead" onClick={() => setStart(playhead.toFixed(3))}>Mark playhead</button></div>
        <div><label>End<input aria-label="Generation range end" inputMode="decimal" value={end} onChange={(event) => setEnd(event.target.value)} /></label><button aria-label="Mark generation end at playhead" onClick={() => setEnd(playhead.toFixed(3))}>Mark playhead</button></div></div>
      <small>Seconds or hh:mm:ss. Decimal commas are accepted.</small>
      <label>What happens?<textarea maxLength={16000} aria-label="New take direction" value={direction} onChange={(event) => setDirection(event.target.value)} placeholder="Lifts his head, looks to his side, then looks back…" /></label>
      <div className="movie-range-inputs"><label>New duration<input aria-label="Generated duration" type="number" min={1} max={15} step={.25} value={seconds} onChange={(event) => setSeconds(Number(event.target.value))} /></label>
        <label>Place the take<select aria-label="Generated take placement" value={placement} onChange={(event) => setPlacement(event.target.value as MovieEditorPlacement)}><option value="masters">Keep in Masters</option><option value="replaceRange">Replace this range</option></select></label></div>
      {valid && <p className="movie-range-summary">{(rangeEnd - rangeStart).toFixed(2)}s selected{placement === "replaceRange" ? ` → ${seconds}s take · movie ${seconds >= rangeEnd - rangeStart ? "grows" : "shortens"} by ${Math.abs(seconds - rangeEnd + rangeStart).toFixed(2)}s` : " · your timeline stays as arranged"}.</p>}
      <label><input type="checkbox" checked={manual} onChange={(event) => setManual(event.target.checked)} /> Write the H3 prompt myself</label>
      {manual ? <label>H3 prompt<textarea aria-label="Manual H3 prompt" value={prompt} onChange={(event) => setPrompt(event.target.value)} maxLength={65536} /></label>
        : <label>Local writing assistant<select aria-label="Editor writing model" value={writer} onChange={(event) => setWriter(event.target.value)}><option value="">Choose a model</option>{models.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}</select><small>One response expands your direction using the two scenes’ text. H3 receives the actual endpoint frames.</small></label>}
      <div className="movie-generation-actions"><button disabled={!valid || direction.trim().length < 3 || seconds < 1 || seconds > 15} onClick={() => void prepare(false)}>Preview endpoints</button><button className="primary-button" disabled={!valid || direction.trim().length < 3 || !Number.isFinite(seconds) || seconds < 1 || seconds > 15 || (manual ? prompt.trim().length < 3 : !writer)} onClick={() => void prepare(true)}><Sparkles size={15} /> Generate take</button></div>
      <small>Preparing endpoints saves your current timeline.</small>
    </fieldset>
    {active && <button className="danger" onClick={() => void cancelMovieRender(project.id).catch((error) => onError(String(error)))}><CircleStop size={15} /> Stop generation</button>}
    {job && <section className="movie-editor-audition"><label>Recent takes<select aria-label="Editor take history" value={job.id} onChange={(event) => setSelected(event.target.value)}>{jobs.map((item) => <option key={item.id} value={item.id}>{item.direction.slice(0, 48)} · {item.status}</option>)}</select></label>
      <div className="movie-endpoint-pair">{[job.first, job.last].map((frame, index) => <figure key={index}>{frame.imagePath && <img src={movieMediaUrl(frame.imagePath)} alt={`${index ? "End" : "Start"} conditioning frame`} />}<figcaption>{index ? "End" : "Start"} · {(index ? job.endSeconds : job.startSeconds).toFixed(2)}s</figcaption></figure>)}</div>
      <p role="status">{job.detail}</p>
      {job.status === "ready" && <><p>{job.startSeconds.toFixed(2)}–{job.endSeconds.toFixed(2)}s → {job.durationSeconds}s · {job.placement === "replaceRange" ? "Replace saved range" : "Keep in Masters"}</p><button disabled={active || disabled || (manual ? prompt.trim().length < 3 : !writer)} onClick={() => void startPrepared()}>Generate this saved range · {job.durationSeconds}s</button></>}
      {job.renderPrompt && <details open={job.status === "writing"}><summary>H3 prompt</summary><p className="movie-saved-prompt">{job.renderPrompt}</p></details>}
      {job.resultPath && <video src={movieMediaUrl(job.resultPath)} controls preload="metadata" />}
      {job.rawPath && !job.resultPath && <><p>The original H3 take is preserved. Finishing its selected duration did not complete.</p><video src={movieMediaUrl(job.rawPath)} controls preload="metadata" /></>}
    </section>}
  </section>;
}
