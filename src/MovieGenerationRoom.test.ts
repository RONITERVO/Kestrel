import { describe, expect, it } from "vitest";
import { bridgeAnchorsForCut, editWithSourceVersion, insertTimelineSourceAfter } from "./MovieGenerationRoom";
import { timelineItems } from "./MovieTimeline";
import type { ClipEdit, MovieEdit, MovieProject } from "./types";

const decision = (id: string, clipId: string, order: number): ClipEdit => ({
  id, clipId, order, enabled: true, trimStart: 0, trimEnd: 0, audioGain: 1,
  sourceVersionId: "", speed: 1, fadeIn: 0, fadeOut: 0, audioFadeIn: 0,
  audioFadeOut: 0, label: "", notes: "",
});

const edit = (clips: ClipEdit[]): MovieEdit => ({
  clips, exportTitle: "Test", exportPreset: "publish", normalizeAudio: false,
  targetLufs: -14, markers: [],
});

const project = {
  clips: [
    { id: "one", index: 0, title: "One", prompt: "", durationSeconds: 10, seed: 1, status: "complete", path: "one.mp4", error: "", versions: [] },
    { id: "two", index: 1, title: "Two", prompt: "", durationSeconds: 8, seed: 2, status: "complete", path: "two.mp4", error: "", versions: [] },
  ],
} as unknown as MovieProject;

describe("Generate audition decisions", () => {
  it("anchors a bridge to exact storyline edit IDs, not repeated source clip IDs", () => {
    const first = decision("edit-a", "one", 0);
    first.trimEnd = 2;
    const second = decision("edit-b", "two", 1);
    second.trimStart = 1.5;
    const anchors = bridgeAnchorsForCut(timelineItems(project, edit([first, second])), 0);
    expect(anchors).toEqual([
      { editId: "edit-a", timeSeconds: 7.96, label: "One · cut out" },
      { editId: "edit-b", timeSeconds: 1.5, label: "Two · cut in" },
    ]);
  });

  it("changes only the selected storyline decision when accepting an audition", () => {
    const repeatedA = decision("edit-a", "one", 0);
    repeatedA.trimStart = 2;
    const repeatedB = decision("edit-b", "one", 1);
    const updated = editWithSourceVersion(edit([repeatedA, repeatedB]), "edit-b", "audition-2");
    expect(updated.clips[0]).toMatchObject({ sourceVersionId: "", trimStart: 2 });
    expect(updated.clips[1]).toMatchObject({ sourceVersionId: "audition-2", trimStart: 0, trimEnd: 0 });
  });

  it("places a generated master after the exact selected edit and normalizes order", () => {
    const updated = insertTimelineSourceAfter(
      edit([decision("edit-a", "one", 0), decision("edit-b", "two", 1)]),
      "bridge-master",
      "edit-a",
      "edit-bridge",
    );
    expect(updated.clips.map((clip) => [clip.id, clip.clipId, clip.order])).toEqual([
      ["edit-a", "one", 0],
      ["edit-bridge", "bridge-master", 1],
      ["edit-b", "two", 2],
    ]);
  });
});
