import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { effectiveModelRuntimePolicy, ModelRuntimePolicyControls } from "./ModelRuntimePolicy";
import type { ControlSettings } from "../../contracts/index";

describe("shared local-model runtime policy", () => {
  it("resolves a selected model override before the global defaults", () => {
    const settings = {
      contextWindow: 32_768,
      maxOutputTokens: 16_384,
      modelOverrides: [{ modelId: "director", contextWindow: 27_648, maxOutputTokens: 27_648 }],
    } as ControlSettings;

    expect(effectiveModelRuntimePolicy(settings, "director")).toEqual({
      contextWindow: 27_648,
      maxOutputTokens: 27_648,
    });
    expect(effectiveModelRuntimePolicy(settings, "other")).toEqual({
      contextWindow: 32_768,
      maxOutputTokens: 16_384,
    });
  });

  it("emits only finite bounded producer edits", () => {
    const onChange = vi.fn();
    render(<ModelRuntimePolicyControls
      value={{ contextWindow: 27_648, maxOutputTokens: 27_648 }}
      disabled={false}
      scope="This production"
      onChange={onChange}
    />);

    fireEvent.change(screen.getByLabelText("This production context window"), { target: { value: "32768" } });
    expect(onChange).toHaveBeenCalledWith({ contextWindow: 32_768, maxOutputTokens: 27_648 });
    fireEvent.change(screen.getByLabelText("This production maximum output"), { target: { value: "" } });
    expect(onChange).toHaveBeenCalledTimes(1);
  });
});
