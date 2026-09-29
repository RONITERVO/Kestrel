// Local models often answer with LaTeX math (\( … \), \[ … \], $$ … $$) that Kestrel does not
// typeset. This rewrites those spans as plain Unicode text so chat reads cleanly, or as words so
// speech says what the math means ("x squared", "a over b") instead of spelling commands or
// symbols. Code, inline code, single-$ currency, and backslashes outside math delimiters are left
// untouched.

/** How converted math is written: Unicode for reading, or words for speech. */
export type MathStyle = "display" | "speech";

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

// How speech says a command. Greek letters are said by name; anything else falls back to its
// display symbol. An empty entry is silent (closing brackets, bars).
const SPOKEN: Record<string, string> = {
  lfloor: "the floor of", rfloor: "", lceil: "the ceiling of", rceil: "", langle: "", rangle: "",
  times: "times", cdot: "times", div: "divided by", pm: "plus or minus", mp: "minus or plus", ast: "times",
  le: "less than or equal to", leq: "less than or equal to", ge: "greater than or equal to", geq: "greater than or equal to",
  ne: "not equal to", neq: "not equal to", approx: "approximately", equiv: "is equivalent to", sim: "is similar to",
  simeq: "is similar to", propto: "is proportional to", ll: "much less than", gg: "much greater than", infty: "infinity",
  to: "to", rightarrow: "to", leftarrow: "from", Rightarrow: "implies", Leftarrow: "is implied by",
  leftrightarrow: "if and only if", Leftrightarrow: "if and only if", iff: "if and only if", implies: "implies",
  mapsto: "maps to", in: "in", notin: "not in", ni: "contains", subset: "a subset of", subseteq: "a subset of",
  supset: "a superset of", supseteq: "a superset of", cup: "union", cap: "intersection", emptyset: "the empty set",
  varnothing: "the empty set", forall: "for all", exists: "there exists", neg: "not", lnot: "not", land: "and",
  wedge: "and", lor: "or", vee: "or", partial: "partial", nabla: "del", sum: "the sum of", prod: "the product of",
  int: "the integral of", oint: "the contour integral of", ldots: "and so on", cdots: "and so on", dots: "and so on",
  vdots: "and so on", degree: "degrees", circ: "composed with", angle: "angle", perp: "perpendicular to",
  parallel: "parallel to", bmod: "mod", mid: "", vert: "", lvert: "", rvert: "", Vert: "", prime: "prime",
  therefore: "therefore", because: "because", sin: "sine", cos: "cosine", tan: "tangent", ln: "the natural log of",
  log: "log", exp: "exp", lim: "the limit", max: "the maximum of", min: "the minimum of",
};

const LIMIT_WORDS: Record<string, string> = {
  sum: "the sum", prod: "the product", int: "the integral", oint: "the contour integral",
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
export function readableMath(markdown: string, style: MathStyle = "display"): string {
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
      out.push(convertProse(prose, style));
      prose = "";
      fence = marker;
      out.push(line + newline);
    } else {
      prose += line + newline;
    }
  }
  out.push(convertProse(prose, style));
  return out.join("");
}

function convertProse(text: string, style: MathStyle): string {
  if (!text) return text;
  // Inline code keeps its exact characters.
  return text.split(/(`[^`\n]*`)/).map((part, index) => index % 2 ? part : convertMath(part, style)).join("");
}

function convertMath(text: string, style: MathStyle): string {
  // Spoken, a displayed equation is its own sentence: the voice pauses after it, and long runs of
  // equations break into passages between equations rather than inside one.
  const display = (tex: string) => {
    const converted = latexToText(tex, style);
    return style === "speech" && converted && !/[.!?]$/.test(converted) ? `${converted}.` : converted;
  };
  return text
    .replace(/\$\$([\s\S]+?)\$\$/g, (_, tex: string) => display(tex))
    .replace(/\\\[([\s\S]+?)\\\]/g, (_, tex: string) => display(tex))
    .replace(/\\\(([\s\S]+?)\\\)/g, (_, tex: string) => latexToText(tex, style));
}

// Stand-ins for escaped braces while grouping braces are removed.
const OPEN_BRACE = "\u0001";
const CLOSE_BRACE = "\u0002";

/** One LaTeX math expression as plain Unicode text, or as words for speech. */
export function latexToText(tex: string, style: MathStyle = "display"): string {
  const speech = style === "speech";
  let text = tex
    .replace(/\\begin\{[a-z*]+\}|\\end\{[a-z*]+\}/g, "")
    .replace(/\\\\/g, "\n")
    .replace(/(^|[^\\])&/g, "$1")
    .replace(/\\\{/g, OPEN_BRACE)
    .replace(/\\\}/g, CLOSE_BRACE)
    .replace(/\\(?:left|right)\s*\./g, "")
    .replace(/\\(?:left|right|big|Big|bigg|Bigg)[lr]?(?![A-Za-z])\s*/g, "")
    .replace(/\^\s*\{?\s*\\circ\s*\}?/g, speech ? " degrees " : "°");
  text = replaceArgumentCommands(text, style);
  if (speech) text = speakLimits(text);
  text = text
    .replace(/\\([%$#&_|])/g, "$1")
    .replace(/\\(quad|qquad|[,;: ])/g, " ")
    .replace(/\\!/g, "")
    .replace(/\\([A-Za-z]+)/g, (_, name: string) => speech ? ` ${spokenSymbol(name)} ` : SYMBOLS[name] ?? name)
    .replace(/\^(\{([^{}]*)\}|\S)/g, (_, raw: string, braced?: string) => speech
      ? ` ${spokenPower(braced ?? raw)} `
      : script(braced ?? raw, SUPERSCRIPT, "^"))
    .replace(/_(\{([^{}]*)\}|\S)/g, (_, raw: string, braced?: string) => speech
      ? ` sub ${(braced ?? raw).trim()} `
      : script(braced ?? raw, SUBSCRIPT, "_"))
    .replace(/[{}]/g, "")
    .replace(new RegExp(OPEN_BRACE, "g"), "{")
    .replace(new RegExp(CLOSE_BRACE, "g"), "}");
  if (speech) text = speakOperators(text);
  text = text
    .replace(/[ \t]+/g, " ")
    .replace(/([⌊⌈⟨]) /g, "$1")
    .replace(/ ([⌋⌉⟩])/g, "$1")
    .replace(/ *\n */g, "\n");
  return text.trim();
}

/** A power as it is said: "squared", "cubed", or "to the power of n". */
export function spokenPower(power: string): string {
  const plain = power.trim();
  return plain === "2" ? "squared" : plain === "3" ? "cubed" : `to the power of ${plain}`;
}

function spokenSymbol(name: string): string {
  if (name in SPOKEN) return SPOKEN[name];
  const symbol = SYMBOLS[name];
  if (symbol && /^\p{Script=Greek}$/u.test(symbol)) return name.replace(/^var/, "").toLowerCase();
  return symbol ?? name;
}

// Sums, products, integrals, and limits read their bounds before their body.
function speakLimits(text: string): string {
  const bound = String.raw`(?:\{([^{}]*)\}|(\\[A-Za-z]+|[^\s{}\\]))`;
  return text
    .replace(new RegExp(String.raw`\\(sum|prod|int|oint)\s*_${bound}\s*\^${bound}`, "g"),
      (_, name: string, lower?: string, lowerChar?: string, upper?: string, upperChar?: string) =>
        ` ${LIMIT_WORDS[name]} from ${lower ?? lowerChar} to ${upper ?? upperChar} of `)
    .replace(new RegExp(String.raw`\\(sum|prod)\s*_${bound}`, "g"),
      (_, name: string, lower?: string, lowerChar?: string) => ` ${LIMIT_WORDS[name]} over ${lower ?? lowerChar} of `)
    .replace(new RegExp(String.raw`\\lim\s*_${bound}`, "g"), (_, under?: string, underChar?: string) =>
      ` the limit as ${(under ?? underChar ?? "").replace(/\\(?:to|rightarrow)(?![A-Za-z])/g, " approaches ")} of `);
}

/** Conditional probability as it is said: "P(A|B)" becomes "P of A given B". */
export function speakConditionals(text: string): string {
  return text.replace(/\b([PE])\s*\(\s*([^|()]+?)\s*\|\s*([^()]+?)\s*\)/g, "$1 of $2 given $3");
}

// Inside math every operator is arithmetic, so it can be said without guessing.
function speakOperators(text: string): string {
  return speakConditionals(text)
    .replace(/([\p{L}\p{N})])'/gu, "$1 prime ")
    .replace(/([\p{L}\p{N})])!/gu, "$1 factorial ")
    .replace(/\s*=\s*/g, " equals ")
    .replace(/\s*\+\s*/g, " plus ")
    .replace(/\s*[-−]\s*/g, " minus ")
    .replace(/\s*\*\s*/g, " times ")
    .replace(/\s*\/\s*/g, " over ")
    .replace(/\s*<\s*/g, " less than ")
    .replace(/\s*>\s*/g, " greater than ")
    .replace(/\|/g, " ");
}

function script(value: string, table: Record<string, string>, marker: string): string {
  const plain = value.trim();
  if (plain && [...plain].every((character) => character in table)) {
    return [...plain].map((character) => table[character]).join("");
  }
  return plain.length === 1 ? `${marker}${plain}` : `${marker}(${plain})`;
}

// \frac{a}{b}, \sqrt{x}, \pmod{n}, and text wrappers, innermost first via brace matching.
function replaceArgumentCommands(text: string, style: MathStyle): string {
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
        replacement = style === "speech"
          ? ` ${atom(first.body, style)} over ${atom(second.body, style)} `
          : `${atom(first.body, style)}/${atom(second.body, style)}`;
        end = second.end;
      } else if (name === "sqrt") {
        replacement = style === "speech" ? ` the square root of ${atom(first.body, style)} ` : `√${atom(first.body, style)}`;
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
function atom(value: string, style: MathStyle): string {
  const inner = replaceArgumentCommands(value, style).trim();
  return /^(?:[\p{L}\p{N}.√]+|\\[A-Za-z]+)$/u.test(inner) ? inner : `(${inner})`;
}
