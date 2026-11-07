import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import * as api from "../../../platform/api";
import { MovieGenerationPanel, parseTimelineSeconds } from "./MovieGenerationPanel";
import { sampleFinishedMovie, sampleH3CompileResult } from "../../../preview/studioFixtures";
import type { H3Scene } from "../../../contracts/index";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function PanelHarness({
  onStartGeneration = vi.fn().mockResolvedValue(undefined),
  onError = vi.fn(),
}: {
  onStartGeneration?: (expandQualityLadder: boolean) => Promise<void>;
  onError?: (message: string) => void;
}) {
  const project = structuredClone(sampleFinishedMovie);
  const [scene, setScene] = useState<H3Scene>(() => structuredClone(project.scenes[0]));
  const [selectedShotId, setSelectedShotId] = useState<string>(project.scenes[0].promptSections[0].id);
  return (
    <MovieGenerationPanel
      project={project}
      scene={scene}
      selectedShotId={selectedShotId}
      compileResult={sampleH3CompileResult}
      rendering={false}
      onSceneChange={setScene}
      onSelectShot={setSelectedShotId}
      onStartGeneration={onStartGeneration}
      onStopGeneration={vi.fn()}
      onError={onError}
    />
  );
}

describe("MovieGenerationPanel MiniMax H3", () => {
  it("parses plain and colon-delimited timeline seconds", () => {
    expect(parseTimelineSeconds("12.5")).toBe(12.5);
    expect(parseTimelineSeconds("01:05")).toBe(65);
    expect(parseTimelineSeconds("01:01:01")).toBe(3661);
    expect(Number.isNaN(parseTimelineSeconds("01:75"))).toBe(true);
  });

  it("inserts @Handle tokens into shot action, stages unstaged entities, and authors <d>[Language] ...</d> dialogue", async () => {
    const stageSpy = vi.spyOn(api, "stageMovieH3Reference").mockResolvedValue({
      projectId: sampleFinishedMovie.id,
      sceneId: sampleFinishedMovie.scenes[0].id,
      laneId: "frog-main-r1",
      laneName: "R1",
      itemId: "staged-pond",
      label: "@Pond",
      startSec: 0,
      endSec: 5,
    });

    render(<PanelHarness />);

    // Click @Pond handle chip to insert @Pond into shot detailed action
    fireEvent.click(screen.getByTitle(/Insert @Pond into shot action/i));
    const actionBox = screen.getByLabelText("Shot detailed action") as HTMLTextAreaElement;
    expect(actionBox.value).toContain("@Pond");

    // @Pond is not in compileResult.ordinals.subjects, so +Stage button is visible
    fireEvent.click(screen.getByTitle(/Stage @Pond in/i));
    await waitFor(() =>
      expect(stageSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          projectId: sampleFinishedMovie.id,
          entityId: "ent-pond",
          startSec: 0,
          endSec: 5,
        }),
      ),
    );

    // Add a new vocal line and edit its spoken text
    fireEvent.click(screen.getByRole("button", { name: /^Line$/i }));
    const spokenInputs = screen.getAllByLabelText("Vocal spoken text") as HTMLInputElement[];
    fireEvent.change(spokenInputs[spokenInputs.length - 1], {
      target: { value: "Count the clouds instead." },
    });
    expect(spokenInputs[spokenInputs.length - 1].value).toBe("Count the clouds instead.");
  });

  it("queues Sonder's 3-pass quality ladder when toggled and displays compiled 6-channel prompt", async () => {
    const onStart = vi.fn().mockResolvedValue(undefined);
    render(<PanelHarness onStartGeneration={onStart} />);

    // Switch to Pass & Mode tab and toggle 3-pass quality ladder
    fireEvent.click(screen.getByRole("button", { name: /Pass & Mode/i }));
    const ladderCheckbox = screen.getByRole("checkbox", {
      name: /Run 3-pass Sonder quality ladder/i,
    });
    fireEvent.click(ladderCheckbox);

    fireEvent.click(screen.getByRole("button", { name: /Queue 3-Pass Quality Ladder/i }));
    await waitFor(() => expect(onStart).toHaveBeenCalledWith(true));

    // Switch to Compiler tab and verify bound <Subject> ordinals and compiled 6-channel prompt
    fireEvent.click(screen.getByRole("button", { name: /Compiler/i }));
    expect(screen.getByText(/608×352/)).toBeTruthy();
    expect(screen.getByText(/@Pip → <Subject 1>/)).toBeTruthy();
    expect(screen.getByText(/Overall Summary: <Subject 1> counts lily pads/)).toBeTruthy();
  });
});
