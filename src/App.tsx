import {
  Archive,
  ArrowLeft,
  BookOpen,
  Check,
  Clapperboard,
  ChevronRight,
  CircleStop,
  Clock3,
  Code2,
  Cpu,
  Download,
  ExternalLink,
  Feather,
  FileText,
  FolderOpen,
  Gauge,
  History,
  Headphones,
  Image as ImageIcon,
  Layers3,
  Library,
  LoaderCircle,
  MemoryStick,
  Menu,
  MessageSquare,
  MonitorCog,
  Plus,
  RefreshCw,
  Search,
  ShieldCheck,
  Sparkles,
  TriangleAlert,
  Upload,
  Wrench,
  X,
  Zap,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  bootstrap,
  cancelResearch,
  applyModelRuntime,
  exportSetupProfileText,
  exportPromptPackText,
  getControlSnapshot,
  getReport,
  getSetupProfileText,
  getPromptPackText,
  getDefaultPromptPackText,
  getSystemSnapshot,
  importSetupProfile,
  importSetupProfileText,
  importPromptPack,
  onProgress,
  pickPromptPackFile,
  openStandalone,
  prepareServices,
  releaseAiMemory,
  resetPromptPack,
  revealLibrary,
  runResearch,
  saveControlSettings,
  savePromptPackText,
  saveResearchSettings,
} from "./api";
import { ControlPlane, DeveloperConsole } from "./ControlPlane";
import { MovieStudio } from "./MovieStudio";
import { MusicStudio } from "./MusicStudio";
import { ImageStudio } from "./ImageStudio";
import { PromptPackVisualEditor } from "./PromptPackVisualEditor";
import { ResearchSpeechPlayer } from "./ResearchSpeech";
import { SetupConsole } from "./Setup";
import {
  STANDARD_CONTEXT_OPTIONS,
  STANDARD_OUTPUT_OPTIONS,
  findProvenHardwareProfile,
} from "./types";
import type {
  AppSnapshot,
  ControlSettings,
  ControlSnapshot,
  ProgressStage,
  ReportSummary,
  ResearchProgress,
  ResearchReport,
  ResearchSettings,
  ServiceState,
  SystemSnapshot,
  ThinkingLevel,
} from "./types";

const emptyProgress: ResearchProgress = {
  jobId: "",
  stage: "preparing",
  title: "Preparing research",
  detail: "Checking the private library and local services…",
  current: 0,
  total: 6,
  elapsedSeconds: 0,
};

const stageOrder: ProgressStage[] = ["preparing", "library", "searching", "reading", "synthesizing", "publishing"];
const stageNames: Record<ProgressStage, string> = {
  preparing: "Prepare",
  library: "Check library",
  searching: "Search",
  reading: "Read sources",
  synthesizing: "Synthesize",
  publishing: "Publish",
  complete: "Complete",
  cancelled: "Cancelled",
  failed: "Failed",
};

type AppView = "setup" | "control" | "research" | "studio" | "image" | "music" | "developer" | "system";

function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [report, setReport] = useState<ResearchReport | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [newResearchOpen, setNewResearchOpen] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [progress, setProgress] = useState<ResearchProgress | null>(null);
  const [activity, setActivity] = useState<ResearchProgress[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<AppView>("research");

  const refresh = useCallback(async () => {
    try {
      const next = await bootstrap();
      setSnapshot(next);
      if (!next.setup.ready) setView("setup");
      setError(null);
      if (!selectedId && next.reports[0]) setSelectedId(next.reports[0].id);
    } catch (cause) {
      setError(String(cause));
    }
  }, [selectedId]);

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

  const chooseReport = (id: string) => {
    setSelectedId(id);
    setSidebarOpen(false);
  };

  if (!snapshot) return <AppBoot error={error} onRetry={refresh} />;

  return (
    <div className={`app-shell app-shell-${view}`} data-view={view}>
      <AppHeader
        status={snapshot.status}
        view={view}
        onView={setView}
        onMenu={() => setSidebarOpen((value) => !value)}
        onNew={() => setNewResearchOpen(true)}
        onPrepare={async () => {
          setProgress(emptyProgress);
          try {
            setSnapshot(await prepareServices());
            setProgress(null);
          } catch (cause) {
            setProgress(null);
            setError(String(cause));
          }
        }}
      />
      <div className={`workspace workspace-${view} ${view !== "research" ? "system-workspace" : ""}`}>
        {view === "research" && <LibrarySidebar
          open={sidebarOpen}
          reports={visibleReports}
          selectedId={selectedId}
          filter={filter}
          root={snapshot.libraryRoot}
          onFilter={setFilter}
          onSelect={chooseReport}
          onNew={() => setNewResearchOpen(true)}
          onReveal={() => void revealLibrary()}
        />}
        <main className={`main-stage main-stage-${view}`}>
          {error && <ErrorBanner message={error} onClose={() => setError(null)} />}
          {view === "setup" ? (
            <SetupConsole snapshot={snapshot} onChanged={setSnapshot} onError={(message) => setError(message)} />
          ) : view === "control" ? (
            <ControlPlane
              control={snapshot.control}
              onChanged={(control) => setSnapshot((current) => current ? { ...current, control } : current)}
              onError={(message) => setError(message)}
            />
          ) : view === "studio" ? (
            <MovieStudio initialComfyRoot={snapshot.settings.comfyRoot} advancedEnabled={snapshot.control.settings.advancedMode} models={snapshot.control.models} selectedModelId={snapshot.control.settings.selectedModelId} controlSettings={snapshot.control.settings} onError={(message) => setError(message)} />
          ) : view === "music" ? (
            <MusicStudio initialComfyRoot={snapshot.settings.comfyRoot} installRoot={snapshot.settings.installRoot} muscriptorSetupReady={snapshot.setup.components.find((component) => component.id === "muscriptor")?.status === "ready"} advancedEnabled={snapshot.control.settings.advancedMode} models={snapshot.control.models} selectedModelId={snapshot.control.settings.selectedModelId} controlSettings={snapshot.control.settings} onError={(message) => setError(message)} />
          ) : view === "image" ? (
            <ImageStudio initialComfyRoot={snapshot.settings.comfyRoot} advancedEnabled={snapshot.control.settings.advancedMode} models={snapshot.control.models} selectedModelId={snapshot.control.settings.selectedModelId} controlSettings={snapshot.control.settings} onError={(message) => setError(message)} />
          ) : view === "developer" ? (
            <DeveloperConsole
              control={snapshot.control}
              onChanged={(control) => setSnapshot((current) => current ? { ...current, control } : current)}
              onError={(message) => setError(message)}
            />
          ) : view === "system" ? (
            <SystemConsole
              initialSettings={snapshot.settings}
              initialControl={snapshot.control.settings}
              onSaved={(settings) => setSnapshot((current) => current ? { ...current, settings } : current)}
              onControlSaved={(control) => setSnapshot((current) => current ? { ...current, control } : current)}
              onImported={(next) => setSnapshot(next)}
              onError={(message) => setError(message)}
            />
          ) : !selectedId && snapshot.reports.length === 0 ? (
            <EmptyLibrary onNew={() => setNewResearchOpen(true)} />
          ) : !report ? (
            <ReaderSkeleton />
          ) : (
            <ResearchReader report={report} onStandalone={() => void openStandalone(report.id)} />
          )}
        </main>
      </div>
      {newResearchOpen && <NewResearchDialog advancedEnabled={snapshot.settings.advancedMode} onClose={() => setNewResearchOpen(false)} onSubmit={handleResearch} />}
      {progress && (
        <ProgressPanel
          progress={progress}
          activity={activity}
          onCancel={() => {
            if (progress.jobId) void cancelResearch(progress.jobId);
            setProgress(null);
          }}
        />
      )}
    </div>
  );
}

function AppBoot({ error, onRetry }: { error: string | null; onRetry: () => Promise<void> }) {
  return (
    <div className="app-boot">
      <div className="brand-mark large"><Feather size={28} /></div>
      <h1>Kestrel</h1>
      <p>{error ?? "Opening your private research library…"}</p>
      {error ? <button className="primary-button" onClick={() => void onRetry()}>Try again</button> : <LoaderCircle className="spin" />}
    </div>
  );
}

function AppHeader({
  status,
  view,
  onView,
  onMenu,
  onNew,
  onPrepare,
}: {
  status: AppSnapshot["status"];
  view: AppView;
  onView: (view: AppView) => void;
  onMenu: () => void;
  onNew: () => void;
  onPrepare: () => void;
}) {
  const allReady = status.modelRuntime === "ready" && status.wikipedia === "ready";
  const sectionLabel = view === "setup" ? "Setup" : view === "control" ? "Control plane" : view === "studio" ? "Movie Studio" : view === "image" ? "Image Studio" : view === "music" ? "Music Production" : view === "developer" ? "Developer" : view === "system" ? "System" : "Research";
  return (
    <header className={`app-header app-header-${view}`}>
      <div className="header-left">
        <div className="window-controls" aria-hidden="true"><span /><span /><span /></div>
        <button type="button" className="icon-button menu-button" aria-label="Toggle library" onClick={onMenu}><Menu /></button>
        <div className="brand-mark"><Feather size={19} /></div>
        <div className="brand-copy"><strong>Kestrel</strong><span>{sectionLabel}</span></div>
      </div>
      <nav className="view-switcher" aria-label="Kestrel sections">
        <button type="button" className={view === "setup" ? "active" : ""} aria-current={view === "setup" ? "page" : undefined} title="Setup" onClick={() => onView("setup")}><Download size={14} /> Setup</button>
        <button type="button" className={view === "control" ? "active" : ""} aria-current={view === "control" ? "page" : undefined} title="Control" onClick={() => onView("control")}><MessageSquare size={14} /> Control</button>
        <button type="button" className={view === "research" ? "active" : ""} aria-current={view === "research" ? "page" : undefined} title="Research" onClick={() => onView("research")}><Library size={14} /> Research</button>
        <button type="button" className={view === "studio" ? "active" : ""} aria-current={view === "studio" ? "page" : undefined} title="Studio" onClick={() => onView("studio")}><Clapperboard size={14} /> Studio</button>
        <button type="button" className={view === "image" ? "active" : ""} aria-current={view === "image" ? "page" : undefined} title="Image" onClick={() => onView("image")}><ImageIcon size={14} /> Image</button>
        <button type="button" className={view === "music" ? "active" : ""} aria-current={view === "music" ? "page" : undefined} title="Music" onClick={() => onView("music")}><Headphones size={14} /> Music</button>
        <button type="button" className={view === "developer" ? "active" : ""} aria-current={view === "developer" ? "page" : undefined} title="Developer" onClick={() => onView("developer")}><Wrench size={14} /> Developer</button>
        <button type="button" className={view === "system" ? "active" : ""} aria-current={view === "system" ? "page" : undefined} title="System" onClick={() => onView("system")}><MonitorCog size={14} /> System</button>
      </nav>
      <div className="header-status" role="status">
        <StatusPill state={status.wikipedia} label={status.archive} />
        <StatusPill state={status.modelRuntime} label={status.model} />
        <div className="privacy-pill" aria-label="Offline only" title="Offline only"><ShieldCheck size={14} /> Offline only</div>
      </div>
      <div className="header-actions">
        {!allReady && <button className="quiet-button" onClick={onPrepare}>Prepare services</button>}
        {view === "research" && <button className="primary-button compact" onClick={onNew}><Plus size={16} /> New research</button>}
      </div>
    </header>
  );
}

function StatusPill({ state, label }: { state: ServiceState; label: string }) {
  return <div className={`status-pill status-${state}`} aria-label={`${label}: ${state}`} title={`${label}: ${state}`}><span className="status-dot" />{label}</div>;
}

function LibrarySidebar({
  open,
  reports,
  selectedId,
  filter,
  root,
  onFilter,
  onSelect,
  onNew,
  onReveal,
}: {
  open: boolean;
  reports: ReportSummary[];
  selectedId: string | null;
  filter: string;
  root: string;
  onFilter: (value: string) => void;
  onSelect: (id: string) => void;
  onNew: () => void;
  onReveal: () => void;
}) {
  return (
    <aside className={`library-sidebar ${open ? "sidebar-open" : ""}`}>
      <div className="sidebar-heading">
        <div><span className="eyebrow">Private library</span><h2>Your research</h2></div>
        <button className="icon-button sidebar-new" aria-label="New research" onClick={onNew}><Plus /></button>
      </div>
      <label className="search-field">
        <Search size={16} />
        <input value={filter} onChange={(event) => onFilter(event.target.value)} placeholder="Find past research" />
        {filter && <button aria-label="Clear search" onClick={() => onFilter("")}><X size={14} /></button>}
      </label>
      <div className="library-count"><Library size={14} /> {reports.length.toLocaleString()} reports</div>
      <nav className="report-list" aria-label="Research library">
        {reports.map((item) => (
          <button key={item.id} className={`report-list-item ${item.id === selectedId ? "selected" : ""}`} onClick={() => onSelect(item.id)}>
            <span className="report-item-title">{item.title}</span>
            <span className="report-item-dek">{item.dek}</span>
            <span className="report-item-meta"><span>Edition {item.edition}</span><span>{item.sourceCount} sources</span><span>{item.readingMinutes} min</span></span>
          </button>
        ))}
        {!reports.length && <div className="empty-list"><Search size={20} /><span>No matching research</span></div>}
      </nav>
      <button className="library-root" onClick={onReveal} title={root}>
        <FolderOpen size={16} />
        <span><strong>Research files</strong><small>{root}</small></span>
        <ChevronRight size={15} />
      </button>
    </aside>
  );
}

function ResearchReader({ report, onStandalone }: { report: ResearchReport; onStandalone: () => void }) {
  const [sourceFocus, setSourceFocus] = useState<string | null>(null);
  const [spokenAnchor, setSpokenAnchor] = useState<string | null>(null);
  const sourceMap = useMemo(() => new Map(report.sources.map((source) => [source.id, source])), [report.sources]);
  const handleSpeechPassage = useCallback((anchorId: string | null) => {
    setSpokenAnchor(anchorId);
    if (anchorId) document.getElementById(anchorId)?.scrollIntoView({ behavior: "smooth", block: "center" });
  }, []);
  const focusSource = (id: string) => {
    setSourceFocus(id);
    document.getElementById(`source-${id}`)?.scrollIntoView({ behavior: "smooth", block: "center" });
  };
  return (
    <div className="reader-layout">
      <article className="research-article">
        <nav className="reader-breadcrumb"><button><ArrowLeft size={15} /> Library</button><span>/</span><span>{report.title}</span></nav>
        <header className={`report-header ${spokenAnchor === "report-overview" ? "speech-active" : ""}`} id="report-overview">
          <div className="report-kicker"><span>Research brief</span><span>Edition {report.edition}</span><span>{formatDate(report.updatedAt)}</span></div>
          <h1>{report.title}</h1>
          <p className="report-dek">{report.dek}</p>
          <div className="report-byline">
            <span><Clock3 size={15} /> {report.readingMinutes} min read</span>
            <span><BookOpen size={15} /> {report.sources.length} inspected sources</span>
            <span><FileText size={15} /> {report.wordCount.toLocaleString()} words</span>
          </div>
        </header>

        <ResearchSpeechPlayer report={report} onPassageChange={handleSpeechPassage} />

        <section className={`answer-card ${spokenAnchor === "short-answer" ? "speech-active" : ""}`} id="short-answer" aria-labelledby="short-answer-title">
          <div className="section-label" id="short-answer-title"><Sparkles size={16} /> Short answer</div>
          <p>{report.answer}</p>
        </section>

        {report.edition > 1 && (
          <aside className={`improvement-note ${spokenAnchor === "edition-improvement" ? "speech-active" : ""}`} id="edition-improvement">
            <div className="improvement-icon"><History size={17} /></div>
            <div><strong>What changed in this edition</strong><p>{report.improvement}</p></div>
            <span className="edition-badge">v{report.edition}</span>
          </aside>
        )}

        <section className={`content-section ${spokenAnchor === "findings" ? "speech-active" : ""}`} id="findings">
          <div className="section-heading"><span className="section-number">01</span><div><span className="eyebrow">The evidence at a glance</span><h2>Key findings</h2></div></div>
          <div className="findings-grid">
            {report.findings.map((finding, index) => (
              <div className="finding-card" key={finding.title}>
                <span className="finding-number">{String(index + 1).padStart(2, "0")}</span>
                <h3>{finding.title}</h3>
                <p>{finding.explanation}</p>
                <CitationRow ids={finding.citations} onFocus={focusSource} />
              </div>
            ))}
          </div>
        </section>

        {report.sections.map((section, index) => (
          <section className={`content-section narrative-section ${spokenAnchor === section.id ? "speech-active" : ""}`} id={section.id} key={section.id}>
            <div className="section-heading"><span className="section-number">{String(index + 2).padStart(2, "0")}</span><div><span className="eyebrow">Deep dive</span><h2>{section.heading}</h2></div></div>
            <p className="section-summary">{section.summary}</p>
            {section.body.map((paragraph, paragraphIndex) => <p key={paragraphIndex}>{paragraph}</p>)}
            <CitationRow ids={section.citations} onFocus={focusSource} labels={sourceMap} />
          </section>
        ))}

        {!!report.timeline.length && (
          <section className={`content-section ${spokenAnchor === "timeline" ? "speech-active" : ""}`} id="timeline">
            <div className="section-heading"><span className="section-number">{String(report.sections.length + 2).padStart(2, "0")}</span><div><span className="eyebrow">Sequence</span><h2>Timeline</h2></div></div>
            <div className="timeline">
              {report.timeline.map((item) => (
                <div className="timeline-item" key={`${item.date}-${item.label}`}>
                  <div className="timeline-date">{item.date}</div><div className="timeline-marker" /><div><h3>{item.label}</h3><p>{item.description}</p><CitationRow ids={item.citations} onFocus={focusSource} /></div>
                </div>
              ))}
            </div>
          </section>
        )}

        <section className={`content-section split-section ${spokenAnchor === "terms" ? "speech-active" : ""}`} id="terms">
          <div>
            <div className="section-heading small"><div><span className="eyebrow">Plain language</span><h2>Terms worth knowing</h2></div></div>
            <dl className="term-list">{report.terms.map((term) => <div key={term.term}><dt>{term.term}</dt><dd>{term.meaning}</dd></div>)}</dl>
          </div>
          <div>
            <div className="section-heading small"><div><span className="eyebrow">Research frontier</span><h2>What remains open</h2></div></div>
            <ol className="question-list">{report.openQuestions.map((question) => <li key={question}>{question}</li>)}</ol>
          </div>
        </section>

        <section className={`content-section sources-section ${spokenAnchor === "sources" ? "speech-active" : ""}`} id="sources">
          <div className="section-heading"><span className="section-number">{String(report.sections.length + 3).padStart(2, "0")}</span><div><span className="eyebrow">Evidence ledger</span><h2>Sources inspected</h2></div></div>
          <p className="sources-intro">Every source below was opened by the local model. Excerpts show the evidence it received; Wikipedia is a tertiary starting point, not a substitute for primary sources.</p>
          <div className="source-list">
            {report.sources.map((source) => (
              <div id={`source-${source.id}`} className={`source-card ${sourceFocus === source.id ? "focused" : ""}`} key={source.id}>
                <span className="source-id">{source.id}</span>
                <div><div className="source-title-row"><h3>{source.title}</h3><span>{source.kind === "wikipedia" ? "Wikipedia" : "Kestrel research"}</span></div>
                  <p className="source-location">{source.section ?? "Full article"} · snapshot {source.snapshot ?? report.archiveSnapshot}</p>
                  <blockquote>{source.excerpt}</blockquote>
                </div>
              </div>
            ))}
          </div>
        </section>

        <footer className="report-footer">
          <div><ShieldCheck size={16} /><span>Produced entirely on this computer with {report.model} and {report.archiveSnapshot}.</span></div>
          <button className="quiet-button" onClick={onStandalone}><ExternalLink size={15} /> Open standalone HTML</button>
        </footer>
      </article>
      <aside className="reader-rail">
        <div className="rail-card">
          <span className="eyebrow">On this page</span>
          <a href="#findings">Key findings</a>
          {report.sections.map((section) => <a href={`#${section.id}`} key={section.id}>{section.heading}</a>)}
          {!!report.timeline.length && <a href="#timeline">Timeline</a>}
          <a href="#terms">Terms & questions</a>
          <a href="#sources">Sources</a>
        </div>
        <div className="rail-card context-card"><Archive size={18} /><strong>Research context</strong><span>{report.model}</span><span>{report.archiveSnapshot}</span>{report.researchProfile === "solo-expedition" && <><span>{report.researchLanes} coordinated lanes</span><span>{report.contextWindow.toLocaleString()} context · {report.outputBudget.toLocaleString()} output</span></>}<span>Edition {report.edition}, never overwritten</span></div>
      </aside>
    </div>
  );
}

function CitationRow({ ids, onFocus, labels }: { ids: string[]; onFocus: (id: string) => void; labels?: Map<string, { title: string }> }) {
  return <div className="citation-row" aria-label="Citations">{ids.map((id) => <button key={id} onClick={() => onFocus(id)} title={labels?.get(id)?.title ?? `Source ${id}`}>{id}</button>)}</div>;
}

function SystemConsole({ initialSettings, initialControl, onSaved, onControlSaved, onImported, onError }: { initialSettings: ResearchSettings; initialControl: ControlSettings; onSaved: (settings: ResearchSettings) => void; onControlSaved: (control: ControlSnapshot) => void; onImported: (snapshot: AppSnapshot) => void; onError: (message: string) => void }) {
  const [system, setSystem] = useState<SystemSnapshot | null>(null);
  const [researchDraft, setResearchDraft] = useState(initialSettings);
  const [controlDraft, setControlDraft] = useState(initialControl);
  const [tab, setTab] = useState<"models" | "research" | "prompts" | "portable">("models");
  const [overrideModelId, setOverrideModelId] = useState(initialControl.selectedModelId ?? "");
  const [busy, setBusy] = useState<"save-models" | "save-research" | "apply" | "release" | "export" | "import" | "refresh-profile" | "save-prompts" | "reset-prompts" | "export-prompts" | "import-prompts" | "reload-prompts" | null>(null);
  const [profilePath, setProfilePath] = useState("");
  const [profileText, setProfileText] = useState("");
  const [profileStatus, setProfileStatus] = useState("");
  const [promptText, setPromptText] = useState("");
  const [lastAppliedPromptText, setLastAppliedPromptText] = useState("");
  const [defaultPromptText, setDefaultPromptText] = useState("");
  const [promptView, setPromptView] = useState<"visual" | "raw">("visual");
  const [promptPath, setPromptPath] = useState("");
  const [promptStatus, setPromptStatus] = useState("");

  const refreshSystem = useCallback(async () => {
    try {
      const next = await getSystemSnapshot();
      setSystem(next);
      setOverrideModelId((current) => current || next.control.selectedModelId || next.models[0]?.id || "");
    } catch (cause) {
      onError(String(cause));
    }
  }, [onError]);

  const refreshPromptText = useCallback(async () => {
    setBusy("reload-prompts");
    try {
      const text = await getPromptPackText();
      setPromptText(text);
      setLastAppliedPromptText(text);
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  }, [onError]);

  useEffect(() => {
    void refreshSystem();
    void getSetupProfileText().then(setProfileText).catch((cause) => onError(String(cause)));
    void refreshPromptText();
    void getDefaultPromptPackText().then(setDefaultPromptText).catch(() => { /* per-prompt "reset to default" stays disabled if this fails */ });
    const timer = window.setInterval(() => void refreshSystem(), 2_500);
    return () => window.clearInterval(timer);
  }, [refreshSystem, onError, refreshPromptText]);

  const updateResearchNumber = (key: keyof ResearchSettings, value: string) => {
    const parsed = Number.parseInt(value, 10);
    if (Number.isFinite(parsed) && parsed > 0) setResearchDraft((current) => ({ ...current, [key]: parsed }));
  };
  const updateControlNumber = (key: "contextWindow" | "maxOutputTokens" | "threads", value: string) => {
    const parsed = Number.parseInt(value, 10);
    if (Number.isFinite(parsed) && parsed > 0) setControlDraft((current) => ({ ...current, [key]: parsed }));
  };
  const modelOverride = controlDraft.modelOverrides.find((item) => item.modelId === overrideModelId);
  const updateOverrideNumber = (key: "contextWindow" | "maxOutputTokens" | "threads", value: string) => {
    const parsed = Number.parseInt(value, 10);
    if (!overrideModelId || !Number.isFinite(parsed) || parsed <= 0) return;
    setControlDraft((current) => {
      const known = current.modelOverrides.find((item) => item.modelId === overrideModelId) ?? { modelId: overrideModelId };
      return { ...current, modelOverrides: [...current.modelOverrides.filter((item) => item.modelId !== overrideModelId), { ...known, [key]: parsed }] };
    });
  };
  const toggleOverride = (enabled: boolean) => {
    if (!overrideModelId) return;
    setControlDraft((current) => ({
      ...current,
      modelOverrides: enabled
        ? [...current.modelOverrides.filter((item) => item.modelId !== overrideModelId), { modelId: overrideModelId, contextWindow: current.contextWindow, maxOutputTokens: current.maxOutputTokens, threads: current.threads, thinkingLevel: current.thinkingLevel }]
        : current.modelOverrides.filter((item) => item.modelId !== overrideModelId),
    }));
  };
  const saveModels = async () => {
    setBusy("save-models");
    try {
      const saved = await saveControlSettings(controlDraft);
      setControlDraft(saved.settings);
      onControlSaved(saved);
      setProfileStatus("App-wide model policy saved. A loaded model keeps its current launch until restarted.");
      await refreshSystem();
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const saveResearch = async () => {
    setBusy("save-research");
    try {
      const saved = await saveResearchSettings(researchDraft);
      setResearchDraft(saved);
      onSaved(saved);
      setSystem((current) => current ? { ...current, settings: saved } : current);
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const apply = async () => {
    if (!window.confirm("Save this app-wide policy and restart the selected local model? Active local-model work will be interrupted.")) return;
    setBusy("apply");
    try {
      const next = await applyModelRuntime(controlDraft);
      setSystem(next);
      setControlDraft(next.control);
      onControlSaved(await getControlSnapshot(false));
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const releaseMemory = async () => {
    if (!window.confirm("Release all AI memory controlled by Kestrel? Active local work will stop; unrelated model applications are left alone.")) return;
    setBusy("release");
    try {
      await releaseAiMemory();
      await refreshSystem();
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const refreshProfileText = async () => {
    setBusy("refresh-profile");
    try {
      setProfileText(await getSetupProfileText());
      setProfileStatus("Editable JSON refreshed from the current app-wide setup.");
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const exportProfile = async () => {
    setBusy("export");
    try {
      const transfer = await exportSetupProfileText(profileText);
      setProfilePath(transfer.path);
      setProfileStatus(transfer.message);
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const acceptImported = async (next: AppSnapshot, message: string) => {
    setResearchDraft(next.settings);
    setControlDraft(next.control.settings);
    setOverrideModelId(next.control.settings.selectedModelId ?? next.control.models[0]?.id ?? "");
    onImported(next);
    setProfileText(await getSetupProfileText());
    setProfileStatus(message);
    await refreshSystem();
  };
  const importProfilePath = async () => {
    const path = profilePath.trim();
    if (!path || !window.confirm("Import this setup profile? Existing local paths are used only when they validate, and trust grants remain locked.")) return;
    setBusy("import");
    try {
      await acceptImported(await importSetupProfile(path), "Profile imported, local components rescanned, and trust grants left unchanged.");
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const importProfileText = async () => {
    if (!profileText.trim() || !window.confirm("Apply the edited setup JSON? Kestrel validates every value and local path before saving.")) return;
    setBusy("import");
    try {
      await acceptImported(await importSetupProfileText(profileText), "Edited setup JSON validated and applied across Kestrel.");
    } catch (cause) {
      onError(String(cause));
    } finally {
      setBusy(null);
    }
  };
  const savePrompts = async () => {
    setBusy("save-prompts");
    try { const next = await savePromptPackText(promptText); setPromptText(next); setLastAppliedPromptText(next); setPromptStatus("Validated and applied to future local-model requests. Active requests keep their captured payload."); }
    catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const resetPrompts = async () => {
    if (!window.confirm("Reset every app-owned prompt to this Kestrel build's defaults?")) return;
    setBusy("reset-prompts");
    try { const next = await resetPromptPack(); setPromptText(next); setLastAppliedPromptText(next); setPromptStatus("Default prompt pack restored."); }
    catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const exportPrompts = async () => {
    setBusy("export-prompts");
    try { const transfer = await exportPromptPackText(promptText); setPromptPath(transfer.path); setPromptStatus(transfer.message); }
    catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const importPrompts = async () => {
    if (!promptPath.trim() || !window.confirm("Import and activate this prompt-only pack for future local-model requests?")) return;
    setBusy("import-prompts");
    try { const next = await importPromptPack(promptPath.trim()); setPromptText(next); setLastAppliedPromptText(next); setPromptStatus("Prompt pack validated, imported, and activated."); }
    catch (cause) { onError(String(cause)); } finally { setBusy(null); }
  };
  const gpu = system?.gpu;
  const usedPercent = gpu ? Math.min(100, (gpu.usedMib / gpu.totalMib) * 100) : 0;
  const models = system?.models ?? [];
  const activeModel = system?.managedRuntime.modelName ?? models.find((item) => item.id === controlDraft.selectedModelId)?.name ?? "No model selected";

  return (
    <div className="system-console">
      <div className="system-console-top">
        <header className="system-hero">
          <div><span className="eyebrow">One runtime policy · every local model</span><h1>System</h1><p>Choose app-wide defaults once. Explicit per-model and workspace settings override them without creating a second server or a hidden model-specific control path.</p></div>
          <div className="system-hero-actions"><button className="quiet-button" disabled={!!busy} onClick={() => void releaseMemory()}>{busy === "release" ? <LoaderCircle className="spin" size={15}/> : <CircleStop size={15}/>} Release AI memory</button><button className="quiet-button" onClick={() => void refreshSystem()}><RefreshCw size={15} /> Refresh</button></div>
        </header>

      <section className="telemetry-grid" aria-label="Live system telemetry">
        <article className="telemetry-card gpu-card">
          <div className="telemetry-title"><Gauge /><span><small>Detected GPU</small><strong>{gpu?.name ?? "GPU telemetry unavailable"}</strong></span></div>
          {gpu && <><div className="vram-number"><strong>{formatMib(gpu.usedMib)}</strong><span>of {formatMib(gpu.totalMib)} used</span></div><div className="vram-track"><span style={{ width: `${usedPercent}%` }} /></div><div className="telemetry-foot"><span>{formatMib(gpu.freeMib)} free</span><span>{gpu.utilizationPercent}% compute</span></div></>}
        </article>
        <article className="telemetry-card"><div className="telemetry-title"><MemoryStick /><span><small>Local model</small><strong>{activeModel}</strong></span></div><p>{system?.managedRuntime.detail ?? "No managed runtime is loaded."}</p></article>
        <article className="telemetry-card"><div className="telemetry-title"><Cpu /><span><small>Effective runtime</small><strong>{(system?.runtime.contextWindow ?? controlDraft.contextWindow).toLocaleString()} context</strong></span></div><div className="runtime-facts"><span>{(system?.runtime.maxOutputTokens ?? controlDraft.maxOutputTokens).toLocaleString()} max output</span><span>1 inference slot</span><span>{controlDraft.modelOverrides.length} model exception{controlDraft.modelOverrides.length === 1 ? "" : "s"}</span></div></article>
      </section>

      <nav className="system-tabs" aria-label="System settings sections"><button className={tab === "models" ? "active" : ""} onClick={() => setTab("models")}><Cpu size={15}/> Model policy</button><button className={tab === "research" ? "active" : ""} onClick={() => setTab("research")}><Library size={15}/> Research policy</button><button className={tab === "prompts" ? "active" : ""} onClick={() => setTab("prompts")}><FileText size={15}/> Prompt pack</button><button className={tab === "portable" ? "active" : ""} onClick={() => setTab("portable")}><ShieldCheck size={15}/> Portable setup</button></nav>
      </div>

      <div className="system-console-body">
      {tab === "models" && (() => {
        const selectedModel = models.find((m) => m.id === controlDraft.selectedModelId);
        const overrideModel = models.find((m) => m.id === overrideModelId);
        const globalProvenProfile = findProvenHardwareProfile(
          system?.provenHardwareProfiles,
          selectedModel?.name ?? selectedModel?.id,
          gpu?.totalMib,
        );
        const overrideProvenProfile = findProvenHardwareProfile(
          system?.provenHardwareProfiles,
          overrideModel?.name ?? overrideModel?.id,
          gpu?.totalMib,
        );

        return (
          <section className="settings-panel system-tab-panel">
            <div className="settings-heading"><div><span className="eyebrow">Fallback everywhere</span><h2>App-wide local model policy</h2><p>Chat, Computer Tasks, Research, and every Studio inherit these values unless their selected model or workspace has an explicit override.</p></div><label className="advanced-toggle"><input type="checkbox" checked={controlDraft.advancedMode} onChange={(event) => setControlDraft((current) => ({ ...current, advancedMode: event.target.checked }))}/><span/><strong>Allow uncapped values</strong></label></div>
            <div className="system-policy-grid">
              <div className="system-policy-column">
                <label className="wide-field"><span>Default local model</span><select value={controlDraft.selectedModelId ?? ""} onChange={(event) => { const value = event.target.value || undefined; setControlDraft((current) => ({ ...current, selectedModelId: value })); setOverrideModelId(event.target.value); }}><option value="">First available model</option>{models.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}</select></label>
                <label className="wide-field"><span>llama.cpp engine</span><input value={controlDraft.enginePath} onChange={(event) => setControlDraft((current) => ({ ...current, enginePath: event.target.value }))}/></label>
                <label className="wide-field"><span>Thinking level (Global fallback)</span><select value={controlDraft.thinkingLevel ?? "high"} onChange={(event) => setControlDraft((current) => ({ ...current, thinkingLevel: event.target.value as ThinkingLevel }))}><option value="off">Off (direct response, no thinking)</option><option value="low">Low reasoning</option><option value="medium">Medium reasoning</option><option value="high">High reasoning (default)</option><option value="max">Max reasoning</option></select></label>
                <div className="model-runtime-row">
                  <TokenDropdownSetting
                    label="Context"
                    hint="Global fallback"
                    value={controlDraft.contextWindow}
                    options={STANDARD_CONTEXT_OPTIONS}
                    recommendedValue={globalProvenProfile?.recommendedContextWindow}
                    recommendedLabel={gpu ? `${formatMib(gpu.totalMib)} GPU` : undefined}
                    disabled={false}
                    allowCustom={controlDraft.advancedMode}
                    onChange={(value) => updateControlNumber("contextWindow", value)}
                  />
                  <TokenDropdownSetting
                    label="Max output"
                    hint="Global fallback"
                    value={controlDraft.maxOutputTokens}
                    options={STANDARD_OUTPUT_OPTIONS}
                    recommendedValue={globalProvenProfile?.recommendedMaxOutputTokens}
                    disabled={false}
                    allowCustom={controlDraft.advancedMode}
                    onChange={(value) => updateControlNumber("maxOutputTokens", value)}
                  />
                  <NumberSetting label="CPU threads" hint="Global fallback" value={controlDraft.threads} disabled={false} onChange={(value) => updateControlNumber("threads", value)}/>
                </div>
              </div>
              <div className="system-policy-column model-exception-card">
                <div className="model-exception-heading"><div><span className="eyebrow">More specific wins</span><strong>Per-model exception</strong></div><label><input type="checkbox" checked={!!modelOverride} disabled={!overrideModelId} onChange={(event) => toggleOverride(event.target.checked)}/> Override this model</label></div>
                <label className="wide-field"><span>Model</span><select value={overrideModelId} onChange={(event) => setOverrideModelId(event.target.value)}><option value="">Choose a model</option>{models.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}</select></label>
                {overrideProvenProfile && modelOverride && (
                  <div className="proven-profile-banner" style={{ display: "flex", justifyContent: "space-between", alignItems: "center", padding: "8px 10px", margin: "4px 0 8px", background: "rgba(183, 232, 102, 0.08)", border: "1px solid #4f683a", borderRadius: 6, fontSize: "11px" }}>
                    <div>
                      <strong style={{ color: "#b7e866" }}>⚡ {overrideProvenProfile.displayName}</strong>
                      <p style={{ margin: "2px 0 0", color: "#9ead9f", fontSize: "10px" }}>{overrideProvenProfile.provenSpeedNotes}</p>
                    </div>
                    <button
                      type="button"
                      className="quiet-button"
                      style={{ fontSize: "10px", padding: "4px 8px" }}
                      disabled={!modelOverride}
                      onClick={() => {
                        if (!overrideModelId) return;
                        setControlDraft((current) => {
                          const known = current.modelOverrides.find((item) => item.modelId === overrideModelId) ?? { modelId: overrideModelId };
                          return {
                            ...current,
                            modelOverrides: [
                              ...current.modelOverrides.filter((item) => item.modelId !== overrideModelId),
                              {
                                ...known,
                                contextWindow: overrideProvenProfile.recommendedContextWindow,
                                maxOutputTokens: overrideProvenProfile.recommendedMaxOutputTokens,
                                thinkingLevel: overrideProvenProfile.recommendedThinkingLevel,
                                threads: overrideProvenProfile.recommendedThreads,
                              },
                            ],
                          };
                        });
                      }}
                    >
                      Auto-tune for GPU
                    </button>
                  </div>
                )}
                <label className="wide-field"><span>Thinking level (Model exception)</span><select disabled={!modelOverride} value={modelOverride?.thinkingLevel ?? controlDraft.thinkingLevel ?? "high"} onChange={(event) => { const val = event.target.value as ThinkingLevel; if (!overrideModelId) return; setControlDraft((current) => { const known = current.modelOverrides.find((item) => item.modelId === overrideModelId) ?? { modelId: overrideModelId }; return { ...current, modelOverrides: [...current.modelOverrides.filter((item) => item.modelId !== overrideModelId), { ...known, thinkingLevel: val }] }; }); }}><option value="off">Off (direct response, no thinking)</option><option value="low">Low reasoning</option><option value="medium">Medium reasoning</option><option value="high">High reasoning</option><option value="max">Max reasoning</option></select></label>
                <div className="model-runtime-row">
                  <TokenDropdownSetting
                    label="Context"
                    hint="Model only"
                    value={modelOverride?.contextWindow ?? controlDraft.contextWindow}
                    options={STANDARD_CONTEXT_OPTIONS}
                    recommendedValue={overrideProvenProfile?.recommendedContextWindow}
                    recommendedLabel={gpu ? `${formatMib(gpu.totalMib)} GPU` : undefined}
                    disabled={!modelOverride}
                    allowCustom={controlDraft.advancedMode}
                    onChange={(value) => updateOverrideNumber("contextWindow", value)}
                  />
                  <TokenDropdownSetting
                    label="Max output"
                    hint="Model only"
                    value={modelOverride?.maxOutputTokens ?? controlDraft.maxOutputTokens}
                    options={STANDARD_OUTPUT_OPTIONS}
                    recommendedValue={overrideProvenProfile?.recommendedMaxOutputTokens}
                    disabled={!modelOverride}
                    allowCustom={controlDraft.advancedMode}
                    onChange={(value) => updateOverrideNumber("maxOutputTokens", value)}
                  />
                  <NumberSetting label="CPU threads" hint="Model only" value={modelOverride?.threads ?? controlDraft.threads} disabled={!modelOverride} onChange={(value) => updateOverrideNumber("threads", value)}/>
                </div>
              </div>
            </div>
            <div className="advanced-warning"><TriangleAlert/><div><strong>No GPU model is assumed.</strong><span>Kestrel uses detected telemetry and the values you save. Uncapped or oversized settings can still exceed a model or machine limit.</span></div></div>
            <div className="settings-actions"><span/><button className="quiet-button" disabled={!!busy} onClick={() => void saveModels()}>{busy === "save-models" ? <LoaderCircle className="spin" size={15}/> : <Check size={15}/>} Save app-wide policy</button><button className="primary-button" disabled={!!busy || models.length === 0} onClick={() => void apply()}>{busy === "apply" ? <LoaderCircle className="spin" size={15}/> : <Zap size={15}/>} Save & restart selected model</button></div>
          </section>
        );
      })()}

      {tab === "research" && <section className="settings-panel system-tab-panel">
        <div className="settings-heading"><div><span className="eyebrow">Workspace-specific override</span><h2>Offline Research policy</h2><p>Standard Research inherits the selected model's System policy. Enable this only when research genuinely needs a different context/output budget or deeper orchestration.</p></div><label className="advanced-toggle"><input type="checkbox" checked={researchDraft.advancedMode} onChange={(event) => setResearchDraft((current) => ({ ...current, advancedMode: event.target.checked }))}/><span/><strong>Research override</strong></label></div>
        <div className={`advanced-settings ${researchDraft.advancedMode ? "enabled" : "disabled"}`}>
          <NumberSetting label="Context override" hint="Research only" value={researchDraft.contextWindow} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("contextWindow", value)}/>
          <NumberSetting label="Output override" hint="Research only" value={researchDraft.maxOutputTokens} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("maxOutputTokens", value)}/>
          <NumberSetting label="Research lanes" hint="Distinct planning angles" value={researchDraft.researchLanes} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("researchLanes", value)}/>
          <NumberSetting label="Results per lane" hint="Compact candidate memory" value={researchDraft.resultsPerLane} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("resultsPerLane", value)}/>
          <NumberSetting label="Source target" hint="Wikipedia pages" value={researchDraft.sourceTarget} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("sourceTarget", value)}/>
          <NumberSetting label="Tool turns" hint="Search/read rounds" value={researchDraft.toolTurns} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("toolTurns", value)}/>
          <NumberSetting label="Thinking budget" hint="Per reasoning pass" value={researchDraft.thinkingBudget} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("thinkingBudget", value)}/>
          <NumberSetting label="Source characters" hint="Per opened section" value={researchDraft.maxSourceChars} disabled={!researchDraft.advancedMode} onChange={(value) => updateResearchNumber("maxSourceChars", value)}/>
        </div>
        <div className="single-context-note"><Zap/><div><strong>One selected model, one inference lease</strong><p>Research searches the local archive concurrently, then coordinates evidence through the same managed runtime used by the rest of Kestrel. It never launches or attaches to a separate model-specific server.</p></div></div>
        <div className="settings-actions"><span/><button className="primary-button" disabled={!!busy} onClick={() => void saveResearch()}>{busy === "save-research" ? <LoaderCircle className="spin" size={15}/> : <Check size={15}/>} Save Research policy</button></div>
      </section>}

      {tab === "prompts" && <section className="settings-panel portability-panel system-tab-panel">
        <div className="settings-heading"><div><span className="eyebrow">Advanced · every local workspace</span><h2>Portable prompt pack</h2><p>One prompt-only JSON document owns Kestrel’s app-authored instructions for chat, Computer Tasks, Research, movie planning and review, image design, music writing, and model qualification. Producer text and generated runtime data remain in their projects.</p></div><FileText/></div>
        <div className="advanced-warning"><TriangleAlert/><div><strong>Prompts guide models; native authority does not move.</strong><span>Editing wording cannot grant filesystem, network, rendering, or tool access, and cannot bypass schema, path, citation, or planning validation. Prompt keys are fixed by this Kestrel build and cannot be added, renamed, or removed.</span></div></div>
        <div className="system-tabs prompt-view-toggle"><button className={promptView === "visual" ? "active" : ""} onClick={() => setPromptView("visual")}><Layers3 size={14}/> Visual editor</button><button className={promptView === "raw" ? "active" : ""} onClick={() => setPromptView("raw")}><Code2 size={14}/> Raw JSON</button></div>
        <div className="portable-editor-grid">
          {promptView === "visual"
            ? <PromptPackVisualEditor jsonText={promptText} savedJsonText={lastAppliedPromptText} defaultJsonText={defaultPromptText} disabled={!!busy} onChange={setPromptText}/>
            : <label className="portable-json"><span>Editable prompt-only JSON</span><textarea value={promptText} disabled={!!busy} onChange={(event) => setPromptText(event.target.value)} spellCheck={false} aria-label="Editable portable prompt pack JSON"/></label>}
          <aside className="portable-file-controls">
            <div className="portable-file-heading">
              <FolderOpen size={15}/>
              <div>
                <strong>Pack actions</strong>
                <small>Import, activate, or reset defaults</small>
              </div>
            </div>
            <label className="wide-field"><span>Import prompt pack path</span><input value={promptPath} onChange={(event) => setPromptPath(event.target.value)} placeholder="C:\\Users\\You\\Kestrel Research\\prompt-packs\\kestrel-prompts.json"/></label>
            {promptStatus && <div className="profile-status" role="status">{promptStatus}</div>}
            <div className="portable-file-buttons">
              <button className="quiet-button" disabled={!!busy} onClick={() => void pickPromptPackFile().then((path) => path && setPromptPath(path)).catch((cause) => onError(String(cause)))}><FolderOpen size={15}/> Choose JSON file</button>
              <button className="quiet-button" disabled={!!busy} onClick={() => void refreshPromptText()}><RefreshCw size={15}/> Reload active pack</button>
              <button className="quiet-button" disabled={!!busy || !promptPath.trim()} onClick={() => void importPrompts()}><Upload size={15}/> Import & activate</button>
              <button className="quiet-button" disabled={!!busy} onClick={() => void resetPrompts()}><RefreshCw size={15}/> Restore build defaults</button>
            </div>
          </aside>
        </div>
        <div className="settings-actions"><button className="quiet-button" disabled={!!busy || !promptText.trim()} onClick={() => void savePrompts()}>{busy === "save-prompts" ? <LoaderCircle className="spin" size={15}/> : <Check size={15}/>} Validate & apply</button><span/><button className="primary-button" disabled={!!busy || !promptText.trim()} onClick={() => void exportPrompts()}>{busy === "export-prompts" ? <LoaderCircle className="spin" size={15}/> : <Download size={15}/>} Export prompt-only JSON</button></div>
      </section>}

      {tab === "portable" && <section className="settings-panel portability-panel system-tab-panel">
        <div className="settings-heading"><div><span className="eyebrow">Entire safe app setup</span><h2>Portable setup JSON</h2><p>This editable document covers component locations, archive settings, global model policy, per-model exceptions, Research policy, and every discovered model identity. It intentionally excludes weights, projects, conversations, developer paths, credentials, and access grants.</p></div><ShieldCheck/></div>
        <div className="portable-editor-grid">
          <label className="portable-json"><span>Editable profile text</span><textarea value={profileText} onChange={(event) => setProfileText(event.target.value)} spellCheck={false} aria-label="Editable portable setup JSON"/></label>
          <aside className="portable-file-controls">
            <div className="portable-file-heading">
              <ShieldCheck size={15}/>
              <div>
                <strong>Setup profile actions</strong>
                <small>Import or refresh from current state</small>
              </div>
            </div>
            <label className="wide-field"><span>Import an existing profile path</span><input value={profilePath} onChange={(event) => setProfilePath(event.target.value)} placeholder="C:\\Users\\You\\Kestrel Research\\setup-profiles\\kestrel-profile.json"/></label>
            {profileStatus && <div className="profile-status" role="status">{profileStatus}</div>}
            <div className="portable-file-buttons">
              <button className="quiet-button" disabled={!!busy} onClick={() => void refreshProfileText()}>{busy === "refresh-profile" ? <LoaderCircle className="spin" size={15}/> : <RefreshCw size={15}/>} Refresh text from app</button>
              <button className="quiet-button" disabled={!!busy || !profilePath.trim()} onClick={() => void importProfilePath()}><Upload size={15}/> Import file</button>
            </div>
          </aside>
        </div>
        <div className="settings-actions"><button className="quiet-button" disabled={!!busy || !profileText.trim()} onClick={() => void importProfileText()}>{busy === "import" ? <LoaderCircle className="spin" size={15}/> : <Check size={15}/>} Validate & apply edited text</button><span/><button className="primary-button" disabled={!!busy || !profileText.trim()} onClick={() => void exportProfile()}>{busy === "export" ? <LoaderCircle className="spin" size={15}/> : <Download size={15}/>} Export edited JSON</button></div>
      </section>}
      </div>
    </div>
  );
}

function NumberSetting({ label, hint, value, disabled, onChange }: { label: string; hint: string; value: number; disabled: boolean; onChange: (value: string) => void }) {
  return <label className="number-setting"><span>{label}<small>{hint}</small></span><input type="number" step="1" value={value} disabled={disabled} onChange={(event) => onChange(event.target.value)} /></label>;
}

function TokenDropdownSetting({
  label,
  hint,
  value,
  options,
  recommendedValue,
  recommendedLabel,
  disabled,
  allowCustom,
  onChange,
}: {
  label: string;
  hint: string;
  value: number;
  options: Array<{ value: number; label: string }>;
  recommendedValue?: number;
  recommendedLabel?: string;
  disabled: boolean;
  allowCustom?: boolean;
  onChange: (value: string) => void;
}) {
  const isKnown = options.some((opt) => opt.value === value);
  const [customMode, setCustomMode] = useState(() => !isKnown);

  return (
    <label className="number-setting token-tier-setting">
      <span>
        {label}
        <small>{hint}</small>
      </span>
      {!customMode ? (
        <select
          value={isKnown ? value : "custom"}
          disabled={disabled}
          onChange={(event) => {
            if (event.target.value === "custom") {
              setCustomMode(true);
            } else {
              onChange(event.target.value);
            }
          }}
        >
          {options.map((opt) => {
            const isRec = recommendedValue === opt.value;
            const text = isRec
              ? `${opt.label} · Recommended ${recommendedLabel ? `(${recommendedLabel})` : ""}`
              : opt.label;
            return (
              <option key={opt.value} value={opt.value}>
                {text}
              </option>
            );
          })}
          {allowCustom && <option value="custom">Custom...</option>}
        </select>
      ) : (
        <div style={{ display: "flex", gap: 4, alignItems: "center" }}>
          <input
            type="number"
            step="1024"
            value={value}
            disabled={disabled}
            onChange={(event) => onChange(event.target.value)}
            style={{ flex: 1 }}
          />
          <button
            type="button"
            className="quiet-button"
            style={{ padding: "0 6px", minHeight: 28, fontSize: 10 }}
            onClick={() => setCustomMode(false)}
            title="Switch back to presets"
          >
            Presets
          </button>
        </div>
      )}
    </label>
  );
}

function formatMib(value: number): string {
  if (!value) return "—";
  return value >= 1024 ? `${(value / 1024).toFixed(1)} GiB` : `${value.toLocaleString()} MiB`;
}

function NewResearchDialog({ advancedEnabled, onClose, onSubmit }: { advancedEnabled: boolean; onClose: () => void; onSubmit: (query: string, depth: "focused" | "thorough" | "expedition") => Promise<void> }) {
  const [query, setQuery] = useState("");
  const [depth, setDepth] = useState<"focused" | "thorough" | "expedition">(advancedEnabled ? "expedition" : "thorough");
  const submit = () => {
    if (query.trim().length >= 4) void onSubmit(query.trim(), depth);
  };
  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={(event) => event.target === event.currentTarget && onClose()}>
      <section className="research-dialog" role="dialog" aria-modal="true" aria-labelledby="new-research-title">
        <button className="icon-button dialog-close" onClick={onClose} aria-label="Close"><X /></button>
        <div className="dialog-icon"><Feather /></div>
        <span className="eyebrow">New offline inquiry</span>
        <h2 id="new-research-title">What would you like to understand?</h2>
        <p>Kestrel will check your existing library first, then inspect the local Wikipedia archive and publish a new, traceable edition.</p>
        <textarea autoFocus value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={(event) => { if ((event.metaKey || event.ctrlKey) && event.key === "Enter") submit(); }} placeholder="Ask a question, name a topic, or describe what you want compared…" />
        <div className="depth-picker">
          <button className={depth === "focused" ? "selected" : ""} onClick={() => setDepth("focused")}><strong>Focused</strong><span>A concise brief from the most relevant sources</span></button>
          <button className={depth === "thorough" ? "selected" : ""} onClick={() => setDepth("thorough")}><strong>Thorough</strong><span>Broader reading, nuance, gaps, and a timeline</span></button>
          {advancedEnabled && <button className={`expedition-choice ${depth === "expedition" ? "selected" : ""}`} onClick={() => setDepth("expedition")}><strong><Layers3 size={14} /> Solo expedition</strong><span>The selected model's Research profile coordinates many archive lanes and a longer synthesis</span></button>}
        </div>
        <div className="dialog-assurance"><ShieldCheck size={16} /><span>No web requests. Model, archive, research, and HTML stay on this computer.</span></div>
        <div className="dialog-actions"><span><kbd>Ctrl</kbd> + <kbd>Enter</kbd></span><button className="primary-button" disabled={query.trim().length < 4} onClick={submit}>Begin research <ChevronRight size={16} /></button></div>
      </section>
    </div>
  );
}

function ProgressPanel({ progress, activity, onCancel }: { progress: ResearchProgress; activity: ResearchProgress[]; onCancel: () => void }) {
  const activeIndex = Math.max(0, stageOrder.indexOf(progress.stage));
  const percent = Math.min(100, Math.max(4, progress.total ? (progress.current / progress.total) * 100 : ((activeIndex + 0.35) / stageOrder.length) * 100));
  return (
    <div className="progress-drawer" role="status" aria-live="polite">
      <div className="progress-header"><div className="progress-spinner"><LoaderCircle className="spin" /></div><div><span className="eyebrow">Local model is researching</span><h2>{progress.title}</h2></div><button className="icon-button" aria-label="Stop research" onClick={onCancel}><CircleStop /></button></div>
      <p className="progress-detail">{progress.detail}</p>
      <div className="progress-track"><span style={{ width: `${percent}%` }} /></div>
      <div className="stage-row">{stageOrder.map((stage, index) => <div className={index < activeIndex ? "done" : index === activeIndex ? "active" : ""} key={stage}><span>{index < activeIndex ? <Check size={12} /> : index + 1}</span><small>{stageNames[stage]}</small></div>)}</div>
      {!!activity.length && <div className="activity-log">{activity.slice(-3).reverse().map((item) => <div key={`${item.stage}-${item.detail}`}><span className={`activity-dot ${item.stage}`} /><span>{item.detail}</span></div>)}</div>}
      <div className="progress-footer"><span><Clock3 size={14} /> {progress.elapsedSeconds ? `${progress.elapsedSeconds}s elapsed` : "Starting now"}</span><button onClick={onCancel}>Stop safely</button></div>
    </div>
  );
}

function ErrorBanner({ message, onClose }: { message: string; onClose: () => void }) {
  return <div className="error-banner"><div><strong>Kestrel needs attention</strong><span>{message}</span></div><button className="icon-button" onClick={onClose}><X /></button></div>;
}

function ReaderSkeleton() {
  return <div className="reader-skeleton"><div className="skeleton short" /><div className="skeleton title" /><div className="skeleton title second" /><div className="skeleton paragraph" /><div className="skeleton card" /></div>;
}

function EmptyLibrary({ onNew }: { onNew: () => void }) {
  return (
    <section className="empty-library">
      <div className="empty-orbit"><Feather /></div>
      <span className="eyebrow">Your private knowledge base starts here</span>
      <h1>Research that stays useful.</h1>
      <p>Ask a question and Kestrel will inspect your local Wikipedia, explain the answer clearly, preserve every source, and save a standalone HTML edition you can reopen years from now.</p>
      <button className="primary-button" onClick={onNew}><Plus size={16} /> Begin your first research</button>
      <div className="empty-assurances"><span><Search size={14} /> Finds related work first</span><span><History size={14} /> Never overwrites an edition</span><span><ShieldCheck size={14} /> No network access</span></div>
    </section>
  );
}

function formatDate(value: string): string {
  return new Intl.DateTimeFormat(undefined, { year: "numeric", month: "short", day: "numeric" }).format(new Date(value));
}

export default App;
