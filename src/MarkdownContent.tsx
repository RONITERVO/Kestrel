import { useState, useMemo, type ReactNode } from "react";
import { Check, Copy, Code2, BarChart2 } from "lucide-react";
import { SpokenText, type SpeechProgressState } from "./spokenHighlight";

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

interface ChartBlock {
  type: "chart";
  text: string;
}

interface ParagraphBlock {
  type: "paragraph";
  text: string;
}

type MarkdownBlock =
  | TableBlock
  | CodeBlock
  | HeadingBlock
  | ListBlock
  | BlockquoteBlock
  | DividerBlock
  | ChartBlock
  | ParagraphBlock;

function isTableDividerLine(line: string): boolean {
  const trimmed = line.trim();
  return (
    trimmed.length > 0 &&
    /^\|?[\s|:=-]+\|?$/.test(trimmed) &&
    trimmed.includes("|") &&
    trimmed.includes("-") &&
    !trimmed.includes("+")
  );
}

function parseTableAlignment(cell: string): TableAlign {
  const trimmed = cell.trim();
  const left = trimmed.startsWith(":");
  const right = trimmed.endsWith(":");
  if (left && right) return "center";
  if (right) return "right";
  if (left) return "left";
  return undefined;
}

function splitTableCells(line: string): string[] {
  const trimmed = line.trim();
  const withoutOuter = trimmed.replace(/^\|/, "").replace(/\|$/, "");
  return withoutOuter.split("|").map((cell) => cell.trim());
}

function isAsciiChartLine(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed) return false;
  // Box-drawing characters
  if (/[\u2500-\u257F\u2580-\u259F\u25A0-\u25FF\u2550-\u256C]/.test(trimmed)) return true;
  // ASCII box corners and edges (+---+ or +===+)
  if (/^\+[-=+]+\+$/.test(trimmed)) return true;
  if (/^\+[-=+]+\s+\+[-=+]+\+?$/.test(trimmed)) return true;
  if (trimmed.startsWith("+--") || trimmed.startsWith("+==")) return true;
  if (/\b(?:-->|==>|->|<-|<--|<==)\b/.test(trimmed) && (trimmed.includes("|") || trimmed.includes("+") || trimmed.includes("["))) return true;
  if (/\[[#=\-*█▒░]{3,}[^\]]*\]/.test(trimmed)) return true;
  return false;
}

function parseMarkdownBlocks(raw: string): MarkdownBlock[] {
  if (!raw) return [];
  const lines = raw.split(/\r?\n/);
  const blocks: MarkdownBlock[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();

    // 1. Empty lines
    if (!trimmed) {
      i++;
      continue;
    }

    // 2. Fenced Code Blocks (``` or ~~~)
    const codeMatch = line.match(/^[ \t]*(```|~~~)([\w-]*)[ \t]*$/);
    if (codeMatch) {
      const fence = codeMatch[1];
      const language = codeMatch[2]?.toLowerCase() || "text";
      const codeLines: string[] = [];
      i++;
      while (i < lines.length) {
        if (lines[i].trim().startsWith(fence)) {
          i++;
          break;
        }
        codeLines.push(lines[i]);
        i++;
      }
      const code = codeLines.join("\n");
      const isChart = ["chart", "ascii", "diagram", "mermaid", "table"].includes(language);
      blocks.push({ type: "code", language, code, isChart });
      continue;
    }

    // 3. Horizontal Rules
    if (/^[ \t]*[-*_=\s]{3,}[ \t]*$/.test(trimmed) && !trimmed.includes("|") && !trimmed.includes("+")) {
      blocks.push({ type: "divider" });
      i++;
      continue;
    }

    // 4. Standalone ASCII / Text Chart Card (if ascii chart or box diagram lines)
    if (isAsciiChartLine(line)) {
      const chartLines: string[] = [];
      while (
        i < lines.length &&
        (isAsciiChartLine(lines[i]) ||
          (lines[i].trim().length > 0 &&
            chartLines.length > 0 &&
            (lines[i].includes("|") || lines[i].includes("+") || lines[i].startsWith(" "))))
      ) {
        chartLines.push(lines[i]);
        i++;
      }
      if (chartLines.length >= 2) {
        blocks.push({ type: "chart", text: chartLines.join("\n") });
        continue;
      } else {
        // Fallback to normal parsing if only single non-diagram line
        i -= chartLines.length;
      }
    }

    // 5. Headings (# H1 to ###### H6)
    const headingMatch = line.match(/^[ \t]*(#{1,6})[ \t]+([^\n]+)$/);
    if (headingMatch) {
      const level = headingMatch[1].length as 1 | 2 | 3 | 4 | 5 | 6;
      blocks.push({ type: "heading", level, text: headingMatch[2].trim() });
      i++;
      continue;
    }

    // 6. Blockquotes (> quote)
    if (trimmed.startsWith(">")) {
      const quoteLines: string[] = [];
      while (i < lines.length && lines[i].trim().startsWith(">")) {
        quoteLines.push(lines[i].trim().replace(/^>[ \t]?/, ""));
        i++;
      }
      blocks.push({ type: "blockquote", text: quoteLines.join("\n") });
      continue;
    }

    // 7. Markdown Tables (GFM standard)
    if (trimmed.includes("|") && (trimmed.startsWith("|") || trimmed.endsWith("|") || trimmed.split("|").length > 2)) {
      const nextLine = lines[i + 1]?.trim();
      if (nextLine && isTableDividerLine(nextLine)) {
        const headers = splitTableCells(line);
        const alignments = splitTableCells(nextLine).map(parseTableAlignment);
        const rows: string[][] = [];
        i += 2; // skip header and divider

        while (i < lines.length) {
          const rowLine = lines[i].trim();
          if (!rowLine || !rowLine.includes("|") || isAsciiChartLine(rowLine)) break;
          if (isTableDividerLine(rowLine)) {
            i++;
            continue;
          }
          const cells = splitTableCells(rowLine);
          // Pad or trim cells to match header count
          while (cells.length < headers.length) cells.push("");
          rows.push(cells.slice(0, headers.length));
          i++;
        }

        blocks.push({ type: "table", headers, alignments, rows });
        continue;
      }
    }

    // 8. Lists (Unordered or Ordered)
    const ulMatch = line.match(/^[ \t]*([*+-])[ \t]+([^\n]+)$/);
    const olMatch = line.match(/^[ \t]*(\d+)[.)][ \t]+([^\n]+)$/);

    if (ulMatch || olMatch) {
      const isOrdered = !!olMatch;
      const startNum = olMatch ? parseInt(olMatch[1], 10) : undefined;
      const items: string[] = [];

      while (i < lines.length) {
        const current = lines[i];
        const match = isOrdered
          ? current.match(/^[ \t]*\d+[.)][ \t]+([^\n]+)$/)
          : current.match(/^[ \t]*[*+-][ \t]+([^\n]+)$/);

        if (match) {
          items.push(match[1].trim());
          i++;
        } else if (current.startsWith("   ") || current.startsWith("\t")) {
          // Indented continuation line
          if (items.length > 0) {
            items[items.length - 1] += `\n${current.trim()}`;
          }
          i++;
        } else {
          break;
        }
      }

      blocks.push({ type: "list", ordered: isOrdered, start: startNum, items });
      continue;
    }

    // 9. Standard Paragraphs (gather contiguous non-empty lines)
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

export function renderInlineMarkdown(text: string): ReactNode[] {
  if (!text) return [];

  // Match links, inline code, bold-italic, bold, italic, strikethrough
  const tokenRegex =
    /(\[[^\]]+\]\([^)]+\)|`[^`]+`|\*\*\*[^*]+\*\*\*|\*\*[^*]+\*\*|__[^_]+__|\*[^*]+\*|_[^_]+_|~~[^~]+~~)/g;

  const parts = text.split(tokenRegex);

  return parts.map((part, index) => {
    if (!part) return null;

    // Link: [Label](url)
    const linkMatch = part.match(/^\[([^\]]+)\]\(([^)]+)\)$/);
    if (linkMatch) {
      const url = linkMatch[2].trim();
      const safeUrl = /^https?:\/\//i.test(url) || url.startsWith("#") || url.startsWith("/") ? url : `https://${url}`;
      return (
        <a
          key={index}
          href={safeUrl}
          target="_blank"
          rel="noopener noreferrer"
          className="markdown-link"
        >
          {linkMatch[1]}
        </a>
      );
    }

    // Inline Code: `code`
    if (part.startsWith("`") && part.endsWith("`") && part.length >= 2) {
      return (
        <code key={index} className="markdown-inline-code">
          {part.slice(1, -1)}
        </code>
      );
    }

    // Bold-italic: ***text***
    if (part.startsWith("***") && part.endsWith("***") && part.length >= 6) {
      return (
        <strong key={index}>
          <em>{part.slice(3, -3)}</em>
        </strong>
      );
    }

    // Bold: **text** or __text__
    if (
      (part.startsWith("**") && part.endsWith("**") && part.length >= 4) ||
      (part.startsWith("__") && part.endsWith("__") && part.length >= 4)
    ) {
      return <strong key={index}>{part.slice(2, -2)}</strong>;
    }

    // Italic: *text* or _text_
    if (
      (part.startsWith("*") && part.endsWith("*") && part.length >= 2) ||
      (part.startsWith("_") && part.endsWith("_") && part.length >= 2)
    ) {
      return <em key={index}>{part.slice(1, -1)}</em>;
    }

    // Strikethrough: ~~text~~
    if (part.startsWith("~~") && part.endsWith("~~") && part.length >= 4) {
      return <del key={index}>{part.slice(2, -2)}</del>;
    }

    return <span key={index}>{part}</span>;
  });
}

function CodeBlockView({ language, code }: { language: string; code: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    void navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

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
        <code>{code}</code>
      </pre>
    </div>
  );
}

function ChartCardView({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    void navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

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
        <code>{text}</code>
      </pre>
    </div>
  );
}

function TableView({ headers, alignments, rows }: TableBlock) {
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
                {renderInlineMarkdown(header)}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, rowIndex) => (
            <tr key={rowIndex}>
              {row.map((cell, colIndex) => (
                <td
                  key={colIndex}
                  style={{ textAlign: alignments[colIndex] }}
                >
                  {renderInlineMarkdown(cell)}
                </td>
              ))}
            </tr>
          ))}
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
            return <TableView key={index} {...block} />;
          case "code":
            return <CodeBlockView key={index} language={block.language} code={block.code} />;
          case "chart":
            return <ChartCardView key={index} text={block.text} />;
          case "heading": {
            const text = speechProgress?.active
              ? <SpokenText text={block.text} progress={speechProgress} />
              : renderInlineMarkdown(block.text);
            switch (block.level) {
              case 1: return <h1 key={index} className="markdown-h1">{text}</h1>;
              case 2: return <h2 key={index} className="markdown-h2">{text}</h2>;
              case 3: return <h3 key={index} className="markdown-h3">{text}</h3>;
              case 4: return <h4 key={index} className="markdown-h4">{text}</h4>;
              case 5: return <h5 key={index} className="markdown-h5">{text}</h5>;
              case 6: return <h6 key={index} className="markdown-h6">{text}</h6>;
            }
            return <h6 key={index} className="markdown-h6">{text}</h6>;
          }
          case "list": {
            const ListTag = block.ordered ? "ol" : "ul";
            return (
              <ListTag key={index} start={block.start} className="markdown-list">
                {block.items.map((item, itemIdx) => (
                  <li key={itemIdx}>
                    {speechProgress?.active
                      ? <SpokenText text={item} progress={speechProgress} />
                      : renderInlineMarkdown(item)}
                  </li>
                ))}
              </ListTag>
            );
          }
          case "blockquote":
            return (
              <blockquote key={index} className="markdown-blockquote">
                {block.text.split("\n").map((line, lineIdx) => (
                  <p key={lineIdx}>
                    {speechProgress?.active
                      ? <SpokenText text={line} progress={speechProgress} />
                      : renderInlineMarkdown(line)}
                  </p>
                ))}
              </blockquote>
            );
          case "divider":
            return <hr key={index} className="markdown-hr" />;
          case "paragraph":
          default:
            return (
              <p key={index} className="markdown-paragraph">
                {speechProgress?.active ? (
                  <SpokenText text={block.text} progress={speechProgress} />
                ) : (
                  renderInlineMarkdown(block.text)
                )}
              </p>
            );
        }
      })}
      {streaming && <span className="stream-cursor-pulse" aria-hidden="true" />}
    </div>
  );
}
