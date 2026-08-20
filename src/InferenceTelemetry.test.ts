import { describe, expect, it } from "vitest";
import {
  beginInferenceTelemetry,
  finishInferenceTelemetry,
  getInferenceTelemetrySnapshot,
  updateInferenceTelemetry,
} from "./InferenceTelemetry";

describe("inference telemetry", () => {
  it("keeps the last completed speed visible and ignores a stale reporter", () => {
    beginInferenceTelemetry("first", "Model A");
    updateInferenceTelemetry("first", { tokenCount: 12, tokensPerSecond: 8.5, exact: false });
    beginInferenceTelemetry("second", "Model B");
    finishInferenceTelemetry("first", { tokenCount: 20, tokensPerSecond: 9, exact: false });
    updateInferenceTelemetry("second", { tokenCount: 30, tokensPerSecond: 14.2, exact: true });
    finishInferenceTelemetry("second", { tokenCount: 30, tokensPerSecond: 14.2, exact: true });

    expect(getInferenceTelemetrySnapshot()).toMatchObject({
      sourceId: "second",
      modelName: "Model B",
      tokensPerSecond: 14.2,
      tokenCount: 30,
      exact: true,
      active: false,
    });
  });
});
