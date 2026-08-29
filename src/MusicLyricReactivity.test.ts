import { describe, expect, it } from "vitest";
import { applyMusicLyricFrameStyles, MusicLyricReactivity } from "./MusicLyricReactivity";

describe("MusicLyricReactivity", () => {
  it("derives one bounded multi-band frame and detects a bass transient", () => {
    const spectrum = new Uint8Array(512);
    const waveformSource = new Uint8Array(1_024).fill(128);
    const frequency = new Uint8Array(spectrum.length);
    const waveform = new Uint8Array(waveformSource.length);
    const analyser = {
      getByteFrequencyData: (target: Uint8Array) => target.set(spectrum),
      getByteTimeDomainData: (target: Uint8Array) => target.set(waveformSource),
    } as unknown as AnalyserNode;
    const reactivity = new MusicLyricReactivity();
    const layout = { horizon: 480 };

    const quiet = reactivity.sample(analyser, frequency, waveform, 0.25, layout, 1_000);
    spectrum.fill(238, 1, 7);
    for (let index = 0; index < waveformSource.length; index += 1) waveformSource[index] = index % 2 ? 224 : 32;
    const impact = reactivity.sample(analyser, frequency, waveform, 0.5, layout, 1_016);

    expect(quiet.hasSignal).toBe(true);
    expect(impact.progress).toBe(0.5);
    expect(impact.bass).toBeGreaterThan(0.7);
    expect(impact.rms).toBeGreaterThan(0.7);
    expect(impact.transient).toBeGreaterThan(0.5);
    expect(impact.bands).toHaveLength(48);
    expect([...impact.bands].every((value) => value >= 0 && value <= 1)).toBe(true);
  });

  it("publishes derived motion values for the lyric typography", () => {
    const element = document.createElement("section");
    const frame = new MusicLyricReactivity().sample(undefined, undefined, undefined, 2, { horizon: 0 }, 1_000);
    applyMusicLyricFrameStyles(element, frame);

    expect(frame.progress).toBe(1);
    expect(element.style.getPropertyValue("--lyric-copy-scale")).toMatch(/^1\./);
    expect(element.style.getPropertyValue("--lyric-glow")).toMatch(/px$/);
  });
});
