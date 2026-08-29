import { SketchbookMusicLyricVisualizer } from "./MusicLyricVisualizer";
import { SignalBloomMusicLyricVisualizer } from "./MusicLyricSignalBloomVisualizer";
import type { MusicLyricTheme } from "./types";

export interface MusicLyricRenderer {
  draw(
    analyser: AnalyserNode | undefined,
    frequency: Uint8Array | undefined,
    timeData: Uint8Array | undefined,
    progress: number,
  ): void;
  destroy?(): void;
}

export const MUSIC_LYRIC_THEMES: ReadonlyArray<{
  id: MusicLyricTheme;
  name: string;
  description: string;
}> = [
  {
    id: "sketchbook",
    name: "Living sketchbook",
    description: "Paper, weather, water, and a hand-drawn travelling sun.",
  },
  {
    id: "signal-bloom",
    name: "Signal bloom",
    description: "A nocturnal spectral field that blooms around rhythm and voice.",
  },
];

export function createMusicLyricVisualizer(
  theme: MusicLyricTheme,
  canvas: HTMLCanvasElement,
): MusicLyricRenderer {
  switch (theme) {
    case "signal-bloom":
      return new SignalBloomMusicLyricVisualizer(canvas);
    case "sketchbook":
    default:
      return new SketchbookMusicLyricVisualizer(canvas);
  }
}
