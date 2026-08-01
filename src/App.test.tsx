import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import App from "./App";

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
});
