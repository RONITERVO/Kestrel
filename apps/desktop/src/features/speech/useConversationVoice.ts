import { useCallback, useEffect, useRef, useState } from "react";
import { speechPreferencesDefaults, type ConversationLanguage, type VadSettings, type ChatStreamEvent, type ComputerTaskEvent, type TaskResponseDelta } from "../../contracts/index";
import { cancelConversationVoice, getConversationVoiceSnapshot, onConversationCapture,
  prepareConversationVoice, speakConversation, startConversationCapture } from "../../platform/api";
import { useSpeech } from "./LocalSpeechControls";
import { claimPlayback, clearPlayback, observePlayback, speechJobId } from "./usePipelinedSpeechPlayer";
import { SpokenClauses } from "./conversationAudio";

export type ConversationControl = {
  listening: boolean; preparing: boolean; installed: boolean; detail: string;
  toggle: () => void; mute: () => void; language: ConversationLanguage;
  updateLanguage: (language: ConversationLanguage) => void;
  vad: VadSettings; updateVad: (patch: Partial<VadSettings>) => void; resetVad: () => void;
};
type Options = { active: boolean; scope: string; onTurn: (text: string) => Promise<void>; onError: (message: string) => void };

/** View adapter only. Native Rust owns microphone, AEC, endpointing, STT and playback. */
export function useConversationVoice(options: Options) {
  const { conversationEnabled, conversationLanguage, updateConversationLanguage, preferencesReady, updateConversationEnabled, conversationVad: vadSettings, updateConversationVad } = useSpeech();
  const latest = useRef(options); latest.current = options;
  const [listening, setListening] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [installed, setInstalled] = useState(false);
  const [detail, setDetail] = useState("");
  const [retry, setRetry] = useState(0);
  const [manualPlayback, setManualPlayback] = useState(false);
  const ready = useRef(false);
  const mounted = useRef(true);
  const epoch = useRef(0);
  const speechJob = useRef<string | null>(null);
  const captureSession = useRef<string | null>(null);
  const closingCapture = useRef<Promise<unknown>>(Promise.resolve());
  const language = useRef(conversationLanguage); language.current = conversationLanguage;
  const queue = useRef<{ response: string; conversation: string; text: string }[]>([]);
  const pumping = useRef(false);
  const inputQueue = useRef<{ session: string; text: string }[]>([]);
  const submitting = useRef(false);
  const response = useRef<{ id: string; conversation: string; clauses: SpokenClauses; text: string } | null>(null);
  const suppressed = useRef(new Set<string>());
  const taskText = useRef(new Map<number, string>());
  const taskRun = useRef("");

  const stopOutput = useCallback(() => {
    epoch.current++;
    const id = speechJob.current; speechJob.current = null;
    if (id) void cancelConversationVoice(id).catch(() => undefined);
    queue.current = [];
    if (response.current) suppressed.current.add(response.current.id);
    if (suppressed.current.size > 32) suppressed.current.delete(suppressed.current.values().next().value!);
    response.current = null; clearPlayback(stopOutput);
  }, []);
  const report = useCallback((error: unknown) => {
    if (!mounted.current) return;
    const message = String(error).slice(0, 1500); setDetail(message); latest.current.onError(message);
  }, []);
  const pump = useCallback(async () => {
    if (pumping.current || !ready.current) return;
    pumping.current = true;
    try {
      while (ready.current && captureSession.current && queue.current.length) {
        const item = queue.current.shift()!; const generation = epoch.current;
        const jobId = speechJobId("conversation-tts"); speechJob.current = jobId;
        claimPlayback(stopOutput);
        try {
          // Native playback bounds synthesis ahead of the speaker and resolves after draining.
          await speakConversation({ jobId, conversationId: item.conversation, responseId: item.response,
            text: item.text, language: language.current, voiceProfileId: "voice-default", audioSessionId: captureSession.current });
        } catch (error) { if (epoch.current === generation && ready.current) { stopOutput(); report(error); } }
        finally { if (speechJob.current === jobId) speechJob.current = null; }
      }
    } finally { pumping.current = false; }
  }, [report, stopOutput]);
  const append = useCallback((conversation: string, id: string, text: string, final = false) => {
    if (!ready.current || suppressed.current.has(id)) return;
    if (response.current?.id !== id) response.current = { id, conversation, clauses: new SpokenClauses(), text: "" };
    response.current.text += text;
    try {
      for (const clause of response.current.clauses.append(text, final)) {
        if (queue.current.length >= 128) throw new Error("Speech cannot keep up with this reply. The full written answer remains available.");
        queue.current.push({ response: id, conversation, text: clause });
      }
      void pump();
    } catch (error) { stopOutput(); report(error); }
  }, [pump, report, stopOutput]);
  const submitTurns = useCallback(async () => {
    if (submitting.current || !ready.current) return;
    submitting.current = true;
    try {
      while (ready.current && inputQueue.current.length) {
        const turn = inputQueue.current.shift()!;
        if (turn.session !== captureSession.current) continue;
        try { await latest.current.onTurn(turn.text); }
        catch (error) { if (turn.session === captureSession.current) report(error); }
      }
    } finally { submitting.current = false; }
  }, [report]);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => observePlayback((owner) => setManualPlayback(owner !== null && owner !== stopOutput)), [stopOutput]);
  useEffect(() => {
    if (!options.active || !conversationEnabled || !preferencesReady) return;
    if (!vadSettings.enabled) { setDetail("Voice detection is disabled. Enable it in microphone settings."); return; }
    if (manualPlayback) { setDetail("Voice conversation pauses during manual speech playback."); return; }
    let disposed = false; let failed = false; let unlisten: (() => void) | undefined;
    const session = speechJobId("conversation-capture");
    setPreparing(true);
    void (async () => {
      try {
        await closingCapture.current;
        if (disposed) return;
        const snapshot = await getConversationVoiceSnapshot();
        if (disposed) return;
        setInstalled(snapshot.installed);
        if (!snapshot.installed) { setDetail(snapshot.detail); setPreparing(false); return; }
        unlisten = await onConversationCapture((event) => {
          if (disposed || event.sessionId !== captureSession.current) return;
          if (event.kind === "listening") { setListening(true); setDetail("Listening while resident CPU speech prepares…"); }
          else if (event.kind === "speech_start") stopOutput();
          else if (event.kind === "capture_error") report(event.detail);
          else if (event.kind === "transcript" && event.text.trim()) {
            if (inputQueue.current.length >= 2) { report("Please pause briefly while the previous speech is submitted."); return; }
            inputQueue.current.push({ session, text: event.text.trim() }); void submitTurns();
          }
        });
        if (disposed) { unlisten(); return; }
        captureSession.current = session;
        // Capture can retain the first utterance while the two model executors warm.
        const capture = startConversationCapture(session);
        closingCapture.current = capture.catch(() => undefined);
        void capture.catch((error) => { failed = true; if (!disposed) { ready.current = false; setListening(false); setPreparing(false); stopOutput(); report(error); } });
        await prepareConversationVoice();
        if (disposed || failed || captureSession.current !== session) return;
        ready.current = true; setPreparing(false);
        setDetail("Listening locally. Speak to continue; Shift-click for manual large Whisper.");
        void submitTurns();
      } catch (error) {
        if (!disposed) { ready.current = false; setPreparing(false); setListening(false); report(error); void cancelConversationVoice(session).catch(() => undefined); }
      }
    })();
    return () => {
      disposed = true; ready.current = false; setListening(false); setPreparing(false);
      unlisten?.(); inputQueue.current = []; captureSession.current = null;
      void cancelConversationVoice(session).catch(() => undefined); stopOutput();
    };
  }, [options.active, options.scope, conversationEnabled, conversationLanguage, preferencesReady, retry, manualPlayback, report, stopOutput, submitTurns,
    vadSettings.enabled, vadSettings.speechThresholdDb, vadSettings.silenceTimeoutSec, vadSettings.minSpeechDurationMs]);


  const chatEvent = useCallback((event: ChatStreamEvent) => {
    if (event.kind === "token") append(event.sessionId, event.requestId, event.content ?? "");
    else if (event.kind === "done") append(event.sessionId, event.requestId, "", true);
    else if (["cancelled", "error"].includes(event.kind)) stopOutput();
  }, [append, stopOutput]);

  const taskDelta = useCallback((event: TaskResponseDelta) => {
    if (taskRun.current !== event.runId) { taskText.current.clear(); taskRun.current = event.runId; }
    if (event.kind === "started") taskText.current.set(event.step, "");
    if (event.kind === "token") {
      const previous = taskText.current.get(event.step) ?? "";
      taskText.current.set(event.step, previous + event.text);
      if (taskText.current.size > 16) taskText.current.delete(taskText.current.keys().next().value!);
      append(event.runId, event.responseId, event.text);
    } else if (event.kind === "done") append(event.runId, event.responseId, "", true);
    else if (event.kind === "cancelled") stopOutput();
  }, [append, stopOutput]);

  const taskEvent = useCallback((event: ComputerTaskEvent) => {
    if (!["message", "question", "done"].includes(event.kind)) return;
    const streamed = taskRun.current === event.runId ? taskText.current.get(event.step) ?? "" : "";
    const remaining = streamed && event.detail.startsWith(streamed) ? event.detail.slice(streamed.length) : event.detail;
    if (remaining.trim()) append(event.runId, speechJobId("task-speech"), remaining, true);
  }, [append]);

  const mute = useCallback(() => {
    ready.current = false; stopOutput(); inputQueue.current = [];
    if (captureSession.current) void cancelConversationVoice(captureSession.current).catch(() => undefined);
    captureSession.current = null; updateConversationEnabled(false);
  }, [stopOutput, updateConversationEnabled]);
  const updateLanguage = useCallback(async (next: ConversationLanguage) => {
    if (next === language.current) return;
    ready.current = false;
    stopOutput();
    inputQueue.current = [];
    if (captureSession.current) void cancelConversationVoice(captureSession.current).catch(() => undefined);
    try { await updateConversationLanguage(next); }
    finally { if (mounted.current) setRetry((value) => value + 1); }
  }, [stopOutput, updateConversationLanguage]);
  const toggle = useCallback(() => {
    if (listening || preparing) mute();
    else { updateConversationEnabled(true); if (!vadSettings.enabled) updateConversationVad({ enabled: true }); setRetry((value) => value + 1); }
  }, [listening, preparing, mute, updateConversationEnabled, updateConversationVad, vadSettings.enabled]);

  return { control: { listening, preparing, installed, detail, toggle, mute, language: conversationLanguage, updateLanguage, vad: vadSettings, updateVad: updateConversationVad,
    resetVad: () => updateConversationVad(speechPreferencesDefaults.conversation.vad) } satisfies ConversationControl,
    chatEvent, taskDelta, taskEvent, stopOutput };
}
