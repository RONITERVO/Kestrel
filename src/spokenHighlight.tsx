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

/**
 * Extracts pure spoken words matching renderHighlightedTokens word boundaries.
 */
export function extractSpeechWords(str: string): string[] {
  if (!str) return [];
  return (str.match(/[\p{L}\p{N}]+/gu) ?? []).map((w) => w.toLowerCase());
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
  if (seconds < words[0].start) {
    return 0;
  }
  if (seconds >= words[words.length - 1].start) {
    return words.length - 1;
  }
  const found = words.findIndex((word) => seconds >= word.start && seconds < word.end);
  if (found >= 0) return found;
  for (let i = 0; i < words.length - 1; i++) {
    if (seconds >= words[i].start && seconds < words[i + 1].start) {
      return i;
    }
  }
  return words.length - 1;
}

export function isWordToken(token: string): boolean {
  return /[\p{L}\p{N}]/u.test(token);
}

export interface BlockHighlightContext {
  activeWordIndex: number;
  tracker: WordOffsetTracker;
}

export interface CandidateBlock {
  id: string;
  text: string;
}

let lastResolvedResult: { activeId: string; activeWordIndex: number } | null = null;
let lastResolvedPassageId: string | null = null;

/**
 * Resolves which candidate block and which word index inside that block
 * corresponds to the currently spoken word at progress.seconds.
 * Tolerant to inserted grammatical expansions and smoothly bridges frame boundaries.
 */
export function resolveActiveBlockAndWord(
  candidates: CandidateBlock[],
  progress?: SpeechProgressState | null,
): { activeId: string; activeWordIndex: number } | null {
  if (!progress || !progress.active || !progress.text || candidates.length === 0) {
    lastResolvedResult = null;
    lastResolvedPassageId = null;
    return null;
  }

  if (lastResolvedPassageId !== progress.passageId) {
    lastResolvedResult = null;
    lastResolvedPassageId = progress.passageId;
  }

  const progWords = extractSpeechWords(progress.text);
  if (progWords.length === 0) return null;

  const rawIdx = getActiveWordIndex(
    progress.text,
    progress.seconds,
    progress.duration,
    progress.timings,
  );

  if (rawIdx < 0) {
    return null;
  }

  const currentProgIdx = Math.min(progWords.length - 1, Math.max(0, rawIdx));
  let targetWord = progWords[currentProgIdx];

  let bestCandidateId: string | null = null;
  let bestWordIndexInBlock = -1;
  let bestScore = -1;

  for (const candidate of candidates) {
    const blockWords = extractSpeechWords(candidate.text);
    if (blockWords.length === 0) continue;

    let effectiveTargetWord = targetWord;
    if (!blockWords.includes(effectiveTargetWord)) {
      for (let delta = 1; delta <= 3; delta++) {
        if (currentProgIdx + delta < progWords.length && blockWords.includes(progWords[currentProgIdx + delta])) {
          effectiveTargetWord = progWords[currentProgIdx + delta];
          break;
        }
        if (currentProgIdx - delta >= 0 && blockWords.includes(progWords[currentProgIdx - delta])) {
          effectiveTargetWord = progWords[currentProgIdx - delta];
          break;
        }
      }
    }

    for (let bIdx = 0; bIdx < blockWords.length; bIdx++) {
      if (blockWords[bIdx] !== effectiveTargetWord) continue;

      let score = 10;
      let left = 1;
      while (
        currentProgIdx - left >= 0 &&
        bIdx - left >= 0 &&
        progWords[currentProgIdx - left] === blockWords[bIdx - left]
      ) {
        score += 10;
        left++;
      }

      let right = 1;
      while (
        currentProgIdx + right < progWords.length &&
        bIdx + right < blockWords.length &&
        progWords[currentProgIdx + right] === blockWords[bIdx + right]
      ) {
        score += 10;
        right++;
      }

      if (score > bestScore) {
        bestScore = score;
        bestCandidateId = candidate.id;
        bestWordIndexInBlock = Math.min(blockWords.length - 1, Math.max(0, bIdx));
      }
    }
  }

  if (bestCandidateId && bestScore >= 5 && bestWordIndexInBlock >= 0) {
    const result = {
      activeId: bestCandidateId,
      activeWordIndex: bestWordIndexInBlock,
    };
    lastResolvedResult = result;
    return result;
  }

  // Boundary bridge: maintain steady visual focus across audio block tails
  if (lastResolvedResult && progress.seconds > 0) {
    return lastResolvedResult;
  }

  return null;
}

export function isPassageActiveForText(
  text: string,
  passageId?: string,
  progress?: SpeechProgressState | null,
): boolean {
  if (!progress || !progress.active) return false;

  if (passageId) {
    if (progress.passageId === passageId) return true;
    if (progress.passageId.startsWith(`${passageId}-`)) return true;
    return false;
  }

  const normText = normalizeSpeechMatchingText(text);
  const normProgress = normalizeSpeechMatchingText(progress.text);
  if (!normText || !normProgress) return false;

  return (
    normText === normProgress ||
    normProgress.includes(normText) ||
    normText.includes(normProgress)
  );
}

export interface WordOffsetTracker {
  current: number;
}

export function renderHighlightedTokens(
  text: string,
  activeWordIndex: number,
  tracker?: WordOffsetTracker,
): ReactNode[] {
  if (!text) return [];
  const tokens = text.split(/([^\p{L}\p{N}]+)/gu);
  const offset = tracker ?? { current: 0 };

  return tokens.map((token, index) => {
    if (!token) return null;

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

  const resolved = resolveActiveBlockAndWord([{ id: "spoken-target", text }], progress);
  const activeIndex = resolved ? resolved.activeWordIndex : -1;

  return (
    <span className={`speech-passage-speaking ${className}`}>
      {renderHighlightedTokens(text, activeIndex)}
    </span>
  );
}
