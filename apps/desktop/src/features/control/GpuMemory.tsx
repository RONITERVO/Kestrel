import {
  Check,
  ChevronDown,
  CircleStop,
  Copy,
  LoaderCircle,
  MemoryStick,
  ShieldCheck,
  Sparkles,
  TriangleAlert,
  Wrench,
  X,
  Zap,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { AppSnapshot, GpuMemoryProcess, VramCleanupPreview, VramCleanupResult } from "../../contracts/index";
import { PagedList } from "../../shared/book/PagedList";
import { formatMib } from "../../shared/format";
import { useInferenceTelemetry } from "./InferenceTelemetry";
import "./gpu-memory.css";

export function GpuMemoryActions({
  gpu,
  onClean,
  onRelease,
  onError,
}: {
  gpu: AppSnapshot["control"]["gpu"];
  onClean: () => Promise<string>;
  onRelease: () => Promise<string>;
  onError: (message: string) => void;
}) {
  const [busy, setBusy] = useState<"clean" | "release" | null>(null);
  const [feedback, setFeedback] = useState("");
  const menuRef = useRef<HTMLDetailsElement>(null);

  useEffect(() => {
    if (!feedback) return;
    const timer = window.setTimeout(() => setFeedback(""), 5_000);
    return () => window.clearTimeout(timer);
  }, [feedback]);

  const run = async (kind: "clean" | "release") => {
    menuRef.current?.removeAttribute("open");
    setBusy(kind);
    try {
      const message = await (kind === "clean" ? onClean() : onRelease());
      if (message) setFeedback(message);
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };

  return <div className={`header-memory-actions ${feedback ? "has-feedback" : ""}`}>
    <button
      type="button"
      className="header-memory-clean"
      disabled={busy !== null}
      title="Preview and close competing GPU applications"
      onClick={() => void run("clean")}
    >
      {busy === "clean" ? <LoaderCircle className="spin" /> : <MemoryStick />}
      <span>{feedback || "Clean VRAM"}</span>
    </button>
    <details className="header-memory-menu" ref={menuRef}>
      <summary
        aria-label="GPU memory options"
        aria-disabled={busy !== null}
        title="GPU memory options"
        onClick={(event) => { if (busy) event.preventDefault(); }}
      ><ChevronDown /></summary>
      <div className="header-memory-popover" role="menu" aria-label="GPU memory actions">
        <div className="header-memory-summary">
          <MemoryStick />
          <span><strong>GPU memory</strong><small>{gpu ? `${formatMib(gpu.freeMib)} free of ${formatMib(gpu.totalMib)}` : "NVIDIA telemetry unavailable"}</small></span>
        </div>
        <p>Clean other GPU apps before loading a model so Kestrel can keep the full workload in VRAM.</p>
        <button type="button" role="menuitem" disabled={busy !== null} onClick={() => void run("clean")}>
          <Sparkles /><span><strong>Clean competing apps</strong><small>Preview exactly what will close</small></span>
        </button>
        <button type="button" role="menuitem" disabled={busy !== null} onClick={() => void run("release")}>
          <CircleStop /><span><strong>Release Kestrel AI memory</strong><small>Stop Kestrel model and media runtimes</small></span>
        </button>
      </div>
    </details>
  </div>;
}

export function VramCleanupDialog({
  preview,
  onClose,
  onClean,
  onForce,
}: {
  preview: VramCleanupPreview;
  onClose: () => void;
  onClean: (approvedPids: number[]) => Promise<VramCleanupResult>;
  onForce: (expectedProcesses: GpuMemoryProcess[]) => Promise<VramCleanupResult>;
}) {
  const [selectedPids, setSelectedPids] = useState<Set<number>>(
    () => new Set(preview.candidates.map((process) => process.pid)),
  );
  const [advanced, setAdvanced] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [result, setResult] = useState<VramCleanupResult | null>(null);
  const [copiedPid, setCopiedPid] = useState<number | null>(null);
  const overridable = preview.exclusions.filter((item) => item.canInclude).length;
  const critical = preview.exclusions.length - overridable;
  const forceableFailures = result?.failed.filter((failure) => failure.canForceClose) ?? [];

  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [busy, onClose]);

  const setSelected = (pid: number, selected: boolean) => {
    setSelectedPids((current) => {
      const next = new Set(current);
      if (selected) next.add(pid);
      else next.delete(pid);
      return next;
    });
  };

  const submit = async () => {
    if (!selectedPids.size) return;
    setBusy(true);
    setError("");
    try {
      setResult(await onClean([...selectedPids]));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  const forceClose = async () => {
    if (!forceableFailures.length) return;
    const names = forceableFailures.slice(0, 3).map((failure) => failure.process.name).join(", ");
    if (!window.confirm(`Force close ${forceableFailures.length} process${forceableFailures.length === 1 ? "" : "es"} (${names})? This uses GpuClean's force method. Unsaved work will be lost.`)) return;
    setBusy(true);
    setError("");
    try {
      setResult(await onForce(forceableFailures.map((failure) => failure.process)));
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };

  const copyPowerShell = async (pid: number, command: string) => {
    try {
      await navigator.clipboard.writeText(command);
      setCopiedPid(pid);
    } catch (cause) {
      setError(`Could not copy the PowerShell command: ${String(cause)}`);
    }
  };

  return <div className="vram-cleanup-overlay">
    <section className="vram-cleanup-dialog" role="dialog" aria-modal="true" aria-labelledby="vram-cleanup-title">
      <header>
        <div className="vram-cleanup-symbol"><MemoryStick /></div>
        <div><span className="eyebrow">Producer GPU maintenance</span><h2 id="vram-cleanup-title">Choose what Clean VRAM closes</h2><p>Uncheck anything you want to keep open. Unsaved work in selected programs may be lost.</p></div>
        <button type="button" aria-label="Close VRAM cleanup" disabled={busy} onClick={onClose}><X /></button>
      </header>

      {result ? <div className="vram-cleanup-result">
        <div className={result.failed.length ? "partial" : "complete"}>{result.failed.length ? <TriangleAlert /> : <Check />}</div>
        <h3 role="status">{result.message}</h3>
        {result.afterGpu && <p>{formatMib(result.afterGpu.freeMib)} of {formatMib(result.afterGpu.totalMib)} is now free.</p>}
        {!!result.freedMib && <small>Observed VRAM released: {formatMib(result.freedMib)}</small>}
        {!!result.failed.length && <ul>{result.failed.map((failure) => <li key={failure.process.pid}>
          <strong>{failure.process.name} <small>PID {failure.process.pid}</small></strong>
          <span>{failure.detail}</span>
          {failure.canForceClose && <em>Force close is an explicit second step and uses the same <code>taskkill /F</code> operation as GpuClean.</em>}
          {failure.powershellCommand && <div className="vram-manual-command">
            <span>Run in PowerShell as administrator</span>
            <code>{failure.powershellCommand}</code>
            <button type="button" className="quiet-button" onClick={() => void copyPowerShell(failure.process.pid, failure.powershellCommand!)}><Copy /> {copiedPid === failure.process.pid ? "Copied" : "Copy admin command"}</button>
          </div>}
        </li>)}</ul>}
        {error && <div className="vram-cleanup-error" role="alert"><TriangleAlert />{error}</div>}
      </div> : <div className="vram-cleanup-body">
        <div className="vram-cleanup-meter">
          <MemoryStick /><span><strong>{preview.gpu ? `${formatMib(preview.gpu.freeMib)} free` : "GPU memory detected"}</strong><small>{preview.gpu ? `${formatMib(preview.gpu.usedMib)} used on ${preview.gpu.name}` : "Per-process memory is unavailable on this driver"}</small></span>
          <b>{selectedPids.size} selected</b>
        </div>

        <section className="vram-process-section" aria-labelledby="vram-default-list">
          <div className="vram-process-heading"><span><strong id="vram-default-list">Ready to clean</strong><small>Selected automatically; uncheck to exclude</small></span><b>{preview.candidates.length}</b></div>
          <PagedList
            className="vram-process-list"
            label="GPU processes ready to clean"
            items={preview.candidates}
            itemKey={(process) => String(process.pid)}
            empty={<p className="vram-empty-list">Nothing is selected automatically. Open Advanced to inspect protected GPU processes.</p>}
            renderItem={(process) => <VramProcessChoice
              process={process}
              checked={selectedPids.has(process.pid)}
              label={`Clean ${process.name} PID ${process.pid}`}
              selectedLabel="Will close"
              idleLabel="Keep open"
              onChange={(selected) => setSelected(process.pid, selected)}
            />}
          />
        </section>

        <section className="vram-advanced-section">
          <button type="button" className="vram-advanced-toggle" aria-expanded={advanced} onClick={() => setAdvanced((value) => !value)}>
            <Wrench /><span><strong>Advanced exclusions</strong><small>{overridable} can be included · {critical} always protected</small></span><ChevronDown />
          </button>
          {advanced && <PagedList
            className="vram-process-list advanced"
            label="Automatically excluded GPU processes"
            items={preview.exclusions}
            itemKey={(item) => String(item.process.pid)}
            empty={<p className="vram-empty-list">No processes were automatically excluded.</p>}
            renderItem={(item) => <VramProcessChoice
              process={item.process}
              checked={item.canInclude && selectedPids.has(item.process.pid)}
              disabled={!item.canInclude}
              label={item.canInclude ? `Include ${item.process.name} PID ${item.process.pid}` : `Always protect ${item.process.name} PID ${item.process.pid}`}
              selectedLabel="Included"
              idleLabel={item.canInclude ? "Excluded" : "Always protected"}
              reason={item.reason}
              onChange={item.canInclude ? (selected) => setSelected(item.process.pid, selected) : undefined}
            />}
          />}
        </section>
        {error && <div className="vram-cleanup-error" role="alert"><TriangleAlert />{error}</div>}
      </div>}

      <footer>
        {result ? <><span>{result.terminated.length} closed · {result.failed.length} need attention</span>{!!forceableFailures.length && <button type="button" className="vram-force-button" disabled={busy} onClick={() => void forceClose()}>{busy ? <LoaderCircle className="spin" /> : <TriangleAlert />} Force close {forceableFailures.length}</button>}<button type="button" className="primary-button" disabled={busy} onClick={onClose}>Done</button></> : <>
          <span>{selectedPids.size ? `${selectedPids.size} process${selectedPids.size === 1 ? "" : "es"} will close` : "Nothing selected"}</span>
          <button type="button" className="quiet-button" disabled={busy} onClick={onClose}>Cancel</button>
          <button type="button" className="primary-button" disabled={busy || !selectedPids.size} onClick={() => void submit()}>{busy ? <LoaderCircle className="spin" /> : <Sparkles />} Clean {selectedPids.size || "VRAM"}</button>
        </>}
      </footer>
    </section>
  </div>;
}

function VramProcessChoice({
  process,
  checked,
  disabled = false,
  label,
  selectedLabel,
  idleLabel,
  reason,
  onChange,
}: {
  process: GpuMemoryProcess;
  checked: boolean;
  disabled?: boolean;
  label: string;
  selectedLabel: string;
  idleLabel: string;
  reason?: string;
  onChange?: (selected: boolean) => void;
}) {
  return <label className={`vram-process-choice ${disabled ? "critical" : ""} ${checked ? "selected" : ""}`}>
    <input type="checkbox" aria-label={label} checked={checked} disabled={disabled} onChange={(event) => onChange?.(event.currentTarget.checked)} />
    <span className="vram-process-check">{disabled ? <ShieldCheck /> : checked ? <Check /> : null}</span>
    <span className="vram-process-copy"><strong>{process.name}</strong><small>{process.kind} · PID {process.pid}{process.memoryMib ? ` · ${formatMib(process.memoryMib)}` : " · VRAM amount unavailable"}</small>{reason && <em>{reason}</em>}<code title={process.executablePath}>{process.executablePath}</code></span>
    <b>{checked ? selectedLabel : idleLabel}</b>
  </label>;
}

export function InferenceSpeedIndicator() {
  const telemetry = useInferenceTelemetry();
  const speed = telemetry.tokensPerSecond === undefined ? "—" : telemetry.tokensPerSecond.toFixed(1);
  const state = telemetry.active ? "Live" : telemetry.observedAt ? "Last" : "Idle";
  const model = telemetry.modelName ? ` · ${telemetry.modelName}` : "";
  const accuracy = telemetry.tokensPerSecond === undefined ? "" : telemetry.exact ? " · measured" : " · estimated";
  const title = telemetry.active
    ? `Live local-model generation speed${model}${accuracy}`
    : telemetry.observedAt
      ? `Last local-model generation speed${model}${accuracy}`
      : "Local-model generation speed will appear here";
  return <div className={`header-inference-speed ${telemetry.active ? "live" : "idle"}`} aria-label={`${state} inference speed: ${speed} tokens per second`} title={title}>
    <Zap size={13} />
    <strong>{speed}</strong>
    <small>tok/s</small>
    <span>{state}</span>
  </div>;
}
