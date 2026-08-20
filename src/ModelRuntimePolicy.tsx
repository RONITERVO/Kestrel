import type { ControlSettings } from "./types";
import limits from "./runtimePolicyLimits.json";

export interface RuntimePolicyValue {
  contextWindow: number;
  maxOutputTokens: number;
}

export function effectiveModelRuntimePolicy(
  settings: ControlSettings | undefined,
  modelId: string | undefined,
): RuntimePolicyValue {
  const fallback = {
    contextWindow: settings?.contextWindow ?? 32_768,
    maxOutputTokens: settings?.maxOutputTokens ?? 32_768,
  };
  if (!settings || !modelId) return fallback;
  const override = settings.modelOverrides?.find((item) => item.modelId === modelId);
  return {
    contextWindow: override?.contextWindow ?? fallback.contextWindow,
    maxOutputTokens: override?.maxOutputTokens ?? fallback.maxOutputTokens,
  };
}

export function ModelRuntimePolicyControls({
  value,
  inherited,
  disabled,
  expert = false,
  scope,
  onChange,
  onReset,
}: {
  value: RuntimePolicyValue;
  inherited?: RuntimePolicyValue;
  disabled: boolean;
  expert?: boolean;
  scope: string;
  onChange: (value: RuntimePolicyValue) => void;
  onReset?: () => void;
}) {
  const change = (key: keyof RuntimePolicyValue, raw: string) => {
    const parsed = Number.parseInt(raw, 10);
    const tier = expert ? limits.advanced : limits.standard;
    const minimum = key === "contextWindow" ? limits.minimumContextWindow : limits.minimumMaxOutputTokens;
    const maximum = key === "contextWindow" ? tier.maximumContextWindow : tier.maximumMaxOutputTokens;
    if (!Number.isFinite(parsed) || parsed < minimum || parsed > maximum) return;
    onChange({ ...value, [key]: parsed });
  };
  const inheritedNow = Boolean(inherited)
    && value.contextWindow === inherited?.contextWindow
    && value.maxOutputTokens === inherited?.maxOutputTokens;
  return <fieldset className="model-runtime-policy" disabled={disabled}>
    <legend>Local model limits</legend>
    <label><span>Context window<small>{scope}</small></span><input aria-label={`${scope} context window`} type="number" min={limits.minimumContextWindow} max={expert ? limits.advanced.maximumContextWindow : limits.standard.maximumContextWindow} step={1_024} value={value.contextWindow} onChange={(event) => change("contextWindow", event.target.value)} /></label>
    <label><span>Maximum output<small>{scope}</small></span><input aria-label={`${scope} maximum output`} type="number" min={limits.minimumMaxOutputTokens} max={expert ? limits.advanced.maximumMaxOutputTokens : limits.standard.maximumMaxOutputTokens} step={1_024} value={value.maxOutputTokens} onChange={(event) => change("maxOutputTokens", event.target.value)} /></label>
    <div className="model-runtime-policy-state"><strong>{inheritedNow ? "Matches current model policy" : "Workspace override"}</strong><small>Changes apply to the next model turn. A context change safely reloads the selected model when required.</small></div>
    {onReset && <button type="button" disabled={disabled || inheritedNow} onClick={onReset}>Restore current model values</button>}
  </fieldset>;
}
