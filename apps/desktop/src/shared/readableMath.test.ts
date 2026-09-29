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
    expect(spoken).toBe("The count is the floor of 1000 over 3 equals 333.");
  });

  it("speaks what the math means instead of its symbols", () => {
    expect(latexToText("x^2 + y^3 = z^{n+1}", "speech")).toBe("x squared plus y cubed equals z to the power of n plus 1");
    expect(latexToText("\\frac{a}{b}", "speech")).toBe("a over b");
    expect(latexToText("\\frac{a+b}{2}", "speech")).toBe("(a plus b) over 2");
    expect(latexToText("\\frac{\\pi}{2}", "speech")).toBe("pi over 2");
    expect(latexToText("\\sqrt{x+1}", "speech")).toBe("the square root of (x plus 1)");
    expect(latexToText("x_1 \\le 3 \\times 4", "speech")).toBe("x sub 1 less than or equal to 3 times 4");
    expect(latexToText("\\sum_{i=1}^{n} i", "speech")).toBe("the sum from i equals 1 to n of i");
    expect(latexToText("\\int_0^1 x\\,dx", "speech")).toBe("the integral from 0 to 1 of x dx");
    expect(latexToText("\\lim_{x \\to 0} f(x)", "speech")).toBe("the limit as x approaches 0 of f(x)");
    expect(latexToText("\\alpha + \\Omega \\ne \\varepsilon", "speech")).toBe("alpha plus omega not equal to epsilon");
    expect(latexToText("30^\\circ", "speech")).toBe("30 degrees");
    expect(latexToText("30^\\circ")).toBe("30°");
  });

  it("keeps powers and fractions through speech cleanup", () => {
    expect(cleanProseForSpeech("Here \\(x^2\\) grows.")).toBe("Here x squared grows.");
    expect(cleanProseForSpeech("The ratio is \\(\\frac{a}{b}\\).")).toBe("The ratio is a over b.");
    expect(cleanProseForSpeech("Area: $$\\pi r^2$$")).toBe("Area: pi r squared.");
  });

  it("speaks math written directly in Unicode or with a caret", () => {
    expect(cleanProseForSpeech("So x² + y² = z².")).toBe("So x squared plus y squared equals z squared.");
    expect(cleanProseForSpeech("About 10⁶ cells at 37°C.")).toBe("About 10 to the power of 6 cells at 37 degrees Celsius.");
    expect(cleanProseForSpeech("Use x^3 and 2^10.")).toBe("Use x cubed and 2 to the power of 10.");
    expect(cleanProseForSpeech("It is 3×4 ± 1.")).toBe("It is 3 times 4 plus or minus 1.");
  });

  it("converts math around a code block but not inside it", () => {
    expect(readableMath("\\(\\alpha\\)\n```\n\\(\\beta\\)\n```\n\\(\\gamma\\)")).toBe("α\n```\n\\(\\beta\\)\n```\nγ");
  });
});
