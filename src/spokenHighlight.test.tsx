import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import {
  getActiveWordIndex,
  isPassageActiveForText,
  SpokenText,
  wordTimings,
  type SpeechProgressState,
} from "./spokenHighlight";

afterEach(() => {
  cleanup();
});

describe("spokenHighlight word-level alignment engine", () => {
  it("calculates proportional word timings when exact Whisper timings are pending", () => {
    const text = "Hello world from Kestrel";
    const timings = wordTimings(text, 2.0);

    expect(timings).toHaveLength(4);
    expect(timings[0].value).toBe("Hello");
    expect(timings[0].start).toBe(0);
    expect(timings[3].value).toBe("Kestrel");
    expect(timings[3].end).toBe(2.0);
  });

  it("identifies active word index based on elapsed playback seconds and exact timings", () => {
    const exactTimings = [
      { value: "The", start: 0, end: 0.3 },
      { value: "quick", start: 0.3, end: 0.8 },
      { value: "brown", start: 0.8, end: 1.2 },
      { value: "fox", start: 1.2, end: 1.6 },
    ];

    expect(getActiveWordIndex("The quick brown fox", 0.1, 1.6, exactTimings)).toBe(0);
    expect(getActiveWordIndex("The quick brown fox", 0.5, 1.6, exactTimings)).toBe(1);
    expect(getActiveWordIndex("The quick brown fox", 1.0, 1.6, exactTimings)).toBe(2);
    expect(getActiveWordIndex("The quick brown fox", 1.4, 1.6, exactTimings)).toBe(3);
  });

  it("accurately isolates the active paragraph and does NOT highlight other paragraphs", () => {
    const p1 = "Founded by Sargon of Akkad, the Akkadian Empire is the first attested empire.";
    const p2 = "The Roman Empire, beginning with Augustus's principate, unified the Mediterranean world.";
    const p3 = "Under Genghis Khan and his successors, the Mongols created the largest contiguous empire.";

    const progress: SpeechProgressState = {
      active: true,
      passageId: "chat-msg-1",
      text: p1,
      seconds: 0.2,
      duration: 1.0,
      timings: [
        { value: "Founded", start: 0, end: 0.1 },
        { value: "by", start: 0.1, end: 0.2 },
        { value: "Sargon", start: 0.2, end: 0.4 },
      ],
    };

    expect(isPassageActiveForText(p1, undefined, progress)).toBe(true);
    expect(isPassageActiveForText(p2, undefined, progress)).toBe(false);
    expect(isPassageActiveForText(p3, undefined, progress)).toBe(false);

    const { container: c1 } = render(<SpokenText text={p1} progress={progress} />);
    const { container: c2 } = render(<SpokenText text={p2} progress={progress} />);
    const { container: c3 } = render(<SpokenText text={p3} progress={progress} />);

    // P1 must have the single active mark for "Sargon"
    const mark1 = c1.querySelector("mark.speech-word-active");
    expect(mark1).toBeInTheDocument();
    expect(mark1).toHaveTextContent("Sargon");

    // P2 and P3 must have NO marks at all
    expect(c2.querySelector("mark")).not.toBeInTheDocument();
    expect(c3.querySelector("mark")).not.toBeInTheDocument();
  });

  it("isolates passages with explicit passageId in research reports", () => {
    const progress: SpeechProgressState = {
      active: true,
      passageId: "section-2-paragraph-1",
      text: "Deep dive text for section 2.",
      seconds: 0.5,
      duration: 1.0,
      timings: [],
    };

    expect(isPassageActiveForText("Section 1 text", "section-1-paragraph-1", progress)).toBe(false);
    expect(isPassageActiveForText("Section 2 text", "section-2-paragraph-1", progress)).toBe(true);
    expect(isPassageActiveForText("Section 3 text", "section-3-paragraph-1", progress)).toBe(false);
  });
});
