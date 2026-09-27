import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { GenerateAssetButton, MovieAssetCreator } from "./MovieAssetCreator";
import * as api from "../../../platform/api";
import type { ImageProject, ImageSummary, MovieImageAssetGeneration, MovieReferenceAsset } from "../../../contracts/index";
import type { ImageHandoff } from "../imageHandoff";

vi.mock("../../../platform/api", async () => ({
  ...await vi.importActual<typeof import("../../../platform/api")>("../../../platform/api"),
  listImageProjects: vi.fn(async () => []),
  getImageProject: vi.fn(),
  listMovieImageAssets: vi.fn(async () => []),
  importImageTakeAsMovieReference: vi.fn(),
}));

const asset = { id: "asset", name: "Night / Form · take 1", path: "", kind: "image" } as MovieReferenceAsset;

beforeEach(() => {
  vi.mocked(api.listImageProjects).mockResolvedValue([{ id: "image-1", title: "Night / Form", status: "complete", updatedAt: "2026-09-01T00:00:00Z", takeCount: 1, activeTakePath: "" } as ImageSummary]);
  vi.mocked(api.getImageProject).mockResolvedValue({ id: "image-1", title: "Night / Form", takes: [{ id: "take-1", status: "complete", path: "" }] } as unknown as ImageProject);
  vi.mocked(api.importImageTakeAsMovieReference).mockResolvedValue(asset);
});
afterEach(() => { cleanup(); vi.clearAllMocks(); });

it("attaches a chosen Image Studio take to the frame that asked for it once the native import finishes", async () => {
  const attach = vi.fn(async () => undefined), defaultAttach = vi.fn();
  render(<MovieAssetCreator production="Tomorrow's Weather" onUse={defaultAttach} onError={vi.fn()}><GenerateAssetButton label="Create last frame" onUse={attach} /></MovieAssetCreator>);
  fireEvent.click(screen.getByRole("button", { name: "Create last frame" }));
  expect(screen.getByRole("dialog", { name: "Choose a picture" })).toHaveTextContent("last frame · Tomorrow's Weather");
  fireEvent.click(await screen.findByRole("button", { name: "Use Night / Form · take 1" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(api.importImageTakeAsMovieReference).toHaveBeenCalledWith("image-1", "take-1");
  expect(attach).toHaveBeenCalledWith(asset);
  expect(defaultAttach).not.toHaveBeenCalled();
});

it("sends a new picture to Image Studio and brings the chosen take back to the same frame", async () => {
  const attach = vi.fn(async () => undefined);
  const onRequestImage = vi.fn<(handoff: ImageHandoff) => void>();
  render(<MovieAssetCreator production="Tomorrow's Weather" onUse={vi.fn()} onRequestImage={onRequestImage} onError={vi.fn()}><GenerateAssetButton label="Create first frame" direction="Mara at the window before the storm" onUse={attach} /></MovieAssetCreator>);
  fireEvent.click(screen.getByRole("button", { name: "Create first frame" }));
  expect(screen.getByLabelText("Picture direction")).toHaveValue("Mara at the window before the storm");
  fireEvent.click(screen.getByRole("button", { name: /Make it in Image Studio/ }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  const handoff = onRequestImage.mock.calls[0][0];
  expect(handoff).toMatchObject({ purpose: "first frame", production: "Tomorrow's Weather", direction: "Mara at the window before the storm" });
  await handoff.deliver("image-1", "take-1");
  expect(api.importImageTakeAsMovieReference).toHaveBeenCalledWith("image-1", "take-1");
  expect(attach).toHaveBeenCalledWith(asset);
});

it("keeps stills made by the retired H3 pass usable without importing them again", async () => {
  const still = { id: "still", name: "Compass still", path: "", kind: "image" } as MovieReferenceAsset;
  vi.mocked(api.listImageProjects).mockResolvedValue([]);
  vi.mocked(api.listMovieImageAssets).mockResolvedValue([{ id: "gen", status: "complete", prompt: "A compass", candidates: [{ frameIndex: 8, asset: still }] } as unknown as MovieImageAssetGeneration]);
  const attach = vi.fn(async () => undefined);
  render(<MovieAssetCreator production="Tomorrow's Weather" onUse={attach} onError={vi.fn()}><GenerateAssetButton /></MovieAssetCreator>);
  fireEvent.click(screen.getByRole("button", { name: "Create image" }));
  fireEvent.click(await screen.findByRole("button", { name: "Use Compass still" }));
  await waitFor(() => expect(attach).toHaveBeenCalledWith(still));
  expect(api.importImageTakeAsMovieReference).not.toHaveBeenCalled();
});
