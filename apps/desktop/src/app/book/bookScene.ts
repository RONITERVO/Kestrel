import * as THREE from "three";
import type { BookLayout, Pose, TurnPhase, TurnRequest } from "./bookModel";
import { READING_POSE } from "./bookModel";
import { quadToMatrix3d, type Point } from "./homography";
import {
  coverTexture,
  ellipsePath,
  leafTexture,
  pageEdgeTexture,
  seeded,
  shadowTexture,
  strokeGeometry,
  wobblyLine,
  type PathPoint,
} from "./sketch";

type Palette = Record<"paper" | "paperEdge" | "cover" | "coverDeep" | "coverLight" | "ribbon" | "ink" | "ink2" | "rule" | "contactShadow" | "contactShadowSoft", string>;

const FOV = 18;
const THICKNESS = 16;
const COVER = 4;
const TURN_MS = 760;
const easeInOut = (t: number) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);

/** The WebGL book reads its colors from tokens.css like every stylesheet; it defines none of its own. */
function readPalette(): Palette {
  const style = getComputedStyle(document.documentElement);
  const token = (name: string) => {
    const value = style.getPropertyValue(name).trim();
    if (!value) throw new Error(`The book color ${name} is missing from app/book/tokens.css.`);
    return value;
  };
  return {
    paper: token("--paper"),
    paperEdge: token("--paper-edge"),
    cover: token("--cover"),
    coverDeep: token("--cover-deep"),
    coverLight: token("--cover-light"),
    ribbon: token("--ribbon"),
    ink: token("--ink"),
    ink2: token("--ink-2"),
    rule: token("--rule"),
    contactShadow: token("--contact-shadow"),
    contactShadowSoft: token("--contact-shadow-soft"),
  };
}

/**
 * The sketchbook object. It renders on demand only: nothing draws while the book is still,
 * so the local model keeps the GPU. The canvas sits above the DOM with pointer-events off;
 * a depth-only occluder in the page plane lets the DOM pages show through it.
 */
export class BookScene {
  readonly renderer: THREE.WebGLRenderer;
  private readonly scene = new THREE.Scene();
  private readonly camera = new THREE.PerspectiveCamera(FOV, 1, 10, 40000);
  private readonly tilt = new THREE.Group();
  private readonly turnGroup = new THREE.Group();
  private readonly book = new THREE.Group();
  private readonly palette = readPalette();
  private textures: THREE.Texture[] = [];
  private bookParts: THREE.Object3D[] = [];
  private rings: THREE.Object3D[] = [];
  private layout: BookLayout | null = null;
  private pose: Pose = { ...READING_POSE };
  private poseFrom: Pose = { ...READING_POSE };
  private poseTo: Pose = { ...READING_POSE };
  private poseStart = 0;
  private poseDuration = 0;
  private turn: (TurnRequest & { start: number; phase: TurnPhase }) | null = null;
  private leaf: { front: THREE.Mesh; back: THREE.Mesh; geometry: THREE.PlaneGeometry; frontCanvas: HTMLCanvasElement; backCanvas: HTMLCanvasElement; frontTexture: THREE.CanvasTexture; backTexture: THREE.CanvasTexture; material: THREE.MeshBasicMaterial[] } | null = null;
  private frame = 0;
  private disposed = false;

  constructor(
    canvas: HTMLCanvasElement,
    private readonly onProject: (transform: string | null, pose: Pose) => void,
  ) {
    this.renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: true, powerPreference: "low-power", preserveDrawingBuffer: false });
    this.renderer.setClearColor(0x000000, 0);
    this.renderer.outputColorSpace = THREE.SRGBColorSpace;
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.5));
    this.scene.add(this.tilt);
    this.tilt.add(this.turnGroup);
    this.turnGroup.add(this.book);
  }

  /** Rebuilds the book around the current DOM layout and draws one frame. */
  setLayout(layout: BookLayout) {
    this.layout = layout;
    const { width, height } = layout.viewport;
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / Math.max(1, height);
    this.camera.position.set(0, 0, height / 2 / Math.tan(THREE.MathUtils.degToRad(FOV / 2)));
    this.camera.near = this.camera.position.z * 0.05;
    this.camera.far = this.camera.position.z * 4;
    this.camera.lookAt(0, 0, 0);
    this.camera.updateProjectionMatrix();
    this.buildBook();
    this.applyPose();
    this.requestRender();
  }

  setPose(pose: Pose, animate: boolean) {
    this.poseFrom = { ...this.pose };
    this.poseTo = { ...pose };
    this.poseStart = performance.now();
    this.poseDuration = animate ? 620 : 0;
    if (!animate) this.pose = { ...pose };
    this.applyPose();
    this.requestRender();
  }

  /** Direct manipulation on the desk: orbit and zoom around the resting pose. */
  nudgePose(delta: Partial<Pose>) {
    const next = {
      pitch: THREE.MathUtils.clamp(this.pose.pitch + (delta.pitch ?? 0), 0, 1.05),
      yaw: THREE.MathUtils.clamp(this.pose.yaw + (delta.yaw ?? 0), -0.9, 0.9),
      zoom: THREE.MathUtils.clamp(this.pose.zoom * (delta.zoom ?? 1), 0.45, 1.15),
    };
    this.poseDuration = 0;
    this.pose = next;
    this.poseTo = next;
    this.applyPose();
    this.requestRender();
  }

  currentPose(): Pose {
    return { ...this.pose };
  }

  turnPage(request: TurnRequest) {
    if (!this.layout) {
      request.onPhase("done");
      return;
    }
    if (this.turn) this.turn.onPhase("done");
    this.prepareLeaf(request);
    this.turn = { ...request, start: performance.now(), phase: "lifting" };
    request.onPhase("lifting");
    this.requestRender();
  }

  requestRender() {
    if (this.disposed || this.frame) return;
    this.frame = requestAnimationFrame((now) => {
      this.frame = 0;
      this.tick(now);
    });
  }

  dispose() {
    this.disposed = true;
    if (this.frame) cancelAnimationFrame(this.frame);
    this.turn?.onPhase("done");
    this.clearParts();
    this.disposeLeaf();
    this.renderer.dispose();
  }

  private tick(now: number) {
    let animating = false;
    if (this.poseDuration > 0) {
      const t = Math.min(1, (now - this.poseStart) / this.poseDuration);
      const k = easeInOut(t);
      this.pose = {
        pitch: THREE.MathUtils.lerp(this.poseFrom.pitch, this.poseTo.pitch, k),
        yaw: THREE.MathUtils.lerp(this.poseFrom.yaw, this.poseTo.yaw, k),
        zoom: THREE.MathUtils.lerp(this.poseFrom.zoom, this.poseTo.zoom, k),
      };
      if (t >= 1) this.poseDuration = 0;
      else animating = true;
      this.applyPose();
    }
    if (this.turn) {
      animating = this.stepTurn(now) || animating;
    }
    this.renderer.render(this.scene, this.camera);
    if (animating) this.requestRender();
  }

  private applyPose() {
    const layout = this.layout;
    if (!layout) return;
    const { pitch, yaw, zoom } = this.pose;
    const centerX = layout.pages.left + layout.pages.width / 2 - layout.viewport.width / 2;
    const centerY = layout.viewport.height / 2 - (layout.pages.top + layout.pages.height / 2);
    this.tilt.position.set(centerX * (1 - pitch * 0.9), centerY * (1 - pitch * 0.9) + pitch * layout.pages.height * 0.04, 0);
    this.tilt.rotation.set(-pitch, 0, 0);
    this.turnGroup.rotation.set(0, 0, yaw);
    this.tilt.scale.setScalar(zoom);
    for (const ring of this.rings) ring.visible = pitch > 0.05 || zoom < 0.98;
    this.tilt.updateMatrixWorld(true);
    this.onProject(this.projectPlane(), this.pose);
  }

  /** Screen-space transform for the DOM plane, or null when the book is flat and unscaled. */
  private projectPlane(): string | null {
    const layout = this.layout;
    if (!layout) return null;
    const { pitch, yaw, zoom } = this.pose;
    if (Math.abs(pitch) < 1e-4 && Math.abs(yaw) < 1e-4 && Math.abs(zoom - 1) < 1e-4) return null;
    const { plane, pages } = layout;
    const cx = pages.left + pages.width / 2;
    const cy = pages.top + pages.height / 2;
    const corners: PathPoint[] = [
      [plane.left - cx, cy - plane.top, 0],
      [plane.left + plane.width - cx, cy - plane.top, 0],
      [plane.left + plane.width - cx, cy - plane.top - plane.height, 0],
      [plane.left - cx, cy - plane.top - plane.height, 0],
    ];
    const vector = new THREE.Vector3();
    const quad = corners.map(([x, y, z]) => {
      vector.set(x, y, z).applyMatrix4(this.book.matrixWorld).project(this.camera);
      return {
        x: ((vector.x + 1) / 2) * layout.viewport.width - plane.left,
        y: ((1 - vector.y) / 2) * layout.viewport.height - plane.top,
      };
    }) as [Point, Point, Point, Point];
    return quadToMatrix3d(plane.width, plane.height, quad);
  }

  private clearParts() {
    for (const part of this.bookParts) {
      part.removeFromParent();
      part.traverse((object) => {
        const mesh = object as THREE.Mesh;
        mesh.geometry?.dispose();
        const material = mesh.material as THREE.Material | THREE.Material[] | undefined;
        if (Array.isArray(material)) material.forEach((item) => item.dispose());
        else material?.dispose();
      });
    }
    this.bookParts = [];
    this.rings = [];
    this.textures.forEach((texture) => texture.dispose());
    this.textures = [];
  }

  private buildBook() {
    const layout = this.layout;
    if (!layout) return;
    this.clearParts();
    const palette = this.palette;
    const { plane, pages, gutter } = layout;
    const cx = pages.left + pages.width / 2;
    const cy = pages.top + pages.height / 2;
    const pw = pages.width;
    const ph = pages.height;
    const left = plane.left - cx;
    const right = plane.left + plane.width - cx;
    const top = cy - plane.top;
    const bottom = cy - plane.top - plane.height;
    const add = (object: THREE.Object3D) => {
      this.book.add(object);
      this.bookParts.push(object);
      return object;
    };

    // The DOM pages are the page surface. This depth-only sheet hides anything behind them.
    const occluder = new THREE.Mesh(new THREE.PlaneGeometry(pw, ph), new THREE.MeshBasicMaterial({ colorWrite: false }));
    occluder.renderOrder = -10;
    add(occluder);
    for (const rect of layout.occluders) {
      const sheet = new THREE.Mesh(new THREE.PlaneGeometry(rect.width, rect.height), new THREE.MeshBasicMaterial({ colorWrite: false }));
      sheet.position.set(rect.left + rect.width / 2 - cx, cy - rect.top - rect.height / 2, 0.2);
      sheet.renderOrder = -10;
      add(sheet);
    }

    const edgeTexture = pageEdgeTexture(palette.paperEdge, palette.ink2);
    edgeTexture.repeat.set(1, THICKNESS / 64);
    const edgeMaterial = new THREE.MeshBasicMaterial({ map: edgeTexture });
    const paperMaterial = new THREE.MeshBasicMaterial({ color: palette.paperEdge });
    const blockWidth = (pw - gutter * 0.35) / 2;
    for (const side of [-1, 1]) {
      const block = new THREE.Mesh(
        new THREE.BoxGeometry(blockWidth, ph, THICKNESS - 1),
        [edgeMaterial, edgeMaterial, edgeMaterial, edgeMaterial, paperMaterial, paperMaterial],
      );
      block.position.set(side * (pw / 2 - blockWidth / 2), 0, -THICKNESS / 2 - 0.6);
      add(block);
    }

    const cover = coverTexture(palette.cover, palette.coverDeep, palette.coverLight);
    cover.repeat.set(plane.width / 256, plane.height / 256);
    const coverMaterial = new THREE.MeshBasicMaterial({ map: cover });
    const coverEdge = new THREE.MeshBasicMaterial({ color: palette.coverDeep });
    const board = new THREE.Mesh(
      new THREE.BoxGeometry(plane.width, plane.height, COVER),
      [coverEdge, coverEdge, coverEdge, coverEdge, coverMaterial, coverEdge],
    );
    board.position.set((left + right) / 2, (top + bottom) / 2, -THICKNESS - COVER / 2);
    add(board);

    // A soft, painted contact shadow on the desk.
    const shadowMap = shadowTexture(palette.contactShadow, palette.contactShadowSoft);
    const shadow = new THREE.Mesh(
      new THREE.PlaneGeometry(plane.width * 1.08, plane.height * 1.1),
      new THREE.MeshBasicMaterial({ map: shadowMap, transparent: true, depthWrite: false }),
    );
    shadow.position.set((left + right) / 2 + 8, (top + bottom) / 2 - 10, -THICKNESS - COVER - 1);
    shadow.renderOrder = -5;
    add(shadow);

    // A golden ribbon from the head of the spine, hanging past the tail onto the desk.
    const ribbonShape = new THREE.Shape();
    const ribbonWidth = Math.max(10, Math.min(16, pw * 0.009));
    const tail = bottom - 22;
    ribbonShape.moveTo(-ribbonWidth / 2, -ph / 2 + 30);
    ribbonShape.lineTo(ribbonWidth / 2, -ph / 2 + 30);
    ribbonShape.lineTo(ribbonWidth / 2 + 2, tail);
    ribbonShape.lineTo(0, tail + 7);
    ribbonShape.lineTo(-ribbonWidth / 2 + 1, tail - 1);
    ribbonShape.closePath();
    const ribbon = new THREE.Mesh(new THREE.ShapeGeometry(ribbonShape), new THREE.MeshBasicMaterial({ color: palette.ribbon }));
    ribbon.position.set(gutter * 0.12, 0, -THICKNESS - COVER - 0.5);
    add(ribbon);

    // Pencil contours: cover outline, page-block rims, spine crease and a few hatch marks.
    const random = seeded(Math.round(pw + ph));
    const paths: PathPoint[][] = [];
    const z = 0.5;
    const coverZ = -THICKNESS;
    paths.push(wobblyLine([left, top, coverZ], [right, top, coverZ], 24, random));
    paths.push(wobblyLine([right, top, coverZ], [right, bottom, coverZ], 24, random));
    paths.push(wobblyLine([right, bottom, coverZ], [left, bottom, coverZ], 24, random));
    paths.push(wobblyLine([left, bottom, coverZ], [left, top, coverZ], 24, random));
    for (const side of [-1, 1]) {
      const outer = side * pw / 2;
      paths.push(wobblyLine([outer, ph / 2, z], [outer, -ph / 2, z], 20, random, 0.5));
      paths.push(wobblyLine([outer, -ph / 2, z], [side * gutter * 0.2, -ph / 2, z], 18, random, 0.5));
      paths.push(wobblyLine([side * gutter * 0.2, ph / 2, z], [outer, ph / 2, z], 18, random, 0.5));
      for (let i = 0; i < 22; i++) {
        const x = side * (gutter + (i / 22) * (pw / 2 - gutter));
        paths.push([[x, bottom + 2, coverZ + 0.5], [x + side * 5, bottom + 7, coverZ + 0.5]]);
      }
    }
    if (layout.crease) paths.push(wobblyLine([0, ph / 2 + 2, z], [0, -ph / 2 - 2, z], 30, random, 0.4));
    const contour = new THREE.Mesh(strokeGeometry(paths, 0.9), new THREE.MeshBasicMaterial({ color: palette.ink2, vertexColors: true, transparent: true, opacity: 0.8 }));
    contour.renderOrder = 2;
    add(contour);

    // Drawn rings at the corners: the handles used to carry the book on the desk and in MR.
    const ringMaterial = new THREE.MeshBasicMaterial({ color: palette.ink2, vertexColors: true });
    for (const [x, y] of [[left, top], [right, top], [right, bottom], [left, bottom]] as const) {
      const ring = new THREE.Mesh(strokeGeometry([ellipsePath(x, y, 10, 20, 16, 28, x * 0.01)], 1.4), ringMaterial);
      ring.visible = false;
      add(ring);
      this.rings.push(ring);
    }
    this.textures.push(edgeTexture, cover, shadowMap);
  }

  private prepareLeaf(request: TurnRequest) {
    const layout = this.layout;
    if (!layout) return;
    this.disposeLeaf();
    const width = (layout.pages.width - layout.gutter) / 2 + layout.gutter / 2;
    const height = layout.pages.height;
    const segments = 36;
    const geometry = new THREE.PlaneGeometry(width, height, segments, 1);
    const frontCanvas = document.createElement("canvas");
    const backCanvas = document.createElement("canvas");
    for (const canvas of [frontCanvas, backCanvas]) {
      canvas.width = 768;
      canvas.height = Math.round(768 * (height / width));
    }
    // Forward: the right page lifts. Its front shows the outgoing page, its back the next chapter.
    const forward = request.direction === 1;
    // Canvas x=0 is always the spine. The back face is seen mirrored, so it is drawn mirrored.
    leafTexture(frontCanvas, forward ? request.from : request.to, false);
    leafTexture(backCanvas, forward ? request.to : request.from, true);
    const frontTexture = new THREE.CanvasTexture(frontCanvas);
    const backTexture = new THREE.CanvasTexture(backCanvas);
    frontTexture.colorSpace = backTexture.colorSpace = THREE.SRGBColorSpace;
    const frontMaterial = new THREE.MeshBasicMaterial({ map: frontTexture, side: THREE.FrontSide, transparent: true, vertexColors: true });
    const backMaterial = new THREE.MeshBasicMaterial({ map: backTexture, side: THREE.BackSide, transparent: true, vertexColors: true });
    const colors = new Float32Array(geometry.attributes.position.count * 3).fill(1);
    geometry.setAttribute("color", new THREE.BufferAttribute(colors, 3));
    const front = new THREE.Mesh(geometry, frontMaterial);
    const back = new THREE.Mesh(geometry, backMaterial);
    front.renderOrder = back.renderOrder = 20;
    frontMaterial.depthTest = backMaterial.depthTest = false;
    this.book.add(front, back);
    this.leaf = { front, back, geometry, frontCanvas, backCanvas, frontTexture, backTexture, material: [frontMaterial, backMaterial] };
    this.bendLeaf(forward ? 0 : Math.PI, request.direction, width);
  }

  /** Curls the leaf around the spine. The free edge leads, the way a finger lifts a page. */
  private bendLeaf(theta: number, direction: 1 | -1, width: number) {
    const leaf = this.leaf;
    if (!leaf) return;
    const position = leaf.geometry.attributes.position as THREE.BufferAttribute;
    const color = leaf.geometry.attributes.color as THREE.BufferAttribute;
    const columns = leaf.geometry.parameters.widthSegments + 1;
    const height = leaf.geometry.parameters.height;
    const lift = Math.sin(theta);
    const xs: number[] = [0];
    const zs: number[] = [0];
    const step = width / (columns - 1);
    for (let i = 1; i < columns; i++) {
      const u = i / (columns - 1);
      const angle = THREE.MathUtils.clamp(theta + direction * 0.95 * lift * u * u, 0, Math.PI);
      xs.push(xs[i - 1] + Math.cos(angle) * step);
      zs.push(zs[i - 1] + Math.sin(angle) * step);
    }
    for (let row = 0; row < 2; row++) {
      for (let i = 0; i < columns; i++) {
        const index = row * columns + i;
        const u = i / (columns - 1);
        const angle = THREE.MathUtils.clamp(theta + direction * 0.95 * lift * u * u, 0, Math.PI);
        position.setXYZ(index, xs[i], row === 0 ? height / 2 : -height / 2, zs[i] + 1.2);
        const shade = 1 - 0.2 * Math.abs(Math.sin(angle)) * (0.6 + 0.4 * u);
        color.setXYZ(index, shade, shade, shade * 0.98);
      }
    }
    position.needsUpdate = true;
    color.needsUpdate = true;
    leaf.geometry.computeBoundingSphere();
  }

  private stepTurn(now: number): boolean {
    const turn = this.turn;
    const layout = this.layout;
    if (!turn || !layout) return false;
    const t = Math.min(1, (now - turn.start) / TURN_MS);
    const k = easeInOut(t);
    const width = (layout.pages.width - layout.gutter) / 2 + layout.gutter / 2;
    const theta = turn.direction === 1 ? k * Math.PI : (1 - k) * Math.PI;
    this.bendLeaf(theta, turn.direction, width);
    const opacity = t < 0.08 ? t / 0.08 : t > 0.88 ? Math.max(0, (1 - t) / 0.12) : 1;
    for (const material of this.leaf?.material ?? []) material.opacity = opacity;
    const phase: TurnPhase = t >= 1 ? "done" : t > 0.88 ? "landing" : t > 0.08 ? "turning" : "lifting";
    if (phase !== turn.phase) {
      turn.phase = phase;
      turn.onPhase(phase);
    }
    if (t >= 1) {
      this.turn = null;
      this.disposeLeaf();
      return false;
    }
    return true;
  }

  private disposeLeaf() {
    const leaf = this.leaf;
    if (!leaf) return;
    leaf.front.removeFromParent();
    leaf.back.removeFromParent();
    leaf.geometry.dispose();
    leaf.frontTexture.dispose();
    leaf.backTexture.dispose();
    leaf.material.forEach((material) => material.dispose());
    this.leaf = null;
  }
}
