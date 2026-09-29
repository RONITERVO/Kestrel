import { useEffect, useMemo, useRef, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import type { SpeechTiming } from "../../contracts/index";
import "./markdown.css";

export interface SpeechProgressState {
  active: boolean;
  sourceKind?: string;
  sourceId?: string;
  passageId: string;
  text: string;
  seconds: number;
  duration: number;
  timings: SpeechTiming[];
  /** Seeks the loaded private audio to an exact word timestamp and starts playback. */
  onSeek?: (seconds: number) => void;
  /** Every passage whose private audio and exact word timings are already available. */
  seekablePassages?: SpeechSeekPassage[];
  onSeekPassage?: (passageId: string, seconds: number) => void;
}

export interface SpeechSeekPassage {
  passageId: string;
  text: string;
  timings: SpeechTiming[];
}

export interface SpeechWordSeekTarget {
  passageId: string;
  seconds: number;
}

export type SpeechSeekTargetMap = Map<string, Map<number, SpeechWordSeekTarget>>;

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

interface TimingAlignment {
  text: string;
  indices: number[];
  sourceWords: string[];
  timingWords: string[];
}

const timingAlignmentCache = new WeakMap<SpeechTiming[], TimingAlignment>();

function tokenSubstitutionCost(left: string, right: string): number {
  if (left === right) return 0;
  if (left.length >= 3 && right.length >= 3 && (left.startsWith(right) || right.startsWith(left))) {
    return 0.35;
  }
  return 1;
}

/**
 * Maps Whisper timing entries onto the words in the producer-visible source text. Whisper may
 * expand one written token into several spoken words (19.8 -> "nineteen point eight") or omit a
 * symbol. Sequence alignment keeps later highlights anchored instead of assuming equal indexes.
 */
function speechTimingAlignment(text: string, timings: SpeechTiming[]): TimingAlignment {
  if (!timings.length) return { text, indices: [], sourceWords: extractSpeechWords(text), timingWords: [] };
  const cached = timingAlignmentCache.get(timings);
  if (cached?.text === text) return cached;

  const sourceWords = extractSpeechWords(text);
  const timingWords = timings.map((timing) => normalizeSpeechMatchingText(timing.value));
  if (!sourceWords.length) {
    return { text, indices: timings.map(() => -1), sourceWords, timingWords };
  }
  const sourceCount = sourceWords.length;
  const timingCount = timingWords.length;
  const gapCost = 0.7;
  const costs = Array.from({ length: sourceCount + 1 }, () => new Float64Array(timingCount + 1));
  const moves = Array.from({ length: sourceCount + 1 }, () => new Uint8Array(timingCount + 1));
  for (let sourceIndex = 1; sourceIndex <= sourceCount; sourceIndex++) {
    costs[sourceIndex][0] = sourceIndex * gapCost;
    moves[sourceIndex][0] = 1;
  }
  for (let timingIndex = 1; timingIndex <= timingCount; timingIndex++) {
    costs[0][timingIndex] = timingIndex * gapCost;
    moves[0][timingIndex] = 2;
  }

  for (let sourceIndex = 1; sourceIndex <= sourceCount; sourceIndex++) {
    for (let timingIndex = 1; timingIndex <= timingCount; timingIndex++) {
      const diagonal = costs[sourceIndex - 1][timingIndex - 1]
        + tokenSubstitutionCost(sourceWords[sourceIndex - 1], timingWords[timingIndex - 1]);
      const sourceOnly = costs[sourceIndex - 1][timingIndex] + gapCost;
      const timingOnly = costs[sourceIndex][timingIndex - 1] + gapCost;
      if (diagonal <= sourceOnly && diagonal <= timingOnly) {
        costs[sourceIndex][timingIndex] = diagonal;
        moves[sourceIndex][timingIndex] = 0;
      } else if (sourceOnly <= timingOnly) {
        costs[sourceIndex][timingIndex] = sourceOnly;
        moves[sourceIndex][timingIndex] = 1;
      } else {
        costs[sourceIndex][timingIndex] = timingOnly;
        moves[sourceIndex][timingIndex] = 2;
      }
    }
  }

  const indices = Array<number>(timingCount).fill(-1);
  let sourceIndex = sourceCount;
  let timingIndex = timingCount;
  while (sourceIndex > 0 || timingIndex > 0) {
    const move = moves[sourceIndex][timingIndex];
    if (sourceIndex > 0 && timingIndex > 0 && move === 0) {
      indices[timingIndex - 1] = sourceIndex - 1;
      sourceIndex -= 1;
      timingIndex -= 1;
    } else if (sourceIndex > 0 && (timingIndex === 0 || move === 1)) {
      sourceIndex -= 1;
    } else {
      timingIndex -= 1;
    }
  }
  for (let index = 0; index < indices.length; index++) {
    if (indices[index] >= 0) continue;
    let previous = index - 1;
    while (previous >= 0 && indices[previous] < 0) previous -= 1;
    let next = index + 1;
    while (next < indices.length && indices[next] < 0) next += 1;
    if (previous >= 0 && next < indices.length) {
      const fraction = (index - previous) / (next - previous);
      indices[index] = Math.round(indices[previous] + fraction * (indices[next] - indices[previous]));
    } else if (previous >= 0) {
      indices[index] = indices[previous];
    } else if (next < indices.length) {
      indices[index] = indices[next];
    } else {
      indices[index] = 0;
    }
  }

  for (let index = 0; index < indices.length; index++) {
    indices[index] = Math.min(sourceCount - 1, Math.max(index > 0 ? indices[index - 1] : 0, indices[index]));
  }
  const alignment = { text, indices, sourceWords, timingWords };
  timingAlignmentCache.set(timings, alignment);
  return alignment;
}

export function mapSpeechTimingsToTextWords(text: string, timings: SpeechTiming[]): number[] {
  return speechTimingAlignment(text, timings).indices;
}

/**
 * Resolves a producer-visible word back to its exact aligned audio timestamp. The rendered text
 * may be one Markdown block inside the larger spoken passage, so nearby words and the current
 * playback position disambiguate repeated words without inventing an estimated timestamp.
 */
export function speechWordStart(
  passageText: string,
  renderedText: string,
  renderedWordIndex: number,
  timings: SpeechTiming[],
  referenceSeconds = 0,
): number | null {
  if (!timings.length || !Number.isInteger(renderedWordIndex) || renderedWordIndex < 0) return null;
  const passageWords = extractSpeechWords(passageText);
  const renderedWords = extractSpeechWords(renderedText);
  const clickedWord = renderedWords[renderedWordIndex];
  if (!clickedWord || !passageWords.length) return null;

  const alignment = speechTimingAlignment(passageText, timings);
  const referenceTimingIndex = getActiveWordIndex(passageText, referenceSeconds, 0, timings);
  const referenceSourceIndex = alignment.indices[referenceTimingIndex] ?? 0;
  let bestSourceIndex = -1;
  let bestContextScore = -1;
  let bestDistance = Number.POSITIVE_INFINITY;

  for (let sourceIndex = 0; sourceIndex < passageWords.length; sourceIndex++) {
    if (passageWords[sourceIndex] !== clickedWord) continue;
    let contextScore = 0;
    for (let delta = -4; delta <= 4; delta++) {
      const renderedIndex = renderedWordIndex + delta;
      const passageIndex = sourceIndex + delta;
      if (
        renderedIndex >= 0
        && renderedIndex < renderedWords.length
        && passageIndex >= 0
        && passageIndex < passageWords.length
        && renderedWords[renderedIndex] === passageWords[passageIndex]
      ) {
        contextScore += 5 - Math.abs(delta);
      }
    }
    const distance = Math.abs(sourceIndex - referenceSourceIndex);
    if (contextScore > bestContextScore || (contextScore === bestContextScore && distance < bestDistance)) {
      bestSourceIndex = sourceIndex;
      bestContextScore = contextScore;
      bestDistance = distance;
    }
  }

  if (bestSourceIndex < 0) return null;
  let exactStart = Number.POSITIVE_INFINITY;
  for (let timingIndex = 0; timingIndex < timings.length; timingIndex++) {
    if (alignment.indices[timingIndex] !== bestSourceIndex) continue;
    const start = timings[timingIndex].start;
    if (Number.isFinite(start)) exactStart = Math.min(exactStart, start);
  }
  return Number.isFinite(exactStart) ? exactStart : null;
}

/** Returns the producer-visible end of a clip, excluding a model-generated tail after the final
 * exact source word. The preserved Opus master remains unchanged and seekable. */
export function speechPlaybackEnd(text: string, timings: SpeechTiming[], duration: number): number {
  if (!Number.isFinite(duration) || duration <= 0 || timings.length < 2) return duration;
  const alignment = speechTimingAlignment(text, timings);
  const lastSourceIndex = alignment.sourceWords.length - 1;
  if (lastSourceIndex < 0) return duration;
  let finalAnchor = -1;
  let bestSuffixScore = 0;
  for (let index = 0; index < alignment.timingWords.length; index++) {
    if (alignment.timingWords[index] !== alignment.sourceWords[lastSourceIndex]) continue;
    let score = 0;
    while (
      index - score >= 0
      && lastSourceIndex - score >= 0
      && alignment.timingWords[index - score] === alignment.sourceWords[lastSourceIndex - score]
    ) {
      score += 1;
    }
    if (score > bestSuffixScore) {
      bestSuffixScore = score;
      finalAnchor = index;
    }
  }
  const requiredSuffix = Math.min(2, alignment.sourceWords.length);
  if (finalAnchor < 0 || bestSuffixScore < requiredSuffix || finalAnchor >= timings.length - 1) return duration;
  const trailingWords = timings.length - finalAnchor - 1;
  if (trailingWords < 2) return duration;
  return Math.min(duration, timings[finalAnchor].end + 0.35);
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

export interface CandidateBlock {
  id: string;
  text: string;
}

export type HighlightResolution = {
  activeId: string;
  activeWordIndex: number;
  /** Position of the highlighted word among all visible words, used to read on from here. */
  documentWord?: number;
};

interface DocumentWords {
  words: string[];
  block: Int32Array;
  offset: Int32Array;
}

const documentWordsCache = new WeakMap<CandidateBlock[], DocumentWords>();

function documentWords(candidates: CandidateBlock[]): DocumentWords {
  const cached = documentWordsCache.get(candidates);
  if (cached) return cached;
  const words: string[] = [];
  const blocks: number[] = [];
  const offsets: number[] = [];
  candidates.forEach((candidate, blockIndex) => {
    extractSpeechWords(candidate.text).forEach((word, offset) => {
      words.push(word);
      blocks.push(blockIndex);
      offsets.push(offset);
    });
  });
  const document = { words, block: Int32Array.from(blocks), offset: Int32Array.from(offsets) };
  documentWordsCache.set(candidates, document);
  return document;
}

interface PassageAnchor {
  /** The visible word each passage word stands for, or -1 for words only spoken ("equals"). */
  positions: Int32Array;
  /** Whether enough of the passage was found to trust the placement. */
  placed: boolean;
}

const passageAnchorCache = new WeakMap<CandidateBlock[], Map<string, PassageAnchor>>();
// Beyond this many visible words, a passage is placed within a window around the reading
// position so a very long document stays cheap to align.
const MAX_ANCHOR_WINDOW = 15_000;
const SPOKEN_ONLY_COST = 0.55;
const VISIBLE_ONLY_COST = 0.55;

/**
 * Places a spoken passage in the visible document by aligning all of its words in order. Words
 * that are only spoken ("equals", "point", "Code block on screen") and visible words that are
 * not spoken are gaps, so a passage full of numbers lands on its own blocks instead of on
 * whichever table repeats the same digits. `hint` (the word being read before this passage)
 * decides between equally good places, preferring to read on rather than jump back.
 */
function anchorPassage(candidates: CandidateBlock[], text: string, hint: number): PassageAnchor {
  let anchors = passageAnchorCache.get(candidates);
  if (!anchors) {
    anchors = new Map();
    passageAnchorCache.set(candidates, anchors);
  }
  const key = `${hint}\u0000${text}`;
  const cached = anchors.get(key);
  if (cached) return cached;

  const document = documentWords(candidates);
  const spoken = extractSpeechWords(text);
  const positions = new Int32Array(spoken.length).fill(-1);
  const windowStart = document.words.length > MAX_ANCHOR_WINDOW
    ? Math.max(0, Math.min(hint - 3_000, document.words.length - MAX_ANCHOR_WINDOW))
    : 0;
  const visible = document.words.slice(windowStart, windowStart + MAX_ANCHOR_WINDOW);
  const rows = spoken.length;
  const columns = visible.length;
  const width = columns + 1;
  const costs = new Float32Array((rows + 1) * width);
  const moves = new Uint8Array((rows + 1) * width);
  const starts = new Int32Array((rows + 1) * width);
  // The passage may begin anywhere in the document at no cost.
  for (let column = 0; column <= columns; column++) starts[column] = column;
  for (let row = 1; row <= rows; row++) {
    const here = row * width;
    const above = (row - 1) * width;
    costs[here] = row * SPOKEN_ONLY_COST;
    moves[here] = 1;
    for (let column = 1; column <= columns; column++) {
      const diagonal = costs[above + column - 1]
        + tokenSubstitutionCost(spoken[row - 1], visible[column - 1]);
      const spokenOnly = costs[above + column] + SPOKEN_ONLY_COST;
      const visibleOnly = costs[here + column - 1] + VISIBLE_ONLY_COST;
      if (diagonal <= spokenOnly && diagonal <= visibleOnly) {
        costs[here + column] = diagonal;
        moves[here + column] = 0;
        starts[here + column] = starts[above + column - 1];
      } else if (spokenOnly <= visibleOnly) {
        costs[here + column] = spokenOnly;
        moves[here + column] = 1;
        starts[here + column] = starts[above + column];
      } else {
        costs[here + column] = visibleOnly;
        moves[here + column] = 2;
        starts[here + column] = starts[here + column - 1];
      }
    }
  }

  // The passage may also end anywhere. Among equally good places, read on from the hint.
  const last = rows * width;
  let end = 0;
  let best = Number.POSITIVE_INFINITY;
  for (let column = 0; column <= columns; column++) {
    const start = windowStart + starts[last + column];
    const bias = start >= hint ? (start - hint) * 1e-6 : 0.25 + (hint - start) * 1e-6;
    if (costs[last + column] + bias < best) {
      best = costs[last + column] + bias;
      end = column;
    }
  }

  let matched = 0;
  let row = rows;
  let column = end;
  while (row > 0) {
    const move = column > 0 ? moves[row * width + column] : 1;
    if (move === 0) {
      if (tokenSubstitutionCost(spoken[row - 1], visible[column - 1]) < 1) {
        positions[row - 1] = windowStart + column - 1;
        matched += 1;
      }
      row -= 1;
      column -= 1;
    } else if (move === 1) {
      row -= 1;
    } else {
      column -= 1;
    }
  }
  // A lone common word ("on" in "Code block on screen") is not a placement; most of a passage,
  // or most of a short visible text the passage covers, is.
  const placed = matched >= Math.min(2, rows)
    && (matched >= rows * 0.25 || matched >= columns * 0.5);
  const anchor = { positions, placed };
  anchors.set(key, anchor);
  return anchor;
}

function resolutionAt(candidates: CandidateBlock[], documentWord: number): HighlightResolution | null {
  const document = documentWords(candidates);
  if (documentWord < 0 || documentWord >= document.words.length) return null;
  return {
    activeId: candidates[document.block[documentWord]].id,
    activeWordIndex: document.offset[documentWord],
    documentWord,
  };
}

export function speechResolutionCacheKey(progress: SpeechProgressState): string {
  return `${progress.sourceKind ?? "unknown"}\u0000${progress.sourceId ?? "unknown"}\u0000${progress.passageId}`;
}

/**
 * Resolves which candidate block and which word index inside that block corresponds to the
 * currently spoken word at progress.seconds. The passage is first placed in the document as a
 * whole, so the highlight follows the passage being read and holds still on words that are only
 * spoken ("equals", "point") or on audio the voice added after the text, instead of jumping to a
 * matching word elsewhere in the reply. `hint` is the word read before this passage began.
 */
export function resolveActiveBlockAndWord(
  candidates: CandidateBlock[],
  progress?: SpeechProgressState | null,
  previous?: HighlightResolution | null,
  hint = previous?.documentWord ?? 0,
): HighlightResolution | null {
  const canResolve = Boolean(progress?.active || (progress?.onSeek && progress.timings.length));
  if (!progress || !canResolve || !progress.text || candidates.length === 0) {
    return null;
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

  const timingMap = progress.timings.length
    ? mapSpeechTimingsToTextWords(progress.text, progress.timings)
    : [];
  const currentProgIdx = Math.min(
    progWords.length - 1,
    Math.max(0, timingMap[rawIdx] ?? rawIdx),
  );

  // Hold still across passages the document does not show ("Code block on screen.").
  const hold = () => previous ?? (progress.seconds > 0 && hint > 0 ? resolutionAt(candidates, hint) : null);
  const anchor = anchorPassage(candidates, progress.text, hint);
  if (!anchor.placed) return hold();

  // A spoken-only word keeps the highlight on the last shown word before it.
  let documentWord = -1;
  for (let index = currentProgIdx; index >= 0 && documentWord < 0; index--) {
    documentWord = anchor.positions[index];
  }
  for (let index = currentProgIdx + 1; index < progWords.length && documentWord < 0; index++) {
    documentWord = anchor.positions[index];
  }
  return documentWord >= 0 ? resolutionAt(candidates, documentWord) : hold();
}

/** Maps every exact cached timing onto its visible Markdown block and word. Passages are placed
 * in the order given, each reading on from where the previous one was found. */
export function buildSpeechSeekTargets(
  candidates: CandidateBlock[],
  passages: SpeechSeekPassage[],
): SpeechSeekTargetMap {
  const targets: SpeechSeekTargetMap = new Map();
  const document = documentWords(candidates);
  let hint = 0;

  for (const passage of passages) {
    if (!passage.text || !passage.timings.length) continue;
    const anchor = anchorPassage(candidates, passage.text, hint);
    if (!anchor.placed) continue;
    const timingMap = mapSpeechTimingsToTextWords(passage.text, passage.timings);

    for (let timingIndex = 0; timingIndex < passage.timings.length; timingIndex++) {
      const timing = passage.timings[timingIndex];
      if (!Number.isFinite(timing.start)) continue;
      const documentWord = anchor.positions[timingMap[timingIndex] ?? -1] ?? -1;
      if (documentWord < 0) continue;
      hint = Math.max(hint, documentWord);
      const blockId = candidates[document.block[documentWord]].id;
      let blockTargets = targets.get(blockId);
      if (!blockTargets) {
        blockTargets = new Map();
        targets.set(blockId, blockTargets);
      }
      const wordIndex = document.offset[documentWord];
      if (!blockTargets.has(wordIndex)) {
        blockTargets.set(wordIndex, { passageId: passage.passageId, seconds: timing.start });
      }
    }
  }

  return targets;
}

export function useSpeechSeekTargets(
  candidates: CandidateBlock[],
  progress?: SpeechProgressState | null,
  passageIdPrefix?: string,
): SpeechSeekTargetMap {
  return useMemo(() => {
    if (!progress) return new Map();
    const cached = (progress.seekablePassages ?? []).filter((passage) => (
      !passageIdPrefix
      || passage.passageId === passageIdPrefix
      || passage.passageId.startsWith(`${passageIdPrefix}-`)
    ));
    const passages = [...cached];
    if (
      progress.onSeek
      && progress.timings.length
      && !passages.some((passage) => passage.passageId === progress.passageId)
    ) {
      passages.push({
        passageId: progress.passageId,
        text: progress.text,
        timings: progress.timings,
      });
    }
    return buildSpeechSeekTargets(candidates, passages);
  }, [candidates, passageIdPrefix, progress?.onSeek, progress?.passageId, progress?.seekablePassages, progress?.text, progress?.timings]);
}

export function useResolvedSpeechHighlight(
  candidates: CandidateBlock[],
  progress?: SpeechProgressState | null,
): HighlightResolution | null {
  const cacheRef = useRef(new Map<string, HighlightResolution>());
  const lastWordRef = useRef(0);
  const key = progress ? speechResolutionCacheKey(progress) : null;
  // Each passage is placed reading on from the last word highlighted before it started.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const hint = useMemo(() => lastWordRef.current, [key, candidates]);
  const resolved = useMemo(
    () => resolveActiveBlockAndWord(candidates, progress, key ? cacheRef.current.get(key) : null, hint),
    [
      candidates,
      hint,
      key,
      progress?.active,
      progress?.duration,
      progress?.seconds,
      progress?.text,
      progress?.timings,
      progress?.onSeek,
    ],
  );
  useEffect(() => {
    cacheRef.current.clear();
    if (!key) {
      return;
    }
    if (resolved) {
      cacheRef.current.set(key, resolved);
      if (resolved.documentWord !== undefined) lastWordRef.current = resolved.documentWord;
    }
  }, [key, resolved]);
  return resolved;
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
  onWordClick?: (wordIndex: number) => void,
  canSeekWord?: (wordIndex: number) => boolean,
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
    const wordStateClass = activeWordIndex < 0
      ? "speech-word-idle"
      : isPast ? "speech-word-spoken" : "speech-word-pending";
    const isSeekable = Boolean(onWordClick && (!canSeekWord || canSeekWord(currentWordIndex)));
    const seekProps = isSeekable ? {
      role: "button",
      tabIndex: 0,
      title: `Play from “${token}”`,
      "aria-label": `Play from ${token}`,
      onClick: (event: MouseEvent<HTMLElement>) => {
        event.preventDefault();
        event.stopPropagation();
        onWordClick?.(currentWordIndex);
      },
      onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
        if (event.key !== "Enter" && event.key !== " ") return;
        event.preventDefault();
        event.stopPropagation();
        onWordClick?.(currentWordIndex);
      },
    } : {};

    if (isWordActive) {
      return (
        <mark key={index} className={`speech-word-active${isSeekable ? " speech-word-seekable" : ""}`} {...seekProps}>
          {token}
        </mark>
      );
    }

    return (
      <span
        key={index}
        className={`${wordStateClass}${isSeekable ? " speech-word-seekable" : ""}`}
        {...seekProps}
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
  const candidates = useMemo(() => [{ id: "spoken-target", text }], [text]);
  const resolved = useResolvedSpeechHighlight(candidates, progress);
  const seekTargets = useSpeechSeekTargets(candidates, progress, passageId);
  if (!text) return null;

  const isActive = isPassageActiveForText(text, passageId, progress);
  const wordTargets = seekTargets.get("spoken-target");
  const isSeekable = Boolean(wordTargets?.size && (progress?.onSeekPassage || progress?.onSeek));
  if ((!isActive && !isSeekable) || !progress) {
    return <span className={className}>{text}</span>;
  }

  const activeIndex = isActive && resolved ? resolved.activeWordIndex : -1;
  const onWordClick = isSeekable
    ? (wordIndex: number) => {
        const target = wordTargets?.get(wordIndex);
        if (!target) return;
        if (progress.onSeekPassage) {
          progress.onSeekPassage(target.passageId, target.seconds);
        } else if (target.passageId === progress.passageId) {
          progress.onSeek?.(target.seconds);
        }
      }
    : undefined;

  return (
    <span className={`speech-passage-speaking ${className}`}>
      {renderHighlightedTokens(text, activeIndex, undefined, onWordClick, (wordIndex) => wordTargets?.has(wordIndex) ?? false)}
    </span>
  );
}
