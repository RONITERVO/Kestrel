import { Bot, Check, ChevronRight, CircleStop, Clipboard, FileCode2, FolderOpen, History, LoaderCircle, MessageSquarePlus, MonitorCog, Play, RefreshCw, Search, Send, ShieldCheck, Sparkles, Square, Trash2, Wrench, Zap } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { cancelChatStream, deleteChatSession, getChatSession, getComputerTask, getControlSnapshot, listChatSessions, listComputerTasks, onChatStream, onComputerTaskEvent, onRuntimeProgress, openTaskArtifact, saveControlSettings, scanLocalModels, startChatStream, startComputerTask, startLocalModel, stopComputerTask, stopLocalModel } from "./api";
import type { ChatMessage, ChatSession, ChatSessionSummary, ChatStreamEvent, ComputerTaskEvent, ComputerTaskRun, ComputerTaskSummary, ControlSnapshot } from "./types";

type Props = {
  control: ControlSnapshot;
  onChanged: (control: ControlSnapshot) => void;
  onError: (message: string) => void;
};

type WorkKind = "chat" | "task";

export function OfflineWorkspace({ control, onChanged, onError }: Props) {
  const [kind, setKind] = useState<WorkKind>("chat");
  const [selectedId, setSelectedId] = useState(control.settings.selectedModelId ?? control.models[0]?.id ?? "");
  const [filter, setFilter] = useState("");
  const [settings, setSettings] = useState(control.settings);
  const [sessions, setSessions] = useState<ChatSessionSummary[]>([]);
  const [session, setSession] = useState<ChatSession | null>(null);
  const [draft, setDraft] = useState("");
  const [stream, setStream] = useState<{ requestId: string; phase: string; content: string; reasoning: string; notice?: string; metrics?: Record<string, unknown> } | null>(null);
  const [tasks, setTasks] = useState<ComputerTaskSummary[]>([]);
  const [task, setTask] = useState<ComputerTaskRun | null>(null);
  const [objective, setObjective] = useState("");
  const [access, setAccess] = useState<"workspace" | "full">("workspace");
  const [working, setWorking] = useState<"scan" | "start" | "stop" | "save" | "task" | null>(null);
  const [runtimeProgress, setRuntimeProgress] = useState<string | null>(null);
  const [newRoot, setNewRoot] = useState("");
  const chatRequestRef = useRef<string | null>(null);
  const taskRunRef = useRef<string | null>(null);
  const earlyTaskEventsRef = useRef<ComputerTaskEvent[]>([]);
  const chatEndRef = useRef<HTMLDivElement>(null);
  const selected = control.models.find((model) => model.id === selectedId);
  const visibleModels = useMemo(() => control.models.filter((model) => `${model.name} ${model.source} ${model.quantization ?? ""}`.toLowerCase().includes(filter.toLowerCase())), [control.models, filter]);

  const refreshHistory = async () => {
    const [nextSessions, nextTasks] = await Promise.all([listChatSessions(), listComputerTasks()]);
    setSessions(nextSessions);
    setTasks(nextTasks);
  };

  useEffect(() => { setSettings(control.settings); }, [control.settings]);
  useEffect(() => {
    void refreshHistory().catch(() => undefined);
    let unmounted = false;
    let chatDispose: (() => void) | undefined;
    let taskDispose: (() => void) | undefined;
    let runtimeDispose: (() => void) | undefined;
    void onChatStream(handleChatEvent).then((dispose) => { if (unmounted) dispose(); else chatDispose = dispose; });
    void onComputerTaskEvent(handleTaskEvent).then((dispose) => { if (unmounted) dispose(); else taskDispose = dispose; });
    void onRuntimeProgress((event) => setRuntimeProgress(event.detail)).then((dispose) => { if (unmounted) dispose(); else runtimeDispose = dispose; });
    const timer = window.setInterval(() => void getControlSnapshot(false).then(onChanged).catch(() => undefined), 2_500);
    return () => { unmounted = true; chatDispose?.(); taskDispose?.(); runtimeDispose?.(); window.clearInterval(timer); };
    // Event handlers use refs and functional state updates, so they remain stable for this subscription.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => { if (typeof chatEndRef.current?.scrollIntoView === "function") chatEndRef.current.scrollIntoView({ behavior: "smooth", block: "end" }); }, [session?.messages.length, stream?.content, stream?.reasoning]);

  function handleChatEvent(event: ChatStreamEvent) {
    if (chatRequestRef.current && event.requestId !== chatRequestRef.current) return;
    if (event.kind === "token" || event.kind === "reasoning") {
      setStream((current) => ({ requestId: event.requestId, phase: "generating", content: (current?.content ?? "") + (event.kind === "token" ? event.content ?? "" : ""), reasoning: (current?.reasoning ?? "") + (event.kind === "reasoning" ? event.content ?? "" : ""), metrics: current?.metrics }));
    } else if (event.kind === "metrics") {
      setStream((current) => current ? { ...current, metrics: event.data } : current);
    } else if (event.kind === "context") {
      setStream((current) => current ? { ...current, notice: event.content } : { requestId: event.requestId, phase: "preparing context", content: "", reasoning: "", notice: event.content });
    } else if (event.kind === "queued" || event.kind === "started") {
      setStream((current) => current ? { ...current, phase: event.kind } : { requestId: event.requestId, phase: event.kind, content: "", reasoning: "" });
    } else if (["done", "cancelled", "error"].includes(event.kind)) {
      const requestId = event.requestId;
      window.setTimeout(() => {
        void getChatSession(event.sessionId).then((next) => setSession(next)).catch((cause) => onError(String(cause)));
        void refreshHistory().catch(() => undefined);
        setStream((current) => current?.requestId === requestId ? null : current);
        chatRequestRef.current = null;
        if (event.kind === "error") onError(event.content ?? "The local generation stopped unexpectedly.");
      }, 80);
    }
  }

  function handleTaskEvent(event: ComputerTaskEvent) {
    if (taskRunRef.current && event.runId !== taskRunRef.current) return;
    setTask((current) => {
      if (current?.id !== event.runId) { earlyTaskEventsRef.current.push(event); return current; }
      return { ...current, status: terminalTaskStatus(event.kind, current.status), updatedAt: event.at, events: [...current.events, event], artifacts: event.kind === "artifact" && event.data?.path && !current.artifacts.includes(event.data.path) ? [...current.artifacts, event.data.path] : current.artifacts };
    });
    if (["done", "cancelled", "error", "limit"].includes(event.kind)) {
      taskRunRef.current = null;
      setWorking(null);
      void refreshHistory().catch(() => undefined);
    }
  }

  const act = async (next: typeof working, action: () => Promise<ControlSnapshot>) => {
    setWorking(next);
    try { onChanged(await action()); } catch (cause) { onError(String(cause)); } finally { setWorking(null); }
  };

  const openSession = async (summary: ChatSessionSummary) => {
    try { const next = await getChatSession(summary.id); setSession(next); setSelectedId(next.modelId); setKind("chat"); } catch (cause) { onError(String(cause)); }
  };

  const openTask = async (summary: ComputerTaskSummary) => {
    try {
      const next = await getComputerTask(summary.id);
      setTask(next);
      taskRunRef.current = ["running", "starting"].includes(next.status) ? next.id : null;
      setKind("task");
    } catch (cause) { onError(String(cause)); }
  };

  const newConversation = () => { setSession(null); setStream(null); chatRequestRef.current = null; setKind("chat"); };

  const send = async () => {
    if (!selected || !draft.trim() || stream) return;
    const message = draft.trim();
    setDraft("");
    const optimistic: ChatMessage = { id: `pending-${Date.now()}`, role: "user", content: message, createdAt: new Date().toISOString() };
    setSession((current) => current ? { ...current, messages: [...current.messages, optimistic] } : current);
    try {
      const started = await startChatStream({ sessionId: session?.id, modelId: selected.id, message, temperature: 0.2, topP: 0.9, topK: 40, maxOutputTokens: settings.maxOutputTokens });
      chatRequestRef.current = started.requestId;
      setSession(started.session);
      setStream((current) => current?.requestId === started.requestId ? current : { requestId: started.requestId, phase: "queued", content: "", reasoning: "" });
      void refreshHistory();
    } catch (cause) { setSession(session); onError(String(cause)); }
  };

  const cancelGeneration = async () => { if (stream) await cancelChatStream(stream.requestId); };

  const removeSession = async (summary: ChatSessionSummary) => {
    if (!window.confirm(`Archive “${summary.title}”? The JSON transcript is retained as a recoverable archive.`)) return;
    try { await deleteChatSession(summary.id); if (session?.id === summary.id) setSession(null); await refreshHistory(); } catch (cause) { onError(String(cause)); }
  };

  const runTask = async () => {
    if (!selected || !objective.trim()) return;
    if (access === "full" && !settings.allowFullAccessAgent) { onError("Full access is locked. Enable it in the Session Inspector and save the profile first."); return; }
    if (access === "full" && !window.confirm("Run this local model with full computer access? It may run programs and change files outside workspace folders. Every action is recorded, but the folder sandbox will not apply.")) return;
    setWorking("task");
    try {
      const run = await startComputerTask({ modelId: selected.id, objective: objective.trim(), access, maxSteps: settings.agentMaxSteps, maxOutputTokens: settings.agentMaxOutputTokens });
      const early = earlyTaskEventsRef.current.filter((event) => event.runId === run.id);
      earlyTaskEventsRef.current = earlyTaskEventsRef.current.filter((event) => event.runId !== run.id);
      const terminal = early.some((event) => ["done", "cancelled", "error", "limit"].includes(event.kind));
      taskRunRef.current = terminal ? null : run.id;
      setTask({ ...run, status: terminal ? terminalTaskStatus(early.at(-1)?.kind ?? "", run.status) : run.status, events: [...run.events, ...early] });
      if (terminal) setWorking(null);
      setObjective("");
      void refreshHistory();
    } catch (cause) { setWorking(null); onError(String(cause)); }
  };

  const stopTask = async () => { if (task) await stopComputerTask(task.id); };

  const save = async () => {
    setWorking("save");
    try { onChanged(await saveControlSettings({ ...settings, selectedModelId: selectedId || undefined })); } catch (cause) { onError(String(cause)); } finally { setWorking(null); }
  };

  const addRoot = () => { const root = newRoot.trim(); if (root && !settings.agentWorkspaceRoots.includes(root)) setSettings({ ...settings, agentWorkspaceRoots: [...settings.agentWorkspaceRoots, root] }); setNewRoot(""); };
  const updatePositiveSetting = (key: "contextWindow" | "maxOutputTokens" | "agentMaxSteps" | "agentMaxOutputTokens", value: string) => {
    const next = Number(value);
    if (Number.isFinite(next) && next > 0) setSettings((current) => ({ ...current, [key]: next }));
  };
  const active = !!stream || !!taskRunRef.current;
  const gpu = control.gpu;

  return <div className="control-plane offline-workspace">
    <aside className="model-drawer">
      <div className="control-product"><strong>KESTREL</strong><span>OFFLINE WORKSPACE</span></div>
      <div className="work-mode-switch"><button className={kind === "chat" ? "active" : ""} onClick={() => setKind("chat")}><Bot/> Chat</button><button className={kind === "task" ? "active" : ""} onClick={() => setKind("task")}><MonitorCog/> Computer</button></div>
      <div className="model-search"><Search size={14}/><input value={filter} onChange={(event) => setFilter(event.target.value)} placeholder="Find a local model"/></div>
      <div className="drawer-title"><span>MODEL</span><button title="Read-only rescan" onClick={() => void act("scan", scanLocalModels)}>{working === "scan" ? <LoaderCircle className="spin"/> : <RefreshCw/>}</button></div>
      <div className="control-models compact-models">{visibleModels.map((model) => <button key={model.id} className={selectedId === model.id ? "selected" : ""} onClick={() => { if (session && model.id !== session.modelId) newConversation(); setSelectedId(model.id); }}><Bot/><span><strong>{model.name}</strong><small>{model.source} · {model.quantization ?? "GGUF"}</small></span>{control.runtime.modelId === model.id && <i>{control.runtime.phase}</i>}</button>)}</div>
      <div className="drawer-title"><span>{kind === "chat" ? "CONVERSATIONS" : "TASK HISTORY"}</span>{kind === "chat" && <button title="New conversation" onClick={newConversation}><MessageSquarePlus/></button>}</div>
      <div className="history-list">{kind === "chat" ? sessions.map((item) => <div key={item.id} className={session?.id === item.id ? "active" : ""}><button onClick={() => void openSession(item)}><span>{item.title}</span><small>{item.messageCount} messages · {relativeTime(item.updatedAt)}</small></button><button title="Archive conversation" onClick={() => void removeSession(item)}><Trash2/></button></div>) : tasks.map((item) => <button key={item.id} className={task?.id === item.id ? "active" : ""} onClick={() => void openTask(item)}><span>{item.objective}</span><small>{item.status} · {item.eventCount} events · {relativeTime(item.updatedAt)}</small></button>)}</div>
      <div className="local-lock"><ShieldCheck/><span><strong>Offline execution</strong><small>Loopback model · durable transcripts · one inference lease</small></span></div>
    </aside>

    <section className="control-center">
      <header className="control-top"><div><span className="eyebrow">{kind === "chat" ? "LOCAL CONVERSATION" : "VISIBLE COMPUTER WORK"}</span><h1>{kind === "chat" ? session?.title ?? selected?.name ?? "Choose a model" : task?.objective ?? "Computer Tasks"}</h1></div><div className="control-actions">{control.runtime.phase === "ready" ? <button className="quiet-button" disabled={active} onClick={() => void act("stop", stopLocalModel)}>{working === "stop" ? <LoaderCircle className="spin"/> : <CircleStop/>}{control.runtime.mode === "attached" ? "Detach" : "Stop"}</button> : <button className="primary-button" disabled={!selected || !!working} onClick={() => selected && void act("start", () => startLocalModel(selected.id))}>{working === "start" ? <LoaderCircle className="spin"/> : <Play/>} Load model</button>}</div></header>
      {kind === "chat" ? <>
        <div className="control-chat" aria-live="polite">{working === "start" && runtimeProgress && <RuntimeNotice title="MODEL STARTUP" detail={runtimeProgress}/>} {session?.messages.length ? session.messages.map((message) => <Message key={message.id} message={message} model={selected?.name}/>) : <Welcome models={control.models.length} context={settings.contextWindow} freeMib={gpu?.freeMib}/>} {stream && <article className="assistant streaming"><span>{selected?.name ?? "MODEL"}<i>{stream.phase}</i></span>{stream.notice && <div className="context-notice">{stream.notice}</div>}{stream.reasoning && <details open={!stream.content}><summary>Reasoning · live</summary><pre>{stream.reasoning}</pre></details>}<RichText value={stream.content || (stream.phase === "queued" ? "Waiting for the inference slot…" : "")}/>{stream.metrics && <Metrics data={stream.metrics}/>}</article>}<div ref={chatEndRef}/></div>
        <div className="control-composer"><textarea value={draft} onChange={(event) => setDraft(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void send(); } }} placeholder={control.runtime.phase === "ready" ? "Message the active local model…" : "Load a model to begin"} disabled={control.runtime.phase !== "ready" || !!stream}/>{stream ? <button title="Stop generation" className="stop-generation" onClick={() => void cancelGeneration()}><Square/></button> : <button title="Send" onClick={() => void send()} disabled={!draft.trim() || control.runtime.phase !== "ready"}><Send/></button>}</div>
      </> : <ComputerTasks run={task} objective={objective} access={access} ready={control.runtime.phase === "ready"} running={!!taskRunRef.current} fullUnlocked={settings.allowFullAccessAgent} onObjective={setObjective} onAccess={setAccess} onRun={() => void runTask()} onStop={() => void stopTask()} onOpen={(path) => task && void openTaskArtifact(task.id, path)} />}
    </section>

    <aside className="control-inspector">
      <span className="eyebrow">SESSION INSPECTOR</span>
      <Metric label="Runtime" value={control.runtime.phase}/><Metric label="Ownership" value={control.runtime.mode}/><Metric label="Context" value={control.runtime.contextWindow ? control.runtime.contextWindow.toLocaleString() : "—"}/><Metric label="Inference" value={active ? "Active here" : control.runtime.inferenceBusy ? "Busy elsewhere" : "Available"}/>
      {gpu && <div className="control-memory"><strong>{gpu.name}</strong><div><span style={{width:`${Math.min(100, gpu.usedMib / gpu.totalMib * 100)}%`}}/></div><small>{formatMib(gpu.usedMib)} used · {formatMib(gpu.freeMib)} free · {gpu.utilizationPercent}% compute</small></div>}
      <p className="runtime-detail">{control.runtime.detail}</p>
      <details className="inspector-section" open><summary>Model profile</summary><div className="inline-runtime-settings"><label>Context<input type="number" disabled={!settings.advancedMode} value={settings.contextWindow} onChange={(event) => updatePositiveSetting("contextWindow", event.target.value)}/></label><label>Max chat output<input type="number" disabled={!settings.advancedMode} value={settings.maxOutputTokens} onChange={(event) => updatePositiveSetting("maxOutputTokens", event.target.value)}/></label><label className="check-line"><input type="checkbox" checked={settings.advancedMode} onChange={(event) => setSettings({...settings, advancedMode:event.target.checked})}/> Advanced, uncapped</label></div></details>
      <details className="inspector-section" open={kind === "task"}><summary>Computer Tasks policy</summary><div className="inline-runtime-settings"><label>Maximum steps<input type="number" value={settings.agentMaxSteps} onChange={(event) => updatePositiveSetting("agentMaxSteps", event.target.value)}/></label><label>Output per decision<input type="number" value={settings.agentMaxOutputTokens} onChange={(event) => updatePositiveSetting("agentMaxOutputTokens", event.target.value)}/></label><label className="check-line danger-toggle"><input type="checkbox" checked={settings.allowFullAccessAgent} onChange={(event) => { if (!event.target.checked || window.confirm("Unlock full computer access? Tasks will be able to run programs and operate outside workspace folders after an additional per-task confirmation.")) setSettings({...settings, allowFullAccessAgent:event.target.checked}); }}/> Unlock full access</label><div className="workspace-roots">{settings.agentWorkspaceRoots.map((root) => <div key={root}><span title={root}>{root}</span><button onClick={() => setSettings({...settings, agentWorkspaceRoots:settings.agentWorkspaceRoots.filter((value) => value !== root)})}>×</button></div>)}<span className="root-entry"><label htmlFor="approved-folder">Approved folder</label><span><input id="approved-folder" value={newRoot} onChange={(event) => setNewRoot(event.target.value)} placeholder="C:\Users\You\Work"/><button type="button" aria-label="Add approved folder" onClick={addRoot}>Add</button></span></span></div></div></details>
      {settings.advancedMode && <div className="control-warning">Invalid or oversized values can stop startup or exhaust VRAM.</div>}
      <button className="quiet-button inspector-save" disabled={!!working || active} onClick={() => void save()}>{working === "save" ? <LoaderCircle className="spin"/> : <Check/>} Save complete profile</button>
      {control.runtime.launchArgs.length > 0 && <details className="launch-proof"><summary>Exact engine launch</summary><pre>{control.runtime.launchArgs.join(" ")}</pre></details>}
      <details className="launch-proof"><summary>Live runtime feed · {control.runtimeLogs.length}</summary><pre>{control.runtimeLogs.length ? control.runtimeLogs.slice(-120).map((entry) => `[${timeOnly(entry.at)} ${entry.stream}] ${entry.line}`).join("\n") : "Attached runtimes do not expose process logs. Managed runtime output will appear here."}</pre></details>
    </aside>
  </div>;
}

function ComputerTasks({ run, objective, access, ready, running, fullUnlocked, onObjective, onAccess, onRun, onStop, onOpen }: { run: ComputerTaskRun | null; objective: string; access: "workspace" | "full"; ready: boolean; running: boolean; fullUnlocked: boolean; onObjective: (value: string) => void; onAccess: (value: "workspace" | "full") => void; onRun: () => void; onStop: () => void; onOpen: (path: string) => void }) {
  return <div className="computer-workspace">{!run || (!running && objective) ? <section className="task-launch"><div className="task-orbit"><MonitorCog/></div><span className="eyebrow">ACTUAL COMPUTER WORK</span><h2>Give the local model a bounded objective.</h2><p>Every decision, tool call, result, error, and artifact stays visible and is saved locally. Workspace mode is the everyday default.</p><textarea value={objective} onChange={(event) => onObjective(event.target.value)} placeholder="Example: Check Downloads. If the newest file is not an SVG cat, create a polished cat.svg there and report its path."/><div className="task-policy"><button className={access === "workspace" ? "active" : ""} onClick={() => onAccess("workspace")}><ShieldCheck/><span><strong>Workspace</strong><small>Only approved folders</small></span></button><button className={access === "full" ? "active danger" : ""} disabled={!fullUnlocked} onClick={() => onAccess("full")}><Zap/><span><strong>Full access</strong><small>{fullUnlocked ? "Programs and all files" : "Locked in profile"}</small></span></button></div><button className="primary-button task-run" disabled={!ready || !objective.trim()} onClick={onRun}><Play/> Start visible task</button></section> : <section className="task-run-view"><header><div><span className="eyebrow">{run.access.toUpperCase()} ACCESS · {run.status.toUpperCase()}</span><h2>{run.objective}</h2></div>{running && <button className="danger-button" onClick={onStop}><Square/> Stop safely</button>}</header><div className="task-timeline" aria-live="polite">{run.events.map((event, index) => <article key={`${event.at}-${index}`} className={`task-event ${event.kind}`}><div className="event-glyph">{event.kind === "artifact" ? <FileCode2/> : event.kind === "tool_start" ? <Wrench/> : event.kind === "thinking" || event.kind === "queued" ? <LoaderCircle className="spin"/> : event.kind === "done" ? <Check/> : <ChevronRight/>}</div><div><header><strong>{event.title}</strong><span>{event.step ? `Step ${event.step}` : "Setup"} · {timeOnly(event.at)}</span></header><pre>{event.detail}</pre>{event.kind === "artifact" && event.data?.path && <button className="artifact-button" onClick={() => onOpen(event.data!.path!)}><FolderOpen/> Open artifact</button>}</div></article>)}</div>{run.artifacts.length > 0 && <footer className="artifact-shelf"><span><FileCode2/> {run.artifacts.length} artifact{run.artifacts.length === 1 ? "" : "s"}</span>{run.artifacts.map((path) => <button key={path} onClick={() => onOpen(path)} title={path}>{fileName(path)}</button>)}</footer>}</section>}</div>;
}

function Message({ message, model }: { message: ChatMessage; model?: string }) { return <article className={message.role}><span>{message.role === "user" ? "YOU" : model ?? "MODEL"}<button title="Copy message" onClick={() => void navigator.clipboard.writeText(message.content)}><Clipboard/></button></span>{message.reasoning && <details><summary>Reasoning</summary><pre>{message.reasoning}</pre></details>}<RichText value={message.content}/></article>; }

function RichText({ value }: { value: string }) {
  if (!value) return <p className="stream-cursor">Thinking</p>;
  const blocks = value.split(/```/g);
  return <div className="rich-message">{blocks.map((block, index) => index % 2 ? <pre key={index}><code>{block.replace(/^\w+\n/, "")}</code></pre> : block.split(/\n{2,}/).filter(Boolean).map((paragraph, child) => <p key={`${index}-${child}`}>{inlineCode(paragraph)}</p>))}</div>;
}

function inlineCode(value: string) { return value.split(/(`[^`]+`)/g).map((part, index) => part.startsWith("`") && part.endsWith("`") ? <code key={index}>{part.slice(1, -1)}</code> : <span key={index}>{part}</span>); }

function Metrics({ data }: { data: Record<string, unknown> }) { const usage = data.usage as Record<string, number> | undefined; const timings = data.timings as Record<string, number> | undefined; return <div className="generation-metrics"><span>{usage?.completion_tokens ?? 0} tokens</span>{timings?.predicted_per_second && <span>{timings.predicted_per_second.toFixed(1)} tok/s</span>}</div>; }
function RuntimeNotice({ title, detail }: { title: string; detail: string }) { return <div className="runtime-feed"><LoaderCircle className="spin"/><span><strong>{title}</strong>{detail}</span></div>; }
function Welcome({ models, context, freeMib }: { models: number; context: number; freeMib?: number }) { return <div className="control-welcome"><Sparkles/><h2>Your private, persistent workspace.</h2><p>Stream a conversation, review reasoning and metrics, or give the same local model a visible computer task. Nothing is sent away.</p><div><span><strong>{models}</strong> models</span><span><strong>{context.toLocaleString()}</strong> context</span><span><strong>{freeMib === undefined ? "—" : formatMib(freeMib)}</strong> VRAM free</span></div></div>; }
function Metric({ label, value }: { label: string; value: string }) { return <div className="control-metric"><span>{label}</span><strong>{value}</strong></div>; }
export function terminalTaskStatus(kind: string, fallback: string) { if (kind === "done") return "completed"; if (kind === "cancelled") return "cancelled"; if (kind === "error" || kind === "limit") return "failed"; return fallback === "starting" ? "running" : fallback; }
function relativeTime(value: string) { const delta = Date.now() - new Date(value).getTime(); if (delta < 60_000) return "now"; if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`; if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`; return new Date(value).toLocaleDateString(); }
function timeOnly(value: string) { return new Date(value).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" }); }
function fileName(path: string) { return path.split(/[\\/]/).pop() ?? path; }
function formatMib(value: number) { return value >= 1024 ? `${(value / 1024).toFixed(1)} GiB` : `${value.toLocaleString()} MiB`; }
