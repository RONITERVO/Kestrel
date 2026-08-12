import { describe, expect, it } from "vitest";
import { appendTimelineSource, formatTimecode, moveTimelineItem, splitTimelineItem, timelineItems } from "./MovieTimeline";
import type { ClipEdit, MovieEdit, MovieProject } from "./types";

const decision = (id: string, clipId: string, order: number): ClipEdit => ({
  id, clipId, order, enabled: true, trimStart: 0, trimEnd: 0, audioGain: 1,
  sourceVersionId: "", speed: 1, fadeIn: 0, fadeOut: 0, audioFadeIn: 0, audioFadeOut: 0, label: "", notes: "",
});

const project = {
  id: "project", clips: [
    { id: "one", index: 0, title: "One", prompt: "", durationSeconds: 10, seed: 1, status: "complete", path: "one.mp4", error: "", versions: [
      { id: "short", createdAt: "", title: "Short", prompt: "", durationSeconds: 8, seed: 2, path: "short.mp4" },
    ] },
    { id: "two", index: 1, title: "Two", prompt: "", durationSeconds: 5, seed: 2, status: "complete", path: "two.mp4", error: "", versions: [] },
  ],
} as MovieProject;

const movieEdit = (clips: ClipEdit[]): MovieEdit => ({
  clips, exportTitle: "Test", exportPreset: "publish", normalizeAudio: false, targetLufs: -14, markers: [],
});

describe("movie timeline decisions", () => {
  it("calculates edited duration from the selected immutable version", () => {
    const edit = decision("a", "one", 0);
    edit.sourceVersionId = "short";
    edit.trimStart = 1;
    edit.trimEnd = 1;
    edit.speed = 2;
    const [item] = timelineItems(project, movieEdit([edit]));
    expect(item.sourcePath).toBe("short.mp4");
    expect(item.outputDuration).toBe(3);
    expect(item.versionLabel).toBe("Version short");
  });

  it("reorders decisions without changing their identities", () => {
    const edit = movieEdit([decision("a", "one", 0), decision("b", "two", 1)]);
    const moved = moveTimelineItem(edit, "b", "a");
    expect(moved.clips.map((item) => [item.id, item.order])).toEqual([["b", 0], ["a", 1]]);
  });

  it("splits one source into adjacent non-overlapping decisions", () => {
    const edit = movieEdit([decision("a", "one", 0), decision("b", "two", 1)]);
    const split = splitTimelineItem(project, edit, "a", 4, "cut-b");
    expect(split.clips.map((item) => item.id)).toEqual(["a", "cut-b", "b"]);
    expect(split.clips[0].trimEnd).toBe(6);
    expect(split.clips[1].trimStart).toBe(4);
    expect(timelineItems(project, split).slice(0, 2).reduce((sum, item) => sum + item.outputDuration, 0)).toBe(10);
  });

  it("appends a preserved master as a new non-destructive storyline decision", () => {
    const appended = appendTimelineSource(movieEdit([decision("a", "one", 0)]), "two", "new-edit");
    expect(appended.clips.map((item) => [item.id, item.clipId, item.order])).toEqual([
      ["a", "one", 0], ["new-edit", "two", 1],
    ]);
    expect(appended.clips[1]).toMatchObject({ enabled: true, speed: 1, label: "", notes: "" });
  });

  it("shows familiar 24 fps producer timecode", () => {
    expect(formatTimecode(65.5)).toBe("00:01:05:12");
    expect(formatTimecode(3661 + 1 / 24)).toBe("01:01:01:01");
  });
});
