import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { demoSnapshot } from "./demo";
import { mergeAttachments, terminalTaskStatus } from "./OfflineWorkspace";
import type { ContextAttachment } from "./types";

const profileApi = vi.hoisted(() => ({
  exportSetupProfile: vi.fn(),
  exportSetupProfileText: vi.fn(),
  getSetupProfileText: vi.fn(),
  importSetupProfile: vi.fn(),
  importSetupProfileText: vi.fn(),
}));

const memoryApi = vi.hoisted(() => ({
  previewVramCleanup: vi.fn(),
  cleanVram: vi.fn(),
  releaseAiMemory: vi.fn(),
}));

vi.mock("./api", async (importOriginal) => ({
  ...await importOriginal<typeof import("./api")>(),
  exportSetupProfile: profileApi.exportSetupProfile,
  exportSetupProfileText: profileApi.exportSetupProfileText,
  getSetupProfileText: profileApi.getSetupProfileText,
  importSetupProfile: profileApi.importSetupProfile,
  importSetupProfileText: profileApi.importSetupProfileText,
  previewVramCleanup: memoryApi.previewVramCleanup,
  cleanVram: memoryApi.cleanVram,
  releaseAiMemory: memoryApi.releaseAiMemory,
}));

beforeEach(() => {
  profileApi.exportSetupProfile.mockReset().mockResolvedValue({ path: "C:\\Research\\portable.json", message: "Safe profile exported." });
  profileApi.exportSetupProfileText.mockReset().mockResolvedValue({ path: "C:\\Research\\portable.json", message: "Validated profile exported." });
  profileApi.getSetupProfileText.mockReset().mockResolvedValue(JSON.stringify({ schemaVersion: 1, app: "Kestrel" }, null, 2));
  profileApi.importSetupProfile.mockReset().mockResolvedValue(demoSnapshot);
  profileApi.importSetupProfileText.mockReset().mockResolvedValue(demoSnapshot);
  memoryApi.previewVramCleanup.mockReset().mockResolvedValue({
    gpu: demoSnapshot.control.gpu,
    candidates: [],
    exclusions: [],
    candidateMemoryMib: 0,
    protectedProcessCount: 0,
  });
  memoryApi.cleanVram.mockReset().mockResolvedValue({
    attempted: [],
    terminated: [],
    failed: [],
    beforeGpu: demoSnapshot.control.gpu,
    afterGpu: demoSnapshot.control.gpu,
    freedMib: 0,
    message: "VRAM is ready.",
  });
  memoryApi.releaseAiMemory.mockReset().mockResolvedValue(demoSnapshot.control);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn().mockResolvedValue(undefined) } });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Kestrel research experience", () => {
  it("keeps composer attachments within native count and byte limits", () => {
    const attachment = (id: string, bytes: number): ContextAttachment => ({
      id,
      name: `${id}.bin`,
      kind: "binary",
      mimeType: "application/octet-stream",
      bytes,
      sha256: id.padEnd(64, "0"),
      storedPath: `${id}.bin`,
      extractedChars: 0,
      contextMode: "metadata_only",
      note: "test",
      createdAt: new Date(0).toISOString(),
    });
    const mib = 1024 * 1024;
    const overBytes = mergeAttachments([], [
      attachment("one", 128 * mib),
      attachment("two", 128 * mib),
      attachment("three", 1),
    ]);
    expect(overBytes.attachments.map((item) => item.id)).toEqual(["one", "two"]);
    expect(overBytes.rejected).toBe(1);

    const overCount = mergeAttachments(
      [],
      Array.from({ length: 13 }, (_, index) => attachment(`item-${index}`, 1)),
    );
    expect(overCount.attachments).toHaveLength(12);
    expect(overCount.rejected).toBe(1);
  });

  it("opens the durable library and renders evidence-oriented research", async () => {
    render(<App />);
    expect(await screen.findByRole("heading", { name: "The Antikythera mechanism" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Key findings" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Sources inspected" })).toBeInTheDocument();
    expect(screen.getByText(/Produced entirely on this computer/)).toBeInTheDocument();
  });

  it("explains offline scope before a new research run", async () => {
    render(<App />);
    const buttons = await screen.findAllByRole("button", { name: /New research/i });
    fireEvent.click(buttons[0]);
    expect(screen.getByRole("dialog", { name: "What would you like to understand?" })).toBeInTheDocument();
    expect(screen.getByText(/No web requests/)).toBeInTheDocument();
    const begin = screen.getByRole("button", { name: /Begin research/ });
    expect(begin).toBeDisabled();
    fireEvent.change(screen.getByPlaceholderText(/Ask a question/), { target: { value: "How did Roman concrete work?" } });
    expect(begin).toBeEnabled();
  });

  it("keeps system and advanced controls one step from research", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^System$/i }));
    expect(await screen.findByRole("heading", { name: "System" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "App-wide local model policy" })).toBeInTheDocument();
    expect(screen.getByText(/No GPU model is assumed/i)).toBeInTheDocument();
    expect(screen.getByText(/Per-model exception/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Research policy/i }));
    expect(screen.getByText(/One selected model, one inference lease/i)).toBeInTheDocument();
    expect(screen.getByText(/never launches or attaches to a separate model-specific server/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Portable setup/i }));
    expect(screen.getByRole("heading", { name: "Portable setup JSON" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Export edited JSON/i })).toBeInTheDocument();
    expect(screen.getByText(/excludes weights, projects, conversations, developer paths, credentials, and access grants/i)).toBeInTheDocument();
    expect((await screen.findByLabelText("Editable portable setup JSON") as HTMLTextAreaElement).value).toContain('"schemaVersion": 1');
    fireEvent.click(screen.getByRole("button", { name: /^Research$/i }));
    expect(await screen.findByText("Your research")).toBeInTheDocument();
  });

  it("puts safe VRAM cleanup and Kestrel memory release in the shared header", async () => {
    const competing = {
      pid: 9123,
      name: "ollama.exe",
      executablePath: "C:\\Other AI\\ollama.exe",
      memoryMib: 4096,
      kind: "AI / compute",
    };
    const keptOpen = {
      pid: 9222,
      name: "Photos.exe",
      executablePath: "C:\\Program Files\\WindowsApps\\Photos.exe",
      memoryMib: 0,
      kind: "GPU application",
    };
    const advancedApp = {
      pid: 9333,
      name: "chrome.exe",
      executablePath: "C:\\Program Files\\Google\\Chrome\\chrome.exe",
      memoryMib: 512,
      kind: "GPU application",
    };
    const criticalProcess = {
      pid: 9444,
      name: "dwm.exe",
      executablePath: "C:\\Windows\\System32\\dwm.exe",
      memoryMib: 0,
      kind: "GPU application",
    };
    memoryApi.previewVramCleanup.mockResolvedValue({
      gpu: demoSnapshot.control.gpu,
      candidates: [competing, keptOpen],
      exclusions: [
        { process: advancedApp, reason: "Excluded by default to protect everyday apps and producer workspaces.", canInclude: true },
        { process: criticalProcess, reason: "This is a Windows system process.", canInclude: false },
      ],
      candidateMemoryMib: competing.memoryMib + keptOpen.memoryMib,
      protectedProcessCount: 2,
    });
    memoryApi.cleanVram.mockResolvedValue({
      attempted: [competing, advancedApp],
      terminated: [competing, advancedApp],
      failed: [],
      beforeGpu: demoSnapshot.control.gpu,
      afterGpu: { ...demoSnapshot.control.gpu!, usedMib: 7032, freeMib: 4912 },
      freedMib: 4096,
      message: "Closed 2 competing GPU processes.",
    });
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: "Clean VRAM" }));
    expect(await screen.findByRole("dialog", { name: "Choose what Clean VRAM closes" })).toBeInTheDocument();
    expect(screen.getByLabelText("Clean ollama.exe PID 9123")).toBeChecked();
    fireEvent.click(screen.getByLabelText("Clean Photos.exe PID 9222"));
    expect(screen.getByLabelText("Clean Photos.exe PID 9222")).not.toBeChecked();

    fireEvent.click(screen.getByRole("button", { name: /Advanced exclusions/i }));
    expect(screen.getByLabelText("Include chrome.exe PID 9333")).not.toBeChecked();
    fireEvent.click(screen.getByLabelText("Include chrome.exe PID 9333"));
    expect(screen.getByLabelText("Always protect dwm.exe PID 9444")).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Clean 2" }));
    await waitFor(() => expect(memoryApi.cleanVram).toHaveBeenCalledWith([competing.pid, advancedApp.pid]));
    expect(await screen.findByText("Closed 2 competing GPU processes.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByRole("dialog", { name: "Choose what Clean VRAM closes" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByLabelText("GPU memory options"));
    fireEvent.click(screen.getByRole("menuitem", { name: /Release Kestrel AI memory/i }));
    await waitFor(() => expect(memoryApi.releaseAiMemory).toHaveBeenCalledTimes(1));
    expect(confirm).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("button", { name: /^System$/i }));
    await screen.findByRole("heading", { name: "System" });
    expect(document.querySelector(".system-hero-actions")?.textContent).toContain("Refresh");
    expect(document.querySelector(".system-hero-actions")?.textContent).not.toContain("Release");
    expect(document.querySelector(".release-memory")).not.toBeInTheDocument();
  });

  it("keeps every section in the shared one-window workspace frame", async () => {
    render(<App />);
    expect(await screen.findByRole("heading", { name: "The Antikythera mechanism" })).toBeInTheDocument();
    const shell = document.querySelector(".app-shell");
    const workspace = document.querySelector(".workspace");
    const stage = document.querySelector(".main-stage");
    expect(shell).toHaveAttribute("data-view", "research");

    for (const [label, view] of [
      ["Setup", "setup"],
      ["Control", "control"],
      ["Research", "research"],
      ["Studio", "studio"],
      ["Music", "music"],
      ["Developer", "developer"],
      ["System", "system"],
    ] as const) {
      const button = screen.getByRole("button", { name: label });
      fireEvent.click(button);
      await waitFor(() => expect(shell).toHaveAttribute("data-view", view));
      expect(workspace).toHaveClass(`workspace-${view}`);
      expect(stage).toHaveClass(`main-stage-${view}`);
      expect(button).toHaveAttribute("aria-current", "page");
    }
  });

  it("retains an opened workspace and its local state while another app tab is visible", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^System$/i }));
    fireEvent.click(screen.getByRole("button", { name: /Portable setup/i }));
    const editor = await screen.findByLabelText("Editable portable setup JSON");
    await waitFor(() => expect((editor as HTMLTextAreaElement).value).toContain('"schemaVersion": 1'));
    fireEvent.change(editor, { target: { value: "producer draft retained across tabs" } });
    await waitFor(() => expect(editor).toHaveValue("producer draft retained across tabs"));

    fireEvent.click(screen.getByRole("button", { name: /^Research$/i }));
    expect(await screen.findByText("Your research")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^System$/i }));

    expect(screen.getByLabelText("Editable portable setup JSON")).toBe(editor);
    expect(editor).toHaveValue("producer draft retained across tabs");
  });

  it("displays the validated portable export beside its editable text", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^System$/i }));
    fireEvent.click(screen.getByRole("button", { name: /Portable setup/i }));
    fireEvent.click(await screen.findByRole("button", { name: /Export edited JSON/i }));

    expect(await screen.findByDisplayValue("C:\\Research\\portable.json")).toBeInTheDocument();
    expect(await screen.findByText(/Validated profile exported/i)).toBeInTheDocument();
    expect(profileApi.exportSetupProfileText).toHaveBeenCalledWith(expect.stringContaining('"schemaVersion": 1'));
  });

  it("requires confirmation and adopts the imported snapshot", async () => {
    const imported = { ...demoSnapshot, status: { ...demoSnapshot.status, archive: "Imported offline archive" } };
    profileApi.importSetupProfile.mockResolvedValue(imported);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^System$/i }));
    fireEvent.click(screen.getByRole("button", { name: /Portable setup/i }));
    fireEvent.change(await screen.findByLabelText("Import an existing profile path"), { target: { value: "C:\\Research\\portable.json" } });
    fireEvent.click(screen.getByRole("button", { name: /Import file/i }));
    expect(profileApi.importSetupProfile).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: /Import file/i }));
    await waitFor(() => expect(profileApi.importSetupProfile).toHaveBeenCalledWith("C:\\Research\\portable.json"));
    expect(await screen.findByText("Imported offline archive")).toBeInTheDocument();
    expect(screen.getByText(/Profile imported.*trust grants left unchanged/i)).toBeInTheDocument();
  });

  it("surfaces profile API errors", async () => {
    profileApi.exportSetupProfileText.mockRejectedValue(new Error("profile storage is read-only"));
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^System$/i }));
    fireEvent.click(screen.getByRole("button", { name: /Portable setup/i }));
    fireEvent.click(await screen.findByRole("button", { name: /Export edited JSON/i }));
    expect(await screen.findByText(/profile storage is read-only/i)).toBeInTheDocument();
  });

  it("keeps the historical control plane and optional developer repair discoverable", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^Control$/i }));
    expect(await screen.findByRole("heading", { name: /Ternary Bonsai/i })).toBeInTheDocument();
    expect(screen.getByText("SESSION INSPECTOR")).toBeInTheDocument();
    expect(screen.getByText(/one inference lease/i)).toBeInTheDocument();
    expect(screen.getByText(/private, persistent workspace/i)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Chat generation" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Save complete profile/i })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Observed model downloader" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Attach local context/i })).toBeInTheDocument();
    expect(screen.getByText("Vision")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Add local model folder/i })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Computer/i }));
    expect(screen.getByRole("heading", { name: /bounded objective/i })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Computer Tasks policy" })).toBeInTheDocument();
    expect(screen.getByText(/Every decision, tool call, result, error, and artifact/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Attach files as context/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Start visible task/i })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: /New chat/i }));
    expect(screen.getByRole("button", { name: /^Chat$/i })).toHaveClass("active");

    fireEvent.click(screen.getByRole("button", { name: /^Developer$/i }));
    expect(await screen.findByRole("heading", { name: "Developer" })).toBeInTheDocument();
    expect(screen.getByText(/Offline independence/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Run offline diagnostics/i })).toBeInTheDocument();
  });

  it("rejects a manually entered non-llama engine", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^Setup$/i }));
    fireEvent.click(screen.getByRole("button", { name: /Use existing files/i }));
    expect(await screen.findByRole("heading", { name: "Use files already on this PC" })).toBeInTheDocument();
    fireEvent.change(await screen.findByRole("textbox", { name: /llama-server\.exe/i }), { target: { value: "C:\\Tools\\program.exe" } });
    fireEvent.click(screen.getByRole("button", { name: /Save & check again/i }));
    expect(await screen.findByText(/must end with llama-server\.exe/i)).toBeInTheDocument();
  });
});

describe("computer task terminal states", () => {
  it.each([
    ["done", "completed"],
    ["cancelled", "cancelled"],
    ["error", "failed"],
    ["limit", "failed"],
    ["question", "waiting"],
  ])("maps %s to %s", (kind, expected) => {
    expect(terminalTaskStatus(kind, "running")).toBe(expected);
  });

  it("promotes the starting fallback while waiting for events", () => {
    expect(terminalTaskStatus("start", "starting")).toBe("running");
  });
});
