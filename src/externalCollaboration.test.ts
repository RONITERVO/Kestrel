import { describe, expect, it } from "vitest";
import {
  buildExternalCollaborationRequest, EXTERNAL_COLLABORATION_FORMAT,
  parseExternalCollaborationResult, parseExternalTextResult,
} from "./externalCollaboration";

describe("external collaboration contract", () => {
  it("builds a provider-neutral request and accepts a fenced matching response", () => {
    const request = buildExternalCollaborationRequest({
      target: "movie-brief",
      role: "a story editor",
      instructions: ["Preserve the producer's intent."],
      context: { existingText: "A lighthouse at dawn." },
      resultTemplate: { text: "Complete editable brief" },
    });
    expect(request).toContain(EXTERNAL_COLLABORATION_FORMAT);
    expect(request).not.toMatch(/OpenAI|ChatGPT|Gemini|Claude/i);

    const result = parseExternalTextResult(`\n\`\`\`json\n${JSON.stringify({
      format: EXTERNAL_COLLABORATION_FORMAT,
      version: 1,
      target: "movie-brief",
      result: { text: "A lighthouse keeper follows a signal into the fog." },
    })}\n\`\`\`\n`, "movie-brief");
    expect(result).toContain("keeper");
  });

  it("rejects a response copied for another field", () => {
    const response = JSON.stringify({
      format: EXTERNAL_COLLABORATION_FORMAT,
      version: 1,
      target: "music-lyrics",
      result: { text: "Wrong target" },
    });
    expect(() => parseExternalCollaborationResult(response, "music-description")).toThrow(/not music-description/i);
  });
});
