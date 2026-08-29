import { describe, expect, it } from "vitest";
import {
  clampFiniteMusicLyricTime,
  estimatedMusicLyricWords,
  extractLyricsForRange,
  reconcileMusicLyricWords,
  truncateUtf8,
  utf8ByteLength,
} from "./MusicLyricsTiming";

describe("MusicLyricsTiming", () => {
  it("truncates producer prompts by UTF-8 bytes without splitting a character", () => {
    const source = `${"歌".repeat(200)} ending`;
    const result = truncateUtf8(source, 512);

    expect(utf8ByteLength(result)).toBeLessThanOrEqual(512);
    expect(source.startsWith(result)).toBe(true);
    expect(result.endsWith("�")).toBe(false);
  });

  it("preserves word timing when a text edit keeps the same token count", () => {
    const existing = [
      { value: "old", start: 1.2, end: 1.8 },
      { value: "line", start: 1.9, end: 2.5 },
    ];

    expect(reconcileMusicLyricWords("new words", 1, 3, existing)).toEqual([
      { value: "new", start: 1.2, end: 1.8 },
      { value: "words", start: 1.9, end: 2.5 },
    ]);
    expect(estimatedMusicLyricWords("one two three", 1, 4)).toEqual([
      { value: "one", start: 1, end: 2 },
      { value: "two", start: 2, end: 3 },
      { value: "three", start: 3, end: 4 },
    ]);
  });

  it("extracts bounded lyric guidance and sanitizes non-finite timing edits", () => {
    expect(extractLyricsForRange("[Verse]\nfirst\nsecond\n(aside)\nthird", 0, 5, 10))
      .toBe("first second");
    expect(clampFiniteMusicLyricTime(Number.NaN, 0, 10, 4)).toBe(4);
    expect(clampFiniteMusicLyricTime(12, 0, 10, 4)).toBe(10);
  });
});
