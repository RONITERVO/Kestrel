import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { TaskMemoryPanel } from "./TaskMemoryPanel";
import type { ComputerTaskEvent, TaskMemorySnapshot } from "../../contracts/index";

const api = vi.hoisted(() => ({ getTaskMemory: vi.fn(), acknowledgeTaskAction: vi.fn() }));
vi.mock("../../platform/api", () => api);
const memory: TaskMemorySnapshot = { taskId: "task", revision: 0, checkpoint: null, eventCount: 1, directionSequences: [], unsettledActions: [1] };
const events: ComputerTaskEvent[] = [{ runId: "task", step: 1, kind: "tool_start", title: "Using write_file", detail: "saved write", data: { path: "D:/approved/draft.txt" }, at: "2026-10-30T10:00:00Z" }];
beforeEach(() => { api.getTaskMemory.mockReset().mockResolvedValue(memory); api.acknowledgeTaskAction.mockReset(); });
afterEach(cleanup);
it("does not ask the producer to reconcile an action that is still running", async () => {
  render(<TaskMemoryPanel taskId="task" events={events} active onError={vi.fn()} />);
  expect(await screen.findByText(/An action is still in progress/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Record my review" })).not.toBeInTheDocument();
});
it("records an explicit review without starting work or claiming success", async () => {
  api.acknowledgeTaskAction.mockResolvedValue({});
  const onError = vi.fn();
  render(<TaskMemoryPanel taskId="task" events={events} onError={onError} />);
  const button = await screen.findByRole("button", { name: "Record my review" });
  expect(button).toBeDisabled();
  expect(screen.getByText(/path: D:\/approved\/draft.txt/)).toBeInTheDocument();
  fireEvent.change(screen.getByRole("textbox", { name: "What did you check?" }), { target: { value: "I checked the file; the new draft is there." } });
  api.getTaskMemory.mockResolvedValue({ ...memory, unsettledActions: [] });
  fireEvent.click(button);
  await waitFor(() => expect(api.acknowledgeTaskAction).toHaveBeenCalledWith("task", 1, "I checked the file; the new draft is there."));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Record my review" })).not.toBeInTheDocument());
  expect(onError).not.toHaveBeenCalled();
});
it("keeps the review and uncertainty visible if native persistence fails", async () => {
  api.acknowledgeTaskAction.mockRejectedValue("Cannot save task history");
  const onError = vi.fn();
  render(<TaskMemoryPanel taskId="task" events={events} onError={onError} />);
  const note = await screen.findByRole("textbox", { name: "What did you check?" });
  fireEvent.change(note, { target: { value: "The draft exists." } });
  fireEvent.click(screen.getByRole("button", { name: "Record my review" }));
  await waitFor(() => expect(onError).toHaveBeenCalledWith("Cannot save task history"));
  expect(note).toHaveValue("The draft exists.");
  expect(screen.getByText(/Kestrel stopped before saving/)).toBeInTheDocument();
});
