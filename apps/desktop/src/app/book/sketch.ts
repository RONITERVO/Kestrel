import * as THREE from "three";

/* Pencil and watercolor helpers for the WebGL book. The stroke builder and pigment grain are
   adapted from Ink Battle's spatial sketchbook (Apache-2.0, see THIRD_PARTY_NOTICES.md): marks are
   authored once, pressure is fixed per path, and nothing boils or changes between frames. */

export type PathPoint = readonly [number, number, number];

export function seeded(seed = 71): () => number {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(1664525, state) + 1013904223) >>> 0;
    return state / 4294967296;
  };
}

/** Thin, pressure-tapered triangular tubes along authored paths. */
export function strokeGeometry(paths: readonly (readonly PathPoint[])[], radius: number): THREE.BufferGeometry {
  const positions: number[] = [];
  const colors: number[] = [];
  const tangent = new THREE.Vector3();
  const normal = new THREE.Vector3();
  const side = new THREE.Vector3();
  const up = new THREE.Vector3(0, 0, 1);
  const across = new THREE.Vector3(1, 0, 0);
  paths.forEach((path, pathIndex) => {
    const rings: THREE.Vector3[][] = [];
    for (let i = 0; i < path.length; i++) {
      const point = path[i];
      const before = path[Math.max(0, i - 1)];
      const after = path[Math.min(path.length - 1, i + 1)];
      tangent.set(after[0] - before[0], after[1] - before[1], after[2] - before[2]).normalize();
      normal.crossVectors(tangent, Math.abs(tangent.z) > 0.9 ? across : up).normalize();
      side.crossVectors(tangent, normal).normalize();
      const pressure = 0.78 + Math.sin(i * 2.3 + pathIndex * 4.1) * 0.16;
      const taper = i === 0 || i === path.length - 1 ? 0.55 : 1;
      rings.push([0, 1, 2].map((j) => {
        const angle = (j * Math.PI * 2) / 3;
        return new THREE.Vector3(...point).addScaledVector(normal, Math.cos(angle) * radius * pressure * taper)
          .addScaledVector(side, Math.sin(angle) * radius * pressure * taper);
      }));
    }
    for (let i = 1; i < rings.length; i++) {
      const value = 0.78 + 0.2 * Math.sin(i * 1.7 + pathIndex) ** 2;
      for (let j = 0; j < 3; j++) {
        const k = (j + 1) % 3;
        for (const vertex of [rings[i - 1][j], rings[i][j], rings[i][k], rings[i - 1][j], rings[i][k], rings[i - 1][k]]) {
          positions.push(vertex.x, vertex.y, vertex.z);
          colors.push(value, value, value);
        }
      }
    }
  });
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("color", new THREE.Float32BufferAttribute(colors, 3));
  geometry.computeBoundingSphere();
  return geometry;
}

/** A slightly wobbly line between two points, so contours read as drawn rather than ruled. */
export function wobblyLine(from: PathPoint, to: PathPoint, steps: number, random: () => number, wobble = 0.8): PathPoint[] {
  return Array.from({ length: steps + 1 }, (_, i) => {
    const t = i / steps;
    const lift = i === 0 || i === steps ? 0 : (random() - 0.5) * wobble;
    return [from[0] + (to[0] - from[0]) * t + lift, from[1] + (to[1] - from[1]) * t + lift, from[2] + (to[2] - from[2]) * t] as const;
  });
}

export function ellipsePath(cx: number, cy: number, z: number, rx: number, ry: number, steps = 26, phase = 0): PathPoint[] {
  return Array.from({ length: steps + 1 }, (_, i) => {
    const t = (i / steps) * Math.PI * 2 + phase;
    const wobble = 1 + Math.sin(t * 3 + phase) * 0.03;
    return [cx + Math.cos(t) * rx * wobble, cy + Math.sin(t) * ry * wobble, z] as const;
  });
}

function canvasTexture(width: number, height: number, draw: (context: CanvasRenderingContext2D) => void): THREE.CanvasTexture {
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (context) draw(context);
  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.anisotropy = 4;
  return texture;
}

function grain(context: CanvasRenderingContext2D, width: number, height: number, amount: number, seed: number) {
  const random = seeded(seed);
  const pixels = context.getImageData(0, 0, width, height);
  for (let i = 0; i < pixels.data.length; i += 4) {
    const value = (random() - 0.5) * amount;
    pixels.data[i] += value;
    pixels.data[i + 1] += value;
    pixels.data[i + 2] += value;
  }
  context.putImageData(pixels, 0, 0);
}

/** Brown bookcloth with dry-brush streaks. */
export function coverTexture(color: string, deep: string, light: string): THREE.CanvasTexture {
  const texture = canvasTexture(256, 256, (context) => {
    context.fillStyle = color;
    context.fillRect(0, 0, 256, 256);
    const random = seeded(19);
    context.globalAlpha = 0.16;
    for (let i = 0; i < 90; i++) {
      context.strokeStyle = i % 3 ? deep : light;
      context.lineWidth = 1 + random() * 3;
      const y = random() * 256;
      context.beginPath();
      context.moveTo(0, y);
      context.bezierCurveTo(80, y + (random() - 0.5) * 14, 170, y + (random() - 0.5) * 14, 256, y + (random() - 0.5) * 8);
      context.stroke();
    }
    context.globalAlpha = 1;
    grain(context, 256, 256, 18, 23);
  });
  texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
  return texture;
}

/** The fore-edge of stacked leaves: fine lines with slight irregularity. */
export function pageEdgeTexture(paper: string, line: string): THREE.CanvasTexture {
  const texture = canvasTexture(64, 256, (context) => {
    context.fillStyle = paper;
    context.fillRect(0, 0, 64, 256);
    const random = seeded(31);
    for (let y = 2; y < 256; y += 3 + Math.floor(random() * 2)) {
      context.strokeStyle = line;
      context.globalAlpha = 0.25 + random() * 0.35;
      context.lineWidth = 0.8;
      context.beginPath();
      context.moveTo(0, y);
      context.lineTo(64, y + (random() - 0.5));
      context.stroke();
    }
    context.globalAlpha = 1;
    grain(context, 64, 256, 10, 37);
  });
  texture.wrapS = texture.wrapT = THREE.RepeatWrapping;
  return texture;
}

/** Soft contact shadow under the book, painted rather than lit. */
export function shadowTexture(strong: string, soft: string): THREE.CanvasTexture {
  return canvasTexture(128, 128, (context) => {
    const gradient = context.createRadialGradient(64, 64, 10, 64, 64, 64);
    gradient.addColorStop(0, strong);
    gradient.addColorStop(0.62, soft);
    gradient.addColorStop(1, "transparent");
    context.fillStyle = gradient;
    context.fillRect(0, 0, 128, 128);
  });
}

export type LeafFace = { heading?: string; numeral?: string; hue: string; paper: string; ink: string; rule: string; spine: string };

/** One side of a turning leaf: paper, a chapter wash and ruled pencil lines. */
export function leafTexture(canvas: HTMLCanvasElement, face: LeafFace, mirrored: boolean) {
  const context = canvas.getContext("2d");
  if (!context) return;
  const { width, height } = canvas;
  context.save();
  context.clearRect(0, 0, width, height);
  if (mirrored) {
    context.translate(width, 0);
    context.scale(-1, 1);
  }
  context.fillStyle = face.paper;
  context.fillRect(0, 0, width, height);
  const random = seeded(face.heading ? face.heading.length * 97 : 11);
  context.globalAlpha = 0.12;
  context.fillStyle = face.hue;
  for (let i = 0; i < 9; i++) {
    context.beginPath();
    const x = (mirrored ? 0.1 : 0.55) * width + random() * width * 0.35;
    const y = random() * height * 0.3;
    context.ellipse(x, y, 60 + random() * 140, 30 + random() * 70, random() * Math.PI, 0, Math.PI * 2);
    context.fill();
  }
  context.globalAlpha = 1;
  const margin = width * 0.1;
  let y = height * 0.2;
  if (face.heading) {
    context.fillStyle = face.ink;
    context.font = `700 ${Math.round(width * 0.075)}px Caveat, cursive`;
    context.fillText(`${face.numeral ? `${face.numeral} · ` : ""}${face.heading}`, margin, height * 0.14);
    context.strokeStyle = face.hue;
    context.lineWidth = 3;
    context.beginPath();
    context.moveTo(margin, height * 0.16);
    context.lineTo(margin + width * 0.42, height * 0.162);
    context.stroke();
    y = height * 0.24;
  }
  context.strokeStyle = face.rule;
  context.lineWidth = 1.3;
  while (y < height * 0.9) {
    const length = width * (0.5 + random() * 0.3);
    context.globalAlpha = 0.55;
    context.beginPath();
    context.moveTo(margin, y);
    context.quadraticCurveTo(margin + length / 2, y + (random() - 0.5) * 2, margin + length, y + (random() - 0.5) * 2);
    context.stroke();
    y += height * 0.045;
  }
  context.globalAlpha = 1;
  context.restore();
  const gradient = context.createLinearGradient(0, 0, width, 0);
  gradient.addColorStop(0, face.spine);
  gradient.addColorStop(0.08, "transparent");
  context.fillStyle = gradient;
  context.fillRect(0, 0, width, height);
}
