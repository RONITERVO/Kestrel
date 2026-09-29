// Local models often answer with LaTeX math (\( … \), \[ … \], $$ … $$) that Kestrel does not
// typeset. This rewrites those spans as plain Unicode text so chat reads cleanly and speech does
// not spell out commands. Code, inline code, single-$ currency, and backslashes outside math
// delimiters are left untouched.

const SYMBOLS: Record<string, string> = {
  lfloor: "⌊", rfloor: "⌋", lceil: "⌈", rceil: "⌉", langle: "⟨", rangle: "⟩",
  times: "×", cdot: "·", div: "÷", pm: "±", mp: "∓", ast: "∗", star: "⋆",
  le: "≤", leq: "≤", ge: "≥", geq: "≥", ne: "≠", neq: "≠", approx: "≈", equiv: "≡", sim: "∼", simeq: "≃", propto: "∝",
  ll: "≪", gg: "≫", infty: "∞", to: "→", rightarrow: "→", leftarrow: "←", Rightarrow: "⇒", Leftarrow: "⇐",
  leftrightarrow: "↔", Leftrightarrow: "⇔", iff: "⇔", implies: "⇒", mapsto: "↦",
  in: "∈", notin: "∉", ni: "∋", subset: "⊂", subseteq: "⊆", supset: "⊃", supseteq: "⊇", cup: "∪", cap: "∩",
  emptyset: "∅", varnothing: "∅", forall: "∀", exists: "∃", neg: "¬", lnot: "¬", land: "∧", wedge: "∧",
  lor: "∨", vee: "∨", oplus: "⊕", otimes: "⊗", partial: "∂", nabla: "∇", sum: "Σ", prod: "Π", int: "∫", oint: "∮",
  ldots: "…", cdots: "⋯", dots: "…", vdots: "⋮", degree: "°", circ: "∘", angle: "∠", perp: "⊥", parallel: "∥",
  bmod: "mod", mid: "|", vert: "|", lvert: "|", rvert: "|", Vert: "‖", prime: "′", therefore: "∴", because: "∵",
  alpha: "α", beta: "β", gamma: "γ", delta: "δ", epsilon: "ε", varepsilon: "ε", zeta: "ζ", eta: "η", theta: "θ",
  vartheta: "ϑ", iota: "ι", kappa: "κ", lambda: "λ", mu: "μ", nu: "ν", xi: "ξ", pi: "π", rho: "ρ", sigma: "σ",
  tau: "τ", upsilon: "υ", phi: "φ", varphi: "φ", chi: "χ", psi: "ψ", omega: "ω",
  Gamma: "Γ", Delta: "Δ", Theta: "Θ", Lambda: "Λ", Xi: "Ξ", Pi: "Π", Sigma: "Σ", Upsilon: "Υ", Phi: "Φ", Psi: "Ψ", Omega: "Ω",
};

// Commands whose only argument is text to keep.
const WRAPPERS = new Set(["text", "textrm", "textbf", "textit", "mathrm", "mathbf", "mathit", "mathsf", "mathtt", "mathcal",
  "mathbb", "operatorname", "boldsymbol", "bm", "hat", "bar", "vec", "tilde", "overline", "underline", "displaystyle", "mbox"]);

const SUPERSCRIPT: Record<string, string> = {
  "0": "⁰", "1": "¹", "2": "²", "3": "³", "4": "⁴", "5": "⁵", "6": "⁶", "7": "⁷", "8": "⁸", "9": "⁹",
  "+": "⁺", "-": "⁻", "=": "⁼", "(": "⁽", ")": "⁾", n: "ⁿ", i: "ⁱ",
};
const SUBSCRIPT: Record<string, string> = {
  "0": "₀", "1": "₁", "2": "₂", "3": "₃", "4": "₄", "5": "₅", "6": "₆", "7": "₇", "8": "₈", "9": "₉",
  "+": "₊", "-": "₋", "=": "₌", "(": "₍", ")": "₎", a: "ₐ", e: "ₑ", i: "ᵢ", j: "ⱼ", k: "ₖ", n: "ₙ", o: "ₒ", x: "ₓ",
};

/** The Markdown with every LaTeX math span outside code rewritten as readable text. */
export function readableMath(markdown: string): string {
  if (!markdown || !/\\[([]|\$\$/.test(markdown)) return markdown;
  const lines = markdown.split(/(\r?\n)/);
  const out: string[] = [];
  let prose = "";
  let fence: string | null = null;
  for (let index = 0; index < lines.length; index += 2) {
    const line = lines[index];
    const newline = lines[index + 1] ?? "";
    const marker = line.trim().match(/^(```|~~~)/)?.[1];
    if (fence) {
      out.push(line + newline);
      if (marker === fence) fence = null;
    } else if (marker) {
      out.push(convertProse(prose));
      prose = "";
      fence = marker;
      out.push(line + newline);
    } else {
      prose += line + newline;
    }
  }
  out.push(convertProse(prose));
  return out.join("");
}

function convertProse(text: string): string {
  if (!text) return text;
  // Inline code keeps its exact characters.
  return text.split(/(`[^`\n]*`)/).map((part, index) => index % 2 ? part : convertMath(part)).join("");
}

function convertMath(text: string): string {
  return text
    .replace(/\$\$([\s\S]+?)\$\$/g, (_, tex: string) => latexToText(tex))
    .replace(/\\\[([\s\S]+?)\\\]/g, (_, tex: string) => latexToText(tex))
    .replace(/\\\(([\s\S]+?)\\\)/g, (_, tex: string) => latexToText(tex));
}

// Stand-ins for escaped braces while grouping braces are removed.
const OPEN_BRACE = "\u0001";
const CLOSE_BRACE = "\u0002";

/** One LaTeX math expression as plain Unicode text. */
export function latexToText(tex: string): string {
  let text = tex
    .replace(/\\begin\{[a-z*]+\}|\\end\{[a-z*]+\}/g, "")
    .replace(/\\\\/g, "\n")
    .replace(/(^|[^\\])&/g, "$1")
    .replace(/\\\{/g, OPEN_BRACE)
    .replace(/\\\}/g, CLOSE_BRACE)
    .replace(/\\(?:left|right)\s*\./g, "")
    .replace(/\\(?:left|right|big|Big|bigg|Bigg)[lr]?(?![A-Za-z])\s*/g, "");
  text = replaceArgumentCommands(text);
  text = text
    .replace(/\\([%$#&_|])/g, "$1")
    .replace(/\\(quad|qquad|[,;: ])/g, " ")
    .replace(/\\!/g, "")
    .replace(/\\([A-Za-z]+)/g, (_, name: string) => SYMBOLS[name] ?? name)
    .replace(/\^(\{([^{}]*)\}|\S)/g, (_, raw: string, braced?: string) => script(braced ?? raw, SUPERSCRIPT, "^"))
    .replace(/_(\{([^{}]*)\}|\S)/g, (_, raw: string, braced?: string) => script(braced ?? raw, SUBSCRIPT, "_"))
    .replace(/[{}]/g, "")
    .replace(new RegExp(OPEN_BRACE, "g"), "{")
    .replace(new RegExp(CLOSE_BRACE, "g"), "}")
    .replace(/[ \t]+/g, " ")
    .replace(/([⌊⌈⟨]) /g, "$1")
    .replace(/ ([⌋⌉⟩])/g, "$1")
    .replace(/ *\n */g, "\n");
  return text.trim();
}

function script(value: string, table: Record<string, string>, marker: string): string {
  const plain = value.trim();
  if (plain && [...plain].every((character) => character in table)) {
    return [...plain].map((character) => table[character]).join("");
  }
  return plain.length === 1 ? `${marker}${plain}` : `${marker}(${plain})`;
}

// \frac{a}{b}, \sqrt{x}, \pmod{n}, and text wrappers, innermost first via brace matching.
function replaceArgumentCommands(text: string): string {
  let result = text;
  for (let guard = 0; guard < 200; guard++) {
    const match = /\\(d?t?frac|sqrt|pmod|[A-Za-z]+)\s*\{/g;
    let changed = false;
    let found: RegExpExecArray | null;
    while ((found = match.exec(result))) {
      const name = found[1];
      if (!["frac", "dfrac", "tfrac", "sqrt", "pmod"].includes(name) && !WRAPPERS.has(name)) continue;
      const first = group(result, found.index + found[0].length - 1);
      if (!first) continue;
      let replacement: string;
      let end = first.end;
      if (name.endsWith("frac")) {
        const second = group(result, first.end);
        if (!second) continue;
        replacement = `${atom(first.body)}/${atom(second.body)}`;
        end = second.end;
      } else if (name === "sqrt") {
        replacement = `√${atom(first.body)}`;
      } else if (name === "pmod") {
        replacement = ` (mod ${first.body.trim()})`;
      } else {
        replacement = first.body;
      }
      result = result.slice(0, found.index) + replacement + result.slice(end);
      changed = true;
      break;
    }
    if (!changed) break;
  }
  return result;
}

function group(text: string, open: number): { body: string; end: number } | null {
  let position = open;
  while (text[position] === " ") position++;
  if (text[position] !== "{") return null;
  let depth = 0;
  for (let index = position; index < text.length; index++) {
    if (text[index] === "\\") { index++; continue; }
    if (text[index] === "{") depth++;
    if (text[index] === "}" && --depth === 0) return { body: text.slice(position + 1, index), end: index + 1 };
  }
  return null;
}

// Parenthesize a fraction or root operand unless it is a single number, name, or command.
function atom(value: string): string {
  const inner = replaceArgumentCommands(value).trim();
  return /^(?:[\p{L}\p{N}.√]+|\\[A-Za-z]+)$/u.test(inner) ? inner : `(${inner})`;
}
