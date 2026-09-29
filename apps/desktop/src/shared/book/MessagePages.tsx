import { ChevronLeft, ChevronRight } from "lucide-react";
import {
  Children, createContext, isValidElement, useCallback, useContext, useEffect, useLayoutEffect, useRef, useState,
  type CSSProperties, type KeyboardEvent, type ReactNode, type RefObject,
} from "react";
import { FlowPages, type FlowPagesController } from "./FlowPages";
import "./paging.css";

/** The height a card may take on its page. A card taller than this pages its own text. */
const MessageRoom = createContext(0);

const TURNER_HEIGHT = 44;
/** A card that keeps with the next one (a question before its reply) takes at most this share of a page. */
const LEAD_SHARE = 0.45;

/**
 * A conversation in chat order: whole messages a page at a time, packed from the newest message
 * backwards so the latest page is the fullest. A message longer than a page is capped at the page
 * and turns its own pages inside its card (see CardPages), so the order of the chat never breaks.
 * A card marked `data-keep-with-next` (a question) always shares its page with the card after it
 * (its reply), which is capped to the room left below it. Opens on the newest messages and stays
 * there while new ones arrive.
 */
export function MessagePages({
  children,
  label,
  unit = "messages",
  resetKey,
  className = "",
  live = false,
}: {
  children: ReactNode;
  label: string;
  /** What one entry is called in the folio, such as "messages" or "steps". */
  unit?: string;
  /** A new value (for example, another conversation) returns to the newest messages. */
  resetKey?: string | number;
  className?: string;
  /** Announce new entries politely, for a conversation or task that is still being written. */
  live?: boolean;
}) {
  const items = Children.toArray(children).filter(isValidElement);
  const frameRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [caps, setCaps] = useState<readonly number[]>([]);
  const [groups, setGroups] = useState<ReadonlyArray<readonly [number, number]>>([]);
  // null follows the newest group; a number is a page the reader turned to.
  const [page, setPage] = useState<number | null>(null);
  const count = items.length;
  // Wrappers are keyed by their message; a new key (a live reply saved, another conversation) is a new element to observe.
  const keys = items.map((child) => String(child.key)).join("\u0000");
  const placement = useRef<{ page: number | null; groups: ReadonlyArray<readonly [number, number]> }>({ page: null, groups: [] });

  // Show the current group at the top of the window; the others sit outside it, hidden and inert.
  // Cards resize without this component rendering, so every measure places the window again.
  const place = useCallback(() => {
    const list = listRef.current;
    if (!list) return;
    const { page: chosen, groups: laidOut } = placement.current;
    const index = laidOut.length ? Math.min(chosen ?? laidOut.length - 1, laidOut.length - 1) : 0;
    const first = laidOut[index] ? (list.children[laidOut[index][0]] as HTMLElement | undefined) : undefined;
    list.style.transform = first ? `translateY(${-first.offsetTop}px)` : "";
  }, []);

  const measure = useCallback(() => {
    const frame = frameRef.current;
    const list = listRef.current;
    if (!frame || !list) return;
    const height = frame.clientHeight;
    // A hidden chapter measures zero; it keeps its pages until it is shown again.
    if (!height) return;
    const room = Math.max(120, height - TURNER_HEIGHT);
    const gap = Number.parseFloat(getComputedStyle(list).rowGap) || 0;
    const nodes = [...list.children] as HTMLElement[];
    const heights = nodes.map((node) => node.offsetHeight);
    const keeps = nodes.map((node) => Boolean(node.firstElementChild?.hasAttribute("data-keep-with-next")));
    const nextCaps = nodes.map((_, index) => {
      if (keeps[index] && index < nodes.length - 1) return Math.floor(room * LEAD_SHARE);
      if (index > 0 && keeps[index - 1]) return Math.max(120, Math.floor(room - heights[index - 1] - gap));
      return room;
    });
    setCaps((current) => (current.length === nextCaps.length && current.every((cap, index) => Math.abs(cap - nextCaps[index]) < 1) ? current : nextCaps));
    // Units never split across pages: a card that keeps with the next one travels with it.
    const unitStart = (end: number) => (end > 0 && keeps[end - 1] ? end - 1 : end);
    const unitHeight = (start: number, end: number) => heights.slice(start, end + 1).reduce((sum, value) => sum + value, 0) + gap * (end - start);
    const next: Array<readonly [number, number]> = [];
    for (let end = heights.length - 1; end >= 0;) {
      let start = unitStart(end);
      let total = unitHeight(start, end);
      while (start > 0) {
        const previous = unitStart(start - 1);
        const added = unitHeight(previous, start - 1);
        if (total + gap + added > room) break;
        total += gap + added;
        start = previous;
      }
      next.unshift([start, end]);
      end = start - 1;
    }
    setGroups((current) => (current.length === next.length && current.every(([a, b], index) => a === next[index][0] && b === next[index][1]) ? current : next));
    place();
  }, [place]);

  useLayoutEffect(() => {
    measure();
    const frame = frameRef.current;
    const list = listRef.current;
    if (!frame || !list || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => measure());
    observer.observe(frame);
    for (const node of list.children) observer.observe(node);
    return () => observer.disconnect();
  }, [measure, keys]);

  useEffect(() => setPage(null), [resetKey]);

  const current = groups.length ? Math.min(page ?? groups.length - 1, groups.length - 1) : 0;
  const group = groups[current];

  useLayoutEffect(() => {
    placement.current = { page, groups };
    place();
  }, [page, groups, place]);

  const span = !group ? "" : group[0] === group[1]
    ? `${unit.replace(/s$/, "")} ${group[0] + 1} of ${count}`
    : `${unit} ${group[0] + 1}–${group[1] + 1} of ${count}`;

  const turn = (next: number) => {
    const clamped = Math.max(0, Math.min(groups.length - 1, next));
    setPage(clamped === groups.length - 1 ? null : clamped);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.defaultPrevented || (event.target as HTMLElement).closest("input, textarea, select, [contenteditable='true']")) return;
    if (event.key === "PageDown") { event.preventDefault(); turn(current + 1); }
    if (event.key === "PageUp") { event.preventDefault(); turn(current - 1); }
  };

  return (
    <div ref={frameRef} className={`message-pages ${className}`} onKeyDown={onKeyDown} aria-label={label} role="region">
      <div className="message-pages-window">
        <div ref={listRef} className="message-pages-list" aria-live={live ? "polite" : undefined}>
          {items.map((child, index) => {
            const shown = !group || (index >= group[0] && index <= group[1]);
            return (
              <MessageRoom.Provider key={child.key ?? index} value={caps[index] ?? 0}>
                <div className="message-pages-item" data-shown={shown} inert={!shown} aria-hidden={shown ? undefined : true}>{child}</div>
              </MessageRoom.Provider>
            );
          })}
        </div>
      </div>
      {groups.length > 1 && group ? (
        <nav className="page-turner" aria-label={`${label}: ${span}`}>
          <button type="button" className="page-corner previous" disabled={current === 0} onClick={() => turn(current - 1)} aria-label={`Earlier ${unit} in ${label}`}>
            <ChevronLeft aria-hidden="true" />
          </button>
          <span className="folio" aria-live="polite">{span}</span>
          <button type="button" className="page-corner next" disabled={current >= groups.length - 1} onClick={() => turn(current + 1)} aria-label={`Later ${unit} in ${label}`}>
            <ChevronRight aria-hidden="true" />
          </button>
        </nav>
      ) : <div className="page-turner single" aria-hidden="true"><span className="folio">— 1 —</span></div>}
    </div>
  );
}

/**
 * The text of one card in a MessagePages conversation. It is as tall as its content until the card
 * would outgrow a page; then it pages inside the card with its own folio.
 */
export function CardPages({
  children,
  label,
  follow = "start",
  resetKey,
  controller,
  className = "",
}: {
  children: ReactNode;
  label: string;
  follow?: "start" | "end";
  resetKey?: string | number;
  controller?: RefObject<FlowPagesController | null>;
  className?: string;
}) {
  const room = useContext(MessageRoom);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [max, setMax] = useState(0);

  useLayoutEffect(() => {
    const body = bodyRef.current;
    const card = body?.closest<HTMLElement>(".message-pages-item");
    if (!body || !card || !room || typeof ResizeObserver === "undefined") return;
    // The card's header and buttons keep their size; the text gets the rest of the page.
    const fit = () => {
      const chrome = card.offsetHeight - body.offsetHeight;
      const next = Math.max(96, Math.floor(room - chrome));
      setMax((current) => (Math.abs(current - next) < 1 ? current : next));
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(card);
    return () => observer.disconnect();
  }, [room]);

  return (
    <div ref={bodyRef} className={`card-pages ${className}`} style={max ? ({ "--flow-max": `${max}px` } as CSSProperties) : undefined}>
      <FlowPages fit label={label} follow={follow} resetKey={resetKey} controller={controller}>{children}</FlowPages>
    </div>
  );
}

export type ReplyView = "answer" | "reasoning" | "request" | "edited";

/**
 * Answer, reasoning, the producer's edit and (for advanced receipts) the exact model request are
 * views of one reply card, so none of them pushes the others away and each turns its own pages.
 */
export function ReplyViewToggle({ view, onView, live = false, answerLabel = "Answer", reasoning = true, request = false, edited = false }: {
  view: ReplyView;
  onView: (view: ReplyView) => void;
  live?: boolean;
  answerLabel?: string;
  /** Offer the reasoning view. */
  reasoning?: boolean;
  /** Offer the exact model request view. */
  request?: boolean;
  /** Offer the producer's edited copy of the reply. */
  edited?: boolean;
}) {
  const options: Array<[ReplyView, string]> = [["answer", answerLabel]];
  if (reasoning) options.push(["reasoning", `Reasoning${live ? " · live" : ""}`]);
  if (edited) options.push(["edited", "Your edit"]);
  if (request) options.push(["request", "Model request"]);
  return (
    <span className="reply-view" role="group" aria-label="Show in this card">
      {options.map(([value, text]) => (
        <button key={value} type="button" className={view === value ? "active" : ""} aria-pressed={view === value} onClick={() => onView(value)}>{text}</button>
      ))}
    </span>
  );
}
