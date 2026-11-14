import { describe, expect, it } from "vitest";
import { SpokenClauses } from "./conversationAudio";

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
