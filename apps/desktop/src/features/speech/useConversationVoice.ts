import { useCallback, useEffect, useRef, useState } from "react";
import { speechPreferencesDefaults, type ConversationLanguage, type VadSettings, type ChatStreamEvent, type ComputerTaskEvent, type TaskResponseDelta } from "../../contracts/index";
import { cancelConversationVoice, getConversationVoiceSnapshot, onConversationAudio,
  prepareConversationVoice, speakConversation, transcribeConversation } from "../../platform/api";
import { useSpeech } from "./LocalSpeechControls";
import { claimPlayback, clearPlayback, speechJobId } from "./usePipelinedSpeechPlayer";
import { decodePcm, pcmBase64, SpokenClauses, UtteranceGate } from "./conversationAudio";
// Worklets must remain packaged same-origin files. Vite otherwise inlines this small
// script as a data URL, which the desktop's script policy correctly rejects.
import captureUrl from "./conversationCapture.worklet.js?url&no-inline";

export type ConversationControl = {
  listening: boolean;
  preparing: boolean;
  installed: boolean;
  detail: string;
  toggle: () => void;
  mute: () => void;
  language: ConversationLanguage;
  updateLanguage: (language: ConversationLanguage) => void;
  vad: VadSettings;
  updateVad: (patch: Partial<VadSettings>) => void;
  resetVad: () => void;
};

type Options = { active: boolean; scope: string; onTurn: (text: string) => Promise<void>; onError: (message: string) => void };

/** Only media-device/playback state lives here. Rust owns preferences, model identity and turns. */
export function useConversationVoice(options: Options) {
  const { conversationEnabled, conversationLanguage, updateConversationLanguage, preferencesReady, updateConversationEnabled, conversationVad: vadSettings, updateConversationVad } = useSpeech();
  const latest = useRef(options); latest.current = options;
  const [listening, setListening] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [installed, setInstalled] = useState(false);
  const [detail, setDetail] = useState("");
  const [retry, setRetry] = useState(0);
  const context = useRef<AudioContext | null>(null);
  const ready = useRef(false);
  const captureActive = useRef(false);
  const mounted = useRef(true);
  const epoch = useRef(0);
  const output = useRef(new Set<AudioBufferSourceNode>());
  const scheduledUntil = useRef(0);
  const speechJob = useRef<string | null>(null);
  const inputJob = useRef<string | null>(null);
  const expectedSequence = useRef(0);
  const language = useRef(conversationLanguage); language.current = conversationLanguage;
  const queue = useRef<{ response: string; conversation: string; text: string }[]>([]);
  const pumping = useRef(false);
  const inputQueue = useRef<Float32Array[]>([]);
  const transcribing = useRef(false);
  const response = useRef<{ id: string; conversation: string; clauses: SpokenClauses; text: string } | null>(null);
  const suppressed = useRef(new Set<string>());
  const taskText = useRef(new Map<number, string>());
  const taskRun = useRef("");

  const stopOutput = useCallback(() => {
    epoch.current++;
    const id = speechJob.current;
    speechJob.current = null;
    if (id) void cancelConversationVoice(id).catch(() => undefined);
    queue.current = [];
    if (response.current) suppressed.current.add(response.current.id);
    if (suppressed.current.size > 32) suppressed.current.delete(suppressed.current.values().next().value!);
    response.current = null;
    for (const source of output.current) { try { source.stop(); } catch { /* already ended */ } }
    output.current.clear();
    scheduledUntil.current = 0;
    clearPlayback(stopOutput);
  }, []);

  const report = useCallback((error: unknown) => {
    if (!mounted.current) return;
    const message = String(error);
    setDetail(message);
    latest.current.onError(message);
  }, []);

  const pump = useCallback(async () => {
    if (pumping.current || !ready.current) return;
    pumping.current = true;
    try {
      while (ready.current && queue.current.length) {
        // Synthesize a short clause ahead, rather than filling minutes of audio in memory.
        const ctx = context.current;
        if (!ctx) break;
        while (ready.current && scheduledUntil.current - ctx.currentTime > 0.35) {
          await new Promise<void>((resolve) => window.setTimeout(resolve, 40));
        }
        const item = queue.current.shift();
        if (!item || !ready.current) continue;
        const generation = epoch.current;
        const jobId = speechJobId("conversation-tts");
        speechJob.current = jobId;
        expectedSequence.current = 0;
        claimPlayback(stopOutput);
        try {
          await speakConversation({ jobId, conversationId: item.conversation, responseId: item.response,
            text: item.text, language: language.current, voiceProfileId: "voice-default" });
        } catch (error) {
          if (epoch.current === generation && ready.current) { stopOutput(); report(error); }
        } finally {
          if (speechJob.current === jobId) speechJob.current = null;
        }
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

  const receiveUtterance = useCallback(async (audio?: Float32Array) => {
    if (!captureActive.current) return;
    if (audio) {
      if (inputQueue.current.length >= 2) { report("Please pause briefly while the previous speech is transcribed."); return; }
      inputQueue.current.push(audio);
    }
    if (transcribing.current || !ready.current) return;
    transcribing.current = true;
    try {
      while (ready.current && inputQueue.current.length) {
        const samples = inputQueue.current.shift()!;
        const jobId = speechJobId("conversation-stt");
        inputJob.current = jobId;
        try {
          const transcript = await transcribeConversation({ jobId, pcmBase64: pcmBase64(samples), language: language.current });
          if (!ready.current || inputJob.current !== jobId) continue;
          if (transcript.text.trim()) await latest.current.onTurn(transcript.text.trim());
        } catch (error) { if (ready.current && inputJob.current === jobId) report(error); }
        finally { if (inputJob.current === jobId) inputJob.current = null; }
      }
    } finally { transcribing.current = false; }
  }, [report]);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  useEffect(() => {
    if (!options.active || !conversationEnabled || !preferencesReady) return;
    if (!vadSettings.enabled) { setDetail("Voice detection is disabled. Enable it in microphone settings."); return; }
    let disposed = false;
    let media: MediaStream | null = null;
    let capture: AudioWorkletNode | null = null;
    let source: MediaStreamAudioSourceNode | null = null;
    let audioDispose: (() => void) | undefined;
    const gate = new UtteranceGate(stopOutput, (pcm) => void receiveUtterance(pcm), vadSettings.speechThresholdDb,
      vadSettings.silenceTimeoutSec * 1000, vadSettings.minSpeechDurationMs);
    setPreparing(true);
    void (async () => {
      try {
        const snapshot = await getConversationVoiceSnapshot();
        if (disposed) return;
        setInstalled(snapshot.installed);
        if (!snapshot.installed) { setDetail(snapshot.detail); return; }
        // WebView/OS permission remains mandatory. A denied request is not retried in a loop.
        media = await navigator.mediaDevices.getUserMedia({ audio: {
          echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1,
        } });
        if (disposed) { media.getTracks().forEach((track) => track.stop()); return; }
        const ctx = new AudioContext(); context.current = ctx;
        await ctx.resume();
        if (ctx.state !== "running") throw new Error("Audio is paused by the WebView. Use the existing microphone button once to enable conversation.");
        try {
          await ctx.audioWorklet.addModule(captureUrl);
        } catch {
          // WebView errors can contain the entire base64 script. Keep the producer's
          // message bounded and actionable instead of filling the window with code.
          throw new Error("Microphone capture could not start. Restart the updated Kestrel app and try the microphone again.");
        }
        if (disposed) return;
        audioDispose = await onConversationAudio((event) => {
          if (disposed || !ready.current || event.jobId !== speechJob.current) return;
          try {
            if (event.sequence !== expectedSequence.current++) throw new Error("A speech audio frame was lost or reordered. The written reply is retained.");
            const samples = decodePcm(event.pcmBase64);
            if (scheduledUntil.current - ctx.currentTime > 35) throw new Error("The spoken clause exceeded the audio buffer limit.");
            const buffer = ctx.createBuffer(1, samples.length, event.sampleRate);
            buffer.copyToChannel(samples, 0);
            const node = ctx.createBufferSource(); node.buffer = buffer; node.connect(ctx.destination);
            output.current.add(node);
            node.onended = () => { output.current.delete(node); node.disconnect(); };
            const start = Math.max(ctx.currentTime + 0.025, scheduledUntil.current);
            node.start(start); scheduledUntil.current = start + buffer.duration;
          } catch (error) { stopOutput(); report(error); }
        });
        if (disposed) { audioDispose(); return; }
        capture = new AudioWorkletNode(ctx, "kestrel-conversation-capture");
        capture.port.onmessage = (event: MessageEvent<Float32Array>) => { if (captureActive.current) gate.push(event.data); };
        source = ctx.createMediaStreamSource(media); source.connect(capture);
        const silent = ctx.createGain(); silent.gain.value = 0;
        capture.connect(silent); silent.connect(ctx.destination);
        captureActive.current = true;
        setListening(true); setDetail("Listening while resident CPU speech prepares…");
        await prepareConversationVoice();
        if (disposed) return;
        ready.current = true;
        void receiveUtterance();
        setListening(true); setDetail("Listening locally. Speak to continue; Shift-click for manual large Whisper.");
      } catch (error) {
        if (!disposed) { report(error); ready.current = false; captureActive.current = false; setListening(false); media?.getTracks().forEach((t) => t.stop()); }
      } finally { if (!disposed) setPreparing(false); }
    })();
    return () => {
      disposed = true; ready.current = false; captureActive.current = false; setListening(false); setPreparing(false);
      gate.reset(); source?.disconnect(); capture?.disconnect(); audioDispose?.();
      media?.getTracks().forEach((track) => track.stop());
      inputQueue.current = [];
      if (inputJob.current) void cancelConversationVoice(inputJob.current).catch(() => undefined);
      inputJob.current = null;
      stopOutput();
      const ctx = context.current; context.current = null;
      if (ctx && ctx.state !== "closed") void ctx.close();
    };
  }, [options.active, options.scope, conversationEnabled, conversationLanguage, preferencesReady, retry, receiveUtterance, report, stopOutput,
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

  const mute = useCallback(() => { ready.current = false; stopOutput(); updateConversationEnabled(false); }, [stopOutput, updateConversationEnabled]);
  const updateLanguage = useCallback(async (next: ConversationLanguage) => {
    if (next === language.current) return;
    ready.current = false;
    stopOutput();
    inputQueue.current = [];
    if (inputJob.current) void cancelConversationVoice(inputJob.current).catch(() => undefined);
    inputJob.current = null;
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
