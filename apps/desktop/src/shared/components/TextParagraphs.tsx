import { useMemo } from "react";
import "./markdown.css";

/**
 * Plain model prose such as reasoning, shown as the paragraphs the model wrote. Book pages then
 * break between paragraphs instead of mid-sentence. The text itself is never changed.
 */
export function TextParagraphs({ text, className = "" }: { text: string; className?: string }) {
  // One or more blank (or whitespace-only) lines end a paragraph.
  const paragraphs = useMemo(() => text.split(/\n(?:[ \t]*\n)+/).filter((part) => part.trim()), [text]);
  return (
    <div className={`text-paragraphs ${className}`}>
      {paragraphs.map((part, index) => <p key={index}>{part}</p>)}
    </div>
  );
}
