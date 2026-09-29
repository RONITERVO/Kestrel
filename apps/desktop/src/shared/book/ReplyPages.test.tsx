import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { ReplyPages } from "./ReplyPages";

afterEach(cleanup);

describe("ReplyPages", () => {
  it("shows the reasoning while the model thinks and lets the reader switch to the answer", () => {
    const { rerender } = render(<ReplyPages label="Proposal" text="" reasoning="Checking the section arc…" live thinkingLevel="high" />);
    expect(screen.getByText("Checking the section arc…")).toBeInTheDocument();
    expect(screen.getByText("HIGH")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reasoning · live" })).toHaveAttribute("aria-pressed", "true");
    rerender(<ReplyPages label="Proposal" text="A warm piano ballad." reasoning="Checking the section arc…" live thinkingLevel="high" />);
    expect(screen.getByText("A warm piano ballad.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Reasoning" }));
    expect(screen.getByText("Checking the section arc…")).toBeInTheDocument();
  });

  it("shows the exact model request as its own view instead of a fold-out", () => {
    render(<ReplyPages label="Proposal" text="A warm ballad." reasoning="Thinking." live={false} request={'{"max_tokens": 16384}'} />);
    expect(screen.queryByText(/max_tokens/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Model request" }));
    expect(screen.getByText('{"max_tokens": 16384}')).toBeInTheDocument();
    expect(screen.queryByText("A warm ballad.")).not.toBeInTheDocument();
  });

  it("honestly reports a model without a separate reasoning channel", () => {
    render(<ReplyPages label="Proposal" text="Done." reasoning="" live={false} />);
    expect(screen.getByText(/did not expose a separate thinking channel/i)).toBeInTheDocument();
  });

  it("clearly shows when thinking is turned off", () => {
    render(<ReplyPages label="Proposal" text="" reasoning="" live thinkingLevel="off" />);
    expect(screen.getByText("OFF")).toBeInTheDocument();
    expect(screen.getByText(/thinking is turned off for this turn/i)).toBeInTheDocument();
  });
});
