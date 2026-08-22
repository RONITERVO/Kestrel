import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { assessVoiceReference, VoiceLibraryDialog } from "./VoiceLibrary";
import type { VoiceLibrarySnapshot } from "./types";

const api = vi.hoisted(() => ({
  setDefault: vi.fn(),
  update: vi.fn(),
  remove: vi.fn(),
  create: vi.fn(),
}));

vi.mock("./api", async (importOriginal) => ({
  ...await importOriginal<typeof import("./api")>(),
  setDefaultVoiceProfile: api.setDefault,
  updateVoiceProfile: api.update,
  deleteVoiceProfile: api.remove,
  createVoiceProfile: api.create,
  localSpeechMediaUrl: (path: string) => `http://kestrel-speech.localhost/${path}`,
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
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Voice Library", () => {
  it("classifies producer reference length without accepting unsafe extremes", () => {
    expect(assessVoiceReference(2.9).tone).toBe("bad");
    expect(assessVoiceReference(8).tone).toBe("good");
    expect(assessVoiceReference(20).tone).toBe("good");
    expect(assessVoiceReference(30).tone).toBe("warn");
    expect(assessVoiceReference(46).tone).toBe("bad");
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
});
