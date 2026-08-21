import type { ResearchReport } from "./types";

export type ResearchSpeechScope = "summary" | "article" | "all";

export interface SpeechPassage {
  id: string;
  label: string;
  anchorId?: string;
  text: string;
}

export type ResearchSpeechPassage = SpeechPassage;

export interface SpeechSplitOptions {
  maxPassageChars?: number;
  stripCodeBlocks?: boolean;
  basePassageId?: string;
  label?: string;
  anchorId?: string;
}

// Chatterbox starts playback sooner with short passages; the player prepares the next one while
// this one is playing. Sentence boundaries keep the joins natural rather than mechanically timed.
export const MAX_PASSAGE_CHARS = 320;

export function normalizedSpeechText(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

export function splitForSpeech(
  text: string,
  maxChars = MAX_PASSAGE_CHARS,
  stripCodeBlocks = false,
): string[] {
  let value = text;
  if (stripCodeBlocks) {
    value = value.replace(/```[\s\S]*?```/g, " Code block available on screen. ");
  }
  value = normalizedSpeechText(value);
  if (!value) return [];
  if (value.length <= maxChars) return [value];

  const sentences = value.match(/[^.!?]+(?:[.!?]+["'\u2019\u201d)]*|$)/g)?.map(normalizedSpeechText).filter(Boolean) ?? [value];
  const chunks: string[] = [];
  let pending = "";
  const flush = () => {
    if (pending) chunks.push(pending);
    pending = "";
  };

  for (const sentence of sentences) {
    if (sentence.length > maxChars) {
      flush();
      for (const word of sentence.split(" ")) {
        if (pending && pending.length + word.length + 1 > maxChars) flush();
        pending = pending ? `${pending} ${word}` : word;
      }
      flush();
    } else if (!pending) {
      pending = sentence;
    } else if (pending.length + sentence.length + 1 <= maxChars) {
      pending = `${pending} ${sentence}`;
    } else {
      flush();
      pending = sentence;
    }
  }
  flush();
  return chunks;
}

export function buildSpeechPassages(
  text: string,
  options: SpeechSplitOptions = {},
): SpeechPassage[] {
  const maxChars = options.maxPassageChars ?? MAX_PASSAGE_CHARS;
  const stripCode = options.stripCodeBlocks ?? false;
  const baseId = options.basePassageId ?? "passage";
  const label = options.label ?? "Passage";
  const anchorId = options.anchorId;
  const chunks = splitForSpeech(text, maxChars, stripCode);

  return chunks.map((chunk, index) => ({
    id: chunks.length === 1 ? baseId : `${baseId}-${index + 1}`,
    label: chunks.length === 1 ? label : `${label} (part ${index + 1} of ${chunks.length})`,
    anchorId,
    text: chunk,
  }));
}

export function buildResearchSpeechPassages(report: ResearchReport, scope: ResearchSpeechScope): ResearchSpeechPassage[] {
  const passages: ResearchSpeechPassage[] = [];
  const add = (id: string, label: string, anchorId: string, text: string) => {
    splitForSpeech(text).forEach((chunk, index, chunks) => passages.push({
      id: chunks.length === 1 ? id : `${id}-${index + 1}`,
      label,
      anchorId,
      text: chunk,
    }));
  };

  add("overview", "Overview", "report-overview", `${report.title}. ${report.dek}`);
  add("short-answer", "Short answer", "short-answer", `Short answer. ${report.answer}`);
  if (scope !== "summary" && report.edition > 1 && report.improvement.trim()) {
    add("edition", "What changed", "edition-improvement", `What changed in this edition. ${report.improvement}`);
  }
  report.findings.forEach((finding, index) => add(
    `finding-${index + 1}`,
    "Key findings",
    "findings",
    `Key finding ${index + 1}. ${finding.title}. ${finding.explanation}`,
  ));

  if (scope === "summary") return passages;

  report.sections.forEach((section, sectionIndex) => {
    add(`section-${sectionIndex + 1}-summary`, section.heading, section.id, `${section.heading}. ${section.summary}`);
    section.body.forEach((paragraph, paragraphIndex) => add(
      `section-${sectionIndex + 1}-paragraph-${paragraphIndex + 1}`,
      section.heading,
      section.id,
      paragraph,
    ));
  });
  report.timeline.forEach((item, index) => add(
    `timeline-${index + 1}`,
    "Timeline",
    "timeline",
    `${item.date}. ${item.label}. ${item.description}`,
  ));
  report.terms.forEach((term, index) => add(
    `term-${index + 1}`,
    "Terms worth knowing",
    "terms",
    `${term.term}. ${term.meaning}`,
  ));
  report.openQuestions.forEach((question, index) => add(
    `question-${index + 1}`,
    "Open questions",
    "terms",
    `Open question ${index + 1}. ${question}`,
  ));
  if (scope === "all") {
    report.sources.forEach((source, index) => add(
      `source-${index + 1}`,
      "Sources inspected",
      "sources",
      `Source ${source.id}. ${source.title}. ${source.section ? `${source.section}. ` : ""}${source.excerpt}`,
    ));
  }
  return passages;
}
