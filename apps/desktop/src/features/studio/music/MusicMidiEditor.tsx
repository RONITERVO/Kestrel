import {
  Check, ChevronDown, Download, Eye, FolderOpen, ListMusic, Pause, Piano,
  Play, Plus, Redo2, Save, Trash2, Undo2, Volume2, VolumeX, X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { MusicMidiDocument, MusicMidiNote, MusicMidiTrack } from "../../../contracts/index";

const NOTE_HEIGHT = 12;
const PIANO_WIDTH = 58;
const GM_PROGRAMS = `Acoustic Grand Piano|Bright Acoustic Piano|Electric Grand Piano|Honky-tonk Piano|Electric Piano 1|Electric Piano 2|Harpsichord|Clavinet|Celesta|Glockenspiel|Music Box|Vibraphone|Marimba|Xylophone|Tubular Bells|Dulcimer|Drawbar Organ|Percussive Organ|Rock Organ|Church Organ|Reed Organ|Accordion|Harmonica|Tango Accordion|Acoustic Guitar nylon|Acoustic Guitar steel|Electric Guitar jazz|Electric Guitar clean|Electric Guitar muted|Overdriven Guitar|Distortion Guitar|Guitar Harmonics|Acoustic Bass|Electric Bass finger|Electric Bass pick|Fretless Bass|Slap Bass 1|Slap Bass 2|Synth Bass 1|Synth Bass 2|Violin|Viola|Cello|Contrabass|Tremolo Strings|Pizzicato Strings|Orchestral Harp|Timpani|String Ensemble 1|String Ensemble 2|Synth Strings 1|Synth Strings 2|Choir Aahs|Voice Oohs|Synth Voice|Orchestra Hit|Trumpet|Trombone|Tuba|Muted Trumpet|French Horn|Brass Section|Synth Brass 1|Synth Brass 2|Soprano Sax|Alto Sax|Tenor Sax|Baritone Sax|Oboe|English Horn|Bassoon|Clarinet|Piccolo|Flute|Recorder|Pan Flute|Blown Bottle|Shakuhachi|Whistle|Ocarina|Lead 1 square|Lead 2 sawtooth|Lead 3 calliope|Lead 4 chiff|Lead 5 charang|Lead 6 voice|Lead 7 fifths|Lead 8 bass lead|Pad 1 new age|Pad 2 warm|Pad 3 polysynth|Pad 4 choir|Pad 5 bowed|Pad 6 metallic|Pad 7 halo|Pad 8 sweep|FX 1 rain|FX 2 soundtrack|FX 3 crystal|FX 4 atmosphere|FX 5 brightness|FX 6 goblins|FX 7 echoes|FX 8 sci-fi|Sitar|Banjo|Shamisen|Koto|Kalimba|Bag Pipe|Fiddle|Shanai|Tinkle Bell|Agogo|Steel Drums|Woodblock|Taiko Drum|Melodic Tom|Synth Drum|Reverse Cymbal|Guitar Fret Noise|Breath Noise|Seashore|Bird Tweet|Telephone Ring|Helicopter|Applause|Gunshot`.split("|");

type SnapDivision = "off" | "1/4" | "1/8" | "1/16" | "1/32";

interface DragState {
  noteId: string;
  mode: "move" | "resize";
  startX: number;
  startY: number;
  original: MusicMidiNote;
}

interface TrackHistoryEntry {
  trackId: string;
  index: number;
  track?: MusicMidiTrack;
}

export function MusicMidiEditor({
  document,
  takeLabel,
  currentTime,
  playing,
  busy,
  onTogglePlay,
  onSeek,
  onSave,
  onExport,
  onReveal,
  onClose,
}: {
  document: MusicMidiDocument;
  takeLabel: string;
  currentTime: number;
  playing: boolean;
  busy: boolean;
  onTogglePlay: () => void;
  onSeek: (seconds: number) => void;
  onSave: (document: MusicMidiDocument) => Promise<MusicMidiDocument | undefined>;
  onExport: () => Promise<string | undefined>;
  onReveal: () => Promise<void>;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState(() => cloneDocument(document));
  const [selectedTrackId, setSelectedTrackId] = useState(document.tracks[0]?.id ?? "");
  const [selectedNoteId, setSelectedNoteId] = useState("");
  const [dirty, setDirty] = useState(false);
  const [undo, setUndo] = useState<TrackHistoryEntry[]>([]);
  const [redo, setRedo] = useState<TrackHistoryEntry[]>([]);
  const [snap, setSnap] = useState<SnapDivision>("1/16");
  const [pixelsPerBeat, setPixelsPerBeat] = useState(42);
  const [drag, setDrag] = useState<DragState>();
  const [notice, setNotice] = useState("Original transcription preserved · edits create revisions");
  const viewport = useRef<HTMLDivElement>(null);
  const draftRef = useRef(draft);

  useEffect(() => {
    const next = cloneDocument(document);
    draftRef.current = next;
    setDraft(next);
    setSelectedTrackId((current) => document.tracks.some((track) => track.id === current) ? current : document.tracks[0]?.id ?? "");
    setSelectedNoteId("");
    setDirty(false);
    setUndo([]);
    setRedo([]);
  }, [document]);

  const selectedTrack = draft.tracks.find((track) => track.id === selectedTrackId) ?? draft.tracks[0];
  const selectedNote = selectedTrack?.notes.find((note) => note.id === selectedNoteId);
  const beats = Math.max(16, draft.durationTicks / Math.max(1, draft.ticksPerQuarter) + 4);
  const canvasWidth = Math.min(24_000, Math.max(900, Math.ceil(beats * pixelsPerBeat)));
  const snapTicks = midiSnapTicks(draft.ticksPerQuarter, snap);
  const playheadTick = midiSecondsToTick(currentTime, draft);
  const bpm = Math.round(60_000_000 / (draft.tempos[0]?.microsecondsPerQuarter || 500_000));
  const signature = draft.timeSignatures[0] ?? { numerator: 4, denominator: 4 };

  const edit = (change: (value: MusicMidiDocument) => MusicMidiDocument) => {
    const current = draftRef.current;
    const next = change(current);
    const historyEntry = changedTrackEntry(current, next);
    if (!historyEntry) return;
    setUndo((history) => [...history.slice(-49), historyEntry]);
    setRedo([]);
    draftRef.current = next;
    setDraft(next);
    setDirty(true);
  };

  const patchTrack = (patch: Partial<MusicMidiTrack>) => {
    if (!selectedTrack) return;
    edit((current) => ({
      ...current,
      tracks: current.tracks.map((track) => track.id === selectedTrack.id ? { ...track, ...patch } : track),
    }));
  };

  const patchNote = (patch: Partial<MusicMidiNote>) => {
    if (!selectedTrack || !selectedNote) return;
    edit((current) => updateNote(current, selectedTrack.id, selectedNote.id, (note) => ({ ...note, ...patch })));
  };

  const undoEdit = () => {
    const previous = undo.at(-1);
    if (!previous) return;
    const current = draftRef.current;
    setRedo((history) => [...history.slice(-49), trackEntry(current, previous.trackId, previous.index)]);
    setUndo((history) => history.slice(0, -1));
    const next = applyTrackEntry(current, previous);
    draftRef.current = next;
    setDraft(next);
    setDirty(true);
  };

  const redoEdit = () => {
    const next = redo.at(-1);
    if (!next) return;
    const current = draftRef.current;
    setUndo((history) => [...history.slice(-49), trackEntry(current, next.trackId, next.index)]);
    setRedo((history) => history.slice(0, -1));
    const restored = applyTrackEntry(current, next);
    draftRef.current = restored;
    setDraft(restored);
    setDirty(true);
  };

  const save = async (): Promise<MusicMidiDocument | undefined> => {
    if (!dirty) return draftRef.current;
    setNotice("Saving a new immutable MIDI revision…");
    const saved = await onSave(draftRef.current);
    if (saved) {
      const next = cloneDocument(saved);
      draftRef.current = next;
      setDraft(next);
      setDirty(false);
      setUndo([]);
      setRedo([]);
      setNotice(`Revision ${saved.revision} saved · earlier revisions remain available`);
    }
    return saved;
  };

  const exportMidi = async () => {
    if (dirty && !await save()) return;
    const path = await onExport();
    setNotice(path ? `Exported to ${path}` : "Export cancelled · project MIDI unchanged");
  };

  const finish = async () => {
    if (dirty && !await save()) return;
    onClose();
  };

  const addTrack = () => {
    const used = new Set(draft.tracks.map((track) => track.channel));
    const channel = Array.from({ length: 16 }, (_, index) => index).find((value) => !used.has(value)) ?? 0;
    const id = stableMidiId("track");
    edit((current) => ({
      ...current,
      tracks: [...current.tracks, { id, name: `New Track ${current.tracks.length + 1}`, channel, program: 0, muted: false, notes: [] }],
    }));
    setSelectedTrackId(id);
    setSelectedNoteId("");
  };

  const deleteTrack = () => {
    if (!selectedTrack || draft.tracks.length <= 1) return;
    const next = draft.tracks.find((track) => track.id !== selectedTrack.id)?.id ?? "";
    edit((current) => ({ ...current, tracks: current.tracks.filter((track) => track.id !== selectedTrack.id) }));
    setSelectedTrackId(next);
    setSelectedNoteId("");
  };

  const deleteNote = () => {
    if (!selectedTrack || !selectedNote) return;
    edit((current) => ({
      ...current,
      tracks: current.tracks.map((track) => track.id === selectedTrack.id
        ? { ...track, notes: track.notes.filter((note) => note.id !== selectedNote.id) }
        : track),
    }));
    setSelectedNoteId("");
  };

  const quantizeTrack = () => {
    if (!selectedTrack || snapTicks <= 1) return;
    edit((current) => ({
      ...current,
      tracks: current.tracks.map((track) => track.id === selectedTrack.id ? {
        ...track,
        notes: track.notes.map((note) => ({
          ...note,
          startTick: quantizeTick(note.startTick, snapTicks),
          durationTicks: Math.max(snapTicks, quantizeTick(note.durationTicks, snapTicks)),
        })),
      } : track),
    }));
    setNotice(`${selectedTrack.name} quantized to ${snap}`);
  };

  const insertNote = (tick: number, pitch: number) => {
    if (!selectedTrack || busy) return;
    const id = stableMidiId("note");
    const durationTicks = Math.max(1, snapTicks || draft.ticksPerQuarter / 4);
    edit((current) => ({
      ...current,
      tracks: current.tracks.map((track) => track.id === selectedTrack.id ? {
        ...track,
        notes: [...track.notes, { id, pitch, startTick: tick, durationTicks, velocity: 96, channel: track.channel }],
      } : track),
    }));
    setSelectedNoteId(id);
    audition(pitch, 96);
  };

  const addNote = (event: React.MouseEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget || event.detail < 2) return;
    const bounds = event.currentTarget.getBoundingClientRect();
    const tick = quantizeTick(Math.max(0, (event.clientX - bounds.left) / pixelsPerBeat * draft.ticksPerQuarter), snapTicks);
    const pitch = clamp(127 - Math.floor((event.clientY - bounds.top) / NOTE_HEIGHT), 0, 127);
    insertNote(tick, pitch);
  };

  const addNoteAtPlayhead = () => {
    insertNote(quantizeTick(playheadTick, snapTicks), selectedNote?.pitch ?? 60);
  };

  useEffect(() => {
    if (!drag || !selectedTrackId) return;
    const move = (event: PointerEvent) => {
      const tickDelta = (event.clientX - drag.startX) / pixelsPerBeat * draft.ticksPerQuarter;
      const pitchDelta = -Math.round((event.clientY - drag.startY) / NOTE_HEIGHT);
      setDraft((current) => {
        const next = updateNote(current, selectedTrackId, drag.noteId, (note) => drag.mode === "resize" ? {
          ...note,
          durationTicks: Math.max(snapTicks || 1, quantizeTick(drag.original.durationTicks + tickDelta, snapTicks)),
        } : {
          ...note,
          startTick: Math.max(0, quantizeTick(drag.original.startTick + tickDelta, snapTicks)),
          pitch: clamp(drag.original.pitch + pitchDelta, 0, 127),
        });
        draftRef.current = next;
        return next;
      });
      setDirty(true);
    };
    const up = () => setDrag(undefined);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up, { once: true });
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [draft.ticksPerQuarter, drag, pixelsPerBeat, selectedTrackId, snapTicks]);

  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "z") {
        event.preventDefault();
        if (event.shiftKey) redoEdit(); else undoEdit();
      } else if ((event.key === "Delete" || event.key === "Backspace") && selectedNoteId) {
        const target = event.target as HTMLElement | null;
        if (!target?.matches("input, textarea, select")) {
          event.preventDefault();
          deleteNote();
        }
      }
    };
    window.addEventListener("keydown", keydown);
    return () => window.removeEventListener("keydown", keydown);
  });

  useEffect(() => {
    const element = viewport.current;
    if (!element || !selectedTrack?.notes.length) return;
    const pitches = selectedTrack.notes.map((note) => note.pitch).sort((left, right) => left - right);
    const median = pitches[Math.floor(pitches.length / 2)] ?? 60;
    element.scrollTop = Math.max(0, (127 - median) * NOTE_HEIGHT - element.clientHeight / 2);
  // Recenter only when the producer selects another track.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedTrackId]);

  const beginDrag = (event: React.PointerEvent, note: MusicMidiNote, mode: DragState["mode"]) => {
    event.stopPropagation();
    setSelectedNoteId(note.id);
    setUndo((history) => [...history.slice(-49), trackEntry(draftRef.current, selectedTrackId)]);
    setRedo([]);
    setDrag({ noteId: note.id, mode, startX: event.clientX, startY: event.clientY, original: { ...note } });
    audition(note.pitch, note.velocity);
  };

  const seekToTick = (tick: number) => {
    onSeek(midiTickToSeconds(clamp(tick, 0, draft.durationTicks), draft));
  };

  const seekWithKeyboard = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const step = Math.max(1, snapTicks);
    if (event.key === "ArrowLeft") seekToTick(playheadTick - step);
    else if (event.key === "ArrowRight") seekToTick(playheadTick + step);
    else if (event.key === "Home") seekToTick(0);
    else if (event.key === "End") seekToTick(draft.durationTicks);
    else return;
    event.preventDefault();
  };

  const addNoteWithKeyboard = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget || !["Enter", " "].includes(event.key)) return;
    event.preventDefault();
    addNoteAtPlayhead();
  };

  return (
    <section className="midi-editor" aria-label="MIDI piano roll editor">
      <header className="midi-editor-toolbar">
        <div className="midi-editor-identity"><span><Piano /></span><div><small>{takeLabel}</small><strong>MIDI Piano Roll</strong></div><em>Revision {draft.revision}{dirty ? " · edited" : " · saved"}</em></div>
        <div className="midi-editor-transport">
          <button type="button" aria-label={playing ? "Pause preserved master" : "Play preserved master"} onClick={onTogglePlay}>{playing ? <Pause /> : <Play />}</button>
          <button type="button" disabled={!undo.length || busy} aria-label="Undo MIDI edit" onClick={undoEdit}><Undo2 /></button>
          <button type="button" disabled={!redo.length || busy} aria-label="Redo MIDI edit" onClick={redoEdit}><Redo2 /></button>
          <label>Snap<select value={snap} onChange={(event) => setSnap(event.target.value as SnapDivision)}><option value="off">Off</option><option value="1/4">1/4</option><option value="1/8">1/8</option><option value="1/16">1/16</option><option value="1/32">1/32</option></select></label>
          <button type="button" disabled={!selectedTrack || snap === "off" || busy} onClick={quantizeTrack}><Check /> Quantize track</button>
          <button type="button" disabled={!selectedTrack || busy} onClick={addNoteAtPlayhead}><Plus /> Add note</button>
          <label>Zoom<input aria-label="MIDI horizontal zoom" type="range" min={24} max={84} value={pixelsPerBeat} onChange={(event) => setPixelsPerBeat(Number(event.target.value))} /></label>
        </div>
        <div className="midi-editor-actions">
          <button type="button" disabled={busy} onClick={() => void onReveal()}><FolderOpen /> Reveal</button>
          <button type="button" disabled={busy} onClick={() => void exportMidi()}><Download /> Export MIDI</button>
          <button type="button" className="midi-save" disabled={!dirty || busy} onClick={() => void save()}><Save /> Save revision</button>
          <button type="button" className="midi-done" disabled={busy} onClick={() => void finish()}><X /> Done</button>
        </div>
      </header>

      <aside className="midi-track-list">
        <header><span><ListMusic /><strong>Tracks</strong></span><div><button type="button" aria-label="Add MIDI track" disabled={busy} onClick={addTrack}><Plus /></button><button type="button" aria-label="Remove selected MIDI track" disabled={busy || draft.tracks.length <= 1} onClick={deleteTrack}><Trash2 /></button></div></header>
        <div>{draft.tracks.map((track, index) => <article key={track.id} className={track.id === selectedTrack?.id ? "active" : ""}><button type="button" className="midi-track-choice" onClick={() => { setSelectedTrackId(track.id); setSelectedNoteId(""); }}><i style={{ background: trackColor(index) }} /><span><strong>{track.name}</strong><small>{GM_PROGRAMS[track.program]} · Ch {track.channel + 1} · {track.notes.length} notes</small></span></button><button type="button" className="midi-track-mute" aria-label={`${track.muted ? "Unmute" : "Mute"} ${track.name}`} onClick={() => { setSelectedTrackId(track.id); edit((current) => ({ ...current, tracks: current.tracks.map((item) => item.id === track.id ? { ...item, muted: !item.muted } : item) })); }}>{track.muted ? <VolumeX /> : <Volume2 />}</button></article>)}</div>
        <footer><span>{draft.tracks.length} tracks</span><span>{draft.tracks.reduce((sum, track) => sum + track.notes.length, 0).toLocaleString()} notes</span></footer>
      </aside>

      <main className="midi-roll-pane">
        <div className="midi-roll-status"><span>{bpm} BPM · {signature.numerator}/{signature.denominator} · {draft.ticksPerQuarter} PPQ</span><span>{notice}</span><span>Double-click grid to add · drag notes to move · drag right edge to resize</span></div>
        <div ref={viewport} className="midi-roll-viewport">
          <div className="midi-roll-content" style={{ width: canvasWidth + PIANO_WIDTH, height: 128 * NOTE_HEIGHT + 27 }}>
            <div className="midi-ruler" role="slider" tabIndex={0} aria-label="MIDI timeline seek" aria-valuemin={0} aria-valuemax={draft.durationSeconds} aria-valuenow={clamp(currentTime, 0, draft.durationSeconds)} onKeyDown={seekWithKeyboard} style={{ left: PIANO_WIDTH, width: canvasWidth, backgroundSize: `${pixelsPerBeat * 4}px 100%` }} onClick={(event) => {
              const bounds = event.currentTarget.getBoundingClientRect();
              const tick = Math.max(0, (event.clientX - bounds.left) / pixelsPerBeat * draft.ticksPerQuarter);
              onSeek(midiTickToSeconds(tick, draft));
            }}>{Array.from({ length: Math.ceil(beats / 4) }, (_, index) => <span key={index} style={{ left: index * pixelsPerBeat * 4 }}>{index + 1}</span>)}</div>
            <div className="midi-keyboard">{Array.from({ length: 128 }, (_, index) => { const pitch = 127 - index; return <span key={pitch} className={isBlackKey(pitch) ? "black" : "white"}>{pitch % 12 === 0 ? noteName(pitch) : ""}</span>; })}</div>
            <div className="midi-note-grid" role="application" tabIndex={0} aria-label="MIDI note grid. Press Enter to add a note at the playhead." aria-disabled={busy} onKeyDown={addNoteWithKeyboard} style={{ left: PIANO_WIDTH, top: 27, width: canvasWidth, height: 128 * NOTE_HEIGHT, backgroundSize: `${pixelsPerBeat}px ${NOTE_HEIGHT}px` }} onClick={addNote}>
              {selectedTrack?.notes.map((note) => <button type="button" key={note.id} aria-label={`${noteName(note.pitch)} at beat ${(note.startTick / draft.ticksPerQuarter + 1).toFixed(2)}`} className={`midi-note ${note.id === selectedNote?.id ? "selected" : ""}`} style={{ left: note.startTick / draft.ticksPerQuarter * pixelsPerBeat, top: (127 - note.pitch) * NOTE_HEIGHT + 1, width: Math.max(5, note.durationTicks / draft.ticksPerQuarter * pixelsPerBeat), height: NOTE_HEIGHT - 2, background: trackColor(draft.tracks.findIndex((track) => track.id === selectedTrack.id)) }} onPointerDown={(event) => beginDrag(event, note, "move")} onDoubleClick={(event) => { event.stopPropagation(); audition(note.pitch, note.velocity); }}><span onPointerDown={(event) => beginDrag(event, note, "resize")} /></button>)}
              <i className="midi-playhead" style={{ left: playheadTick / draft.ticksPerQuarter * pixelsPerBeat }} />
            </div>
          </div>
        </div>
      </main>

      <aside className="midi-event-inspector">
        <header><span><Eye /><strong>{selectedNote ? "Note inspector" : "Track inspector"}</strong></span><ChevronDown /></header>
        {selectedTrack && <fieldset disabled={busy}>
          <label>Track name<input maxLength={256} value={selectedTrack.name} onChange={(event) => patchTrack({ name: event.target.value })} /></label>
          <label>Instrument<select value={selectedTrack.program} onChange={(event) => patchTrack({ program: Number(event.target.value) })}>{GM_PROGRAMS.map((name, index) => <option key={name} value={index}>{index + 1}. {name}</option>)}</select></label>
          <div className="midi-inspector-pair"><label>Channel<input type="number" min={1} max={16} value={selectedTrack.channel + 1} onChange={(event) => { const channel = event.currentTarget.valueAsNumber - 1; if (Number.isFinite(channel) && channel >= 0 && channel <= 15) patchTrack({ channel, notes: selectedTrack.notes.map((note) => ({ ...note, channel })) }); }} /></label><label>Output<button type="button" className={selectedTrack.muted ? "muted" : ""} onClick={() => patchTrack({ muted: !selectedTrack.muted })}>{selectedTrack.muted ? <VolumeX /> : <Volume2 />}{selectedTrack.muted ? "Muted" : "Included"}</button></label></div>
          {selectedNote ? <>
            <hr />
            <div className="midi-note-name"><Piano /><span><strong>{noteName(selectedNote.pitch)}</strong><small>MIDI {selectedNote.pitch}</small></span><button type="button" aria-label="Delete selected MIDI note" onClick={deleteNote}><Trash2 /></button></div>
            <div className="midi-inspector-pair"><label>Pitch<input type="number" min={0} max={127} value={selectedNote.pitch} onChange={(event) => finiteMidiValue(event.currentTarget.valueAsNumber, 0, 127, (pitch) => { patchNote({ pitch }); audition(pitch, selectedNote.velocity); })} /></label><label>Velocity<input type="number" min={1} max={127} value={selectedNote.velocity} onChange={(event) => finiteMidiValue(event.currentTarget.valueAsNumber, 1, 127, (velocity) => patchNote({ velocity }))} /></label></div>
            <div className="midi-inspector-pair"><label>Start beat<input type="number" min={1} step={.25} value={round(selectedNote.startTick / draft.ticksPerQuarter + 1)} onChange={(event) => { const beat = event.currentTarget.valueAsNumber; if (Number.isFinite(beat) && beat >= 1) patchNote({ startTick: Math.round((beat - 1) * draft.ticksPerQuarter) }); }} /></label><label>Length beats<input type="number" min={.03125} step={.25} value={round(selectedNote.durationTicks / draft.ticksPerQuarter)} onChange={(event) => { const length = event.currentTarget.valueAsNumber; if (Number.isFinite(length) && length > 0) patchNote({ durationTicks: Math.max(1, Math.round(length * draft.ticksPerQuarter)) }); }} /></label></div>
          </> : <div className="midi-inspector-empty"><Piano /><span>Select a note to edit exact pitch, timing, length, and velocity.</span></div>}
        </fieldset>}
      </aside>
    </section>
  );
}

export function midiTickToSeconds(tick: number, document: MusicMidiDocument): number {
  const tempos = [...document.tempos].sort((left, right) => left.tick - right.tick);
  let seconds = 0;
  let previousTick = 0;
  let tempo = 500_000;
  for (const change of tempos) {
    if (change.tick > tick) break;
    seconds += (change.tick - previousTick) * tempo / document.ticksPerQuarter / 1_000_000;
    previousTick = change.tick;
    tempo = change.microsecondsPerQuarter;
  }
  return seconds + Math.max(0, tick - previousTick) * tempo / document.ticksPerQuarter / 1_000_000;
}

export function midiSecondsToTick(seconds: number, document: MusicMidiDocument): number {
  const tempos = [...document.tempos].sort((left, right) => left.tick - right.tick);
  let remaining = Math.max(0, seconds);
  let previousTick = 0;
  let tempo = 500_000;
  for (const change of tempos) {
    const segmentSeconds = (change.tick - previousTick) * tempo / document.ticksPerQuarter / 1_000_000;
    if (remaining < segmentSeconds) break;
    remaining -= segmentSeconds;
    previousTick = change.tick;
    tempo = change.microsecondsPerQuarter;
  }
  return previousTick + remaining * document.ticksPerQuarter * 1_000_000 / tempo;
}

export function quantizeTick(value: number, division: number): number {
  if (!Number.isFinite(value)) return 0;
  return division > 1 ? Math.max(0, Math.round(value / division) * division) : Math.max(0, Math.round(value));
}

function midiSnapTicks(ticksPerQuarter: number, snap: SnapDivision): number {
  if (snap === "off") return 1;
  const denominator = Number(snap.split("/")[1]);
  return Math.max(1, Math.round(ticksPerQuarter * 4 / denominator));
}

function updateNote(document: MusicMidiDocument, trackId: string, noteId: string, change: (note: MusicMidiNote) => MusicMidiNote): MusicMidiDocument {
  return { ...document, tracks: document.tracks.map((track) => track.id === trackId ? { ...track, notes: track.notes.map((note) => note.id === noteId ? change(note) : note) } : track) };
}

function cloneDocument(document: MusicMidiDocument): MusicMidiDocument {
  return structuredClone(document);
}

function trackEntry(document: MusicMidiDocument, trackId: string, fallbackIndex = document.tracks.length): TrackHistoryEntry {
  const index = document.tracks.findIndex((track) => track.id === trackId);
  return {
    trackId,
    index: index >= 0 ? index : fallbackIndex,
    track: index >= 0 ? structuredClone(document.tracks[index]) : undefined,
  };
}

function changedTrackEntry(before: MusicMidiDocument, after: MusicMidiDocument): TrackHistoryEntry | undefined {
  const ids = new Set([...before.tracks.map((track) => track.id), ...after.tracks.map((track) => track.id)]);
  for (const trackId of ids) {
    const beforeTrack = before.tracks.find((track) => track.id === trackId);
    const afterTrack = after.tracks.find((track) => track.id === trackId);
    if (beforeTrack !== afterTrack) {
      const afterIndex = after.tracks.findIndex((track) => track.id === trackId);
      return trackEntry(before, trackId, Math.max(0, afterIndex));
    }
  }
  return undefined;
}

function applyTrackEntry(document: MusicMidiDocument, entry: TrackHistoryEntry): MusicMidiDocument {
  const tracks = document.tracks.filter((track) => track.id !== entry.trackId);
  if (entry.track) tracks.splice(Math.min(entry.index, tracks.length), 0, structuredClone(entry.track));
  return { ...document, tracks };
}

function stableMidiId(prefix: string): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return `${prefix}-${crypto.randomUUID()}`;
  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function finiteMidiValue(value: number, minimum: number, maximum: number, apply: (value: number) => void) {
  if (Number.isFinite(value) && value >= minimum && value <= maximum) apply(Math.round(value));
}

function noteName(pitch: number): string {
  const names = ["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"];
  return `${names[pitch % 12]}${Math.floor(pitch / 12) - 1}`;
}

function isBlackKey(pitch: number): boolean {
  return [1, 3, 6, 8, 10].includes(pitch % 12);
}

function trackColor(index: number): string {
  return ["#e6a33c", "#4ca4cf", "#9b73c8", "#68ae78", "#d66b63", "#d8c35d", "#5fb8ad", "#c47ca5"][Math.max(0, index) % 8];
}

function audition(pitch: number, velocity: number) {
  const AudioContextClass = window.AudioContext || (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!AudioContextClass) return;
  const context = new AudioContextClass();
  const oscillator = context.createOscillator();
  const gain = context.createGain();
  oscillator.type = "triangle";
  oscillator.frequency.value = 440 * 2 ** ((pitch - 69) / 12);
  gain.gain.setValueAtTime(0.0001, context.currentTime);
  gain.gain.exponentialRampToValueAtTime(Math.max(.015, velocity / 127 * .12), context.currentTime + .012);
  gain.gain.exponentialRampToValueAtTime(.0001, context.currentTime + .32);
  oscillator.connect(gain).connect(context.destination);
  oscillator.start();
  oscillator.stop(context.currentTime + .34);
  oscillator.addEventListener("ended", () => void context.close(), { once: true });
}

function round(value: number): number {
  return Math.round(value * 1000) / 1000;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value));
}
