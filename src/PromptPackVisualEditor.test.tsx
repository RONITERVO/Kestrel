import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PromptPackVisualEditor } from "./PromptPackVisualEditor";

afterEach(() => {
  cleanup();
});

function pack(prompts: Record<string, string>): string {
  return JSON.stringify({ format: "kestrel.prompt-pack", version: 1, prompts }, null, 2);
}

const basePrompts = {
  "chat.system": "You are Kestrel. Use {{tone}} and stay concise.",
  "computer.system": "You control the computer with tools.",
  "computer.tool.read_file": "Read a file at the given path.",
  "research.system": "Research using {{model_label}} and cite sources.",
};

describe("PromptPackVisualEditor", () => {
  it("lists every prompt grouped into categories derived from the key prefix", () => {
    const text = pack(basePrompts);
    render(<PromptPackVisualEditor jsonText={text} savedJsonText={text} defaultJsonText={text} disabled={false} onChange={vi.fn()}/>);
    expect(screen.getByLabelText(/Edit prompt chat\.system/)).toBeInTheDocument();
    expect(screen.getByLabelText(/Edit prompt computer\.tool\.read_file/)).toBeInTheDocument();
    expect(screen.getByLabelText(/Show computer category \(2 prompts\)/)).toBeInTheDocument();
    expect(screen.getByLabelText(/Show all categories \(4 prompts\)/)).toBeInTheDocument();
  });

  it("shows the selected prompt's text in the editor", () => {
    const text = pack(basePrompts);
    render(<PromptPackVisualEditor jsonText={text} savedJsonText={text} defaultJsonText={text} disabled={false} onChange={vi.fn()}/>);
    fireEvent.click(screen.getByLabelText(/Edit prompt computer\.tool\.read_file/));
    expect(screen.getByLabelText("Prompt text for computer.tool.read_file")).toHaveValue("Read a file at the given path.");
  });

  it("edits only the selected prompt's value and preserves the rest of the pack", () => {
    const text = pack(basePrompts);
    const onChange = vi.fn();
    render(<PromptPackVisualEditor jsonText={text} savedJsonText={text} defaultJsonText={text} disabled={false} onChange={onChange}/>);
    fireEvent.click(screen.getByLabelText(/Edit prompt chat\.system/));
    fireEvent.change(screen.getByLabelText("Prompt text for chat.system"), { target: { value: "New instructions with {{tone}}." } });
    expect(onChange).toHaveBeenCalledTimes(1);
    const updated = JSON.parse(onChange.mock.calls[0][0]);
    expect(updated.prompts["chat.system"]).toBe("New instructions with {{tone}}.");
    expect(updated.prompts["computer.system"]).toBe(basePrompts["computer.system"]);
  });

  it("filters the list by search text across keys and prompt content", () => {
    const text = pack(basePrompts);
    render(<PromptPackVisualEditor jsonText={text} savedJsonText={text} defaultJsonText={text} disabled={false} onChange={vi.fn()}/>);
    fireEvent.change(screen.getByPlaceholderText(/Search prompt keys or text/), { target: { value: "cite sources" } });
    expect(screen.getByLabelText(/Edit prompt research\.system/)).toBeInTheDocument();
    expect(screen.queryByLabelText(/Edit prompt chat\.system/)).not.toBeInTheDocument();
  });

  it("filters the list by category", () => {
    const text = pack(basePrompts);
    render(<PromptPackVisualEditor jsonText={text} savedJsonText={text} defaultJsonText={text} disabled={false} onChange={vi.fn()}/>);
    fireEvent.click(screen.getByLabelText(/Show computer category/));
    expect(screen.getByLabelText(/Edit prompt computer\.system/)).toBeInTheDocument();
    expect(screen.getByLabelText(/Edit prompt computer\.tool\.read_file/)).toBeInTheDocument();
    expect(screen.queryByLabelText(/Edit prompt chat\.system/)).not.toBeInTheDocument();
  });

  it("marks a prompt as modified only when it differs from the last saved pack", () => {
    const saved = pack(basePrompts);
    const current = pack({ ...basePrompts, "chat.system": "Edited system prompt." });
    render(<PromptPackVisualEditor jsonText={current} savedJsonText={saved} defaultJsonText={saved} disabled={false} onChange={vi.fn()}/>);
    const changedRow = screen.getByLabelText(/Edit prompt chat\.system/);
    const unchangedRow = screen.getByLabelText(/Edit prompt research\.system/);
    expect(changedRow.querySelector(".prompt-visual-modified-dot")).not.toBeNull();
    expect(unchangedRow.querySelector(".prompt-visual-modified-dot")).toBeNull();
  });

  it("reverts the selected prompt to the last saved value", () => {
    const saved = pack(basePrompts);
    const current = pack({ ...basePrompts, "chat.system": "Edited system prompt." });
    const onChange = vi.fn();
    render(<PromptPackVisualEditor jsonText={current} savedJsonText={saved} defaultJsonText={saved} disabled={false} onChange={onChange}/>);
    fireEvent.click(screen.getByLabelText(/Edit prompt chat\.system/));
    fireEvent.click(screen.getByRole("button", { name: /Revert to saved/ }));
    const updated = JSON.parse(onChange.mock.calls[0][0]);
    expect(updated.prompts["chat.system"]).toBe(basePrompts["chat.system"]);
  });

  it("resets the selected prompt to the build default", () => {
    const defaults = pack(basePrompts);
    const current = pack({ ...basePrompts, "chat.system": "Edited system prompt." });
    const onChange = vi.fn();
    render(<PromptPackVisualEditor jsonText={current} savedJsonText={current} defaultJsonText={defaults} disabled={false} onChange={onChange}/>);
    fireEvent.click(screen.getByLabelText(/Edit prompt chat\.system/));
    fireEvent.click(screen.getByRole("button", { name: /Reset to default/ }));
    const updated = JSON.parse(onChange.mock.calls[0][0]);
    expect(updated.prompts["chat.system"]).toBe(basePrompts["chat.system"]);
  });

  it("disables revert/reset actions when there is nothing to revert or reset", () => {
    const text = pack(basePrompts);
    render(<PromptPackVisualEditor jsonText={text} savedJsonText={text} defaultJsonText={text} disabled={false} onChange={vi.fn()}/>);
    fireEvent.click(screen.getByLabelText(/Edit prompt chat\.system/));
    expect(screen.getByRole("button", { name: /Revert to saved/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Reset to default/ })).toBeDisabled();
  });

  it("warns when a prompt's variables no longer match the build default's required set", () => {
    const defaults = pack(basePrompts);
    const current = pack({ ...basePrompts, "chat.system": "You are Kestrel. Stay concise without a tone variable." });
    render(<PromptPackVisualEditor jsonText={current} savedJsonText={current} defaultJsonText={defaults} disabled={false} onChange={vi.fn()}/>);
    fireEvent.click(screen.getByLabelText(/Edit prompt chat\.system/));
    expect(screen.getByText(/Kestrel will reject this prompt until it uses exactly these variables/)).toBeInTheDocument();
    expect(screen.getByText("{{tone}}")).toHaveClass("missing");
  });

  it("shows a friendly message instead of crashing when the JSON is not valid", () => {
    render(<PromptPackVisualEditor jsonText="{ not valid json" savedJsonText="" defaultJsonText="" disabled={false} onChange={vi.fn()}/>);
    expect(screen.getByText(/The JSON has a syntax error\./)).toBeInTheDocument();
  });

  it("disables the textarea while a save/reset/import is in progress", () => {
    const text = pack(basePrompts);
    render(<PromptPackVisualEditor jsonText={text} savedJsonText={text} defaultJsonText={text} disabled onChange={vi.fn()}/>);
    fireEvent.click(screen.getByLabelText(/Edit prompt chat\.system/));
    expect(screen.getByLabelText("Prompt text for chat.system")).toBeDisabled();
  });
});
