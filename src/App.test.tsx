import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import App from "./App";
import { terminalTaskStatus } from "./OfflineWorkspace";

afterEach(cleanup);

describe("Kestrel research experience", () => {
  it("opens the durable library and renders evidence-oriented research", async () => {
    render(<App />);
    expect(await screen.findByRole("heading", { name: "The Antikythera mechanism" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Key findings" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Sources inspected" })).toBeInTheDocument();
    expect(screen.getByText(/Produced entirely on this computer/)).toBeInTheDocument();
  });

  it("explains offline scope before a new research run", async () => {
    render(<App />);
    const buttons = await screen.findAllByRole("button", { name: /New research/i });
    fireEvent.click(buttons[0]);
    expect(screen.getByRole("dialog", { name: "What would you like to understand?" })).toBeInTheDocument();
    expect(screen.getByText(/No web requests/)).toBeInTheDocument();
    const begin = screen.getByRole("button", { name: /Begin research/ });
    expect(begin).toBeDisabled();
    fireEvent.change(screen.getByPlaceholderText(/Ask a question/), { target: { value: "How did Roman concrete work?" } });
    expect(begin).toBeEnabled();
  });

  it("keeps system and advanced controls one step from research", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^System$/i }));
    expect(await screen.findByRole("heading", { name: "System" })).toBeInTheDocument();
    expect(screen.getByText(/one model researcher/i)).toBeInTheDocument();
    expect(screen.getByText(/intentionally uncapped/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^Research$/i }));
    expect(await screen.findByText("Your research")).toBeInTheDocument();
  });

  it("keeps the historical control plane and optional developer repair discoverable", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^Control$/i }));
    expect(await screen.findByRole("heading", { name: /Ternary Bonsai/i })).toBeInTheDocument();
    expect(screen.getByText("SESSION INSPECTOR")).toBeInTheDocument();
    expect(screen.getByText(/one inference lease/i)).toBeInTheDocument();
    expect(screen.getByText(/private, persistent workspace/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Computer/i }));
    expect(screen.getByRole("heading", { name: /bounded objective/i })).toBeInTheDocument();
    expect(screen.getByText(/Every decision, tool call, result, error, and artifact/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Start visible task/i })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: /^Developer$/i }));
    expect(await screen.findByRole("heading", { name: "Developer" })).toBeInTheDocument();
    expect(screen.getByText(/Offline independence/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Run offline diagnostics/i })).toBeInTheDocument();
  });
});

describe("computer task terminal states", () => {
  it.each([
    ["done", "completed"],
    ["cancelled", "cancelled"],
    ["error", "failed"],
    ["limit", "failed"],
  ])("maps %s to %s", (kind, expected) => {
    expect(terminalTaskStatus(kind, "running")).toBe(expected);
  });

  it("promotes the starting fallback while waiting for events", () => {
    expect(terminalTaskStatus("start", "starting")).toBe("running");
  });
});
