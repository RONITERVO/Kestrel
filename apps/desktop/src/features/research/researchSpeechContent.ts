import type { ResearchReport, ResearchSpeechScope } from "../../contracts/index";
import { splitForSpeech, type SpeechPassage } from "../../shared/speech/text";
export type { ResearchSpeechScope } from "../../contracts/index";
export * from "../../shared/speech/text";
export type ResearchSpeechPassage = SpeechPassage;

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
