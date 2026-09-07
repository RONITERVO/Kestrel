import { createElement } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MovieTimeline, appendTimelineSource, formatTimecode, moveTimelineItem, splitTimelineItem, timelineItems } from "./MovieTimeline";
import type { ClipEdit, MovieEdit, MovieProject } from "../../../contracts/index";

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

afterEach(cleanup);

describe("movie timeline decisions", () => {
  it("lets the producer undo a completed native replacement while keeping both masters", () => {
    const edit = movieEdit([decision("a", "one", 0)]);
    const current: MovieProject = { ...project, title: "Movie", references: [], settings: {
      width: 768, height: 448, clipSeconds: 5, steps: 20, maxClips: 4096, seed: 0,
      temperature: .45, topP: .9, topK: 20, thinkingBudget: 32768,
      maxOutputTokens: 32768, comfyRoot: "", refImageSize: "match",
    }, edit };
    const onChange = vi.fn();
    const view = render(createElement(MovieTimeline, { project: current, value: edit, disabled: false, onChange }));
    expect(screen.getByRole("button", { name: "Undo timeline change" })).toBeDisabled();
    const replacement = movieEdit([decision("generated", "two", 0)]);
    view.rerender(createElement(MovieTimeline, { project: { ...current, edit: replacement }, value: replacement, disabled: false, onChange }));
    fireEvent.click(screen.getByRole("button", { name: "Undo timeline change" }));
    expect(onChange).toHaveBeenLastCalledWith(edit);
    expect(current.clips.map((clip) => clip.id)).toEqual(["one", "two"]);
  });

  it("keeps a two-hour timeline bounded while exposing later masters and track edits", () => {
    const clips = Array.from({ length: 1440 }, (_, index) => ({ ...project.clips[1], id: `clip-${index}`, title: `Scene ${index + 1}`, index }));
    const edit = movieEdit(clips.map((clip, index) => decision(`edit-${index}`, clip.id, index)));
    const longProject: MovieProject = { ...project, title: "Two hours", clips, references: [], settings: {
      width: 768, height: 448, clipSeconds: 5, steps: 20, maxClips: 4096, seed: 0,
      temperature: .45, topP: .9, topK: 20, thinkingBudget: 32768,
      maxOutputTokens: 32768, comfyRoot: "", refImageSize: "match",
    }, edit };
    const { container } = render(createElement(MovieTimeline, { project: longProject, value: edit, disabled: false, onChange: vi.fn() }));
    expect(container.querySelectorAll(".editor-media-row")).toHaveLength(80);
    expect(container.querySelectorAll(".picture .editor-track-canvas > button").length).toBeLessThan(20);
    expect(container.querySelectorAll(".editor-ruler > span").length).toBeLessThan(40);
    fireEvent.click(screen.getByRole("button", { name: "Next masters" }));
    expect(container.querySelector(".editor-media-row strong")).toHaveTextContent("Scene 81");
    const tracks = screen.getByLabelText("Timeline tracks");
    fireEvent.scroll(tracks, { target: { scrollLeft: 720 * 5 * 68 } });
    expect(container.querySelectorAll(".picture .editor-track-canvas > button").length).toBeLessThan(20);
    expect(container.querySelector(".picture .editor-track-canvas")).toHaveTextContent("Scene 721");
    expect(edit.clips).toHaveLength(1440);
  });

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

  it("keeps only the leading and trailing fades when a clip is split", () => {
    const original = decision("a", "one", 0);
    Object.assign(original, { fadeIn: 1, fadeOut: 2, audioFadeIn: .5, audioFadeOut: 1.5 });
    const split = splitTimelineItem(project, movieEdit([original]), "a", 4, "cut-b");
    expect(split.clips[0]).toMatchObject({ fadeIn: 1, fadeOut: 0, audioFadeIn: .5, audioFadeOut: 0 });
    expect(split.clips[1]).toMatchObject({ fadeIn: 0, fadeOut: 2, audioFadeIn: 0, audioFadeOut: 1.5 });
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
