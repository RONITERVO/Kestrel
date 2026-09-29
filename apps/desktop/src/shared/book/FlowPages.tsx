import { ChevronLeft, ChevronRight } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import "./paging.css";

/**
 * Blocks whose continuation onto the next page is worth a cue: messages, events and cards
 * (articles), whole replies and reasoning, and their text.
 */
const CONTINUING_BLOCKS = "article, .markdown-content, .text-paragraphs, p, li, pre, blockquote";

/** Below this page height, keeping every paragraph whole wastes most of the page. */
const SHORT_PAGE = 320;

/**
 * How far an element sits from the start of the columns, in layout pixels. Offsets ignore CSS
 * transforms, so this stays exact while the book lies on the desk under a 3D projection, where
 * getBoundingClientRect reports the tilted on-screen box and would name the wrong page.
 */
export function layoutOffset(element: Element, columns: HTMLElement): number {
  const fromPage = (node: Element) => {
    let left = 0;
    for (let current = node as HTMLElement | null; current; current = current.offsetParent as HTMLElement | null) {
      left += current.offsetLeft;
    }
    return left;
  };
  return fromPage(element) - fromPage(columns);
}

export type FlowPagesController = {
  /** Turns to the page that holds this element (for contents links, citations and narration). */
  showElement: (element: Element) => void;
  turnTo: (page: number) => void;
};

/**
 * Flows long content into book pages instead of a scrollbar. Content is laid out in fixed-height
 * columns, one column per page; turning a page moves by exactly one column. Everything stays in
 * the DOM, so search, focus and anchors keep working: focusing or scrolling to something on
 * another page turns to that page. `follow="end"` keeps the newest page open while content grows.
 * `fit` sizes the pages to their content up to a cap (`--flow-max`), for a card that pages only
 * when its text is longer than the room it has.
 */
export function FlowPages({
  children,
  label,
  resetKey,
  follow = "start",
  className = "",
  gap = 48,
  onPages,
  controller,
  anchors,
  onAnchorPages,
  footer,
  fit = false,
}: {
  children: ReactNode;
  label: string;
  /** A new value (for example, a different report) returns to the first or last page. */
  resetKey?: string | number;
  follow?: "start" | "end";
  className?: string;
  gap?: number;
  onPages?: (state: { page: number; pages: number }) => void;
  controller?: RefObject<FlowPagesController | null>;
  /** Element ids whose page numbers are reported through onAnchorPages, for a contents list. */
  anchors?: readonly string[];
  onAnchorPages?: (pages: Record<string, number>) => void;
  /** Controls that belong to every page of this document, shown above the folio. */
  footer?: ReactNode;
  /** As tall as the content up to `--flow-max`; pages and a folio appear only past that height. */
  fit?: boolean;
}) {
  const viewportRef = useRef<HTMLDivElement>(null);
  const columnsRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(0);
  const [page, setPage] = useState(0);
  const [pages, setPages] = useState(1);
  const [continuing, setContinuing] = useState<ReadonlySet<number>>(() => new Set());
  const [short, setShort] = useState(false);
  const atEnd = useRef(true);
  const widthRef = useRef(0);
  const stride = width + gap;

  const measure = useCallback(() => {
    const viewport = viewportRef.current;
    const columns = columnsRef.current;
    if (!viewport || !columns) return;
    const nextWidth = viewport.clientWidth;
    // A hidden chapter measures zero; it keeps its pages until it is shown again.
    if (!nextWidth) return;
    if (nextWidth !== widthRef.current) {
      // The columns take the new width on the next render; the layout effect counts pages then.
      widthRef.current = nextWidth;
      setWidth(nextWidth);
      return;
    }
    const total = Math.max(1, Math.round((columns.scrollWidth + gap) / (nextWidth + gap)));
    setPages(total);
    // On a short page paragraphs may split between lines (still three lines a side) instead of moving whole.
    setShort(total > 1 && viewport.clientHeight < SHORT_PAGE);
    // A page whose last reply or paragraph carries on gets a "continues" cue in its free foot line.
    // A block's fragments fill consecutive pages from the one it starts on; both the start and the
    // fragment count are layout facts, so the cue stays right while the book is tilted.
    const pageOf = (element: Element) => Math.max(0, Math.floor((layoutOffset(element, columns) + 1) / (nextWidth + gap)));
    const found = new Set<number>();
    for (const block of columns.querySelectorAll(CONTINUING_BLOCKS)) {
      const fragments = block.getClientRects().length;
      if (fragments < 2) continue;
      const first = pageOf(block);
      for (let index = first; index < first + fragments - 1; index += 1) found.add(index);
    }
    setContinuing((current) => (current.size === found.size && [...found].every((index) => current.has(index)) ? current : found));
    if (anchors?.length && onAnchorPages) {
      const found: Record<string, number> = {};
      for (const id of anchors) {
        const element = columns.ownerDocument.getElementById(id);
        if (element && columns.contains(element)) found[id] = pageOf(element);
      }
      onAnchorPages(found);
    }
    setPage((current) => (follow === "end" && atEnd.current ? total - 1 : Math.min(current, total - 1)));
  }, [anchors, follow, gap, onAnchorPages]);

  // The scroll position follows the page. State updaters stay pure: React may replay a queued
  // updater after a newer turn, and a scroll written from inside one would undo that turn.
  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (viewport && width) viewport.scrollLeft = page * stride;
  }, [page, stride, width]);

  useLayoutEffect(() => {
    if (width) measure();
  }, [measure, width]);

  useLayoutEffect(() => {
    measure();
    const viewport = viewportRef.current;
    const columns = columnsRef.current;
    if (!viewport || !columns || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => measure());
    observer.observe(viewport);
    const mutations = new MutationObserver(() => measure());
    // Opening or closing a <details> section changes how many pages the content needs.
    mutations.observe(columns, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ["open"] });
    // Hand-lettered headings reflow the pages once their fonts arrive.
    const fonts = document.fonts;
    fonts?.addEventListener?.("loadingdone", measure);
    void fonts?.ready.then(measure);
    return () => {
      observer.disconnect();
      mutations.disconnect();
      fonts?.removeEventListener?.("loadingdone", measure);
    };
  }, [measure]);

  useEffect(() => {
    atEnd.current = true;
    setPage(follow === "end" ? Math.max(0, pages - 1) : 0);
    // Only a new document resets the reading position; page count changes are handled by measure().
  }, [resetKey]);

  useEffect(() => onPages?.({ page, pages }), [onPages, page, pages]);

  const turn = useCallback((next: number) => {
    const clamped = Math.max(0, Math.min(pages - 1, next));
    atEnd.current = clamped === pages - 1;
    setPage(clamped);
  }, [pages]);

  useEffect(() => {
    if (!controller) return;
    controller.current = {
      turnTo: turn,
      showElement: (element) => {
        const columns = columnsRef.current;
        if (!columns || !stride || !columns.contains(element)) return;
        turn(Math.floor((layoutOffset(element, columns) + 1) / stride));
      },
    };
    return () => {
      controller.current = null;
    };
  }, [controller, stride, turn]);

  // Focus, find-in-page and scrollIntoView move the hidden scroll position; snap it to a page.
  const onScroll = () => {
    const viewport = viewportRef.current;
    if (!viewport || !stride) return;
    const next = Math.round(viewport.scrollLeft / stride);
    if (Math.abs(viewport.scrollLeft - next * stride) > 1) viewport.scrollLeft = next * stride;
    if (next !== page) {
      atEnd.current = next === pages - 1;
      setPage(next);
    }
  };

  return (
    <section
      className={`flow-pages ${fit ? `flow-fit ${pages > 1 ? "is-paged" : ""}` : ""} ${short ? "flow-short" : ""} ${className}`}
      aria-label={label}
      onKeyDown={(event) => {
        const target = event.target as HTMLElement;
        if (target.closest("input, textarea, select, [contenteditable='true']")) return;
        if (event.key === "PageDown" || (event.key === "ArrowRight" && event.altKey)) {
          event.preventDefault();
          turn(page + 1);
        } else if (event.key === "PageUp" || (event.key === "ArrowLeft" && event.altKey)) {
          event.preventDefault();
          turn(page - 1);
        }
      }}
    >
      <div ref={viewportRef} className="flow-viewport" onScroll={onScroll}>
        <div
          ref={columnsRef}
          className="flow-columns"
          style={width ? { columnWidth: `${width}px`, columnGap: `${gap}px`, width: `${width}px` } : undefined}
        >
          {children}
        </div>
      </div>
      {continuing.has(page) && page < pages - 1 && (
        <button type="button" className="flow-continues" aria-label={`Continues on page ${page + 2}`} onClick={() => turn(page + 1)}>
          continues <ChevronRight aria-hidden="true" />
        </button>
      )}
      {footer && <div className="flow-footer">{footer}</div>}
      {(!fit || pages > 1) && <PageTurner page={page} pages={pages} label={label} onTurn={turn} />}
    </section>
  );
}

/** The folio: where you are, how much remains, and the corners that turn. */
export function PageTurner({ page, pages, label, onTurn }: { page: number; pages: number; label: string; onTurn: (page: number) => void }) {
  if (pages <= 1) return <div className="page-turner single" aria-hidden="true"><span className="folio">— 1 —</span></div>;
  return (
    <nav className="page-turner" aria-label={`${label}: page ${page + 1} of ${pages}`}>
      <button type="button" className="page-corner previous" disabled={page === 0} onClick={() => onTurn(page - 1)} aria-label={`Previous page of ${label}`}>
        <ChevronLeft aria-hidden="true" />
      </button>
      <span className="folio" aria-live="polite">page {page + 1} of {pages}</span>
      <button type="button" className="page-corner next" disabled={page >= pages - 1} onClick={() => onTurn(page + 1)} aria-label={`Next page of ${label}`}>
        <ChevronRight aria-hidden="true" />
      </button>
    </nav>
  );
}
