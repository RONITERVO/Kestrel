import { Check, LoaderCircle, MonitorCog, RefreshCw, ShieldCheck, Wrench } from "lucide-react";
import { useEffect, useState } from "react";
import { getControlSnapshot, onDeveloperProgress, runCodexRepair, runNativeDiagnostics, saveControlSettings } from "./api";
import { OfflineWorkspace } from "./OfflineWorkspace";
import type { ControlSnapshot, DeveloperRepairReport } from "./types";

type ControlProps = {
  control: ControlSnapshot;
  onChanged: (control: ControlSnapshot) => void;
  onError: (message: string) => void;
};

export function ControlPlane(props: ControlProps) {
  return <OfflineWorkspace {...props}/>;
}

export function DeveloperConsole({ control, onChanged, onError }: ControlProps) {
  const [issue, setIssue] = useState("");
  const [output, setOutput] = useState("");
  const [report, setReport] = useState<DeveloperRepairReport | null>(null);
  const [busy, setBusy] = useState<"diagnose" | "repair" | "save" | null>(null);
  const [projectRoot, setProjectRoot] = useState(control.settings.projectRoot);
  const [progressDetail, setProgressDetail] = useState<string | null>(null);

  useEffect(() => {
    void getControlSnapshot().then(onChanged).catch((cause) => onError(String(cause)));
    // Parent callbacks only write application state; probing once keeps offline startup free of Codex processes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    let dispose: (() => void) | undefined;
    void onDeveloperProgress((event) => setProgressDetail(event.detail)).then((unlisten) => { dispose = unlisten; });
    return () => dispose?.();
  }, []);

  const refresh = async () => onChanged(await getControlSnapshot());
  const diagnose = async () => {
    setBusy("diagnose");
    setOutput("");
    try { setOutput(await runNativeDiagnostics()); await refresh(); } catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const repair = async () => {
    if (!window.confirm("Allow Codex to edit only this Git workspace, run the fixed checks, and leave the diff uncommitted for review?")) return;
    setBusy("repair");
    setReport(null);
    try { const next = await runCodexRepair(issue); setReport(next); setOutput(next.diagnosticsAfter); await refresh(); } catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const saveRoot = async () => {
    setBusy("save");
    try { onChanged(await saveControlSettings({...control.settings, projectRoot})); } catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const dev = control.developer;

  return <div className="developer-console">
    <header className="system-hero"><div><span className="eyebrow">OPTIONAL MAINTAINER ASSISTANT</span><h1>Developer</h1><p>Native checks diagnose Kestrel offline. When Codex is available, one scoped repair run can maintain the Rust backend without becoming part of the research runtime.</p></div><button className="quiet-button" onClick={() => void refresh()}><RefreshCw/> Refresh</button></header>
    <section className="dev-status-grid"><article><span>Codex CLI</span><strong>{dev.codexAvailable ? dev.codexVersion : "Not installed"}</strong><small>{dev.codexAuthenticated ? "Signed in" : "Offline or signed out"}</small></article><article><span>Git safety boundary</span><strong>{dev.gitRepository ? "Repository verified" : "Invalid project root"}</strong><small>{dev.worktreeClean ? "Clean worktree" : "Existing changes will be preserved"}</small></article><article><span>Offline independence</span><strong>Always local-capable</strong><small>Chat, research, and Computer Tasks never require Codex</small></article></section>
    <section className="dev-workspace"><div className="dev-root"><label htmlFor="kestrel-project-root">Kestrel project root</label><div><input id="kestrel-project-root" value={projectRoot} onChange={(event) => setProjectRoot(event.target.value)}/><button className="quiet-button" onClick={() => void saveRoot()} disabled={!!busy}>{busy === "save" ? <LoaderCircle className="spin"/> : <Check/>} Save</button></div></div><label><span>Observed issue or desired backend repair</span><textarea value={issue} onChange={(event) => setIssue(event.target.value)} placeholder="Example: A cancelled Computer Task leaves its run marked active after restart."/></label><div className="dev-actions"><button className="quiet-button" onClick={() => void diagnose()} disabled={!!busy}>{busy === "diagnose" ? <LoaderCircle className="spin"/> : <MonitorCog/>} Run offline diagnostics</button><button className="primary-button" onClick={() => void repair()} disabled={!!busy || !dev.codexAvailable || !dev.codexAuthenticated}>{busy === "repair" ? <LoaderCircle className="spin"/> : <Wrench/>} Diagnose & repair with Codex</button></div><div className="advanced-warning"><ShieldCheck/><div><strong>Codex is scoped, ephemeral, and reviewable.</strong><span>Workspace-write only. No automatic commit. Fixed verification after edits. Offline features never call this path.</span></div></div></section>
    {busy && progressDetail && <div className="repair-result"><strong>{busy === "repair" ? "Codex maintenance in progress" : "Native diagnostics in progress"}</strong><span>{progressDetail}</span></div>}
    {report && <div className={`repair-result ${report.success ? "success" : "failed"}`}><strong>{report.summary}</strong><span>{report.reportPath}</span></div>}
    {output && <section className="diagnostic-output"><div><span>DIAGNOSTIC TRANSCRIPT</span><button onClick={() => void navigator.clipboard.writeText(output)}>Copy</button></div><pre>{output}</pre></section>}
  </div>;
}
