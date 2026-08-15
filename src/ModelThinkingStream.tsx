import { LoaderCircle, Sparkles } from "lucide-react";
import { useLayoutEffect, useRef, useState } from "react";

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
  className = "",
}: {
  text: string;
  active: boolean;
  modelName?: string;
  className?: string;
}) {
  const streamRef = useRef<HTMLPreElement>(null);
  const followingRef = useRef(true);
  const [following, setFollowing] = useState(true);
  const message = text || (active
    ? "Waiting for an explicit thinking channel from the local model…"
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

  return <section className={`model-thinking-stream ${active ? "live" : "saved"} ${className}`.trim()} aria-label="Model thinking stream">
    <header>
      <span>{active ? <LoaderCircle className="spin" /> : <Sparkles />}<strong>Model thinking</strong></span>
      <span className="model-thinking-meta"><small>{modelName ? `${modelName} · ` : ""}{active ? "live" : "turn ended"}</small>{active && !following && <button onClick={followLive}>Jump live</button>}</span>
    </header>
    <pre ref={streamRef} className={text ? "" : "model-thinking-empty"} onScroll={(event) => {
      const stream = event.currentTarget;
      const next = stream.scrollHeight - stream.scrollTop - stream.clientHeight < 24;
      followingRef.current = next;
      setFollowing(next);
    }}>{message}</pre>
    <footer>Provisional working notes only. Kestrel never applies this text to the production.</footer>
  </section>;
}
