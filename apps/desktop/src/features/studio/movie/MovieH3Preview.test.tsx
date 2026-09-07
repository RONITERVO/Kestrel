import tauriConfig from "../../../../../../src-tauri/tauri.conf.json";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { MovieH3Preview } from "./MovieH3Preview";
import * as api from "../../../platform/api";
import type { MovieRenderPreviewEvent } from "../../../contracts/index";

vi.mock("../../../platform/api", () => ({
  onMovieRenderPreview: vi.fn(async () => () => undefined),
  getMovieRenderState: vi.fn(async () => ({ active: true })),
  getMovieImageAssetRenderState: vi.fn(async () => ({ active: true })),
}));
afterEach(() => { cleanup(); vi.clearAllMocks(); });

const frame = {
  kind: "frame", target: "movieClip", jobId: "take", projectId: "movie",
  detail: "Approximate live preview · sample 6 of 20", step: 6, total: 20,
  mimeType: "video/mp4", dataUrl: "data:video/mp4;base64,AQID",
} as MovieRenderPreviewEvent;

async function send(event: MovieRenderPreviewEvent) {
  await act(async () => vi.mocked(api.onMovieRenderPreview).mock.calls[0][0](event));
}

it("allows native video preview URLs under the packaged media policy", () => {
  // jsdom does not enforce CSP. Check the native window policy too: a working
  // development fixture previously hid this release-only playback failure.
  const directives = new Map<string, string[]>(tauriConfig.app.security.csp.split(";").map((item) => {
    const [directive, ...sources] = item.trim().split(/\s+/);
    return [directive, sources];
  }));
  expect(directives.get("media-src")).toContain(new URL(frame.dataUrl!).protocol);
});

it("shows playback failure while keeping progress and Stop usable, then recovers on the next preview", async () => {
  const stop = vi.fn();
  const { container } = render(<MovieH3Preview projectId="movie" active onStop={stop} />);
  await waitFor(() => expect(api.onMovieRenderPreview).toHaveBeenCalledTimes(1));
  await send(frame);
  const video = container.querySelector("video")!;
  expect(video).toHaveAttribute("src", frame.dataUrl);
  fireEvent.error(video);
  expect(screen.getByRole("alert")).toHaveTextContent("Preview couldn’t play. Rendering continues.");
  expect(video).not.toBeVisible();
  expect(screen.getByRole("progressbar")).toHaveAttribute("value", "6");
  fireEvent.click(screen.getByRole("button", { name: "Stop" }));
  expect(stop).toHaveBeenCalledTimes(1);
  await send({ ...frame, dataUrl: "data:image/webp;base64,BAUG", mimeType: "image/webp", step: 7 });
  const image = screen.getByRole("img", { name: "Approximate H3 generation preview" });
  fireEvent.load(image);
  expect(image).toBeVisible();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("shows an image decode failure for image-asset previews too", async () => {
  render(<MovieH3Preview assetId="asset" active />);
  await waitFor(() => expect(api.onMovieRenderPreview).toHaveBeenCalledTimes(1));
  await send({ ...frame, target: "imageAsset", projectId: undefined, jobId: "asset", mimeType: "image/jpeg", dataUrl: "data:image/jpeg;base64,AQID" });
  fireEvent.error(screen.getByRole("img", { name: "Approximate H3 generation preview" }));
  expect(screen.getByRole("alert")).toBeVisible();
});
