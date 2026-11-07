import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import * as api from "../../../platform/api";
import { MovieStudio } from "./MovieStudio";
import {
  sampleFinishedMovie, sampleH3CompileResult, sampleH3Environment, sampleMovie,
} from "../../../preview/studioFixtures";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function mockBaseStudioApis() {
  vi.spyOn(api, "listMovies").mockResolvedValue([
    { ...sampleFinishedMovie, clipCount: sampleFinishedMovie.clips.length },
    { ...sampleMovie, clipCount: 0 },
  ]);
  vi.spyOn(api, "getMovie").mockImplementation(async (id) =>
    structuredClone(id === sampleMovie.id ? sampleMovie : sampleFinishedMovie),
  );
  vi.spyOn(api, "getMovieH3Environment").mockResolvedValue({
    ...sampleH3Environment,
    hasRef2vTurboLora: false,
  });
  vi.spyOn(api, "listMovieReferenceAssets").mockResolvedValue([]);
  vi.spyOn(api, "compileMovieH3Window").mockResolvedValue(structuredClone(sampleH3CompileResult));
  vi.spyOn(api, "onMovieProject").mockResolvedValue(() => undefined);
  vi.spyOn(api, "onStudioPromptDraft").mockResolvedValue(() => undefined);
  vi.spyOn(api, "onMovieRenderPreview").mockResolvedValue(() => undefined);
  vi.spyOn(api, "getMovieRenderState").mockResolvedValue({
    active: false,
  });
}

describe("MovieStudio MiniMax H3 Workspace", () => {
  it("loads the active H3 production, installs Turbo LoRAs, and creates a 64x32 Soundtrack scene", async () => {
    mockBaseStudioApis();
    const installSpy = vi
      .spyOn(api, "installMovieH3TurboLoras")
      .mockResolvedValue({ ...sampleH3Environment, hasRef2vTurboLora: true, hasFl2vTurboLora: true });
    const createSceneSpy = vi.spyOn(api, "createMovieH3Scene").mockResolvedValue(
      structuredClone(sampleFinishedMovie),
    );

    render(<MovieStudio advancedEnabled={false} onError={vi.fn()} />);

    await screen.findByRole("heading", { name: "The Frog Who Measured the Sky" });

    // Install H3 Turbo LoRAs from the environment bar
    fireEvent.click(screen.getByRole("button", { name: /Install H3 Turbo LoRAs/i }));
    await waitFor(() => expect(installSpy).toHaveBeenCalledTimes(1));

    // Create a 64x32 Soundtrack scene
    fireEvent.click(screen.getByRole("button", { name: /64×32 Soundtrack/i }));
    await waitFor(() =>
      expect(createSceneSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          projectId: sampleFinishedMovie.id,
          kind: "soundtrack",
          width: 64,
          height: 32,
        }),
      ),
    );
  });

  it("saves unsaved scene edits before starting H3 window generation", async () => {
    mockBaseStudioApis();
    const saveSceneSpy = vi
      .spyOn(api, "saveMovieH3Scene")
      .mockImplementation(async ({ scene }) => ({
        ...structuredClone(sampleFinishedMovie),
        scenes: sampleFinishedMovie.scenes.map((s) => (s.id === scene.id ? scene : s)),
      }));
    const startGenSpy = vi.spyOn(api, "startMovieH3Generation").mockResolvedValue({
      projectId: sampleFinishedMovie.id,
      jobIds: ["job-new-1"],
      queuedCount: 1,
      detail: "Queued 1 H3 window.",
    });

    render(<MovieStudio advancedEnabled={false} onError={vi.fn()} />);
    await screen.findByRole("heading", { name: "The Frog Who Measured the Sky" });

    // Edit shot detailed action to mark scene dirty
    const actionInput = screen.getByLabelText("Shot detailed action");
    fireEvent.change(actionInput, {
      target: { value: "@Pip leaps toward @Crow across @Pond." },
    });
    expect(screen.getByText("Unsaved scene edits")).toBeTruthy();

    // Clicking Generate H3 Window saves the scene first and then starts H3 generation
    fireEvent.click(screen.getByRole("button", { name: /Generate H3 Window/i }));
    await waitFor(() => expect(saveSceneSpy).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(startGenSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          projectId: sampleFinishedMovie.id,
          expandQualityLadder: false,
        }),
      ),
    );
  });

  it("inspects take telemetry, places a take onto the timeline, and renders a master export", async () => {
    mockBaseStudioApis();
    const job = sampleFinishedMovie.renderJobs[0];
    const inspectSpy = vi.spyOn(api, "inspectMovieH3Take").mockResolvedValue({
      jobId: job.id,
      projectId: sampleFinishedMovie.id,
      sceneId: job.sceneId,
      label: job.label,
      status: job.status,
      passPreset: job.passPreset,
      taskMode: job.taskMode,
      resolvedModel: job.resolvedModel,
      seed: job.seed,
      compiledPrompt: job.compiledPrompt,
      takeMediaPath: job.takeMediaPath,
      takeSha256: job.takeSha256,
      geometry: job.geometry,
      colorDrift: job.colorDrift,
      quality: job.quality,
      error: "",
    });
    const placeSpy = vi
      .spyOn(api, "placeMovieH3Take")
      .mockResolvedValue(structuredClone(sampleFinishedMovie));
    const exportSpy = vi.spyOn(api, "renderMovieEdit").mockResolvedValue({
      projectId: sampleFinishedMovie.id,
      sceneId: "frog-main",
      export: sampleFinishedMovie.exports[0],
    });

    render(<MovieStudio advancedEnabled={false} onError={vi.fn()} />);
    await screen.findByRole("heading", { name: "The Frog Who Measured the Sky" });

    // Open Takes & Telemetry room
    fireEvent.click(screen.getByRole("button", { name: /Takes & Telemetry/i }));
    await waitFor(() => expect(inspectSpy).toHaveBeenCalledWith(sampleFinishedMovie.id, job.id));
    await screen.findByText(/Ridge affine RGB match applied/i);

    fireEvent.click(screen.getByRole("button", { name: /Place Take on Timeline/i }));
    await waitFor(() =>
      expect(placeSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          projectId: sampleFinishedMovie.id,
          jobId: job.id,
          placementMode: "trimmed",
        }),
      ),
    );

    // Open Deliver room and render master
    fireEvent.click(screen.getByRole("button", { name: /Deliver/i }));
    fireEvent.click(screen.getByRole("button", { name: /Render Master/i }));
    await waitFor(() =>
      expect(exportSpy).toHaveBeenCalledWith(
        expect.objectContaining({
          projectId: sampleFinishedMovie.id,
          preset: "publish",
          peakHeadroomDb: -1,
        }),
      ),
    );
  });
});
