import { LoaderCircle, Sparkles, ZapOff } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";
import type { ThinkingLevel } from "./types";

const MAX_VISIBLE_THINKING_CHARS = 160_000;
const OMITTED_PREFIX = "[Earlier model thinking omitted from this live view]\n\n";

export function appendModelThinking(current: string, token: string): string {
  const combined = current + token;
  if (combined.length <= MAX_VISIBLE_THINKING_CHARS) return combined;
  return OMITTED_PREFIX + combined.slice(-(MAX_VISIBLE_THINKING_CHARS - OMITTED_PREFIX.length));
}

export function ModelThinkingStream({
  text,
  active,
  modelName,
  thinkingLevel,
  thinkingOff = false,
  className = "",
}: {
  text: string;
  active: boolean;
  modelName?: string;
  thinkingLevel?: ThinkingLevel | string;
  thinkingOff?: boolean;
  className?: string;
}) {
  const streamRef = useRef<HTMLPreElement>(null);
  const followingRef = useRef(true);
  const [following, setFollowing] = useState(true);

  const isOff = thinkingOff || thinkingLevel === "off";
  const levelDisplay = thinkingLevel && thinkingLevel !== "off" ? String(thinkingLevel).toUpperCase() : "";

  const message = isOff
    ? "Thinking is turned off for this turn. The model writes directly to the response without a separate reasoning pass."
    : text || (active
      ? `Waiting for explicit thinking from the local model${thinkingLevel && thinkingLevel !== "off" ? ` (${thinkingLevel} level)` : ""}…`
      : "This model did not expose a separate thinking channel for this turn.");

  useLayoutEffect(() => {
    const stream = streamRef.current;
    if (!stream || !followingRef.current) return;
    stream.scrollTop = stream.scrollHeight;
  }, [active, text]);

  const followLive = () => {
    const stream = streamRef.current;
    followingRef.current = true;
    setFollowing(true);
    if (stream) stream.scrollTop = stream.scrollHeight;
  };

  return <section className={`model-thinking-stream ${isOff ? "thinking-off" : active ? "live" : "saved"} ${className}`.trim()} aria-label="Model thinking stream">
    <header>
      <span>
        {isOff ? <ZapOff className="icon-sm" /> : active ? <LoaderCircle className="spin icon-sm" /> : <Sparkles className="icon-sm" />}
        <strong>Model thinking</strong>
        {isOff ? (
          <span className="thinking-level-badge thinking-off-badge">OFF</span>
        ) : levelDisplay ? (
          <span className="thinking-level-badge">{levelDisplay}</span>
        ) : null}
      </span>
      <span className="model-thinking-meta">
        <small>{modelName ? `${modelName} · ` : ""}{isOff ? "thinking off" : active ? "live" : "turn ended"}</small>
        {!isOff && active && !following && <button onClick={followLive}>Jump live</button>}
      </span>
    </header>
    <pre ref={streamRef} className={text && !isOff ? "" : isOff ? "model-thinking-empty model-thinking-off" : "model-thinking-empty"} onScroll={(event) => {
      const stream = event.currentTarget;
      const next = stream.scrollHeight - stream.scrollTop - stream.clientHeight < 24;
      followingRef.current = next;
      setFollowing(next);
    }}>{message}</pre>
    <footer>{isOff ? "Thinking is disabled in settings. Enable Low, Medium, High, or Max to stream model thoughts." : "Provisional working notes only. Kestrel never applies this text to the production."}</footer>
  </section>;
}
