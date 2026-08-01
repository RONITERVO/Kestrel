import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { demoSnapshot } from "./demo";
import { terminalTaskStatus } from "./OfflineWorkspace";

const profileApi = vi.hoisted(() => ({
  exportSetupProfile: vi.fn(),
  importSetupProfile: vi.fn(),
}));

vi.mock("./api", async (importOriginal) => ({
  ...await importOriginal<typeof import("./api")>(),
  exportSetupProfile: profileApi.exportSetupProfile,
  importSetupProfile: profileApi.importSetupProfile,
}));

beforeEach(() => {
  profileApi.exportSetupProfile.mockReset().mockResolvedValue({ path: "C:\\Research\\portable.json", message: "Safe profile exported." });
  profileApi.importSetupProfile.mockReset().mockResolvedValue(demoSnapshot);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn().mockResolvedValue(undefined) } });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Kestrel research experience", () => {
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
    expect(screen.getByText(/one model researcher/i)).toBeInTheDocument();
    expect(screen.getByText(/intentionally uncapped/i)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Portable setup" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Export safe profile/i })).toBeInTheDocument();
    expect(screen.getByText(/never contain weights, chats, research, credentials/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^Research$/i }));
    expect(await screen.findByText("Your research")).toBeInTheDocument();
  });

  it("displays safe exports and explains clipboard failures", async () => {
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn().mockRejectedValue(new Error("denied")) } });
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^System$/i }));
    fireEvent.click(await screen.findByRole("button", { name: /Export safe profile/i }));

    expect(await screen.findByDisplayValue("C:\\Research\\portable.json")).toBeInTheDocument();
    expect(await screen.findByText(/Safe profile exported.*Copy the displayed path manually/i)).toBeInTheDocument();
  });

  it("requires confirmation and adopts the imported snapshot", async () => {
    const imported = { ...demoSnapshot, status: { ...demoSnapshot.status, archive: "Imported offline archive" } };
    profileApi.importSetupProfile.mockResolvedValue(imported);
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^System$/i }));
    fireEvent.change(await screen.findByLabelText("Profile JSON path"), { target: { value: "C:\\Research\\portable.json" } });
    fireEvent.click(screen.getByRole("button", { name: /Import profile/i }));
    expect(profileApi.importSetupProfile).not.toHaveBeenCalled();

    confirm.mockReturnValue(true);
    fireEvent.click(screen.getByRole("button", { name: /Import profile/i }));
    await waitFor(() => expect(profileApi.importSetupProfile).toHaveBeenCalledWith("C:\\Research\\portable.json"));
    expect(await screen.findByText("Imported offline archive")).toBeInTheDocument();
    expect(screen.getByText(/Profile imported.*Full Access remains locked/i)).toBeInTheDocument();
  });

  it("surfaces profile API errors", async () => {
    profileApi.exportSetupProfile.mockRejectedValue(new Error("profile storage is read-only"));
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^System$/i }));
    fireEvent.click(await screen.findByRole("button", { name: /Export safe profile/i }));
    expect(await screen.findByText(/profile storage is read-only/i)).toBeInTheDocument();
  });

  it("keeps the historical control plane and optional developer repair discoverable", async () => {
    render(<App />);
    fireEvent.click(await screen.findByRole("button", { name: /^Control$/i }));
    expect(await screen.findByRole("heading", { name: /Ternary Bonsai/i })).toBeInTheDocument();
    expect(screen.getByText("SESSION INSPECTOR")).toBeInTheDocument();
    expect(screen.getByText(/one inference lease/i)).toBeInTheDocument();
    expect(screen.getByText(/private, persistent workspace/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Attach local context/i })).toBeInTheDocument();
    expect(screen.getByText("Vision")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Add local model folder/i })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /Computer/i }));
    expect(screen.getByRole("heading", { name: /bounded objective/i })).toBeInTheDocument();
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
    fireEvent.click(await screen.findByRole("button", { name: /^Control$/i }));
    fireEvent.change(await screen.findByRole("combobox", { name: "llama-server" }), { target: { value: "C:\\Tools\\program.exe" } });
    fireEvent.click(screen.getByRole("button", { name: /Save complete profile/i }));
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
