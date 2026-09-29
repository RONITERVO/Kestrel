import { describe, expect, it } from "vitest";
import { cleanProseForSpeech, splitForSpeech } from "./text";

describe("speech passage text", () => {
  it("keeps conditional probability and unit ratios meaningful for the voice", () => {
    expect(cleanProseForSpeech("Here P(H1|D) = 0.768.")).toBe("Here P of H1 given D equals 0 point 768.");
    expect(cleanProseForSpeech("\\[P(H|D) = \\frac{0.451}{0.5875}\\]")).toBe("P of H given D equals 0 point 451 over 0 point 5875.");
    expect(cleanProseForSpeech("A 10,000 kg projectile at 5 km/s.")).toBe("A 10,000 kg projectile at 5 km per s.");
    expect(cleanProseForSpeech("Scores 88/100 and ratio 0.42/0.5.")).toBe("Scores 88 out of 100 and ratio 0 point 42 over 0 point 5.");
  });

  it("reads numbered part headings as numbers", () => {
    expect(cleanProseForSpeech("## IV. The Shape of the Problem\n\nText.")).toBe("4. The Shape of the Problem. Text.");
    expect(cleanProseForSpeech("# X. The Thirty Shots")).toBe("10. The Thirty Shots.");
    expect(cleanProseForSpeech("## I Was There")).toBe("I Was There.");
  });

  it("keeps digit-dense passages shorter because every digit becomes a spoken word", () => {
    const prose = "The river bends toward the harbor where the lamps are lit each evening. ".repeat(4);
    const numbers = "Values 0.5875 0.0945 0.4510 0.7680 0.1610 0.0710 and 23,500 then 0.1575 again. ".repeat(4);
    expect(splitForSpeech(prose).length).toBe(1);
    expect(splitForSpeech(numbers).length).toBeGreaterThan(2);
  });

  it("breaks a long run of equations at its clauses and never inside a number", () => {
    const equations = [
      "She wrote:",
      "\\[E_{new} = 23,500 \\times 0.60 \\times 0.75 \\times 0.70 \\times 0.50\\]",
      "\\[E_{new} = 23,500 \\times 0.1575\\]",
      "Using the reductions:",
      "\\[E_{new} = 23,500 \\times 0.60 \\times 0.75 \\times 0.70 \\times 0.50 \\times 0.25 \\times 0.125 \\times 0.0625\\]",
    ].join("\n\n");
    const chunks = splitForSpeech(equations);
    expect(chunks.length).toBeGreaterThan(1);
    for (const chunk of chunks) {
      expect(chunk).not.toMatch(/\bpoint$/);
      expect(chunk).not.toMatch(/^point\b/);
      expect(chunk).not.toMatch(/:$/);
    }
    expect(chunks.join(" ")).toContain("0 point 0625");
  });
});
