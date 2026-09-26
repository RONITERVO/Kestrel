import type { LeafFace } from "./sketch";

/* Plain data shared by the DOM shell and the WebGL scene. Importing this module never loads
   three.js, so the book renders at once and the scene arrives on demand. */

export type Rect = { left: number; top: number; width: number; height: number };

/** Measured DOM geometry. The DOM layout is the truth; the WebGL book is built around it. */
export type BookLayout = {
  viewport: { width: number; height: number };
  /** The book's outer (cover) box. The DOM plane that carries pages and index tabs. */
  plane: Rect;
  /** The open spread, both pages and the gutter. */
  pages: Rect;
  gutter: number;
  /** Draw the pencil crease down the spine. Wide spreads (editors) keep their middle clear. */
  crease: boolean;
  /** Other DOM surfaces lying in the page plane, such as index tabs. They hide what is beneath them. */
  occluders: Rect[];
};

export type Pose = { pitch: number; yaw: number; zoom: number };
export const READING_POSE: Pose = { pitch: 0, yaw: 0, zoom: 1 };
export const DESK_POSE: Pose = { pitch: 0.72, yaw: -0.1, zoom: 0.84 };

export type TurnPhase = "lifting" | "turning" | "landing" | "done";
export type TurnRequest = {
  direction: 1 | -1;
  from: LeafFace;
  to: LeafFace;
  onPhase: (phase: TurnPhase) => void;
};

/** WebGL is optional. jsdom, disabled GPUs and lost contexts keep the same book in CSS. */
export function webglAvailable(): boolean {
  if (typeof window === "undefined" || typeof document === "undefined") return false;
  if (navigator.userAgent.includes("jsdom")) return false;
  try {
    const probe = document.createElement("canvas");
    return !!(probe.getContext("webgl2") ?? probe.getContext("webgl"));
  } catch {
    return false;
  }
}
