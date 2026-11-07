import { createElement, useState } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  activeMonitorSourceAtPlayhead, clampSceneWindow, formatTimecode,
  MovieTimeline, snapToFrame, splitH3VideoClip,
} from "./MovieTimeline";
import { sampleFinishedMovie, sampleH3CompileResult } from "../../../preview/studioFixtures";
import type { H3Scene, H3VideoClip, MovieProject } from "../../../contracts/index";

vi.mock("./MovieAssetCreator", () => ({
  GenerateAssetButton: () => null,
}));

afterEach(cleanup);

function cloneProject(): { project: MovieProject; scene: H3Scene } {
  const project = structuredClone(sampleFinishedMovie);
  const scene = structuredClone(project.scenes[0]);
  return { project, scene };
}

function Harness({
  initialProject,
  initialScene,
  onSave,
}: {
  initialProject: MovieProject;
  initialScene: H3Scene;
  onSave?: (scene: H3Scene) => void;
}) {
  const [scene, setScene] = useState(initialScene);
  const [savedScene, setSavedScene] = useState(initialScene);
  const [playhead, setPlayhead] = useState(0);
  return createElement(
    "div",
    null,
    createElement(
      "button",
      {
        type: "button",
        onClick: () => {
          const placed: H3VideoClip = {
            id: "vclip-pass-2",
            sourceKind: "take",
            sourceId: "job-frog-2",
            label: "Pass 2 1MP Detail",
            timelineStartSec: 5,
            durationSec: 5,
            sourceInSec: 0,
            sourceDurationSec: 5,
            opacity: 1,
            audioGain: 1,
            includeEmbeddedAudio: true,
            enabled: true,
          };
          const next: H3Scene = {
            ...scene,
            videoLanes: scene.videoLanes.map((l, idx) =>
              idx === 0 ? { ...l, clips: [...l.clips, placed] } : l,
            ),
          };
          setScene(next);
          setSavedScene(next);
        },
      },
      "Simulate Native Take Placement",
    ),
    createElement(MovieTimeline, {
      project: initialProject,
      scene,
      savedScene,
      playheadSec: playhead,
      compileResult: sampleH3CompileResult,
      dirty: scene !== savedScene,
      onSceneChange: setScene,
      onSaveScene: () => {
        setSavedScene(scene);
        onSave?.(scene);
      },
      onPlayhead: setPlayhead,
    }),
  );
}

describe("MovieTimeline MiniMax H3 NLE", () => {
  it("snaps seconds to 24fps frames, formats SMPTE timecodes, and clamps H3 windows to <= 15s", () => {
    expect(snapToFrame(1.0416667, 24)).toBeCloseTo(25 / 24, 5);
    expect(formatTimecode(65.5, 24)).toBe("00:01:05:12");
    expect(formatTimecode(3661 + 1 / 24, 24)).toBe("01:01:01:01");

    const { scene } = cloneProject();
    const clampedOut = clampSceneWindow({ ...scene.window, inSec: 0, outSec: 5 }, { outSec: 25 }, 30);
    expect(clampedOut.inSec).toBe(10);
    expect(clampedOut.outSec).toBe(25);
    expect(clampedOut.outSec - clampedOut.inSec).toBe(15);

    const clampedIn = clampSceneWindow({ ...scene.window, inSec: 10, outSec: 25 }, { inSec: 2 }, 30);
    expect(clampedIn.inSec).toBe(2);
    expect(clampedIn.outSec).toBe(17);
  });

  it("splits a placed H3 video clip cleanly with frame-snapped source offsets", () => {
    const { scene } = cloneProject();
    const split = splitH3VideoClip(scene, scene.videoLanes[0].id, "vclip-1", 2, "vclip-right");
    const clips = split.videoLanes[0].clips;
    expect(clips).toHaveLength(2);
    expect(clips[0]).toMatchObject({ id: "vclip-1", timelineStartSec: 0, durationSec: 2, sourceInSec: 0 });
    expect(clips[1]).toMatchObject({ id: "vclip-right", timelineStartSec: 2, durationSec: 3, sourceInSec: 2 });
  });

  it("resolves the active program monitor source at playhead respecting lane solo/mute", () => {
    const { project, scene } = cloneProject();
    const hit = activeMonitorSourceAtPlayhead(project, scene, 1.5);
    expect(hit).toMatchObject({
      id: "job-frog-1",
      sourceKind: "renderedTake",
      timelineStartSec: 0,
      durationSec: 5,
    });

    const mutedScene: H3Scene = {
      ...scene,
      videoLanes: scene.videoLanes.map((l) => ({ ...l, muted: true })),
    };
    expect(activeMonitorSourceAtPlayhead(project, mutedScene, 1.5)).toBeUndefined();
  });

  it("renders live specificity verdicts (<Subject 1>, winner) and supports undo/redo across native take placements", () => {
    const { project, scene } = cloneProject();
    render(createElement(Harness, { initialProject: project, initialScene: scene }));

    // Staged @Pip and @Crow show their compiled <Subject 1> / <Subject 2> tags
    expect(screen.getByText("<Subject 1>")).toBeTruthy();
    expect(screen.getByText("<Subject 2>")).toBeTruthy();

    // Mutate scene duration to create an undoable local edit
    const durInput = screen.getByLabelText("Scene duration seconds") as HTMLInputElement;
    fireEvent.change(durInput, { target: { value: "24" } });
    expect(durInput.value).toBe("24");

    // Simulate a native take placement arriving from the H3 queue
    fireEvent.click(screen.getByRole("button", { name: "Simulate Native Take Placement" }));
    expect(screen.getByText(/Pass 2 1MP Detail/)).toBeTruthy();

    // Undo and Redo remain available and preserve the native take history
    const undoBtn = screen.getByRole("button", { name: "Undo timeline change" });
    expect(undoBtn).not.toBeDisabled();
    fireEvent.click(undoBtn);
    const redoBtn = screen.getByRole("button", { name: "Redo timeline change" });
    expect(redoBtn).not.toBeDisabled();
    fireEvent.click(redoBtn);
    expect(screen.getByText(/Pass 2 1MP Detail/)).toBeTruthy();
  });

  it("windows ruler ticks and clip buttons on a 2-hour scene so the DOM stays bounded", () => {
    const { project, scene } = cloneProject();
    const longClips: H3VideoClip[] = Array.from({ length: 1_440 }, (_, idx) => ({
      id: `clip-${idx + 1}`,
      sourceKind: "take",
      sourceId: "job-frog-1",
      label: `Take ${idx + 1}`,
      timelineStartSec: idx * 5,
      durationSec: 5,
      sourceInSec: 0,
      sourceDurationSec: 5,
      opacity: 1,
      audioGain: 1,
      includeEmbeddedAudio: true,
      enabled: true,
    }));
    const longScene: H3Scene = {
      ...scene,
      durationSec: 7_200,
      videoLanes: [{ ...scene.videoLanes[0], clips: longClips }],
    };

    const view = render(
      createElement(MovieTimeline, {
        project,
        scene: longScene,
        savedScene: longScene,
        playheadSec: 0,
        compileResult: sampleH3CompileResult,
        onSceneChange: () => undefined,
        onPlayhead: () => undefined,
      }),
    );

    expect(view.container.querySelectorAll(".h3-ruler-tick").length).toBeLessThan(40);
    expect(view.container.querySelectorAll(".h3-clip-block.video").length).toBeLessThanOrEqual(24);
  });
});
