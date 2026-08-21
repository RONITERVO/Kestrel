import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MarkdownContent } from "./MarkdownContent";

beforeEach(() => {
  Object.assign(navigator, {
    clipboard: {
      writeText: vi.fn().mockResolvedValue(undefined),
    },
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("MarkdownContent component", () => {
  it("renders GitHub-style markdown tables with header alignment and cell data", () => {
    const tableMarkdown = `
# Benchmark Results

| Model | Speed | Accuracy | Status |
| :--- | :---: | ---: | --- |
| Chatterbox | 0.4s | 99.2% | Active |
| Whisper | 0.2s | 98.5% | Ready |
`;
    render(<MarkdownContent value={tableMarkdown} />);

    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Benchmark Results");
    expect(screen.getByRole("table")).toBeInTheDocument();

    const headers = screen.getAllByRole("columnheader");
    expect(headers).toHaveLength(4);
    expect(headers[0]).toHaveTextContent("Model");
    expect(headers[0]).toHaveStyle({ textAlign: "left" });
    expect(headers[1]).toHaveTextContent("Speed");
    expect(headers[1]).toHaveStyle({ textAlign: "center" });
    expect(headers[2]).toHaveTextContent("Accuracy");
    expect(headers[2]).toHaveStyle({ textAlign: "right" });

    const cells = screen.getAllByRole("cell");
    expect(cells).toHaveLength(8);
    expect(cells[0]).toHaveTextContent("Chatterbox");
    expect(cells[1]).toHaveTextContent("0.4s");
    expect(cells[2]).toHaveTextContent("99.2%");
    expect(cells[3]).toHaveTextContent("Active");
  });

  it("renders code blocks with language badge and working copy button", async () => {
    const codeMarkdown = `
Here is the code:
\`\`\`typescript
interface User {
  id: string;
  name: string;
}
\`\`\`
`;
    render(<MarkdownContent value={codeMarkdown} />);

    expect(screen.getByText("typescript")).toBeInTheDocument();
    expect(screen.getByText(/interface User/)).toBeInTheDocument();

    const copyBtn = screen.getByRole("button", { name: "Copy code to clipboard" });
    expect(copyBtn).toHaveTextContent("Copy");

    fireEvent.click(copyBtn);
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
      "interface User {\n  id: string;\n  name: string;\n}",
    );
    expect(screen.getByText("Copied")).toBeInTheDocument();
  });

  it("detects and renders ASCII diagrams and text charts in chart cards", () => {
    const chartMarkdown = `
Architecture Diagram:
+-------------------+      +-------------------+
| Chatterbox TTS    | ---> | Audio Synthesis   |
+-------------------+      +-------------------+
| Whisper Alignment | ---> | Timestamp Engine  |
+-------------------+      +-------------------+
`;
    render(<MarkdownContent value={chartMarkdown} />);

    expect(screen.getByText("Diagram / Text Chart")).toBeInTheDocument();
    expect(screen.getByText(/Chatterbox TTS/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy diagram to clipboard" })).toBeInTheDocument();
  });

  it("renders headings from H1 to H6, lists, blockquotes, and horizontal rules", () => {
    const markdown = `
# Heading 1
## Heading 2
### Heading 3
#### Heading 4
##### Heading 5
###### Heading 6

> This is a critical architectural note.
> Another quote line.

---

* First unordered item
* Second unordered item

1. First ordered step
2. Second ordered step
`;
    const { container } = render(<MarkdownContent value={markdown} />);

    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Heading 1");
    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Heading 2");
    expect(screen.getByRole("heading", { level: 3 })).toHaveTextContent("Heading 3");
    expect(screen.getByRole("heading", { level: 4 })).toHaveTextContent("Heading 4");
    expect(screen.getByRole("heading", { level: 5 })).toHaveTextContent("Heading 5");
    expect(screen.getByRole("heading", { level: 6 })).toHaveTextContent("Heading 6");

    const blockquote = container.querySelector("blockquote");
    expect(blockquote).toBeInTheDocument();
    expect(blockquote).toHaveTextContent("This is a critical architectural note.");

    expect(container.querySelector("hr")).toBeInTheDocument();

    const listItems = screen.getAllByRole("listitem");
    expect(listItems).toHaveLength(4);
    expect(listItems[0]).toHaveTextContent("First unordered item");
    expect(listItems[2]).toHaveTextContent("First ordered step");
  });

  it("renders inline bold, italic, strikethrough, inline code, and safe links", () => {
    const inline = `Check **bold text**, *italic text*, ***bold italic***, ~~strikethrough~~, \`const x = 10;\`, and [Kestrel Docs](https://kestrel.local/docs).`;
    const { container } = render(<MarkdownContent value={inline} />);

    expect(container.querySelector("strong")).toHaveTextContent("bold text");
    expect(container.querySelector("em")).toHaveTextContent("italic text");
    expect(container.querySelector("del")).toHaveTextContent("strikethrough");
    expect(container.querySelector("code")).toHaveTextContent("const x = 10;");

    const link = screen.getByRole("link", { name: "Kestrel Docs" });
    expect(link).toHaveAttribute("href", "https://kestrel.local/docs");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
  });

  it("handles live streaming token generation with streaming cursor and partial unclosed blocks", () => {
    const unclosedCode = "Generating response:\n```python\ndef compute():\n    return 42";
    const { container, rerender } = render(<MarkdownContent value={unclosedCode} streaming={true} />);

    expect(container.querySelector(".stream-cursor-pulse")).toBeInTheDocument();
    expect(screen.getByText(/def compute/)).toBeInTheDocument();

    rerender(<MarkdownContent value="" streaming={true} />);
    expect(container.querySelector(".stream-cursor-pulse")).toBeInTheDocument();
  });

  it("renders live word-level highlight when speechProgress is provided", () => {
    const text = "Kestrel provides fast local inference.";
    const { container } = render(
      <MarkdownContent
        value={text}
        speechProgress={{
          active: true,
          passageId: "p1",
          text,
          seconds: 0.8,
          duration: 2.0,
          timings: [
            { value: "Kestrel", start: 0, end: 0.4 },
            { value: "provides", start: 0.4, end: 0.8 },
            { value: "fast", start: 0.8, end: 1.2 },
            { value: "local", start: 1.2, end: 1.6 },
            { value: "inference.", start: 1.6, end: 2.0 },
          ],
        }}
      />,
    );

    const mark = container.querySelector("mark.speech-word-active");
    expect(mark).toBeInTheDocument();
    expect(mark).toHaveTextContent("fast");
  });

  it("renders live word-level highlight inside ASCII diagram / text charts", () => {
    const chartMarkdown = "+-------------------+\n| Chatterbox TTS    |\n+-------------------+\n";
    const { container } = render(
      <MarkdownContent
        value={chartMarkdown}
        speechProgress={{
          active: true,
          passageId: "p1",
          text: "Chatterbox TTS",
          seconds: 0.1,
          duration: 1.0,
          timings: [
            { value: "Chatterbox", start: 0, end: 0.5 },
            { value: "TTS", start: 0.5, end: 1.0 },
          ],
        }}
      />,
    );

    const mark = container.querySelector(".markdown-chart-body mark.speech-word-active");
    expect(mark).toBeInTheDocument();
    expect(mark).toHaveTextContent("Chatterbox");

    // Symbol tokens (diagram lines) must be preserved
    const symbols = container.querySelectorAll(".speech-symbol-token");
    expect(symbols.length).toBeGreaterThanOrEqual(2);
  });

  it("renders live word-level highlight inside Markdown table headers and cells", () => {
    const tableMarkdown = `| Empire | Duration |\n| --- | --- |\n| Akkadian | 180 years |`;
    const { container } = render(
      <MarkdownContent
        value={tableMarkdown}
        speechProgress={{
          active: true,
          passageId: "p1",
          text: "Akkadian 180 years",
          seconds: 0.1,
          duration: 1.0,
          timings: [
            { value: "Akkadian", start: 0, end: 0.5 },
            { value: "180", start: 0.5, end: 0.8 },
            { value: "years", start: 0.8, end: 1.0 },
          ],
        }}
      />,
    );

    const mark = container.querySelector(".markdown-table mark.speech-word-active");
    expect(mark).toBeInTheDocument();
    expect(mark).toHaveTextContent("Akkadian");
  });
});
