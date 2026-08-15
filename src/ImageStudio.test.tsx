import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { compiledPrompt, ImageStudio, parseImageProposal } from "./ImageStudio";
import type { ImageProject } from "./types";

describe("ImageStudio", () => {
  it("opens a no-code project brief from the fixed-window empty state", async () => {
    render(<ImageStudio advancedEnabled models={[]} onError={vi.fn()} />);
    const button = await screen.findByRole("button", { name: /new image project/i });
    fireEvent.click(button);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByLabelText(/idea or complete brief/i)).toBeInTheDocument();
  });

  it("accepts a bounded Ideogram JSON proposal with exact text and layout", () => {
    const proposal = parseImageProposal(JSON.stringify({
      high_level_description: "A graphic poster for a night exhibition.",
      style_description: {
        aesthetics: "Swiss editorial grid",
        lighting: "Flat studio lighting",
        photo: "Crisp print texture",
        medium: "Graphic design",
        color_palette: ["#111111", "#F2C14E"],
      },
      compositional_deconstruction: {
        background: "Matte black paper with a narrow yellow rule.",
        elements: [{
          type: "text",
          bbox: [80, 100, 260, 900],
          text: "NIGHT / FORM",
          desc: "Large upright bold sans-serif title.",
          color_palette: ["#F2C14E"],
        }],
      },
    }));
    expect(proposal.elements[0].text).toBe("NIGHT / FORM");
    expect(proposal.elements[0].bbox).toEqual([80, 100, 260, 900]);
    expect(proposal.style.medium).toBe("Graphic design");
    expect(proposal.style.mode).toBe("photo");
  });

  it("accepts artwork styling, normalizes palettes, and compiles the exclusive ordered schema", () => {
    const proposal = parseImageProposal(JSON.stringify({
      high_level_description: "A geometric editorial bird poster.",
      style_description: {
        aesthetics: "Restrained and precise",
        lighting: "Flat graphic light",
        medium: "Risograph print",
        art_style: "Bauhaus-inspired geometric illustration",
        color_palette: ["#aabbcc"],
      },
      compositional_deconstruction: {
        background: "Warm uncoated paper.",
        elements: [{ type: "obj", bbox: [100, 100, 900, 900], desc: "Angular kestrel", color_palette: ["#cc3300"] }],
      },
    }));
    const prompt = compiledPrompt({
      highLevelDescription: proposal.highLevelDescription,
      style: proposal.style,
      background: proposal.background,
      elements: proposal.elements,
    } as ImageProject);
    expect(proposal.style.mode).toBe("art");
    expect(proposal.style.colorPalette).toEqual(["#AABBCC"]);
    expect(Object.keys(prompt)).toEqual(["high_level_description", "style_description", "compositional_deconstruction"]);
    expect(Object.keys(prompt.style_description as object)).toEqual(["aesthetics", "lighting", "medium", "art_style", "color_palette"]);
    expect(prompt.style_description).not.toHaveProperty("photo");
    expect(Object.keys((prompt.compositional_deconstruction as { elements: object[] }).elements[0])).toEqual(["type", "bbox", "desc", "color_palette"]);
  });

  it("rejects mixed photo and artwork schema instead of guessing", () => {
    expect(() => parseImageProposal(JSON.stringify({
      high_level_description: "A complete image description.",
      style_description: { aesthetics: "Editorial", lighting: "Daylight", photo: "Clean", art_style: "Painted", medium: "Mixed media" },
      compositional_deconstruction: { background: "A real location.", elements: [] },
    }))).toThrow(/exactly one/i);
  });

  it("rejects malformed or inverted model-authored boxes before producer state changes", () => {
    expect(() => parseImageProposal(JSON.stringify({
      high_level_description: "A complete image description.",
      style_description: { aesthetics: "Editorial", lighting: "Daylight", photo: "Clean", medium: "Photograph", color_palette: [] },
      compositional_deconstruction: {
        background: "A real location.",
        elements: [{ type: "obj", bbox: [900, 50, 100, 950], desc: "Subject" }],
      },
    }))).toThrow(/bbox/i);
  });
});
