import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoSnapshot } from "../../preview/fixtures";
import type { StrataDashboardSnapshot, StrataDashboardRequestDetail } from "../../contracts/index";
import { StrataDashboardButton } from "./StrataDashboard";

const api = vi.hoisted(() => ({ getStrataDashboard: vi.fn(), getStrataRequestDetail: vi.fn() }));
vi.mock("../../platform/api", () => api);
const request: StrataDashboardRequestDetail = {
  request: { id: "abcdef123456", path: "/v1/chat/completions", state: "completed", startedAt: 1, wallSeconds: 30, queueSeconds: 0, loadSeconds: 0, firstTokenSeconds: 3, promptTokens: 4096, reusedTokens: 32, outputTokens: 512, prefillTokensPerSecond: 800, outputTokensPerSecond: 57 },
  input: "Input text", output: "Verified image result", reasoning: "Recorded reasoning", inputTruncated: false, outputTruncated: false, reasoningTruncated: false, error: null,
};
const snapshot: StrataDashboardSnapshot = {
  runtime: demoSnapshot.control.runtime, available: true, detail: "Live observations from the managed Strata runtime.",
  activity: { model: "Strata", contextWindow: 65536, images: true, phase: "thinking", queued: 0, promptTokens: 4096, promptRead: null, promptTotal: 4096, generatedTokens: 512, maxOutputTokens: 32768, elapsedSeconds: 30, tokensPerSecond: 57, prefillTokensPerSecond: 800 },
  hardware: null, requests: [request.request], requestCapture: true,
};

beforeEach(() => { api.getStrataDashboard.mockReset().mockResolvedValue(snapshot); api.getStrataRequestDetail.mockReset().mockResolvedValue(request); });
afterEach(cleanup);

describe("Strata dashboard", () => {
  it("observes only after opening and fetches request text only when selected", async () => {
    render(<StrataDashboardButton />);
    expect(api.getStrataDashboard).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Strata dashboard" }));
    expect(await screen.findByText("Strata")).toBeInTheDocument();
    expect(screen.getByText(/65.?536 context/)).toBeInTheDocument();
    expect(api.getStrataRequestDetail).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /chat\/completions/ }));
    expect(await screen.findByText("Verified image result")).toBeInTheDocument();
    expect(api.getStrataRequestDetail).toHaveBeenCalledWith("abcdef123456");
    fireEvent.click(screen.getByRole("tab", { name: "input" }));
    expect(await screen.findByText("Input text")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Close Strata dashboard" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Strata dashboard" })).toHaveFocus();
  });

  it("reports an unloaded runtime without attempting to load a model", async () => {
    api.getStrataDashboard.mockResolvedValue({ ...snapshot, available: false, activity: null, requests: [], detail: "Start a Strata model in Kestrel to view its dashboard." });
    render(<StrataDashboardButton />);
    fireEvent.click(screen.getByRole("button", { name: "Strata dashboard" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Start a Strata model"));
    expect(api.getStrataRequestDetail).not.toHaveBeenCalled();
  });
});
