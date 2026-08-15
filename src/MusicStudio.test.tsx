import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { applyTaggedLyrics, MusicStudio } from "./MusicStudio";
import type { MusicSection } from "./types";

const sections: MusicSection[] = [
  { id: "11111111-1111-4111-8111-111111111111", tag: "Verse", name: "Verse 1", bars: 8, lyrics: "old one", direction: "piano" },
  { id: "22222222-2222-4222-8222-222222222222", tag: "Chorus", name: "Chorus", bars: 8, lyrics: "old hook", direction: "wide" },
  { id: "33333333-3333-4333-8333-333333333333", tag: "Verse", name: "Verse 2", bars: 8, lyrics: "old two", direction: "bass" },
];

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
});
