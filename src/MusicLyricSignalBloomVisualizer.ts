/* SPDX-FileCopyrightText: 2026 Roni Tervo
 * SPDX-License-Identifier: Apache-2.0
 *
 * An original Kestrel visual lyric renderer. It is dependency-free, bounded, and shares the
 * producer's single Web Audio analyser with every other visual theme.
 */

import type { MusicLyricFrame } from "./MusicLyricReactivity";

interface LightSeed {
  angle: number;
  distance: number;
  drift: number;
  size: number;
  phase: number;
  warmth: number;
}

interface SignalSpark {
  x: number;
  y: number;
  vx: number;
  vy: number;
  age: number;
  duration: number;
  hue: number;
}

interface SignalPulse {
  x: number;
  y: number;
  age: number;
  duration: number;
  strength: number;
}

export class SignalBloomMusicLyricVisualizer {
  private readonly context: CanvasRenderingContext2D;
  private readonly trailCanvas: HTMLCanvasElement;
  private readonly trailContext: CanvasRenderingContext2D;
  private frameBands = new Float32Array(48);
  private readonly lights: LightSeed[] = Array.from({ length: 112 }, (_, index) => ({
    angle: seeded(index * 6 + 1) * Math.PI * 2,
    distance: 0.08 + seeded(index * 6 + 2) * 0.92,
    drift: (seeded(index * 6 + 3) - 0.5) * 0.16,
    size: 0.45 + seeded(index * 6 + 4) * 2.1,
    phase: seeded(index * 6 + 5) * Math.PI * 2,
    warmth: seeded(index * 6 + 6),
  }));
  private readonly lightPositions = new Float32Array(224);
  private readonly sparks: SignalSpark[] = Array.from({ length: 72 }, () => ({
    x: 0, y: 0, vx: 0, vy: 0, age: 1, duration: 1, hue: 190,
  }));
  private readonly pulses: SignalPulse[] = Array.from({ length: 6 }, () => ({
    x: 0, y: 0, age: 1, duration: 1, strength: 0,
  }));
  private bloom = 0;
  private rotation = 0;
  private sparkCursor = 0;
  private pulseCursor = 0;
  private lastTransientBurstAt = -1;

  constructor(private readonly canvas: HTMLCanvasElement) {
    const context = canvas.getContext("2d");
    if (!context) throw new Error("The Signal bloom lyric canvas is unavailable.");
    this.context = context;
    this.trailCanvas = document.createElement("canvas");
    const trailContext = this.trailCanvas.getContext("2d");
    if (!trailContext) throw new Error("The Signal bloom trail canvas is unavailable.");
    this.trailContext = trailContext;
  }

  draw(frame: MusicLyricFrame) {
    this.resize();
    this.frameBands = frame.bands;
    this.bloom = Math.max(
      this.bloom * Math.exp(-frame.delta * 3.8),
      frame.transient,
      frame.beat * 1.08,
    );
    this.rotation += frame.delta * (0.045 + frame.presence * 0.16 + frame.centroid * 0.08);

    const ratio = window.devicePixelRatio || 1;
    const width = this.canvas.width / ratio;
    const height = this.canvas.height / ratio;
    const journeyX = width * (0.18 + frame.progress * 0.64);
    const journeyY = height * (0.47 + Math.sin(frame.progress * Math.PI * 2.4) * 0.055);
    const active = frame.layout.activeWord;
    const focusX = active ? journeyX * 0.55 + (active.left + active.right) * 0.225 : journeyX;
    const focusY = active ? journeyY * 0.55 + (active.top + active.bottom) * 0.225 : journeyY;
    const context = this.context;
    this.decayTrails(frame);
    context.clearRect(0, 0, this.canvas.width, this.canvas.height);
    context.save();
    context.scale(ratio, ratio);
    this.drawNight(context, width, height, focusX, focusY, frame.energy, frame.time);
    this.drawConstellation(context, width, height, focusX, focusY, frame.energy, frame.air, frame.time);
    context.restore();
    context.save();
    context.globalAlpha = 0.3 + frame.energy * 0.1;
    context.drawImage(this.trailCanvas, 0, 0);
    context.restore();
    context.save();
    context.scale(ratio, ratio);
    this.drawRibbons(context, width, height, focusX, focusY, frame.presence, frame.time);
    this.drawBloom(
      context,
      width,
      height,
      focusX,
      focusY,
      frame.bass,
      frame.beat,
      frame.presence,
      frame.air,
      frame.time,
    );
    this.emitReactiveBursts(focusX, focusY, frame);
    this.drawPulsesAndSparks(context, frame);
    this.drawWave(context, width, height, frame.waveform, frame.energy, frame.air, frame.time, frame.hasSignal);
    this.drawJourney(context, width, height, focusX, focusY, frame.progress, frame.energy, frame.time);
    context.restore();
    this.captureTrails(width, height, ratio, focusX, focusY, frame);
  }

  destroy() {
    for (const spark of this.sparks) spark.age = spark.duration;
    for (const pulse of this.pulses) pulse.age = pulse.duration;
    this.trailContext.clearRect(0, 0, this.trailCanvas.width, this.trailCanvas.height);
  }

  private resize() {
    const rectangle = this.canvas.getBoundingClientRect();
    const ratio = window.devicePixelRatio || 1;
    const width = Math.max(1, Math.round(rectangle.width * ratio));
    const height = Math.max(1, Math.round(rectangle.height * ratio));
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
      this.trailCanvas.width = width;
      this.trailCanvas.height = height;
    }
  }

  private decayTrails(frame: MusicLyricFrame) {
    const decayRate = 5.8 - frame.energy * 1.4 - frame.air * 0.45;
    const eraseAlpha = 1 - Math.exp(-frame.delta * decayRate);
    const context = this.trailContext;
    context.save();
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.globalCompositeOperation = "destination-out";
    context.fillStyle = `rgba(0, 0, 0, ${eraseAlpha})`;
    context.fillRect(0, 0, this.trailCanvas.width, this.trailCanvas.height);
    context.restore();
  }

  private captureTrails(
    width: number,
    height: number,
    ratio: number,
    focusX: number,
    focusY: number,
    frame: MusicLyricFrame,
  ) {
    const decayRate = 5.8 - frame.energy * 1.4 - frame.air * 0.45;
    const eraseAlpha = 1 - Math.exp(-frame.delta * decayRate);
    const captureAlpha = Math.min(0.14, eraseAlpha * (0.7 + frame.presence * 0.18));
    const context = this.trailContext;
    context.save();
    context.scale(ratio, ratio);
    this.drawRibbons(context, width, height, focusX, focusY, frame.presence, frame.time, captureAlpha);
    this.drawBloom(
      context,
      width,
      height,
      focusX,
      focusY,
      frame.bass,
      frame.beat,
      frame.presence,
      frame.air,
      frame.time,
      captureAlpha,
    );
    context.restore();
  }

  private drawNight(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    focusX: number,
    focusY: number,
    energy: number,
    time: number,
  ) {
    context.fillStyle = "#050713";
    context.fillRect(0, 0, width, height);
    const wash = context.createRadialGradient(focusX, focusY, 0, focusX, focusY, Math.max(width, height) * 0.72);
    wash.addColorStop(0, `rgba(65, 40, 128, ${0.2 + energy * 0.22})`);
    wash.addColorStop(0.42, `rgba(16, 50, 86, ${0.17 + energy * 0.14})`);
    wash.addColorStop(1, "rgba(3, 5, 14, 0)");
    context.fillStyle = wash;
    context.fillRect(0, 0, width, height);

    context.save();
    context.globalAlpha = 0.12;
    context.strokeStyle = "#8deaff";
    context.lineWidth = 0.55;
    const spacing = Math.max(34, Math.min(width, height) * 0.075);
    const offsetX = (time * 5) % spacing;
    const offsetY = (time * 2.5) % spacing;
    for (let x = -spacing + offsetX; x < width + spacing; x += spacing) {
      context.beginPath();
      context.moveTo(x, 0);
      context.lineTo(x + height * 0.18, height);
      context.stroke();
    }
    for (let y = -spacing + offsetY; y < height + spacing; y += spacing) {
      context.beginPath();
      context.moveTo(0, y);
      context.lineTo(width, y + width * 0.035);
      context.stroke();
    }
    context.restore();
  }

  private drawConstellation(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    focusX: number,
    focusY: number,
    energy: number,
    air: number,
    time: number,
  ) {
    const radius = Math.min(width, height) * (0.24 + energy * 0.1);
    context.save();
    context.globalCompositeOperation = "lighter";
    for (let index = 0; index < this.lights.length; index += 1) {
      const light = this.lights[index];
      const band = this.frameBands[index % this.frameBands.length] ?? 0;
      const angle = light.angle + this.rotation * (0.45 + light.drift) + Math.sin(time * 0.2 + light.phase) * 0.04;
      const distance = radius * light.distance * (1 + band * 0.24 + this.bloom * 0.08);
      const x = focusX + Math.cos(angle) * distance * (1.55 + light.drift);
      const y = focusY + Math.sin(angle) * distance * 0.82;
      this.lightPositions[index * 2] = x;
      this.lightPositions[index * 2 + 1] = y;
    }
    context.globalAlpha = 0.07 + air * 0.13;
    context.strokeStyle = "rgba(126,224,255,.8)";
    context.lineWidth = 0.45;
    for (let index = 0; index < this.lights.length; index += 3) {
      const target = (index + 7 + Math.floor(this.lights[index].warmth * 9)) % this.lights.length;
      const x = this.lightPositions[index * 2];
      const y = this.lightPositions[index * 2 + 1];
      const targetX = this.lightPositions[target * 2];
      const targetY = this.lightPositions[target * 2 + 1];
      if (Math.hypot(targetX - x, targetY - y) > radius * 0.76) continue;
      context.beginPath();
      context.moveTo(x, y);
      context.lineTo(targetX, targetY);
      context.stroke();
    }
    context.globalAlpha = 1;
    for (let index = 0; index < this.lights.length; index += 1) {
      const light = this.lights[index];
      const band = this.frameBands[index % this.frameBands.length] ?? 0;
      const x = this.lightPositions[index * 2];
      const y = this.lightPositions[index * 2 + 1];
      const alpha = 0.11 + band * 0.5 + air * 0.2;
      const size = light.size * (0.7 + band * 1.9 + this.bloom * 0.8);
      context.fillStyle = light.warmth > 0.78
        ? `rgba(255, 189, 116, ${alpha})`
        : light.warmth > 0.4
          ? `rgba(131, 229, 255, ${alpha})`
          : `rgba(186, 137, 255, ${alpha})`;
      context.beginPath();
      context.arc(x, y, size, 0, Math.PI * 2);
      context.fill();
      if (band > 0.42) {
        context.globalAlpha = Math.min(0.5, band * 0.42);
        context.beginPath();
        context.moveTo(x - size * 4, y);
        context.lineTo(x + size * 4, y);
        context.moveTo(x, y - size * 4);
        context.lineTo(x, y + size * 4);
        context.strokeStyle = context.fillStyle;
        context.lineWidth = 0.55;
        context.stroke();
        context.globalAlpha = 1;
      }
    }
    context.restore();
  }

  private drawRibbons(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    focusX: number,
    focusY: number,
    presence: number,
    time: number,
    opacity = 1,
  ) {
    const colors = [
      [104, 235, 255],
      [174, 105, 255],
      [255, 123, 188],
    ] as const;
    context.save();
    context.globalCompositeOperation = "lighter";
    for (let ribbon = 0; ribbon < colors.length; ribbon += 1) {
      const [red, green, blue] = colors[ribbon];
      const vertical = (ribbon - 1) * height * 0.055;
      const gradient = context.createLinearGradient(0, 0, width, 0);
      gradient.addColorStop(0, `rgba(${red},${green},${blue},0)`);
      gradient.addColorStop(0.2, `rgba(${red},${green},${blue},${0.08 + presence * 0.11})`);
      gradient.addColorStop(0.54, `rgba(${red},${green},${blue},${0.24 + presence * 0.26})`);
      gradient.addColorStop(1, `rgba(${red},${green},${blue},0)`);
      context.strokeStyle = gradient;
      context.lineCap = "round";
      context.shadowColor = `rgba(${red},${green},${blue},.55)`;
      context.shadowBlur = 18 + presence * 34;
      for (let echo = 0; echo < 3; echo += 1) {
        context.globalAlpha = opacity * (0.82 - echo * 0.23);
        context.lineWidth = 1.2 + echo * 2.4 + presence * 3.2;
        context.beginPath();
        for (let point = 0; point <= 96; point += 1) {
          const fraction = point / 96;
          const x = fraction * width;
          const bandIndex = Math.min(this.frameBands.length - 1, Math.floor(fraction * this.frameBands.length));
          const band = this.frameBands[(bandIndex + ribbon * 7) % this.frameBands.length] ?? 0;
          const focusPull = Math.exp(-Math.pow((x - focusX) / Math.max(1, width * 0.24), 2));
          const wave = Math.sin(fraction * Math.PI * (3.2 + ribbon * 0.52) + time * (0.38 + ribbon * 0.08));
          const fine = Math.sin(fraction * Math.PI * 17 - time * 0.8 + ribbon) * 0.18;
          const y = focusY + vertical + (wave + fine) * height * (0.028 + band * 0.08) * (0.55 + focusPull);
          if (point === 0) context.moveTo(x, y + echo * 2); else context.lineTo(x, y + echo * 2);
        }
        context.stroke();
      }
    }
    context.restore();
  }

  private drawBloom(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    focusX: number,
    focusY: number,
    bass: number,
    beat: number,
    presence: number,
    air: number,
    time: number,
    opacity = 1,
  ) {
    const smaller = Math.min(width, height);
    const core = smaller * (0.055 + bass * 0.042 + this.bloom * 0.014 + beat * 0.018);
    context.save();
    context.globalAlpha = opacity;
    context.globalCompositeOperation = "lighter";
    const aura = context.createRadialGradient(focusX, focusY, 0, focusX, focusY, core * 5.2);
    aura.addColorStop(0, `rgba(245, 253, 255, ${0.5 + bass * 0.3})`);
    aura.addColorStop(0.12, `rgba(104, 235, 255, ${0.2 + presence * 0.28})`);
    aura.addColorStop(0.46, `rgba(150, 86, 255, ${0.12 + this.bloom * 0.14})`);
    aura.addColorStop(1, "rgba(50, 20, 120, 0)");
    context.fillStyle = aura;
    context.beginPath();
    context.arc(focusX, focusY, core * 5.2, 0, Math.PI * 2);
    context.fill();

    for (let ring = 0; ring < 4; ring += 1) {
      const radius = core * (1.2 + ring * 0.82 + this.bloom * (0.82 + ring * 0.16) + beat * 0.45);
      context.beginPath();
      for (let point = 0; point <= this.frameBands.length; point += 1) {
        const bandIndex = point % this.frameBands.length;
        const band = this.frameBands[bandIndex] ?? 0;
        const angle = point / this.frameBands.length * Math.PI * 2 + this.rotation * (ring % 2 ? -1 : 1);
        const petal = 1 + band * (0.18 + ring * 0.035) + Math.sin(angle * 6 + time + ring) * 0.025;
        const x = focusX + Math.cos(angle) * radius * petal;
        const y = focusY + Math.sin(angle) * radius * petal * 0.86;
        if (point === 0) context.moveTo(x, y); else context.lineTo(x, y);
      }
      context.closePath();
      context.strokeStyle = ring % 2
        ? `rgba(184, 130, 255, ${0.2 + presence * 0.34})`
        : `rgba(111, 235, 255, ${0.22 + air * 0.42})`;
      context.lineWidth = 0.8 + ring * 0.45 + bass * 1.2 + beat * 0.75;
      context.shadowColor = context.strokeStyle;
      context.shadowBlur = 9 + bass * 18;
      context.stroke();
    }

    context.fillStyle = `rgba(246, 253, 255, ${0.72 + bass * 0.25})`;
    context.shadowColor = "rgba(125, 235, 255, .9)";
    context.shadowBlur = 22 + this.bloom * 34;
    context.beginPath();
    context.arc(focusX, focusY, Math.max(2.4, core * 0.16), 0, Math.PI * 2);
    context.fill();
    context.restore();
  }

  private emitReactiveBursts(focusX: number, focusY: number, frame: MusicLyricFrame) {
    if (frame.beatTrigger) {
      const pulse = this.pulses[this.pulseCursor % this.pulses.length];
      this.pulseCursor += 1;
      pulse.x = focusX;
      pulse.y = focusY;
      pulse.age = 0;
      pulse.duration = 0.68 + frame.beat * 0.5;
      pulse.strength = Math.max(0.35, frame.beat);
    }

    if (frame.transient < 0.22 || frame.time - this.lastTransientBurstAt < 0.095) return;
    this.lastTransientBurstAt = frame.time;

    const count = 4 + Math.floor(frame.transient * 11 + frame.air * 5 + frame.beat * 3);
    for (let index = 0; index < count; index += 1) {
      const spark = this.sparks[this.sparkCursor % this.sparks.length];
      this.sparkCursor += 1;
      const seed = seeded(this.sparkCursor * 13 + index * 7);
      const angle = seed * Math.PI * 2;
      const speed = 34 + seeded(this.sparkCursor * 17 + index) * (115 + frame.transient * 130);
      spark.x = focusX;
      spark.y = focusY;
      spark.vx = Math.cos(angle) * speed;
      spark.vy = Math.sin(angle) * speed * 0.78;
      spark.age = 0;
      spark.duration = 0.34 + seeded(this.sparkCursor * 23 + index) * 0.62;
      spark.hue = 185 + frame.centroid * 105 + (seeded(this.sparkCursor + index) - 0.5) * 34;
    }
  }

  private drawPulsesAndSparks(context: CanvasRenderingContext2D, frame: MusicLyricFrame) {
    context.save();
    context.globalCompositeOperation = "lighter";
    for (const pulse of this.pulses) {
      if (pulse.age >= pulse.duration) continue;
      pulse.age += frame.delta;
      const progress = Math.min(1, pulse.age / pulse.duration);
      const eased = 1 - Math.pow(1 - progress, 3);
      const radius = (18 + pulse.strength * 125) * eased;
      context.globalAlpha = (1 - progress) * (0.32 + pulse.strength * 0.38);
      context.strokeStyle = pulse.strength > 0.58 ? "#e1a3ff" : "#82efff";
      context.lineWidth = 0.7 + (1 - progress) * pulse.strength * 2.4;
      context.shadowColor = context.strokeStyle;
      context.shadowBlur = 10 + pulse.strength * 22;
      context.beginPath();
      context.ellipse(pulse.x, pulse.y, radius * 1.35, radius, this.rotation * 0.2, 0, Math.PI * 2);
      context.stroke();
    }
    for (const spark of this.sparks) {
      if (spark.age >= spark.duration) continue;
      spark.age += frame.delta;
      const progress = Math.min(1, spark.age / spark.duration);
      const previousX = spark.x;
      const previousY = spark.y;
      spark.vx *= Math.exp(-frame.delta * 2.1);
      spark.vy *= Math.exp(-frame.delta * 2.1);
      spark.x += spark.vx * frame.delta;
      spark.y += spark.vy * frame.delta;
      context.globalAlpha = Math.pow(1 - progress, 1.4) * 0.78;
      context.strokeStyle = `hsl(${spark.hue} 92% 76%)`;
      context.lineWidth = 0.65 + (1 - progress) * 1.35;
      context.shadowColor = context.strokeStyle;
      context.shadowBlur = 8;
      context.beginPath();
      context.moveTo(previousX, previousY);
      context.lineTo(spark.x, spark.y);
      context.stroke();
    }
    context.restore();
  }

  private drawWave(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    timeData: Uint8Array | undefined,
    energy: number,
    air: number,
    time: number,
    hasSignal: boolean,
  ) {
    const baseline = height * 0.78;
    context.save();
    context.globalCompositeOperation = "lighter";
    const gradient = context.createLinearGradient(0, 0, width, 0);
    gradient.addColorStop(0, "rgba(102,230,255,0)");
    gradient.addColorStop(0.24, `rgba(102,230,255,${0.2 + air * 0.4})`);
    gradient.addColorStop(0.72, `rgba(255,130,193,${0.18 + energy * 0.36})`);
    gradient.addColorStop(1, "rgba(255,130,193,0)");
    context.strokeStyle = gradient;
    context.lineWidth = 1.1 + air * 2.4;
    context.shadowColor = "rgba(92, 224, 255, .5)";
    context.shadowBlur = 10;
    context.beginPath();
    const points = Math.min(180, Math.max(64, timeData?.length ?? 128));
    for (let point = 0; point < points; point += 1) {
      const fraction = point / Math.max(1, points - 1);
      const sourceIndex = Math.floor(fraction * Math.max(0, (timeData?.length ?? points) - 1));
      const sample = hasSignal && timeData
        ? ((timeData[sourceIndex] ?? 128) - 128) / 128
        : Math.sin(time * 1.2 + point * 0.22) * 0.12 + Math.sin(time * 0.45 + point * 0.07) * 0.06;
      const x = fraction * width;
      const y = baseline + sample * height * (0.075 + energy * 0.08);
      if (point === 0) context.moveTo(x, y); else context.lineTo(x, y);
    }
    context.stroke();
    context.restore();
  }

  private drawJourney(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    focusX: number,
    focusY: number,
    progress: number,
    energy: number,
    time: number,
  ) {
    const startX = width * 0.12;
    const endX = width * 0.88;
    const y = height * 0.89;
    context.save();
    context.strokeStyle = "rgba(168, 214, 242, .15)";
    context.lineWidth = 1;
    context.beginPath();
    context.moveTo(startX, y);
    context.bezierCurveTo(width * 0.35, y - height * 0.025, width * 0.63, y + height * 0.022, endX, y);
    context.stroke();
    const markerX = startX + (endX - startX) * progress;
    const markerY = y + Math.sin(progress * Math.PI * 2 - time * 0.08) * height * 0.008;
    context.fillStyle = `rgba(120, 235, 255, ${0.5 + energy * 0.4})`;
    context.shadowColor = "rgba(120,235,255,.75)";
    context.shadowBlur = 12;
    context.beginPath();
    context.arc(markerX, markerY, 2 + energy * 3, 0, Math.PI * 2);
    context.fill();
    context.globalAlpha = 0.18;
    context.beginPath();
    context.moveTo(markerX, markerY);
    context.lineTo(focusX, focusY);
    context.strokeStyle = "#a8efff";
    context.stroke();
    context.restore();
  }
}

function seeded(seed: number): number {
  const value = Math.sin((seed + 11) * 91.731) * 43_758.5453;
  return value - Math.floor(value);
}
