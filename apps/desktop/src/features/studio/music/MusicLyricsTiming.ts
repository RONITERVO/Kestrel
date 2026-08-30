import type { MusicLyricSegment, MusicLyricWord } from "../../../contracts/index";

export function musicLyricSegmentAt(
  segments: MusicLyricSegment[],
  seconds: number,
): MusicLyricSegment | undefined {
  return segments.find((segment, index) => seconds >= segment.start
    && (seconds < segment.end || index === segments.length - 1 && seconds <= segment.end));
}

export function musicLyricDisplaySegmentAt(
  segments: MusicLyricSegment[],
  seconds: number,
): MusicLyricSegment | undefined {
  const active = musicLyricSegmentAt(segments, seconds);
  if (active) return active;
  for (let index = segments.length - 1; index >= 0; index -= 1) {
    const segment = segments[index];
    if (seconds > segment.end && seconds <= segment.end + 0.42) return segment;
  }
  return undefined;
}

export function extractLyricsForRange(
  lyrics: string,
  start: number,
  end: number,
  totalDuration: number,
): string {
  const lines = lyrics
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("[") && !line.startsWith("{") && !line.startsWith("("));
  if (!lines.length) return "";
  const duration = Math.max(0.01, totalDuration);
  const startFraction = Math.max(0, start / duration);
  const endFraction = Math.min(1, end / duration);
  const startIndex = Math.floor(startFraction * lines.length);
  const endIndex = Math.min(lines.length, Math.ceil(endFraction * lines.length));
  return lines.slice(startIndex, Math.max(startIndex + 1, endIndex)).join(" ");
}

export function estimatedMusicLyricWords(
  primary: string,
  start: number,
  end: number,
): MusicLyricWord[] {
  const words = primary.trim().split(/\s+/u).filter(Boolean);
  if (!words.length) return [];
  const duration = Math.max(0.05, end - start);
  const step = duration / words.length;
  return words.map((value, index) => ({
    value,
    start: roundMusicLyricTime(start + step * index),
    end: roundMusicLyricTime(start + step * (index + 1)),
  }));
}

export function reconcileMusicLyricWords(
  primary: string,
  start: number,
  end: number,
  existing: MusicLyricWord[],
): MusicLyricWord[] {
  const values = primary.trim().split(/\s+/u).filter(Boolean);
  if (values.length !== existing.length) return estimatedMusicLyricWords(primary, start, end);
  return existing.map((word, index) => ({ ...word, value: values[index] }));
}

export function wordProgress(start: number, end: number, currentTime: number): number {
  if (currentTime <= start) return 0;
  if (currentTime >= end || end <= start) return 1;
  return (currentTime - start) / (end - start);
}

export function newMusicLyricId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (character) => {
    const random = Math.floor(Math.random() * 16);
    return (character === "x" ? random : (random & 0x3) | 0x8).toString(16);
  });
}

export function roundMusicLyricTime(seconds: number): number {
  return Math.round(seconds * 100) / 100;
}

export function clampFiniteMusicLyricTime(
  value: number,
  minimum: number,
  maximum: number,
  fallback: number,
): number {
  if (!Number.isFinite(value)) return fallback;
  return Math.max(minimum, Math.min(maximum, value));
}

export function utf8ByteLength(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

export function truncateUtf8(value: string, maximumBytes: number): string {
  if (utf8ByteLength(value) <= maximumBytes) return value;
  let used = 0;
  let result = "";
  for (const character of value) {
    const bytes = utf8ByteLength(character);
    if (used + bytes > maximumBytes) break;
    result += character;
    used += bytes;
  }
  return result;
}

export function formatMusicLyricTime(seconds: number): string {
  if (!Number.isFinite(seconds)) return "00:00";
  const safe = Math.max(0, seconds);
  const minutes = Math.floor(safe / 60);
  return `${minutes.toString().padStart(2, "0")}:${Math.floor(safe % 60).toString().padStart(2, "0")}`;
}

export function formatPreciseMusicLyricTime(seconds: number): string {
  const safe = Math.max(0, Number.isFinite(seconds) ? seconds : 0);
  const minutes = Math.floor(safe / 60);
  return `${minutes.toString().padStart(2, "0")}:${(safe % 60).toFixed(1).padStart(4, "0")}`;
}
