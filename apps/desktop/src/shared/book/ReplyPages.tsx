import { useState } from "react";
import { TextParagraphs } from "../components/TextParagraphs";
import { FlowPages } from "./FlowPages";
import { ReplyViewToggle, type ReplyView } from "./MessagePages";
import "./paging.css";

/**
 * One model reply that fills its panel: the answer and the reasoning are two views, each turning
 * its own pages. Reasoning shows while the model thinks and the answer takes over when it starts,
 * unless the reader picked a view. While live, the newest page stays open.
 */
export function ReplyPages({
  text,
  reasoning,
  live,
  label,
  answerLabel = "Answer",
  placeholder = "",
  preformatted = false,
  thinkingLevel,
  request,
}: {
  text: string;
  reasoning: string;
  live: boolean;
  label: string;
  answerLabel?: string;
  placeholder?: string;
  /** Keep the answer's exact layout (for structured JSON) instead of reading paragraphs. */
  preformatted?: boolean;
  thinkingLevel?: string;
  /** The exact model request, shown byte for byte as a third view (advanced receipts). */
  request?: string;
}) {
  const [chosen, setChosen] = useState<ReplyView | null>(null);
  const view: ReplyView = chosen ?? (text || !reasoning ? "answer" : "reasoning");
  const shown = view === "request" ? request ?? "" : view === "reasoning" ? reasoning : text || placeholder;
  const off = thinkingLevel === "off";
  // Say plainly why there is no reasoning to show.
  const note = reasoning ? "" : off
    ? "Thinking is turned off for this turn; the model writes the answer directly."
    : live
      ? (text ? "" : "Waiting for the model's thinking…")
      : "This model did not expose a separate thinking channel for this turn.";
  return (
    <div className="reply-pages">
      {(reasoning || request || thinkingLevel || note) && (
        <div className="reply-pages-head">
          {(reasoning || request) && <ReplyViewToggle view={view} onView={setChosen} live={live && !text} answerLabel={answerLabel} reasoning={Boolean(reasoning)} request={Boolean(request)} />}
          {thinkingLevel && <span className={`thinking-level-badge ${off ? "thinking-off-badge" : ""}`}>{thinkingLevel.toUpperCase()}</span>}
          {note && <small>{note}</small>}
        </div>
      )}
      <FlowPages label={label} follow={live ? "end" : "start"} resetKey={view}>
        {view === "request" || (view === "answer" && preformatted)
          ? <pre className="reply-pages-pre">{shown}</pre>
          : <TextParagraphs className="reply-pages-text" text={shown} />}
      </FlowPages>
    </div>
  );
}
