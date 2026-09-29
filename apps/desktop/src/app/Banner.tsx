import { Bird, BookOpen, Rotate3d, ShieldCheck } from "lucide-react";
import type { AppSnapshot, ServiceStatus } from "../contracts/index";
import { GpuMemoryActions, InferenceSpeedIndicator } from "../features/control/GpuMemory";
import type { BookControls } from "./book/BookShell";
import { chapterOf, type AppView } from "./book/chapters";

type ServiceState = ServiceStatus["modelRuntime"];

const stateWords: Record<ServiceState, string> = {
  ready: "ready",
  starting: "starting",
  stopped: "stopped",
  unavailable: "unavailable",
};

/**
 * The paper strip taped across the top of the desk: who you are working with and what is
 * running. Status is always written out in words, so a screenshot alone explains the state.
 */
export function BookBanner({
  view,
  status,
  gpu,
  controls,
  onPrepare,
  onCleanVram,
  onReleaseAiMemory,
  onError,
}: {
  view: AppView;
  status: AppSnapshot["status"];
  gpu: AppSnapshot["control"]["gpu"];
  controls: BookControls;
  onPrepare: () => void;
  onCleanVram: () => Promise<string>;
  onReleaseAiMemory: () => Promise<string>;
  onError: (message: string) => void;
}) {
  const allReady = status.modelRuntime === "ready" && status.wikipedia === "ready";
  const chapter = chapterOf(view);
  return (
    <header className={`app-header book-banner app-header-${view}`}>
      <div className="banner-brand">
        <span className="banner-mark" aria-hidden="true"><Bird /></span>
        <strong>Kestrel</strong>
        <span>{chapter.numeral} · {chapter.label}</span>
      </div>
      <div className="banner-status header-status" role="status">
        <StatusPill state={status.wikipedia} label={status.archive} kind="Archive" />
        <StatusPill state={status.modelRuntime} label={status.model} kind="Model" />
        <span className="chip ok privacy-pill" aria-label="Offline only" title="Offline only"><ShieldCheck size={14} /> Offline only</span>
      </div>
      <div className="banner-actions header-actions">
        <InferenceSpeedIndicator />
        <GpuMemoryActions gpu={gpu} onClean={onCleanVram} onRelease={onReleaseAiMemory} onError={onError} />
        {!allReady && <button className="quiet-button compact" onClick={onPrepare}>Prepare services</button>}
        {controls.webgl && (
          <button
            type="button"
            className="quiet-button compact book-pose-toggle"
            aria-pressed={controls.lifted}
            title={controls.lifted ? "Return to the flat reading view (Esc)" : "Lay the book on the desk to turn it around"}
            onClick={controls.toggleLifted}
          >
            {controls.lifted ? <BookOpen size={15} /> : <Rotate3d size={15} />}
            {controls.lifted ? "Read flat" : "Lay on desk"}
          </button>
        )}
      </div>
    </header>
  );
}

function StatusPill({ state, label, kind }: { state: ServiceState; label: string; kind: string }) {
  const tone = state === "ready" ? "ok" : state === "starting" ? "wait" : state === "unavailable" ? "stop" : "";
  return (
    <span className={`chip status-pill status-${state} ${tone}`} aria-label={`${label}: ${state}`} title={`${kind}: ${label} (${state})`}>
      <span className="status-dot" aria-hidden="true" />
      {label}
      <small>{stateWords[state]}</small>
    </span>
  );
}
