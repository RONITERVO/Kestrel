import { useEffect, useState } from "react";
import type { ComputerTaskEvent, TaskMemorySnapshot } from "../../contracts/index";
import { acknowledgeTaskAction, getTaskMemory } from "../../platform/api";
import { FlowPages } from "../../shared/book/FlowPages";

/** An inspectable view of native memory. Corrections go through the conversation's composer. */
export function TaskMemoryPanel({ taskId, events, active = false, onError }: { taskId: string; events: ComputerTaskEvent[]; active?: boolean; onError: (message: string) => void }) {
  const [memory, setMemory] = useState<TaskMemorySnapshot | null>(null);
  const [loading, setLoading] = useState(false);
  const [reviewNote, setReviewNote] = useState("");
  async function review(sequence: number) {
    setLoading(true);
    try {
      await acknowledgeTaskAction(taskId, sequence, reviewNote);
      setReviewNote("");
      setMemory(await getTaskMemory(taskId));
    } catch (error) { onError(String(error)); }
    finally { setLoading(false); }
  }
  useEffect(() => {
    let current = true;
    setLoading(true);
    getTaskMemory(taskId).then((value) => { if (current) setMemory(value); }, (error) => { if (current) onError(String(error)); })
      .finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
  }, [taskId, events.length, onError]);
  const uncertain = memory?.unsettledActions[0];
  return <section className="task-memory-panel" aria-label="Working plan and memory">
    {loading && <p role="status">Reading saved memory…</p>}
    {memory && <FlowPages label="Working memory" footer={uncertain !== undefined && !active ? <div className="task-review">
      <label>What did you check?<textarea value={reviewNote} onChange={(event) => setReviewNote(event.target.value)} placeholder="For example: the new file exists and its content is correct." maxLength={4000} /></label>
      <p>Recording your review does not run anything or mark the action successful. Send a message afterward when you want to continue.</p>
      <button type="button" disabled={loading || !reviewNote.trim()} onClick={() => void review(uncertain)}>Record my review</button>
    </div> : undefined}>
      <p>These are Kestrel’s working notes. Send a message to correct or change direction. Your original messages and results remain saved.</p>
      <p>{memory.checkpoint?.summary ?? "No working notes yet. The conversation and action history are saved automatically."}</p>
      {memory.checkpoint?.plan.map((item) => <p key={item.id}><strong>{item.status === "done" ? "Done" : item.status === "inProgress" ? "In progress" : item.status === "blocked" ? "Needs attention" : "Next"}:</strong> {item.description}</p>)}
      {memory.checkpoint?.decisions.map((decision, i) => <p key={`decision-${i}`}>{decision}</p>)}
      {memory.checkpoint?.nextSteps.map((step, i) => <p key={`next-${i}`}>Next: {step}</p>)}
      {memory.unsettledActions.length > 0 && <div>
        <p role="status">{active ? "An action is still in progress. Its outcome will be saved when it finishes." : "Kestrel stopped before saving an action’s outcome. It will check what happened before making more changes. If it cannot verify the result, review the affected file or project below."}</p>
        {memory.unsettledActions.slice(0, 1).map((sequence) => <div key={sequence}>
          <p><strong>Action {sequence}:</strong> {events[sequence - 1]?.title ?? "Interrupted action"}</p>
          <p>{["path", "from", "to", "command"].map((key) => {
            const value = events[sequence - 1]?.data?.[key];
            return typeof value === "string" ? `${key}: ${value}` : "";
          }).filter(Boolean).join(" · ")}</p>
          <details><summary>Saved action preview</summary><pre>{events[sequence - 1]?.detail.slice(0, 1500) ?? "Reopen the conversation to load this record."}</pre><p>The full record is in Activity and technical details.</p></details>
        </div>)}
      </div>}
    </FlowPages>}
  </section>;
}
