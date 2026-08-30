import { useEffect, useRef, useSyncExternalStore } from "react";

export interface InferenceTelemetrySnapshot {
  sourceId: string;
  modelName?: string;
  tokensPerSecond?: number;
  tokenCount: number;
  active: boolean;
  exact: boolean;
  observedAt: number;
}

const EMPTY_SNAPSHOT: InferenceTelemetrySnapshot = {
  sourceId: "",
  tokenCount: 0,
  active: false,
  exact: false,
  observedAt: 0,
};

let snapshot = EMPTY_SNAPSHOT;
const listeners = new Set<() => void>();

function publish(next: InferenceTelemetrySnapshot) {
  snapshot = next;
  listeners.forEach((listener) => listener());
}

export function getInferenceTelemetrySnapshot(): InferenceTelemetrySnapshot {
  return snapshot;
}

export function subscribeInferenceTelemetry(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function beginInferenceTelemetry(sourceId: string, modelName?: string) {
  publish({ sourceId, modelName, tokenCount: 0, active: true, exact: false, observedAt: Date.now() });
}

export function updateInferenceTelemetry(
  sourceId: string,
  update: Pick<InferenceTelemetrySnapshot, "tokenCount" | "tokensPerSecond" | "exact">,
) {
  if (snapshot.sourceId !== sourceId || !snapshot.active) return;
  publish({ ...snapshot, ...update, observedAt: Date.now() });
}

export function finishInferenceTelemetry(
  sourceId: string,
  update?: Pick<InferenceTelemetrySnapshot, "tokenCount" | "tokensPerSecond" | "exact">,
) {
  if (snapshot.sourceId !== sourceId) return;
  publish({ ...snapshot, ...update, active: false, observedAt: Date.now() });
}

export function useInferenceTelemetry(): InferenceTelemetrySnapshot {
  return useSyncExternalStore(
    subscribeInferenceTelemetry,
    getInferenceTelemetrySnapshot,
    getInferenceTelemetrySnapshot,
  );
}

let nextReporterId = 0;

/**
 * Reports one producer-visible local-model turn to the single app-level speed display.
 * Kestrel's inference gate permits only one active model turn, so the newest active reporter owns
 * the header. A reporter may replace its live estimate with llama-server's exact final timing.
 */
export function useInferenceTelemetryReporter({
  active,
  text,
  modelName,
  exactTokensPerSecond,
  exactTokenCount,
}: {
  active: boolean;
  text: string;
  modelName?: string;
  exactTokensPerSecond?: number;
  exactTokenCount?: number;
}) {
  const sourceIdRef = useRef("");
  if (!sourceIdRef.current) sourceIdRef.current = `inference-${++nextReporterId}`;
  const startedAtRef = useRef<number | undefined>(undefined);
  const wasActiveRef = useRef(false);
  const latestRef = useRef({ tokenCount: 0, tokensPerSecond: undefined as number | undefined, exact: false });

  useEffect(() => {
    const sourceId = sourceIdRef.current;
    if (!active) {
      if (wasActiveRef.current) finishInferenceTelemetry(sourceId, latestRef.current);
      wasActiveRef.current = false;
      return;
    }
    if (!wasActiveRef.current) {
      startedAtRef.current = performance.now();
      latestRef.current = { tokenCount: 0, tokensPerSecond: undefined, exact: false };
      beginInferenceTelemetry(sourceId, modelName);
    }
    wasActiveRef.current = true;

    const report = () => {
      const tokenCount = exactTokenCount ?? Math.max(0, Math.round(text.length / 3.8));
      const elapsed = startedAtRef.current === undefined
        ? 0
        : Math.max(0.1, (performance.now() - startedAtRef.current) / 1_000);
      const exact = Number.isFinite(exactTokensPerSecond) && (exactTokensPerSecond ?? 0) > 0;
      const tokensPerSecond = exact
        ? Number(exactTokensPerSecond!.toFixed(1))
        : tokenCount > 0
          ? Number((tokenCount / elapsed).toFixed(1))
          : undefined;
      latestRef.current = { tokenCount, tokensPerSecond, exact };
      updateInferenceTelemetry(sourceId, latestRef.current);
    };
    report();
    const timer = window.setInterval(report, 250);
    return () => window.clearInterval(timer);
  }, [active, exactTokenCount, exactTokensPerSecond, modelName, text]);

  useEffect(() => () => {
    if (wasActiveRef.current) finishInferenceTelemetry(sourceIdRef.current, latestRef.current);
  }, []);
}
