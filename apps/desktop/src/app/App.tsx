import { Bird, LoaderCircle, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  bootstrap,
  cancelResearch,
  cleanVram,
  forceCleanVram,
  getReport,
  onProgress,
  openStandalone,
  prepareServices,
  previewVramCleanup,
  releaseAiMemory,
  revealLibrary,
  runResearch,
} from "../platform/api";
import { ControlPlane, DeveloperConsole } from "../features/control/ControlPlane";
import { VramCleanupDialog } from "../features/control/GpuMemory";
import { SystemConsole } from "../features/control/SystemConsole";
import { NewResearchDialog, ResearchChapter, ResearchProgressPanel } from "../features/research/ResearchChapter";
import { SetupConsole } from "../features/setup/Setup";
import { ImageStudio } from "../features/studio/image/ImageStudio";
import { MovieStudio } from "../features/studio/movie/MovieStudio";
import { MusicStudio } from "../features/studio/music/MusicStudio";
import type {
  AppSnapshot,
  GpuMemoryProcess,
  ResearchProgress,
  ResearchReport,
  VramCleanupPreview,
  VramCleanupResult,
} from "../contracts/index";
import { BookBanner } from "./Banner";
import { BookShell } from "./book/BookShell";
import type { AppView } from "./book/chapters";
import "./app.css";

export type { AppView } from "./book/chapters";

const emptyProgress: ResearchProgress = {
  jobId: "",
  stage: "preparing",
  title: "Preparing research",
  detail: "Checking the private library and local services…",
  current: 0,
  total: 6,
  elapsedSeconds: 0,
};

export function retainAppView(views: ReadonlySet<AppView>, view: AppView): Set<AppView> {
  if (views.has(view)) return new Set(views);
  return new Set([...views, view]);
}

/** Window composition: one book, one retained chapter per workspace, and the shared dialogs. */
function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [report, setReport] = useState<ResearchReport | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [newResearchOpen, setNewResearchOpen] = useState(false);
  const [progress, setProgress] = useState<ResearchProgress | null>(null);
  const [activity, setActivity] = useState<ResearchProgress[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [vramCleanup, setVramCleanup] = useState<VramCleanupPreview | null>(null);
  const [view, setView] = useState<AppView>("research");
  const [mountedViews, setMountedViews] = useState<Set<AppView>>(() => new Set(["research"]));
  const handleError = useCallback((message: string) => setError(message), []);

  const showView = useCallback((next: AppView) => {
    setMountedViews((current) => retainAppView(current, next));
    setView(next);
  }, []);

  const refresh = useCallback(async () => {
    try {
      const next = await bootstrap();
      setSnapshot(next);
      if (!next.setup.ready) showView("setup");
      setError(null);
      if (!selectedId && next.reports[0]) setSelectedId(next.reports[0].id);
    } catch (cause) {
      setError(String(cause));
    }
  }, [selectedId, showView]);

  useEffect(() => {
    void refresh();
    let dispose: (() => void) | undefined;
    void onProgress((event) => {
      setProgress(event);
      setActivity((items) => [...items.filter((item) => item.stage !== event.stage), event].slice(-8));
    }).then((unlisten) => {
      dispose = unlisten;
    });
    return () => dispose?.();
  }, [refresh]);

  useEffect(() => {
    if (!progress) return;
    const timer = window.setInterval(() => {
      setProgress((current) => current ? { ...current, elapsedSeconds: current.elapsedSeconds + 1 } : null);
    }, 1_000);
    return () => window.clearInterval(timer);
  }, [progress?.jobId]);

  useEffect(() => {
    if (!selectedId) return;
    let active = true;
    setReport(null);
    void getReport(selectedId)
      .then((next) => active && setReport(next))
      .catch((cause) => active && setError(String(cause)));
    return () => {
      active = false;
    };
  }, [selectedId]);

  const visibleReports = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    if (!needle) return snapshot?.reports ?? [];
    return (snapshot?.reports ?? []).filter((item) => `${item.title} ${item.query} ${item.dek}`.toLowerCase().includes(needle));
  }, [filter, snapshot]);

  const handleResearch = async (query: string, depth: "focused" | "thorough" | "expedition") => {
    setNewResearchOpen(false);
    setActivity([]);
    setProgress({ ...emptyProgress, detail: `Preparing “${query}”` });
    setError(null);
    try {
      const next = await runResearch({ query, depth });
      setReport(next);
      setSelectedId(next.id);
      setProgress(null);
      await refresh();
    } catch (cause) {
      setProgress(null);
      setError(String(cause));
    }
  };

  const cleanCompetingGpuMemory = useCallback(async (): Promise<string> => {
    setError(null);
    const preview = await previewVramCleanup();
    if (preview.gpu) {
      setSnapshot((current) => current
        ? { ...current, control: { ...current.control, gpu: preview.gpu } }
        : current);
    }
    if (!preview.candidates.length && !preview.exclusions.length) return "VRAM ready";
    setVramCleanup(preview);
    return "";
  }, []);

  const applyVramCleanup = useCallback(async (approvedPids: number[]): Promise<VramCleanupResult> => {
    setError(null);
    const result = await cleanVram(approvedPids);
    if (result.afterGpu) {
      setSnapshot((current) => current
        ? { ...current, control: { ...current.control, gpu: result.afterGpu } }
        : current);
    }
    if (result.failed.length) {
      const failures = result.failed
        .slice(0, 4)
        .map((failure) => `${failure.process.name}: ${failure.detail}`)
        .join(" ");
      setError(`VRAM cleanup was only partly completed. ${failures}`);
    }
    return result;
  }, []);

  const forceVramCleanup = useCallback(async (expectedProcesses: GpuMemoryProcess[]): Promise<VramCleanupResult> => {
    setError(null);
    const result = await forceCleanVram(expectedProcesses);
    if (result.afterGpu) {
      setSnapshot((current) => current
        ? { ...current, control: { ...current.control, gpu: result.afterGpu } }
        : current);
    }
    if (result.failed.length) {
      setError("Some processes still need an administrator PowerShell. Copy only the commands Kestrel provides in the cleanup result.");
    }
    return result;
  }, []);

  const releaseKestrelAiMemory = useCallback(async (): Promise<string> => {
    if (!window.confirm("Release all AI memory controlled by Kestrel? Active local work will stop; unrelated applications are left alone.")) return "";
    setError(null);
    const control = await releaseAiMemory();
    setSnapshot((current) => current ? {
      ...current,
      control,
      status: { ...current.status, modelRuntime: "stopped" },
    } : current);
    return "AI memory released";
  }, []);

  const prepare = async () => {
    setProgress(emptyProgress);
    try {
      setSnapshot(await prepareServices());
      setProgress(null);
    } catch (cause) {
      setProgress(null);
      setError(String(cause));
    }
  };

  if (!snapshot) return <AppBoot error={error} onRetry={refresh} />;

  const chapter = (id: AppView, content: ReactNode) => mountedViews.has(id) && (
    <section className="retained-app-view" data-view-id={id} hidden={view !== id} aria-hidden={view !== id}>
      {content}
    </section>
  );

  return (
    <>
      <BookShell
        view={view}
        onView={showView}
        banner={(controls) => (
          <BookBanner
            view={view}
            status={snapshot.status}
            gpu={snapshot.control.gpu}
            controls={controls}
            onPrepare={() => void prepare()}
            onCleanVram={cleanCompetingGpuMemory}
            onReleaseAiMemory={releaseKestrelAiMemory}
            onError={handleError}
          />
        )}
      >
        <main className={`main-stage main-stage-${view}`}>
          {chapter("setup", <SetupConsole snapshot={snapshot} onChanged={setSnapshot} onError={handleError} />)}
          {chapter("control", (
            <ControlPlane
              control={snapshot.control}
              visible={view === "control"}
              onChanged={(control) => setSnapshot((current) => current ? { ...current, control } : current)}
              onError={handleError}
            />
          ))}
          {chapter("research", (
            <ResearchChapter
              reports={visibleReports}
              totalReports={snapshot.reports.length}
              selectedId={selectedId}
              report={report}
              filter={filter}
              root={snapshot.libraryRoot}
              onFilter={setFilter}
              onSelect={setSelectedId}
              onNew={() => setNewResearchOpen(true)}
              onReveal={() => void revealLibrary()}
              onStandalone={(id) => void openStandalone(id)}
            />
          ))}
          {chapter("studio", <MovieStudio initialComfyRoot={snapshot.settings.comfyRoot} advancedEnabled={snapshot.control.settings.advancedMode} models={snapshot.control.models} selectedModelId={snapshot.control.settings.selectedModelId ?? undefined} controlSettings={snapshot.control.settings} onError={handleError} />)}
          {chapter("image", <ImageStudio initialComfyRoot={snapshot.settings.comfyRoot} advancedEnabled={snapshot.control.settings.advancedMode} models={snapshot.control.models} selectedModelId={snapshot.control.settings.selectedModelId ?? undefined} controlSettings={snapshot.control.settings} onError={handleError} />)}
          {chapter("music", <MusicStudio initialComfyRoot={snapshot.settings.comfyRoot} installRoot={snapshot.settings.installRoot} muscriptorSetupReady={snapshot.setup.components.find((component) => component.id === "muscriptor")?.status === "ready"} advancedEnabled={snapshot.control.settings.advancedMode} models={snapshot.control.models} selectedModelId={snapshot.control.settings.selectedModelId ?? undefined} controlSettings={snapshot.control.settings} onError={handleError} />)}
          {chapter("developer", (
            <DeveloperConsole
              control={snapshot.control}
              onChanged={(control) => setSnapshot((current) => current ? { ...current, control } : current)}
              onError={handleError}
            />
          ))}
          {chapter("system", (
            <SystemConsole
              visible={view === "system"}
              initialSettings={snapshot.settings}
              initialControl={snapshot.control.settings}
              onSaved={(settings) => setSnapshot((current) => current ? { ...current, settings } : current)}
              onControlSaved={(control) => setSnapshot((current) => current ? { ...current, control } : current)}
              onImported={(next) => setSnapshot(next)}
              onError={handleError}
            />
          ))}
        </main>
      </BookShell>
      {error && <ErrorBanner message={error} onClose={() => setError(null)} />}
      {vramCleanup && <VramCleanupDialog
        preview={vramCleanup}
        onClose={() => setVramCleanup(null)}
        onClean={applyVramCleanup}
        onForce={forceVramCleanup}
      />}
      {newResearchOpen && <NewResearchDialog advancedEnabled={snapshot.settings.advancedMode} onClose={() => setNewResearchOpen(false)} onSubmit={handleResearch} />}
      {progress && (
        <ResearchProgressPanel
          progress={progress}
          activity={activity}
          onCancel={() => {
            if (progress.jobId) void cancelResearch(progress.jobId);
            setProgress(null);
          }}
        />
      )}
    </>
  );
}

function AppBoot({ error, onRetry }: { error: string | null; onRetry: () => Promise<void> }) {
  return (
    <div className="app-boot">
      <div className="banner-mark large"><Bird size={30} /></div>
      <h1>Kestrel</h1>
      <p>{error ?? "Opening your private research library…"}</p>
      {error ? <button className="primary-button" onClick={() => void onRetry()}>Try again</button> : <LoaderCircle className="spin" />}
    </div>
  );
}

/** A slip pinned over the book when something needs the producer's attention. */
function ErrorBanner({ message, onClose }: { message: string; onClose: () => void }) {
  return <div className="error-banner" role="alert"><div><strong>Kestrel needs attention</strong><span>{message}</span></div><button className="icon-button" aria-label="Dismiss" onClick={onClose}><X /></button></div>;
}

export default App;
