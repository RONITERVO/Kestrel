/* SPDX-FileCopyrightText: 2026 Roni Tervo
 * SPDX-License-Identifier: Apache-2.0
 */

export interface MusicLyricBounds {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

export interface MusicLyricLayout {
  horizon: number;
  primary?: MusicLyricBounds;
  translation?: MusicLyricBounds;
  activeWord?: MusicLyricBounds;
}

export interface MusicLyricFrame {
  now: number;
  time: number;
  delta: number;
  progress: number;
  hasSignal: boolean;
  energy: number;
  subBass: number;
  bass: number;
  lowMid: number;
  mid: number;
  presence: number;
  air: number;
  rms: number;
  flux: number;
  transient: number;
  beat: number;
  beatTrigger: boolean;
  centroid: number;
  bands: Float32Array;
  waveform?: Uint8Array;
  layout: MusicLyricLayout;
}

/**
 * Samples the single producer-owned analyser and turns it into one stable visual contract.
 * Renderers never read Web Audio independently, so every visual layer reacts to the same frame.
 */
export class MusicLyricReactivity {
  private readonly bands = new Float32Array(48);
  private readonly previousSpectrum = new Float32Array(512);
  private lastNow = 0;
  private bassEnvelope = 0;
  private transientEnvelope = 0;
  private beatEnvelope = 0;
  private beatBaseline = 0;
  private beatDeviation = 0.025;
  private lastBeatAt = -1;
  private hasPreviousSpectrum = false;

  sample(
    analyser: AnalyserNode | undefined,
    frequency: Uint8Array | undefined,
    waveform: Uint8Array | undefined,
    progress: number,
    layout: MusicLyricLayout,
    now = performance.now(),
  ): MusicLyricFrame {
    const delta = this.lastNow > 0
      ? Math.min(0.08, Math.max(0.001, (now - this.lastNow) / 1_000))
      : 1 / 60;
    this.lastNow = now;
    const time = now / 1_000;
    const hasSignal = Boolean(analyser && frequency?.length && waveform?.length);
    if (analyser && frequency && waveform) {
      analyser.getByteFrequencyData(frequency as Uint8Array<ArrayBuffer>);
      analyser.getByteTimeDomainData(waveform as Uint8Array<ArrayBuffer>);
    }

    // Fractions of Nyquist: approximately 22–77, 66–265, 265–990, 1–3 kHz,
    // 3–8 kHz, and 8–18 kHz at the common 44.1 kHz sample rate.
    const subBass = hasSignal && frequency ? fractionalEnergy(frequency, 0.001, 0.0035) : idle(time, 0.17, 0.8);
    const bass = hasSignal && frequency ? fractionalEnergy(frequency, 0.003, 0.012) : idle(time + 0.9, 0.2, 1.15);
    const lowMid = hasSignal && frequency ? fractionalEnergy(frequency, 0.012, 0.045) : idle(time + 1.8, 0.16, 0.63);
    const mid = hasSignal && frequency ? fractionalEnergy(frequency, 0.045, 0.14) : idle(time + 2.7, 0.14, 0.47);
    const presence = hasSignal && frequency ? fractionalEnergy(frequency, 0.14, 0.36) : idle(time + 3.6, 0.11, 0.38);
    const air = hasSignal && frequency ? fractionalEnergy(frequency, 0.36, 0.82) : idle(time + 4.5, 0.08, 0.29);
    const energy = clamp01(subBass * 0.12 + bass * 0.22 + lowMid * 0.24 + mid * 0.2 + presence * 0.15 + air * 0.07);
    const rms = hasSignal && waveform ? waveformRms(waveform) : idle(time + 5.4, 0.1, 0.74);
    const { flux, lowFlux, centroid } = hasSignal && frequency
      ? this.compareSpectrum(frequency)
      : { flux: idle(time + 6.3, 0.035, 1.7), lowFlux: 0, centroid: 0.34 + Math.sin(time * 0.11) * 0.04 };

    const previousBass = this.bassEnvelope;
    const bassRate = bass > previousBass ? 22 : 5;
    this.bassEnvelope += (bass - previousBass) * Math.min(1, delta * bassRate);
    const bassRise = Math.max(0, bass - previousBass);
    const onset = Math.max(0, bassRise - 0.012) * 8.5 + flux * 2.4 + Math.max(0, rms - energy) * 0.8;
    this.transientEnvelope = Math.max(this.transientEnvelope * Math.exp(-delta * 6.2), clamp01(onset));

    // A beat is deliberately narrower than a transient: only a positive low-band spectral
    // change or a fast bass attack may trigger it. The adaptive floor follows the current song
    // slowly, while the refractory interval prevents a sustained bass note from retriggering.
    const beatInput = hasSignal ? clamp01(lowFlux * 0.82 + bassRise * 0.65) : 0;
    const beatThreshold = this.beatBaseline + Math.max(0.025, this.beatDeviation * 1.65);
    let beatHit = 0;
    if (beatInput > beatThreshold && (this.lastBeatAt < 0 || time - this.lastBeatAt >= 0.17)) {
      beatHit = clamp01((beatInput - beatThreshold) / Math.max(0.075, this.beatDeviation * 2.4));
      if (beatHit >= 0.12) this.lastBeatAt = time;
    }
    this.beatEnvelope = Math.max(this.beatEnvelope * Math.exp(-delta * 8.5), beatHit);
    const learnedBeatInput = Math.min(
      beatInput,
      this.beatBaseline + this.beatDeviation * 2.5 + 0.04,
    );
    this.beatBaseline += (learnedBeatInput - this.beatBaseline) * Math.min(1, delta * 1.4);
    this.beatDeviation += (
      Math.abs(learnedBeatInput - this.beatBaseline) - this.beatDeviation
    ) * Math.min(1, delta * 1.8);

    for (let index = 0; index < this.bands.length; index += 1) {
      const normalized = index / Math.max(1, this.bands.length - 1);
      const sourceIndex = Math.min(
        Math.max(0, (frequency?.length ?? 1) - 1),
        Math.floor(Math.pow(normalized, 1.55) * Math.max(0, (frequency?.length ?? 1) - 1)),
      );
      const target = hasSignal && frequency
        ? (frequency[sourceIndex] ?? 0) / 255
        : 0.045 + Math.pow(Math.sin(time * 0.48 + index * 0.23) * 0.5 + 0.5, 3) * 0.13;
      const rate = target > this.bands[index] ? 20 : 6;
      this.bands[index] += (target - this.bands[index]) * Math.min(1, delta * rate);
    }

    return {
      now,
      time,
      delta,
      progress: clamp01(progress),
      hasSignal,
      energy,
      subBass,
      bass,
      lowMid,
      mid,
      presence,
      air,
      rms,
      flux,
      transient: this.transientEnvelope,
      beat: this.beatEnvelope,
      beatTrigger: beatHit >= 0.12,
      centroid,
      bands: this.bands,
      waveform: hasSignal ? waveform : undefined,
      layout,
    };
  }

  private compareSpectrum(frequency: Uint8Array): { flux: number; lowFlux: number; centroid: number } {
    let positiveDifference = 0;
    let lowPositiveDifference = 0;
    let magnitude = 0;
    let weighted = 0;
    const count = Math.min(frequency.length, this.previousSpectrum.length);
    const lowStart = Math.max(1, Math.floor(count * 0.002));
    const lowEnd = Math.max(lowStart + 1, Math.min(count, Math.ceil(count * 0.012)));
    for (let index = 1; index < count; index += 1) {
      const value = (frequency[index] ?? 0) / 255;
      if (this.hasPreviousSpectrum) {
        const positive = Math.max(0, value - this.previousSpectrum[index]);
        positiveDifference += positive;
        if (index >= lowStart && index < lowEnd) lowPositiveDifference += positive;
      }
      this.previousSpectrum[index] = value;
      magnitude += value;
      weighted += value * (index / Math.max(1, count - 1));
    }
    this.hasPreviousSpectrum = true;
    return {
      flux: clamp01(positiveDifference / Math.max(1, count - 1) * 5.5),
      lowFlux: clamp01(lowPositiveDifference / Math.max(1, lowEnd - lowStart) * 1.35),
      centroid: magnitude > 0.0001 ? clamp01(weighted / magnitude) : 0,
    };
  }
}

export function applyMusicLyricFrameStyles(element: HTMLElement, frame: MusicLyricFrame) {
  element.style.setProperty("--lyric-energy", frame.energy.toFixed(4));
  element.style.setProperty("--lyric-bass", frame.bass.toFixed(4));
  element.style.setProperty("--lyric-presence", frame.presence.toFixed(4));
  element.style.setProperty("--lyric-air", frame.air.toFixed(4));
  element.style.setProperty("--lyric-transient", frame.transient.toFixed(4));
  element.style.setProperty("--lyric-beat", frame.beat.toFixed(4));
  element.style.setProperty("--lyric-copy-lift", `${(-frame.energy * 4.5).toFixed(2)}px`);
  element.style.setProperty("--lyric-copy-scale", (1 + frame.transient * 0.012 + frame.beat * 0.008).toFixed(4));
  element.style.setProperty("--lyric-active-scale", (1.01 + frame.transient * 0.036 + frame.beat * 0.018).toFixed(4));
  element.style.setProperty("--lyric-translation-lift", `${(-frame.lowMid * 5).toFixed(2)}px`);
  element.style.setProperty("--lyric-glow", `${(18 + frame.presence * 34 + frame.transient * 12).toFixed(1)}px`);
}

function fractionalEnergy(values: Uint8Array, start: number, end: number): number {
  const safeStart = Math.max(0, Math.min(values.length - 1, Math.floor(values.length * start)));
  const safeEnd = Math.max(safeStart + 1, Math.min(values.length, Math.ceil(values.length * end)));
  let total = 0;
  let weightTotal = 0;
  for (let index = safeStart; index < safeEnd; index += 1) {
    const position = (index - safeStart) / Math.max(1, safeEnd - safeStart - 1);
    const weight = 0.72 + Math.sin(position * Math.PI) * 0.28;
    total += (values[index] ?? 0) * weight;
    weightTotal += weight;
  }
  return total / Math.max(1, weightTotal) / 255;
}

function waveformRms(values: Uint8Array): number {
  let squareTotal = 0;
  for (let index = 0; index < values.length; index += 1) {
    const sample = ((values[index] ?? 128) - 128) / 128;
    squareTotal += sample * sample;
  }
  return clamp01(Math.sqrt(squareTotal / Math.max(1, values.length)) * 2.2);
}

function idle(time: number, floor: number, speed: number): number {
  return clamp01(floor + Math.sin(time * speed) * 0.026 + Math.sin(time * speed * 0.31 + 1.2) * 0.018);
}

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}
