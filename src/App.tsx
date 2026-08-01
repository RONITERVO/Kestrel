import {
  Archive,
  ArrowLeft,
  BookOpen,
  Check,
  ChevronRight,
  CircleStop,
  Clock3,
  ExternalLink,
  Feather,
  FileText,
  FolderOpen,
  History,
  Library,
  LoaderCircle,
  Menu,
  Plus,
  Search,
  ShieldCheck,
  Sparkles,
  X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  bootstrap,
  cancelResearch,
  getReport,
  onProgress,
  openStandalone,
  prepareServices,
  revealLibrary,
  runResearch,
} from "./api";
import type {
  AppSnapshot,
  ProgressStage,
  ReportSummary,
  ResearchProgress,
  ResearchReport,
  ServiceState,
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

  const refresh = useCallback(async () => {
    try {
      const next = await bootstrap();
      setSnapshot(next);
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

  const handleResearch = async (query: string, depth: "focused" | "thorough") => {
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
    <div className="app-shell">
      <AppHeader
        status={snapshot.status}
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
      <div className="workspace">
        <LibrarySidebar
          open={sidebarOpen}
          reports={visibleReports}
          selectedId={selectedId}
          filter={filter}
          root={snapshot.libraryRoot}
          onFilter={setFilter}
          onSelect={chooseReport}
          onNew={() => setNewResearchOpen(true)}
          onReveal={() => void revealLibrary()}
        />
        <main className="main-stage">
          {error && <ErrorBanner message={error} onClose={() => setError(null)} />}
          {!selectedId && snapshot.reports.length === 0 ? (
            <EmptyLibrary onNew={() => setNewResearchOpen(true)} />
          ) : !report ? (
            <ReaderSkeleton />
          ) : (
            <ResearchReader report={report} onStandalone={() => void openStandalone(report.id)} />
          )}
        </main>
      </div>
      {newResearchOpen && <NewResearchDialog onClose={() => setNewResearchOpen(false)} onSubmit={handleResearch} />}
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
  onMenu,
  onNew,
  onPrepare,
}: {
  status: AppSnapshot["status"];
  onMenu: () => void;
  onNew: () => void;
  onPrepare: () => void;
}) {
  const allReady = status.bonsai === "ready" && status.wikipedia === "ready";
  return (
    <header className="app-header">
      <div className="header-left">
        <button className="icon-button menu-button" aria-label="Toggle library" onClick={onMenu}><Menu /></button>
        <div className="brand-mark"><Feather size={19} /></div>
        <div className="brand-copy"><strong>Kestrel</strong><span>Research</span></div>
      </div>
      <div className="header-status" role="status">
        <StatusPill state={status.wikipedia} label={status.archive} />
        <StatusPill state={status.bonsai} label={status.model} />
        <div className="privacy-pill"><ShieldCheck size={14} /> Offline only</div>
      </div>
      <div className="header-actions">
        {!allReady && <button className="quiet-button" onClick={onPrepare}>Prepare services</button>}
        <button className="primary-button compact" onClick={onNew}><Plus size={16} /> New research</button>
      </div>
    </header>
  );
}

function StatusPill({ state, label }: { state: ServiceState; label: string }) {
  return <div className={`status-pill status-${state}`}><span className="status-dot" />{label}</div>;
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
  const sourceMap = useMemo(() => new Map(report.sources.map((source) => [source.id, source])), [report.sources]);
  const focusSource = (id: string) => {
    setSourceFocus(id);
    document.getElementById(`source-${id}`)?.scrollIntoView({ behavior: "smooth", block: "center" });
  };
  return (
    <div className="reader-layout">
      <article className="research-article">
        <nav className="reader-breadcrumb"><button><ArrowLeft size={15} /> Library</button><span>/</span><span>{report.title}</span></nav>
        <header className="report-header">
          <div className="report-kicker"><span>Research brief</span><span>Edition {report.edition}</span><span>{formatDate(report.updatedAt)}</span></div>
          <h1>{report.title}</h1>
          <p className="report-dek">{report.dek}</p>
          <div className="report-byline">
            <span><Clock3 size={15} /> {report.readingMinutes} min read</span>
            <span><BookOpen size={15} /> {report.sources.length} inspected sources</span>
            <span><FileText size={15} /> {report.wordCount.toLocaleString()} words</span>
          </div>
        </header>

        <section className="answer-card" aria-labelledby="short-answer-title">
          <div className="section-label" id="short-answer-title"><Sparkles size={16} /> Short answer</div>
          <p>{report.answer}</p>
        </section>

        {report.edition > 1 && (
          <aside className="improvement-note">
            <div className="improvement-icon"><History size={17} /></div>
            <div><strong>What changed in this edition</strong><p>{report.improvement}</p></div>
            <span className="edition-badge">v{report.edition}</span>
          </aside>
        )}

        <section className="content-section" id="findings">
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
          <section className="content-section narrative-section" id={section.id} key={section.id}>
            <div className="section-heading"><span className="section-number">{String(index + 2).padStart(2, "0")}</span><div><span className="eyebrow">Deep dive</span><h2>{section.heading}</h2></div></div>
            <p className="section-summary">{section.summary}</p>
            {section.body.map((paragraph, paragraphIndex) => <p key={paragraphIndex}>{paragraph}</p>)}
            <CitationRow ids={section.citations} onFocus={focusSource} labels={sourceMap} />
          </section>
        ))}

        {!!report.timeline.length && (
          <section className="content-section" id="timeline">
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

        <section className="content-section split-section" id="terms">
          <div>
            <div className="section-heading small"><div><span className="eyebrow">Plain language</span><h2>Terms worth knowing</h2></div></div>
            <dl className="term-list">{report.terms.map((term) => <div key={term.term}><dt>{term.term}</dt><dd>{term.meaning}</dd></div>)}</dl>
          </div>
          <div>
            <div className="section-heading small"><div><span className="eyebrow">Research frontier</span><h2>What remains open</h2></div></div>
            <ol className="question-list">{report.openQuestions.map((question) => <li key={question}>{question}</li>)}</ol>
          </div>
        </section>

        <section className="content-section sources-section" id="sources">
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
        <div className="rail-card context-card"><Archive size={18} /><strong>Research context</strong><span>{report.model}</span><span>{report.archiveSnapshot}</span><span>Edition {report.edition}, never overwritten</span></div>
      </aside>
    </div>
  );
}

function CitationRow({ ids, onFocus, labels }: { ids: string[]; onFocus: (id: string) => void; labels?: Map<string, { title: string }> }) {
  return <div className="citation-row" aria-label="Citations">{ids.map((id) => <button key={id} onClick={() => onFocus(id)} title={labels?.get(id)?.title ?? `Source ${id}`}>{id}</button>)}</div>;
}

function NewResearchDialog({ onClose, onSubmit }: { onClose: () => void; onSubmit: (query: string, depth: "focused" | "thorough") => Promise<void> }) {
  const [query, setQuery] = useState("");
  const [depth, setDepth] = useState<"focused" | "thorough">("thorough");
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
      <div className="progress-header"><div className="progress-spinner"><LoaderCircle className="spin" /></div><div><span className="eyebrow">Bonsai is researching</span><h2>{progress.title}</h2></div><button className="icon-button" aria-label="Stop research" onClick={onCancel}><CircleStop /></button></div>
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
