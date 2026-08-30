import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoSnapshot } from "../../app/demo";
import { mergeSetupControlSnapshot, SetupConsole } from "./Setup";

const setupApi = vi.hoisted(() => ({
  install: vi.fn(),
  pickFolder: vi.fn(),
  save: vi.fn(),
  scanModels: vi.fn(),
}));

vi.mock("../../platform/api", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../platform/api")>(),
  installSetupComponent: setupApi.install,
  pickSetupFolder: setupApi.pickFolder,
  saveSetupLocations: setupApi.save,
  scanSetupModelFolder: setupApi.scanModels,
}));

beforeEach(() => {
  setupApi.install.mockReset().mockResolvedValue(demoSnapshot);
  setupApi.pickFolder.mockReset().mockResolvedValue("");
  setupApi.save.mockReset().mockResolvedValue(demoSnapshot);
  setupApi.scanModels.mockReset().mockResolvedValue({});
});

afterEach(cleanup);

describe("SetupConsole", () => {
  it("keeps installed components and editable expert locations visible", () => {
    render(<SetupConsole snapshot={demoSnapshot} onChanged={vi.fn()} onError={vi.fn()} />);
    expect(screen.getByRole("heading", { name: "Kestrel essentials are ready." })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Included local model" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "MiniMax H3 Movie Studio" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Ideogram 4 Image Studio" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "MiniMax Music 3 Production" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Whisper dictation + local voice" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Install Whisper + voice" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "MuScriptor audio to MIDI" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Open the official access page/ })).toHaveAttribute("href", "https://huggingface.co/MuScriptor/muscriptor-large");
    expect(screen.getByRole("heading", { name: "Add another local model" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Observed model downloader" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Use existing files/ })).toBeInTheDocument();
  });

  it("shows one-click essentials on a clean machine", () => {
    const clean = {
      ...demoSnapshot,
      setup: {
        ...demoSnapshot.setup,
        ready: false,
        components: demoSnapshot.setup.components.map((item) => ({ ...item, status: "missing" })),
      },
    };
    render(<SetupConsole snapshot={clean} onChanged={vi.fn()} onError={vi.fn()} />);
    expect(screen.getByRole("button", { name: /Set up essentials/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Set up production suite/ })).toBeInTheDocument();
    expect(screen.getByText(/Interrupted downloads resume/)).toBeInTheDocument();
  });

  it("installs every production component after saving a new empty location", async () => {
    const installRoot = "E:\\Empty Kestrel";
    let current = {
      ...demoSnapshot,
      settings: { ...demoSnapshot.settings, installRoot },
      setup: {
        ...demoSnapshot.setup,
        installRoot,
        components: demoSnapshot.setup.components.map((item) =>
          ["media", "studio", "music", "speech"].includes(item.id)
            ? { ...item, status: "missing" }
            : item,
        ),
      },
    };
    setupApi.save.mockResolvedValue(current);
    setupApi.install.mockImplementation(async (request: { component: string; installRoot: string }) => {
      current = {
        ...current,
        setup: {
          ...current.setup,
          components: current.setup.components.map((item) =>
            item.id === request.component ? { ...item, status: "ready" } : item,
          ),
        },
      };
      return current;
    });

    render(<SetupConsole snapshot={demoSnapshot} onChanged={vi.fn()} onError={vi.fn()} />);
    fireEvent.change(screen.getByDisplayValue(demoSnapshot.settings.installRoot), {
      target: { value: installRoot },
    });
    fireEvent.click(screen.getByRole("button", { name: /Set up production suite/ }));

    await waitFor(() => expect(setupApi.install).toHaveBeenCalledTimes(4));
    expect(setupApi.install.mock.calls.map(([request]) => request.component)).toEqual([
      "media", "studio", "music", "speech",
    ]);
    expect(setupApi.install.mock.calls.every(([request]) => request.installRoot === installRoot)).toBe(true);
  });

  it("requires explicit Ideogram non-commercial acceptance and uses the saved location", async () => {
    const installRoot = "E:\\Image Models";
    const saved = {
      ...demoSnapshot,
      settings: { ...demoSnapshot.settings, installRoot },
      setup: { ...demoSnapshot.setup, installRoot },
    };
    setupApi.save.mockResolvedValue(saved);
    render(<SetupConsole snapshot={demoSnapshot} onChanged={vi.fn()} onError={vi.fn()} />);
    const card = screen.getByRole("heading", { name: "Ideogram 4 Image Studio" }).closest("article");
    const installButton = card?.querySelector("button");
    expect(installButton).not.toBeNull();

    fireEvent.click(installButton!);
    expect(screen.getByRole("dialog", { name: /Ideogram 4 is non-commercial/i })).toBeInTheDocument();
    expect(setupApi.install).not.toHaveBeenCalled();
    const acceptButton = screen.getByRole("button", { name: /accept and install/i });
    expect(acceptButton).toBeDisabled();
    fireEvent.click(screen.getByRole("checkbox", { name: /have read and accept/i }));
    fireEvent.click(acceptButton);

    await waitFor(() => expect(setupApi.install).toHaveBeenCalledOnce());
    expect(setupApi.install).toHaveBeenCalledWith(expect.objectContaining({
      component: "image",
      installRoot,
      acceptIdeogramNonCommercialLicense: true,
    }));
  });

  it("requires the completed gated checkpoint and explicit MuScriptor acceptance", async () => {
    render(<SetupConsole snapshot={demoSnapshot} onChanged={vi.fn()} onError={vi.fn()} />);
    fireEvent.change(screen.getByPlaceholderText("Completed MuScriptor large model.safetensors"), {
      target: { value: "C:\\Users\\Producer\\Downloads\\model.safetensors" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Verify & use existing" }));

    expect(screen.getByRole("dialog", { name: /MuScriptor is for permitted non-commercial transcription/i })).toBeInTheDocument();
    const prepare = screen.getByRole("button", { name: /Prepare offline MuScriptor/i });
    expect(prepare).toBeDisabled();
    fireEvent.click(screen.getByRole("checkbox", { name: /accepted the official conditions/i }));
    fireEvent.click(prepare);

    await waitFor(() => expect(setupApi.install).toHaveBeenCalledWith(expect.objectContaining({
      component: "muscriptor",
      muscriptorCheckpointPath: "C:\\Users\\Producer\\Downloads\\model.safetensors",
      acceptMuscriptorNonCommercialLicense: true,
    })));
  });

  it("discovers models across an existing folder and reuses them for component setup", async () => {
    const musicId = "music:minimax_music3_dit_int8_convrot.safetensors";
    const existingPath = "D:\\AI\\ComfyUI\\models\\diffusion_models\\minimax_music3_dit_int8_convrot.safetensors";
    setupApi.pickFolder.mockResolvedValue("D:\\AI");
    setupApi.scanModels.mockResolvedValue({ [musicId]: existingPath });
    render(<SetupConsole snapshot={demoSnapshot} onChanged={vi.fn()} onError={vi.fn()} />);

    fireEvent.click(screen.getByRole("button", { name: /Use existing files/i }));
    fireEvent.click(screen.getByRole("button", { name: /Find models in a folder/i }));

    await waitFor(() => expect(setupApi.scanModels).toHaveBeenCalledWith("D:\\AI"));
    expect(screen.getByText(/Found 1 supported model file/)).toBeInTheDocument();
    const musicCard = screen.getByRole("heading", { name: "MiniMax Music 3 Production" }).closest("article");
    fireEvent.click(musicCard!.querySelector("button")!);
    await waitFor(() => expect(setupApi.install).toHaveBeenCalledWith(expect.objectContaining({
      component: "music",
      existingModelPaths: expect.objectContaining({ [musicId]: existingPath }),
    })));
  });

  it("merges downloader completion into the latest setup snapshot", () => {
    const latest = {
      ...demoSnapshot,
      settings: { ...demoSnapshot.settings, advancedMode: true },
      setup: { ...demoSnapshot.setup, ready: false },
    };
    const completedControl = {
      ...demoSnapshot.control,
      settings: { ...demoSnapshot.control.settings, selectedModelId: "downloaded-model" },
    };

    const merged = mergeSetupControlSnapshot(latest, completedControl);

    expect(merged?.settings.advancedMode).toBe(true);
    expect(merged?.setup.ready).toBe(false);
    expect(merged?.control.settings.selectedModelId).toBe("downloaded-model");
  });
});
