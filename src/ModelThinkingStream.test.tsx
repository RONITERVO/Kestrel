import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { appendModelThinking, ModelThinkingStream } from "./ModelThinkingStream";

describe("ModelThinkingStream", () => {
  it("shows explicit reasoning while a local model is working", () => {
    render(<ModelThinkingStream text="Checking the section arc…" active modelName="Bonsai" thinkingLevel="high" />);
    expect(screen.getByRole("region", { name: /model thinking stream/i })).toHaveTextContent("Checking the section arc…");
    expect(screen.getByText(/Bonsai · live/i)).toBeInTheDocument();
    expect(screen.getByText("HIGH")).toBeInTheDocument();
  });

  it("honestly reports models without a separate reasoning channel", () => {
    render(<ModelThinkingStream text="" active={false} />);
    expect(screen.getByText(/did not expose a separate thinking channel/i)).toBeInTheDocument();
  });

  it("clearly shows when thinking is turned off", () => {
    render(<ModelThinkingStream text="" active={false} modelName="Bonsai" thinkingLevel="off" />);
    expect(screen.getByText("OFF")).toBeInTheDocument();
    expect(screen.getByText(/thinking is turned off for this turn/i)).toBeInTheDocument();
    expect(screen.getByText(/thinking off/i)).toBeInTheDocument();
  });

  it("bounds a long live stream while retaining its newest tokens", () => {
    const result = appendModelThinking("a".repeat(160_000), "latest-token");
    expect(result).toContain("Earlier model thinking omitted");
    expect(result.endsWith("latest-token")).toBe(true);
    expect(result.length).toBeLessThanOrEqual(160_000);
  });
});
