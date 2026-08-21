import { useState, useMemo, type ReactNode } from "react";
import { Check, Copy, Code2, BarChart2 } from "lucide-react";
import {
  SpokenText,
  isPassageActiveForText,
  getActiveWordIndex,
  renderHighlightedTokens,
  type SpeechProgressState,
  type WordOffsetTracker,
} from "./spokenHighlight";

export interface MarkdownContentProps {
  value: string;
  streaming?: boolean;
  className?: string;
  speechProgress?: SpeechProgressState | null;
}

type TableAlign = "left" | "center" | "right" | undefined;

interface TableBlock {
  type: "table";
  headers: string[];
  alignments: TableAlign[];
  rows: string[][];
}

interface CodeBlock {
  type: "code";
  language: string;
  code: string;
  isChart?: boolean;
}

interface ChartBlock {
  type: "chart";
  text: string;
}

interface HeadingBlock {
  type: "heading";
  level: 1 | 2 | 3 | 4 | 5 | 6;
  text: string;
}

interface ListBlock {
  type: "list";
  ordered: boolean;
  start?: number;
  items: string[];
}

interface BlockquoteBlock {
  type: "blockquote";
  text: string;
}

interface DividerBlock {
  type: "divider";
}

interface ParagraphBlock {
  type: "paragraph";
  text: string;
}

type MarkdownBlock =
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

    const codeMatch = trimmed.match(/^[ \t]*(```|~~~)(.*)$/);
    if (codeMatch) {
      const fence = codeMatch[1];
      const language = codeMatch[2]?.toLowerCase() || "text";
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
        isChart: ["chart", "ascii", "mermaid"].includes(language),
      });
      continue;
    }

    // 3. Horizontal Rules
    if (/^[ \t]*[-*_]{3,}[ \t]*$/.test(trimmed)) {
      blocks.push({ type: "divider" });
      i++;
      continue;
    }

    if (isAsciiChartLine(line)) {
      const chartLines: string[] = [];
      while (i < lines.length && isAsciiChartLine(lines[i])) {
        chartLines.push(lines[i]);
        i++;
      }
      blocks.push({ type: "chart", text: chartLines.join("\n") });
      continue;
    }

    const headingMatch = line.match(/^[ \t]*(#{1,6})[ \t]+([^\n]+)$/);
    if (headingMatch) {
      const level = headingMatch[1].length as 1 | 2 | 3 | 4 | 5 | 6;
      blocks.push({ type: "heading", level, text: headingMatch[2].trim() });
      i++;
      continue;
    }

    if (trimmed.startsWith(">")) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].trim().startsWith(">")) {
        quoteLines.push(lines[i].replace(/^[ \t]*>[ \t]?/, ""));
        i++;
      }
      blocks.push({ type: "blockquote", text: quoteLines.join("\n") });
      continue;
    }

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
        } else if (cur.trim().startsWith("  ") || cur.trim().startsWith("\t")) {
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
}

export function getBlockSpeechHighlight(
  blockText: string,
  speechProgress?: SpeechProgressState | null,
): BlockHighlightContext | null {
  if (!speechProgress || !speechProgress.active) return null;
  if (!isPassageActiveForText(blockText, undefined, speechProgress)) return null;

  const activeWordIndex = getActiveWordIndex(
    speechProgress.text || blockText,
    speechProgress.seconds,
    speechProgress.duration,
    speechProgress.timings,
  );

  return {
    activeWordIndex,
    tracker: { current: 0 },
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

    if (part.startsWith("`") && part.endsWith("`") && part.length >= 2) {
      const inner = part.slice(1, -1);
      return (
        <code key={index} className="markdown-inline-code">
          {highlight
            ? renderHighlightedTokens(inner, highlight.activeWordIndex, highlight.tracker)
            : inner}
        </code>
      );
    }

    if (part.startsWith("***") && part.endsWith("***") && part.length >= 6) {
      const inner = part.slice(3, -3);
      return (
        <strong key={index}>
          <em>
            {highlight
              ? renderHighlightedTokens(inner, highlight.activeWordIndex, highlight.tracker)
              : inner}
          </em>
        </strong>
      );
    }

    if (
      (part.startsWith("**") && part.endsWith("**") && part.length >= 4) ||
      (part.startsWith("__") && part.endsWith("__") && part.length >= 4)
    ) {
      const inner = part.slice(2, -2);
      return (
        <strong key={index}>
          {highlight
            ? renderHighlightedTokens(inner, highlight.activeWordIndex, highlight.tracker)
            : inner}
        </strong>
      );
    }

    if (
      (part.startsWith("*") && part.endsWith("*") && part.length >= 2) ||
      (part.startsWith("_") && part.endsWith("_") && part.length >= 2)
    ) {
      const inner = part.slice(1, -1);
      return (
        <em key={index}>
          {highlight
            ? renderHighlightedTokens(inner, highlight.activeWordIndex, highlight.tracker)
            : inner}
        </em>
      );
    }

    if (part.startsWith("~~") && part.endsWith("~~") && part.length >= 4) {
      const inner = part.slice(2, -2);
      return (
        <del key={index}>
          {highlight
            ? renderHighlightedTokens(inner, highlight.activeWordIndex, highlight.tracker)
            : inner}
        </del>
      );
    }

    return highlight ? (
      <span key={index}>
        {renderHighlightedTokens(part, highlight.activeWordIndex, highlight.tracker)}
      </span>
    ) : (
      <span key={index}>{part}</span>
    );
  });
}

function CodeBlockView({
  language,
  code,
  speechProgress,
}: {
  language: string;
  code: string;
  speechProgress?: SpeechProgressState | null;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    void navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const isCodeActive = isPassageActiveForText(code, undefined, speechProgress);

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
          {isCodeActive ? (
            <SpokenText text={code} progress={speechProgress} />
          ) : (
            code
          )}
        </code>
      </pre>
    </div>
  );
}

function ChartCardView({
  text,
  speechProgress,
}: {
  text: string;
  speechProgress?: SpeechProgressState | null;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    void navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const isChartActive = isPassageActiveForText(text, undefined, speechProgress);

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
          {isChartActive ? (
            <SpokenText text={text} progress={speechProgress} />
          ) : (
            text
          )}
        </code>
      </pre>
    </div>
  );
}

function TableView({
  headers,
  alignments,
  rows,
  speechProgress,
}: TableBlock & { speechProgress?: SpeechProgressState | null }) {
  const headerText = headers.join(" ");
  const headerHighlight = getBlockSpeechHighlight(headerText, speechProgress);

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
            const rowText = row.join(" ");
            const rowHighlight = getBlockSpeechHighlight(rowText, speechProgress);
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
            return <TableView key={index} {...block} speechProgress={speechProgress} />;
          case "code":
            return <CodeBlockView key={index} language={block.language} code={block.code} speechProgress={speechProgress} />;
          case "chart":
            return <ChartCardView key={index} text={block.text} speechProgress={speechProgress} />;
          case "heading": {
            const highlight = getBlockSpeechHighlight(block.text, speechProgress);
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
                  const highlight = getBlockSpeechHighlight(item, speechProgress);
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
                  const highlight = getBlockSpeechHighlight(line, speechProgress);
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
            const highlight = getBlockSpeechHighlight(block.text, speechProgress);
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
