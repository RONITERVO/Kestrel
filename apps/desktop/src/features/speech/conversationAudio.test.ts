import { describe, expect, it, vi } from "vitest";
import { decodePcm, pcmBase64, SpokenClauses, UtteranceGate } from "./conversationAudio";

describe("conversation speech boundaries", () => {
  it("starts with a completed clause and flushes its last fragment exactly once", () => {
    const clauses = new SpokenClauses();
    expect(clauses.append("Hello there.")).toEqual([]);
    expect(clauses.append(" Here is ")).toEqual(["Hello there."]);
    expect(clauses.append("the result", true)).toEqual(["Here is the result"]);
    expect(clauses.append("", true)).toEqual([]);
  });
  it("does not split a decimal or speak fragmented private/code blocks", () => {
    const clauses = new SpokenClauses();
    expect(clauses.append("The value is 3.")).toEqual([]);
    expect(clauses.append("14. <thi")).toEqual(["The value is 3.14."]);
    expect(clauses.append("nk>private reasoning</th")).toEqual([]);
    expect(clauses.append("ink>```secret code``")).toEqual([]);
    expect(clauses.append("` Finished.", true)).toEqual(["Finished."]);
  });
  it("bounds pending text instead of losing a growing reply silently", () => {
    expect(() => new SpokenClauses().append("x".repeat(32769))).toThrow(/limit/);
  });
  it("keeps long prose before hidden code in bounded spoken clauses", () => {
    const prose = "These words should all be heard ".repeat(60).trim();
    const clauses = new SpokenClauses().append(`${prose} <think>private</think>\n\`\`\`hidden code\`\`\``, true);
    expect(clauses.every((clause) => clause.length <= 180)).toBe(true);
    expect(clauses.join(" ")).toBe(prose);
  });
});

describe("continuous microphone segmentation", () => {
  const silence = () => new Float32Array(320);
  const speech = () => Float32Array.from({ length: 320 }, (_, i) => Math.sin(i * 0.11) * 0.1);
  it("ignores idle silence and short noise, then submits one final utterance and rearms", () => {
    const start = vi.fn(); const utterance = vi.fn();
    const gate = new UtteranceGate(start, utterance);
    for (let i = 0; i < 100; i++) gate.push(silence());
    gate.push(speech());
    for (let i = 0; i < 35; i++) gate.push(silence());
    expect(utterance).not.toHaveBeenCalled();
    for (let turn = 0; turn < 2; turn++) {
      for (let i = 0; i < 20; i++) gate.push(speech());
      for (let i = 0; i < 35; i++) gate.push(silence());
    }
    expect(start).toHaveBeenCalledTimes(2);
    expect(utterance).toHaveBeenCalledTimes(2);
    expect(utterance.mock.calls.every(([pcm]) => pcm.length <= 480000)).toBe(true);
  });
  it("retains a mid-sentence pause and bounds an uninterrupted utterance", () => {
    const utterance = vi.fn(); const gate = new UtteranceGate(() => {}, utterance);
    for (let i = 0; i < 20; i++) gate.push(speech());
    for (let i = 0; i < 15; i++) gate.push(silence());
    expect(utterance).not.toHaveBeenCalled();
    for (let i = 0; i < 1500; i++) gate.push(speech());
    expect(utterance).toHaveBeenCalledTimes(1);
    expect(utterance.mock.calls[0][0].length).toBe(480000);
  });
  it("encodes bounded little-endian PCM accurately", () => {
    const result = decodePcm(pcmBase64(new Float32Array([-1, 0, 0.5, 1])));
    expect(result[0]).toBeCloseTo(-1, 3); expect(result[2]).toBeCloseTo(0.5, 3);
    expect(() => decodePcm(btoa("x"))).toThrow();
  });
});
