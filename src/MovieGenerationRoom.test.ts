import { describe, expect, it } from "vitest";
import {
  boundedGenerationFrame, editWithSourceVersion, insertTimelineSourceAfter, insertTimelineSourceBefore,
  generationRequestIsActive, mergeGenerationAgentEvents, parseGenerationTimecode, preferredFrameAnalystModelId,
  replayGenerationAgentEvents, replacementRangeAnchors, transitionAnchorsForPosition,
} from "./MovieGenerationRoom";
import { timelineItems } from "./MovieTimeline";
import type { ClipEdit, MovieEdit, MovieGenerationAgentEvent, MovieProject } from "./types";

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
  it("anchors an existing cut to exact storyline edit IDs, not repeated source clip IDs", () => {
    const first = decision("edit-a", "one", 0);
    first.trimEnd = 2;
    const second = decision("edit-b", "two", 1);
    second.trimStart = 1.5;
    const anchors = transitionAnchorsForPosition(timelineItems(project, edit([first, second])), "between", 0);
    expect(anchors).toEqual({
      firstAnchor: { editId: "edit-a", timeSeconds: 7.96, label: "One · cut out" },
      lastAnchor: { editId: "edit-b", timeSeconds: 1.5, label: "Two · cut in" },
    });
  });

  it("uses only the constrained story endpoint before the opening and after the ending", () => {
    const first = decision("edit-a", "one", 0);
    first.trimStart = 1.25;
    const last = decision("edit-b", "two", 1);
    last.trimEnd = 2;
    const items = timelineItems(project, edit([first, last]));
    expect(transitionAnchorsForPosition(items, "before", 0)).toEqual({
      lastAnchor: { editId: "edit-a", timeSeconds: 1.25, label: "One · story begins" },
    });
    expect(transitionAnchorsForPosition(items, "after", 1)).toEqual({
      firstAnchor: { editId: "edit-b", timeSeconds: 5.96, label: "Two · story ends" },
    });
  });

  it("creates frame-aligned replacement endpoints inside the selected visible shot", () => {
    const selected = decision("edit-a", "one", 0);
    selected.trimStart = 1;
    selected.trimEnd = 2;
    const [item] = timelineItems(project, edit([selected]));
    expect(replacementRangeAnchors(item, 2.011, 6.03)).toEqual({
      firstAnchor: { editId: "edit-a", timeSeconds: 2, label: "One · replacement in" },
      lastAnchor: { editId: "edit-a", timeSeconds: 6.041666666666667, label: "One · replacement out" },
    });
  });

  it("accepts editor timecode or seconds and rejects impossible frame fields", () => {
    expect(parseGenerationTimecode("00:01:02:12")).toBe(62.5);
    expect(parseGenerationTimecode("6.25")).toBe(6.25);
    expect(parseGenerationTimecode("00:00:12:24")).toBeUndefined();
    expect(parseGenerationTimecode("00:67:00:00")).toBeUndefined();
    expect(boundedGenerationFrame(4.019, 1, 7)).toBe(4);
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

  it("can place a preserved audition before the first story edit", () => {
    const updated = insertTimelineSourceBefore(
      edit([decision("edit-a", "one", 0), decision("edit-b", "two", 1)]),
      "opening-master",
      "edit-a",
      "edit-opening",
    );
    expect(updated.clips.map((clip) => [clip.id, clip.order])).toEqual([
      ["edit-opening", 0], ["edit-a", 1], ["edit-b", 2],
    ]);
  });

  it("prefers a pinned vision model and never treats an unverified projector as visual", () => {
    const models = [
      { id: "text", supportsVision: false, mmprojPath: undefined },
      { id: "vision-a", supportsVision: true, mmprojPath: "a-mmproj.gguf" },
      { id: "vision-b", supportsVision: true, mmprojPath: "b-mmproj.gguf" },
      { id: "metadata-only", supportsVision: true, mmprojPath: undefined },
    ];
    expect(preferredFrameAnalystModelId(models, ["text", "vision-b"])).toBe("vision-b");
    expect(preferredFrameAnalystModelId(models, ["missing"])).toBe("vision-a");
    expect(preferredFrameAnalystModelId(models.slice(0, 1), ["text"])).toBe("");
  });

  it("replays every partial output channel and the exact incomplete stream reason", () => {
    const event = (sequence: number, kind: string, content: string, extra = {}): MovieGenerationAgentEvent => ({
      sequence, kind, content, requestId: "request-1", projectId: "project-1",
      modelRole: "reviewer", at: `2026-08-20T00:00:0${sequence}Z`, ...extra,
    });
    const events = mergeGenerationAgentEvents(
      [event(1, "turn-start", "Fresh-context review started")],
      [
        event(2, "reasoning", "Checking continuity."),
        event(3, "token", "Submitting the review."),
        event(4, "advanced-token", "{\"approved\":true"),
        event(5, "stream-failed", "model stream ended before its completion marker", { completionMarkerSeen: false }),
        event(4, "advanced-token", "{\"approved\":true"),
      ],
    );

    expect(events).toHaveLength(5);
    const replayed = replayGenerationAgentEvents(events);
    expect(replayed.roleStreams.reviewer.reasoning).toContain("Checking continuity.");
    expect(replayed.roleStreams.reviewer.text).toContain("Submitting the review.");
    expect(replayed.roleStreams.reviewer.toolArguments).toBe("{\"approved\":true");
    expect(replayed.roleStreams.reviewer.status).toContain("completion marker missing");
    expect(replayed.roleStreams.reviewer.failed).toBe(true);
  });

  it("does not demote a locally pending Director command during backend preparation", () => {
    expect(generationRequestIsActive(false, true)).toBe(true);
    expect(generationRequestIsActive(true, false)).toBe(true);
    expect(generationRequestIsActive(false, false)).toBe(false);
  });
});
