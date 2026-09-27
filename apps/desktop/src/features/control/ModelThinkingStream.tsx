/** Live reasoning is bounded so a very long thinking pass cannot grow without limit. */
const MAX_VISIBLE_THINKING_CHARS = 160_000;
const OMITTED_PREFIX = "[Earlier model thinking omitted from this live view]\n\n";

export function appendModelThinking(current: string, token: string): string {
  const combined = current + token;
  if (combined.length <= MAX_VISIBLE_THINKING_CHARS) return combined;
  return OMITTED_PREFIX + combined.slice(-(MAX_VISIBLE_THINKING_CHARS - OMITTED_PREFIX.length));
}
