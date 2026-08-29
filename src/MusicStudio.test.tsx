import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { midiSecondsToTick, midiTickToSeconds, MusicMidiEditor, quantizeTick } from "./MusicMidiEditor";
import { applyTaggedLyrics, managedMuscriptorPaths, MusicStudio } from "./MusicStudio";
import { musicLyricDisplaySegmentAt, musicLyricSegmentAt, MusicLyricsProducer, wordProgress } from "./MusicLyricsProducer";
import * as api from "./api";
import type { MusicLyricsDocument, MusicMidiDocument, MusicProject, MusicSection, MusicTake } from "./types";

vi.mock("./api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./api")>();
  return {
    ...actual,
    getMusicProject: vi.fn(),
    listMusicProjects: vi.fn(async () => []),
    musicMediaUrl: vi.fn((path: string) => `http://kestrel-media.localhost/music/${encodeURIComponent(path)}`),
  };
});

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

  it("selects the timed lyric cue and computes exact word reveal progress", () => {
    const cue = { id: "cue-1", start: 2, end: 5, primary: "stay here", translation: "", words: [] };
    expect(musicLyricSegmentAt([cue], 1.99)).toBeUndefined();
    expect(musicLyricSegmentAt([cue], 2)).toBe(cue);
    expect(musicLyricSegmentAt([cue], 5)).toBe(cue);
    expect(musicLyricDisplaySegmentAt([cue], 5.3)).toBe(cue);
    expect(musicLyricDisplaySegmentAt([cue], 5.43)).toBeUndefined();
    expect(wordProgress(3, 4, 2.5)).toBe(0);
    expect(wordProgress(3, 4, 3.5)).toBe(0.5);
    expect(wordProgress(3, 4, 5)).toBe(1);
  });

  it("starts playback at the exact clicked lyric word", () => {
    const take = {
      id: "take-1",
      durationSeconds: 10,
      resolvedModel: "Music 3",
    } as MusicTake;
    const project = { id: "project-1", title: "Night signal", takes: [take] } as MusicProject;
    const lyricDocument = {
      revision: 0,
      source: "producer-timing-draft",
      language: "auto",
      theme: "sketchbook",
      updatedAt: "2026-08-29T00:00:00Z",
      showTranslation: true,
      segments: [{
        id: "cue-1",
        start: 2,
        end: 5,
        primary: "stay here",
        translation: "",
        words: [
          { value: "stay", start: 2, end: 3 },
          { value: "here", start: 3, end: 4 },
        ],
      }],
    } as MusicLyricsDocument;
    const onSeek = vi.fn();
    const onTogglePlay = vi.fn();
    const canvas = vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    render(<MusicLyricsProducer project={project} take={take} document={lyricDocument} audio={null} currentTime={2.2} playing={false} busy={false} status="" onTogglePlay={onTogglePlay} onSeek={onSeek} onChange={vi.fn()} onSave={vi.fn()} onSync={vi.fn()} onCancelSync={vi.fn()} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Play from here" }));
    expect(onSeek).toHaveBeenCalledWith(3);
    expect(onTogglePlay).toHaveBeenCalledTimes(1);
    canvas.mockRestore();
  });

  it("previews the second visual theme through the shared producer and saves it explicitly", async () => {
    const take = { id: "take-1", durationSeconds: 10, resolvedModel: "Music 3" } as MusicTake;
    const project = { id: "project-1", title: "Night signal", takes: [take] } as MusicProject;
    const document = {
      schemaVersion: 1,
      takeId: take.id,
      sourceSha256: "a".repeat(64),
      revision: 2,
      language: "English",
      source: "whisper-local",
      transcript: "stay here",
      theme: "sketchbook",
      showTranslation: true,
      createdAt: "2026-08-29T00:00:00Z",
      updatedAt: "2026-08-29T00:00:00Z",
      segments: [{ id: "cue-1", start: 2, end: 5, primary: "stay here", translation: "", words: [] }],
    } satisfies MusicLyricsDocument;
    const onChange = vi.fn();
    const onSave = vi.fn(async (next: MusicLyricsDocument) => ({
      ...next,
      revision: next.revision + 1,
      updatedAt: "2026-08-29T00:01:00Z",
    }));
    const canvas = vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    const common = { project, take, audio: null, currentTime: 2.2, playing: false, busy: false, status: "", onTogglePlay: vi.fn(), onSeek: vi.fn(), onChange, onSave, onSync: vi.fn(), onCancelSync: vi.fn(), onClose: vi.fn() };
    const view = render(<MusicLyricsProducer {...common} document={document} />);
    const producer = within(view.container);

    fireEvent.change(producer.getByRole("combobox", { name: "Lyric visual theme" }), { target: { value: "signal-bloom" } });
    const preview = onChange.mock.calls[0][0] as MusicLyricsDocument;
    expect(preview.theme).toBe("signal-bloom");
    view.rerender(<MusicLyricsProducer {...common} document={preview} />);
    expect(producer.getByRole("region", { name: "Visual lyric producer" })).toHaveClass("theme-signal-bloom");
    fireEvent.click(producer.getByRole("button", { name: "Save look" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(expect.objectContaining({ theme: "signal-bloom" })));
    view.unmount();
    canvas.mockRestore();
  });

  it("supports easy cue and per-word timestamp editing with convenient playhead set buttons", async () => {
    const take = { id: "take-1", durationSeconds: 10, resolvedModel: "Music 3" } as MusicTake;
    const project = { id: "project-1", title: "Night signal", takes: [take] } as MusicProject;
    const document = {
      schemaVersion: 1,
      takeId: take.id,
      sourceSha256: "a".repeat(64),
      revision: 1,
      language: "English",
      source: "whisper-local",
      transcript: "deep and steep",
      theme: "sketchbook",
      showTranslation: true,
      createdAt: "2026-08-29T00:00:00Z",
      updatedAt: "2026-08-29T00:00:00Z",
      segments: [{
        id: "cue-1",
        start: 1.0,
        end: 4.0,
        primary: "deep and steep",
        translation: "",
        words: [
          { value: "deep", start: 1.0, end: 1.8 },
          { value: "and", start: 1.9, end: 2.3 },
          { value: "steep", start: 2.4, end: 3.5 },
        ],
      }],
    } satisfies MusicLyricsDocument;
    const onChange = vi.fn();
    const canvas = vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    const common = {
      project,
      take,
      audio: null,
      currentTime: 1.25,
      playing: false,
      busy: false,
      status: "",
      onTogglePlay: vi.fn(),
      onSeek: vi.fn(),
      onChange,
      onSave: vi.fn(),
      onSync: vi.fn(),
      onCancelSync: vi.fn(),
      onClose: vi.fn(),
    };
    const view = render(<MusicLyricsProducer {...common} document={document} />);
    const producer = within(view.container);

    // Open timing editor
    fireEvent.click(producer.getByRole("button", { name: "Edit timing" }));
    expect(producer.getByText("Lyrics & timing")).toBeInTheDocument();

    // Cue set start button uses current playhead (1.25s)
    fireEvent.click(producer.getByTitle("Set cue start to playhead position"));
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      segments: [expect.objectContaining({
        id: "cue-1",
        start: 1.25,
      })],
    }));

    // Word set start button sets word start to current playhead (1.25s)
    const setStartBtn = producer.getByTitle('Set start of "deep" to playhead (00:01.3)');
    fireEvent.click(setStartBtn);
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      segments: [expect.objectContaining({
        words: expect.arrayContaining([
          expect.objectContaining({ value: "deep", start: 1.25 }),
        ]),
      })],
    }));

    // Add word adds a new word and keeps primary in sync
    fireEvent.click(producer.getByTitle("Add word at current playhead"));
    expect(onChange).toHaveBeenCalledWith(expect.objectContaining({
      segments: [expect.objectContaining({
        words: expect.arrayContaining([
          expect.objectContaining({ value: "word" }),
        ]),
      })],
    }));

    view.unmount();
    canvas.mockRestore();
  });

  it("supports targeted Whisper range repair with start prompt conditioning and buffer guidance", async () => {
    const take = { id: "take-1", durationSeconds: 30, resolvedModel: "Music 3", lyrics: "Deep and steep\nA silent geometry" } as MusicTake;
    const project = { id: "project-1", title: "Night signal", takes: [take] } as MusicProject;
    const document = {
      schemaVersion: 1,
      takeId: take.id,
      sourceSha256: "a".repeat(64),
      revision: 1,
      language: "English",
      source: "whisper-local",
      transcript: "deep and steep",
      theme: "sketchbook",
      showTranslation: true,
      createdAt: "2026-08-29T00:00:00Z",
      updatedAt: "2026-08-29T00:00:00Z",
      segments: [{
        id: "cue-1",
        start: 5.0,
        end: 12.0,
        primary: "deep and steep",
        translation: "",
        words: [],
      }],
    } satisfies MusicLyricsDocument;
    const onRepairRange = vi.fn();
    const canvas = vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    const common = {
      project,
      take,
      audio: null,
      currentTime: 6.5,
      playing: false,
      busy: false,
      status: "",
      onTogglePlay: vi.fn(),
      onSeek: vi.fn(),
      onChange: vi.fn(),
      onSave: vi.fn(),
      onSync: vi.fn(),
      onRepairRange,
      onCancelSync: vi.fn(),
      onClose: vi.fn(),
    };
    const view = render(<MusicLyricsProducer {...common} document={document} />);
    const producer = within(view.container);

    // Open timing editor
    fireEvent.click(producer.getByRole("button", { name: "Edit timing" }));

    // Switch to Whisper Repair tab
    fireEvent.click(producer.getByRole("button", { name: /Whisper Repair/i }));
    expect(producer.getByText("Targeted Whisper forced alignment")).toBeInTheDocument();

    // Fill prompt from take lyrics
    fireEvent.click(producer.getByTitle("Extract matching lines from generated take lyrics"));
    const promptArea = producer.getByPlaceholderText("Type or paste the exact expected sung words for this section…") as HTMLTextAreaElement;
    expect(promptArea.value.length).toBeGreaterThan(0);

    // Test Audio Copilot listening
    const onDraftAudioPrompt = vi.fn().mockResolvedValue({ transcription: "Heard sung words by audio LLM", modelName: "Gemma 12B" });
    view.rerender(<MusicLyricsProducer {...common} document={document} onDraftAudioPrompt={onDraftAudioPrompt} />);
    const copilotBtn = producer.getByRole("button", { name: /Audio Copilot/i });
    expect(copilotBtn).toBeInTheDocument();
    fireEvent.click(copilotBtn);
    await vi.waitFor(() => {
      expect(onDraftAudioPrompt).toHaveBeenCalled();
    });

    // Click Re-sync range
    const syncRangeBtn = producer.getByRole("button", { name: /Re-sync range with Whisper/i });
    expect(syncRangeBtn).toBeInTheDocument();

    view.unmount();
    canvas.mockRestore();
  });

  it("loads local music with the CORS mode required for audible Web Audio analysis", async () => {
    const take = {
      id: "take-1",
      createdAt: "2026-08-28T00:00:00Z",
      status: "complete",
      detail: "Ready",
      error: "",
      path: "C:\\Kestrel Research\\music\\project-1\\take.wav",
      bytes: 1,
      sha256: "a".repeat(64),
      durationSeconds: 10,
      seed: 42,
      resolvedModel: "Music 3",
      caption: "Ambient pop, 96 BPM",
      lyrics: "stay here",
      promptId: "prompt-1",
      exactGraph: {},
      midiPath: "",
      midiReceiptPath: "",
      midiSourcePath: "",
      midiDocumentPath: "",
      midiRevision: 0,
      lyricsDocumentPath: "",
      lyricsReceiptPath: "",
      lyricsRevision: 0,
    } satisfies MusicTake;
    const project = {
      schemaVersion: 1,
      id: "project-1",
      title: "Night signal",
      idea: "",
      caption: take.caption,
      instrumental: false,
      sections,
      settings: { maxDurationSeconds: 120, steps: 20, cfgScale: 4, topK: 50, seed: 42, tiledDecode: false, modelVariant: "auto", comfyRoot: "" },
      midi: { executablePath: "", modelPath: "", instruments: "" },
      takes: [take],
      activeTakeId: take.id,
      status: "ready",
      phase: "complete",
      detail: "Ready",
      error: "",
      createdAt: take.createdAt,
      updatedAt: take.createdAt,
    } satisfies MusicProject;
    vi.mocked(api.listMusicProjects).mockResolvedValueOnce([{
      id: project.id,
      title: project.title,
      status: project.status,
      updatedAt: project.updatedAt,
      takeCount: 1,
      activeTakePath: take.path,
    }]);
    vi.mocked(api.getMusicProject).mockResolvedValueOnce(project);

    const view = render(<MusicStudio advancedEnabled={false} models={[]} onError={vi.fn()} />);
    await waitFor(() => expect(view.container.querySelector("audio")).toBeInTheDocument());
    const player = view.container.querySelector("audio");
    expect(player).toHaveAttribute("crossorigin", "anonymous");
    expect(player?.crossOrigin).toBe("anonymous");
  });

  it("keeps source identity while producer edits become an explicit revision save", async () => {
    const onSave = vi.fn(async (document: MusicMidiDocument) => ({ ...document, revision: document.revision + 1 }));
    render(<MusicMidiEditor document={midiDocument} takeLabel="Take 1" currentTime={0} playing={false} busy={false} onTogglePlay={vi.fn()} onSeek={vi.fn()} onSave={onSave} onExport={vi.fn()} onReveal={vi.fn()} onClose={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Add MIDI track" }));
    fireEvent.click(screen.getByRole("button", { name: "Undo MIDI edit" }));
    fireEvent.click(screen.getByRole("button", { name: "Redo MIDI edit" }));
    fireEvent.click(screen.getByRole("button", { name: /Save revision/i }));
    await waitFor(() => expect(onSave).toHaveBeenCalled());
    const saved = onSave.mock.calls[0][0];
    expect(saved.sourceSha256).toBe(midiDocument.sourceSha256);
    expect(saved.revision).toBe(2);
    expect(saved.tracks).toHaveLength(2);
    expect(screen.getByText(/Revision 3 saved/i)).toBeInTheDocument();
  });

  it("provides keyboard seek and add-note controls without treating note clicks as grid insertion", () => {
    const onSeek = vi.fn();
    const view = render(<MusicMidiEditor document={midiDocument} takeLabel="Take 1" currentTime={0} playing={false} busy={false} onTogglePlay={vi.fn()} onSeek={onSeek} onSave={vi.fn()} onExport={vi.fn()} onReveal={vi.fn()} onClose={vi.fn()} />);
    const editor = within(view.container);

    fireEvent.keyDown(editor.getByRole("slider", { name: "MIDI timeline seek" }), { key: "ArrowRight" });
    expect(onSeek).toHaveBeenCalledWith(expect.any(Number));

    const originalNote = editor.getByRole("button", { name: /C4 at beat 1\.00/i });
    fireEvent.doubleClick(originalNote);
    expect(editor.getAllByRole("button", { name: /C4 at beat/i })).toHaveLength(1);

    fireEvent.keyDown(editor.getByRole("application", { name: /MIDI note grid/i }), { key: "Enter" });
    expect(editor.getAllByRole("button", { name: /C4 at beat/i })).toHaveLength(2);
    fireEvent.click(editor.getByRole("button", { name: "Add note" }));
    expect(editor.getAllByRole("button", { name: /C4 at beat/i })).toHaveLength(3);
  });
});
