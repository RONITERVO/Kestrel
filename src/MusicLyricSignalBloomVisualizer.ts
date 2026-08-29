/* SPDX-FileCopyrightText: 2026 Roni Tervo
 * SPDX-License-Identifier: Apache-2.0
 *
 * An original Kestrel visual lyric renderer. It is dependency-free, bounded, and shares the
 * producer's single Web Audio analyser with every other visual theme.
 */

interface LightSeed {
  angle: number;
  distance: number;
  drift: number;
  size: number;
  phase: number;
  warmth: number;
}

export class SignalBloomMusicLyricVisualizer {
  private readonly context: CanvasRenderingContext2D;
  private readonly bands = new Float32Array(48);
  private readonly lights: LightSeed[] = Array.from({ length: 112 }, (_, index) => ({
    angle: seeded(index * 6 + 1) * Math.PI * 2,
    distance: 0.08 + seeded(index * 6 + 2) * 0.92,
    drift: (seeded(index * 6 + 3) - 0.5) * 0.16,
    size: 0.45 + seeded(index * 6 + 4) * 2.1,
    phase: seeded(index * 6 + 5) * Math.PI * 2,
    warmth: seeded(index * 6 + 6),
  }));
  private lastTime = performance.now();
  private bassEnvelope = 0;
  private bloom = 0;
  private rotation = 0;

  constructor(private readonly canvas: HTMLCanvasElement) {
    const context = canvas.getContext("2d");
    if (!context) throw new Error("The Signal bloom lyric canvas is unavailable.");
    this.context = context;
  }

  draw(
    analyser: AnalyserNode | undefined,
    frequency: Uint8Array | undefined,
    timeData: Uint8Array | undefined,
    progress: number,
  ) {
    this.resize();
    const now = performance.now();
    const delta = Math.min(0.08, Math.max(0, (now - this.lastTime) / 1_000));
    this.lastTime = now;
    const time = now / 1_000;
    const hasSignal = Boolean(analyser && frequency && timeData);
    if (analyser && frequency && timeData) {
      analyser.getByteFrequencyData(frequency as Uint8Array<ArrayBuffer>);
      analyser.getByteTimeDomainData(timeData as Uint8Array<ArrayBuffer>);
    }

    const energy = hasSignal && frequency
      ? average(frequency, 2, Math.floor(frequency.length * 0.76))
      : idle(time, 0.13);
    const bass = hasSignal && frequency
      ? average(frequency, 0, Math.min(18, frequency.length))
      : idle(time + 1.7, 0.16);
    const presence = hasSignal && frequency
      ? average(frequency, Math.min(18, frequency.length - 1), Math.min(58, frequency.length))
      : idle(time + 3.1, 0.1);
    const air = hasSignal && frequency
      ? average(frequency, Math.floor(frequency.length * 0.55), frequency.length)
      : idle(time + 4.7, 0.07);

    const previousBass = this.bassEnvelope;
    const envelopeRate = bass > previousBass ? 18 : 4.2;
    this.bassEnvelope += (bass - previousBass) * Math.min(1, envelopeRate * delta);
    const onset = Math.max(0, bass - previousBass - 0.018);
    this.bloom = Math.max(this.bloom * Math.exp(-delta * 3.8), Math.min(1, onset * 15));
    this.rotation += delta * (0.045 + presence * 0.16);

    for (let index = 0; index < this.bands.length; index += 1) {
      const normalized = index / Math.max(1, this.bands.length - 1);
      const sourceIndex = Math.min(
        (frequency?.length ?? 1) - 1,
        Math.floor(Math.pow(normalized, 1.48) * Math.max(0, (frequency?.length ?? 1) - 1)),
      );
      const target = hasSignal && frequency
        ? (frequency[sourceIndex] ?? 0) / 255
        : 0.055 + Math.pow(Math.sin(time * 0.42 + index * 0.21) * 0.5 + 0.5, 3) * 0.11;
      this.bands[index] += (target - this.bands[index]) * Math.min(1, delta * (target > this.bands[index] ? 17 : 5));
    }

    const ratio = window.devicePixelRatio || 1;
    const width = this.canvas.width / ratio;
    const height = this.canvas.height / ratio;
    const safeProgress = Math.max(0, Math.min(1, progress));
    const focusX = width * (0.18 + safeProgress * 0.64);
    const focusY = height * (0.47 + Math.sin(safeProgress * Math.PI * 2.4) * 0.055);
    const context = this.context;
    context.clearRect(0, 0, this.canvas.width, this.canvas.height);
    context.save();
    context.scale(ratio, ratio);
    this.drawNight(context, width, height, focusX, focusY, energy, time);
    this.drawConstellation(context, width, height, focusX, focusY, energy, air, time);
    this.drawRibbons(context, width, height, focusX, focusY, presence, time);
    this.drawBloom(context, width, height, focusX, focusY, bass, presence, air, time);
    this.drawWave(context, width, height, timeData, energy, air, time, hasSignal);
    this.drawJourney(context, width, height, focusX, focusY, safeProgress, energy, time);
    context.restore();
  }

  private resize() {
    const rectangle = this.canvas.getBoundingClientRect();
    const ratio = window.devicePixelRatio || 1;
    const width = Math.max(1, Math.round(rectangle.width * ratio));
    const height = Math.max(1, Math.round(rectangle.height * ratio));
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
    }
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
      const band = this.bands[index % this.bands.length] ?? 0;
      const angle = light.angle + this.rotation * (0.45 + light.drift) + Math.sin(time * 0.2 + light.phase) * 0.04;
      const distance = radius * light.distance * (1 + band * 0.24 + this.bloom * 0.08);
      const x = focusX + Math.cos(angle) * distance * (1.55 + light.drift);
      const y = focusY + Math.sin(angle) * distance * 0.82;
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
        context.globalAlpha = 0.82 - echo * 0.23;
        context.lineWidth = 1.2 + echo * 2.4 + presence * 3.2;
        context.beginPath();
        for (let point = 0; point <= 96; point += 1) {
          const fraction = point / 96;
          const x = fraction * width;
          const bandIndex = Math.min(this.bands.length - 1, Math.floor(fraction * this.bands.length));
          const band = this.bands[(bandIndex + ribbon * 7) % this.bands.length] ?? 0;
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
    presence: number,
    air: number,
    time: number,
  ) {
    const smaller = Math.min(width, height);
    const core = smaller * (0.055 + bass * 0.045 + this.bloom * 0.018);
    context.save();
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
      const radius = core * (1.2 + ring * 0.82 + this.bloom * (1.1 + ring * 0.2));
      context.beginPath();
      for (let point = 0; point <= this.bands.length; point += 1) {
        const bandIndex = point % this.bands.length;
        const band = this.bands[bandIndex] ?? 0;
        const angle = point / this.bands.length * Math.PI * 2 + this.rotation * (ring % 2 ? -1 : 1);
        const petal = 1 + band * (0.18 + ring * 0.035) + Math.sin(angle * 6 + time + ring) * 0.025;
        const x = focusX + Math.cos(angle) * radius * petal;
        const y = focusY + Math.sin(angle) * radius * petal * 0.86;
        if (point === 0) context.moveTo(x, y); else context.lineTo(x, y);
      }
      context.closePath();
      context.strokeStyle = ring % 2
        ? `rgba(184, 130, 255, ${0.2 + presence * 0.34})`
        : `rgba(111, 235, 255, ${0.22 + air * 0.42})`;
      context.lineWidth = 0.8 + ring * 0.45 + bass * 1.5;
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

function average(values: Uint8Array, start: number, end: number): number {
  const safeStart = Math.max(0, Math.min(values.length, start));
  const safeEnd = Math.max(safeStart + 1, Math.min(values.length, end));
  let total = 0;
  for (let index = safeStart; index < safeEnd; index += 1) total += values[index] ?? 0;
  return total / Math.max(1, safeEnd - safeStart) / 255;
}

function idle(time: number, floor: number): number {
  return floor + Math.sin(time * 0.61) * 0.018 + Math.sin(time * 0.19 + 1.2) * 0.014;
}

function seeded(seed: number): number {
  const value = Math.sin((seed + 11) * 91.731) * 43_758.5453;
  return value - Math.floor(value);
}
