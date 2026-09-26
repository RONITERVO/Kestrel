export type Point = { x: number; y: number };

/**
 * CSS `matrix3d` that maps an element's untransformed box (0,0)-(width,height) onto four
 * screen points given relative to that box's origin: top-left, top-right, bottom-right,
 * bottom-left. The DOM pages follow the WebGL book through this projective transform, so
 * their controls stay real, focusable and hit-tested in every pose.
 */
export function quadToMatrix3d(width: number, height: number, quad: readonly [Point, Point, Point, Point]): string | null {
  if (width <= 0 || height <= 0) return null;
  const [p0, p1, p2, p3] = quad;
  const dx1 = p1.x - p2.x;
  const dx2 = p3.x - p2.x;
  const dx3 = p0.x - p1.x + p2.x - p3.x;
  const dy1 = p1.y - p2.y;
  const dy2 = p3.y - p2.y;
  const dy3 = p0.y - p1.y + p2.y - p3.y;
  let g = 0;
  let h = 0;
  if (Math.abs(dx3) > 1e-9 || Math.abs(dy3) > 1e-9) {
    const determinant = dx1 * dy2 - dx2 * dy1;
    if (Math.abs(determinant) < 1e-12) return null;
    g = (dx3 * dy2 - dx2 * dy3) / determinant;
    h = (dx1 * dy3 - dx3 * dy1) / determinant;
  }
  const a = p1.x - p0.x + g * p1.x;
  const b = p3.x - p0.x + h * p3.x;
  const d = p1.y - p0.y + g * p1.y;
  const e = p3.y - p0.y + h * p3.y;
  const values = [
    a / width, d / width, 0, g / width,
    b / height, e / height, 0, h / height,
    0, 0, 1, 0,
    p0.x, p0.y, 0, 1,
  ];
  if (values.some((value) => !Number.isFinite(value))) return null;
  return `matrix3d(${values.map((value) => Number(value.toPrecision(12))).join(",")})`;
}

/** Applies the same projective mapping in script, for tests and for hit checks. */
export function mapThroughQuad(width: number, height: number, quad: readonly [Point, Point, Point, Point], point: Point): Point | null {
  const matrix = quadToMatrix3d(width, height, quad);
  if (!matrix) return null;
  const m = matrix.slice(9, -1).split(",").map(Number);
  const w = m[3] * point.x + m[7] * point.y + m[15];
  return { x: (m[0] * point.x + m[4] * point.y + m[12]) / w, y: (m[1] * point.x + m[5] * point.y + m[13]) / w };
}
