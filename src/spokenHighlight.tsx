import { type ReactNode } from "react";
import type { SpeechTiming } from "./types";

export interface SpeechProgressState {
  active: boolean;
  sourceKind?: string;
  sourceId?: string;
  passageId: string;
  text: string;
  seconds: number;
  duration: number;
  timings: SpeechTiming[];
}

export function normalizeSpeechMatchingText(str: string): string {
  return str.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");
}

export function wordTimings(text: string, duration: number): SpeechTiming[] {
  const words = text.match(/\S+/g) ?? [];
  const weights = words.map((word) => Math.max(1, word.replace(/[^\p{L}\p{N}]/gu, "").length));
  const total = weights.reduce((sum, value) => sum + value, 0) || 1;
  let cursor = 0;
  return words.map((value, index) => {
    const start = (duration * cursor) / total;
    cursor += weights[index];
    return { value, start, end: (duration * cursor) / total };
  });
}

export function getActiveWordIndex(
  text: string,
  seconds: number,
  duration: number,
  exact: SpeechTiming[] = [],
): number {
  if (seconds < 0) return -1;
  const words = exact.length ? exact : wordTimings(text, duration || Math.max(1, text.length / 15));
  if (!words.length) return -1;
  const found = words.findIndex((word) => seconds >= word.start && seconds < word.end);
  if (found >= 0) return found;
  if (seconds > 0 && seconds >= (words[words.length - 1]?.end ?? 0)) {
    return words.length - 1;
  }
  return 0;
}

export function isWordToken(token: string): boolean {
  return /[\p{L}\p{N}]/u.test(token);
}

export function isPassageActiveForText(
  text: string,
  passageId?: string,
  progress?: SpeechProgressState | null,
): boolean {
  if (!progress || !progress.active) return false;

  // 1. If explicit passageId is provided (e.g. Research Reader sections/paragraphs)
  if (passageId) {
    if (progress.passageId === passageId) return true;
    if (progress.passageId.startsWith(`${passageId}-`)) return true;
    return false;
  }

  // 2. Chat / Markdown blocks without explicit passageId:
  const normText = normalizeSpeechMatchingText(text);
  const normProgress = normalizeSpeechMatchingText(progress.text);
  if (!normText || !normProgress) return false;

  // Must have at least 6 characters of normalized text to prevent single-word false positives
  if (normText.length < 6 && normProgress.length >= 6) {
    return normProgress === normText || normProgress.startsWith(normText);
  }

  // Exact match
  if (normText === normProgress) return true;

  // Significant containment (one contains the other)
  if (normProgress.startsWith(normText) || normText.startsWith(normProgress)) return true;
  if (normProgress.includes(normText) && normText.length >= 10) return true;
  if (normText.includes(normProgress) && normProgress.length >= 10) return true;

  return false;
}

export interface WordOffsetTracker {
  current: number;
}

export function renderHighlightedTokens(
  text: string,
  activeWordIndex: number,
  tracker?: WordOffsetTracker,
): ReactNode[] {
  const tokens = text.split(/(\s+)/);
  const offset = tracker ?? { current: 0 };

  return tokens.map((token, index) => {
    if (/^\s+$/.test(token)) {
      return token;
    }

    // Pure punctuation/border/symbol tokens do not advance the word counter
    if (!isWordToken(token)) {
      return (
        <span key={index} className="speech-symbol-token">
          {token}
        </span>
      );
    }

    const currentWordIndex = offset.current++;
    const isWordActive = currentWordIndex === activeWordIndex;
    const isPast = currentWordIndex < activeWordIndex;

    if (isWordActive) {
      return (
        <mark key={index} className="speech-word-active">
          {token}
        </mark>
      );
    }

    return (
      <span
        key={index}
        className={isPast ? "speech-word-spoken" : "speech-word-pending"}
      >
        {token}
      </span>
    );
  });
}

export function SpokenText({
  text,
  passageId,
  progress,
  className = "",
}: {
  text: string;
  passageId?: string;
  progress?: SpeechProgressState | null;
  className?: string;
}) {
  if (!text) return null;

  const isActive = isPassageActiveForText(text, passageId, progress);
  if (!isActive || !progress) {
    return <span className={className}>{text}</span>;
  }

  const activeIndex = getActiveWordIndex(
    progress.text || text,
    progress.seconds,
    progress.duration,
    progress.timings,
  );

  return (
    <span className={`speech-passage-speaking ${className}`}>
      {renderHighlightedTokens(text, activeIndex)}
    </span>
  );
}
