/* SPDX-FileCopyrightText: 2026 Roni Tervo
 * SPDX-License-Identifier: Apache-2.0
 *
 * Adapted for Kestrel from the audio-reactive sketchbook renderer in Visual Music Lyrics.
 * Kestrel keeps every transient collection bounded and drives event timing from the shared,
 * deterministic reactivity frame instead of unbounded frame-rate-dependent randomness.
 */

import type { MusicLyricBounds, MusicLyricFrame } from "./MusicLyricReactivity";

interface RainDrop {
  x: number;
  y: number;
  speed: number;
  length: number;
  depth: number;
  background: boolean;
}

interface Splat {
  x: number;
  y: number;
  age: number;
  duration: number;
  strength: number;
}

interface Bird {
  x: number;
  y: number;
  vx: number;
  vy: number;
  flap: number;
  depth: number;
}

interface Fish {
  x: number;
  y: number;
  vx: number;
  vy: number;
}

const MAX_DROPS = 420;
const MAX_SPLATS = 96;
const MAX_BIRDS = 5;

export class SketchbookMusicLyricVisualizer {
  private readonly context: CanvasRenderingContext2D;
  private readonly seeds = Array.from({ length: 256 }, (_, index) => pseudoRandom(index + 7));
  private readonly drops: RainDrop[] = [];
  private readonly splats: Splat[] = [];
  private readonly birds: Bird[] = [];
  private readonly terrainCoordinates = [new Float32Array(144), new Float32Array(144)];
  private scrollX = 0;
  private rainBudget = 0;
  private nextBirdAt = 0;
  private nextFishAt = 0;
  private eventIndex = 0;
  private fish?: Fish;

  constructor(private readonly canvas: HTMLCanvasElement) {
    const context = canvas.getContext("2d");
    if (!context) throw new Error("The Living sketchbook lyric canvas is unavailable.");
    this.context = context;
  }

  draw(frame: MusicLyricFrame) {
    this.resize();
    const ratio = lyricPixelRatio();
    const width = this.canvas.width / ratio;
    const height = this.canvas.height / ratio;
    const horizon = clamp(frame.layout.horizon || height * 0.56, height * 0.47, height * 0.69);
    const sunX = width * (0.3 + frame.progress * 0.4);
    const sunY = horizon - height * (0.245 + frame.energy * 0.035);
    this.scrollX += frame.delta * (17 + frame.energy * 58 + frame.lowMid * 28);

    const context = this.context;
    context.clearRect(0, 0, this.canvas.width, this.canvas.height);
    context.save();
    context.scale(ratio, ratio);
    this.drawPaper(context, width, height, frame);
    this.drawSky(context, width, horizon, frame);
    this.drawSun(context, width, height, sunX, sunY, frame);
    this.drawTerrain(context, width, height, horizon, frame);
    this.drawWater(context, width, height, horizon, sunX, frame);
    this.drawClouds(context, width, height, sunX, frame);
    this.drawWindNotation(context, width, height, horizon, frame);
    this.processWildlife(context, width, height, horizon, frame);
    this.processWeather(context, width, height, horizon, frame.layout.translation, frame);
    context.restore();
  }

  destroy() {
    this.drops.length = 0;
    this.splats.length = 0;
    this.birds.length = 0;
    this.fish = undefined;
  }

  private resize() {
    const rectangle = this.canvas.getBoundingClientRect();
    const ratio = lyricPixelRatio();
    const width = Math.max(1, Math.round(rectangle.width * ratio));
    const height = Math.max(1, Math.round(rectangle.height * ratio));
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
    }
  }

  private drawPaper(context: CanvasRenderingContext2D, width: number, height: number, frame: MusicLyricFrame) {
    context.fillStyle = "#f4eee1";
    context.fillRect(0, 0, width, height);
    context.save();
    context.globalAlpha = 0.045 + frame.air * 0.045;
    context.fillStyle = "#4b423d";
    for (let index = 0; index < 520; index += 1) {
      const seed = this.seeds[index % this.seeds.length];
      const x = (seed * width * 17 + index * 37) % width;
      const y = (seed * height * 23 + index * 61) % height;
      const size = 0.45 + this.seeds[(index + 83) % this.seeds.length] * (0.7 + frame.air);
      context.fillRect(x, y, size, size * 0.55);
    }
    context.restore();
  }

  private drawSky(context: CanvasRenderingContext2D, width: number, horizon: number, frame: MusicLyricFrame) {
    context.save();
    context.globalCompositeOperation = "multiply";
    const gradient = context.createLinearGradient(0, 0, 0, horizon);
    gradient.addColorStop(0, `hsla(${210 - frame.centroid * 24}, ${15 + frame.presence * 18}%, ${85 - frame.energy * 17}%, ${0.29 + frame.energy * 0.2})`);
    gradient.addColorStop(0.6, `rgba(204, 213, 218, ${0.12 + frame.lowMid * 0.12})`);
    gradient.addColorStop(1, "rgba(240,230,220,.05)");
    context.fillStyle = gradient;
    context.fillRect(0, 0, width, horizon);
    context.restore();
  }

  private drawSun(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    centerX: number,
    centerY: number,
    frame: MusicLyricFrame,
  ) {
    const smaller = Math.min(width, height);
    const radius = smaller * (0.105 + frame.bass * 0.03 + frame.transient * 0.007 + frame.beat * 0.018);
    const lift = smaller * (0.055 + frame.energy * 0.09);
    context.save();
    context.globalCompositeOperation = "multiply";
    const glow = context.createRadialGradient(centerX, centerY, radius * 0.35, centerX, centerY, radius * 3.4);
    glow.addColorStop(0, `rgba(235,150,45,${0.19 + frame.bass * 0.14 + frame.beat * 0.09})`);
    glow.addColorStop(0.46, `rgba(235,150,45,${0.075 + frame.transient * 0.06 + frame.beat * 0.07})`);
    glow.addColorStop(1, "rgba(255,255,255,0)");
    context.fillStyle = glow;
    context.beginPath();
    context.arc(centerX, centerY, radius * 3.4, 0, Math.PI * 2);
    context.fill();

    context.globalCompositeOperation = "source-over";
    context.lineCap = "round";
    context.lineJoin = "round";
    context.strokeStyle = "rgba(219,126,26,.68)";
    context.globalAlpha = 0.38 + frame.bass * 0.52;
    for (let ray = 0; ray < 30; ray += 1) {
      const angle = ray / 30 * Math.PI * 2 + Math.sin(frame.time * 0.6) * 0.03;
      const inner = radius * 1.12 + Math.sin(frame.time * 2 + ray) * 2.5;
      const length = 8 + frame.bass * 34 * this.seeds[ray] + frame.transient * 7 + frame.beat * 25;
      context.lineWidth = 1.1 + frame.bass * 1.8 + frame.beat * 0.9;
      context.beginPath();
      context.moveTo(centerX + Math.cos(angle) * inner, centerY + Math.sin(angle) * inner * 0.9);
      context.lineTo(
        centerX + Math.cos(angle) * (inner + length) + this.jitter(centerX, centerY, 4, frame.time),
        centerY + Math.sin(angle) * (inner + length) * 0.9 + this.jitter(centerY, centerX, 4, frame.time),
      );
      context.stroke();
    }

    for (let pass = 0; pass < 4; pass += 1) {
      context.beginPath();
      for (let point = 0; point <= 156; point += 1) {
        const fraction = point / 156;
        const angle = fraction * Math.PI * 2;
        const band = frame.bands[Math.floor(fraction * (frame.bands.length - 1))] ?? 0;
        const seed = this.seeds[(point + pass * 31) % this.seeds.length];
        const wobble = Math.sin(frame.time * (1.35 + seed) + point * 0.19 + pass) * smaller * 0.0055;
        const rough = Math.sin(point * 0.53 + seed * 9 + frame.time * 0.34) * smaller * 0.0038;
        const lineRadius = radius + band * lift + wobble + rough + pass * smaller * 0.003;
        const x = centerX + Math.cos(angle) * lineRadius;
        const y = centerY + Math.sin(angle) * lineRadius * 0.9;
        if (point === 0) context.moveTo(x, y); else context.lineTo(x, y);
      }
      context.closePath();
      context.strokeStyle = pass === 3 ? "rgba(35,30,28,.72)" : "rgba(219,126,26,.7)";
      context.globalAlpha = pass === 3 ? 0.7 : 0.62 + frame.energy * 0.2;
      context.lineWidth = 0.9 + pass * 0.48 + frame.transient * 0.45 + frame.beat * 0.55;
      context.stroke();
    }
    context.restore();
  }

  private drawTerrain(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    horizon: number,
    frame: MusicLyricFrame,
  ) {
    context.save();
    for (let layer = 0; layer < 2; layer += 1) {
      const points = 72;
      const coordinates = this.terrainCoordinates[layer];
      const parallax = layer === 0 ? 0.24 : 0.56;
      const heightScale = layer === 0 ? 0.105 : 0.155 + frame.lowMid * 0.025;
      context.beginPath();
      context.moveTo(0, horizon);
      for (let index = 0; index < points; index += 1) {
        const x = index * width / (points - 1);
        const y = horizon - this.terrainNoise(x + this.scrollX * parallax, layer) * height * heightScale;
        coordinates[index * 2] = x;
        coordinates[index * 2 + 1] = y;
        context.lineTo(x, y);
      }
      context.lineTo(width, horizon);
      context.closePath();
      context.globalCompositeOperation = "multiply";
      context.fillStyle = layer === 0
        ? `rgba(78,88,91,${0.08 + frame.lowMid * 0.05})`
        : `rgba(62,76,78,${0.15 + frame.lowMid * 0.08})`;
      context.fill();
      context.globalCompositeOperation = "source-over";
      context.strokeStyle = layer === 0 ? "rgba(35,30,28,.24)" : "rgba(35,30,28,.48)";
      context.lineWidth = layer === 0 ? 1 : 1.45;
      context.beginPath();
      for (let index = 0; index < points; index += 1) {
        const baseX = coordinates[index * 2];
        const baseY = coordinates[index * 2 + 1];
        const x = baseX + this.jitter(baseX, baseY, 1.6 + layer, frame.time);
        const y = baseY + this.jitter(baseY, baseX, 1.6 + layer, frame.time);
        if (index === 0) context.moveTo(x, y); else context.lineTo(x, y);
      }
      context.stroke();
    }
    context.beginPath();
    context.moveTo(0, horizon);
    context.lineTo(width, horizon);
    context.strokeStyle = "rgba(35,30,28,.65)";
    context.lineWidth = 1;
    context.stroke();
    context.restore();
  }

  private drawWater(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    horizon: number,
    sunX: number,
    frame: MusicLyricFrame,
  ) {
    context.save();
    context.globalCompositeOperation = "multiply";
    const wash = context.createLinearGradient(0, horizon, 0, height);
    wash.addColorStop(0, `rgba(110,140,160,${0.19 + frame.lowMid * 0.08})`);
    wash.addColorStop(1, `rgba(78,111,133,${0.25 + frame.energy * 0.09})`);
    context.fillStyle = wash;
    context.fillRect(0, horizon, width, height - horizon);
    context.globalCompositeOperation = "source-over";

    let row = 0;
    for (let y = horizon; y < height; y += 6, row += 1) {
      const depth = (y - horizon) / Math.max(1, height - horizon);
      if (this.seeds[(row * 7 + 19) % this.seeds.length] > 0.83) continue;
      const shift = Math.sin(y * 0.1 + frame.time * 4) * 15 * depth;
      const reflectionWidth = width * (0.07 + depth * 0.17 + frame.beat * 0.018) + Math.sin(y * 0.05 + frame.time) * 10;
      const breakup = (this.seeds[(row * 11 + 3) % this.seeds.length] - 0.5) * width * 0.06 * depth;
      context.fillStyle = `rgba(226,143,38,${0.32 * (1 - depth) * (0.5 + frame.energy * 0.58 + frame.beat * 0.22)})`;
      context.fillRect(sunX - reflectionWidth / 2 + shift + breakup, y, reflectionWidth, 1.5 + depth * 2.5);
    }

    const waveCount = 5 + Math.floor(frame.energy * 4 + frame.air * 2);
    context.globalAlpha = 0.24 + frame.air * 0.4 + frame.beat * 0.1;
    context.strokeStyle = "rgba(35,30,28,.72)";
    context.lineWidth = 0.9 + frame.air * 2 + frame.beat * 0.8;
    context.lineCap = "round";
    for (let wave = 0; wave < waveCount; wave += 1) {
      const depth = (wave + frame.time * (0.35 + frame.energy * 0.4) % 1) / waveCount;
      if (depth <= 0 || depth >= 1) continue;
      const y = horizon + Math.pow(depth, 1.5) * (height - horizon);
      const waveWidth = width * (0.3 + depth * 0.7);
      const left = (width - waveWidth) / 2;
      context.beginPath();
      for (let point = 0; point < 72; point += 1) {
        const x = left + waveWidth * point / 71;
        const sampleIndex = Math.floor((point / 72 + depth) * (frame.waveform?.length ?? 72)) % (frame.waveform?.length ?? 72);
        const sample = frame.waveform
          ? ((frame.waveform[sampleIndex] ?? 128) - 128) / 128
          : Math.sin(frame.time * 2 + point * 0.35 + wave) * 0.15;
        const scratch = Math.sin(point * 0.62 + frame.time * 2.4 + wave) * (1 + depth * 3);
        const offset = sample * height * (0.075 + frame.rms * 0.035) * depth + scratch;
        if (point === 0) context.moveTo(x, y + offset); else context.lineTo(x, y + offset);
      }
      context.stroke();
    }
    context.restore();
  }

  private drawClouds(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    sunX: number,
    frame: MusicLyricFrame,
  ) {
    context.save();
    context.globalCompositeOperation = "multiply";
    context.filter = `blur(${10 + frame.energy * 7}px)`;
    const segments = 12;
    for (let index = 0; index < segments; index += 1) {
      const value = frame.bands[index * 2] ?? 0;
      if (value < 0.025) continue;
      const centerX = (index + 0.5) * width / segments;
      const warmth = Math.max(0, 1 - Math.abs(centerX - sunX) / (width * 0.42));
      const hue = 210 - warmth * 170;
      const lightness = 95 - frame.energy * 47 + warmth * 14;
      context.fillStyle = `hsla(${hue}, ${10 + warmth * 42}%, ${lightness}%, ${value * 0.72})`;
      const centerY = -10 + value * height * 0.15 + Math.sin(frame.time + index) * 10;
      const radius = width * 0.105 + value * width * 0.14;
      context.beginPath();
      for (let point = 0; point <= 14; point += 1) {
        const angle = point / 14 * Math.PI * 2;
        let shapedRadius = radius * (1 + Math.sin(angle * 3 + frame.time + index) * 0.28);
        if (angle > Math.PI && angle < Math.PI * 2) shapedRadius *= 0.32;
        const x = centerX + Math.cos(angle) * shapedRadius;
        const y = Math.min(centerY + Math.sin(angle) * shapedRadius, centerY + shapedRadius);
        if (point === 0) context.moveTo(x, y); else context.lineTo(x, y);
      }
      context.closePath();
      context.fill();
    }
    context.restore();
  }

  private drawWindNotation(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    horizon: number,
    frame: MusicLyricFrame,
  ) {
    context.save();
    context.strokeStyle = `rgba(24,75,165,${0.04 + frame.air * 0.22})`;
    context.lineWidth = 0.65 + frame.air;
    context.lineCap = "round";
    for (let line = 0; line < 5; line += 1) {
      const seed = this.seeds[170 + line];
      const y = horizon * (0.18 + line * 0.12) + Math.sin(frame.time * 0.4 + line) * 12;
      const travel = (frame.time * (24 + frame.air * 70) + seed * width) % (width * 1.4) - width * 0.2;
      const length = width * (0.08 + seed * 0.13);
      context.beginPath();
      context.moveTo(travel, y);
      context.bezierCurveTo(travel + length * 0.28, y - height * 0.012, travel + length * 0.72, y + height * 0.012, travel + length, y);
      context.stroke();
    }
    context.restore();
  }

  private processWildlife(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    horizon: number,
    frame: MusicLyricFrame,
  ) {
    if (frame.time >= this.nextBirdAt && this.birds.length < MAX_BIRDS && (frame.energy > 0.1 || !frame.hasSignal)) {
      const seed = this.nextEventSeed();
      const fromLeft = seed > 0.5;
      this.birds.push({
        x: fromLeft ? -24 : width + 24,
        y: horizon - height * (0.16 + this.nextEventSeed() * 0.27),
        vx: (fromLeft ? 1 : -1) * (85 + this.nextEventSeed() * 95 + frame.energy * 80),
        vy: (this.nextEventSeed() - 0.5) * 28,
        flap: this.nextEventSeed() * Math.PI * 2,
        depth: 0.55 + this.nextEventSeed() * 0.65,
      });
      this.nextBirdAt = frame.time + 5 + this.nextEventSeed() * 8;
    }
    if (!this.fish && frame.time >= this.nextFishAt && (frame.beatTrigger || !frame.hasSignal)) {
      const direction = this.nextEventSeed() > 0.5 ? 1 : -1;
      this.fish = {
        x: width * (0.2 + this.nextEventSeed() * 0.6),
        y: horizon + 4,
        vx: direction * (90 + this.nextEventSeed() * 130),
        vy: -250 - this.nextEventSeed() * 230 - frame.beat * 90,
      };
      this.pushSplat(this.fish.x, horizon, 0.42, 0.9);
      this.nextFishAt = frame.time + 10 + this.nextEventSeed() * 16;
    }

    context.save();
    context.strokeStyle = "rgba(35,30,28,.78)";
    context.lineWidth = 1.35;
    context.lineCap = "round";
    context.lineJoin = "round";
    if (this.fish) {
      this.fish.x += this.fish.vx * frame.delta;
      this.fish.vy += 680 * frame.delta;
      this.fish.y += this.fish.vy * frame.delta;
      if (this.fish.y > horizon + 22 || this.fish.x < -50 || this.fish.x > width + 50) {
        this.pushSplat(this.fish.x, horizon, 0.44, 1);
        this.fish = undefined;
      } else {
        const angle = Math.atan2(this.fish.vy, this.fish.vx);
        context.save();
        context.translate(this.fish.x, this.fish.y);
        context.rotate(angle);
        context.beginPath();
        context.moveTo(-6, 0);
        context.quadraticCurveTo(0, -5, 8, 0);
        context.quadraticCurveTo(0, 5, -6, 0);
        context.moveTo(-6, 0);
        context.lineTo(-11, -4);
        context.moveTo(-6, 0);
        context.lineTo(-11, 4);
        context.stroke();
        context.restore();
      }
    }

    for (let index = this.birds.length - 1; index >= 0; index -= 1) {
      const bird = this.birds[index];
      bird.flap += (12 + frame.presence * 16) * frame.delta;
      if (this.fish && this.fish.y < horizon) {
        bird.vx += (this.fish.x - bird.x) * 0.55 * frame.delta;
        bird.vy += (this.fish.y - bird.y) * 0.55 * frame.delta;
      } else {
        bird.vy += (Math.sin(frame.time + index) * 45 - bird.vy) * frame.delta;
      }
      const speed = Math.hypot(bird.vx, bird.vy);
      if (speed > 320) {
        bird.vx = bird.vx / speed * 320;
        bird.vy = bird.vy / speed * 320;
      }
      bird.x += bird.vx * frame.delta;
      bird.y += bird.vy * frame.delta;
      if (bird.x < -110 || bird.x > width + 110 || bird.y < -100 || bird.y > horizon + 30) {
        this.birds.splice(index, 1);
        continue;
      }
      const flap = Math.sin(bird.flap) * 6 * bird.depth;
      context.globalAlpha = 0.42 + bird.depth * 0.42;
      context.lineWidth = bird.depth * 1.35;
      context.beginPath();
      context.moveTo(bird.x - 8 * bird.depth, bird.y - flap);
      context.quadraticCurveTo(bird.x - 4, bird.y, bird.x, bird.y + 2);
      context.quadraticCurveTo(bird.x + 4, bird.y, bird.x + 8 * bird.depth, bird.y - flap);
      context.stroke();
    }
    context.restore();
  }

  private processWeather(
    context: CanvasRenderingContext2D,
    width: number,
    height: number,
    horizon: number,
    translation: MusicLyricBounds | undefined,
    frame: MusicLyricFrame,
  ) {
    const rainIntensity = Math.max(0, frame.energy - 0.24) * 2.2 + frame.transient * 0.32;
    this.rainBudget = Math.min(18, this.rainBudget + rainIntensity * frame.delta * 76);
    while (this.rainBudget >= 1 && this.drops.length < MAX_DROPS) {
      this.rainBudget -= 1;
      const depth = 0.48 + this.nextEventSeed() * 1.45;
      this.drops.push({
        x: this.nextEventSeed() * width,
        y: -24,
        speed: 480 * depth + this.nextEventSeed() * 240 + frame.energy * 330,
        length: 8 * depth + this.nextEventSeed() * 17,
        depth,
        background: this.nextEventSeed() > 0.58,
      });
    }

    context.save();
    context.lineCap = "round";
    context.strokeStyle = "rgba(24,75,165,.38)";
    context.beginPath();
    for (let index = this.drops.length - 1; index >= 0; index -= 1) {
      const drop = this.drops[index];
      drop.y += drop.speed * frame.delta;
      drop.x += drop.speed * (0.025 + frame.air * 0.025) * frame.delta;
      let impactY: number | undefined;
      if (drop.background && drop.y >= horizon) {
        impactY = horizon;
      } else if (!drop.background && translation && pointInside(drop.x, drop.y, translation)) {
        impactY = drop.y;
      } else if (!drop.background && drop.y >= height) {
        impactY = height;
      }
      if (impactY !== undefined) {
        this.pushSplat(drop.x, impactY, 0.22 + drop.depth * 0.08, drop.depth);
        this.drops.splice(index, 1);
        continue;
      }
      context.globalAlpha = drop.background ? 0.16 : 0.28 + drop.depth * 0.15;
      context.lineWidth = drop.background ? 0.7 : 0.8 + drop.depth * 0.55;
      context.moveTo(drop.x, drop.y);
      context.lineTo(drop.x + drop.speed * 0.018, drop.y + drop.length);
    }
    context.stroke();

    for (let index = this.splats.length - 1; index >= 0; index -= 1) {
      const splat = this.splats[index];
      splat.age += frame.delta;
      if (splat.age >= splat.duration) {
        this.splats.splice(index, 1);
        continue;
      }
      const progress = splat.age / splat.duration;
      const radius = progress * (8 + splat.strength * 5);
      context.globalAlpha = (1 - progress) * 0.42;
      context.strokeStyle = "rgba(24,75,165,.82)";
      context.lineWidth = 0.8;
      context.beginPath();
      context.ellipse(splat.x, splat.y, radius * 2, radius * 0.38, 0, 0, Math.PI * 2);
      context.stroke();
      if (splat.strength > 0.9) {
        context.beginPath();
        for (let ray = 0; ray < 5; ray += 1) {
          const angle = ray / 5 * Math.PI * 2;
          context.moveTo(splat.x + Math.cos(angle) * radius * 0.3, splat.y + Math.sin(angle) * radius * 0.12);
          context.lineTo(splat.x + Math.cos(angle) * radius, splat.y + Math.sin(angle) * radius * 0.38);
        }
        context.stroke();
      }
    }
    context.restore();
  }

  private pushSplat(x: number, y: number, duration: number, strength: number) {
    if (this.splats.length >= MAX_SPLATS) this.splats.shift();
    this.splats.push({ x, y, age: 0, duration, strength });
  }

  private nextEventSeed(): number {
    this.eventIndex = (this.eventIndex + 1) % 1_000_000;
    return pseudoRandom(this.eventIndex * 17 + 311);
  }

  private jitter(x: number, y: number, amount: number, time: number): number {
    const frame = Math.floor(time * 12) % 3;
    const hash = Math.sin(x * 12.9898 + y * 78.233 + frame * 13.131) * 43_758.5453;
    return (hash - Math.floor(hash) - 0.5) * amount;
  }

  private terrainNoise(value: number, layer: number): number {
    const rolling = Math.sin(value * (0.0046 + layer * 0.0005)) * 0.5 + 0.5;
    const detail = Math.sin(value * 0.012 + layer) * 0.25 + Math.sin(value * 0.03 + layer * 2) * 0.125;
    return (rolling + detail) * 0.7 + Math.abs(Math.sin(value * 0.008 + layer)) * 0.3;
  }
}

function pointInside(x: number, y: number, bounds: MusicLyricBounds): boolean {
  return x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom;
}

function pseudoRandom(seed: number): number {
  const value = Math.sin(seed * 12.9898) * 43_758.5453;
  return value - Math.floor(value);
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}

function lyricPixelRatio(): number {
  return Math.min(2, Math.max(1, window.devicePixelRatio || 1));
}
