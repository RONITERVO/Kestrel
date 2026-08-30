import { useState, useMemo, type ReactNode } from "react";
import { Check, Copy, Code2, BarChart2 } from "lucide-react";
import {
  useResolvedSpeechHighlight,
  renderHighlightedTokens,
  useSpeechSeekTargets,
  type CandidateBlock,
  type SpeechSeekTargetMap,
  type SpeechProgressState,
  type WordOffsetTracker,
} from "./spokenHighlight";
import { cleanProseForSpeech } from "../../features/research/researchSpeechContent";

export interface MarkdownContentProps {
  value: string;
  streaming?: boolean;
  className?: string;
  speechProgress?: SpeechProgressState | null;
}

type TableAlign = "left" | "center" | "right" | undefined;

export interface TableBlock {
  type: "table";
  headers: string[];
  alignments: TableAlign[];
  rows: string[][];
}

export interface CodeBlock {
  type: "code";
  language: string;
  code: string;
}

export interface ChartBlock {
  type: "chart";
  text: string;
}

export interface HeadingBlock {
  type: "heading";
  level: 1 | 2 | 3 | 4 | 5 | 6;
  text: string;
}

export interface ListBlock {
  type: "list";
  ordered: boolean;
  start?: number;
  items: string[];
}

export interface BlockquoteBlock {
  type: "blockquote";
  text: string;
}

export interface DividerBlock {
  type: "divider";
}

export interface ParagraphBlock {
  type: "paragraph";
  text: string;
}

export type MarkdownBlock =
  | TableBlock
  | CodeBlock
  | ChartBlock
  | HeadingBlock
  | ListBlock
  | BlockquoteBlock
  | DividerBlock
  | ParagraphBlock;

function isTableDividerLine(line: string): boolean {
  const trimmed = line.trim();
  return (
    trimmed.length > 0 &&
    /^[\s+|:=-]+$/.test(trimmed) &&
    (trimmed.includes("|") || trimmed.includes("+") || trimmed.includes("-") || trimmed.includes("="))
  );
}

function parseTableRow(line: string): string[] {
  const trimmed = line.trim();
  const withoutOuter = trimmed.replace(/^\|/, "").replace(/\|$/, "");
  return withoutOuter.split("|").map((c) => c.trim());
}

function parseAlignment(cell: string): TableAlign {
  const trimmed = cell.trim();
  if (trimmed.startsWith(":") && trimmed.endsWith(":")) return "center";
  if (trimmed.endsWith(":")) return "right";
  if (trimmed.startsWith(":")) return "left";
  return undefined;
}

function isAsciiChartLine(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return false;
  if (/^[ \t]*[-*_]{3,}[ \t]*$/.test(trimmed)) return false;
  if (/^[+\-=|_]{3,}$/.test(trimmed) && (trimmed.includes("+") || trimmed.includes("|"))) return true;
  if (/^[+\-=|_\s]+$/.test(trimmed) && trimmed.length >= 4 && (trimmed.includes("+") || trimmed.includes("|"))) return true;
  if (
    (trimmed.startsWith("+") && trimmed.endsWith("+")) ||
    (trimmed.startsWith("|") && trimmed.endsWith("|")) ||
    (trimmed.startsWith("┌") && trimmed.endsWith("┐")) ||
    (trimmed.startsWith("└") && trimmed.endsWith("┘")) ||
    (trimmed.startsWith("├") && trimmed.endsWith("┤"))
  ) {
    return true;
  }
  if (
    trimmed.includes("-->") ||
    trimmed.includes("<--") ||
    trimmed.includes("==>")
  ) {
    return true;
  }
  return false;
}

export function parseMarkdownBlocks(markdown: string): MarkdownBlock[] {
  if (!markdown) return [];
  const lines = markdown.split(/\r?\n/);
  const blocks: MarkdownBlock[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();

    if (!trimmed) {
      i++;
      continue;
    }

    // 1. GFM Tables
    if (
      trimmed.includes("|") &&
      lines[i + 1] &&
      isTableDividerLine(lines[i + 1])
    ) {
      const headers = parseTableRow(line);
      const dividerCells = parseTableRow(lines[i + 1]);
      const alignments = dividerCells.map(parseAlignment);
      const rows: string[][] = [];
      i += 2;

      while (i < lines.length) {
        const cur = lines[i].trim();
        if (!cur || !cur.includes("|")) break;
        if (isTableDividerLine(cur)) {
          i++;
          continue;
        }
        rows.push(parseTableRow(cur));
        i++;
      }

      blocks.push({
        type: "table",
        headers,
        alignments,
        rows,
      });
      continue;
    }

    // 2. Fenced Code Blocks (```lang or ~~~lang)
    const codeMatch = trimmed.match(/^[ \t]*(```|~~~)(.*)$/);
    if (codeMatch) {
      const fence = codeMatch[1];
      const language = codeMatch[2]?.trim().split(/\s+/, 1)[0]?.toLowerCase() || "text";
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(fence)) {
        codeLines.push(lines[i]);
        i++;
      }
      i++;
      blocks.push({
        type: "code",
        language,
        code: codeLines.join("\n"),
      });
      continue;
    }

    // 3. Horizontal Rules
    if (/^[ \t]*[-*_]{3,}[ \t]*$/.test(trimmed)) {
      blocks.push({ type: "divider" });
      i++;
      continue;
    }

    // 4. Standalone ASCII / Text Chart Card
    if (isAsciiChartLine(line)) {
      const chartLines: string[] = [];
      while (i < lines.length && isAsciiChartLine(lines[i])) {
        chartLines.push(lines[i]);
        i++;
      }
      blocks.push({ type: "chart", text: chartLines.join("\n") });
      continue;
    }

    // 5. Headings (# H1 to ###### H6)
    const headingMatch = line.match(/^[ \t]*(#{1,6})[ \t]+([^\n]+)$/);
    if (headingMatch) {
      const level = headingMatch[1].length as 1 | 2 | 3 | 4 | 5 | 6;
      blocks.push({ type: "heading", level, text: headingMatch[2].trim() });
      i++;
      continue;
    }

    // 6. Blockquotes (> Quote)
    if (trimmed.startsWith(">")) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].trim().startsWith(">")) {
        quoteLines.push(lines[i].replace(/^[ \t]*>[ \t]?/, ""));
        i++;
      }
      blocks.push({ type: "blockquote", text: quoteLines.join("\n") });
      continue;
    }

    // 7. Unordered & Ordered Lists
    const unorderedMatch = line.match(/^[ \t]*([*+-])[ \t]+([^\n]+)$/);
    const orderedMatch = line.match(/^[ \t]*(\d+)[.)][ \t]+([^\n]+)$/);

    if (unorderedMatch || orderedMatch) {
      const ordered = Boolean(orderedMatch);
      const start = orderedMatch ? parseInt(orderedMatch[1], 10) : undefined;
      const items: string[] = [];

      while (i < lines.length) {
        const cur = lines[i];
        const uMatch = cur.match(/^[ \t]*[*+-][ \t]+([^\n]+)$/);
        const oMatch = cur.match(/^[ \t]*\d+[.)][ \t]+([^\n]+)$/);

        if (ordered && oMatch) {
          items.push(oMatch[1]);
          i++;
        } else if (!ordered && uMatch) {
          items.push(uMatch[1]);
          i++;
        } else if (cur.startsWith("  ") || cur.startsWith("\t")) {
          if (items.length > 0) {
            items[items.length - 1] += `\n${cur.trim()}`;
          }
          i++;
        } else {
          break;
        }
      }

      blocks.push({ type: "list", ordered, start, items });
      continue;
    }

    // 8. Regular Paragraphs
    const paragraphLines: string[] = [];
    while (i < lines.length) {
      const curLine = lines[i];
      const curTrimmed = curLine.trim();
      if (!curTrimmed) break;
      if (
        curTrimmed.match(/^[ \t]*(```|~~~)/) ||
        curTrimmed.match(/^[ \t]*#{1,6}[ \t]+/) ||
        curTrimmed.startsWith(">") ||
        (curTrimmed.includes("|") && lines[i + 1] && isTableDividerLine(lines[i + 1])) ||
        curTrimmed.match(/^[ \t]*[*+-][ \t]+/) ||
        curTrimmed.match(/^[ \t]*\d+[.)][ \t]+/) ||
        isAsciiChartLine(curLine)
      ) {
        break;
      }
      paragraphLines.push(curLine);
      i++;
    }
    if (paragraphLines.length > 0) {
      blocks.push({ type: "paragraph", text: paragraphLines.join("\n") });
    } else {
      i++;
    }
  }

  return blocks;
}

export interface BlockHighlightContext {
  activeWordIndex: number;
  tracker: WordOffsetTracker;
  onWordClick?: (wordIndex: number) => void;
  canSeekWord?: (wordIndex: number) => boolean;
}

export function collectCandidateBlocks(blocks: MarkdownBlock[]): CandidateBlock[] {
  const candidates: CandidateBlock[] = [];
  blocks.forEach((block, bIdx) => {
    switch (block.type) {
      case "table": {
        candidates.push({ id: `table-${bIdx}-hdr`, text: block.headers.join(" ") });
        block.rows.forEach((row, rIdx) => {
          candidates.push({ id: `table-${bIdx}-row-${rIdx}`, text: row.join(" ") });
        });
        break;
      }
      case "heading":
        candidates.push({ id: `heading-${bIdx}`, text: block.text });
        break;
      case "list":
        block.items.forEach((item, iIdx) => {
          candidates.push({ id: `list-${bIdx}-${iIdx}`, text: item });
        });
        break;
      case "blockquote":
        block.text.split("\n").forEach((line, lIdx) => {
          candidates.push({ id: `quote-${bIdx}-${lIdx}`, text: line });
        });
        break;
      case "chart":
        candidates.push({ id: `chart-${bIdx}`, text: block.text });
        break;
      case "code":
        candidates.push({ id: `code-${bIdx}`, text: block.code });
        break;
      case "paragraph":
        candidates.push({ id: `para-${bIdx}`, text: block.text });
        break;
    }
  });
  return candidates;
}

export function getBlockSpeechHighlight(
  elementId: string,
  activeHighlight: { activeId: string; activeWordIndex: number } | null,
  progress?: SpeechProgressState | null,
  seekTargets?: SpeechSeekTargetMap,
): BlockHighlightContext | null {
  const isActiveBlock = activeHighlight?.activeId === elementId;
  const wordTargets = seekTargets?.get(elementId);
  if (!isActiveBlock && !wordTargets?.size) return null;
  return {
    activeWordIndex: progress?.active && isActiveBlock ? activeHighlight.activeWordIndex : -1,
    tracker: { current: 0 },
    onWordClick: wordTargets?.size && progress && (progress.onSeekPassage || progress.onSeek)
      ? (wordIndex: number) => {
          const target = wordTargets.get(wordIndex);
          if (!target) return;
          if (progress.onSeekPassage) {
            progress.onSeekPassage(target.passageId, target.seconds);
          } else if (target.passageId === progress.passageId) {
            progress.onSeek?.(target.seconds);
          }
        }
      : undefined,
    canSeekWord: wordTargets?.size ? (wordIndex: number) => wordTargets.has(wordIndex) : undefined,
  };
}

export function renderInlineMarkdown(
  text: string,
  highlight?: BlockHighlightContext | null,
): ReactNode[] {
  if (!text) return [];

  const tokenRegex =
    /(\[[^\]]+\]\([^)]+\)|`[^`]+`|\*\*\*[^*]+\*\*\*|\*\*[^*]+\*\*|__[^_]+__|\*[^*]+\*|_[^_]+_|~~[^~]+~~)/g;

  const parts = text.split(tokenRegex);

  return parts.map((part, index) => {
    if (!part) return null;

    // Link: [Label](url)
    const linkMatch = part.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
    if (linkMatch) {
      const url = linkMatch[2].trim();
      const safeUrl =
        /^https?:\/\//i.test(url) || url.startsWith("#") || url.startsWith("/")
          ? url
          : `https://${url}`;
      return (
        <a
          key={index}
          href={safeUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="markdown-link"
        >
          {highlight
            ? renderHighlightedTokens(linkMatch[1], highlight.activeWordIndex, highlight.tracker)
            : linkMatch[1]}
        </a>
      );
    }

    // Inline Code: `code`
    if (part.startsWith("`") && part.endsWith("`") && part.length >= 2) {
      const inner = part.slice(1, -1);
      return (
        <code key={index} className="markdown-inline-code">
          {highlight
            ? renderHighlightedTokens(inner, highlight.activeWordIndex, highlight.tracker, highlight.onWordClick, highlight.canSeekWord)
            : inner}
        </code>
      );
    }

    // Bold-italic: ***text***
    if (part.startsWith("***") && part.endsWith("***") && part.length >= 6) {
      const inner = part.slice(3, -3);
      return (
        <strong key={index}>
          <em>
            {highlight
              ? renderHighlightedTokens(inner, highlight.activeWordIndex, highlight.tracker, highlight.onWordClick, highlight.canSeekWord)
              : inner}
          </em>
        </strong>
      );
    }

    // Bold: **text** or __text__
    if (
      (part.startsWith("**") && part.endsWith("**") && part.length >= 4) ||
      (part.startsWith("__") && part.endsWith("__") && part.length >= 4)
    ) {
      const inner = part.slice(2, -2);
      return (
        <strong key={index}>
          {highlight
            ? renderHighlightedTokens(inner, highlight.activeWordIndex, highlight.tracker, highlight.onWordClick, highlight.canSeekWord)
            : inner}
        </strong>
      );
    }

    // Italic: *text* or _text_
    if (
      (part.startsWith("*") && part.endsWith("*") && part.length >= 2) ||
      (part.startsWith("_") && part.endsWith("_") && part.length >= 2)
    ) {
      const inner = part.slice(1, -1);
      return (
        <em key={index}>
          {highlight
            ? renderHighlightedTokens(inner, highlight.activeWordIndex, highlight.tracker, highlight.onWordClick, highlight.canSeekWord)
            : inner}
        </em>
      );
    }

    // Strikethrough: ~~text~~
    if (part.startsWith("~~") && part.endsWith("~~") && part.length >= 4) {
      const inner = part.slice(2, -2);
      return (
        <del key={index}>
          {highlight
            ? renderHighlightedTokens(inner, highlight.activeWordIndex, highlight.tracker, highlight.onWordClick, highlight.canSeekWord)
            : inner}
        </del>
      );
    }

    return highlight ? (
      <span key={index}>
        {renderHighlightedTokens(part, highlight.activeWordIndex, highlight.tracker, highlight.onWordClick, highlight.canSeekWord)}
      </span>
    ) : (
      <span key={index}>{part}</span>
    );
  });
}

function CodeBlockView({
  elementId,
  activeHighlight,
  language,
  code,
  speechProgress,
  seekTargets,
}: {
  elementId: string;
  activeHighlight: { activeId: string; activeWordIndex: number } | null;
  language: string;
  code: string;
  speechProgress?: SpeechProgressState | null;
  seekTargets: SpeechSeekTargetMap;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    void navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const highlight = getBlockSpeechHighlight(elementId, activeHighlight, speechProgress, seekTargets);

  return (
    <div className="markdown-code-card">
      <div className="markdown-code-header">
        <span className="markdown-code-lang">
          <Code2 size={13} />
          {language || "code"}
        </span>
        <button
          type="button"
          className="markdown-code-copy"
          onClick={handleCopy}
          title="Copy code"
          aria-label="Copy code to clipboard"
        >
          {copied ? (
            <>
              <Check size={12} className="copy-success" />
              <span>Copied</span>
            </>
          ) : (
            <>
              <Copy size={12} />
              <span>Copy</span>
            </>
          )}
        </button>
      </div>
      <pre className="markdown-code-pre">
        <code>
          {highlight ? (
            renderHighlightedTokens(code, highlight.activeWordIndex, highlight.tracker, highlight.onWordClick, highlight.canSeekWord)
          ) : (
            code
          )}
        </code>
      </pre>
    </div>
  );
}

function ChartCardView({
  elementId,
  activeHighlight,
  text,
  speechProgress,
  seekTargets,
}: {
  elementId: string;
  activeHighlight: { activeId: string; activeWordIndex: number } | null;
  text: string;
  speechProgress?: SpeechProgressState | null;
  seekTargets: SpeechSeekTargetMap;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    void navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const highlight = getBlockSpeechHighlight(elementId, activeHighlight, speechProgress, seekTargets);

  return (
    <div className="markdown-chart-card">
      <div className="markdown-chart-header">
        <span className="markdown-chart-title">
          <BarChart2 size={13} />
          <span>Diagram / Text Chart</span>
        </span>
        <button
          type="button"
          className="markdown-code-copy"
          onClick={handleCopy}
          title="Copy diagram"
          aria-label="Copy diagram to clipboard"
        >
          {copied ? (
            <>
              <Check size={12} className="copy-success" />
              <span>Copied</span>
            </>
          ) : (
            <>
              <Copy size={12} />
              <span>Copy</span>
            </>
          )}
        </button>
      </div>
      <pre className="markdown-chart-body">
        <code>
          {highlight ? (
            renderHighlightedTokens(text, highlight.activeWordIndex, highlight.tracker, highlight.onWordClick, highlight.canSeekWord)
          ) : (
            text
          )}
        </code>
      </pre>
    </div>
  );
}

function TableView({
  blockIndex,
  activeHighlight,
  headers,
  alignments,
  rows,
  speechProgress,
  seekTargets,
}: TableBlock & {
  blockIndex: number;
  activeHighlight: { activeId: string; activeWordIndex: number } | null;
  speechProgress?: SpeechProgressState | null;
  seekTargets: SpeechSeekTargetMap;
}) {
  const headerId = `table-${blockIndex}-hdr`;
  const headerHighlight = getBlockSpeechHighlight(headerId, activeHighlight, speechProgress, seekTargets);

  return (
    <div className="markdown-table-wrapper">
      <table className="markdown-table">
        <thead>
          <tr>
            {headers.map((header, colIndex) => (
              <th
                key={colIndex}
                style={{ textAlign: alignments[colIndex] }}
              >
                {renderInlineMarkdown(header, headerHighlight)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, rowIndex) => {
            const rowId = `table-${blockIndex}-row-${rowIndex}`;
            const rowHighlight = getBlockSpeechHighlight(rowId, activeHighlight, speechProgress, seekTargets);
            return (
              <tr key={rowIndex}>
                {row.map((cell, colIndex) => (
                  <td
                    key={colIndex}
                    style={{ textAlign: alignments[colIndex] }}
                  >
                    {renderInlineMarkdown(cell, rowHighlight)}
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

export function MarkdownContent({
  value,
  streaming = false,
  className = "",
  speechProgress = null,
}: MarkdownContentProps) {
  const blocks = useMemo(() => parseMarkdownBlocks(value), [value]);
  const candidates = useMemo(() => collectCandidateBlocks(blocks), [blocks]);
  const activeHighlight = useResolvedSpeechHighlight(candidates, speechProgress);
  const seekTargets = useSpeechSeekTargets(candidates, speechProgress);

  if (!value && streaming) {
    return (
      <div className={`markdown-content streaming ${className}`}>
        <span className="stream-cursor-pulse" aria-label="Generating response…" />
      </div>
    );
  }

  if (!blocks.length && !value) {
    return null;
  }

  return (
    <div className={`markdown-content ${streaming ? "streaming" : ""} ${className}`}>
      {blocks.map((block, index) => {
        switch (block.type) {
          case "table":
            return (
              <TableView
                key={index}
                blockIndex={index}
                activeHighlight={activeHighlight}
                {...block}
                speechProgress={speechProgress}
                seekTargets={seekTargets}
              />
            );
          case "code":
            return (
              <CodeBlockView
                key={index}
                elementId={`code-${index}`}
                activeHighlight={activeHighlight}
                language={block.language}
                code={block.code}
                speechProgress={speechProgress}
                seekTargets={seekTargets}
              />
            );
          case "chart":
            return (
              <ChartCardView
                key={index}
                elementId={`chart-${index}`}
                activeHighlight={activeHighlight}
                text={block.text}
                speechProgress={speechProgress}
                seekTargets={seekTargets}
              />
            );
          case "heading": {
            const highlight = getBlockSpeechHighlight(`heading-${index}`, activeHighlight, speechProgress, seekTargets);
            const children = renderInlineMarkdown(block.text, highlight);
            switch (block.level) {
              case 1: return <h1 key={index} className="markdown-h1">{children}</h1>;
              case 2: return <h2 key={index} className="markdown-h2">{children}</h2>;
              case 3: return <h3 key={index} className="markdown-h3">{children}</h3>;
              case 4: return <h4 key={index} className="markdown-h4">{children}</h4>;
              case 5: return <h5 key={index} className="markdown-h5">{children}</h5>;
              case 6: return <h6 key={index} className="markdown-h6">{children}</h6>;
            }
            return <h6 key={index} className="markdown-h6">{children}</h6>;
          }
          case "list": {
            const ListTag = block.ordered ? "ol" : "ul";
            return (
              <ListTag key={index} start={block.start} className="markdown-list">
                {block.items.map((item, itemIdx) => {
                  const highlight = getBlockSpeechHighlight(`list-${index}-${itemIdx}`, activeHighlight, speechProgress, seekTargets);
                  return (
                    <li key={itemIdx}>
                      {renderInlineMarkdown(item, highlight)}
                    </li>
                  );
                })}
              </ListTag>
            );
          }
          case "blockquote":
            return (
              <blockquote key={index} className="markdown-blockquote">
                {block.text.split("\n").map((line, lineIdx) => {
                  const highlight = getBlockSpeechHighlight(`quote-${index}-${lineIdx}`, activeHighlight, speechProgress, seekTargets);
                  return (
                    <p key={lineIdx}>
                      {renderInlineMarkdown(line, highlight)}
                    </p>
                  );
                })}
              </blockquote>
            );
          case "divider":
            return <hr key={index} className="markdown-hr" />;
          case "paragraph":
          default: {
            const highlight = getBlockSpeechHighlight(`para-${index}`, activeHighlight, speechProgress, seekTargets);
            return (
              <p key={index} className="markdown-paragraph">
                {renderInlineMarkdown(block.text, highlight)}
              </p>
            );
          }
        }
      })}
      {streaming && <span className="stream-cursor-pulse" aria-hidden="true" />}
    </div>
  );
}
