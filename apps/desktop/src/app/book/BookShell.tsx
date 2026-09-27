import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { DESK_POSE, READING_POSE, webglAvailable, type BookLayout, type Rect, type TurnPhase } from "./bookModel";
import type { BookScene } from "./bookScene";
import { CHAPTERS, chapterIndex, chapterOf, type AppView } from "./chapters";
import "./book.css";

export type BookControls = {
  /** True while the book lies on the desk at an angle; false in the flat reading pose. */
  lifted: boolean;
  toggleLifted: () => void;
  webgl: boolean;
};

/** Chapters whose work surface spans both pages: editors keep the middle of the spread clear. */
const WIDE: ReadonlySet<AppView> = new Set(["studio", "image", "music"]);

type Turn = { from: AppView; to: AppView; direction: 1 | -1 };

function cssColor(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function offsetRect(element: HTMLElement, stage: HTMLElement): Rect {
  let left = 0;
  let top = 0;
  let node: HTMLElement | null = element;
  while (node && node !== stage) {
    left += node.offsetLeft;
    top += node.offsetTop;
    node = node.offsetParent as HTMLElement | null;
  }
  return { left, top, width: element.offsetWidth, height: element.offsetHeight };
}

/**
 * The Kestrel book. The DOM owns layout, text and every control; WebGL draws the bound object
 * around it (cover, page block, ribbon, rings) and the leaf that turns between chapters.
 */
export function BookShell({
  view,
  onView,
  banner,
  children,
}: {
  view: AppView;
  onView: (view: AppView) => void;
  banner: (controls: BookControls) => ReactNode;
  children: ReactNode;
}) {
  const shellRef = useRef<HTMLDivElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const planeRef = useRef<HTMLDivElement>(null);
  const workspaceRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<BookScene | null>(null);
  const previousView = useRef(view);
  const [webgl, setWebgl] = useState(false);
  const [lifted, setLifted] = useState(false);
  const wide = WIDE.has(view);
  const wideRef = useRef(wide);
  wideRef.current = wide;
  const current = chapterIndex(view);
  const chapter = chapterOf(view);

  const measure = useCallback((): BookLayout | null => {
    const stage = stageRef.current;
    const plane = planeRef.current;
    const workspace = workspaceRef.current;
    if (!stage || !plane || !workspace) return null;
    // Coordinates are relative to the desk (the stage), which is exactly the canvas area.
    const gutter = Number.parseFloat(getComputedStyle(workspace).getPropertyValue("--gutter")) || 30;
    return {
      viewport: { width: stage.clientWidth, height: stage.clientHeight },
      plane: offsetRect(plane, stage),
      pages: offsetRect(workspace, stage),
      gutter,
      crease: !wideRef.current,
      occluders: [...plane.querySelectorAll<HTMLElement>(".book-tabs button")].map((tab) => offsetRect(tab, stage)),
    };
  }, []);

  // WebGL is optional: the same book is drawn in CSS until, or unless, the scene is ready.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !webglAvailable()) return;
    let disposed = false;
    let scene: BookScene | null = null;
    const lost = (event: Event) => {
      event.preventDefault();
      sceneRef.current?.dispose();
      sceneRef.current = null;
      if (planeRef.current) planeRef.current.style.transform = "";
      setWebgl(false);
      setLifted(false);
    };
    void import("./bookScene").then(({ BookScene }) => {
      if (disposed) return;
      try {
        scene = new BookScene(canvas, (transform) => {
          const plane = planeRef.current;
          if (plane) plane.style.transform = transform ?? "";
        });
      } catch {
        return;
      }
      sceneRef.current = scene;
      canvas.addEventListener("webglcontextlost", lost);
      setWebgl(true);
    }).catch(() => undefined);
    return () => {
      disposed = true;
      canvas.removeEventListener("webglcontextlost", lost);
      scene?.dispose();
      sceneRef.current = null;
    };
  }, []);

  // Keep the drawn book fitted to the DOM spread.
  useLayoutEffect(() => {
    const stage = stageRef.current;
    if (!stage || !webgl) return;
    const update = () => {
      const layout = measure();
      if (layout) sceneRef.current?.setLayout(layout);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(stage);
    window.addEventListener("resize", update);
    void document.fonts?.ready.then(update);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", update);
    };
  }, [measure, webgl, wide]);

  // Chapter changes turn a leaf. The outgoing chapter stays on the near page until it is covered.
  useEffect(() => {
    const from = previousView.current;
    previousView.current = view;
    const shell = shellRef.current;
    const scene = sceneRef.current;
    if (from === view || !shell || !scene) return;
    const turn: Turn = { from, to: view, direction: chapterIndex(view) > chapterIndex(from) ? 1 : -1 };
    const section = (id: AppView) => shell.querySelector<HTMLElement>(`.retained-app-view[data-view-id="${id}"]`);
    const outgoing = section(turn.from);
    const incoming = section(turn.to);
    const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
    if (reduced || !outgoing || !incoming) return;
    const fromChapter = chapterOf(turn.from);
    const toChapter = chapterOf(turn.to);
    const face = (id: AppView, numeral: string, heading: string) => ({
      numeral,
      heading,
      hue: cssColor(`--ch-${id}`),
      paper: cssColor("--paper"),
      ink: cssColor("--ink"),
      rule: cssColor("--rule"),
    });
    const setPhase = (phase: TurnPhase) => {
      if (phase === "done") {
        delete shell.dataset.turnPhase;
        delete shell.dataset.turnDirection;
        outgoing.removeAttribute("data-turn-role");
        incoming.removeAttribute("data-turn-role");
        const layout = measure();
        if (layout) sceneRef.current?.setLayout(layout);
        return;
      }
      shell.dataset.turnPhase = phase;
      shell.dataset.turnDirection = turn.direction === 1 ? "forward" : "backward";
      outgoing.dataset.turnRole = "from";
      incoming.dataset.turnRole = "to";
    };
    // Index tabs change sides as soon as the chapter changes; fit the book to them first.
    const layout = measure();
    if (layout) scene.setLayout(layout);
    scene.turnPage({
      direction: turn.direction,
      from: face(turn.from, fromChapter.numeral, fromChapter.label),
      to: face(turn.to, toChapter.numeral, toChapter.label),
      onPhase: setPhase,
    });
  }, [measure, view]);

  const toggleLifted = useCallback(() => {
    const scene = sceneRef.current;
    if (!scene) return;
    setLifted((value) => {
      scene.setPose(value ? READING_POSE : DESK_POSE, true);
      return !value;
    });
  }, []);

  useEffect(() => {
    if (!lifted) return;
    const settle = (event: KeyboardEvent) => {
      if (event.key === "Escape") toggleLifted();
    };
    window.addEventListener("keydown", settle);
    return () => window.removeEventListener("keydown", settle);
  }, [lifted, toggleLifted]);

  // Pages never scroll. The browser can still scroll a clipped container to reveal a focused field,
  // a find-in-page match or a scrollIntoView target, which slides the page under its headings.
  // Put such containers back; turnable pages and the scrollers the book allows are left alone.
  useEffect(() => {
    const workspace = workspaceRef.current;
    if (!workspace) return;
    const settle = (event: Event) => {
      const target = event.target;
      if (!(target instanceof HTMLElement) || !workspace.contains(target) || target.closest(".flow-viewport") === target) return;
      const style = getComputedStyle(target);
      if (style.overflowY !== "hidden" && style.overflowX !== "hidden") return;
      if (style.overflowY === "hidden" && target.scrollTop) target.scrollTop = 0;
      if (style.overflowX === "hidden" && target.scrollLeft) target.scrollLeft = 0;
    };
    workspace.addEventListener("scroll", settle, true);
    return () => workspace.removeEventListener("scroll", settle, true);
  }, []);

  // On the desk, dragging the desk orbits the book and a corner ring turns it.
  const orbit = (event: ReactPointerEvent<HTMLElement>, mode: "orbit" | "ring") => {
    const scene = sceneRef.current;
    if (!scene || !lifted || event.button !== 0) return;
    if (mode === "orbit" && event.target !== event.currentTarget) return;
    event.preventDefault();
    const target = event.currentTarget;
    target.setPointerCapture(event.pointerId);
    let lastX = event.clientX;
    let lastY = event.clientY;
    const move = (next: PointerEvent) => {
      const dx = next.clientX - lastX;
      const dy = next.clientY - lastY;
      lastX = next.clientX;
      lastY = next.clientY;
      scene.nudgePose(mode === "orbit" ? { yaw: dx * 0.0022, pitch: dy * 0.0025 } : { yaw: dx * 0.0035 - dy * 0.0012 });
    };
    const end = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", end);
      target.removeEventListener("pointercancel", end);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", end);
    target.addEventListener("pointercancel", end);
  };

  return (
    <div
      ref={shellRef}
      className={`app-shell app-shell-${view} ${webgl ? "has-gl" : "no-gl"} ${wide ? "is-wide" : "is-split"} ${lifted ? "is-lifted" : ""}`}
      data-view={view}
      data-chapter={view}
    >
      {banner({ lifted, toggleLifted, webgl })}
      <div
        ref={stageRef}
        className="book-stage"
        onPointerDown={(event) => orbit(event, "orbit")}
        onDoubleClick={(event) => {
          if (lifted && event.target === event.currentTarget) toggleLifted();
        }}
        onWheel={(event) => {
          if (lifted && event.target === event.currentTarget) sceneRef.current?.nudgePose({ zoom: event.deltaY > 0 ? 0.95 : 1.05 });
        }}
      >
        <div ref={planeRef} className="book-plane">
          <nav className="book-tabs view-switcher" aria-label="Kestrel sections">
            {CHAPTERS.map((item, index) => {
              const Icon = item.icon;
              const active = item.id === view;
              return (
                <button
                  key={item.id}
                  type="button"
                  className={active ? "active" : ""}
                  data-chapter={item.id}
                  data-side={index < current ? "left" : "right"}
                  style={{ "--tab-index": index } as CSSProperties}
                  aria-current={active ? "page" : undefined}
                  title={`${item.numeral} · ${item.label} — ${item.purpose}`}
                  onClick={() => onView(item.id)}
                >
                  <Icon aria-hidden="true" />
                  <span>{item.label}</span>
                </button>
              );
            })}
          </nav>
          <div ref={workspaceRef} className={`workspace workspace-${view}`}>
            <header className="running-heads" aria-hidden="true">
              <span className="running-head-left">{chapter.numeral} · {chapter.label}</span>
              <span className="running-head-right">{chapter.purpose}</span>
            </header>
            {children}
            <span className="book-gutter" aria-hidden="true" />
          </div>
          {(["tl", "tr", "br", "bl"] as const).map((corner) => (
            <span
              key={corner}
              className={`book-ring book-ring-${corner}`}
              aria-hidden="true"
              onPointerDown={(event) => orbit(event, "ring")}
            />
          ))}
        </div>
        <canvas ref={canvasRef} className="book-gl" aria-hidden="true" />
      </div>
    </div>
  );
}
