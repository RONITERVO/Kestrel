import type {
  ControlSettings,
  ProvenHardwareProfile,
  ThinkingLevel,
} from "../../contracts/index";

// View projections only. Rust resolves and validates the effective runtime policy before use.
export function thinkingBudgetForLevel(level: ThinkingLevel, maxOutputTokens = 32768): number {
  switch (level) {
    case "off": return 0;
    case "low": return Math.min(2048, maxOutputTokens);
    case "medium": return Math.min(8192, maxOutputTokens);
    case "high": return Math.min(16384, maxOutputTokens);
    case "max": return maxOutputTokens;
  }
}

export function thinkingLevelFromBudget(budget: number): ThinkingLevel {
  if (budget <= 0) return "off";
  if (budget <= 2048) return "low";
  if (budget <= 8192) return "medium";
  if (budget <= 20000) return "high";
  return "max";
}

export function effectiveThinkingLevelForModel(
  control?: ControlSettings,
  modelId?: string,
): ThinkingLevel {
  if (!control) return "high";
  if (modelId) {
    const override = control.modelOverrides?.find((item) => item.modelId === modelId);
    if (override?.thinkingLevel) return override.thinkingLevel;
  }
  return control.thinkingLevel ?? "high";
}

export function findProvenHardwareProfile(
  profiles: ProvenHardwareProfile[] | undefined,
  modelName: string | undefined,
  vramMib: number | undefined,
): ProvenHardwareProfile | undefined {
  if (!profiles?.length || !modelName) return undefined;
  const lower = modelName.toLowerCase();
  return profiles.find((profile) => {
    if (!lower.includes(profile.modelPattern.toLowerCase())) return false;
    if (profile.quantizationPattern && !lower.includes(profile.quantizationPattern.toLowerCase())) return false;
    if (vramMib !== undefined) {
      if (vramMib < profile.minVramMib) return false;
      if (profile.maxVramMib !== undefined && vramMib > profile.maxVramMib) return false;
    }
    return true;
  });
}

export const STANDARD_CONTEXT_OPTIONS: Array<{ value: number; label: string }> = [
  { value: 4096, label: "4k (4,096 tokens)" },
  { value: 8192, label: "8k (8,192 tokens)" },
  { value: 16384, label: "16k (16,384 tokens)" },
  { value: 24576, label: "24k (24,576 tokens)" },
  { value: 32768, label: "32k (32,768 tokens)" },
  { value: 49152, label: "48k (49,152 tokens)" },
  { value: 65536, label: "64k (65,536 tokens)" },
  { value: 131072, label: "128k (131,072 tokens)" },
  { value: 262144, label: "256k (262,144 tokens)" },
];

export const STANDARD_OUTPUT_OPTIONS: Array<{ value: number; label: string }> = [
  { value: 2048, label: "2k (2,048 tokens)" },
  { value: 4096, label: "4k (4,096 tokens)" },
  { value: 8192, label: "8k (8,192 tokens)" },
  { value: 16384, label: "16k (16,384 tokens)" },
  { value: 32768, label: "32k (32,768 tokens)" },
];
