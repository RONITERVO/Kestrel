import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import {
  getActiveWordIndex,
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

  it("renders SpokenText with active mark and past/pending word styles", () => {
    const text = "First second third fourth";
    const progress: SpeechProgressState = {
      active: true,
      passageId: "msg-1",
      text,
      seconds: 0.6,
      duration: 1.2,
      timings: [
        { value: "First", start: 0, end: 0.3 },
        { value: "second", start: 0.3, end: 0.6 },
        { value: "third", start: 0.6, end: 0.9 },
        { value: "fourth", start: 0.9, end: 1.2 },
      ],
    };

    const { container } = render(
      <SpokenText text={text} passageId="msg-1" progress={progress} />,
    );

    const mark = container.querySelector("mark.speech-word-active");
    expect(mark).toBeInTheDocument();
    expect(mark).toHaveTextContent("third");

    const spokenWords = container.querySelectorAll(".speech-word-spoken");
    expect(spokenWords.length).toBeGreaterThanOrEqual(2);
    expect(spokenWords[0]).toHaveTextContent("First");
    expect(spokenWords[1]).toHaveTextContent("second");

    const pendingWords = container.querySelectorAll(".speech-word-pending");
    expect(pendingWords.length).toBeGreaterThanOrEqual(1);
    expect(pendingWords[0]).toHaveTextContent("fourth");
  });

  it("renders plain text when speech progress is inactive or for a different passage", () => {
    const { container } = render(
      <SpokenText
        text="Normal un-spoken text"
        passageId="msg-1"
        progress={{
          active: true,
          passageId: "other-msg",
          text: "Other text",
          seconds: 0.5,
          duration: 1.0,
          timings: [],
        }}
      />,
    );

    expect(container.querySelector("mark")).not.toBeInTheDocument();
    expect(container).toHaveTextContent("Normal un-spoken text");
  });
});
