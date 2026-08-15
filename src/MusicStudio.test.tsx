import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { midiSecondsToTick, midiTickToSeconds, MusicMidiEditor, quantizeTick } from "./MusicMidiEditor";
import { applyTaggedLyrics, managedMuscriptorPaths, MusicStudio } from "./MusicStudio";
import type { MusicMidiDocument, MusicSection } from "./types";

const sections: MusicSection[] = [
  { id: "11111111-1111-4111-8111-111111111111", tag: "Verse", name: "Verse 1", bars: 8, lyrics: "old one", direction: "piano" },
  { id: "22222222-2222-4222-8222-222222222222", tag: "Chorus", name: "Chorus", bars: 8, lyrics: "old hook", direction: "wide" },
  { id: "33333333-3333-4333-8333-333333333333", tag: "Verse", name: "Verse 2", bars: 8, lyrics: "old two", direction: "bass" },
];

const midiDocument: MusicMidiDocument = {
  schemaVersion: 1,
  takeId: "take-1",
  sourceSha256: "a".repeat(64),
  revision: 2,
  ticksPerQuarter: 480,
  durationTicks: 1920,
  durationSeconds: 2,
  tempos: [
    { tick: 0, microsecondsPerQuarter: 500000 },
    { tick: 960, microsecondsPerQuarter: 1000000 },
  ],
  timeSignatures: [{ tick: 0, numerator: 4, denominator: 4 }],
  tracks: [{
    id: "track-1",
    name: "Piano",
    channel: 0,
    program: 0,
    muted: false,
    notes: [{ id: "note-1", pitch: 60, startTick: 0, durationTicks: 480, velocity: 96, channel: 0 }],
  }],
};

describe("MusicStudio", () => {
  it("applies tagged model lyrics without losing stable producer section identity", () => {
    const next = applyTaggedLyrics(sections, "[Verse]\nnew one\n\n[Chorus]\nnew hook\n\n[Verse]\nnew two");
    expect(next.map((section) => section.id)).toEqual(sections.map((section) => section.id));
    expect(next.map((section) => section.lyrics)).toEqual(["new one", "new hook", "new two"]);
    expect(next[2].direction).toBe("bass");
  });

  it("keeps the producer's section plan when untagged text targets a section", () => {
    const next = applyTaggedLyrics(sections, "one untagged lyric idea");
    expect(next).toHaveLength(3);
    expect(next[0].lyrics).toBe("one untagged lyric idea");
    expect(next[1]).toEqual(sections[1]);
  });

  it("does not erase the producer arrangement when a model returns only unknown tags", () => {
    expect(applyTaggedLyrics(sections, "[Not A Real Section]\nidea")).toEqual(sections);
  });

  it("derives the managed MuScriptor runner and checkpoint without producer path editing", () => {
    expect(managedMuscriptorPaths("C:\\Kestrel AI\\")).toEqual({
      executable: "C:\\Kestrel AI\\MuScriptor\\runtime\\uvx.exe",
      model: "C:\\Kestrel AI\\MuScriptor\\models\\model.safetensors",
    });
  });

  it("keeps unmentioned producer sections while merging a partial tagged proposal", () => {
    const next = applyTaggedLyrics(sections, "[Chorus]\nreplacement hook\n\n[Bridge]\na new release");
    expect(next.slice(0, 3).map((section) => section.id)).toEqual(sections.map((section) => section.id));
    expect(next[0]).toEqual(sections[0]);
    expect(next[1]).toEqual({ ...sections[1], lyrics: "replacement hook" });
    expect(next[2]).toEqual(sections[2]);
    expect(next[3]).toMatchObject({ tag: "Bridge", lyrics: "a new release" });
  });

  it("opens with a producer-facing new-song action when the library is empty", async () => {
    render(<MusicStudio advancedEnabled models={[]} onError={vi.fn()} />);
    await waitFor(() => expect(screen.getByRole("button", { name: /New song/i })).toBeInTheDocument());
    expect(screen.getByText(/You own every section/i)).toBeInTheDocument();
  });

  it("maps playhead time across tempo changes and quantizes without negative ticks", () => {
    expect(midiTickToSeconds(960, midiDocument)).toBeCloseTo(1);
    expect(midiTickToSeconds(1440, midiDocument)).toBeCloseTo(2);
    expect(midiSecondsToTick(2, midiDocument)).toBeCloseTo(1440);
    expect(quantizeTick(358, 120)).toBe(360);
    expect(quantizeTick(-20, 120)).toBe(0);
  });

  it("keeps source identity while producer edits become an explicit revision save", async () => {
    const onSave = vi.fn(async (document: MusicMidiDocument) => ({ ...document, revision: document.revision + 1 }));
    render(<MusicMidiEditor document={midiDocument} takeLabel="Take 1" currentTime={0} playing={false} busy={false} onTogglePlay={vi.fn()} onSeek={vi.fn()} onSave={onSave} onExport={vi.fn()} onReveal={vi.fn()} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Add MIDI track" }));
    fireEvent.click(screen.getByRole("button", { name: /Save revision/i }));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    const saved = onSave.mock.calls[0][0];
    expect(saved.sourceSha256).toBe(midiDocument.sourceSha256);
    expect(saved.revision).toBe(2);
    expect(saved.tracks).toHaveLength(2);
    expect(screen.getByText(/Revision 3 saved/i)).toBeInTheDocument();
  });
});
