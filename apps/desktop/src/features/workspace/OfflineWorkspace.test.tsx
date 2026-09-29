import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoSnapshot } from "../../preview/fixtures";
import type { ChatSession } from "../../contracts/index";
import { OfflineWorkspace } from "./OfflineWorkspace";

const api = vi.hoisted(() => ({
  listChatSessions: vi.fn(),
  getChatSession: vi.fn(),
  saveChatReplyEdit: vi.fn(),
  discardChatReplyEdit: vi.fn(),
}));

vi.mock("../../platform/api", async (importOriginal) => ({
  ...await importOriginal<typeof import("../../platform/api")>(),
  listChatSessions: api.listChatSessions,
  listComputerTasks: vi.fn(async () => []),
  getChatSession: api.getChatSession,
  saveChatReplyEdit: api.saveChatReplyEdit,
  discardChatReplyEdit: api.discardChatReplyEdit,
  onChatStream: vi.fn(async () => () => undefined),
  onComputerTaskEvent: vi.fn(async () => () => undefined),
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

beforeEach(() => {
  api.listChatSessions.mockReset().mockResolvedValue([
    { id: "chat-1", title: "The Cartographer", modelId: original.modelId, updatedAt: original.updatedAt, messageCount: 2 },
  ]);
  api.getChatSession.mockReset().mockResolvedValue(original);
  api.saveChatReplyEdit.mockReset().mockImplementation(async (_session: string, _message: string, content: string) => withEdit(content));
  api.discardChatReplyEdit.mockReset().mockResolvedValue(original);
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
