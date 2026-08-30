/**
 * Bounded, offline preparation for imported voice references.
 *
 * This intentionally selects one continuous excerpt. Joining disjoint audible
 * regions can combine different speakers or performances and can manufacture
 * unnatural transitions in the voice-cloning reference.
 */

export const VOICE_EXCERPT_SECONDS = 20;
export const MAX_EXCERPT_ANALYSIS_SECONDS = 5 * 60;

const MAX_SOURCE_BYTES = 32 * 1024 * 1024;
const MIN_SAMPLE_RATE = 8_000;
const MAX_SAMPLE_RATE = 384_000;
const MAX_CHANNELS = 8;
const ANALYSIS_FRAME_SECONDS = 0.05;
const ANALYSIS_SAMPLE_RATE = 8_000;

export interface VoiceExcerptWindow {
  startSample: number;
  endSample: number;
  durationSeconds: number;
  activityRatio: number;
}

export interface VoiceExcerptResult {
  blob: Blob;
  durationSeconds: number;
  originalDurationSeconds: number;
  startSeconds: number;
  endSeconds: number;
}

export interface VoiceExcerptOptions {
  knownDurationSeconds?: number;
  startSeconds?: number;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}

function percentile(sortedValues: number[], ratio: number): number {
  if (!sortedValues.length) return -100;
  const index = Math.round(clamp(ratio, 0, 1) * (sortedValues.length - 1));
  return sortedValues[index];
}

function validateChannels(channels: Float32Array[], sampleRate: number): number {
  if (!Number.isFinite(sampleRate) || sampleRate < MIN_SAMPLE_RATE || sampleRate > MAX_SAMPLE_RATE) {
    throw new Error("The recording has an unsupported sample rate.");
  }
  if (!channels.length || channels.length > MAX_CHANNELS) {
    throw new Error(`Voice references must contain between 1 and ${MAX_CHANNELS} audio channels.`);
  }
  const length = channels[0]?.length ?? 0;
  if (!length || channels.some((channel) => channel.length !== length)) {
    throw new Error("The recording contains empty or inconsistent audio channels.");
  }
  return length;
}

/**
 * Finds the strongest continuous target-sized window using a bounded adaptive
 * RMS energy scan. Energy is only a useful starting point, not speaker
 * recognition, so the UI must ask the user to listen to the result.
 */
export function selectVoiceExcerptWindow(
  channels: Float32Array[],
  sampleRate: number,
  targetDurationSeconds = VOICE_EXCERPT_SECONDS,
): VoiceExcerptWindow {
  const totalSamples = validateChannels(channels, sampleRate);
  if (!Number.isFinite(targetDurationSeconds) || targetDurationSeconds <= 0) {
    throw new Error("The requested voice excerpt duration is invalid.");
  }

  const targetSamples = Math.min(totalSamples, Math.max(1, Math.round(targetDurationSeconds * sampleRate)));
  const frameSamples = Math.max(1, Math.round(sampleRate * ANALYSIS_FRAME_SECONDS));
  const frameCount = Math.ceil(totalSamples / frameSamples);
  const sampleStride = Math.max(1, Math.floor(sampleRate / ANALYSIS_SAMPLE_RATE));
  const energiesDb = new Float64Array(frameCount);

  for (let frame = 0; frame < frameCount; frame++) {
    const start = frame * frameSamples;
    const end = Math.min(totalSamples, start + frameSamples);
    let sumSquares = 0;
    let values = 0;
    for (let sample = start; sample < end; sample += sampleStride) {
      for (const channel of channels) {
        const value = channel[sample];
        if (Number.isFinite(value)) sumSquares += value * value;
        values++;
      }
    }
    const rms = Math.sqrt(sumSquares / Math.max(1, values));
    energiesDb[frame] = clamp(20 * Math.log10(rms + 1e-7), -100, 0);
  }

  const orderedEnergies = Array.from(energiesDb).sort((left, right) => left - right);
  const noiseFloorDb = percentile(orderedEnergies, 0.2);
  const highEnergyDb = percentile(orderedEnergies, 0.85);
  const activityThresholdDb = clamp(
    Math.max(noiseFloorDb + 8, highEnergyDb - 24),
    -55,
    -24,
  );

  const scores = new Float64Array(frameCount);
  const activeFrames = new Uint8Array(frameCount);
  for (let frame = 0; frame < frameCount; frame++) {
    const db = energiesDb[frame];
    if (db >= activityThresholdDb) {
      activeFrames[frame] = 1;
      scores[frame] = 1 + clamp((db - activityThresholdDb) / 24, 0, 1);
      if (db > -1) scores[frame] -= 0.75;
    } else {
      scores[frame] = -0.25;
    }
  }

  const windowFrames = Math.min(frameCount, Math.max(1, Math.ceil(targetSamples / frameSamples)));
  const scorePrefix = new Float64Array(frameCount + 1);
  const activePrefix = new Uint32Array(frameCount + 1);
  for (let frame = 0; frame < frameCount; frame++) {
    scorePrefix[frame + 1] = scorePrefix[frame] + scores[frame];
    activePrefix[frame + 1] = activePrefix[frame] + activeFrames[frame];
  }

  const latestStartFrame = frameCount - windowFrames;
  const centeredStartFrame = latestStartFrame / 2;
  let bestStartFrame = 0;
  let bestScore = Number.NEGATIVE_INFINITY;
  let bestDistanceFromCenter = Number.POSITIVE_INFINITY;
  for (let startFrame = 0; startFrame <= latestStartFrame; startFrame++) {
    const endFrame = startFrame + windowFrames;
    const score = scorePrefix[endFrame] - scorePrefix[startFrame];
    const distanceFromCenter = Math.abs(startFrame - centeredStartFrame);
    if (
      score > bestScore + 1e-9
      || (Math.abs(score - bestScore) <= 1e-9 && distanceFromCenter < bestDistanceFromCenter)
    ) {
      bestStartFrame = startFrame;
      bestScore = score;
      bestDistanceFromCenter = distanceFromCenter;
    }
  }

  const bestEndFrame = bestStartFrame + windowFrames;
  const detectedActiveFrames = activePrefix[bestEndFrame] - activePrefix[bestStartFrame];
  const startSample = Math.min(bestStartFrame * frameSamples, totalSamples - targetSamples);
  const endSample = startSample + targetSamples;
  return {
    startSample,
    endSample,
    durationSeconds: targetSamples / sampleRate,
    activityRatio: detectedActiveFrames / windowFrames,
  };
}

export function voiceExcerptWindowFromStart(
  totalSamples: number,
  sampleRate: number,
  startSeconds: number,
): VoiceExcerptWindow {
  if (!Number.isSafeInteger(totalSamples) || totalSamples <= 0) {
    throw new Error("The recording contains no audio samples.");
  }
  if (!Number.isFinite(sampleRate) || sampleRate < MIN_SAMPLE_RATE || sampleRate > MAX_SAMPLE_RATE) {
    throw new Error("The recording has an unsupported sample rate.");
  }
  if (!Number.isFinite(startSeconds) || startSeconds < 0) {
    throw new Error("The selected voice excerpt start time is invalid.");
  }

  const targetSamples = Math.min(totalSamples, Math.round(VOICE_EXCERPT_SECONDS * sampleRate));
  const requestedStartSample = Math.round(startSeconds * sampleRate);
  const startSample = clamp(requestedStartSample, 0, totalSamples - targetSamples);
  return {
    startSample,
    endSample: startSample + targetSamples,
    durationSeconds: targetSamples / sampleRate,
    activityRatio: 0,
  };
}

export function encodeMonoPcmWav(samples: Float32Array, sampleRate: number): Blob {
  if (!samples.length) throw new Error("The selected voice excerpt is empty.");
  if (!Number.isFinite(sampleRate) || sampleRate < MIN_SAMPLE_RATE || sampleRate > MAX_SAMPLE_RATE) {
    throw new Error("The recording has an unsupported sample rate.");
  }

  const bytesPerSample = 2;
  const dataSize = samples.length * bytesPerSample;
  if (dataSize > 0xffff_ffff - 44) {
    throw new Error("The selected voice excerpt is too large to encode as WAV.");
  }
  const output = new ArrayBuffer(44 + dataSize);
  const view = new DataView(output);
  const writeAscii = (offset: number, value: string) => {
    for (let index = 0; index < value.length; index++) {
      view.setUint8(offset + index, value.charCodeAt(index));
    }
  };

  writeAscii(0, "RIFF");
  view.setUint32(4, 36 + dataSize, true);
  writeAscii(8, "WAVE");
  writeAscii(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * bytesPerSample, true);
  view.setUint16(32, bytesPerSample, true);
  view.setUint16(34, 16, true);
  writeAscii(36, "data");
  view.setUint32(40, dataSize, true);

  let offset = 44;
  for (const rawSample of samples) {
    const sample = Number.isFinite(rawSample) ? clamp(rawSample, -1, 1) : 0;
    const integer = sample < 0 ? Math.round(sample * 0x8000) : Math.round(sample * 0x7fff);
    view.setInt16(offset, integer, true);
    offset += bytesPerSample;
  }
  return new Blob([output], { type: "audio/wav" });
}

function downmixWindow(channels: Float32Array[], window: VoiceExcerptWindow, sampleRate: number): Float32Array {
  const length = window.endSample - window.startSample;
  const mono = new Float32Array(length);
  for (let index = 0; index < length; index++) {
    let sum = 0;
    for (const channel of channels) {
      const sample = channel[window.startSample + index];
      if (Number.isFinite(sample)) sum += sample;
    }
    mono[index] = sum / channels.length;
  }

  const fadeSamples = Math.min(Math.floor(sampleRate * 0.01), Math.floor(length / 2));
  for (let index = 0; index < fadeSamples; index++) {
    const gain = index / Math.max(1, fadeSamples);
    mono[index] *= gain;
    mono[length - 1 - index] *= gain;
  }
  return mono;
}

export async function createVoiceReferenceExcerpt(
  source: Blob,
  options: VoiceExcerptOptions = {},
): Promise<VoiceExcerptResult> {
  if (!source.size) throw new Error("The selected recording is empty.");
  if (source.size > MAX_SOURCE_BYTES) throw new Error("Choose a voice reference smaller than 32 MiB.");
  const { knownDurationSeconds, startSeconds } = options;
  if (knownDurationSeconds !== undefined) {
    if (!Number.isFinite(knownDurationSeconds) || knownDurationSeconds <= 0) {
      throw new Error("The recording duration is invalid.");
    }
    if (knownDurationSeconds > MAX_EXCERPT_ANALYSIS_SECONDS) {
      throw new Error("In-app excerpt selection supports recordings up to 5 minutes. Use an audio editor to extract one clean 8–20 second passage from this file.");
    }
  }

  const AudioCtx = window.AudioContext
    || (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!AudioCtx) throw new Error("This desktop WebView cannot analyze audio excerpts.");

  const context = new AudioCtx();
  try {
    let decoded: AudioBuffer;
    try {
      decoded = await context.decodeAudioData(await source.arrayBuffer());
    } catch (cause) {
      throw new Error("This desktop WebView could not decode the recording for excerpt selection.", { cause });
    }

    const channels = Array.from(
      { length: decoded.numberOfChannels },
      (_, channel) => decoded.getChannelData(channel),
    );
    const totalSamples = validateChannels(channels, decoded.sampleRate);
    const originalDurationSeconds = totalSamples / decoded.sampleRate;
    if (originalDurationSeconds > MAX_EXCERPT_ANALYSIS_SECONDS) {
      throw new Error("In-app excerpt selection supports recordings up to 5 minutes. Use an audio editor to extract one clean 8–20 second passage from this file.");
    }

    const window = startSeconds === undefined
      ? selectVoiceExcerptWindow(channels, decoded.sampleRate, VOICE_EXCERPT_SECONDS)
      : voiceExcerptWindowFromStart(totalSamples, decoded.sampleRate, startSeconds);
    const mono = downmixWindow(channels, window, decoded.sampleRate);
    return {
      blob: encodeMonoPcmWav(mono, decoded.sampleRate),
      durationSeconds: window.durationSeconds,
      originalDurationSeconds,
      startSeconds: window.startSample / decoded.sampleRate,
      endSeconds: window.endSample / decoded.sampleRate,
    };
  } finally {
    if (context.state !== "closed") {
      await context.close().catch(() => undefined);
    }
  }
}
