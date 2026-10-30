import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoSnapshot } from "../../preview/fixtures";
import type { ChatSession, ComputerTaskEvent, ComputerTaskRun } from "../../contracts/index";
import { OfflineWorkspace } from "./OfflineWorkspace";

const api = vi.hoisted(() => ({
  listChatSessions: vi.fn(),
  getChatSession: vi.fn(),
  saveChatReplyEdit: vi.fn(),
  discardChatReplyEdit: vi.fn(),
  listComputerTasks: vi.fn(),
  getComputerTask: vi.fn(),
  sendProducerMessage: vi.fn(),
  getProducerTaskStatus: vi.fn(),
  onComputerTaskEvent: vi.fn(),
}));

vi.mock("../../platform/api", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../platform/api")>(),
  listChatSessions: api.listChatSessions,
  listComputerTasks: api.listComputerTasks,
  getComputerTask: api.getComputerTask,
  sendProducerMessage: api.sendProducerMessage,
  getProducerTaskStatus: api.getProducerTaskStatus,
  getChatSession: api.getChatSession,
  saveChatReplyEdit: api.saveChatReplyEdit,
  discardChatReplyEdit: api.discardChatReplyEdit,
  onChatStream: vi.fn(async () => () => undefined),
  onComputerTaskEvent: api.onComputerTaskEvent,
  onRuntimeProgress: vi.fn(async () => () => undefined),
  getControlSnapshot: vi.fn(async () => demoSnapshot.control),
}));

const original: ChatSession = {
  id: "chat-1",
  title: "The Cartographer",
  modelId: demoSnapshot.control.models[0]?.id ?? "model",
  createdAt: "2026-09-29T10:00:00Z",
  updatedAt: "2026-09-29T10:05:00Z",
  messages: [
    { id: "question", role: "user", content: "Write a story.", attachments: [], createdAt: "2026-09-29T10:00:00Z" },
    { id: "reply", role: "assistant", content: "The tide rose 0.55 m.", attachments: [], createdAt: "2026-09-29T10:05:00Z" },
  ],
};

function withEdit(content: string): ChatSession {
  return {
    ...original,
    messages: original.messages.map((message) => message.id === "reply"
      ? { ...message, edited: { content, updatedAt: "2026-09-29T11:00:00Z" } }
      : message),
  };
}

let taskEventListener: (event: ComputerTaskEvent) => void;
beforeEach(() => {
  api.onComputerTaskEvent.mockReset().mockImplementation(async (listener) => {
    taskEventListener = listener;
    return () => undefined;
  });
  api.listComputerTasks.mockReset().mockResolvedValue([]);
  api.getComputerTask.mockReset();
  api.sendProducerMessage.mockReset();
  api.getProducerTaskStatus.mockReset().mockResolvedValue(null);
  api.listChatSessions.mockReset().mockResolvedValue([
    { id: "chat-1", title: "The Cartographer", modelId: original.modelId, updatedAt: original.updatedAt, messageCount: 2 },
  ]);
  api.getChatSession.mockReset().mockResolvedValue(original);
  api.saveChatReplyEdit.mockReset().mockImplementation(async (_session: string, _message: string, content: string) => withEdit(content));
  api.discardChatReplyEdit.mockReset().mockResolvedValue(original);
});

describe("producer conversation", () => {
  const savedTask: ComputerTaskRun = {
    id: "saved-producer", objective: "Keep the fox", modelId: original.modelId,
    access: "workspace", status: "completed", createdAt: original.createdAt, updatedAt: original.updatedAt,
    events: [], artifacts: [], attachments: [],
  };
  const questionEvent: ComputerTaskEvent = {
    runId: savedTask.id, step: 1, kind: "question", title: "Needs your input",
    detail: "Which shorter wording?", at: "2026-10-30T13:06:11Z",
    data: { options: ["Goodnight, little fox. Sleep well.", "Goodnight, little fox. Dream of summer."], recommendedIndex: 0 },
  };

  it("refreshes unselected history when another client reaches a question", async () => {
    api.listComputerTasks.mockResolvedValue([{ ...savedTask, status: "running", eventCount: 0, artifactCount: 0 }]);
    api.getComputerTask.mockResolvedValue({ ...savedTask, status: "waiting", events: [questionEvent] });
    render(<OfflineWorkspace control={demoSnapshot.control} onChanged={vi.fn()} onError={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /Computer/i }));
    expect(await screen.findByRole("button", { name: /Keep the fox/ })).toHaveTextContent("running");
    api.listComputerTasks.mockResolvedValue([{ ...savedTask, status: "waiting", eventCount: 1, artifactCount: 0 }]);
    act(() => taskEventListener(questionEvent));
    await waitFor(() => expect(screen.getByRole("button", { name: /Keep the fox/ })).toHaveTextContent("waiting"));
    fireEvent.click(screen.getByRole("button", { name: /Keep the fox/ }));
    await screen.findByRole("textbox", { name: "Message Kestrel" });
    expect(screen.getAllByText("Which shorter wording?")).toHaveLength(1);
    expect(screen.getByRole("list")).toHaveTextContent("Goodnight, little fox. Sleep well.");
  });

  it("follows a selected conversation continued from another native client", async () => {
    api.listComputerTasks.mockResolvedValue([{ ...savedTask, eventCount: 0, artifactCount: 0 }]);
    api.getComputerTask.mockResolvedValue(savedTask);
    render(<OfflineWorkspace control={demoSnapshot.control} onChanged={vi.fn()} onError={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /Computer/i }));
    fireEvent.click(await screen.findByRole("button", { name: /Keep the fox/ }));
    await screen.findByRole("textbox", { name: "Message Kestrel" });
    act(() => taskEventListener({ ...questionEvent, kind: "queued", title: "Message saved", detail: "Continuing", data: undefined }));
    expect(await screen.findByRole("button", { name: "Stop safely" })).toBeInTheDocument();
    act(() => taskEventListener(questionEvent));
    await screen.findByRole("combobox", { name: "Suggested answers" });
    expect(screen.queryByRole("button", { name: "Stop safely" })).not.toBeInTheDocument();
    fireEvent.change(screen.getByRole("combobox", { name: "Suggested answers" }), { target: { value: "Goodnight, little fox. Sleep well." } });
    expect(screen.getByRole("textbox", { name: "Message Kestrel" })).toHaveValue("Goodnight, little fox. Sleep well.");
    expect(api.sendProducerMessage).not.toHaveBeenCalled();
  });

  it("keeps events arriving while a saved transcript is opening", async () => {
    api.listComputerTasks.mockResolvedValue([{ ...savedTask, eventCount: 0, artifactCount: 0 }]);
    let resolveTask!: (run: ComputerTaskRun) => void;
    api.getComputerTask.mockImplementation(() => new Promise<ComputerTaskRun>((resolve) => { resolveTask = resolve; }));
    render(<OfflineWorkspace control={demoSnapshot.control} onChanged={vi.fn()} onError={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: /Computer/i }));
    fireEvent.click(await screen.findByRole("button", { name: /Keep the fox/ }));
    await act(async () => {
      taskEventListener(questionEvent);
      resolveTask({ ...savedTask, status: "running" });
    });
    await screen.findByRole("combobox", { name: "Suggested answers" });
    expect(screen.queryByRole("button", { name: "Stop safely" })).not.toBeInTheDocument();
    expect(screen.getAllByText("Which shorter wording?")).toHaveLength(1);
  });

  it.each(["completed", "running", "paused", "interrupted"])("accepts ordinary follow-up messages when %s", async (status) => {
    const task: ComputerTaskRun = {
      id: "producer-1", objective: "Make a rain lesson", modelId: original.modelId,
      access: "workspace", status, createdAt: original.createdAt, updatedAt: original.updatedAt,
      events: [{ runId: "producer-1", step: 1, kind: "tool_result", title: "Private technical detail", detail: "large raw tool output", at: original.updatedAt }],
      artifacts: [], attachments: [],
    };
    api.listComputerTasks.mockResolvedValue([{ ...task, eventCount: 1, artifactCount: 0 }]);
    api.getComputerTask.mockResolvedValue(task);
    api.sendProducerMessage.mockResolvedValue({ id: task.id, status: "running", eventCount: 2, artifactPaths: [] });
    const onError = vi.fn();
    render(<OfflineWorkspace control={{ ...demoSnapshot.control, runtime: { ...demoSnapshot.control.runtime, phase: "stopped" } }} onChanged={vi.fn()} onError={onError} />);
    fireEvent.click(screen.getByRole("button", { name: /Computer/i }));
    fireEvent.click(await screen.findByRole("button", { name: /Make a rain lesson/ }));
    const composer = await screen.findByRole("textbox", { name: "Message Kestrel" });
    expect(screen.queryByText("large raw tool output")).not.toBeInTheDocument();
    fireEvent.change(composer, { target: { value: "Please make it Finnish and keep the pictures." } });
    fireEvent.click(screen.getByRole("button", { name: /^Send$/ }));
    await waitFor(() => expect(api.sendProducerMessage).toHaveBeenCalledWith({ runId: task.id, answer: "Please make it Finnish and keep the pictures." }));
    await waitFor(() => expect(composer).toHaveValue(""));
    expect(onError).not.toHaveBeenCalled();
  });
});

afterEach(() => cleanup());

describe("editing a reply for listening and export", () => {
  it("keeps the model's reply and shows the producer's edit beside it", async () => {
    const onError = vi.fn();
    render(<OfflineWorkspace control={demoSnapshot.control} onChanged={vi.fn()} onError={onError} />);
    fireEvent.click(await screen.findByRole("button", { name: /The Cartographer/ }));
    const reply = (await screen.findByText("The tide rose 0.55 m.")).closest("article")!;

    expect(within(reply).queryByRole("button", { name: "Your edit" })).not.toBeInTheDocument();
    fireEvent.click(within(reply).getByRole("button", { name: "Edit reply" }));
    const editor = within(reply).getByRole("textbox", { name: "Edited reply" });
    expect(editor).toHaveValue("The tide rose 0.55 m.");
    expect(within(reply).getByText(/The model keeps reading its original answer/)).toBeInTheDocument();
    fireEvent.change(editor, { target: { value: "The tide rose by more than half a meter." } });
    fireEvent.click(within(reply).getByRole("button", { name: "Save edit" }));

    await waitFor(() => expect(api.saveChatReplyEdit).toHaveBeenCalledWith("chat-1", "reply", "The tide rose by more than half a meter."));
    const edited = await screen.findByText("The tide rose by more than half a meter.");
    const card = edited.closest("article")!;
    expect(within(card).getByRole("button", { name: "Your edit" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(within(card).getByRole("button", { name: "Answer" }));
    expect(await within(card).findByText("The tide rose 0.55 m.")).toBeInTheDocument();

    // Your own messages cannot be edited.
    const question = screen.getByText("Write a story.").closest("article")!;
    expect(within(question).queryByRole("button", { name: "Edit reply" })).not.toBeInTheDocument();

    fireEvent.click(within(card).getByRole("button", { name: "Edit reply" }));
    fireEvent.click(within(card).getByRole("button", { name: "Revert to original" }));
    await waitFor(() => expect(api.discardChatReplyEdit).toHaveBeenCalledWith("chat-1", "reply"));
    await waitFor(() => expect(within(card).queryByRole("button", { name: "Your edit" })).not.toBeInTheDocument());
    expect(onError).not.toHaveBeenCalled();
  });
});
