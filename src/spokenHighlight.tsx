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

  const isCurrentPassage =
    Boolean(progress?.active) &&
    (!passageId ||
      progress?.passageId === passageId ||
      Boolean(progress?.passageId?.startsWith(`${passageId}-`)) ||
      Boolean(passageId?.startsWith(`${progress?.passageId}-`)));

  if (!isCurrentPassage || !progress) {
    return <span className={className}>{text}</span>;
  }

  const activeIndex = getActiveWordIndex(
    progress.text || text,
    progress.seconds,
    progress.duration,
    progress.timings,
  );

  const tokens = text.split(/(\s+)/);
  let wordCounter = 0;

  return (
    <span className={`speech-passage-speaking ${className}`}>
      {tokens.map((token, index) => {
        if (/^\s+$/.test(token)) {
          return token;
        }
        const currentWordIndex = wordCounter++;
        const isActive = currentWordIndex === activeIndex;
        const isPast = currentWordIndex < activeIndex;

        if (isActive) {
          return (
            <mark
              key={index}
              className="speech-word-active"
              ref={(node) => {
                if (node && typeof node.scrollIntoView === "function") {
                  node.scrollIntoView({ behavior: "smooth", block: "nearest", inline: "nearest" });
                }
              }}
            >
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
      })}
    </span>
  );
}
