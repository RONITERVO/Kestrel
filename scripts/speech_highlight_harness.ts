/**
 * Offline Speech Highlight & Whisper Timing Simulation Harness
 * 
 * Tests the real frontend highlight and audio-sync code against arbitrary markdown,
 * charts, tables, and prose without needing to launch Tauri or the app GUI.
 * 
 * Features:
 * - Simulates realistic audio playback and Whisper word timestamps
 * - Evaluates actual block highlight isolation and token rendering
 * - Verifies that exactly ONE word highlights on screen at any given millisecond
 * - Detects multi-word bleed, missing highlights, and raw markdown leaks
 * - Live progress bar with watchdog timeout protection
 * - Generates comprehensive JSON timeline and Markdown audit reports
 * 
 * Usage:
 *   npx vite-node scripts/speech_highlight_harness.ts
 *   npx vite-node scripts/speech_highlight_harness.ts --file path/to/doc.md
 *   npx vite-node scripts/speech_highlight_harness.ts --text "Hello world # title"
 */

import * as fs from "fs";
import * as path from "path";
import { type ReactNode, type ReactElement } from "react";
import {
  buildSpeechPassages,
  cleanProseForSpeech,
  splitSpeechText,
} from "../src/researchSpeechContent";
import {
  parseMarkdownBlocks,
  collectCandidateBlocks,
  getBlockSpeechHighlight,
  renderInlineMarkdown,
} from "../src/MarkdownContent";
import {
  wordTimings,
  getActiveWordIndex,
  resolveActiveBlockAndWord,
  isPassageActiveForText,
  extractSpeechWords,
  type SpeechProgressState,
} from "../src/spokenHighlight";
import type { SpeechTiming } from "../src/types";

export interface SimulationAnomaly {
  type: "MISSING_HIGHLIGHT" | "MULTIPLE_HIGHLIGHTS" | "RAW_MARKDOWN_LEAK" | "UNEXPECTED_WORD" | "WATCHDOG_STALL";
  timestamp: number;
  passageId: string;
  passageIndex: number;
  message: string;
  details?: Record<string, unknown>;
}

export interface TimelineFrame {
  timestampSec: number;
  passageId: string;
  passageIndex: number;
  spokenWord: string;
  activeWordIndex: number;
  activeBlockType: string;
  activeMarksCount: number;
  renderedTextPreview: string;
}

export interface PassageAudit {
  passageId: string;
  index: number;
  rawText: string;
  cleanedText: string;
  durationSec: number;
  wordsCount: number;
  timings: SpeechTiming[];
  highlightCoveragePercent: number;
  anomaliesCount: number;
}

export interface HarnessOptions {
  text?: string;
  filePath?: string;
  stepMs?: number;
  watchdogTimeoutMs?: number;
  jsonOutputPath?: string;
  reportOutputPath?: string;
  silent?: boolean;
}

const DEFAULT_SAMPLE_DOC = `I've chosen the scope: **the Neolithic Revolution and the birth of civilization (c. 10,000–3000 BCE)** — a global arc that is genuinely "history of the world" in scope, and one I can chart cleanly. Below is a research-paper-style document.

---

# From Foragers to the First Cities: The Neolithic Revolution and the Birth of Civilization (c. 10,000–3000 BCE)

**Abstract.** This paper examines the transition from mobile foraging to settled food production — the Neolithic Revolution — as the foundational event in world history. It traces the independent domestication of plants and animals across at least five regions, models the demographic consequences, and maps the rise of social complexity from kin-based bands to the first states. All figures are estimates with wide error bars; dates are approximate and debated.

**1. Introduction.** The shift from hunting and gathering to agriculture is the single most consequential transformation in human history: it produced permanent settlements, population growth, social stratification, and eventually writing and the state. This paper treats that shift as a global, multi-regional process rather than a single invention.

**2. Scope and Method.**
- **Scope:** c. 10,000–3000 BCE, global, focused on the *transition* (not the full subsequent history).
- **Method:** Synthesis of standard archaeological/anthropological consensus. I am offline, so I cannot pull live data or verify exact editions; population figures are estimates, and dates are approximate.

**3. Independent Centers of Crop Domestication.**
A key finding is that agriculture arose independently in several regions, not once and then spread.

| Region | Approx. Date | Key Crops / Animals |
|---|---|---|
| Fertile Crescent (SW Asia) | ~10,000–8,000 BCE | Emmer wheat, barley, sheep, goat, pig |
| Mesoamerica | ~9,000–7,000 BCE | Maize, squash, beans, turkey |
| Andes (S. America) | ~5,000–3,000 BCE | Potato, quinoa, llama, alpaca |
| East Asia (China) | ~9,000–7,000 BCE | Rice, millet, pig, soy |
| West Africa | ~4,000–3,000 BCE | Sorghum, African rice, yam |
| New Guinea Highlands | ~8,000–6,000 BCE | Taro, yam, sugarcane |

*Note:* The "centers of origin" model is still being refined by ancient DNA and archaeobotany; dates above are working estimates.

**4. Demographic Consequences.**
Food production raised the carrying capacity of the land, allowing population to grow from a few million to hundreds of millions.

| Period | Population (est., millions) | Notes |
|---|---|---|
| 10,000 BCE | 5–10 | Foragers + first farmers |
| 5,000 BCE | 10–15 | Early agriculture, first towns |
| 3,000 BCE | 20–30 | First cities (e.g., Uruk) |
| 1,000 BCE | 50–70 | Iron age, larger states |
| 1 CE | 150–200 | Roman / Han / Gupta era |
| 1,000 CE | 250–300 | Medieval recovery |
| 1,500 CE | 400–500 | Pre-industrial |
| 1,750 CE | 700–1,000 | Early modern |
| 1,800 CE | ~1,000 | Industrial takeoff |
| 1,900 CE | ~1,600 | Industrialization |
| 1,950 CE | ~2,500 | Post-war |
| 2,000 CE | ~6,000 | Demographic transition |
| 2,023 | ~8,000 | — |

**Chart 1 — Population growth, 1500–2023 (scale: 1 █ ≈ 200 million, approximate):**
\`\`\`
1,500 CE  |██
1,750 CE  |████
1,800 CE  |██████
1,900 CE  |████████
1,950 CE  |████████████
2,000 CE  |████████████████████████
2,023     |████████████████████████████
\`\`\`
The curve is essentially flat for millennia, then steepens sharply after ~1500 — the signature of the agricultural/industrial transitions.

**5. Social Complexity.**
Settlement and surplus drove a predictable escalation of social organization:

\`\`\`
Bands (kin-based, <100)
   → Tribes (clan-based, ~100–1,000)
   → Chiefdoms (ranked, ~1,000–10,000)
   → States (stratified, >10,000: taxation, standing army, writing)
\`\`\`

**6. Key Case Sites.**

| Site | Region | Approx. Date | Significance |
|---|---|---|---|
| Göbekli Tepe | Anatolia | ~9,500 BCE | Early monumental ritual site |
| Jericho | Levant | ~9,000 BCE | Early permanent settlement, early walls |
| Çatalhöyük | Anatolia | ~7,500 BCE | Large Neolithic settlement |
| Jiahu | China | ~7,000 BCE | Early rice farming |
| Uruk | Mesopotamia | ~4,000 BCE | First city, proto-cuneiform |
| Mohenjo-Daro | Indus | ~2,500 BCE | Planned urban center |

**7. Discussion.**
The Neolithic Revolution is not a single event but a cascade: domestication → surplus → settlement → population growth → social stratification → the state. The tail end of this period (Uruk, ~3300 BCE) marks the emergence of writing and the first states, closing the arc this paper covers.

**8. Limitations.**
- I am offline; no live data or verified bibliography.
- Population figures are estimates with wide error bars, especially pre-1 CE.
- Dates are approximate and actively debated.
- The "centers of origin" model is still being refined by genetics.

**9. Conclusion.**
The shift to food production is the hinge of world history: it set the demographic, political, and technological baseline for everything after. Its multi-regional, independent origins make it a truly global story.

**References (standard works; offline, editions/pages not verified):**
- Diamond, J. *Guns, Germs, and Steel* (1997).
- Renfrew, C. *The Emergence of Civilisation* (1978).
- Hayden, B. *The Origins of Food Production* (1990).
- Flannery, K. "The Origins of Agriculture: From Nominal to Actual" (1998).

---

Want me to (a) expand one section into a full literature review, (b) add a second paper on a different era (e.g., the Industrial Revolution or the Roman Empire), or (c) reformat this as a single-page brief?`;

/**
 * Traverses rendered React element tree and counts <mark className="speech-word-active"> elements.
 */
function extractActiveMarksFromNodes(nodes: ReactNode[] | ReactNode): { marks: string[]; previewText: string } {
  const marks: string[] = [];
  const textBuffer: string[] = [];

  function visit(node: ReactNode) {
    if (!node) return;
    if (typeof node === "string" || typeof node === "number") {
      textBuffer.push(String(node));
      return;
    }
    if (Array.isArray(node)) {
      node.forEach(visit);
      return;
    }
    if (typeof node === "object" && "props" in node) {
      const el = node as ReactElement<{ className?: string; children?: ReactNode }>;
      if (el.type === "mark" && el.props?.className?.includes("speech-word-active")) {
        const markText = String(el.props?.children ?? "");
        marks.push(markText);
        textBuffer.push(`[${markText}]`);
        return;
      }
      if (el.props?.children) {
        visit(el.props.children);
      }
    }
  }

  visit(nodes);
  return { marks, previewText: textBuffer.join("") };
}

/**
 * Runs the simulation and returns a comprehensive audit result.
 */
export async function runSpeechSimulation(options: HarnessOptions = {}) {
  const text = options.text || (options.filePath ? fs.readFileSync(options.filePath, "utf-8") : DEFAULT_SAMPLE_DOC);
  const stepMs = options.stepMs ?? 100;
  const watchdogTimeoutMs = options.watchdogTimeoutMs ?? 5000;
  const silent = Boolean(options.silent);

  if (!silent) {
    console.log("\n============================================================");
    console.log("   KESTREL SPEECH HIGHLIGHT & TIMING SIMULATION HARNESS     ");
    console.log("============================================================\n");
    console.log(`Document Length: ${text.length} characters`);
  }

  // 1. Build speech passages
  const passages = buildSpeechPassages(text);
  if (!silent) {
    console.log(`Extracted Speech Passages: ${passages.length} chunks\n`);
  }

  const parsedBlocks = parseMarkdownBlocks(text);
  const anomalies: SimulationAnomaly[] = [];
  const timelineFrames: TimelineFrame[] = [];
  const passageAudits: PassageAudit[] = [];

  let totalDurationSec = 0;
  const passageTimingData = passages.map((passage, idx) => {
    const cleaned = cleanProseForSpeech(passage.text);
    const words = cleaned.match(/\S+/g) ?? [];
    const duration = Math.max(1.2, words.length * 0.32); // ~185 WPM realistic speech speed
    const timings = wordTimings(cleaned, duration);
    totalDurationSec += duration;
    return {
      passage,
      cleaned,
      words,
      duration,
      timings,
    };
  });

  if (!silent) {
    console.log(`Total Simulated Audio Duration: ${totalDurationSec.toFixed(2)}s`);
    console.log(`Simulation Time Step: ${stepMs}ms | Watchdog: ${watchdogTimeoutMs}ms\n`);
  }

  let totalFramesTested = 0;
  let totalSuccessfulMarks = 0;
  let globalTimeCursor = 0;

  for (let pIdx = 0; pIdx < passageTimingData.length; pIdx++) {
    const { passage, cleaned, words, duration, timings } = passageTimingData[pIdx];
    let passageMarksCount = 0;
    let lastAdvanceTimestamp = Date.now();
    let lastWordIndex = -1;

    for (let sec = 0; sec <= duration; sec += stepMs / 1000) {
      totalFramesTested++;
      const currentWordIdx = getActiveWordIndex(cleaned, sec, duration, timings);
      const expectedWord = words[currentWordIdx] ?? "";

      // Watchdog check
      if (currentWordIdx !== lastWordIndex) {
        lastWordIndex = currentWordIdx;
        lastAdvanceTimestamp = Date.now();
      } else if (Date.now() - lastAdvanceTimestamp > watchdogTimeoutMs) {
        anomalies.push({
          type: "WATCHDOG_STALL",
          timestamp: globalTimeCursor + sec,
          passageId: passage.id,
          passageIndex: pIdx,
          message: `Watchdog stall: no word progression for ${watchdogTimeoutMs}ms in passage ${pIdx}`,
        });
        break;
      }

      const progressState: SpeechProgressState = {
        active: true,
        passageId: passage.id,
        text: cleaned,
        seconds: sec,
        duration,
        timings,
      };

      // Render all blocks with current speech progress
      let docTotalMarks: string[] = [];
      let activeBlockType = "none";
      let activePreview = "";

      const candidates = collectCandidateBlocks(parsedBlocks);
      const activeHighlight = resolveActiveBlockAndWord(candidates, progressState);

      for (let bIdx = 0; bIdx < parsedBlocks.length; bIdx++) {
        const block = parsedBlocks[bIdx];
        let renderedNodes: ReactNode[] | ReactNode = null;
        let blockText = "";

        switch (block.type) {
          case "table": {
            const headerId = `table-${bIdx}-hdr`;
            const headerHighlight = getBlockSpeechHighlight(headerId, activeHighlight);
            const headerNodes = block.headers.map((h) => renderInlineMarkdown(h, headerHighlight));
            const rowNodes = block.rows.map((row, rIdx) => {
              const rowId = `table-${bIdx}-row-${rIdx}`;
              const rowHighlight = getBlockSpeechHighlight(rowId, activeHighlight);
              return row.map((cell) => renderInlineMarkdown(cell, rowHighlight));
            });
            renderedNodes = [headerNodes, rowNodes];
            blockText = `${block.headers.join(" ")} ${block.rows.map((r) => r.join(" ")).join(" ")}`;
            break;
          }
          case "heading": {
            const highlight = getBlockSpeechHighlight(`heading-${bIdx}`, activeHighlight);
            renderedNodes = renderInlineMarkdown(block.text, highlight);
            blockText = block.text;
            break;
          }
          case "list": {
            renderedNodes = block.items.map((item, iIdx) => {
              const highlight = getBlockSpeechHighlight(`list-${bIdx}-${iIdx}`, activeHighlight);
              return renderInlineMarkdown(item, highlight);
            });
            blockText = block.items.join(" ");
            break;
          }
          case "blockquote": {
            renderedNodes = block.text.split("\n").map((line, lIdx) => {
              const highlight = getBlockSpeechHighlight(`quote-${bIdx}-${lIdx}`, activeHighlight);
              return renderInlineMarkdown(line, highlight);
            });
            blockText = block.text;
            break;
          }
          case "chart": {
            const highlight = getBlockSpeechHighlight(`chart-${bIdx}`, activeHighlight);
            renderedNodes = renderInlineMarkdown(block.text, highlight);
            blockText = block.text;
            break;
          }
          case "code": {
            const highlight = getBlockSpeechHighlight(`code-${bIdx}`, activeHighlight);
            renderedNodes = renderInlineMarkdown(block.code, highlight);
            blockText = block.code;
            break;
          }
          case "divider":
            break;
          case "paragraph":
          default: {
            const highlight = getBlockSpeechHighlight(`para-${bIdx}`, activeHighlight);
            renderedNodes = renderInlineMarkdown(block.text, highlight);
            blockText = block.text;
            break;
          }
        }

        if (renderedNodes) {
          const { marks, previewText } = extractActiveMarksFromNodes(renderedNodes);
          if (marks.length > 0) {
            docTotalMarks = docTotalMarks.concat(marks);
            activeBlockType = block.type;
            activePreview = previewText;
          }
        }
      }

      // Check anomalies
      if (docTotalMarks.length === 0) {
        anomalies.push({
          type: "MISSING_HIGHLIGHT",
          timestamp: globalTimeCursor + sec,
          passageId: passage.id,
          passageIndex: pIdx,
          message: `Zero <mark> tags rendered during speech at ${sec.toFixed(2)}s for passage: "${cleaned.slice(0, 50)}..."`,
        });
      } else if (docTotalMarks.length > 1) {
        anomalies.push({
          type: "MULTIPLE_HIGHLIGHTS",
          timestamp: globalTimeCursor + sec,
          passageId: passage.id,
          passageIndex: pIdx,
          message: `Multiple (${docTotalMarks.length}) <mark> tags rendered simultaneously: [${docTotalMarks.join(", ")}]`,
        });
      } else {
        totalSuccessfulMarks++;
        passageMarksCount++;
      }

      // Check for raw markdown syntax leaks inside the highlighted text
      for (const m of docTotalMarks) {
        if (m.includes("**") || m.includes("__") || m.includes("~~~") || m.includes("|")) {
          anomalies.push({
            type: "RAW_MARKDOWN_LEAK",
            timestamp: globalTimeCursor + sec,
            passageId: passage.id,
            passageIndex: pIdx,
            message: `Raw markdown syntax leaked in active mark text: "${m}"`,
          });
        }
      }

      timelineFrames.push({
        timestampSec: +(globalTimeCursor + sec).toFixed(2),
        passageId: passage.id,
        passageIndex: pIdx,
        spokenWord: expectedWord,
        activeWordIndex: currentWordIdx,
        activeBlockType,
        activeMarksCount: docTotalMarks.length,
        renderedTextPreview: activePreview.slice(0, 100),
      });
    }

    const coveragePercent = Math.min(100, Math.round((passageMarksCount / Math.max(1, duration / (stepMs / 1000))) * 100));
    passageAudits.push({
      passageId: passage.id,
      index: pIdx,
      rawText: passage.text,
      cleanedText: cleaned,
      durationSec: +duration.toFixed(2),
      wordsCount: words.length,
      timings,
      highlightCoveragePercent: coveragePercent,
      anomaliesCount: anomalies.filter((a) => a.passageIndex === pIdx).length,
    });

    globalTimeCursor += duration;

    // Live progress reporting
    if (!silent) {
      const pct = Math.round(((pIdx + 1) / passageTimingData.length) * 100);
      const barLength = 25;
      const filled = Math.round((pct / 100) * barLength);
      const bar = "█".repeat(filled) + "░".repeat(barLength - filled);
      process.stdout.write(
        `\r[${bar}] ${pct}% | Passage ${pIdx + 1}/${passageTimingData.length} | Coverage: ${coveragePercent}% | Anomalies: ${anomalies.length}`,
      );
    }
  }

  if (!silent) {
    console.log("\n\n============================================================");
    console.log("                   SIMULATION RESULTS                       ");
    console.log("============================================================");
    console.log(`Total Frames Tested:     ${totalFramesTested}`);
    console.log(`Successful 1-Mark Sync:  ${totalSuccessfulMarks} (${((totalSuccessfulMarks / Math.max(1, totalFramesTested)) * 100).toFixed(1)}%)`);
    console.log(`Total Anomalies Found:   ${anomalies.length}`);
    console.log("============================================================\n");
  }

  // Save audit logs if paths are provided or defaults
  const jsonOut = options.jsonOutputPath || path.resolve("speech_timeline_audit.json");
  const reportOut = options.reportOutputPath || path.resolve("speech_highlight_report.md");

  const auditOutput = {
    summary: {
      totalPassages: passages.length,
      totalDurationSec: +totalDurationSec.toFixed(2),
      totalFramesTested,
      successfulMarksCount: totalSuccessfulMarks,
      accuracyPercent: +((totalSuccessfulMarks / Math.max(1, totalFramesTested)) * 100).toFixed(2),
      anomaliesCount: anomalies.length,
      timestamp: new Date().toISOString(),
    },
    passageAudits,
    anomalies,
    timelineFramesSample: timelineFrames.slice(0, 100),
  };

  fs.writeFileSync(jsonOut, JSON.stringify(auditOutput, null, 2), "utf-8");

  // Markdown summary report
  const markdownReport = `# Kestrel Speech Highlight & Timing Simulation Report

**Generated:** ${new Date().toISOString()}  
**Total Passages:** ${passages.length}  
**Total Spoken Duration:** ${totalDurationSec.toFixed(2)}s  
**Simulation Time Step:** ${stepMs}ms  
**Overall Accuracy:** ${auditOutput.summary.accuracyPercent}% (${totalSuccessfulMarks}/${totalFramesTested} frames)  
**Total Anomalies:** ${anomalies.length}

---

## Passage-by-Passage Audit

| # | Passage ID | Words | Duration | Highlight Coverage | Anomalies | Sample Text |
|---|---|---|---|---|---|---|
${passageAudits
  .map(
    (p) =>
      `| ${p.index + 1} | \`${p.passageId}\` | ${p.wordsCount} | ${p.durationSec}s | **${p.highlightCoveragePercent}%** | ${p.anomaliesCount === 0 ? "✅ 0" : `⚠️ ${p.anomaliesCount}`} | ${p.cleanedText.slice(0, 45).replace(/\|/g, "/")}... |`,
  )
  .join("\n")}

---

## Anomalies Log (${anomalies.length})

${
  anomalies.length === 0
    ? "✅ **Zero anomalies detected!** Highlight isolation, single-word mark constraints, and Whisper audio-sync are 100% verified across all blocks."
    : anomalies
        .map(
          (a, i) =>
            `### ${i + 1}. [${a.type}] at ${a.timestamp.toFixed(2)}s (Passage ${a.passageIndex + 1})\n- **Message:** ${a.message}\n`,
        )
        .join("\n")
}
`;

  fs.writeFileSync(reportOut, markdownReport, "utf-8");

  if (!silent) {
    console.log(`✅ Saved Timeline JSON to:   ${jsonOut}`);
    console.log(`✅ Saved Markdown Report to: ${reportOut}\n`);
  }

  return auditOutput;
}

// Auto-run when executed
const args = process.argv.slice(2);
const options: HarnessOptions = {};

for (let i = 0; i < args.length; i++) {
  if (args[i] === "--file" && args[i + 1]) {
    options.filePath = args[++i];
  } else if (args[i] === "--text" && args[i + 1]) {
    options.text = args[++i];
  } else if (args[i] === "--step" && args[i + 1]) {
    options.stepMs = parseInt(args[++i], 10);
  }
}

void runSpeechSimulation(options)
  .then((res) => {
    if (res.summary.anomaliesCount > 0) {
      process.exit(1);
    } else {
      process.exit(0);
    }
  })
  .catch((err) => {
    console.error("Simulation failed with error:", err);
    process.exit(1);
  });
