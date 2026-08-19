import { LoaderCircle, Sparkles, Zap, ZapOff } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ThinkingLevel } from "./types";

const MAX_VISIBLE_THINKING_CHARS = 160_000;
const OMITTED_PREFIX = "[Earlier model thinking omitted from this live view]\n\n";

export function appendModelThinking(current: string, token: string): string {
  const combined = current + token;
  if (combined.length <= MAX_VISIBLE_THINKING_CHARS) return combined;
  return OMITTED_PREFIX + combined.slice(-(MAX_VISIBLE_THINKING_CHARS - OMITTED_PREFIX.length));
}

export interface StreamMetricsState {
  tokens: number;
  tokPerSec: number;
  elapsedSec: number;
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

  const startRef = useRef<number | null>(null);
  const [metrics, setMetrics] = useState<StreamMetricsState | null>(null);

  // Track start time and live speed metrics
  useEffect(() => {
    if (isOff) return;
    if (active) {
      if (text.length > 0 && !startRef.current) {
        startRef.current = performance.now();
      }
      const interval = window.setInterval(() => {
        if (!startRef.current || text.length === 0) return;
        const elapsed = Math.max(0.1, (performance.now() - startRef.current) / 1000);
        const tokens = Math.max(1, Math.round(text.length / 3.8));
        const tokPerSec = Number((tokens / elapsed).toFixed(1));
        setMetrics({ tokens, tokPerSec, elapsedSec: Number(elapsed.toFixed(1)) });
      }, 250);
      return () => window.clearInterval(interval);
    } else {
      if (startRef.current && text.length > 0) {
        const elapsed = Math.max(0.1, (performance.now() - startRef.current) / 1000);
        const tokens = Math.max(1, Math.round(text.length / 3.8));
        const tokPerSec = Number((tokens / elapsed).toFixed(1));
        setMetrics({ tokens, tokPerSec, elapsedSec: Number(elapsed.toFixed(1)) });
      }
    }
  }, [active, text, isOff]);

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
        {!isOff && metrics && text.length > 0 && (
          <span className={`thinking-speed-badge ${active ? "live" : "settled"}`} title={`${metrics.tokens} estimated tokens in ${metrics.elapsedSec}s`}>
            <Zap size={9} /> {metrics.tokPerSec} tok/s · {metrics.tokens} tok
          </span>
        )}
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
