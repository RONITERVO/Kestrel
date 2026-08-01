import { Bot, Check, CircleStop, LoaderCircle, MonitorCog, Play, RefreshCw, Search, Send, ShieldCheck, Wrench, Zap } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { getControlSnapshot, onDeveloperProgress, onRuntimeProgress, runCodexRepair, runNativeDiagnostics, saveControlSettings, scanLocalModels, sendLocalChat, startLocalModel, stopLocalModel } from "./api";
import type { ControlSnapshot, DeveloperRepairReport } from "./types";

type ControlProps = {
  control: ControlSnapshot;
  onChanged: (control: ControlSnapshot) => void;
  onError: (message: string) => void;
};

export function ControlPlane({ control, onChanged, onError }: ControlProps) {
  const [selectedId, setSelectedId] = useState(control.settings.selectedModelId ?? control.models[0]?.id ?? "");
  const [filter, setFilter] = useState("");
  const [draft, setDraft] = useState("");
  const [messages, setMessages] = useState<Array<{ role: "user" | "assistant"; content: string; reasoning?: string }>>([]);
  const [working, setWorking] = useState<"scan" | "start" | "stop" | "chat" | "save" | null>(null);
  const [settings, setSettings] = useState(control.settings);
  const [runtimeProgress, setRuntimeProgress] = useState<string | null>(null);
  const changedRef = useRef(onChanged);
  changedRef.current = onChanged;
  const selected = control.models.find((model) => model.id === selectedId);
  const visible = control.models.filter((model) => `${model.name} ${model.source} ${model.quantization ?? ""}`.toLowerCase().includes(filter.toLowerCase()));
  const gpu = control.gpu;

  useEffect(() => {
    void getControlSnapshot().then((next) => changedRef.current(next)).catch(() => undefined);
    const timer = window.setInterval(() => void getControlSnapshot().then((next) => changedRef.current(next)).catch(() => undefined), 2_500);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    let dispose: (() => void) | undefined;
    void onRuntimeProgress((event) => setRuntimeProgress(event.detail)).then((unlisten) => { dispose = unlisten; });
    return () => dispose?.();
  }, []);

  const act = async (kind: typeof working, action: () => Promise<ControlSnapshot>) => {
    setWorking(kind);
    try { onChanged(await action()); } catch (cause) { onError(String(cause)); } finally { setWorking(null); }
  };
  const send = async () => {
    if (!selected || !draft.trim()) return;
    const message = draft.trim();
    setDraft("");
    setMessages((items) => [...items, { role: "user", content: message }]);
    setWorking("chat");
    try {
      const response = await sendLocalChat({ modelId: selected.id, message, temperature: 0.2, topP: 0.9, topK: 40, maxOutputTokens: settings.maxOutputTokens });
      setMessages((items) => [...items, { role: "assistant", content: response.content, reasoning: response.reasoning }]);
      onChanged(await getControlSnapshot());
    } catch (cause) { onError(String(cause)); } finally { setWorking(null); }
  };
  const save = async () => {
    setWorking("save");
    try { onChanged(await saveControlSettings({ ...settings, selectedModelId: selectedId || undefined })); } catch (cause) { onError(String(cause)); } finally { setWorking(null); }
  };

  return <div className="control-plane">
    <aside className="model-drawer">
      <div className="control-product"><strong>KESTREL</strong><span>LOCAL CONTROL PLANE</span></div>
      <div className="model-search"><Search size={14}/><input value={filter} onChange={(event) => setFilter(event.target.value)} placeholder="Find a local model"/></div>
      <div className="drawer-title"><span>MODEL LIBRARY</span><button title="Read-only rescan" onClick={() => void act("scan", scanLocalModels)}>{working === "scan" ? <LoaderCircle className="spin"/> : <RefreshCw/>}</button></div>
      <div className="control-models">{visible.map((model) => <button key={model.id} className={selectedId === model.id ? "selected" : ""} onClick={() => setSelectedId(model.id)}><Bot/><span><strong>{model.name}</strong><small>{model.source} · {model.quantization ?? "GGUF"} · {formatBytes(model.bytes)}</small></span>{control.runtime.modelId === model.id && <i>{control.runtime.phase}</i>}</button>)}{!visible.length && <p>No GGUF models indexed. Add a root in System or rescan known local folders.</p>}</div>
      <div className="local-lock"><ShieldCheck/><span><strong>Local-only runtime</strong><small>Loopback · one inference lease · no silent offload</small></span></div>
    </aside>
    <section className="control-center">
      <header className="control-top"><div><span className="eyebrow">{selected?.architecture ?? "LOCAL GGUF"}</span><h1>{selected?.name ?? "Choose a model"}</h1></div><div className="control-actions">{control.runtime.phase === "ready" ? <button className="quiet-button" onClick={() => void act("stop", stopLocalModel)}>{working === "stop" ? <LoaderCircle className="spin"/> : <CircleStop/>}{control.runtime.mode === "attached" ? "Detach" : "Stop"}</button> : <button className="primary-button" disabled={!selected || !!working} onClick={() => selected && void act("start", () => startLocalModel(selected.id))}>{working === "start" ? <LoaderCircle className="spin"/> : <Play/>} Load model</button>}</div></header>
      <div className="control-chat">{working === "start" && runtimeProgress && <div className="runtime-feed"><LoaderCircle className="spin"/><span><strong>MODEL STARTUP</strong>{runtimeProgress}</span></div>}{messages.length ? messages.map((message, index) => <article key={index} className={message.role}><span>{message.role === "user" ? "YOU" : selected?.name ?? "MODEL"}</span>{message.reasoning && <details><summary>Reasoning</summary><pre>{message.reasoning}</pre></details>}<p>{message.content}</p></article>) : <div className="control-welcome"><Zap/><h2>Your models. Nothing hidden.</h2><p>Load a local model, inspect exactly how it runs, or move directly into source-grounded offline research.</p><div><span><strong>{control.models.length}</strong> models</span><span><strong>{settings.contextWindow.toLocaleString()}</strong> context</span><span><strong>{gpu ? formatMib(gpu.freeMib) : "—"}</strong> VRAM free</span></div></div>}</div>
      <div className="control-composer"><textarea value={draft} onChange={(event) => setDraft(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.shiftKey) { event.preventDefault(); void send(); } }} placeholder={control.runtime.phase === "ready" ? "Message the active local model…" : "Load a model to begin"} disabled={control.runtime.phase !== "ready" || working === "chat"}/><button onClick={() => void send()} disabled={!draft.trim() || control.runtime.phase !== "ready" || working === "chat"}>{working === "chat" ? <LoaderCircle className="spin"/> : <Send/>}</button></div>
    </section>
    <aside className="control-inspector">
      <span className="eyebrow">SESSION INSPECTOR</span>
      <ControlMetric label="Runtime" value={control.runtime.phase}/><ControlMetric label="Ownership" value={control.runtime.mode}/><ControlMetric label="Context" value={control.runtime.contextWindow ? control.runtime.contextWindow.toLocaleString() : "—"}/><ControlMetric label="Inference" value={control.runtime.inferenceBusy ? "Busy · queued safely" : "Available"}/>
      {gpu && <div className="control-memory"><strong>{gpu.name}</strong><div><span style={{width:`${Math.min(100, gpu.usedMib / gpu.totalMib * 100)}%`}}/></div><small>{formatMib(gpu.usedMib)} used · {formatMib(gpu.freeMib)} free</small></div>}
      <p className="runtime-detail">{control.runtime.detail}</p>
      {control.runtime.launchArgs.length > 0 && <details className="launch-proof"><summary>Exact engine launch</summary><pre>{control.runtime.launchArgs.join(" ")}</pre></details>}
      <div className="inline-runtime-settings"><label>Context<input type="number" disabled={!settings.advancedMode} value={settings.contextWindow} onChange={(event) => setSettings({...settings, contextWindow:Number(event.target.value)})}/></label><label>Max output<input type="number" disabled={!settings.advancedMode} value={settings.maxOutputTokens} onChange={(event) => setSettings({...settings, maxOutputTokens:Number(event.target.value)})}/></label><label className="check-line"><input type="checkbox" checked={settings.advancedMode} onChange={(event) => setSettings({...settings, advancedMode:event.target.checked})}/> Advanced, uncapped</label><button className="quiet-button" disabled={!!working} onClick={() => void save()}>{working === "save" ? <LoaderCircle className="spin"/> : <Check/>} Save runtime profile</button></div>
    </aside>
  </div>;
}

function ControlMetric({ label, value }: { label: string; value: string }) { return <div className="control-metric"><span>{label}</span><strong>{value}</strong></div>; }

export function DeveloperConsole({ control, onChanged, onError }: ControlProps) {
  const [issue, setIssue] = useState("");
  const [output, setOutput] = useState("");
  const [report, setReport] = useState<DeveloperRepairReport | null>(null);
  const [busy, setBusy] = useState<"diagnose" | "repair" | "save" | null>(null);
  const [projectRoot, setProjectRoot] = useState(control.settings.projectRoot);
  const [progressDetail, setProgressDetail] = useState<string | null>(null);
  useEffect(() => {
    void getControlSnapshot().then(onChanged).catch((cause) => onError(String(cause)));
    // Parent callbacks only write application state; probing once on mount keeps offline startup free of Codex processes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    let dispose: (() => void) | undefined;
    void onDeveloperProgress((event) => setProgressDetail(event.detail)).then((unlisten) => { dispose = unlisten; });
    return () => dispose?.();
  }, []);
  const refresh = async () => onChanged(await getControlSnapshot());
  const diagnose = async () => { setBusy("diagnose"); setOutput(""); try { setOutput(await runNativeDiagnostics()); await refresh(); } catch (cause) { onError(String(cause)); } finally { setBusy(null); } };
  const repair = async () => {
    if (!window.confirm("Allow Codex to edit only this Git workspace, run the fixed checks, and leave the diff uncommitted for review?")) return;
    setBusy("repair"); setReport(null);
    try { const next = await runCodexRepair(issue); setReport(next); setOutput(next.diagnosticsAfter); await refresh(); } catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const saveRoot = async () => { setBusy("save"); try { onChanged(await saveControlSettings({...control.settings, projectRoot})); } catch (cause) { onError(String(cause)); } finally { setBusy(null); } };
  const dev = control.developer;
  return <div className="developer-console">
    <header className="system-hero"><div><span className="eyebrow">OPTIONAL MAINTAINER ASSISTANT</span><h1>Developer</h1><p>Native checks diagnose Kestrel offline. When Codex is available, one scoped repair run can maintain the Rust backend without becoming part of the research runtime.</p></div><button className="quiet-button" onClick={() => void refresh()}><RefreshCw/> Refresh</button></header>
    <section className="dev-status-grid"><article><span>Codex CLI</span><strong>{dev.codexAvailable ? dev.codexVersion : "Not installed"}</strong><small>{dev.codexAuthenticated ? "Signed in" : "Offline or signed out"}</small></article><article><span>Git safety boundary</span><strong>{dev.gitRepository ? "Repository verified" : "Invalid project root"}</strong><small>{dev.worktreeClean ? "Clean worktree" : "Existing changes will be preserved"}</small></article><article><span>Research independence</span><strong>Always offline-capable</strong><small>Codex cannot run while strict research is active</small></article></section>
    <section className="dev-workspace"><label><span>Kestrel project root</span><div><input value={projectRoot} onChange={(event) => setProjectRoot(event.target.value)}/><button className="quiet-button" onClick={() => void saveRoot()} disabled={!!busy}>{busy === "save" ? <LoaderCircle className="spin"/> : <Check/>} Save</button></div></label><label><span>Observed issue or desired backend repair</span><textarea value={issue} onChange={(event) => setIssue(event.target.value)} placeholder="Example: Research cancellation leaves the runtime marked busy after an unreadable Kiwix result."/></label><div className="dev-actions"><button className="quiet-button" onClick={() => void diagnose()} disabled={!!busy}>{busy === "diagnose" ? <LoaderCircle className="spin"/> : <MonitorCog/>} Run offline diagnostics</button><button className="primary-button" onClick={() => void repair()} disabled={!!busy || !dev.codexAvailable || !dev.codexAuthenticated}>{busy === "repair" ? <LoaderCircle className="spin"/> : <Wrench/>} Diagnose & repair with Codex</button></div><div className="advanced-warning"><ShieldCheck/><div><strong>Codex is scoped, ephemeral, and reviewable.</strong><span>Workspace-write only. No shell network access, no automatic commit, fixed verification after edits. Research never calls this path.</span></div></div></section>
    {busy && progressDetail && <div className="repair-result"><strong>{busy === "repair" ? "Codex maintenance in progress" : "Native diagnostics in progress"}</strong><span>{progressDetail}</span></div>}
    {report && <div className={`repair-result ${report.success ? "success" : "failed"}`}><strong>{report.summary}</strong><span>{report.reportPath}</span></div>}
    {output && <section className="diagnostic-output"><div><span>DIAGNOSTIC TRANSCRIPT</span><button onClick={() => void navigator.clipboard.writeText(output)}>Copy</button></div><pre>{output}</pre></section>}
  </div>;
}

function formatBytes(bytes: number) { const units = ["B", "KiB", "MiB", "GiB", "TiB"]; let value = bytes; let unit = 0; while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; } return `${value.toFixed(unit > 2 ? 2 : 1)} ${units[unit]}`; }
function formatMib(value: number) { return value >= 1024 ? `${(value / 1024).toFixed(1)} GiB` : `${value.toLocaleString()} MiB`; }
