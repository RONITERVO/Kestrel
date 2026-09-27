import { describe, expect, it } from "vitest";
import { appendModelThinking } from "./ModelThinkingStream";

describe("appendModelThinking", () => {
  it("bounds a long live stream while retaining its newest tokens", () => {
    const result = appendModelThinking("a".repeat(160_000), "latest-token");
    expect(result).toContain("Earlier model thinking omitted");
    expect(result.endsWith("latest-token")).toBe(true);
    expect(result.length).toBeLessThanOrEqual(160_000);
  });
});
