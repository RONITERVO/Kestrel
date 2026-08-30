import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createVoiceReferenceExcerpt,
  encodeMonoPcmWav,
  MAX_EXCERPT_ANALYSIS_SECONDS,
  selectVoiceExcerptWindow,
  voiceExcerptWindowFromStart,
} from "./voiceReferenceProcessing";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("voice reference processing", () => {
  it("selects one bounded continuous window around the strongest activity", () => {
    const sampleRate = 8_000;
    const channel = new Float32Array(sampleRate * 60);
    channel.fill(0.18, sampleRate * 5, sampleRate * 10);
    channel.fill(0.42, sampleRate * 32, sampleRate * 41);

    const window = selectVoiceExcerptWindow([channel], sampleRate, 20);

    expect(window.endSample - window.startSample).toBe(sampleRate * 20);
    expect(window.startSample / sampleRate).toBeGreaterThanOrEqual(21);
    expect(window.endSample / sampleRate).toBeLessThanOrEqual(52);
    expect(window.activityRatio).toBeGreaterThan(0.4);
  });

  it("still caps a continuous long recording instead of retaining the whole span", () => {
    const sampleRate = 8_000;
    const channel = new Float32Array(sampleRate * 60).fill(0.3);

    const window = selectVoiceExcerptWindow([channel], sampleRate, 20);

    expect(window.durationSeconds).toBe(20);
    expect(window.startSample).toBe(sampleRate * 20);
    expect(window.endSample).toBe(sampleRate * 40);
  });

  it("uses an explicit playhead and clamps it to a complete final excerpt", () => {
    const sampleRate = 8_000;
    const totalSamples = sampleRate * 60;

    const selected = voiceExcerptWindowFromStart(totalSamples, sampleRate, 17.25);
    const nearEnd = voiceExcerptWindowFromStart(totalSamples, sampleRate, 55);

    expect(selected.startSample).toBe(sampleRate * 17.25);
    expect(selected.endSample).toBe(sampleRate * 37.25);
    expect(nearEnd.startSample).toBe(sampleRate * 40);
    expect(nearEnd.endSample).toBe(sampleRate * 60);
  });

  it("encodes finite mono 16-bit PCM WAV data", async () => {
    const samples = new Float32Array([Number.NaN, -1, -0.5, 0, 0.5, 1, Number.POSITIVE_INFINITY]);
    const blob = encodeMonoPcmWav(samples, 48_000);
    const view = new DataView(await blob.arrayBuffer());

    expect(blob.type).toBe("audio/wav");
    expect(blob.size).toBe(44 + samples.length * 2);
    expect(String.fromCharCode(...new Uint8Array(await blob.arrayBuffer(), 0, 4))).toBe("RIFF");
    expect(view.getUint16(20, true)).toBe(1);
    expect(view.getUint16(22, true)).toBe(1);
    expect(view.getUint32(24, true)).toBe(48_000);
    expect(view.getUint16(34, true)).toBe(16);
    expect(view.getInt16(44, true)).toBe(0);
  });

  it("decodes, downmixes, and returns a saveable continuous excerpt", async () => {
    const sampleRate = 8_000;
    const left = new Float32Array(sampleRate * 60);
    const right = new Float32Array(sampleRate * 60);
    left.fill(0.35, sampleRate * 28, sampleRate * 40);
    right.fill(0.25, sampleRate * 28, sampleRate * 40);
    const decoded = {
      numberOfChannels: 2,
      sampleRate,
      length: left.length,
      getChannelData: (channel: number) => channel ? right : left,
    };
    const close = vi.fn().mockResolvedValue(undefined);
    class MockAudioContext {
      state = "running";
      decodeAudioData = vi.fn().mockResolvedValue(decoded);
      close = close;
    }
    vi.stubGlobal("AudioContext", MockAudioContext);

    const result = await createVoiceReferenceExcerpt(new Blob(["audio"]), { knownDurationSeconds: 60 });
    const view = new DataView(await result.blob.arrayBuffer());

    expect(result.originalDurationSeconds).toBe(60);
    expect(result.durationSeconds).toBe(20);
    expect(result.endSeconds - result.startSeconds).toBe(20);
    expect(view.getUint16(22, true)).toBe(1);
    expect(view.getUint32(24, true)).toBe(sampleRate);
    expect(close).toHaveBeenCalledOnce();
  });

  it("rejects oversized-duration analysis before allocating decoded PCM", async () => {
    const constructor = vi.fn();
    vi.stubGlobal("AudioContext", constructor);

    await expect(createVoiceReferenceExcerpt(
      new Blob(["audio"]),
      { knownDurationSeconds: MAX_EXCERPT_ANALYSIS_SECONDS + 0.1 },
    )).rejects.toThrow("up to 5 minutes");
    expect(constructor).not.toHaveBeenCalled();
  });
});
