import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { GenerateAssetButton, MovieAssetCreator } from "./MovieAssetCreator";
import * as api from "../../../platform/api";
import type { MovieImageAssetGeneration } from "../../../contracts/index";

vi.mock("../../../platform/api", async () => ({ ...await vi.importActual<typeof import("../../../platform/api")>("../../../platform/api"), listMovieImageAssets: vi.fn(async () => []), onMovieImageAsset: vi.fn(async () => () => undefined), startMovieImageAsset: vi.fn(async () => "request") }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });

it("creates an image at a later frame attachment and waits for native attachment before closing", async () => {
  const attach = vi.fn(async () => undefined), defaultAttach = vi.fn();
  render(<MovieAssetCreator comfyRoot="D:/AI/ComfyUI" onUse={defaultAttach} onError={vi.fn()}><GenerateAssetButton label="Create last frame" onUse={attach} /></MovieAssetCreator>);
  fireEvent.click(screen.getByRole("button", { name: "Create last frame" }));
  fireEvent.change(screen.getByLabelText("Image asset direction"), { target: { value: "A keeper looking through the window" } });
  fireEvent.click(screen.getByRole("button", { name: /Generate images/ }));
  await waitFor(() => expect(api.startMovieImageAsset).toHaveBeenCalledTimes(1));
  const request = vi.mocked(api.startMovieImageAsset).mock.calls[0][0];
  expect(request.width).toBe(768); expect(request.height).toBe(448);
  expect(screen.getByRole("region", { name: "Live H3 preview" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Close image creator" })).toBeDisabled();
  const asset = { id: "asset", name: "Window", path: "", kind: "image" };
  await act(async () => vi.mocked(api.onMovieImageAsset).mock.calls[0][0]({ requestId: request.requestId, kind: "complete", stage: "complete", detail: "Image saved", progress: 1, at: "", generation: { id: request.requestId, status: "complete", prompt: "Window", candidates: [{ frameIndex: 8, asset }] } as MovieImageAssetGeneration }));
  fireEvent.click(screen.getByRole("button", { name: /Use image · 9$/ }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(attach).toHaveBeenCalledWith(asset); expect(defaultAttach).not.toHaveBeenCalled();
});
