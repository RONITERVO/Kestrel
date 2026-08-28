import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  assessVoiceReference,
  diagnoseAudioDecodeError,
  formatErrorMessage,
  VoiceLibraryDialog,
} from "./VoiceLibrary";
import type { VoiceLibrarySnapshot } from "./types";

const api = vi.hoisted(() => ({
  setDefault: vi.fn(),
  update: vi.fn(),
  remove: vi.fn(),
  create: vi.fn(),
}));

const processing = vi.hoisted(() => ({
  createExcerpt: vi.fn(),
}));

vi.mock("./api", async (importOriginal) => ({
  ...await importOriginal<typeof import("./api")>(),
  setDefaultVoiceProfile: api.setDefault,
  updateVoiceProfile: api.update,
  deleteVoiceProfile: api.remove,
  createVoiceProfile: api.create,
  localSpeechMediaUrl: (path: string) => `http://kestrel-speech.localhost/${path}`,
}));

vi.mock("./voiceReferenceProcessing", async (importOriginal) => ({
  ...await importOriginal<typeof import("./voiceReferenceProcessing")>(),
  createVoiceReferenceExcerpt: processing.createExcerpt,
}));

const snapshot: VoiceLibrarySnapshot = {
  defaultProfileId: "voice-default",
  profiles: [
    { id: "voice-default", name: "Chatterbox Default", language: "Auto", tags: ["Built in"], source: "built-in", consentConfirmed: true, performance: "natural", createdAt: "", updatedAt: "" },
    { id: "voice-narrator", name: "Evening Narrator", language: "English", tags: ["Warm"], source: "imported", consentConfirmed: true, performance: "restrained", referenceRelativePath: "voices/objects/a.wav", referenceSha256: "a", referenceSeconds: 12, originalFileName: "voice.wav", createdAt: "2026-01-01", updatedAt: "2026-01-01" },
  ],
};

beforeEach(() => {
  api.setDefault.mockReset().mockResolvedValue({ ...snapshot, defaultProfileId: "voice-narrator" });
  api.update.mockReset().mockResolvedValue(snapshot);
  api.remove.mockReset().mockResolvedValue({ profiles: [snapshot.profiles[0]], defaultProfileId: "voice-default" });
  api.create.mockReset().mockResolvedValue(snapshot);
  processing.createExcerpt.mockReset().mockResolvedValue({
    blob: new Blob(["trimmed"], { type: "audio/wav" }),
    durationSeconds: 20,
    originalDurationSeconds: 60,
    startSeconds: 18,
    endSeconds: 38,
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("Voice Library", () => {
  it("classifies producer reference length without accepting unsafe extremes", () => {
    const tooShort = assessVoiceReference(2.9);
    expect(tooShort.tone).toBe("bad");
    expect(tooShort.text).toContain("Too short (2.9s)");

    expect(assessVoiceReference(8).tone).toBe("good");
    expect(assessVoiceReference(20).tone).toBe("good");
    expect(assessVoiceReference(30).tone).toBe("warn");

    const tooLong = assessVoiceReference(46);
    expect(tooLong.tone).toBe("bad");
    expect(tooLong.text).toContain("Too long (46.0s)");
  });

  it("diagnoses audio decoding failures with format-specific guidance", () => {
    const m4aFile = new File(["dummy"], "speaker.m4a", { type: "audio/mp4" });
    expect(diagnoseAudioDecodeError(m4aFile)).toContain("audio codec may not be supported");

    const opusFile = new File(["dummy"], "voice.opus", { type: "audio/ogg" });
    expect(diagnoseAudioDecodeError(opusFile)).toContain("Ogg Opus or WebM Opus");

    const flacFile = new File(["dummy"], "sample.flac", { type: "audio/flac" });
    expect(diagnoseAudioDecodeError(flacFile)).toContain("FLAC file");

    const wavFile = new File(["dummy"], "recording.wav", { type: "audio/wav" });
    expect(diagnoseAudioDecodeError(wavFile)).toContain("standard PCM WAV");

    const unknownFile = new File(["dummy"], "audio.raw", { type: "" });
    expect(diagnoseAudioDecodeError(unknownFile)).toContain("WAV, MP3, FLAC, Ogg/Opus, WebM, or AAC M4A");
  });

  it("formats error messages cleanly without redundant Error prefixes", () => {
    expect(formatErrorMessage(new Error("This is a failure."))).toBe("This is a failure.");
    expect(formatErrorMessage(new Error("Error: Nested prefix"))).toBe("Nested prefix");
    expect(formatErrorMessage("Error: raw string failure")).toBe("raw string failure");
    expect(formatErrorMessage("voice library request is invalid: choose a shorter clip")).toBe("choose a shorter clip");
    expect(formatErrorMessage(null)).toBe("An unknown error occurred.");
    expect(formatErrorMessage(undefined)).toBe("An unknown error occurred.");
    expect(formatErrorMessage("")).toBe("An unknown error occurred.");
  });

  it("shows voice provenance and changes the app-wide default", async () => {
    const onSnapshot = vi.fn();
    render(<VoiceLibraryDialog snapshot={snapshot} onSnapshot={onSnapshot} onClose={() => undefined} />);

    expect(screen.getByRole("dialog", { name: "Voice Library" })).toBeInTheDocument();
    expect(screen.getByText("Evening Narrator")).toBeInTheDocument();
    expect(screen.getByLabelText("Evening Narrator reference recording")).toHaveAttribute("src", "http://kestrel-speech.localhost/voices/objects/a.wav");
    fireEvent.click(screen.getByRole("button", { name: "Use across Kestrel" }));

    await waitFor(() => expect(api.setDefault).toHaveBeenCalledWith("voice-narrator"));
    expect(onSnapshot).toHaveBeenCalledWith(expect.objectContaining({ defaultProfileId: "voice-narrator" }));
  });

  it("requires confirmation before deleting only a custom voice", async () => {
    vi.spyOn(window, "confirm").mockReturnValue(true);
    const onSnapshot = vi.fn();
    render(<VoiceLibraryDialog snapshot={snapshot} onSnapshot={onSnapshot} onClose={() => undefined} />);

    expect(screen.queryByRole("button", { name: "Delete Chatterbox Default" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Delete Evening Narrator" }));
    await waitFor(() => expect(api.remove).toHaveBeenCalledWith("voice-narrator"));
    expect(onSnapshot).toHaveBeenCalled();
  });

  it("renders format guidance helper in the custom voice creation panel", () => {
    render(<VoiceLibraryDialog snapshot={snapshot} onSnapshot={vi.fn()} onClose={() => undefined} />);
    fireEvent.click(screen.getByRole("button", { name: "Add a custom voice" }));
    expect(screen.getByText(/WAV, MP3, FLAC, Ogg\/Opus, WebM, or M4A/i)).toBeInTheDocument();
  });

  it("offers and applies a bounded continuous excerpt for a long import", async () => {
    const originalCreate = URL.createObjectURL;
    const originalRevoke = URL.revokeObjectURL;
    URL.createObjectURL = vi.fn()
      .mockReturnValueOnce("blob:source-audio")
      .mockReturnValueOnce("blob:excerpt-audio");
    URL.revokeObjectURL = vi.fn();
    const originalAudio = globalThis.Audio;

    try {
      render(<VoiceLibraryDialog snapshot={snapshot} onSnapshot={vi.fn()} onClose={() => undefined} />);
      fireEvent.click(screen.getByRole("button", { name: "Add a custom voice" }));

      const file = new File(["dummy-audio-content"], "long-recording.wav", { type: "audio/wav" });
      const input = document.querySelector('input[type="file"]') as HTMLInputElement;

      class MockAudio {
        src = "";
        duration = 60.0;
        preload = "";
        onloadedmetadata: (() => void) | null = null;
        onerror: (() => void) | null = null;
        constructor() {
          setTimeout(() => this.onloadedmetadata?.(), 10);
        }
      }
      globalThis.Audio = MockAudio as unknown as typeof Audio;

      fireEvent.change(input, { target: { files: [file] } });

      await waitFor(() => {
        expect(screen.getByText(/Too long \(60.0s\)/i)).toBeInTheDocument();
      });

      expect(screen.getByRole("button", { name: "Use 20s from playhead" })).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Find active 20s" }));
      await waitFor(() => expect(processing.createExcerpt).toHaveBeenCalledWith(file, {
        knownDurationSeconds: 60,
        startSeconds: undefined,
      }));
      expect(await screen.findByRole("status")).toHaveTextContent("one continuous 20.0-second excerpt from 0:18–0:38");
      expect(screen.getByText(/Good reference length/i)).toBeInTheDocument();
    } finally {
      globalThis.Audio = originalAudio;
      URL.createObjectURL = originalCreate;
      URL.revokeObjectURL = originalRevoke;
    }
  });
});
