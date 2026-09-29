import { useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { PageTurner } from "./FlowPages";
import "./paging.css";

const TURNER_HEIGHT = 40;

/**
 * A list that shows whole items a page at a time. Every visible entry is complete and the folio
 * says how many pages exist, so nothing hides below a scrollbar. The page holding the selected
 * item opens automatically. Without layout (tests, first paint) every item is shown.
 *
 * The frame must get its height from its surroundings (a flex share or a max-height), never from
 * its own items: the item count is chosen from that height, so a height that followed the items
 * would feed back into the choice.
 */
export function PagedList<T>({
  items,
  itemKey,
  renderItem,
  label,
  selectedKey,
  empty,
  className = "",
  as = "div",
}: {
  items: readonly T[];
  itemKey: (item: T) => string;
  renderItem: (item: T) => ReactNode;
  label: string;
  selectedKey?: string | null;
  empty?: ReactNode;
  className?: string;
  as?: "div" | "nav";
}) {
  const frameRef = useRef<HTMLDivElement>(null);
  const [perPage, setPerPage] = useState(0);
  const [page, setPage] = useState(0);
  const tallest = useRef(0);
  const decided = useRef<{ room: number; count: number; columns: number; perPage: number } | null>(null);
  const pages = perPage ? Math.max(1, Math.ceil(items.length / perPage)) : 1;

  useLayoutEffect(() => {
    const frame = frameRef.current;
    if (!frame || typeof ResizeObserver === "undefined") return;
    const fit = () => {
      const list = frame.querySelector<HTMLElement>(":scope > .paged-list-items");
      const rendered = list ? [...list.children] as HTMLElement[] : [];
      if (!list || !rendered.length) return;
      // The room the frame may take: its max-height when capped, otherwise its laid-out height.
      const maxHeight = getComputedStyle(frame).maxHeight;
      const cap = maxHeight.endsWith("%")
        ? ((frame.parentElement?.clientHeight ?? 0) * Number.parseFloat(maxHeight)) / 100 || Number.NaN
        : Number.parseFloat(maxHeight);
      const room = Number.isFinite(cap) ? Math.max(cap, frame.clientHeight) : frame.clientHeight;
      if (!room) return;
      // A grid list (thumbnails) shows several items per row.
      const listStyle = getComputedStyle(list);
      const columns = Math.max(1, listStyle.gridTemplateColumns.split(" ").filter((track) => track && track !== "none").length);
      // Re-deciding for the same room and items can only oscillate; it never adds information.
      const previous = decided.current;
      if (previous && previous.count === items.length && previous.columns === columns && Math.abs(previous.room - room) < 2) return;
      const gap = Number.parseFloat(listStyle.rowGap) || 0;
      // Plan for the tallest entry seen so far, so a page never clips its last item.
      tallest.current = Math.max(tallest.current, ...rendered.map((item) => item.offsetHeight));
      const itemHeight = tallest.current + gap;
      if (!itemHeight) return;
      const fitsAll = Math.ceil(items.length / columns) * itemHeight - gap <= room;
      const next = fitsAll ? items.length : Math.max(1, Math.floor((room - TURNER_HEIGHT + gap) / itemHeight)) * columns;
      decided.current = { room, count: items.length, columns, perPage: next };
      setPerPage((current) => (current === next ? current : next));
    };
    decided.current = null;
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(frame);
    return () => observer.disconnect();
  }, [items.length]);

  // Open the selected item's page when the selection or the page size changes, not on every parent
  // render: a reader browsing older pages (while audio plays, say) must not be pulled back.
  const shownSelection = useRef<string | null>(null);
  useLayoutEffect(() => {
    if (!perPage || selectedKey == null) return;
    const signature = `${selectedKey}\u0000${perPage}`;
    if (shownSelection.current === signature) return;
    const index = items.findIndex((item) => itemKey(item) === selectedKey);
    if (index < 0) return;
    shownSelection.current = signature;
    setPage(Math.floor(index / perPage));
  }, [itemKey, items, perPage, selectedKey]);

  const current = Math.min(page, pages - 1);
  const visible = useMemo(
    () => (perPage && perPage < items.length ? items.slice(current * perPage, current * perPage + perPage) : items),
    [current, items, perPage],
  );
  const Frame = as;

  return (
    <div ref={frameRef} className={`paged-list ${className}`}>
      <Frame className="paged-list-items" aria-label={label}>
        {visible.length ? visible.map((item) => <div className="paged-list-item" key={itemKey(item)}>{renderItem(item)}</div>) : empty}
      </Frame>
      {pages > 1 && <PageTurner page={current} pages={pages} label={label} onTurn={(next) => setPage(Math.max(0, Math.min(pages - 1, next)))} />}
    </div>
  );
}
