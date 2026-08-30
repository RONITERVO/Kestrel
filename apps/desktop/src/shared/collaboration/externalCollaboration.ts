export const EXTERNAL_COLLABORATION_FORMAT = "kestrel.external-collaboration.response";
export const EXTERNAL_COLLABORATION_VERSION = 1;
export const MAX_EXTERNAL_COLLABORATION_BYTES = 2 * 1024 * 1024;

export type ExternalCollaborationTarget =
  | "movie-brief"
  | "movie-image-description"
  | "movie-reference-description"
  | "image-design"
  | "music-description"
  | "music-lyrics"
  | "movie-generation-direction";

interface ExternalRequestOptions {
  target: ExternalCollaborationTarget;
  role: string;
  instructions: string[];
  context: unknown;
  resultTemplate: unknown;
}

export function buildExternalCollaborationRequest({ target, role, instructions, context, resultTemplate }: ExternalRequestOptions): string {
  const response = {
    format: EXTERNAL_COLLABORATION_FORMAT,
    version: EXTERNAL_COLLABORATION_VERSION,
    target,
    result: resultTemplate,
  };
  const request = [
    `Act as ${role} for a Kestrel production. You are a collaborator, not an execution agent: do not call tools, claim to change files, or invent file paths.`,
    "Return exactly one JSON object and no Markdown commentary. Treat existing text as producer material: it may be an idea, notes, a partial draft, or exact wording, so infer the producer's intent instead of blindly appending it.",
    "",
    "Required response envelope and field spelling:",
    JSON.stringify(response, null, 2),
    "",
    "Production rules:",
    ...instructions.map((instruction) => `- ${instruction}`),
    "- Keep JSON strings validly escaped. Put no analysis, reasoning, or commentary outside result.",
    "",
    "Complete bounded production context:",
    JSON.stringify(context, null, 2),
  ].join("\n");
  if (new Blob([request]).size > MAX_EXTERNAL_COLLABORATION_BYTES) {
    throw new Error("This production context is too large for one collaborator exchange. Shorten the current draft or exchange a smaller part.");
  }
  return request;
}

export function parseExternalCollaborationResult(text: string, target: ExternalCollaborationTarget): unknown {
  if (new Blob([text]).size > MAX_EXTERNAL_COLLABORATION_BYTES) {
    throw new Error("The collaborator response is larger than the 2 MiB exchange limit.");
  }
  const cleaned = stripJsonFence(text);
  let value: unknown;
  try {
    value = JSON.parse(cleaned);
  } catch (error) {
    throw new Error(`The pasted response is not valid JSON: ${String(error)}`);
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("The pasted response must be one JSON object.");
  }
  const envelope = value as Record<string, unknown>;
  if (envelope.format !== EXTERNAL_COLLABORATION_FORMAT) {
    throw new Error(`Unsupported collaborator response format. Expected ${EXTERNAL_COLLABORATION_FORMAT}.`);
  }
  if (envelope.version !== EXTERNAL_COLLABORATION_VERSION) {
    throw new Error(`Unsupported collaborator response version. Expected ${EXTERNAL_COLLABORATION_VERSION}.`);
  }
  if (envelope.target !== target) {
    throw new Error(`This response is for ${String(envelope.target || "an unknown target")}, not ${target}. Copy a fresh request from this field.`);
  }
  if (!("result" in envelope)) throw new Error("The collaborator response is missing its result field.");
  return envelope.result;
}

export function parseExternalTextResult(text: string, target: ExternalCollaborationTarget, maximumCharacters = 65_536): string {
  const result = parseExternalCollaborationResult(text, target);
  if (!result || typeof result !== "object" || Array.isArray(result)) {
    throw new Error("The collaborator result must be an object containing text.");
  }
  const output = (result as Record<string, unknown>).text;
  if (typeof output !== "string" || output.trim().length < 3) {
    throw new Error("The collaborator result needs non-empty text.");
  }
  if (output.length > maximumCharacters) {
    throw new Error(`The collaborator result exceeds this field's ${maximumCharacters.toLocaleString()} character limit.`);
  }
  return output.trim();
}

export function stripJsonFence(text: string): string {
  const trimmed = text.trim();
  const fenced = /^```(?:json)?\s*([\s\S]*?)\s*```$/i.exec(trimmed);
  return fenced ? fenced[1].trim() : trimmed;
}
