import type { MusicLyricFrame } from "./MusicLyricReactivity";
import type { MusicLyricTheme } from "./types";

export interface MusicLyricRenderer {
  draw(frame: MusicLyricFrame): void;
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

export async function createMusicLyricVisualizer(
  theme: MusicLyricTheme,
  canvas: HTMLCanvasElement,
): Promise<MusicLyricRenderer> {
  switch (theme) {
    case "signal-bloom": {
      const { SignalBloomMusicLyricVisualizer } = await import("./MusicLyricSignalBloomVisualizer");
      return new SignalBloomMusicLyricVisualizer(canvas);
    }
    case "sketchbook":
    default: {
      const { SketchbookMusicLyricVisualizer } = await import("./MusicLyricVisualizer");
      return new SketchbookMusicLyricVisualizer(canvas);
    }
  }
}
