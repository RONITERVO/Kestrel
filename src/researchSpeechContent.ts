import type { ResearchReport } from "./types";

export type ResearchSpeechScope = "summary" | "article" | "all";

export interface ResearchSpeechPassage {
  id: string;
  label: string;
  anchorId: string;
  text: string;
}

// Chatterbox starts playback sooner with short passages; the player prepares the next one while
// this one is playing. Sentence boundaries keep the joins natural rather than mechanically timed.
const MAX_PASSAGE_CHARS = 320;

function normalized(text: string): string {
  return text.replace(/\s+/g, " ").trim();
}

function splitForSpeech(text: string): string[] {
  const value = normalized(text);
  if (!value) return [];
  if (value.length <= MAX_PASSAGE_CHARS) return [value];

  const sentences = value.match(/[^.!?]+(?:[.!?]+["'\u2019\u201d)]*|$)/g)?.map(normalized).filter(Boolean) ?? [value];
  const chunks: string[] = [];
  let pending = "";
  const flush = () => {
    if (pending) chunks.push(pending);
    pending = "";
  };

  for (const sentence of sentences) {
    if (sentence.length > MAX_PASSAGE_CHARS) {
      flush();
      for (const word of sentence.split(" ")) {
        if (pending && pending.length + word.length + 1 > MAX_PASSAGE_CHARS) flush();
        pending = pending ? `${pending} ${word}` : word;
      }
      flush();
    } else if (!pending) {
      pending = sentence;
    } else if (pending.length + sentence.length + 1 <= MAX_PASSAGE_CHARS) {
      pending = `${pending} ${sentence}`;
    } else {
      flush();
      pending = sentence;
    }
  }
  flush();
  return chunks;
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
