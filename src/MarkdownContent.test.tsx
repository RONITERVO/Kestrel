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

  it("uses only the first trimmed code-fence info token as the language", () => {
    render(<MarkdownContent value={'```  typescript title="sample"\nconst ready = true;\n```'} />);
    expect(screen.getByText("typescript")).toBeInTheDocument();
    expect(screen.queryByText(/title="sample"/)).not.toBeInTheDocument();
  });

  it("keeps indented list continuation text with its item", () => {
    render(<MarkdownContent value={"- First line\n  continued detail\n- Second line"} />);
    const items = screen.getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("First line continued detail");
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

  it("starts from the exact timestamp when a timed word is clicked", () => {
    const onSeek = vi.fn();
    const text = "Kestrel provides fast local inference.";
    render(
      <MarkdownContent
        value={text}
        speechProgress={{
          active: true,
          passageId: "p1",
          text,
          seconds: 0.1,
          duration: 2.0,
          timings: [
            { value: "Kestrel", start: 0, end: 0.4 },
            { value: "provides", start: 0.4, end: 0.8 },
            { value: "fast", start: 0.8, end: 1.2 },
            { value: "local", start: 1.2, end: 1.6 },
            { value: "inference", start: 1.6, end: 2.0 },
          ],
          onSeek,
        }}
      />,
    );

    screen.getByRole("button", { name: "Play from local" }).click();
    expect(onSeek).toHaveBeenCalledOnce();
    expect(onSeek).toHaveBeenCalledWith(1.2);
  });

  it("keeps cached timed words clickable while playback is idle", () => {
    const onSeek = vi.fn();
    const text = "Click any saved word.";
    render(
      <MarkdownContent
        value={text}
        speechProgress={{
          active: false,
          passageId: "recording-1",
          text,
          seconds: 0,
          duration: 1.4,
          timings: [
            { value: "Click", start: 0, end: 0.3 },
            { value: "any", start: 0.3, end: 0.6 },
            { value: "saved", start: 0.6, end: 1.0 },
            { value: "word", start: 1.0, end: 1.4 },
          ],
          onSeek,
        }}
      />,
    );

    screen.getByRole("button", { name: "Play from saved" }).click();
    expect(onSeek).toHaveBeenCalledWith(0.6);
    expect(screen.queryByText("Click", { selector: "mark" })).not.toBeInTheDocument();
  });

  it("can restart from words in earlier cached passages while another passage is active", () => {
    const onSeekPassage = vi.fn();
    const first = "Photon energy is lower for red light.";
    const second = "Hot objects begin by glowing red.";
    const third = "Cultural association links red with heat.";
    const timingPassage = (passageId: string, text: string, offset: number) => ({
      passageId,
      text,
      timings: text.match(/[\p{L}\p{N}]+/gu)!.map((value, index) => ({
        value,
        start: offset + index * 0.25,
        end: offset + (index + 1) * 0.25,
      })),
    });
    const passages = [
      timingPassage("answer-1", first, 0),
      timingPassage("answer-2", second, 0),
      timingPassage("answer-3", third, 0),
    ];

    render(
      <MarkdownContent
        value={`${first}\n\n${second}\n\n${third}`}
        speechProgress={{
          active: true,
          passageId: "answer-3",
          text: third,
          seconds: 0.1,
          duration: 1.75,
          timings: passages[2].timings,
          onSeek: vi.fn(),
          seekablePassages: passages,
          onSeekPassage,
        }}
      />,
    );

    screen.getByRole("button", { name: "Play from lower" }).click();
    expect(onSeekPassage).toHaveBeenLastCalledWith("answer-1", 0.75);
    screen.getByRole("button", { name: "Play from glowing" }).click();
    expect(onSeekPassage).toHaveBeenLastCalledWith("answer-2", 1);
    screen.getByRole("button", { name: "Play from association" }).click();
    expect(onSeekPassage).toHaveBeenLastCalledWith("answer-3", 0.25);
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

  it("strictly isolates active word to ONLY the spoken paragraph without highlighting other paragraphs or table rows", () => {
    const complexDoc = `
I've chosen the scope: the **Neolithic Revolution** and the birth of civilization.

# From Foragers to the First Cities: The Neolithic Revolution

**Abstract.** This paper examines the transition from mobile foraging to settled food production.

1. **Introduction.** The shift from hunting and gathering to agriculture.

* **Scope:** c. 10,000–3000 BCE, global.
* **Method:** Synthesis of standard consensus.

| Region | Approx. Date | Key Crops |
| --- | --- | --- |
| Fertile Crescent | ~10,000–8,000 BCE | Emmer wheat, barley |
| Andes | ~5,000–3,000 BCE | Potato, quinoa |
`;

    // Only the Abstract paragraph is being spoken
    const spokenPassage = "Abstract. This paper examines the transition from mobile foraging to settled food production.";

    const { container } = render(
      <MarkdownContent
        value={complexDoc}
        speechProgress={{
          active: true,
          passageId: "p-abstract",
          text: spokenPassage,
          seconds: 0.5,
          duration: 2.0,
          timings: [
            { value: "Abstract.", start: 0, end: 0.3 },
            { value: "This", start: 0.3, end: 0.5 },
            { value: "paper", start: 0.5, end: 0.8 },
          ],
        }}
      />,
    );

    // There should be EXACTLY ONE <mark> in the entire document!
    const marks = container.querySelectorAll("mark.speech-word-active");
    expect(marks).toHaveLength(1);
    expect(marks[0]).toHaveTextContent("paper");

    // All other elements must have ZERO marks
    expect(container.querySelector("h1 mark")).not.toBeInTheDocument();
    expect(container.querySelector("ul mark")).not.toBeInTheDocument();
    expect(container.querySelector("table mark")).not.toBeInTheDocument();

    // No raw double asterisks displayed in the document
    expect(container.textContent).not.toContain("**");
  });
});
