import {
  AudioLines,
  Bot,
  Check,
  ChevronRight,
  CircleStop,
  Clipboard,
  Download,
  FileCode2,
  FileText,
  FolderOpen,
  History,
  Image,
  LoaderCircle,
  MessageSquarePlus,
  MonitorCog,
  Paperclip,
  Play,
  RefreshCw,
  Search,
  Send,
  ShieldCheck,
  Sparkles,
  Square,
  Trash2,
  Wrench,
  X,
  Zap,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  cancelChatStream,
  deleteChatSession,
  getChatSession,
  getComputerTask,
  getControlSnapshot,
  listChatSessions,
  listComputerTasks,
  onChatStream,
  onComputerTaskEvent,
  onRuntimeProgress,
  openContextAttachment,
  openTaskArtifact,
  pickContextFiles,
  pickLocalModelFolder,
  releaseAiMemory,
  resumeComputerTask,
  saveControlSettings,
  scanLocalModels,
  startChatStream,
  startComputerTask,
  startLocalModel,
  stopComputerTask,
  stopLocalModel,
} from "./api";
import type {
  ChatMessage,
  ChatSession,
  ChatSessionSummary,
  ChatStreamEvent,
  ComputerTaskEvent,
  ComputerTaskRun,
  ComputerTaskSummary,
  ContextAttachment,
  ControlSnapshot,
} from "./types";

type Props = {
  control: ControlSnapshot;
  onChanged: (control: ControlSnapshot) => void;
  onError: (message: string) => void;
};

type WorkKind = "chat" | "task";

export function OfflineWorkspace({ control, onChanged, onError }: Props) {
  const [kind, setKind] = useState<WorkKind>("chat");
  const [selectedId, setSelectedId] = useState(
    control.settings.selectedModelId ?? control.models[0]?.id ?? "",
  );
  const [filter, setFilter] = useState("");
  const [settings, setSettings] = useState(control.settings);
  const [sessions, setSessions] = useState<ChatSessionSummary[]>([]);
  const [session, setSession] = useState<ChatSession | null>(null);
  const [draft, setDraft] = useState("");
  const [chatAttachments, setChatAttachments] = useState<ContextAttachment[]>(
    [],
  );
  const [stream, setStream] = useState<{
    requestId: string;
    phase: string;
    content: string;
    reasoning: string;
    notice?: string;
    metrics?: Record<string, unknown>;
  } | null>(null);
  const [tasks, setTasks] = useState<ComputerTaskSummary[]>([]);
  const [task, setTask] = useState<ComputerTaskRun | null>(null);
  const [objective, setObjective] = useState("");
  const [taskAttachments, setTaskAttachments] = useState<ContextAttachment[]>(
    [],
  );
  const [taskAnswer, setTaskAnswer] = useState("");
  const [access, setAccess] = useState<"workspace" | "full">("workspace");
  const [working, setWorking] = useState<
    "scan" | "start" | "stop" | "release" | "save" | "task" | "attach" | null
  >(null);
  const [stoppingTask, setStoppingTask] = useState(false);
  const [runtimeProgress, setRuntimeProgress] = useState<string | null>(null);
  const [newRoot, setNewRoot] = useState("");
  const selected = control.models.find((model) => model.id === selectedId);
  const chatRequestRef = useRef<string | null>(null);
  const pendingRedirectRef = useRef<{
    message: string;
    attachments: ContextAttachment[];
  } | null>(null);
  const chatTerminalRef = useRef<{ kind: string; content?: string } | null>(
    null,
  );
  const latestChatRef = useRef({ selected, settings, session });
  const taskRunRef = useRef<string | null>(null);
  const latestTaskRef = useRef<ComputerTaskRun | null>(task);
  const taskStartingRef = useRef(false);
  const earlyTaskEventsRef = useRef<ComputerTaskEvent[]>([]);
  const chatEndRef = useRef<HTMLDivElement>(null);
  const enginePathHasValidName = /(?:^|[\\/])llama-server\.exe$/i.test(
    settings.enginePath.trim(),
  );
  const visibleModels = useMemo(
    () =>
      control.models.filter((model) =>
        `${model.name} ${model.source} ${model.quantization ?? ""}`
          .toLowerCase()
          .includes(filter.toLowerCase()),
      ),
    [control.models, filter],
  );

  const refreshHistory = async () => {
    const [nextSessions, nextTasks] = await Promise.all([
      listChatSessions(),
      listComputerTasks(),
    ]);
    setSessions(nextSessions);
    setTasks(nextTasks);
  };

  useEffect(() => {
    setSettings(control.settings);
  }, [control.settings]);
  useEffect(() => {
    latestChatRef.current = { selected, settings, session };
  }, [selected, settings, session]);
  useEffect(() => {
    latestTaskRef.current = task;
  }, [task]);
  useEffect(() => {
    void refreshHistory().catch(() => undefined);
    let unmounted = false;
    let chatDispose: (() => void) | undefined;
    let taskDispose: (() => void) | undefined;
    let runtimeDispose: (() => void) | undefined;
    void onChatStream(handleChatEvent).then((dispose) => {
      if (unmounted) dispose();
      else chatDispose = dispose;
    });
    void onComputerTaskEvent(handleTaskEvent).then((dispose) => {
      if (unmounted) dispose();
      else taskDispose = dispose;
    });
    void onRuntimeProgress((event) => setRuntimeProgress(event.detail)).then(
      (dispose) => {
        if (unmounted) dispose();
        else runtimeDispose = dispose;
      },
    );
    const timer = window.setInterval(
      () =>
        void getControlSnapshot(false)
          .then(onChanged)
          .catch(() => undefined),
      2_500,
    );
    return () => {
      unmounted = true;
      chatDispose?.();
      taskDispose?.();
      runtimeDispose?.();
      window.clearInterval(timer);
    };
    // Event handlers use refs and functional state updates, so they remain stable for this subscription.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (typeof chatEndRef.current?.scrollIntoView === "function")
      chatEndRef.current.scrollIntoView({ behavior: "smooth", block: "end" });
  }, [session?.messages.length, stream?.content, stream?.reasoning]);

  function handleChatEvent(event: ChatStreamEvent) {
    if (chatRequestRef.current && event.requestId !== chatRequestRef.current)
      return;
    if (event.kind === "token" || event.kind === "reasoning") {
      setStream((current) => ({
        requestId: event.requestId,
        phase: "generating",
        content:
          (current?.content ?? "") +
          (event.kind === "token" ? (event.content ?? "") : ""),
        reasoning:
          (current?.reasoning ?? "") +
          (event.kind === "reasoning" ? (event.content ?? "") : ""),
        metrics: current?.metrics,
      }));
    } else if (event.kind === "metrics") {
      setStream((current) =>
        current ? { ...current, metrics: event.data } : current,
      );
    } else if (event.kind === "context") {
      setStream((current) =>
        current
          ? { ...current, notice: event.content }
          : {
              requestId: event.requestId,
              phase: "preparing context",
              content: "",
              reasoning: "",
              notice: event.content,
            },
      );
    } else if (event.kind === "queued" || event.kind === "started") {
      setStream((current) =>
        current
          ? { ...current, phase: event.kind }
          : {
              requestId: event.requestId,
              phase: event.kind,
              content: "",
              reasoning: "",
            },
      );
    } else if (["done", "cancelled", "error"].includes(event.kind)) {
      chatTerminalRef.current = { kind: event.kind, content: event.content };
      setStream((current) =>
        current
          ? {
              ...current,
              phase: event.kind === "done" ? "finishing" : event.kind,
            }
          : current,
      );
    } else if (event.kind === "settled") {
      const terminal = chatTerminalRef.current;
      chatTerminalRef.current = null;
      void (async () => {
        try {
          const next = await getChatSession(event.sessionId);
          setSession(next);
          await refreshHistory();
          setStream((current) =>
            current?.requestId === event.requestId ? null : current,
          );
          chatRequestRef.current = null;
          if (terminal?.kind === "error")
            onError(
              terminal.content ?? "The local generation stopped unexpectedly.",
            );
          const redirect = pendingRedirectRef.current;
          pendingRedirectRef.current = null;
          if (redirect)
            await launchChat(redirect.message, next, redirect.attachments);
        } catch (cause) {
          setStream(null);
          chatRequestRef.current = null;
          pendingRedirectRef.current = null;
          onError(String(cause));
        }
      })();
    }
  }

  function handleTaskEvent(event: ComputerTaskEvent) {
    if (taskRunRef.current && event.runId !== taskRunRef.current) return;
    if (taskStartingRef.current) {
      earlyTaskEventsRef.current.push(event);
      return;
    }
    if (latestTaskRef.current?.id !== event.runId) {
      earlyTaskEventsRef.current.push(event);
      return;
    }
    setTask((current) => {
      if (current?.id !== event.runId) return current;
      return {
        ...current,
        status: terminalTaskStatus(event.kind, current.status),
        updatedAt: event.at,
        events: [...current.events, event],
        artifacts:
          event.kind === "artifact" &&
          event.data?.path &&
          !current.artifacts.includes(event.data.path)
            ? [...current.artifacts, event.data.path]
            : current.artifacts,
      };
    });
    if (
      ["done", "cancelled", "error", "limit", "question"].includes(event.kind)
    ) {
      taskRunRef.current = null;
      setStoppingTask(false);
      setWorking(null);
      void refreshHistory().catch(() => undefined);
    }
  }

  const act = async (
    next: typeof working,
    action: () => Promise<ControlSnapshot>,
  ) => {
    setWorking(next);
    try {
      onChanged(await action());
    } catch (cause) {
      onError(String(cause));
    } finally {
      setWorking(null);
    }
  };

  const openSession = async (summary: ChatSessionSummary) => {
    if (stream) return;
    try {
      const next = await getChatSession(summary.id);
      setSession(next);
      setSelectedId(next.modelId);
      setKind("chat");
    } catch (cause) {
      onError(String(cause));
    }
  };

  const openTask = async (summary: ComputerTaskSummary) => {
    if (taskRunRef.current) return;
    try {
      const next = await getComputerTask(summary.id);
      setTask(next);
      taskRunRef.current = ["running", "starting"].includes(next.status)
        ? next.id
        : null;
      setKind("task");
    } catch (cause) {
      onError(String(cause));
    }
  };

  const newConversation = () => {
    if (stream) return;
    setSession(null);
    setChatAttachments([]);
    chatRequestRef.current = null;
    pendingRedirectRef.current = null;
    setKind("chat");
  };
  const newTask = () => {
    if (active) return;
    setTask(null);
    setObjective("");
    setTaskAttachments([]);
    setTaskAnswer("");
    setKind("task");
  };

  async function launchChat(
    message: string,
    baseSession: ChatSession | null,
    attachments: ContextAttachment[] = [],
  ) {
    const current = latestChatRef.current;
    if (!current.selected) return;
    const optimistic: ChatMessage = {
      id: `pending-${Date.now()}`,
      role: "user",
      content: message || "Analyze the attached local context.",
      attachments,
      createdAt: new Date().toISOString(),
    };
    setSession(
      baseSession
        ? { ...baseSession, messages: [...baseSession.messages, optimistic] }
        : baseSession,
    );
    try {
      const started = await startChatStream({
        sessionId: baseSession?.id,
        modelId: current.selected.id,
        message,
        attachmentIds: attachments.map((item) => item.id),
        temperature: 0.2,
        topP: 0.9,
        topK: 40,
        maxOutputTokens: current.settings.maxOutputTokens,
      });
      chatRequestRef.current = started.requestId;
      setSession(started.session);
      setStream((current) =>
        current?.requestId === started.requestId
          ? current
          : {
              requestId: started.requestId,
              phase: "queued",
              content: "",
              reasoning: "",
            },
      );
      void refreshHistory();
    } catch (cause) {
      setSession(baseSession);
      setChatAttachments(
        (current) => mergeAttachments(current, attachments).attachments,
      );
      onError(String(cause));
    }
  }

  function reconcileChatStop(requestId: string) {
    window.setTimeout(() => {
      if (chatRequestRef.current !== requestId) return;
      void getControlSnapshot(false)
        .then(async (snapshot) => {
          if (
            snapshot.runtime.inferenceBusy ||
            chatRequestRef.current !== requestId
          )
            return;
          let baseSession = latestChatRef.current.session;
          const sessionId = baseSession?.id;
          if (sessionId) {
            const next = await getChatSession(sessionId);
            setSession(next);
            baseSession = next;
          }
          const redirect = pendingRedirectRef.current;
          setStream(null);
          chatRequestRef.current = null;
          chatTerminalRef.current = null;
          pendingRedirectRef.current = null;
          await refreshHistory();
          if (redirect)
            await launchChat(
              redirect.message,
              baseSession,
              redirect.attachments,
            );
        })
        .catch((cause) => onError(String(cause)));
    }, 5_000);
  }

  const send = async () => {
    if (!selected || (!draft.trim() && chatAttachments.length === 0)) return;
    const message = draft.trim();
    const attachments = chatAttachments;
    setDraft("");
    setChatAttachments([]);
    if (stream) {
      pendingRedirectRef.current = { message, attachments };
      setStream((current) =>
        current ? { ...current, phase: "redirecting" } : current,
      );
      try {
        await cancelChatStream(stream.requestId);
        reconcileChatStop(stream.requestId);
      } catch (cause) {
        pendingRedirectRef.current = null;
        setDraft(message);
        setChatAttachments(attachments);
        onError(String(cause));
      }
      return;
    }
    await launchChat(message, session, attachments);
  };

  const cancelGeneration = async () => {
    if (!stream) return;
    setStream((current) =>
      current ? { ...current, phase: "stopping" } : current,
    );
    try {
      await cancelChatStream(stream.requestId);
      reconcileChatStop(stream.requestId);
    } catch (cause) {
      onError(String(cause));
    }
  };

  const continueGeneration = async () => {
    if (stream) return;
    await launchChat(
      "Continue from the interrupted answer. Do not repeat completed material; first account for my latest instructions.",
      session,
    );
  };

  const attachFiles = async (target: WorkKind) => {
    setWorking("attach");
    try {
      const result = await pickContextFiles();
      const merged = mergeAttachments(
        target === "chat" ? chatAttachments : taskAttachments,
        result.attachments,
      );
      if (target === "chat") setChatAttachments(merged.attachments);
      else setTaskAttachments(merged.attachments);
      const failures = [...result.failures];
      if (merged.rejected > 0) {
        failures.push(
          `${merged.rejected} file(s) exceeded the 12-file or 256 MiB message limit.`,
        );
      }
      if (failures.length > 0) {
        onError(`Some files were not attached:\n${failures.join("\n")}`);
      }
    } catch (cause) {
      onError(String(cause));
    } finally {
      setWorking(null);
    }
  };

  const addModelFolder = async () => {
    setWorking("attach");
    try {
      const root = await pickLocalModelFolder();
      if (root && !settings.extraModelRoots.includes(root)) {
        setSettings((current) => ({
          ...current,
          extraModelRoots: [...current.extraModelRoots, root],
        }));
      }
    } catch (cause) {
      onError(String(cause));
    } finally {
      setWorking(null);
    }
  };

  const removeSession = async (summary: ChatSessionSummary) => {
    if (
      !window.confirm(
        `Archive “${summary.title}”? The JSON transcript is retained as a recoverable archive.`,
      )
    )
      return;
    try {
      await deleteChatSession(summary.id);
      if (session?.id === summary.id) setSession(null);
      await refreshHistory();
    } catch (cause) {
      onError(String(cause));
    }
  };

  const runTask = async () => {
    if (!selected || (!objective.trim() && taskAttachments.length === 0))
      return;
    if (access === "full" && !settings.allowFullAccessAgent) {
      onError(
        "Full access is locked. Enable it in the Session Inspector and save the profile first.",
      );
      return;
    }
    if (
      access === "full" &&
      !window.confirm(
        "Run this local model with full computer access? It may run programs and change files outside workspace folders. Every action is recorded, but the folder sandbox will not apply.",
      )
    )
      return;
    setWorking("task");
    taskStartingRef.current = true;
    try {
      const run = await startComputerTask({
        modelId: selected.id,
        objective:
          objective.trim() ||
          "Analyze the attached local context and complete the implied task safely.",
        attachmentIds: taskAttachments.map((item) => item.id),
        access,
        maxSteps: settings.agentMaxSteps,
        maxOutputTokens: settings.agentMaxOutputTokens,
      });
      const early = earlyTaskEventsRef.current.filter(
        (event) => event.runId === run.id,
      );
      earlyTaskEventsRef.current = earlyTaskEventsRef.current.filter(
        (event) => event.runId !== run.id,
      );
      const terminal = early
        .filter((event) =>
          ["done", "cancelled", "error", "limit", "question"].includes(
            event.kind,
          ),
        )
        .at(-1);
      taskRunRef.current = terminal ? null : run.id;
      const initialized = {
        ...run,
        status: terminal
          ? terminalTaskStatus(terminal.kind, run.status)
          : run.status,
        events: [...run.events, ...early],
      };
      latestTaskRef.current = initialized;
      setTask(initialized);
      taskStartingRef.current = false;
      setWorking(null);
      setObjective("");
      setTaskAttachments([]);
      void refreshHistory();
    } catch (cause) {
      taskStartingRef.current = false;
      setWorking(null);
      onError(String(cause));
    }
  };

  const stopTask = async () => {
    if (!task || stoppingTask) return;
    setStoppingTask(true);
    try {
      await stopComputerTask(task.id);
      window.setTimeout(() => setStoppingTask(false), 5_000);
    } catch (cause) {
      setStoppingTask(false);
      onError(String(cause));
    }
  };

  const resumeTask = async () => {
    if (!task || !taskAnswer.trim()) return;
    if (
      task.access === "full" &&
      !window.confirm(
        "Continue this task with full computer access? The model may run programs and modify files outside workspace folders.",
      )
    )
      return;
    setWorking("task");
    taskStartingRef.current = true;
    try {
      const next = await resumeComputerTask({
        runId: task.id,
        answer: taskAnswer.trim(),
      });
      const early = earlyTaskEventsRef.current.filter(
        (event) => event.runId === next.id,
      );
      earlyTaskEventsRef.current = earlyTaskEventsRef.current.filter(
        (event) => event.runId !== next.id,
      );
      const additional = early.filter(
        (event) =>
          !next.events.some(
            (known) =>
              known.at === event.at &&
              known.kind === event.kind &&
              known.title === event.title,
          ),
      );
      const terminal = additional
        .filter((event) =>
          ["done", "cancelled", "error", "limit", "question"].includes(
            event.kind,
          ),
        )
        .at(-1);
      taskRunRef.current = terminal ? null : next.id;
      const initialized = {
        ...next,
        status: terminal
          ? terminalTaskStatus(terminal.kind, next.status)
          : next.status,
        events: [...next.events, ...additional],
      };
      latestTaskRef.current = initialized;
      setTask(initialized);
      taskStartingRef.current = false;
      setTaskAnswer("");
      void refreshHistory();
    } catch (cause) {
      taskStartingRef.current = false;
      onError(String(cause));
    } finally {
      setWorking(null);
    }
  };

  const releaseMemory = async () => {
    if (
      !window.confirm(
        "Release all AI memory controlled by Kestrel? This safely cancels active Kestrel work, stops the configured Bonsai model service, and removes abandoned Kestrel model processes. Other model apps are left alone.",
      )
    )
      return;
    pendingRedirectRef.current = null;
    setWorking("release");
    try {
      const next = await releaseAiMemory();
      onChanged(next);
      setStream(null);
      chatRequestRef.current = null;
      taskRunRef.current = null;
      if (task) setTask(await getComputerTask(task.id));
      await refreshHistory();
    } catch (cause) {
      onError(String(cause));
    } finally {
      setWorking(null);
    }
  };

  const save = async () => {
    if (!enginePathHasValidName) {
      onError(
        "The model engine path must end with llama-server.exe. Choose a verified local engine before saving.",
      );
      return;
    }
    setWorking("save");
    try {
      onChanged(
        await saveControlSettings({
          ...settings,
          selectedModelId: selectedId || undefined,
        }),
      );
    } catch (cause) {
      onError(String(cause));
    } finally {
      setWorking(null);
    }
  };

  const addRoot = () => {
    const root = newRoot.trim();
    if (root && !settings.agentWorkspaceRoots.includes(root))
      setSettings({
        ...settings,
        agentWorkspaceRoots: [...settings.agentWorkspaceRoots, root],
      });
    setNewRoot("");
  };
  const updatePositiveSetting = (
    key:
      | "contextWindow"
      | "maxOutputTokens"
      | "agentMaxSteps"
      | "agentMaxOutputTokens",
    value: string,
  ) => {
    const next = Number(value);
    if (Number.isFinite(next) && next > 0)
      setSettings((current) => ({ ...current, [key]: next }));
  };
  const active = !!stream || !!taskRunRef.current;
  const resumableAnswer =
    !stream &&
    session?.messages.at(-1)?.role === "assistant" &&
    ["interrupted", "limited"].includes(session.messages.at(-1)?.status ?? "");
  const gpu = control.gpu;

  return (
    <div className="control-plane offline-workspace" data-mode={kind}>
      <aside className="model-drawer">
        <div className="control-product">
          <strong>KESTREL</strong>
          <span>OFFLINE WORKSPACE</span>
        </div>
        <div className="work-mode-switch">
          <button
            className={kind === "chat" ? "active" : ""}
            onClick={() => setKind("chat")}
          >
            <Bot /> Chat
          </button>
          <button
            className={kind === "task" ? "active" : ""}
            onClick={() => setKind("task")}
          >
            <MonitorCog /> Computer
          </button>
        </div>
        <div className="model-search">
          <Search size={14} />
          <input
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            placeholder="Find a local model"
          />
        </div>
        <div className="drawer-title">
          <span>MODEL</span>
          <button
            title="Read-only rescan"
            onClick={() => void act("scan", scanLocalModels)}
          >
            {working === "scan" ? (
              <LoaderCircle className="spin" />
            ) : (
              <RefreshCw />
            )}
          </button>
        </div>
        <div className="control-models compact-models">
          {visibleModels.map((model) => (
            <button
              key={model.id}
              disabled={active}
              className={selectedId === model.id ? "selected" : ""}
              onClick={() => {
                if (session && model.id !== session.modelId) newConversation();
                setSelectedId(model.id);
              }}
            >
              <Bot />
              <span>
                <strong>{model.name}</strong>
                <small>
                  {model.source} · {model.quantization ?? "GGUF"}
                  {model.supportsVision ? " · vision" : ""}
                  {model.supportsAudio ? " · audio" : ""}
                </small>
              </span>
              {control.runtime.modelId === model.id && (
                <i>{control.runtime.phase}</i>
              )}
            </button>
          ))}
        </div>
        <button
          type="button"
          className="drawer-model-add"
          onClick={() => void addModelFolder()}
          disabled={working === "attach"}
        >
          {working === "attach" ? <LoaderCircle className="spin" /> : <FolderOpen />}
          Add local model folder
        </button>
        {settings.extraModelRoots.length > 0 && <div className="drawer-model-roots" aria-label="Additional model folders">
          {settings.extraModelRoots.map((root) => <div key={root}>
            <span title={root}>{root}</span>
            <button
              aria-label={`Remove model folder ${root}`}
              onClick={() => setSettings((current) => ({
                ...current,
                extraModelRoots: current.extraModelRoots.filter((value) => value !== root),
              }))}
            ><X /></button>
          </div>)}
        </div>}
        <div className="drawer-title">
          <span>{kind === "chat" ? "CONVERSATIONS" : "TASK HISTORY"}</span>
          {kind === "chat" && (
            <button
              title="New conversation"
              disabled={!!stream}
              onClick={newConversation}
            >
              <MessageSquarePlus />
            </button>
          )}
        </div>
        <div className="history-list">
          {kind === "chat"
            ? sessions.map((item) => (
                <div
                  key={item.id}
                  className={session?.id === item.id ? "active" : ""}
                >
                  <button
                    disabled={!!stream}
                    onClick={() => void openSession(item)}
                  >
                    <span>{item.title}</span>
                    <small>
                      {item.messageCount} messages ·{" "}
                      {relativeTime(item.updatedAt)}
                    </small>
                  </button>
                  <button
                    title="Archive conversation"
                    disabled={!!stream}
                    onClick={() => void removeSession(item)}
                  >
                    <Trash2 />
                  </button>
                </div>
              ))
            : tasks.map((item) => (
                <button
                  key={item.id}
                  disabled={!!taskRunRef.current}
                  className={task?.id === item.id ? "active" : ""}
                  onClick={() => void openTask(item)}
                >
                  <span>{item.objective}</span>
                  <small>
                    {item.status} · {item.eventCount} events ·{" "}
                    {relativeTime(item.updatedAt)}
                  </small>
                </button>
              ))}
        </div>
        <div className="local-lock">
          <ShieldCheck />
          <span>
            <strong>Offline execution</strong>
            <small>
              Loopback model · durable transcripts · one inference lease
            </small>
          </span>
        </div>
      </aside>

      <section className="control-center">
        <header className="control-top">
          <div>
            <span className="eyebrow">
              {kind === "chat" ? "LOCAL CONVERSATION" : "VISIBLE COMPUTER WORK"}
            </span>
            <h1>
              {kind === "chat"
                ? (session?.title ?? selected?.name ?? "Choose a model")
                : (task?.objective ?? "Computer Tasks")}
            </h1>
          </div>
          <div className="control-actions">
            {kind === "chat" ? (
              <button
                className="quiet-button"
                disabled={!!stream}
                onClick={newConversation}
              >
                <MessageSquarePlus /> New chat
              </button>
            ) : (
              <>
                <button
                  className="quiet-button"
                  disabled={!!stream}
                  onClick={newConversation}
                >
                  <MessageSquarePlus /> New chat
                </button>
                <button
                  className="quiet-button"
                  disabled={active}
                  onClick={newTask}
                >
                  <MessageSquarePlus /> New task
                </button>
              </>
            )}
            {control.runtime.phase === "ready" ? (
              <button
                className="quiet-button"
                disabled={active}
                onClick={() => void act("stop", stopLocalModel)}
              >
                {working === "stop" ? (
                  <LoaderCircle className="spin" />
                ) : (
                  <CircleStop />
                )}
                {control.runtime.mode === "attached" ? "Detach" : "Stop"}
              </button>
            ) : (
              <button
                className="primary-button"
                disabled={!selected || !!working}
                onClick={() =>
                  selected &&
                  void act("start", () => startLocalModel(selected.id))
                }
              >
                {working === "start" ? (
                  <LoaderCircle className="spin" />
                ) : (
                  <Play />
                )}{" "}
                Load model
              </button>
            )}
          </div>
        </header>
        {kind === "chat" ? (
          <>
            <div className="control-chat" aria-live="polite">
              {working === "start" && runtimeProgress && (
                <RuntimeNotice title="MODEL STARTUP" detail={runtimeProgress} />
              )}{" "}
              {session?.messages.length ? (
                session.messages.map((message) => (
                  <Message
                    key={message.id}
                    message={message}
                    model={selected?.name}
                    onError={onError}
                  />
                ))
              ) : (
                <Welcome
                  models={control.models.length}
                  context={settings.contextWindow}
                  freeMib={gpu?.freeMib}
                />
              )}{" "}
              {stream && (
                <article className="assistant streaming">
                  <span>
                    {selected?.name ?? "MODEL"}
                    <i>{stream.phase}</i>
                  </span>
                  {stream.notice && (
                    <div className="context-notice">{stream.notice}</div>
                  )}
                  {stream.reasoning && (
                    <details open={!stream.content}>
                      <summary>Reasoning · live</summary>
                      <pre>{stream.reasoning}</pre>
                    </details>
                  )}
                  <RichText
                    value={
                      stream.content ||
                      (stream.phase === "queued"
                        ? "Waiting for the inference slot…"
                        : "")
                    }
                  />
                  {stream.metrics && <Metrics data={stream.metrics} />}
                </article>
              )}
              <div ref={chatEndRef} />
            </div>
            {resumableAnswer && (
              <div className="chat-resume">
                <span>The partial answer is saved.</span>
                <button onClick={() => void continueGeneration()}>
                  <Play /> Continue answer
                </button>
              </div>
            )}
            <div className={`control-composer ${stream ? "steerable" : ""}`}>
              {chatAttachments.length > 0 && (
                <AttachmentShelf
                  attachments={chatAttachments}
                  removable
                  onError={onError}
                  onRemove={(id) =>
                    setChatAttachments((items) =>
                      items.filter((item) => item.id !== id),
                    )
                  }
                />
              )}
              <div className="composer-row">
                <button
                  className="attach-button"
                  title="Attach local context"
                  disabled={
                    control.runtime.phase !== "ready" ||
                    !!taskRunRef.current ||
                    working === "attach"
                  }
                  onClick={() => void attachFiles("chat")}
                >
                  {working === "attach" ? (
                    <LoaderCircle className="spin" />
                  ) : (
                    <Paperclip />
                  )}
                </button>
                <textarea
                  value={draft}
                  onChange={(event) => setDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" && !event.shiftKey) {
                      event.preventDefault();
                      void send();
                    }
                  }}
                  placeholder={
                    control.runtime.phase !== "ready"
                      ? "Load a model to begin"
                      : taskRunRef.current
                        ? "Computer task is using the inference slot…"
                        : stream
                          ? "Add context or a message to redirect this answer…"
                          : "Message the active local model…"
                  }
                  disabled={
                    control.runtime.phase !== "ready" || !!taskRunRef.current
                  }
                />
                {stream && !draft.trim() && chatAttachments.length === 0 ? (
                  <button
                    title="Stop generation"
                    className="stop-generation"
                    onClick={() => void cancelGeneration()}
                  >
                    <Square />
                  </button>
                ) : (
                  <button
                    title={
                      stream ? "Send and redirect the current answer" : "Send"
                    }
                    onClick={() => void send()}
                    disabled={
                      (!draft.trim() && chatAttachments.length === 0) ||
                      control.runtime.phase !== "ready" ||
                      !!taskRunRef.current
                    }
                  >
                    {stream ? <Zap /> : <Send />}
                  </button>
                )}
              </div>
            </div>
          </>
        ) : (
          <ComputerTasks
            run={task}
            objective={objective}
            attachments={taskAttachments}
            answer={taskAnswer}
            access={access}
            ready={control.runtime.phase === "ready"}
            running={!!taskRunRef.current}
            stopping={stoppingTask}
            resuming={working === "task"}
            attaching={working === "attach"}
            fullUnlocked={settings.allowFullAccessAgent}
            onObjective={setObjective}
            onRemoveAttachment={(id) =>
              setTaskAttachments((items) =>
                items.filter((item) => item.id !== id),
              )
            }
            onAttach={() => void attachFiles("task")}
            onAnswer={setTaskAnswer}
            onAccess={setAccess}
            onRun={() => void runTask()}
            onResume={() => void resumeTask()}
            onStop={() => void stopTask()}
            onOpen={(path) => task && void openTaskArtifact(task.id, path)}
            onError={onError}
          />
        )}
      </section>

      <aside className="control-inspector">
        <span className="eyebrow">SESSION INSPECTOR</span>
        <div className="control-metric-grid">
          <Metric label="Runtime" value={control.runtime.phase} />
          <Metric label="Ownership" value={control.runtime.mode} />
          <Metric
            label="Context"
            value={
              control.runtime.contextWindow
                ? control.runtime.contextWindow.toLocaleString()
                : "—"
            }
          />
          <Metric
            label="Media"
            value={
              selected?.supportsAudio
                ? "Vision + audio"
                : selected?.supportsVision
                  ? "Vision"
                  : "Text extraction"
            }
          />
          <Metric
            label="Inference"
            value={
              active
                ? "Active here"
                : control.runtime.inferenceBusy
                  ? "Busy elsewhere"
                  : "Available"
            }
          />
        </div>
        {gpu && (
          <div className="control-memory">
            <strong>{gpu.name}</strong>
            <div>
              <span
                style={{
                  width: `${Math.min(100, (gpu.usedMib / gpu.totalMib) * 100)}%`,
                }}
              />
            </div>
            <small>
              {formatMib(gpu.usedMib)} used · {formatMib(gpu.freeMib)} free ·{" "}
              {gpu.utilizationPercent}% compute
            </small>
          </div>
        )}
        <p className="runtime-detail">{control.runtime.detail}</p>
        <button
          className="release-memory"
          disabled={working === "release"}
          onClick={() => void releaseMemory()}
        >
          {working === "release" ? (
            <LoaderCircle className="spin" />
          ) : (
            <CircleStop />
          )}
          <span>
            <strong>Release AI memory</strong>
            <small>Stop Kestrel + configured Bonsai models</small>
          </span>
        </button>
        {kind === "chat" && <section className="inspector-section inspector-mode-settings">
          <h2>Chat generation</h2>
          <div className="inline-runtime-settings inspector-setting-grid">
            <label>
              Context
              <input
                type="number"
                disabled={!settings.advancedMode}
                value={settings.contextWindow}
                onChange={(event) =>
                  updatePositiveSetting("contextWindow", event.target.value)
                }
              />
            </label>
            <label>
              Max output
              <input
                type="number"
                disabled={!settings.advancedMode}
                value={settings.maxOutputTokens}
                onChange={(event) =>
                  updatePositiveSetting("maxOutputTokens", event.target.value)
                }
              />
            </label>
            <label className="check-line inspector-wide-setting">
              <input
                type="checkbox"
                checked={settings.advancedMode}
                onChange={(event) =>
                  setSettings({
                    ...settings,
                    advancedMode: event.target.checked,
                  })
                }
              />{" "}
              Advanced, uncapped
            </label>
          </div>
        </section>}
        {kind === "task" && <section className="inspector-section inspector-mode-settings">
          <h2>Computer Tasks policy</h2>
          <div className="inline-runtime-settings inspector-setting-grid">
            <label>
              Maximum steps
              <input
                type="number"
                value={settings.agentMaxSteps}
                onChange={(event) =>
                  updatePositiveSetting("agentMaxSteps", event.target.value)
                }
              />
            </label>
            <label>
              Output per decision
              <input
                type="number"
                value={settings.agentMaxOutputTokens}
                onChange={(event) =>
                  updatePositiveSetting(
                    "agentMaxOutputTokens",
                    event.target.value,
                  )
                }
              />
            </label>
            <label className="check-line danger-toggle">
              <input
                type="checkbox"
                checked={settings.allowFullAccessAgent}
                onChange={(event) => {
                  if (
                    !event.target.checked ||
                    window.confirm(
                      "Unlock full computer access? Tasks will be able to run programs and operate outside workspace folders after an additional per-task confirmation.",
                    )
                  )
                    setSettings({
                      ...settings,
                      allowFullAccessAgent: event.target.checked,
                    });
                }}
              />{" "}
              Unlock full access
            </label>
            <div className="workspace-roots">
              {settings.agentWorkspaceRoots.map((root) => (
                <div key={root}>
                  <span title={root}>{root}</span>
                  <button
                    onClick={() =>
                      setSettings({
                        ...settings,
                        agentWorkspaceRoots:
                          settings.agentWorkspaceRoots.filter(
                            (value) => value !== root,
                          ),
                      })
                    }
                  >
                    ×
                  </button>
                </div>
              ))}
              <span className="root-entry">
                <label htmlFor="approved-folder">Approved folder</label>
                <span>
                  <input
                    id="approved-folder"
                    value={newRoot}
                    onChange={(event) => setNewRoot(event.target.value)}
                    placeholder="C:\Users\You\Work"
                  />
                  <button
                    type="button"
                    aria-label="Add approved folder"
                    onClick={addRoot}
                  >
                    Add
                  </button>
                </span>
              </span>
            </div>
          </div>
        </section>}
        {kind === "chat" && settings.advancedMode && (
          <div className="control-warning">
            Invalid or oversized values can stop startup or exhaust VRAM.
          </div>
        )}
        <button
          className="quiet-button inspector-save"
          disabled={!!working || active}
          onClick={() => void save()}
        >
          {working === "save" ? <LoaderCircle className="spin" /> : <Check />}{" "}
          Save complete profile
        </button>
        {control.runtime.launchArgs.length > 0 && (
          <details className="launch-proof">
            <summary>Exact engine launch</summary>
            <pre>{control.runtime.launchArgs.join(" ")}</pre>
          </details>
        )}
        <details className="launch-proof">
          <summary>Live runtime feed · {control.runtimeLogs.length}</summary>
          <pre>
            {control.runtimeLogs.length
              ? control.runtimeLogs
                  .slice(-120)
                  .map(
                    (entry) =>
                      `[${timeOnly(entry.at)} ${entry.stream}] ${entry.line}`,
                  )
                  .join("\n")
              : "Attached runtimes do not expose process logs. Managed runtime output will appear here."}
          </pre>
        </details>
      </aside>
    </div>
  );
}

function ComputerTasks({
  run,
  objective,
  attachments,
  answer,
  access,
  ready,
  running,
  stopping,
  resuming,
  attaching,
  fullUnlocked,
  onObjective,
  onRemoveAttachment,
  onAttach,
  onAnswer,
  onAccess,
  onRun,
  onResume,
  onStop,
  onOpen,
  onError,
}: {
  run: ComputerTaskRun | null;
  objective: string;
  attachments: ContextAttachment[];
  answer: string;
  access: "workspace" | "full";
  ready: boolean;
  running: boolean;
  stopping: boolean;
  resuming: boolean;
  attaching: boolean;
  fullUnlocked: boolean;
  onObjective: (value: string) => void;
  onRemoveAttachment: (id: string) => void;
  onAttach: () => void;
  onAnswer: (value: string) => void;
  onAccess: (value: "workspace" | "full") => void;
  onRun: () => void;
  onResume: () => void;
  onStop: () => void;
  onOpen: (path: string) => void;
  onError: (message: string) => void;
}) {
  const resumable =
    !!run &&
    ["waiting", "cancelled", "interrupted", "failed"].includes(run.status);
  const question = run
    ? [...run.events].reverse().find((event) => event.kind === "question")
    : undefined;
  return (
    <div className="computer-workspace">
      {!run || (!running && (objective || attachments.length > 0)) ? (
        <section className="task-launch">
          <div className="task-orbit">
            <MonitorCog />
          </div>
          <span className="eyebrow">ACTUAL COMPUTER WORK</span>
          <h2>Give the local model a bounded objective.</h2>
          <p>
            Every decision, tool call, result, error, and artifact stays visible
            and is saved locally. Attachments become durable task context.
            Decision-critical questions pause safely for your answer. Workspace
            mode is the everyday default.
          </p>
          <textarea
            value={objective}
            onChange={(event) => onObjective(event.target.value)}
            placeholder="Describe the outcome, or attach context and ask the model to inspect it."
          />
          {attachments.length > 0 && (
            <AttachmentShelf
              attachments={attachments}
              removable
              onRemove={onRemoveAttachment}
              onError={onError}
            />
          )}
          <button
            className="context-attach"
            disabled={attaching}
            onClick={onAttach}
          >
            {attaching ? <LoaderCircle className="spin" /> : <Paperclip />}{" "}
            Attach files as context
          </button>
          <div className="task-policy">
            <button
              className={access === "workspace" ? "active" : ""}
              onClick={() => onAccess("workspace")}
            >
              <ShieldCheck />
              <span>
                <strong>Workspace</strong>
                <small>Only approved folders</small>
              </span>
            </button>
            <button
              className={access === "full" ? "active danger" : ""}
              disabled={!fullUnlocked}
              onClick={() => onAccess("full")}
            >
              <Zap />
              <span>
                <strong>Full access</strong>
                <small>
                  {fullUnlocked
                    ? "Programs and all files"
                    : "Locked in profile"}
                </small>
              </span>
            </button>
          </div>
          <button
            className="primary-button task-run"
            disabled={!ready || (!objective.trim() && attachments.length === 0)}
            onClick={onRun}
          >
            <Play /> Start visible task
          </button>
        </section>
      ) : (
        <section className="task-run-view">
          <header>
            <div>
              <span className="eyebrow">
                {run.access.toUpperCase()} ACCESS · {run.status.toUpperCase()}
              </span>
              <h2>{run.objective}</h2>
              {(run.attachments?.length ?? 0) > 0 && (
                <AttachmentShelf
                  attachments={run.attachments!}
                  onError={onError}
                />
              )}
            </div>
            {running && (
              <button
                className="danger-button"
                disabled={stopping}
                onClick={onStop}
              >
                {stopping ? <LoaderCircle className="spin" /> : <Square />}{" "}
                {stopping ? "Stopping…" : "Stop safely"}
              </button>
            )}
          </header>
          <div className="task-timeline" aria-live="polite">
            {run.events.map((event, index) => (
              <article
                key={`${event.at}-${index}`}
                className={`task-event ${event.kind}`}
              >
                <div className="event-glyph">
                  {event.kind === "artifact" ? (
                    <FileCode2 />
                  ) : event.kind === "tool_start" ? (
                    <Wrench />
                  ) : event.kind === "thinking" || event.kind === "queued" ? (
                    <LoaderCircle className="spin" />
                  ) : event.kind === "done" ? (
                    <Check />
                  ) : (
                    <ChevronRight />
                  )}
                </div>
                <div>
                  <header>
                    <strong>{event.title}</strong>
                    <span>
                      {event.step ? `Step ${event.step}` : "Setup"} ·{" "}
                      {timeOnly(event.at)}
                    </span>
                  </header>
                  <pre>{event.detail}</pre>
                  {event.kind === "artifact" && event.data?.path && (
                    <button
                      className="artifact-button"
                      onClick={() => onOpen(event.data!.path!)}
                    >
                      <FolderOpen /> Open artifact
                    </button>
                  )}
                </div>
              </article>
            ))}
          </div>
          {resumable && (
            <section className="task-question">
              <span className="eyebrow">
                {run.status === "waiting"
                  ? "YOUR DECISION"
                  : "CONTINUE DURABLE TASK"}
              </span>
              <h3>
                {question?.detail ?? "Add direction before the model resumes."}
              </h3>
              {question?.data?.options && (
                <div className="question-options">
                  {question.data.options.map((option, index) => (
                    <button
                      key={option}
                      className={
                        question.data?.recommendedIndex === index
                          ? "recommended"
                          : ""
                      }
                      onClick={() => onAnswer(option)}
                    >
                      {option}
                      {question.data?.recommendedIndex === index && (
                        <small>Recommended</small>
                      )}
                    </button>
                  ))}
                </div>
              )}
              <textarea
                value={answer}
                onChange={(event) => onAnswer(event.target.value)}
                placeholder="Answer or add a precise continuation instruction…"
              />
              <button
                className="primary-button"
                disabled={!ready || !answer.trim() || resuming}
                onClick={onResume}
              >
                {resuming ? <LoaderCircle className="spin" /> : <Play />} Resume
                this task
              </button>
            </section>
          )}
          {run.artifacts.length > 0 && (
            <footer className="artifact-shelf">
              <span>
                <FileCode2 /> {run.artifacts.length} artifact
                {run.artifacts.length === 1 ? "" : "s"}
              </span>
              {run.artifacts.map((path) => (
                <button key={path} onClick={() => onOpen(path)} title={path}>
                  {fileName(path)}
                </button>
              ))}
            </footer>
          )}
        </section>
      )}
    </div>
  );
}

function Message({
  message,
  model,
  onError,
}: {
  message: ChatMessage;
  model?: string;
  onError: (message: string) => void;
}) {
  return (
    <article className={message.role}>
      <span>
        {message.role === "user" ? "YOU" : (model ?? "MODEL")}
        {message.status && (
          <i>
            {message.status === "limited"
              ? "output limit · continuation available"
              : "interrupted · partial saved"}
          </i>
        )}
        <button
          title="Copy message"
          onClick={() => void navigator.clipboard.writeText(message.content)}
        >
          <Clipboard />
        </button>
      </span>
      {(message.attachments?.length ?? 0) > 0 && (
        <AttachmentShelf attachments={message.attachments!} onError={onError} />
      )}{" "}
      {message.reasoning && (
        <details>
          <summary>Reasoning</summary>
          <pre>{message.reasoning}</pre>
        </details>
      )}
      <RichText value={message.content} />
    </article>
  );
}

function AttachmentShelf({
  attachments,
  removable = false,
  onRemove,
  onError,
}: {
  attachments: ContextAttachment[];
  removable?: boolean;
  onRemove?: (id: string) => void;
  onError: (message: string) => void;
}) {
  return (
    <div className="attachment-shelf">
      {attachments.map((attachment) => (
        <div
          className={`attachment-chip ${attachment.kind}`}
          key={attachment.id}
          title={`${attachment.note}\n${attachment.sha256}`}
        >
          <button
            className="attachment-open"
            onClick={() =>
              void openContextAttachment(attachment.id).catch((cause) =>
                onError(String(cause)),
              )
            }
          >
            <AttachmentIcon attachment={attachment} />
            <span>
              <strong>{attachment.name}</strong>
              <small>
                {attachment.contextMode === "native_media"
                  ? "native media"
                  : attachment.contextMode === "metadata_only"
                    ? "metadata only"
                    : `${attachment.extractedChars.toLocaleString()} characters`}{" "}
                · {formatBytes(attachment.bytes)}
              </small>
            </span>
          </button>
          {removable && (
            <button
              className="attachment-remove"
              aria-label={`Remove ${attachment.name}`}
              onClick={() => onRemove?.(attachment.id)}
            >
              <X />
            </button>
          )}
        </div>
      ))}
    </div>
  );
}

function AttachmentIcon({ attachment }: { attachment: ContextAttachment }) {
  if (attachment.kind === "image") return <Image />;
  if (attachment.kind === "audio") return <AudioLines />;
  return <FileText />;
}

function RichText({ value }: { value: string }) {
  if (!value) return <p className="stream-cursor">Thinking</p>;
  const blocks = value.split(/```/g);
  return (
    <div className="rich-message">
      {blocks.map((block, index) =>
        index % 2 ? (
          <pre key={index}>
            <code>{block.replace(/^\w+\n/, "")}</code>
          </pre>
        ) : (
          block
            .split(/\n{2,}/)
            .filter(Boolean)
            .map((paragraph, child) => (
              <p key={`${index}-${child}`}>{inlineCode(paragraph)}</p>
            ))
        ),
      )}
    </div>
  );
}

function inlineCode(value: string) {
  return value
    .split(/(`[^`]+`)/g)
    .map((part, index) =>
      part.startsWith("`") && part.endsWith("`") ? (
        <code key={index}>{part.slice(1, -1)}</code>
      ) : (
        <span key={index}>{part}</span>
      ),
    );
}

function Metrics({ data }: { data: Record<string, unknown> }) {
  const usage = data.usage as Record<string, number> | undefined;
  const timings = data.timings as Record<string, number> | undefined;
  return (
    <div className="generation-metrics">
      <span>{usage?.completion_tokens ?? 0} tokens</span>
      {timings?.predicted_per_second && (
        <span>{timings.predicted_per_second.toFixed(1)} tok/s</span>
      )}
    </div>
  );
}
function RuntimeNotice({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="runtime-feed">
      <LoaderCircle className="spin" />
      <span>
        <strong>{title}</strong>
        {detail}
      </span>
    </div>
  );
}
function Welcome({
  models,
  context,
  freeMib,
}: {
  models: number;
  context: number;
  freeMib?: number;
}) {
  return (
    <div className="control-welcome">
      <Sparkles />
      <h2>Your private, persistent workspace.</h2>
      <p>
        Stream a conversation, review reasoning and metrics, or give the same
        local model a visible computer task. Nothing is sent away.
      </p>
      <div>
        <span>
          <strong>{models}</strong> models
        </span>
        <span>
          <strong>{context.toLocaleString()}</strong> context
        </span>
        <span>
          <strong>{freeMib === undefined ? "—" : formatMib(freeMib)}</strong>{" "}
          VRAM free
        </span>
      </div>
    </div>
  );
}
function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="control-metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
export function terminalTaskStatus(kind: string, fallback: string) {
  if (kind === "done") return "completed";
  if (kind === "cancelled") return "cancelled";
  if (kind === "question") return "waiting";
  if (kind === "error" || kind === "limit") return "failed";
  return fallback === "starting" ? "running" : fallback;
}
function relativeTime(value: string) {
  const delta = Date.now() - new Date(value).getTime();
  if (delta < 60_000) return "now";
  if (delta < 3_600_000) return `${Math.floor(delta / 60_000)}m ago`;
  if (delta < 86_400_000) return `${Math.floor(delta / 3_600_000)}h ago`;
  return new Date(value).toLocaleDateString();
}
function timeOnly(value: string) {
  return new Date(value).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}
function fileName(path: string) {
  return path.split(/[\\/]/).pop() ?? path;
}
function formatMib(value: number) {
  return value >= 1024
    ? `${(value / 1024).toFixed(1)} GiB`
    : `${value.toLocaleString()} MiB`;
}
function formatBytes(value: number) {
  if (value >= 1024 * 1024 * 1024)
    return `${(value / 1024 / 1024 / 1024).toFixed(2)} GiB`;
  if (value >= 1024 * 1024) return `${(value / 1024 / 1024).toFixed(1)} MiB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${value} B`;
}
export function mergeAttachments(
  current: ContextAttachment[],
  added: ContextAttachment[],
) {
  const attachments: ContextAttachment[] = [];
  const seen = new Set<string>();
  let bytes = 0;
  let rejected = 0;
  for (const attachment of [...current, ...added]) {
    if (seen.has(attachment.id)) continue;
    seen.add(attachment.id);
    if (
      attachments.length >= 12 ||
      bytes + attachment.bytes > 256 * 1024 * 1024
    ) {
      rejected += 1;
      continue;
    }
    attachments.push(attachment);
    bytes += attachment.bytes;
  }
  return { attachments, rejected };
}
