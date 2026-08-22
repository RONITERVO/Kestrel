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

function isTableDivider(line: string): boolean {
  const trimmed = line.trim();
  return (
    trimmed.length > 0 &&
    /^[\s+|:=-]+$/.test(trimmed) &&
    (trimmed.includes("|") || trimmed.includes("+") || trimmed.includes("-") || trimmed.includes("="))
  );
}

function parseTableRowCells(line: string): string[] {
  const trimmed = line.trim();
  const withoutOuter = trimmed.replace(/^\|/, "").replace(/\|$/, "");
  return withoutOuter.split("|").map((cell) => cell.trim()).filter((cell) => cell.length > 0);
}

/**
 * Converts Markdown tables and ASCII box diagrams into natural spoken prose sentences.
 */
function convertTablesAndCharts(text: string): string {
  const lines = text.split(/\r?\n/);
  const output: string[] = [];
  let tableHeaders: string[] | null = null;
  let inTable = false;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line) {
      tableHeaders = null;
      inTable = false;
      continue;
    }

    // Skip divider lines like |---|---| or +---+---+ or |===|
    if (isTableDivider(line)) {
      inTable = true;
      continue;
    }

    // Check if line is a table row or box drawing with columns
    if (line.includes("|") && (line.startsWith("|") || line.endsWith("|") || line.split("|").length > 2)) {
      const cells = parseTableRowCells(line);
      if (!cells.length) continue;

      // Check if next line is a markdown separator (|---|) indicating this line is the header
      const nextLine = lines[i + 1]?.trim();
      if (nextLine && isTableDivider(nextLine) && !inTable) {
        tableHeaders = cells;
        inTable = true;
        continue;
      }

      if (tableHeaders && tableHeaders.length === cells.length) {
        // Read first cell as primary subject and remaining cells as details
        if (cells.length > 1) {
          output.push(`${cells[0]}: ${cells.slice(1).join(", ")}.`);
        } else {
          output.push(`${cells[0]}.`);
        }
      } else {
        output.push(`${cells.join(", ")}.`);
      }
      continue;
    }

    tableHeaders = null;
    inTable = false;
    output.push(line);
  }

  return output.join("\n");
}

/**
 * Normalizes rich markdown, formatted tables, ASCII charts, code blocks,
 * and special characters into natural, human-spoken newspaper/audiobook prose.
 * Eliminates neural TTS stuttering and symbol babbling while preserving
 * multilingual phonetics and accurate Whisper alignment.
 */
export function cleanProseForSpeech(raw: string, stripCodeBlocks = true): string {
  if (!raw) return "";

  let text = raw;

  // 1. Replace multi-line code blocks with a natural spoken cue
  if (stripCodeBlocks) {
    text = text.replace(/```[\s\S]*?```/g, " Code block on screen. ");
  }

  // 2. Decode HTML entities and strip HTML tags
  text = text
    .replace(/&amp;/gi, " and ")
    .replace(/&lt;/gi, " less than ")
    .replace(/&gt;/gi, " greater than ")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;/gi, "'")
    .replace(/&nbsp;/gi, " ")
    .replace(/<[^>]+>/g, " ");

  // 3. Convert Markdown tables and ASCII box diagrams into readable prose
  text = convertTablesAndCharts(text);

  // 4. Natural pronunciation of numbers, ranges, and approximations
  // Date ranges: 10,000-8,000 -> 10,000 to 8,000
  text = text.replace(/(\d+(?:,\d+)?)\s*[-–—]\s*(\d+(?:,\d+)?)/g, "$1 to $2");
  // Negative dates / circa prefix: -10,000 -> c. 10,000
  text = text.replace(/(?<=\s|^)-(\d+(?:,\d+)?)/g, "c. $1");
  // Approximation tildes: ~1,000 -> approximately 1,000
  text = text.replace(/~+(\d+(?:,\d+)?)/g, "approximately $1");
  // Mathematical plus: foragers + first farmers -> foragers plus first farmers
  text = text.replace(/(\w+)\s*\+\s*(\w+)/g, "$1 plus $2");

  // 5. Markdown syntax stripping
  // Links: [Text](url) -> Text
  text = text.replace(/\[([^\]]+)\]\([^)]+\)/g, "$1");
  // Images: ![Alt](url) -> ""
  text = text.replace(/!\[[^\]]*\]\([^)]+\)/g, " ");
  // Strip raw URLs to clean domain
  text = text.replace(/https?:\/\/(?:www\.)?([^\s/?#]+)(?:[^\s)]*)?/gi, "$1");
  // Headers: # Heading -> Heading.
  text = text.replace(/^[ \t]*#{1,6}[ \t]+([^\n]+)/gm, "$1. ");
  // Blockquotes: > quote -> quote
  text = text.replace(/^[ \t]*>[ \t]*/gm, " ");
  // List bullets: *, -, + at start of line
  text = text.replace(/^[ \t]*[*+-][ \t]+/gm, " ");
  // Numbered lists: 1. Item -> 1. Item
  text = text.replace(/^[ \t]*(\d+)[.)][ \t]+/gm, "$1. ");
  // Bold, italic, strikethrough markdown
  text = text
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/\*([^*]+)\*/g, "$1")
    .replace(/~~([^~]+)~~/g, "$1");
  // Inline code backticks: `foo` -> foo
  text = text.replace(/`([^`]+)`/g, "$1");
  // Horizontal rules: ---, ***, ===
  text = text.replace(/^[ \t]*[-*_=\s]{3,}[ \t]*$/gm, " ");

  // 6. Symbol translation & noise cleanup
  // Convert currency symbols before numbers to natural spoken words (including M, K, B multipliers)
  text = text
    .replace(/\$(\d+(?:[.,]\d+)?)\s*([kKmMbBtT])\b/g, "$1$2 dollars")
    .replace(/\$(\d+(?:[.,]\d+)?)/g, "$1 dollars")
    .replace(/€(\d+(?:[.,]\d+)?)\s*([kKmMbBtT])\b/g, "$1$2 euros")
    .replace(/€(\d+(?:[.,]\d+)?)/g, "$1 euros")
    .replace(/£(\d+(?:[.,]\d+)?)\s*([kKmMbBtT])\b/g, "$1$2 pounds")
    .replace(/£(\d+(?:[.,]\d+)?)/g, "$1 pounds")
    .replace(/¥(\d+(?:[.,]\d+)?)\s*([kKmMbBtT])\b/g, "$1$2 yen")
    .replace(/¥(\d+(?:[.,]\d+)?)/g, "$1 yen")
    .replace(/(\d+(?:[.,]\d+)?)\s*%/g, "$1 percent")
    .replace(/\+(\d+(?:[.,]\d+)?)/g, "plus $1")
    .replace(/-\s*(\d+(?:[.,]\d+)?)/g, "minus $1");

  // Common relational and direction symbols
  text = text
    .replace(/-->|->|=>/g, " to ")
    .replace(/<--|<-|<=/g, " from ")
    .replace(/\s+&\s+/g, " and ")
    .replace(/@/g, " at ")
    .replace(/\s*=\s*/g, " equals ")
    .replace(/([a-zA-Z0-9])\/([a-zA-Z0-9])/g, "$1 or $2");

  // Convert underscores (snake_case and decorative) to clean spaces
  text = text.replace(/_+/g, " ");

  // 6. Strip non-spoken symbol noise (including #, ¤, ^, ~, |, \, §, °, ±, ², ³, µ, ¶, ©, ®, ™, etc.)
  // Strip Unicode Box Drawing, Block Elements, Geometric Shapes, and Dingbats
  text = text.replace(/[\u2500-\u257F\u2580-\u259F\u25A0-\u25FF\u2B00-\u2BFF\u2600-\u26FF\u2700-\u27BF]/g, " ");
  // Strip emoji ranges
  text = text.replace(/[\u{1F300}-\u{1F9FF}\u{1FA00}-\u{1FAFF}\u{1F600}-\u{1F64F}\u{1F680}-\u{1F6FF}]/gu, " ");

  // Strip unpronounceable characters and symbol clusters: #, ¤, *, +, ^, ~, \, |, <, >, etc.
  // Preserves Unicode letters (\p{L}), numbers (\p{N}), natural punctuation (.,!?:;'"()-) and natural quotes/hyphens in words
  text = text.replace(/[#¤*+^~\\|<>§°±²³µ¶©®™•·‣⁃✓✔✕✖✗★☆▲▼◄►◆◇●○■□]/gu, " ");

  // Strip parentheses/brackets that contain only whitespace or symbol noise
  text = text.replace(/\([^\p{L}\p{N}]*\)/gu, " ");
  text = text.replace(/\[[^\p{L}\p{N}]*\]/gu, " ");
  text = text.replace(/\{[^\p{L}\p{N}]*\}/gu, " ");

  // 7. Punctuation & Stutter Normalization
  // Replace repeated periods/dashes/quotes
  text = text
    .replace(/\.{4,}/g, "...")
    .replace(/-{2,}/g, " - ")
    .replace(/={2,}/g, " ")
    .replace(/\?{2,}/g, "?")
    .replace(/!{2,}/g, "!")
    .replace(/["'”’]{2,}/g, '"');

  // Strip isolated quotes/hyphens that are not part of words
  text = text.replace(/(^|\s)[-–—'"“”‘’]+(?=\s|$)/g, " ");

  // Fix isolated punctuation or dangling punctuation at start of words/sentences
  text = text
    .replace(/\s+([.,!?:;])/g, "$1")
    .replace(/^[.,:;!?-]+/, "")
    .replace(/\(\s*\)/g, " ")
    .replace(/\[\s*\]/g, " ")
    .replace(/\{\s*\}/g, " ");

  // 8. Collapse whitespace and trim
  return text.replace(/\s+/g, " ").trim();
}

export function splitForSpeech(
  text: string,
  maxChars = MAX_PASSAGE_CHARS,
  stripCodeBlocks = true,
): string[] {
  const value = cleanProseForSpeech(text, stripCodeBlocks);
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
  const stripCode = options.stripCodeBlocks ?? true;
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
