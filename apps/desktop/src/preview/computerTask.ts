import type { ComputerTaskRun } from "../contracts/index";

// A finished Computer Task for the browser preview, with a long final answer so the transcript's
// page flow can be checked without the desktop application.
const at = "2026-09-20T12:00:00Z";
const games = ["Skyrim Special Edition", "Skyrim VR", "GTA V", "Red Dead Redemption 2", "Assetto Corsa Competizione", "Elden Ring", "Hades", "Stardew Valley"];
const answer = [
  "| Game | Folder |",
  "|------|--------|",
  ...games.map((game) => `| **${game}** | \`My Games\\${game}\` |`),
  "",
  "**Benchmark tools (not games, but game-adjacent):**",
  "- **3DMark** - GPU/CPU benchmark",
  "- **VRMark** - VR benchmark",
  "",
  `So you have **${games.length} games** in your workspace. ${"The Rockstar Games folder also has a duplicate on your laptop with the same saves. ".repeat(3)}`,
].join("\n");

export const sampleComputerTask = {
  id: "preview-task", objective: "what games i have", modelId: "preview-model", access: "workspace", status: "completed",
  createdAt: at, updatedAt: at, artifacts: [],
  events: [
    { runId: "preview-task", step: 1, kind: "tool_start", title: "List directory", detail: "C:\\Users\\Researcher\\Documents\\My Games", at },
    { runId: "preview-task", step: 1, kind: "tool_result", title: "Directory listing", detail: games.map((game) => `${game}/`).join("\n"), at },
    { runId: "preview-task", step: 2, kind: "thinking", title: "Model is deciding", detail: "Planning the next visible action (medium thinking).", data: { thinkingLevel: "medium" }, at },
    { runId: "preview-task", step: 3, kind: "reasoning", title: "Reasoning", detail: "Let me look deeper into the Skyrim folders to see if there's more detail.", at },
    { runId: "preview-task", step: 3, kind: "done", title: "Completed", detail: answer, at },
  ],
} satisfies ComputerTaskRun;
