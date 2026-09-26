import {
  Archive,
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
  Layers3,
  Library,
  LoaderCircle,
  Plus,
  Search,
  ShieldCheck,
  Sparkles,
  X,
} from "lucide-react";
import { useCallback, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";
import type { ReportSummary, ResearchProgress, ResearchReport } from "../../contracts/index";
import { FlowPages, type FlowPagesController } from "../../shared/book/FlowPages";
import { PagedList } from "../../shared/book/PagedList";
import { SpokenText, type SpeechProgressState } from "../../shared/components/spokenHighlight";
import { formatDate } from "../../shared/format";
import { ResearchSpeechPlayer } from "./ResearchSpeech";
import "./research.css";

type ProgressStage = ResearchProgress["stage"];

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

type ContentsEntry = { id: string; label: string };

function reportContents(report: ResearchReport): ContentsEntry[] {
  return [
    { id: "report-overview", label: "Overview & short answer" },
    { id: "findings", label: "Key findings" },
    ...report.sections.map((section) => ({ id: section.id, label: section.heading })),
    ...(report.timeline.length ? [{ id: "timeline", label: "Timeline" }] : []),
    { id: "terms", label: "Terms & open questions" },
    { id: "sources", label: "Sources inspected" },
  ];
}

/**
 * The Research chapter. The left page is the library and the contents of the open report,
 * with narration; the right page is the report itself, flowing onto turnable pages.
 */
export function ResearchChapter({
  reports,
  totalReports,
  selectedId,
  report,
  filter,
  root,
  onFilter,
  onSelect,
  onNew,
  onReveal,
  onStandalone,
}: {
  reports: ReportSummary[];
  totalReports: number;
  selectedId: string | null;
  report: ResearchReport | null;
  filter: string;
  root: string;
  onFilter: (value: string) => void;
  onSelect: (id: string) => void;
  onNew: () => void;
  onReveal: () => void;
  onStandalone: (id: string) => void;
}) {
  const controller = useRef<FlowPagesController | null>(null);
  const [anchorPages, setAnchorPages] = useState<Record<string, number>>({});
  const [spokenPassageId, setSpokenPassageId] = useState<string | null>(null);
  const [speechProgress, setSpeechProgress] = useState<SpeechProgressState | null>(null);
  const contents = useMemo(() => (report ? reportContents(report) : []), [report]);
  const anchors = useMemo(() => contents.map((entry) => entry.id), [contents]);

  const showElementById = useCallback((id: string) => {
    const element = document.getElementById(id);
    if (element) controller.current?.showElement(element);
  }, []);
  // Narration turns the page to the passage being read: its paragraph, else its section.
  const handleSpeechPassage = useCallback((anchorId: string | null, passageId?: string | null) => {
    setSpokenPassageId(passageId ?? null);
    const candidates = passageId ? [`speech-target-${passageId}`, `speech-target-${passageId.replace(/-\d+$/, "")}`] : [];
    const target = [...candidates, anchorId].find((id) => id && document.getElementById(id));
    if (target) showElementById(target);
  }, [showElementById]);
  const handleAnchorPages = useCallback((next: Record<string, number>) => {
    setAnchorPages((current) => {
      const keys = Object.keys(next);
      return keys.length === Object.keys(current).length && keys.every((key) => current[key] === next[key]) ? current : next;
    });
  }, []);

  return (
    <div className="research-chapter spread">
      <aside className="page page-left library-sidebar">
        <div className="sidebar-heading">
          <div><span className="eyebrow">Private library</span><h2>Your research</h2></div>
          <button className="primary-button compact" onClick={onNew}><Plus size={16} /> New research</button>
        </div>
        <label className="search-field">
          <Search size={16} />
          <input value={filter} onChange={(event) => onFilter(event.target.value)} placeholder="Find past research" aria-label="Find past research" />
          {filter && <button aria-label="Clear search" onClick={() => onFilter("")}><X size={14} /></button>}
        </label>
        <div className="library-count"><Library size={14} /> {reports.length === totalReports ? `${totalReports.toLocaleString()} reports` : `${reports.length} of ${totalReports} reports`}</div>
        <PagedList
          as="nav"
          className="report-list"
          label="Research library"
          items={reports}
          itemKey={(item) => item.id}
          selectedKey={selectedId}
          empty={<div className="empty-list"><Search size={20} /><span>No matching research</span></div>}
          renderItem={(item) => (
            <button className={`report-list-item ${item.id === selectedId ? "selected" : ""}`} aria-current={item.id === selectedId ? "true" : undefined} onClick={() => onSelect(item.id)}>
              <span className="report-item-title">{item.title}</span>
              <span className="report-item-dek">{item.dek}</span>
              <span className="report-item-meta"><span>Edition {item.edition}</span><span>{item.sourceCount} sources</span><span>{item.readingMinutes} min</span></span>
            </button>
          )}
        />
        {report && (
          <section className="report-contents" aria-label="In this report">
            <span className="eyebrow">In this report</span>
            <PagedList
              className="report-contents-list"
              label="In this report"
              items={contents}
              itemKey={(entry) => entry.id}
              renderItem={(entry) => (
                <button type="button" onClick={() => showElementById(entry.id)}>
                  <span>{entry.label}</span>
                  <i aria-hidden="true" />
                  <b>{anchorPages[entry.id] === undefined ? "" : anchorPages[entry.id] + 1}</b>
                </button>
              )}
            />
          </section>
        )}
        <div className="library-footer">
          {report && <div className="context-card"><Archive size={16} /><span><strong>{report.model}</strong><small>{report.archiveSnapshot} · edition {report.edition}, never overwritten{report.researchProfile === "solo-expedition" ? ` · ${report.researchLanes} lanes · ${report.contextWindow.toLocaleString()} context` : ""}</small></span></div>}
          <button className="library-root" onClick={onReveal} title={root}>
            <FolderOpen size={16} />
            <span><strong>Research files</strong><small>{root}</small></span>
            <ChevronRight size={15} />
          </button>
        </div>
      </aside>
      <main className="page page-right reader-page">
        {!selectedId && totalReports === 0 ? (
          <EmptyLibrary onNew={onNew} />
        ) : !report ? (
          <ReaderSkeleton />
        ) : (
          <ResearchReader
            report={report}
            controller={controller}
            anchors={anchors}
            onAnchorPages={handleAnchorPages}
            spokenPassageId={spokenPassageId}
            speechProgress={speechProgress}
            onStandalone={() => onStandalone(report.id)}
            narration={<ResearchSpeechPlayer report={report} onPassageChange={handleSpeechPassage} onSpeechProgress={setSpeechProgress} />}
          />
        )}
      </main>
    </div>
  );
}

function ResearchReader({
  report,
  controller,
  anchors,
  onAnchorPages,
  spokenPassageId,
  speechProgress,
  onStandalone,
  narration,
}: {
  report: ResearchReport;
  controller: RefObject<FlowPagesController | null>;
  anchors: readonly string[];
  onAnchorPages: (pages: Record<string, number>) => void;
  spokenPassageId: string | null;
  speechProgress: SpeechProgressState | null;
  onStandalone: () => void;
  narration: ReactNode;
}) {
  const [sourceFocus, setSourceFocus] = useState<string | null>(null);
  const sourceMap = useMemo(() => new Map(report.sources.map((source) => [source.id, source])), [report.sources]);
  const focusSource = (id: string) => {
    setSourceFocus(id);
    const element = document.getElementById(`source-${id}`);
    if (element) controller.current?.showElement(element);
  };
  const spoken = (prefix: string) => (spokenPassageId?.startsWith(prefix) ? "speech-passage-active" : "");
  return (
    <FlowPages
      className="research-article"
      label="Report pages"
      resetKey={report.id}
      controller={controller}
      anchors={anchors}
      onAnchorPages={onAnchorPages}
      footer={narration}
    >
      <header className="report-header" id="report-overview">
        <div className="report-kicker"><span>Research brief</span><span>Edition {report.edition}</span><span>{formatDate(report.updatedAt)}</span></div>
        <h1>{report.title}</h1>
        <p className={`report-dek ${spoken("overview")}`} id="speech-target-overview">
          <SpokenText text={report.dek} passageId="overview" progress={speechProgress} />
        </p>
        <div className="report-byline">
          <span><Clock3 size={15} /> {report.readingMinutes} min read</span>
          <span><BookOpen size={15} /> {report.sources.length} inspected sources</span>
          <span><FileText size={15} /> {report.wordCount.toLocaleString()} words</span>
        </div>
      </header>

      <section className="answer-card" id="short-answer" aria-labelledby="short-answer-title">
        <div className="section-label" id="short-answer-title"><Sparkles size={16} /> Short answer</div>
        <p className={spoken("short-answer")} id="speech-target-short-answer">
          <SpokenText text={report.answer} passageId="short-answer" progress={speechProgress} />
        </p>
      </section>

      {report.edition > 1 && (
        <aside className="improvement-note" id="edition-improvement">
          <div className="improvement-icon"><History size={17} /></div>
          <div>
            <strong>What changed in this edition</strong>
            <p className={spoken("edition")} id="speech-target-edition">
              <SpokenText text={report.improvement} passageId="edition" progress={speechProgress} />
            </p>
          </div>
          <span className="edition-badge">v{report.edition}</span>
        </aside>
      )}

      <div className="section-heading" id="findings"><span className="section-number">01</span><div><span className="eyebrow">The evidence at a glance</span><h2>Key findings</h2></div></div>
      {report.findings.map((finding, index) => (
        <div className={`finding-card ${spoken(`finding-${index + 1}`)}`} id={`speech-target-finding-${index + 1}`} key={finding.title}>
          <span className="finding-number">{String(index + 1).padStart(2, "0")}</span>
          <h3>{finding.title}</h3>
          <p><SpokenText text={finding.explanation} passageId={`finding-${index + 1}`} progress={speechProgress} /></p>
          <CitationRow ids={finding.citations} onFocus={focusSource} />
        </div>
      ))}

      {report.sections.map((section, index) => (
        <section className="content-section narrative-section" key={section.id}>
          <div className="section-heading" id={section.id}><span className="section-number">{String(index + 2).padStart(2, "0")}</span><div><span className="eyebrow">Deep dive</span><h2>{section.heading}</h2></div></div>
          <p className={`section-summary ${spoken(`section-${index + 1}-summary`)}`} id={`speech-target-section-${index + 1}-summary`}>
            <SpokenText text={section.summary} passageId={`section-${index + 1}-summary`} progress={speechProgress} />
          </p>
          {section.body.map((paragraph, paragraphIndex) => (
            <p className={spoken(`section-${index + 1}-paragraph-${paragraphIndex + 1}`)} id={`speech-target-section-${index + 1}-paragraph-${paragraphIndex + 1}`} key={paragraphIndex}>
              <SpokenText text={paragraph} passageId={`section-${index + 1}-paragraph-${paragraphIndex + 1}`} progress={speechProgress} />
            </p>
          ))}
          <CitationRow ids={section.citations} onFocus={focusSource} labels={sourceMap} />
        </section>
      ))}

      {!!report.timeline.length && (
        <>
          <div className="section-heading" id="timeline"><span className="section-number">{String(report.sections.length + 2).padStart(2, "0")}</span><div><span className="eyebrow">Sequence</span><h2>Timeline</h2></div></div>
          {report.timeline.map((item, index) => (
            <div className={`timeline-item ${spoken(`timeline-${index + 1}`)}`} id={`speech-target-timeline-${index + 1}`} key={`${item.date}-${item.label}`}>
              <div className="timeline-date">{item.date}</div><div className="timeline-marker" /><div><h3>{item.label}</h3><p><SpokenText text={item.description} passageId={`timeline-${index + 1}`} progress={speechProgress} /></p><CitationRow ids={item.citations} onFocus={focusSource} /></div>
            </div>
          ))}
        </>
      )}

      <div className="section-heading small" id="terms"><div><span className="eyebrow">Plain language</span><h2>Terms worth knowing</h2></div></div>
      <dl className="term-list">
        {report.terms.map((term, index) => (
          <div className={spoken(`term-${index + 1}`)} id={`speech-target-term-${index + 1}`} key={term.term}>
            <dt>{term.term}</dt>
            <dd><SpokenText text={term.meaning} passageId={`term-${index + 1}`} progress={speechProgress} /></dd>
          </div>
        ))}
      </dl>
      <div className="section-heading small"><div><span className="eyebrow">Research frontier</span><h2>What remains open</h2></div></div>
      <ol className="question-list">
        {report.openQuestions.map((question, index) => (
          <li className={spoken(`question-${index + 1}`)} id={`speech-target-question-${index + 1}`} key={question}>
            <SpokenText text={question} passageId={`question-${index + 1}`} progress={speechProgress} />
          </li>
        ))}
      </ol>

      <div className="section-heading" id="sources"><span className="section-number">{String(report.sections.length + 3).padStart(2, "0")}</span><div><span className="eyebrow">Evidence ledger</span><h2>Sources inspected</h2></div></div>
      <p className="sources-intro">Every source below was opened by the local model. Excerpts show the evidence it received; Wikipedia is a tertiary starting point, not a substitute for primary sources.</p>
      {report.sources.map((source, index) => (
        <div id={`source-${source.id}`} className={`source-card ${sourceFocus === source.id ? "focused" : ""} ${spoken(`source-${index + 1}`)}`} key={source.id}>
          <span className="source-id">{source.id}</span>
          <div>
            <div className="source-title-row"><h3>{source.title}</h3><span>{source.kind === "wikipedia" ? "Wikipedia" : "Kestrel research"}</span></div>
            <p className="source-location">{source.section ?? "Full article"} · snapshot {source.snapshot ?? report.archiveSnapshot}</p>
            <blockquote id={`speech-target-source-${index + 1}`}>
              <SpokenText text={source.excerpt} passageId={`source-${index + 1}`} progress={speechProgress} />
            </blockquote>
          </div>
        </div>
      ))}

      <footer className="report-footer">
        <div><ShieldCheck size={16} /><span>Produced entirely on this computer with {report.model} and {report.archiveSnapshot}.</span></div>
        <button className="quiet-button compact" onClick={onStandalone}><ExternalLink size={15} /> Open standalone HTML</button>
      </footer>
    </FlowPages>
  );
}

function CitationRow({ ids, onFocus, labels }: { ids: string[]; onFocus: (id: string) => void; labels?: Map<string, { title: string }> }) {
  return <div className="citation-row" aria-label="Citations">{ids.map((id) => <button key={id} onClick={() => onFocus(id)} title={labels?.get(id)?.title ?? `Source ${id}`}>{id}</button>)}</div>;
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

export function NewResearchDialog({ advancedEnabled, onClose, onSubmit }: { advancedEnabled: boolean; onClose: () => void; onSubmit: (query: string, depth: "focused" | "thorough" | "expedition") => Promise<void> }) {
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

export function ResearchProgressPanel({ progress, activity, onCancel }: { progress: ResearchProgress; activity: ResearchProgress[]; onCancel: () => void }) {
  const activeIndex = Math.max(0, stageOrder.indexOf(progress.stage));
  const percent = Math.min(100, Math.max(4, progress.total ? (progress.current / progress.total) * 100 : ((activeIndex + 0.35) / stageOrder.length) * 100));
  return (
    <div className="progress-drawer" role="status" aria-live="polite">
      <div className="progress-header"><div className="progress-spinner"><LoaderCircle className="spin" /></div><div><span className="eyebrow">Local model is researching</span><h2>{progress.title}</h2></div><button className="icon-button" aria-label="Stop research" onClick={onCancel}><CircleStop /></button></div>
      <p className="progress-detail">{progress.detail}</p>
      <div className="progress-track"><span style={{ width: `${percent}%` }} /></div>
      <div className="stage-row">{stageOrder.map((stage, index) => <div className={index < activeIndex ? "done" : index === activeIndex ? "active" : ""} key={stage}><span>{index < activeIndex ? <Check size={12} /> : index + 1}</span><small>{stageNames[stage]}</small></div>)}</div>
      {!!activity.length && <div className="activity-log">{activity.slice(-3).reverse().map((item) => <div key={`${item.stage}-${item.detail}`}><span className={`activity-dot ${item.stage}`} /><span>{item.detail}</span></div>)}</div>}
      <div className="progress-footer"><span><Clock3 size={14} /> {progress.elapsedSeconds ? `${progress.elapsedSeconds}s elapsed` : "Starting now"}</span><button className="quiet-button compact" onClick={onCancel}>Stop safely</button></div>
    </div>
  );
}
