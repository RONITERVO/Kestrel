import { Activity, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { StrataDashboardSnapshot, StrataDashboardRequestDetail } from "../../contracts/index";
import { getStrataDashboard, getStrataRequestDetail } from "../../platform/api";
import { FlowPages } from "../../shared/book/FlowPages";
import { PagedList } from "../../shared/book/PagedList";
import "./strata-dashboard.css";

const number = (value: number | null | undefined, digits = 1) => value == null ? "—" : value.toLocaleString(undefined, { maximumFractionDigits: digits });

export function StrataDashboardButton() {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  return <>
    <button ref={button} type="button" className="quiet-button compact" onClick={() => setOpen(true)}><Activity size={15} /> Strata dashboard</button>
    {open && createPortal(<StrataDashboard onClose={() => { setOpen(false); button.current?.focus(); }} />, document.body)}
  </>;
}

export function StrataDashboard({ onClose }: { onClose: () => void }) {
  const [snapshot, setSnapshot] = useState<StrataDashboardSnapshot>();
  const [error, setError] = useState("");
  const [selected, setSelected] = useState<string>();
  const [detail, setDetail] = useState<StrataDashboardRequestDetail>();
  const [detailError, setDetailError] = useState("");
  const [tab, setTab] = useState<"output" | "reasoning" | "input">("output");
  const [paused, setPaused] = useState(false);
  const dialog = useRef<HTMLElement>(null);
  const close = useRef<HTMLButtonElement>(null);

  useEffect(() => { close.current?.focus(); }, []);
  useEffect(() => {
    if (paused) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      if (!document.hidden) {
        try {
          const next = await getStrataDashboard();
          if (!disposed) { setSnapshot(next); setError(""); if (!next.available) setSelected(undefined); }
        } catch (cause) { if (!disposed) setError(String(cause)); }
      }
      if (!disposed) timer = setTimeout(() => void refresh(), 2000);
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, [paused]);

  useEffect(() => { setDetail(undefined); setDetailError(""); }, [selected]);
  useEffect(() => {
    if (!selected || paused) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    async function refresh() {
      if (!document.hidden) {
        try {
          const next = await getStrataRequestDetail(selected!);
          if (!disposed) { setDetail(next); setDetailError(""); }
          if (!["queued", "loading", "reading", "generating", "running", "streaming"].includes(next.request.state)) return;
        } catch (cause) { if (!disposed) setDetailError(String(cause)); return; }
      }
      if (!disposed && !paused) timer = setTimeout(() => void refresh(), 2000);
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, [selected, paused]);

  const activity = snapshot?.activity;
  const hw = snapshot?.hardware;
  const selectedRequest = detail?.request ?? snapshot?.requests.find(r => r.id === selected);
  const text = detail?.[tab] ?? "";
  const truncated = detail && (tab === "input" ? detail.inputTruncated : tab === "reasoning" ? detail.reasoningTruncated : detail.outputTruncated);

  return <div className="strata-dashboard-overlay">
    <section ref={dialog} className="strata-dashboard" role="dialog" aria-modal="true" aria-labelledby="strata-dashboard-title" onKeyDown={event => {
      if (event.key === "Escape") { event.stopPropagation(); onClose(); }
      if (event.key === "Tab") {
        const nodes = [...(dialog.current?.querySelectorAll<HTMLElement>('button:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex="0"]') ?? [])];
        const first = nodes[0], last = nodes.at(-1);
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    }}>
      <header><div><span className="eyebrow">Live local runtime</span><h2 id="strata-dashboard-title">Strata dashboard</h2><p>{activity?.model ?? snapshot?.runtime.modelName ?? "Kestrel’s managed model"}</p></div><div><button type="button" className="quiet-button" aria-pressed={paused} onClick={() => setPaused(!paused)}>{paused ? "Resume updates" : "Pause updates"}</button><button ref={close} type="button" aria-label="Close Strata dashboard" onClick={onClose}><X /></button></div></header>
      <p className="strata-dashboard-notice" role="status">{error || snapshot?.detail || "Reading the managed runtime…"}</p>
      {snapshot?.available && <>
        <div className="strata-dashboard-cards">
          <Metric label="Activity" value={snapshot.runtime.inferenceBusy && activity?.phase === "idle" ? "Preparing request" : activity?.phase || "idle"} note={`${number(activity?.contextWindow, 0)} context · ${activity?.images ? "images on" : "images off"} · ${number(activity?.queued, 0)} queued`} />
          <Metric label="GPU" value={`${number(hw?.gpuPercent)}%`} note={`${hw?.gpuName || "GPU"} · ${number(hw?.gpuUsedMib, 0)} / ${number(hw?.gpuTotalMib, 0)} MiB`} series={hw?.gpuHistory} maximum={100} />
          <Metric label="System CPU" value={`${number(hw?.cpuPercent)}%`} note={`${number(hw?.gpuTemperature)} °C GPU · ${number(hw?.gpuWatts)} W`} series={hw?.cpuHistory} maximum={100} />
          <Metric label="System RAM" value={`${number(hw?.ramUsedGib)} / ${number(hw?.ramTotalGib)} GiB`} note="Recent history from Strata" series={hw?.ramPercentHistory} maximum={100} />
          <Metric label="Prompt processing" value={`${number(activity?.prefillTokensPerSecond)} tok/s`} note={`${number(activity?.promptRead, 0)} / ${number(activity?.promptTotal ?? activity?.promptTokens, 0)} prompt tokens`} />
          <Metric label="Output speed" value={`${number(activity?.tokensPerSecond)} tok/s`} note={`${number(activity?.generatedTokens, 0)} / ${number(activity?.maxOutputTokens, 0)} tokens · ${number(activity?.elapsedSeconds)} s`} series={hw?.outputSpeedHistory} />
        </div>
        <div className="strata-dashboard-workspace">
          <div className="strata-dashboard-requests"><h3>Recent requests</h3><PagedList label="Strata request pages" items={snapshot.requests} itemKey={item => item.id} empty={<p>{snapshot.requestCapture ? "No requests in this runtime yet." : "Request text is unavailable. Update Strata if needed, then restart the model."}</p>} renderItem={item => <button type="button" className={selected === item.id ? "strata-dashboard-request selected" : "strata-dashboard-request"} onClick={() => setSelected(item.id)}><strong>{item.path}</strong><span>{item.state} · {number(item.wallSeconds)} s</span><small>{item.id} · {number(item.outputTokens, 0)} output tokens</small></button>} /></div>
          <div className="strata-dashboard-detail"><h3>Request details</h3>{!selected ? <p>Choose a request to inspect its timing, input, output and reasoning.</p> : <>
            <div className="strata-dashboard-request-facts"><span>First token <b>{number(selectedRequest?.firstTokenSeconds)} s</b></span><span>Queue / load <b>{number(selectedRequest?.queueSeconds)} / {number(selectedRequest?.loadSeconds)} s</b></span><span>Prompt / reused <b>{number(selectedRequest?.promptTokens, 0)} / {number(selectedRequest?.reusedTokens, 0)}</b></span><span>Prefill / output <b>{number(selectedRequest?.prefillTokensPerSecond)} / {number(selectedRequest?.outputTokensPerSecond)} tok/s</b></span></div>
            <div className="strata-dashboard-tabs" role="tablist" aria-label="Request content">{(["output", "reasoning", "input"] as const).map(name => <button key={name} type="button" role="tab" aria-selected={tab === name} onClick={() => setTab(name)}>{name}</button>)}</div>
            {detailError && <p role="alert">{detailError}</p>}{detail?.error && <p role="alert">{detail.error}</p>}{truncated && <p>This captured text was truncated by the runtime or the dashboard’s size limit.</p>}
            <FlowPages label="Strata request content pages" resetKey={`${selected}:${tab}`} className="strata-dashboard-content"><pre>{detail ? text || "No text recorded for this view." : "Reading request…"}</pre></FlowPages>
          </>}</div>
        </div>
      </>}
      <footer>Monitoring uses the current Kestrel runtime. Updates stop when this view is closed. Runtime credentials stay in native code.</footer>
    </section>
  </div>;
}

function Metric({ label, value, note, series, maximum }: { label: string; value: string; note: string; series?: Array<number | null>; maximum?: number }) {
  const top = maximum ?? Math.max(1, ...(series ?? []).filter((n): n is number => n != null));
  const points = (series ?? []).map((n, i) => n == null ? null : `${i * 100 / Math.max(1, (series?.length ?? 1) - 1)},${28 - Math.min(1, n / top) * 26}`);
  const segments: string[][] = [];
  for (const point of points) { if (point == null) segments.push([]); else { if (!segments.length) segments.push([]); segments.at(-1)!.push(point); } }
  return <article><small>{label}</small><strong>{value}</strong><span>{note}</span>{series && <svg viewBox="0 0 100 30" preserveAspectRatio="none" role="img" aria-label={`${label} recent history`}>{segments.map((segment, i) => <polyline key={i} points={segment.join(" ")} />)}</svg>}</article>;
}
