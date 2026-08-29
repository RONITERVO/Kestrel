/* SPDX-FileCopyrightText: 2026 Roni Tervo
 * SPDX-License-Identifier: Apache-2.0
 *
 * Adapted for Kestrel from the audio-reactive sketchbook renderer in Visual Music Lyrics.
 */

interface RainDrop {
  x: number;
  y: number;
  speed: number;
  length: number;
}

export class SketchbookMusicLyricVisualizer {
  private readonly context: CanvasRenderingContext2D;
  private readonly seeds = Array.from({ length: 128 }, (_, index) => pseudoRandom(index + 7));
  private readonly bins = new Float32Array(32);
  private readonly drops: RainDrop[] = [];
  private lastTime = performance.now();
  private scrollX = 0;

  constructor(private readonly canvas: HTMLCanvasElement) {
    const context = canvas.getContext("2d");
    if (!context) throw new Error("The visual lyric canvas is unavailable.");
    this.context = context;
  }

  draw(analyser: AnalyserNode | undefined, frequency: Uint8Array | undefined, timeData: Uint8Array | undefined, progress: number) {
    this.resize();
    const now = performance.now();
    const delta = Math.min(0.1, Math.max(0, (now - this.lastTime) / 1_000));
    this.lastTime = now;
    const time = now / 1_000;
    if (analyser && frequency && timeData) {
      analyser.getByteFrequencyData(frequency as Uint8Array<ArrayBuffer>);
      analyser.getByteTimeDomainData(timeData as Uint8Array<ArrayBuffer>);
    }
    const energy = frequency ? averageEnergy(frequency, 3, Math.floor(frequency.length * 0.72)) : idleEnergy(time);
    const bass = frequency ? averageEnergy(frequency, 0, Math.min(18, frequency.length)) : idleEnergy(time + 2) * 0.7;
    const treble = frequency ? averageEnergy(frequency, Math.min(40, frequency.length - 1), frequency.length) : idleEnergy(time + 4) * 0.5;
    for (let index = 0; index < this.bins.length; index += 1) {
      const target = frequency ? (frequency[index] ?? 0) / 255 : Math.abs(Math.sin(time + index * 0.2)) * 0.2;
      this.bins[index] += (target - this.bins[index]) * Math.min(1, 15 * delta);
    }
    this.scrollX += delta * (20 + energy * 60);

    const ratio = window.devicePixelRatio || 1;
    const width = this.canvas.width / ratio;
    const height = this.canvas.height / ratio;
    const horizon = height * 0.56;
    const sunX = width * (0.3 + Math.max(0, Math.min(1, progress)) * 0.4);
    const sunY = horizon - height * 0.28;
    const context = this.context;
    context.clearRect(0, 0, this.canvas.width, this.canvas.height);
    context.save();
    context.scale(ratio, ratio);
    this.drawPaper(context, width, height);
    this.drawSky(context, width, horizon);
    this.drawSun(context, width, height, time, energy, bass, sunX, sunY);
    this.drawTerrain(context, width, height, horizon, time);
    this.drawWater(context, width, height, horizon, time, timeData, treble, energy, sunX);
    this.drawClouds(context, width, height, time, energy, sunX);
    this.drawRain(context, width, height, horizon, energy, delta, time);
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

  private jitter(x: number, y: number, amount: number, time: number) {
    const frame = Math.floor(time * 12) % 3;
    const hash = Math.sin(x * 12.9898 + y * 78.233 + frame * 13.131) * 43_758.5453;
    return (hash - Math.floor(hash) - 0.5) * amount;
  }

  private terrainNoise(value: number) {
    const rolling = Math.sin(value * 0.005) * 0.5 + 0.5;
    const detail = Math.sin(value * 0.012) * 0.25 + Math.sin(value * 0.03) * 0.125;
    return (rolling + detail) * 0.7 + Math.abs(Math.sin(value * 0.008)) * 0.3;
  }

  private drawPaper(context: CanvasRenderingContext2D, width: number, height: number) {
    context.fillStyle = "#f4eee1";
    context.fillRect(0, 0, width, height);
    context.globalAlpha = 0.055;
    context.fillStyle = "#4b423d";
    for (let index = 0; index < 420; index += 1) {
      const seed = this.seeds[index % this.seeds.length];
      const x = (seed * width * 17 + index * 37) % width;
      const y = (seed * height * 23 + index * 61) % height;
      context.fillRect(x, y, 0.7, 0.7);
    }
    context.globalAlpha = 1;
  }

  private drawSky(context: CanvasRenderingContext2D, width: number, horizon: number) {
    context.save();
    context.globalCompositeOperation = "multiply";
    const gradient = context.createLinearGradient(0, 0, 0, horizon);
    gradient.addColorStop(0, "rgba(200, 210, 220, .42)");
    gradient.addColorStop(1, "rgba(240, 230, 220, .08)");
    context.fillStyle = gradient;
    context.fillRect(0, 0, width, horizon);
    context.restore();
  }

  private drawSun(context: CanvasRenderingContext2D, width: number, height: number, time: number, energy: number, bass: number, centerX: number, centerY: number) {
    const smaller = Math.min(width, height);
    const radius = smaller * (0.1 + bass * 0.025);
    context.save();
    context.globalCompositeOperation = "multiply";
    const glow = context.createRadialGradient(centerX, centerY, radius * 0.4, centerX, centerY, radius * 3);
    glow.addColorStop(0, "rgba(235, 150, 45, .24)");
    glow.addColorStop(1, "rgba(255,255,255,0)");
    context.fillStyle = glow;
    context.beginPath();
    context.arc(centerX, centerY, radius * 3, 0, Math.PI * 2);
    context.fill();
    context.globalCompositeOperation = "source-over";
    context.strokeStyle = "rgba(219, 126, 26, .62)";
    context.lineCap = "round";
    for (let ray = 0; ray < 26; ray += 1) {
      const angle = ray / 26 * Math.PI * 2 + Math.sin(time * 0.6) * 0.03;
      const inner = radius * 1.1;
      const length = 8 + bass * 36 * this.seeds[ray];
      context.lineWidth = 1.2 + bass * 1.8;
      context.beginPath();
      context.moveTo(centerX + Math.cos(angle) * inner, centerY + Math.sin(angle) * inner);
      context.lineTo(centerX + Math.cos(angle) * (inner + length), centerY + Math.sin(angle) * (inner + length));
      context.stroke();
    }
    for (let pass = 0; pass < 3; pass += 1) {
      context.beginPath();
      for (let point = 0; point <= 120; point += 1) {
        const angle = point / 120 * Math.PI * 2;
        const wobble = Math.sin(time * 1.6 + point * 0.19 + pass) * smaller * 0.005;
        const lift = energy * smaller * 0.025 * this.seeds[(point + pass * 17) % this.seeds.length];
        const lineRadius = radius + wobble + lift + pass * 2;
        const x = centerX + Math.cos(angle) * lineRadius;
        const y = centerY + Math.sin(angle) * lineRadius * 0.9;
        if (point === 0) context.moveTo(x, y); else context.lineTo(x, y);
      }
      context.closePath();
      context.strokeStyle = pass === 2 ? "rgba(35,30,28,.75)" : "rgba(219,126,26,.7)";
      context.lineWidth = 1 + pass * 0.6;
      context.stroke();
    }
    context.restore();
  }

  private drawTerrain(context: CanvasRenderingContext2D, width: number, height: number, horizon: number, time: number) {
    const points = 64;
    const coordinates: Array<{ x: number; y: number }> = [];
    context.save();
    context.beginPath();
    context.moveTo(0, horizon);
    for (let index = 0; index < points; index += 1) {
      const x = index * width / (points - 1);
      const y = horizon - this.terrainNoise(x + this.scrollX * 0.5) * height * 0.14;
      coordinates.push({ x, y });
      context.lineTo(x, y);
    }
    context.lineTo(width, horizon);
    context.closePath();
    context.fillStyle = "rgba(84, 94, 92, .22)";
    context.fill();
    context.strokeStyle = "rgba(35,30,28,.5)";
    context.lineWidth = 1.5;
    context.beginPath();
    coordinates.forEach((point, index) => {
      const x = point.x + this.jitter(point.x, point.y, 2, time);
      const y = point.y + this.jitter(point.y, point.x, 2, time);
      if (index === 0) context.moveTo(x, y); else context.lineTo(x, y);
    });
    context.stroke();
    context.restore();
  }

  private drawWater(context: CanvasRenderingContext2D, width: number, height: number, horizon: number, time: number, timeData: Uint8Array | undefined, treble: number, energy: number, sunX: number) {
    context.save();
    context.globalCompositeOperation = "multiply";
    context.fillStyle = "rgba(110, 140, 160, .27)";
    context.fillRect(0, horizon, width, height - horizon);
    context.globalCompositeOperation = "source-over";
    for (let y = horizon; y < height; y += 7) {
      const depth = (y - horizon) / Math.max(1, height - horizon);
      const reflectionWidth = width * (0.05 + depth * 0.16);
      context.fillStyle = `rgba(226, 143, 38, ${0.23 * (1 - depth) * (0.5 + energy * 0.5)})`;
      context.fillRect(sunX - reflectionWidth / 2 + Math.sin(y * 0.1 + time * 4) * depth * 12, y, reflectionWidth, 2);
    }
    context.strokeStyle = "rgba(35,30,28,.48)";
    context.lineWidth = 1 + treble * 1.7;
    for (let wave = 0; wave < 6; wave += 1) {
      const depth = (wave + time * 0.45 % 1) / 6;
      const y = horizon + Math.pow(depth, 1.5) * (height - horizon);
      const waveWidth = width * (0.32 + depth * 0.68);
      const left = (width - waveWidth) / 2;
      context.beginPath();
      for (let point = 0; point < 64; point += 1) {
        const x = left + waveWidth * point / 63;
        const sampleIndex = Math.floor((point / 64 + depth) * (timeData?.length ?? 64)) % (timeData?.length ?? 64);
        const sample = timeData ? ((timeData[sampleIndex] ?? 128) - 128) / 128 : Math.sin(time * 2 + point * 0.35 + wave) * 0.15;
        const offset = sample * height * 0.075 * depth + Math.sin(point * 0.62 + time * 2.4 + wave) * (1 + depth * 2);
        if (point === 0) context.moveTo(x, y + offset); else context.lineTo(x, y + offset);
      }
      context.stroke();
    }
    context.restore();
  }

  private drawClouds(context: CanvasRenderingContext2D, width: number, height: number, time: number, energy: number, sunX: number) {
    context.save();
    context.globalCompositeOperation = "multiply";
    context.filter = "blur(12px)";
    for (let index = 0; index < 12; index += 1) {
      const value = this.bins[index * 2] ?? 0;
      if (value < 0.03) continue;
      const x = (index + 0.5) * width / 12;
      const warmth = Math.max(0, 1 - Math.abs(x - sunX) / (width * 0.42));
      const hue = 210 - warmth * 170;
      const lightness = 95 - energy * 40 + warmth * 12;
      context.fillStyle = `hsla(${hue}, ${10 + warmth * 40}%, ${lightness}%, ${value * 0.58})`;
      context.beginPath();
      context.ellipse(x, -4 + value * height * 0.13 + Math.sin(time + index) * 8, width * (0.09 + value * 0.08), height * (0.035 + value * 0.05), 0, 0, Math.PI * 2);
      context.fill();
    }
    context.restore();
  }

  private drawRain(context: CanvasRenderingContext2D, width: number, height: number, horizon: number, energy: number, delta: number, time: number) {
    if (energy > 0.36) {
      const count = Math.floor((energy - 0.3) * 7);
      for (let index = 0; index < count; index += 1) {
        const depth = 0.6 + Math.random() * 1.2;
        this.drops.push({ x: Math.random() * width, y: -20, speed: 520 * depth + energy * 300, length: 9 * depth + Math.random() * 12 });
      }
    }
    context.save();
    context.strokeStyle = "rgba(24,75,165,.26)";
    context.lineCap = "round";
    for (let index = this.drops.length - 1; index >= 0; index -= 1) {
      const drop = this.drops[index];
      drop.y += drop.speed * delta;
      drop.x += drop.speed * 0.025 * energy * delta;
      context.lineWidth = 0.7 + drop.length / 18;
      context.beginPath();
      context.moveTo(drop.x + this.jitter(drop.x, drop.y, 1.5, time), drop.y);
      context.lineTo(drop.x + 3, drop.y + drop.length);
      context.stroke();
      if (drop.y > height || (drop.y > horizon && Math.random() > 0.82)) this.drops.splice(index, 1);
    }
    if (this.drops.length > 1_200) this.drops.splice(0, this.drops.length - 1_200);
    context.restore();
  }
}

function averageEnergy(values: Uint8Array, start: number, end: number): number {
  const safeStart = Math.max(0, Math.min(values.length, start));
  const safeEnd = Math.max(safeStart + 1, Math.min(values.length, end));
  let total = 0;
  for (let index = safeStart; index < safeEnd; index += 1) total += values[index] ?? 0;
  return total / (safeEnd - safeStart) / 255;
}

function idleEnergy(time: number): number {
  return 0.12 + Math.sin(time * 0.7) * 0.025 + Math.sin(time * 0.23) * 0.018;
}

function pseudoRandom(seed: number): number {
  const value = Math.sin(seed * 12.9898) * 43_758.5453;
  return value - Math.floor(value);
}
