import { useEffect, useState } from "react";
import type { AppActionDescription, ProducerWorkflowRun } from "../../contracts/index";
import { describeAppAction, executeAppAction, listAppActions, listProducerWorkflows, createProducerWorkflow, startProducerWorkflow, stopProducerWorkflow } from "../../platform/api";
import { FlowPages } from "../../shared/book/FlowPages";
import { PagedList } from "../../shared/book/PagedList";
import "./producer.css";

export function ProducerWorkspace({ onClose, onDelegate, onError }: { onClose: () => void; onDelegate: () => void; onError: (error: string) => void }) {
  const [commands, setCommands] = useState<string[]>([]);
  const [command, setCommand] = useState("get_producer_environment");
  const [description, setDescription] = useState<AppActionDescription | null>(null);
  const [args, setArgs] = useState("{}");
  const [result, setResult] = useState("");
  const [busy, setBusy] = useState(false);
  const [runs, setRuns] = useState<ProducerWorkflowRun[]>([]);
  const [plan, setPlan] = useState('{\n  "title": "My production",\n  "steps": [\n    { "id": "library", "command": "get_producer_environment", "args": {}, "wait": null }\n  ]\n}');
  const fail = (error: unknown) => onError(String(error));
  useEffect(() => { void listAppActions().then(setCommands).catch(fail); }, []);
  useEffect(() => { let live = true; void describeAppAction(command).then((value) => { if (live) setDescription(value); }).catch(fail); return () => { live = false; }; }, [command]);
  useEffect(() => {
    let live = true;
    const refresh = () => { void listProducerWorkflows().then((value) => { if (live) setRuns(value); }).catch(fail); };
    refresh(); const timer = window.setInterval(refresh, 3000);
    return () => { live = false; window.clearInterval(timer); };
  }, []);
  const perform = async (action: () => Promise<unknown>) => {
    setBusy(true);
    try { setResult(JSON.stringify(await action(), null, 2)); setRuns(await listProducerWorkflows()); }
    catch (error) { fail(error); }
    finally { setBusy(false); }
  };
  return <div className="spread offline-workspace producer-workspace">
    <section className="page page-left">
      <FlowPages label="Producer actions" footer={<div className="producer-buttons"><button className="primary-button" type="button" onClick={onDelegate}>Describe a production</button><button className="quiet-button" type="button" onClick={onClose}>Back to Control</button></div>}>
        <span className="eyebrow">Your local production desk</span>
        <h1>What would you like to make?</h1>
        <p>Describe what you need: a narrated lesson, a set of product images, a song, a slideshow or a film. The local agent can research, write, generate and assemble the work, asking you when a choice matters.</p>
        <p>Describe a production opens Computer Tasks, where you can answer questions, follow progress and stop the work. You can also write and edit directly in the Music, Image and Studio chapters.</p>
        <details className="producer-advanced"><summary>Advanced: individual app actions</summary>
        <p>Use the same native actions as an agent, with explicit arguments and recorded results.</p>
        <label>Action<select value={command} onChange={(event) => setCommand(event.target.value)}>{commands.map((name) => <option key={name}>{name}</option>)}</select></label>
        <p>{description?.guidance}</p>
        <details><summary>Arguments and native types</summary><pre>{JSON.stringify(description, null, 2)}</pre></details>
        <label>Arguments (JSON)<textarea rows={8} value={args} onChange={(event) => setArgs(event.target.value)} /></label>
        <button className="quiet-button" type="button" disabled={busy || !description || description.access === "human" || description.access === "protocol"} onClick={() => void perform(() => executeAppAction({ requestId: crypto.randomUUID(), command, args: JSON.parse(args) }))}>Run action</button>
        <p>A start action acknowledges the job. Inspect its project or get_app_job to verify completion. Actions requiring personal consent remain in their desktop controls.</p>
        </details>
        {result && <details open><summary>Native result</summary><pre>{result}</pre></details>}
      </FlowPages>
    </section>
    <section className="page page-right">
      <FlowPages label="Production workflows">
        <h2>Saved production workflows</h2>
        <p>Ask the local agent to save a workflow for a longer production. Review it here before starting or continuing. Closing Kestrel preserves progress; work resumes only when you choose.</p>
        <details className="producer-advanced"><summary>Advanced: write a plan</summary>
        <label>Plan (JSON)<textarea rows={10} value={plan} onChange={(event) => setPlan(event.target.value)} /></label>
        <button className="quiet-button" type="button" disabled={busy} onClick={() => void perform(() => createProducerWorkflow(JSON.parse(plan)))}>Save plan for review</button>
        </details>
        {!runs.length && <p>No saved workflows yet. A production can also run directly as a Computer Task.</p>}
        <PagedList items={runs} itemKey={(run) => run.id} label="Production workflows" renderItem={(run) => <article className="producer-run" key={run.id}>
          <h3>{run.plan.title}</h3><p>{run.status} · {run.nextStep} of {run.plan.steps.length} steps</p>
          {run.error && <p role="status">{run.error}</p>}
          <details><summary>Review saved plan and results</summary><pre>{JSON.stringify(run, null, 2)}</pre></details>
          <div className="producer-buttons"><button className="quiet-button" type="button" disabled={busy || run.status === "running" || run.status === "completed"} onClick={() => void perform(() => startProducerWorkflow(run.id))}>{run.status === "ready" ? "Start" : "Resume after review"}</button>
          <button className="quiet-button" type="button" disabled={busy || run.status !== "running"} onClick={() => void perform(() => stopProducerWorkflow(run.id))}>Stop scheduling</button></div>
        </article>} />
        <p>Stop scheduling prevents later steps. Use the render's cancel action to stop work already submitted.</p>
      </FlowPages>
    </section>
  </div>;
}
