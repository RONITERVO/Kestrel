import { describe, expect, it } from "vitest";
import { latexToText, readableMath } from "./readableMath";
import { cleanProseForSpeech } from "./speech/text";

describe("readableMath", () => {
  it("rewrites display math from a local model as plain text", () => {
    const qwen = [
      "Justification:",
      "",
      "\\[",
      "\\left\\lfloor \\frac{1000}{3} \\right\\rfloor + \\left\\lfloor \\frac{1000}{5} \\right\\rfloor - \\left\\lfloor \\frac{1000}{15} \\right\\rfloor",
      "= 333 + 200 - 66 = 467.",
      "\\]",
    ].join("\n");
    expect(readableMath(qwen)).toBe("Justification:\n\n⌊1000/3⌋ + ⌊1000/5⌋ - ⌊1000/15⌋\n= 333 + 200 - 66 = 467.");
  });

  it("rewrites inline math and keeps the surrounding sentence", () => {
    expect(readableMath("Let \\(u = x^3\\) and \\(v = \\sin(x)\\).")).toBe("Let u = x³ and v = sin(x).");
    expect(readableMath("\\[\n\\frac{d}{dx}(x^3 \\sin x) = 3x^2 \\sin x + x^3 \\cos x\n\\]"))
      .toBe("d/dx(x³ sin x) = 3x² sin x + x³ cos x");
    expect(readableMath("Area: $$\\pi r^2$$ square units")).toBe("Area: π r² square units");
  });

  it("parenthesizes compound fractions and roots", () => {
    expect(latexToText("\\frac{a+b}{2}")).toBe("(a+b)/2");
    expect(latexToText("\\sqrt{2} + \\sqrt{x+1}")).toBe("√2 + √(x+1)");
    expect(latexToText("\\frac{\\pi}{2}")).toBe("π/2");
    expect(latexToText("\\frac{\\sqrt{3}}{2}")).toBe("√3/2");
  });

  it("keeps operators spaced and escaped characters literal", () => {
    expect(latexToText("3 \\times 4 \\le 12")).toBe("3 × 4 ≤ 12");
    expect(latexToText("\\{1, 2\\} \\cup \\{3\\}")).toBe("{1, 2} ∪ {3}");
    expect(latexToText("50\\% \\text{ of } x_1")).toBe("50% of x₁");
    expect(latexToText("a \\leftarrow b \\Rightarrow c")).toBe("a ← b ⇒ c");
    expect(latexToText("\\sum_{i=1}^{n} i")).toBe("Σᵢ₌₁ⁿ i");
    expect(latexToText("e^{x+y}")).toBe("e^(x+y)");
  });

  it("leaves code, currency, and backslashes outside math untouched", () => {
    const code = "```python\nprint(r\"\\(x\\)\")\n```\nUse `\\[a-z\\]` in regex.";
    expect(readableMath(code)).toBe(code);
    expect(readableMath("It costs $5 and $10.")).toBe("It costs $5 and $10.");
    expect(readableMath("Open C:\\Users\\me\\Documents.")).toBe("Open C:\\Users\\me\\Documents.");
  });

  it("keeps LaTeX commands out of speech", () => {
    const spoken = cleanProseForSpeech("The count is \\(\\left\\lfloor \\frac{1000}{3} \\right\\rfloor = 333\\).");
    expect(spoken).not.toMatch(/frac|lfloor|\\/);
    expect(spoken).toContain("333");
  });

  it("converts math around a code block but not inside it", () => {
    expect(readableMath("\\(\\alpha\\)\n```\n\\(\\beta\\)\n```\n\\(\\gamma\\)")).toBe("α\n```\n\\(\\beta\\)\n```\nγ");
  });
});
